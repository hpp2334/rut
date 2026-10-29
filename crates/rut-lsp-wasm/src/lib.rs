//! rut-lsp-wasm — the language core AND its wasm ABI, one crate.
//! The queries (`crate::analysis` and the sibling modules below) and the
//! values live here beside the wasm-bindgen face; the VS Code extension
//! instantiates it and registers its providers directly — no server
//! process, no per-platform binaries, no second crate to keep in step.
//!
//! wasm-bindgen over the language core (the `rut-wasm` pattern): the
//! module exports ONE class, [`RutLsp`]; the host constructs it with
//! `new RutLsp()` and calls its methods. Every string answer is the
//! JSON envelope; the wasm-bindgen runtime owns the string copies both
//! ways (the raw-ABI era's `rut_begin`/`rut_alloc` arena and its
//! ptr/len plumbing are gone — no `unsafe` anywhere in this crate):
//!
//!   new RutLsp()                               the language core, with
//!                                              the std surface pre-indexed
//!   legend() -> String                         token-type names, JSON array
//!   analyze(uri, src) -> String                store the doc + return
//!                                              {diags, tokens, symbols}
//!   forget(uri)                                drop a closed doc
//!   hover(uri, line, ch) -> String             LSP Hover JSON or null
//!   complete(uri, line, ch) -> String          LSP CompletionItem array
//!   complete_in(uri, src, line, ch) -> String  the same array over the
//!                                              CURRENT text (stored
//!                                              first — the doc-sync
//!                                              lane; completion fires
//!                                              on the keystroke, the
//!                                              analyze lane debounces)
//!   definition(uri, line, ch) -> String        LSP Location array or null
//!   type_definition(uri, line, ch) -> String   same shape
//!   inlay(uri, sl, sc, el, ec) -> String       LSP InlayHint array or null
//!   references(uri, line, ch, incl) -> String  LSP Location array or null
//!   signature_help(uri, line, ch) -> String    LSP SignatureHelp or null
//!   add_def(uri, src)                          index a workspace file
//!   parse_manifest(text) -> String             rut.jsonc bytes -> the
//!                                              dep-table JSON (or the
//!                                              error envelope)
//!   set_lock(text) -> String                   rut.lock bytes -> the
//!                                              url→pin ledger
//!                                              parse_manifest stamps
//!                                              onto url rows
//!   set_direct_deps(names) -> String           the root manifest's dep
//!                                              names (JSON array) — the
//!                                              direct-dep stamp the dep
//!                                              indexes carry (the
//!                                              auto-import tier's gate)
//!   add_def_named(uri, name, src)              index a dep source file
//!                                              under its module NAME
//!   add_def_mod(uri, name, path, src)          index a dep source at a
//!                                              MOD PATH; the envelope
//!                                              names the file's own mod
//!                                              decls (the host's mount
//!                                              walk reads its next hop
//!                                              from them)
//!   add_bundle(bytes) -> String                .rutbundle bytes -> index
//!                                              the bundle's own sources;
//!                                              envelope names module +
//!                                              entry paths (or the error)
//!   extract_bundle(bytes) -> String            .rutbundle bytes -> the
//!                                              editable surface envelope
//!                                              {module, namespace,
//!                                              consts, files:[{path,
//!                                              src, mode, mod_path}]}
//!                                              — NO indexing, JS owns
//!                                              the disk
//!   doc_len(uri) -> u32                        an open doc's stored
//!                                              length (the smoke's
//!                                              round-trip check)
//!
//! The package the hosts consume is `pkg/`: `wasm-pack build --target
//! web` writes the generated glue there, then `scripts/build-lsp-pkg.mjs`
//! installs this crate's JS face (`js/index.js` — the one place that
//! `JSON.parse`s the answers) and its declarations (`js/index.d.ts`)
//! beside it. Hosts import `pkg/index.js` and never declare these shapes
//! themselves; the field names live in `wire` (this module's serializers)
//! and the JS face's d.ts mirror, locked by `tsc` in every host and by
//! the extension's wire-shape e2e block.
//!
//! These are the deps face: the manifest/bundle plumbing runs through
//! [`crate::deps`] (pure — bytes in, values out) into the SAME
//! `state.defs` store `add_def` writes, so a dep's surface resolves
//! in hover/completion/definition exactly like a workspace file — one
//! index, no drift. `extract_bundle` is the same plumbing's extract
//! lane — bytes in, the surface envelope out, NO index write (JS owns
//! the disk; re-indexing goes back through `add_def`/`add_def_mod`).
//! `parse_manifest`/`add_bundle`/`extract_bundle` answer with an error
//! envelope (`{"error": …}`) on bad bytes; the host surfaces it as a
//! hint, never silent unparsed state.
//!
//! One deviation from `rut-wasm`'s one-shot model carries over from the
//! raw ABI: an LSP serves a long-lived editing session, so the instance
//! OWNS its session state (open docs + the index) for its lifetime —
//! the host holds one `RutLsp` per boot and mutates it per request.
//! Single-threaded JS, one `RefCell` borrow per call: trivially safe,
//! structurally.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

use crate::hover::DefIndex;
use ls_types::Uri;
use wasm_bindgen::prelude::wasm_bindgen;

mod wire;

// ---- the language core — the queries and the values ----
//
// One crate now: the queries live beside the ABI that serves them, so
// a feature cannot land on one face and miss the other. `query` is a
// private directory with public leaves — the canonical paths stay the
// flat `crate::analysis`, `crate::completion`, … that the ABI bodies
// (and the tests) spell.
mod query;

pub use query::{analysis, check, completion, definition, inlay, references, signature_help};

pub mod deps;
pub mod hover;
pub mod line_index;
pub mod mods;
pub mod semantic;
pub mod std_surface;

// ---- the language core's state (one instance per editing session) ----

struct Core {
    /// open documents by URI (the LSP's full-text sync)
    docs: HashMap<String, String>,
    /// the std surface + workspace files, in lookup order
    defs: Vec<DefIndex>,
    /// url → pin, from the workspace root's `rut.lock` (the host reads
    /// the file and hands the bytes to `set_lock`; the manifest
    /// grammar carries no pins). `parse_manifest` stamps the rows
    /// onto its url deps.
    lock_pins: BTreeMap<String, String>,
    /// the direct-dep module names — the open document's own package
    /// name plus every dep row its ROOT manifest declares (`deps`,
    /// `dev-deps` at the walk root). The host hands the
    /// set to `set_direct_deps` when it parses a root manifest;
    /// every dep index named here stamps `direct_dep`, the
    /// auto-import tier's gate (a transitive-only dep's `use` is the
    /// compiler's D5 error). Union-accumulating: multi-folder walks
    /// and re-walks re-add the same names, never shrink the world.
    direct_deps: HashSet<String>,
}

impl Core {
    /// the open document's file-module context, read off the STAMPED
    /// twin: whichever face indexed the file under its real uri (the
    /// extension's dep walk / workspace indexer) laid down the (module,
    /// mod_path) pair — the fresh per-query doc index borrows it.
    /// Unstamped files answer the default (position-path completion off;
    /// nothing else reads this).
    fn doc_mods_of(&self, uri: &str) -> crate::mods::DocMods {
        for d in &self.defs {
            if d.origin == uri {
                return crate::mods::DocMods {
                    mod_path: d.mod_path.clone(),
                    pkg: d.module.clone(),
                };
            }
        }
        crate::mods::DocMods::default()
    }

    /// the completion query over the stored doc — both completion
    /// entries' body (`complete_in` has already re-stored the text)
    fn complete_stored(&self, uri: &str, line: u32, ch: u32) -> String {
        let Some(src) = self.docs.get(uri).cloned() else {
            return "[]".to_string();
        };
        let doc = self.doc_mods_of(uri);
        let items = crate::analysis::complete_at(uri, &src, &self.defs, line, ch, &doc);
        json_envelope(serde_json::json!(items))
    }
}

/// the language core as the host holds it — one per editing session
#[wasm_bindgen]
pub struct RutLsp {
    core: RefCell<Core>,
}

#[wasm_bindgen]
impl RutLsp {
    /// the core boots with the std surface pre-indexed — the same
    /// the crate indexes at construction
    #[wasm_bindgen(constructor)]
    pub fn new() -> RutLsp {
        RutLsp {
            core: RefCell::new(Core {
                docs: HashMap::new(),
                defs: crate::std_surface::indexes(),
                lock_pins: BTreeMap::new(),
                direct_deps: HashSet::new(),
            }),
        }
    }

    /// the semantic-token legend (names in token-type index order)
    pub fn legend(&self) -> String {
        json_envelope(serde_json::json!(crate::semantic::TokenType::legend()))
    }

    /// open / FULL-sync change: store the document, return one analysis —
    /// `{diags, tokens, symbols}` (the values the LSP face would push over
    /// the wire). The diags are the MERGED set (B1+B2): the parse lane's
    /// diags plus the check lane's — the doc's package (its stamped twin
    /// names it) compiles with its dep closure and the checker's per-file
    /// diags map back through the file stamps. No package context →
    /// parse-only, exactly the pre-B1 answer.
    pub fn analyze(&self, uri: &str, src: &str) -> String {
        let mut core = self.core.borrow_mut();
        core.docs.insert(uri.to_string(), src.to_string());
        // analyze_at wires related-info URIs + hover provenance, exactly
        // wired like `analysis::analyze_at`; a degenerate URI still analyzes
        let a = match Uri::from_str(uri) {
            Ok(u) => crate::analysis::analyze_at(&u, src, crate::analysis::mode_of(uri)),
            Err(_) => {
                let mut a = crate::analysis::analyze(src, crate::analysis::mode_of(uri));
                a.index.origin = uri.to_string();
                a
            }
        };
        let diags = {
            let dm = core.doc_mods_of(uri);
            let groups = if dm.pkg.is_some() {
                crate::check::groups_from_defs(&core.defs)
            } else {
                Vec::new()
            };
            crate::check::merged_doc_diags(uri, src, dm.pkg, dm.mod_path, groups, a.diags)
        };
        json_envelope(wire::Analysis {
            diags,
            // SemanticTokens serializes `data` to the flat u32 wire format
            // (the same bytes a stdio face would put on the wire)
            tokens: ls_types::SemanticTokens {
                result_id: None,
                data: a.tokens,
            },
            symbols: a.symbols,
        })
    }

    /// close: drop the document (the workspace/std index stays)
    pub fn forget(&self, uri: &str) {
        self.core.borrow_mut().docs.remove(uri);
    }

    /// hover at an LSP position — the doc index first, then std +
    /// workspace; `null` when there's nothing to say
    pub fn hover(&self, uri: &str, line: u32, ch: u32) -> String {
        let core = self.core.borrow();
        let Some(src) = core.docs.get(uri).cloned() else {
            return "null".to_string();
        };
        let doc = core.doc_mods_of(uri);
        let out = crate::analysis::hover_at(uri, &src, &core.defs, &doc, line, ch);
        json_envelope(serde_json::json!(out))
    }

    /// completions at an LSP position (member after `.`, the use-path and
    /// qualified-position tiers) — the doc's file-module context rides
    /// from its stamped twin (see `doc_mods_of`)
    pub fn complete(&self, uri: &str, line: u32, ch: u32) -> String {
        self.core.borrow().complete_stored(uri, line, ch)
    }

    /// completions over the document's CURRENT text — the sync twin of
    /// `complete`, `analyze`'s uri+src shape: the text is stored FIRST,
    /// so the tiers lex the text the cursor sits in, dot included.
    /// The extension debounces `analyze` (~200 ms) while completion
    /// fires on the keystroke — the stored-only read raced that window
    /// and answered a stale tail (typing `log.` hit the bare keyword
    /// dump when the stored doc still ended at `log`).
    pub fn complete_in(&self, uri: &str, src: &str, line: u32, ch: u32) -> String {
        {
            let mut core = self.core.borrow_mut();
            core.docs.insert(uri.to_string(), src.to_string());
        }
        self.core.borrow().complete_stored(uri, line, ch)
    }

    /// definitions at an LSP position — the doc index first, then std +
    /// workspace; `null` when the doc is unknown, `[]` when nothing
    /// resolves. Targets carry the doc URI, a workspace origin, or the std
    /// surface's true `rut/...` source path (relative — the extension
    /// resolves it against the workspace). The doc's file-module context
    /// rides from its stamped twin (see `doc_mods_of`) — the mod-edge
    /// clicks and the own-package use lanes key on it.
    pub fn definition(&self, uri: &str, line: u32, ch: u32) -> String {
        let core = self.core.borrow();
        let Some(src) = core.docs.get(uri).cloned() else {
            return "null".to_string();
        };
        let doc = core.doc_mods_of(uri);
        let locs = crate::analysis::definition_at(uri, &src, &core.defs, &doc, line, ch);
        locations_envelope(&locs)
    }

    /// type definitions at an LSP position (an expression -> its type's
    /// declaration) — same shape as `definition`
    pub fn type_definition(&self, uri: &str, line: u32, ch: u32) -> String {
        let core = self.core.borrow();
        let Some(src) = core.docs.get(uri).cloned() else {
            return "null".to_string();
        };
        let doc = core.doc_mods_of(uri);
        let locs = crate::analysis::type_definition_at(uri, &src, &core.defs, &doc, line, ch);
        locations_envelope(&locs)
    }

    /// inlay hints for a document range — the inline inference display:
    /// TYPE hints on unannotated bindings + PARAMETER hints at exact-arity
    /// call sites. `null` when the doc is unknown, `[]` when nothing
    /// resolves. The hints are full LSP values (position + label + kind +
    /// tooltip); the client renders them.
    pub fn inlay(
        &self,
        uri: &str,
        start_line: u32,
        start_ch: u32,
        end_line: u32,
        end_ch: u32,
    ) -> String {
        let core = self.core.borrow();
        let Some(src) = core.docs.get(uri).cloned() else {
            return "null".to_string();
        };
        let hints = crate::analysis::inlay_hints_at(
            uri,
            &src,
            &core.defs,
            ls_types::Range {
                start: ls_types::Position {
                    line: start_line,
                    character: start_ch,
                },
                end: ls_types::Position {
                    line: end_line,
                    character: end_ch,
                },
            },
        );
        json_envelope(serde_json::json!(hints))
    }

    /// references — the definition index read BACKWARDS: every position
    /// whose go-to-definition lands on the declaration under `line`/`ch`.
    /// Within-file through the binding pass (shadow-aware), cross-file
    /// through the use graph's reverse edges. `include_decl` (0/1) follows
    /// the LSP toggle. `null` when the doc is unknown, `[]` when nothing
    /// resolves — the same `{uri, range}` Location JSON `definition`
    /// speaks.
    pub fn references(&self, uri: &str, line: u32, ch: u32, include_decl: u32) -> String {
        let core = self.core.borrow();
        let Some(src) = core.docs.get(uri).cloned() else {
            return "null".to_string();
        };
        let doc = core.doc_mods_of(uri);
        let locs = crate::analysis::references_at(
            uri,
            &src,
            &core.defs,
            &doc,
            line,
            ch,
            include_decl != 0,
        );
        locations_envelope(&locs)
    }

    /// signature help at an LSP position — the callee resolves through the
    /// same machinery the parameter-name hints ride (free calls by unique
    /// recorded params, methods through the receiver's type head), the
    /// signature renders verbatim, the active parameter comes from the
    /// comma/paren depth at the request position. Ambiguity or arity
    /// mismatch → `null` (never wrong help).
    pub fn signature_help(&self, uri: &str, line: u32, ch: u32) -> String {
        let core = self.core.borrow();
        let Some(src) = core.docs.get(uri).cloned() else {
            return "null".to_string();
        };
        let out = crate::analysis::signature_help_at(uri, &src, &core.defs, line, ch);
        json_envelope(serde_json::json!(out))
    }

    /// index one workspace file (the host's stand-in for an fs walk —
    /// server's fs walk — the 500-file cap is the caller's). Re-indexing
    /// a URI replaces its entry.
    pub fn add_def(&self, uri: &str, src: &str) {
        let idx = crate::analysis::index_at(uri, src);
        let mut core = self.core.borrow_mut();
        match core.defs.iter().position(|d| d.origin == uri) {
            Some(slot) => core.defs[slot] = idx,
            None => core.defs.push(idx),
        }
    }

    // ---- the deps face (crate::deps — one engine, no JS-side parsing) ----

    /// parse one `rut.jsonc` — the dep-table JSON out (the module's
    /// identity, its entry sources with the mode each indexes in, and
    /// its dep rows with the url rows' cache paths and lock-sourced
    /// pins). A malformed manifest is the error envelope — the host
    /// surfaces it as a hint.
    pub fn parse_manifest(&self, text: &str) -> String {
        let core = self.core.borrow();
        match crate::deps::dep_table_pinned(text, &core.lock_pins) {
            Ok(t) => json_envelope(wire::DepTable::from(&t)),
            Err(e) => json_envelope(wire::Error::new(e)),
        }
    }

    /// hand the dep-table face the workspace root's `rut.lock` bytes — the
    /// url→pin ledger `parse_manifest` stamps onto url rows (the lock is
    /// the pin's only source; the manifest grammar retired `sha256`).
    /// Answers `{"pins": N}`, or the `{"error": …}` envelope for a
    /// malformed lock — the host surfaces it as a hint, and the (empty)
    /// ledger stands so url rows answer `sha256: null` (the unpinned
    /// refusal), never guessed pins.
    pub fn set_lock(&self, text: &str) -> String {
        let mut core = self.core.borrow_mut();
        match crate::deps::lock_pins(text) {
            Ok(pins) => {
                let n = pins.len();
                core.lock_pins = pins;
                json_envelope(wire::Pins { pins: n })
            }
            Err(e) => {
                core.lock_pins = BTreeMap::new();
                json_envelope(wire::Error::new(e))
            }
        }
    }

    /// hand the dep face the direct-dep module names — the open
    /// document's own package name plus every dep row its ROOT manifest
    /// declares. Every dep index named here stamps `direct_dep` (the
    /// auto-import tier's gate): `add_def_named` / `add_def_mod`
    /// / `add_bundle` consult the set at index time, so the host
    /// calls this once per root manifest BEFORE mounting its rows. The
    /// text is a JSON array of names (`["pouch", "calc"]`); names
    /// ACCUMULATE (a set — multi-folder walks and re-walks re-add the
    /// same names). Answers `{"direct": N}` (the set size), or the error
    /// envelope for malformed JSON.
    pub fn set_direct_deps(&self, names_json: &str) -> String {
        let mut core = self.core.borrow_mut();
        match serde_json::from_str::<Vec<String>>(names_json) {
            Ok(names) => {
                for n in names {
                    core.direct_deps.insert(n);
                }
                json_envelope(wire::Direct {
                    direct: core.direct_deps.len(),
                })
            }
            Err(e) => json_envelope(wire::Error::new(e.to_string())),
        }
    }

    /// index one dep source file under its module NAME — `deps::index_dep`
    /// into the same store `add_def` writes (re-indexing an origin
    /// replaces its entry), so `use <name>::` resolves it like any index.
    /// The mode rides the URI convention (`mode_of`, identical to
    /// `add_def`); namespace/consts have no voice here — a dep
    /// needing them indexes through `add_bundle`, whose manifest is in
    /// the bytes. The direct-dep marker stamps from the host's set.
    pub fn add_def_named(&self, uri: &str, name: &str, src: &str) {
        let mut idx =
            crate::deps::index_dep(name, uri, src, crate::analysis::mode_of(uri), None, &[]);
        let mut core = self.core.borrow_mut();
        idx.direct_dep = core.direct_deps.contains(name);
        match core.defs.iter().position(|d| d.origin == uri) {
            Some(slot) => core.defs[slot] = idx,
            None => core.defs.push(idx),
        }
    }

    /// index one dep source file under its module NAME at a MOD PATH —
    /// the mod-aware twin of `add_def_named` (the dep walk's per-file
    /// move for a mod-carrying package: the root at `""`, each child at
    /// its path). Returns the parsed file's own `mod` declarations —
    /// `{ "decls": [{ "name", "vis" }] }` — so the host's mount walk
    /// reads its next hop from the wasm's parse (JS moves bytes, the
    /// module parses; no JS-side rut parsing).
    pub fn add_def_mod(&self, uri: &str, name: &str, mod_path: &str, src: &str) -> String {
        let mut idx = crate::deps::index_dep_mod(
            name,
            uri,
            mod_path,
            src,
            crate::analysis::mode_of(uri),
            None,
            &[],
        );
        let mut core = self.core.borrow_mut();
        idx.direct_dep = core.direct_deps.contains(name);
        let decls: Vec<wire::ModDecl> = idx
            .mods
            .iter()
            .map(|m| wire::ModDecl {
                name: m.name.clone(),
                vis: if matches!(m.vis, rut_ast::ast::Vis::Pub) {
                    "pub"
                } else {
                    "mod"
                },
            })
            .collect();
        match core.defs.iter().position(|d| d.origin == uri) {
            Some(slot) => core.defs[slot] = idx,
            None => core.defs.push(idx),
        }
        json_envelope(wire::ModDecls { decls })
    }

    /// unpack one `.rutbundle` — `deps::bundle_sources` reads the
    /// archive's own manifest, then each entry source indexes under the
    /// module NAME the bundle carries (origin `bundle:<module>/<path>`,
    /// replace-by-origin like every index). The envelope names the
    /// module + the entry paths indexed, or the error — a
    /// corrupt/foreign archive never lands bytes. The bytes param is
    /// `&[u8]`: the archive is a zip, not text.
    pub fn add_bundle(&self, bytes: &[u8]) -> String {
        let b = match crate::deps::bundle_sources(bytes) {
            Ok(b) => b,
            Err(e) => return json_envelope(serde_json::json!({ "error": e })),
        };
        let mut paths = Vec::with_capacity(b.files.len());
        let mut core = self.core.borrow_mut();
        let direct = core.direct_deps.contains(&b.module);
        for f in &b.files {
            let uri = format!("bundle:{}/{}", b.module, f.path);
            // a mounted mod child rides as `<path>/mod.rut` (a host tree's
            // as `<path>/mod.d.rut`, the rows lane's root as `mod.rut`) —
            // its mod path stamps the index in the file's own mode; flat
            // files keep today's shape. A DECL file is the declaration
            // entry (wire v12): ONE parse, the inline nesting stamps one
            // index per module — the consolidated group entry indexes whole.
            let idxs: Vec<_> = match crate::deps::bundle_file_mod_path(&f.path) {
                Some(p) if f.mode == rut_parser::Mode::Decl => crate::deps::index_dep_decl_group(
                    &b.module,
                    &uri,
                    &p,
                    &f.src,
                    b.namespace.as_deref(),
                    &b.consts,
                )
                .into_iter()
                .map(|(_, idx)| idx)
                .collect(),
                Some(p) => vec![crate::deps::index_dep_mod(
                    &b.module,
                    &uri,
                    &p,
                    &f.src,
                    f.mode,
                    b.namespace.as_deref(),
                    &b.consts,
                )],
                None => vec![crate::deps::index_dep(
                    &b.module,
                    &uri,
                    &f.src,
                    f.mode,
                    b.namespace.as_deref(),
                    &b.consts,
                )],
            };
            for mut idx in idxs {
                idx.direct_dep = direct;
                // one uri per module: the shared entry uri for the root
                // stamp, `#<mod path>` for the nested ones (the (module,
                // mod_path) pair is the real key; the uri only names the
                // origin in diagnostics)
                match core.defs.iter().position(|d| d.origin == uri) {
                    Some(slot) => core.defs[slot] = idx,
                    None => core.defs.push(idx),
                }
            }
            paths.push(f.path.clone());
        }
        json_envelope(wire::BundleIndex {
            module: b.module,
            namespace: b.namespace,
            consts: b.consts,
            files: paths,
        })
    }

    /// unpack one `.rutbundle` into the envelope WITHOUT indexing — the
    /// pure-extract twin of `add_bundle`: bytes in, the editable
    /// surface out as `{ module, namespace, consts, files: [{ path, src,
    /// mode, mod_path }] }`. JS owns the disk: the extension writes the
    /// files and re-indexes them through `add_def`/`add_def_mod`
    /// (real `file://` origins, the go-to-definition face), so this lane
    /// has NO index side effect. `mod_path` rides per file
    /// (`deps::bundle_file_mod_path` — the rows-lane root spells `""`, a
    /// mounted child its `<path>`; a flat file stays `null`), so the host
    /// never re-derives the mount rule in TypeScript. A corrupt/foreign
    /// archive is the error envelope, never bytes.
    pub fn extract_bundle(&self, bytes: &[u8]) -> String {
        let b = match crate::deps::bundle_sources(bytes) {
            Ok(b) => b,
            Err(e) => return json_envelope(serde_json::json!({ "error": e })),
        };
        let files: Vec<wire::BundleFile> =
            b.files.iter().map(wire::BundleFile::from_source).collect();
        json_envelope(wire::ExtractBundle {
            module: b.module,
            namespace: b.namespace,
            consts: b.consts,
            files,
        })
    }

    /// A SOURCE-ZIP cache entry's extract — the source transport's editor
    /// face: the zip's own package files (`src/**`) as the editable tree,
    /// each with its mod path derived from the `src/` shell (`src/mod.rut`
    /// → `""` the root, `src/a/mod.rut` → `"a"`, `src/a.rut` → `"a"`).
    /// The embedded `deps/<identity>/` trees belong to other pkgs and stay
    /// out (the consumer's own dep rows cover them).
    pub fn extract_srczip(&self, bytes: &[u8]) -> String {
        let b = match crate::deps::srczip_sources(bytes) {
            Ok(b) => b,
            Err(e) => return json_envelope(serde_json::json!({ "error": e })),
        };
        let files: Vec<wire::BundleFile> = b
            .files
            .iter()
            .map(|f| {
                let rel = f.path.strip_prefix("src/").unwrap_or(&f.path);
                let mod_path = if rel == "mod.rut" || rel == "mod.d.rut" {
                    Some(String::new())
                } else if let Some(p) = rel.strip_suffix("/mod.rut") {
                    Some(p.to_string())
                } else if let Some(p) = rel.strip_suffix("/mod.d.rut") {
                    Some(p.to_string())
                } else {
                    rel.strip_suffix(".rut").map(|p| p.to_string())
                };
                wire::BundleFile {
                    path: f.path.clone(),
                    src: f.src.clone(),
                    mode: wire::mode_str(f.mode),
                    mod_path,
                }
            })
            .collect();
        json_envelope(wire::ExtractSrczip {
            module: b.module,
            namespace: b.namespace,
            consts: b.consts,
            files,
            // the unpack law's ingredients: the archive's own manifest (the
            // host lays it beside the files and the pkg's own rows mount
            // through the ordinary dir lane) and the embedded
            // `deps/<identity>/` entries (the ride-along groups, laid under
            // the tree; first mount wins)
            manifest: b.manifest,
            embedded: b
                .embedded
                .iter()
                .map(|(path, src)| wire::Embedded {
                    path: path.clone(),
                    src: src.clone(),
                })
                .collect(),
        })
    }

    /// an open doc's stored text length — the smoke test's round-trip
    /// check
    pub fn doc_len(&self, uri: &str) -> u32 {
        self.core
            .borrow()
            .docs
            .get(uri)
            .map(|s| s.len() as u32)
            .unwrap_or(0)
    }
}

impl Default for RutLsp {
    fn default() -> Self {
        Self::new()
    }
}

// ---- the envelope writers (the answers are JSON strings) ----
//
// The field names live in `wire` — this module only serializes. The
// error envelope spells `{"error": …}`; every other answer is the
// struct `wire` declares, so the JS face's `.d.ts` has one mirror to
// keep true.

fn json_envelope<T: serde::Serialize>(value: T) -> String {
    match serde_json::to_string(&value) {
        Ok(json) => json,
        Err(_) => "null".to_string(), // unreachable for these types
    }
}

/// the definition-target envelope — `DefLocation` is serde-free in the
/// queries (they carry no serializer), so this module spells
/// the LSP `Location` JSON: `{ uri, range }`
fn locations_envelope(locs: &[crate::definition::DefLocation]) -> String {
    let value: Vec<wire::Location> = locs
        .iter()
        .map(|l| wire::Location {
            uri: l.uri.clone(),
            range: l.range,
        })
        .collect();
    json_envelope(value)
}
