//! rut-lsp-wasm — the language core as an in-process wasm module.
//! The VS Code extension instantiates this and registers its providers
//! directly — no server process, no per-platform binaries. The native
//! `rut-lsp` stdio server remains the face for editors that speak LSP;
//! both run the *same* queries from `rut_lsp::analysis`, so they can't
//! drift.
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
//! `rut_lsp::deps` (pure — bytes in, values out) into the SAME
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

use ls_types::Uri;
use rut_lsp::hover::DefIndex;
use wasm_bindgen::prelude::wasm_bindgen;

mod wire;

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
    fn doc_mods_of(&self, uri: &str) -> rut_lsp::mods::DocMods {
        for d in &self.defs {
            if d.origin == uri {
                return rut_lsp::mods::DocMods {
                    mod_path: d.mod_path.clone(),
                    pkg: d.module.clone(),
                };
            }
        }
        rut_lsp::mods::DocMods::default()
    }

    /// the completion query over the stored doc — both completion
    /// entries' body (`complete_in` has already re-stored the text)
    fn complete_stored(&self, uri: &str, line: u32, ch: u32) -> String {
        let Some(src) = self.docs.get(uri).cloned() else {
            return "[]".to_string();
        };
        let doc = self.doc_mods_of(uri);
        let items = rut_lsp::analysis::complete_at(uri, &src, &self.defs, line, ch, &doc);
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
    /// indexes the native server embeds
    #[wasm_bindgen(constructor)]
    pub fn new() -> RutLsp {
        RutLsp {
            core: RefCell::new(Core {
                docs: HashMap::new(),
                defs: rut_lsp::std_surface::indexes(),
                lock_pins: BTreeMap::new(),
                direct_deps: HashSet::new(),
            }),
        }
    }

    /// the semantic-token legend (names in token-type index order)
    pub fn legend(&self) -> String {
        json_envelope(serde_json::json!(rut_lsp::semantic::TokenType::legend()))
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
        // like the stdio server; a degenerate URI still analyzes
        let a = match Uri::from_str(uri) {
            Ok(u) => rut_lsp::analysis::analyze_at(&u, src, rut_lsp::analysis::mode_of(uri)),
            Err(_) => {
                let mut a = rut_lsp::analysis::analyze(src, rut_lsp::analysis::mode_of(uri));
                a.index.origin = uri.to_string();
                a
            }
        };
        let diags = {
            let dm = core.doc_mods_of(uri);
            let groups = if dm.pkg.is_some() {
                rut_lsp::check::groups_from_defs(&core.defs)
            } else {
                Vec::new()
            };
            rut_lsp::check::merged_doc_diags(uri, src, dm.pkg, dm.mod_path, groups, a.diags)
        };
        json_envelope(wire::Analysis {
            diags,
            // SemanticTokens serializes `data` to the flat u32 wire format
            // (the same bytes the stdio server puts on the wire)
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
        let out = rut_lsp::analysis::hover_at(uri, &src, &core.defs, &doc, line, ch);
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
        let locs = rut_lsp::analysis::definition_at(uri, &src, &core.defs, &doc, line, ch);
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
        let locs = rut_lsp::analysis::type_definition_at(uri, &src, &core.defs, &doc, line, ch);
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
        let hints = rut_lsp::analysis::inlay_hints_at(
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
        let locs = rut_lsp::analysis::references_at(
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
        let out = rut_lsp::analysis::signature_help_at(uri, &src, &core.defs, line, ch);
        json_envelope(serde_json::json!(out))
    }

    /// index one workspace file (the host's stand-in for the native
    /// server's fs walk — the 500-file cap is the caller's). Re-indexing
    /// a URI replaces its entry.
    pub fn add_def(&self, uri: &str, src: &str) {
        let idx = rut_lsp::analysis::index_at(uri, src);
        let mut core = self.core.borrow_mut();
        match core.defs.iter().position(|d| d.origin == uri) {
            Some(slot) => core.defs[slot] = idx,
            None => core.defs.push(idx),
        }
    }

    // ---- the deps face (rut_lsp::deps — one engine, two faces) ----

    /// parse one `rut.jsonc` — the dep-table JSON out (the module's
    /// identity, its entry sources with the mode each indexes in, and
    /// its dep rows with the url rows' cache paths and lock-sourced
    /// pins). A malformed manifest is the error envelope — the host
    /// surfaces it as a hint.
    pub fn parse_manifest(&self, text: &str) -> String {
        let core = self.core.borrow();
        match rut_lsp::deps::dep_table_pinned(text, &core.lock_pins) {
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
        match rut_lsp::deps::lock_pins(text) {
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
            rut_lsp::deps::index_dep(name, uri, src, rut_lsp::analysis::mode_of(uri), None, &[]);
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
        let mut idx = rut_lsp::deps::index_dep_mod(
            name,
            uri,
            mod_path,
            src,
            rut_lsp::analysis::mode_of(uri),
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
        let b = match rut_lsp::deps::bundle_sources(bytes) {
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
            let idxs: Vec<_> = match rut_lsp::deps::bundle_file_mod_path(&f.path) {
                Some(p) if f.mode == rut_parser::Mode::Decl => rut_lsp::deps::index_dep_decl_group(
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
                Some(p) => vec![rut_lsp::deps::index_dep_mod(
                    &b.module,
                    &uri,
                    &p,
                    &f.src,
                    f.mode,
                    b.namespace.as_deref(),
                    &b.consts,
                )],
                None => vec![rut_lsp::deps::index_dep(
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
        let b = match rut_lsp::deps::bundle_sources(bytes) {
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
        let b = match rut_lsp::deps::srczip_sources(bytes) {
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

/// the definition-target envelope — `DefLocation` is serde-free in
/// `rut-lsp` (the pure core carries no serializer), so the shim spells
/// the LSP `Location` JSON: `{ uri, range }`
fn locations_envelope(locs: &[rut_lsp::definition::DefLocation]) -> String {
    let value: Vec<wire::Location> = locs
        .iter()
        .map(|l| wire::Location {
            uri: l.uri.clone(),
            range: l.range,
        })
        .collect();
    json_envelope(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POUCH_BUNDLE: &[u8] = include_bytes!("../../../dist/std/pouch.rutbundle");
    /// the CDN url's own sha256 — the cache filename rut-lsp computes
    /// (the URL STRING is hashed, never the bytes)
    const POUCH_CACHE: &str =
        ".rut/cache/02bcf87de17b5c095f2c1a768f20f2506a10b9a56a1290ffe4241536e672dbe0.rutbundle";

    /// parse an export's JSON answer
    fn take(json: String) -> serde_json::Value {
        serde_json::from_str(&json).expect("the envelope is valid JSON")
    }

    /// The plugin manifest's dep shape, INLINE and unpinned: the real
    /// `examples/03-plugin/plugin/rut.jsonc` migrates off the retired
    /// manifest `sha256` key in the docs/examples phase — the grammar
    /// refuses the key today, so the host-side fixture is the shape
    /// every manifest must now spell (pins ride `set_lock`).
    const PLUGIN_SHAPE: &str = r#"{
  "format": "rutbundle",
  "format_version": 11,
  "name": "plugin",
  "deps": {
    "server": { "path": "../server" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle" }
  }
}"#;

    const POUCH_URL: &str =
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle";
    const POUCH_PIN: &str = "63ddd765287243ef45f23105471ff3eae5c5a0ac1d521a3d22636745fd5e72d8";

    /// hand a lock spelling `rows` to `set_lock`
    fn set_lock(lsp: &RutLsp, rows: Vec<rut_driver::LockRow>) -> serde_json::Value {
        let text = rut_driver::render_lock(&rut_driver::LockFile::from_rows(rows));
        take(lsp.set_lock(&text))
    }

    #[test]
    fn manifest_envelope_spells_the_dep_table() {
        let lsp = RutLsp::new();
        // the lock WITHOUT a pouch row stands first: the url row answers
        // `sha256: null` — the unpinned refusal, never a guessed pin
        let out = set_lock(&lsp, vec![]);
        assert_eq!(out["pins"], 0, "{out}");
        let out = take(lsp.parse_manifest(PLUGIN_SHAPE));
        assert_eq!(out["name"], "plugin");
        // the body rides the src/mod.rut convention (the entry keys are repealed)
        assert_eq!(
            out["entries"],
            serde_json::json!([{ "path": "src/mod.rut", "mode": "Impl" }]),
            "the entry carries its mode: {}",
            out["entries"]
        );
        let rows = out["deps"].as_array().unwrap();
        let server = rows.iter().find(|r| r["name"] == "server").unwrap();
        assert_eq!(
            server["source"],
            serde_json::json!({ "kind": "path", "dir": "../server" })
        );
        assert_eq!(server["optional"], false);
        let pouch = rows.iter().find(|r| r["name"] == "pouch").unwrap();
        assert_eq!(pouch["source"]["kind"], "url");
        assert_eq!(pouch["source"]["cache_path"], POUCH_CACHE);
        assert!(
            pouch["source"]["sha256"].is_null(),
            "no lock row ⇒ unpinned: {}",
            pouch["source"]
        );

        // the lock's row stamps the pin — the JSON keeps the `sha256`
        // field, now LOCK-SOURCED (the manifest itself stays unpinned)
        let out = set_lock(
            &lsp,
            vec![rut_driver::LockRow {
                pkg: "pouch".into(),
                url: POUCH_URL.into(),
                sha256: POUCH_PIN.into(),
            }],
        );
        assert_eq!(out["pins"], 1, "{out}");
        let out = take(lsp.parse_manifest(PLUGIN_SHAPE));
        let pouch = out["deps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "pouch")
            .unwrap();
        assert_eq!(
            pouch["source"]["sha256"], POUCH_PIN,
            "the pin is lock-sourced"
        );
        assert_eq!(pouch["source"]["cache_path"], POUCH_CACHE);
    }

    #[test]
    fn a_malformed_lock_is_the_error_envelope_and_stands_empty() {
        let lsp = RutLsp::new();
        let bad = take(lsp.set_lock(
            "{\"format\": \"rutlock\", \"format_version\": 1, \"deps\": [{\"pkg\": \"p\", \"url\": \"https://x/p.rutbundle\", \"sha256\": \"nothex\"}]}",
        ));
        assert!(
            bad["error"].as_str().unwrap().contains("64 hex digits"),
            "{bad}"
        );
        // the empty ledger stands: the url row answers null (the host
        // shows the unpinned refusal), never half-parsed pins
        let out = take(lsp.parse_manifest(PLUGIN_SHAPE));
        let pouch = out["deps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "pouch")
            .unwrap();
        assert!(pouch["source"]["sha256"].is_null(), "{}", pouch["source"]);
    }

    #[test]
    fn a_decl_entry_and_the_error_envelope() {
        let lsp = RutLsp::new();
        let src = r#"{ "name": "host", "type": "host", "entry": { "type": "./surface.d.rut" } }"#;
        let out = take(lsp.parse_manifest(src));
        assert_eq!(
            out["entries"],
            serde_json::json!([{ "path": "src/surface.d.rut", "mode": "Decl" }]),
            "entry.type indexes in Decl mode"
        );
        // a malformed manifest is the error envelope — the host's hint
        let bad = take(lsp.parse_manifest("{\"name\": six}"));
        assert!(
            bad["error"].as_str().unwrap().starts_with("line 1: "),
            "{bad}"
        );
    }

    #[test]
    fn complete_in_answers_locals_and_members_at_the_end_of_buffer() {
        let lsp = RutLsp::new();
        // plan3 phase 1 (A7): the doc ends EXACTLY at the incomplete
        // stmt — the parser ends the block's span at the EOF token, the
        // cursor sits ON `hi`, and before the scope extension the member
        // tier dropped the receiver (an empty list). The fn body's scope
        // extends through the source end, so the member tier answers
        // through the doc-sync entry — never the bare keyword dump.
        let doc = "class List {\n    items: i32;\n}\nimpl List {\n    fn count(self) -> i32 { return 0; }\n}\nfn go() -> nil {\n    let list = List.new();\n    list.";
        let uri = "file:///ws/eof-member.rut";
        let last = doc.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = doc[..last].matches('\n').count() as u32;
        let ch = (doc.len() - last) as u32;
        let items = take(lsp.complete_in(uri, doc, line, ch));
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        for want in ["items", "count"] {
            assert!(
                labels.contains(&want),
                "member completion at EOF lacks '{want}': {labels:?}"
            );
        }
        assert!(
            !labels.contains(&"fn"),
            "no keyword dump after a dot at EOF: {labels:?}"
        );

        // the locals tier (A1) at the same moment: `che⏐` after
        // `let mut checked …` — the binding rides the variable kind
        let doc = "fn go() -> nil {\n    let mut checked: str = \" \";\n    che";
        let uri = "file:///ws/eof-locals.rut";
        let last = doc.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = doc[..last].matches('\n').count() as u32;
        let ch = (doc.len() - last) as u32;
        let items = take(lsp.complete_in(uri, doc, line, ch));
        let arr = items.as_array().unwrap();
        let labels: Vec<&str> = arr.iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert!(
            labels.contains(&"checked"),
            "the local completes at EOF: {labels:?}"
        );
        let checked = arr.iter().find(|i| i["label"] == "checked").unwrap();
        assert_eq!(checked["kind"], 6, "LSP VARIABLE kind: {}", checked["kind"]);
        // ahead of the keyword table
        let checked_at = arr.iter().position(|i| i["label"] == "checked").unwrap();
        let let_at = arr.iter().position(|i| i["label"] == "let").unwrap();
        assert!(checked_at < let_at, "locals ride ahead of the keywords");
    }

    #[test]
    fn complete_in_answers_the_type_tier_at_an_annotation() {
        let lsp = RutLsp::new();
        // A3 through the bindgen face: `let v: u⏐` — the primitive table
        // rides the prefix, the keyword table / fns / locals stay off
        let doc = "fn go() -> nil {\n    let v: u\n}\n";
        let uri = "file:///ws/type-tier.rut";
        let items = take(lsp.complete_in(uri, doc, 1, 12));
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        for want in ["u8", "u16", "u32", "u64"] {
            assert!(
                labels.contains(&want),
                "primitive `{want}` completes at the annotation: {labels:?}"
            );
        }
        for absent in ["fn", "let", "go"] {
            assert!(
                !labels.contains(&absent),
                "`{absent}` must stay off a type position: {labels:?}"
            );
        }
        // the bare colon boundary — the full tier, no keywords
        let doc = "fn go() -> nil {\n    let v: \n}\n";
        let items = take(lsp.complete_in(uri, doc, 1, 11));
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(
            labels.contains(&"i32") && labels.contains(&"str"),
            "the bare colon tier: {labels:?}"
        );
        assert!(
            !labels.contains(&"fn"),
            "no keyword table at the boundary: {labels:?}"
        );
    }

    #[test]
    fn the_bare_scope_gates_a_dep_until_used() {
        let lsp = RutLsp::new();
        // A2 through the bindgen face: a named dep's fn is NOT a plain
        // bare item until the doc `use`s it — the only bare-doc offer
        // rides the import edit, and the direct-dep set gates the tier
        // a root manifest declaring ONLY the doc's own pkg — kitdep is
        // mounted but NOT direct (the transitive-only shape)
        take(lsp.set_direct_deps("[\"myapp\"]"));
        lsp.add_def_named(
            "file:///ws/kitdep/lib.rut",
            "kitdep",
            "pub fn tag() -> i32 {\n    return 1;\n}\n",
        );
        let doc = "fn main() -> nil {\n}\n";
        let uri = "file:///ws/bare.rut";
        take(lsp.analyze(uri, doc));
        let items = take(lsp.complete(uri, 1, 0));
        let arr = items.as_array().unwrap();
        assert!(
            arr.iter().all(|i| i["label"] != "tag"),
            "a transitive-only dep's fn must not offer at all: {:?}",
            arr.iter().map(|i| &i["label"]).collect::<Vec<_>>()
        );

        // the direct-dep set gains kitdep (the root manifest's row):
        // the bare doc offers `tag` ONLY through the import edit …
        take(lsp.set_direct_deps("[\"myapp\", \"kitdep\"]"));
        lsp.add_def_named(
            "file:///ws/kitdep/lib.rut",
            "kitdep",
            "pub fn tag() -> i32 {\n    return 1;\n}\n",
        );
        let items = take(lsp.complete(uri, 1, 0));
        let arr = items.as_array().unwrap();
        let imp = arr
            .iter()
            .find(|i| i["label"] == "tag")
            .expect("the direct dep's fn offers");
        assert!(
            imp["additionalTextEdits"]
                .as_array()
                .is_some_and(|e| !e.is_empty()),
            "the offer carries the use-insert edit: {imp}"
        );

        // … and once the doc `use`s it, `tag` is a PLAIN bare item —
        // exactly one, with no edit riding
        let doc = "use kitdep::{ tag };\nfn main() -> nil {\n}\n";
        let uri = "file:///ws/bare-used.rut";
        take(lsp.analyze(uri, doc));
        let items = take(lsp.complete(uri, 1, 0));
        let arr = items.as_array().unwrap();
        let hits: Vec<&serde_json::Value> = arr.iter().filter(|i| i["label"] == "tag").collect();
        assert_eq!(hits.len(), 1, "one `tag` item: {hits:?}");
        assert!(
            hits[0]["additionalTextEdits"].is_null(),
            "the imported fn completes plain: {}",
            hits[0]
        );
    }

    #[test]
    fn definition_on_the_mod_edge_names_the_child_file_top() {
        let lsp = RutLsp::new();
        // the plan-C lane through the bindgen face: the root file is
        // BOTH a stamped index (its uri carries the (module, mod_path)
        // pair `doc_mods_of` reads) and the analyzed doc; the `helpers`
        // ident in `pub mod helpers;` jumps to the mounted child at 0:0
        let root = "pub mod helpers;\nentry fn main() -> nil {\n}\n";
        let child = "pub struct Slot {\n    n: i32;\n}\n";
        let ru = "file:///ws/edgemod/mod.rut";
        let cu = "file:///ws/edgemod/helpers/mod.rut";
        take(lsp.add_def_mod(ru, "edgemod", "", root));
        take(lsp.add_def_mod(cu, "edgemod", "helpers", child));
        take(lsp.analyze(ru, root));
        // the edge ident starts at char 8 (`pub mod `)
        let locs = take(lsp.definition(ru, 0, 8));
        let arr = locs.as_array().unwrap();
        assert_eq!(arr.len(), 1, "exactly one target: {locs}");
        assert!(
            arr[0]["uri"]
                .as_str()
                .unwrap()
                .ends_with("/edgemod/helpers/mod.rut"),
            "the child uri: {locs}"
        );
        assert_eq!(
            arr[0]["range"],
            serde_json::json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}),
            "the child file top: {locs}"
        );

        // an UNSTAMPED doc with the same text misses (the doc context
        // keys the lane — the scratch uri carries no stamp)
        let scratch = "file:///ws/edgemod-scratch.rut";
        take(lsp.analyze(scratch, root));
        let none = take(lsp.definition(scratch, 0, 8));
        assert!(
            none.as_array().unwrap().is_empty(),
            "unstamped miss: {none}"
        );
    }

    #[test]
    fn add_def_named_feeds_the_use_path_tier() {
        let lsp = RutLsp::new();
        // a DIFFERENT module than the mod-tree fixture below — each test
        // owns its own instance, but the fixture stays distinct so the
        // two tests' envelopes never read as one pkg
        // a dep source under its module NAME
        lsp.add_def_named(
            "file:///ws/toolkit/lib.rut",
            "toolkit",
            "pub class Widget {\n    id: i32;\n}\npub fn tag() -> i32 {\n    return 1;\n}\n",
        );
        let doc = "use toolkit::\nfn main() -> nil {\n}\n";
        let uri = "file:///ws/main.rut";
        take(lsp.analyze(uri, doc));
        // `use toolkit::⏐` — the dep's public names
        let items = take(lsp.complete(uri, 0, 13));
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(
            labels.contains(&"Widget"),
            "the dep type completes: {labels:?}"
        );
        assert!(labels.contains(&"tag"), "the dep fn completes: {labels:?}");
        // `use ⏐` — the module-name tier sees the named index
        let mods = take(lsp.complete(uri, 0, 4));
        let names: Vec<&str> = mods
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"toolkit"),
            "the module name completes: {names:?}"
        );
    }

    #[test]
    fn complete_in_syncs_the_doc_and_the_dot_guard_holds() {
        let lsp = RutLsp::new();
        // ctor inference rides the embedded ink surface: `Logger("h")`
        // types the binding, `log.` lists the impl methods. The stored
        // doc is STALE on purpose (the tail still short the dot — the
        // analyze debounce's window): the `_in` entry stores the
        // CURRENT text first, so the member tier still answers.
        let stale = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log\n}\n";
        let fresh = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log.\n}\n";
        let uri = "file:///ws/sync.rut";
        take(lsp.analyze(uri, stale));
        let items = take(lsp.complete_in(uri, fresh, 2, 8));
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        for want in ["new", "debug", "info", "warn", "error"] {
            assert!(
                labels.contains(&want),
                "`{want}` completes through the ctor binding: {labels:?}"
            );
        }
        // the bare tier's host fns never dump into a member position
        assert!(
            !labels.contains(&"abs"),
            "no host-fn dump after a dot: {labels:?}"
        );

        // the dot guard: a receiver that is not a name (`probe().` — the
        // token before the dot is a `)`) answers EMPTY, never the bare
        // keyword/host-fn dump
        let guarded = "fn go() -> nil {\n    let x = probe().\n}\n";
        let none = take(lsp.complete_in(uri, guarded, 1, 20));
        assert!(none.as_array().unwrap().is_empty(), "{none}");
    }

    #[test]
    fn add_bundle_indexes_the_pouch_surface() {
        let lsp = RutLsp::new();
        let out = take(lsp.add_bundle(POUCH_BUNDLE));
        assert_eq!(out["module"], "pouch");
        assert_eq!(out["files"], serde_json::json!(["mod.d.rut"]));
        // a corrupt archive is the error envelope, never silent state
        let err = take(lsp.add_bundle(b"definitely not a zip"));
        assert!(
            err["error"].as_str().unwrap().contains("not a zip"),
            "{err}"
        );

        // the indexed surface resolves: the use name jumps to BOTH pouch
        // indexes — the std surface's true source path AND the bundle's
        let doc = "use pouch::{ Vec };\nfn main() -> nil {\n    let v: Vec<i32> = Vec::new();\n    v.push(1);\n}\n";
        let uri = "file:///ws/uses-pouch.rut";
        take(lsp.analyze(uri, doc));
        let locs = take(lsp.definition(uri, 0, 13)); // the use-line `Vec`
        let uris: Vec<&str> = locs
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["uri"].as_str().unwrap())
            .collect();
        assert!(
            uris.contains(&"bundle:pouch/mod.d.rut"),
            "the bundle index answers go-to-definition: {uris:?}"
        );
        // and the member surface hovers through the named module
        let push_ch = doc.lines().nth(3).unwrap().find("push").unwrap() as u32;
        let h = take(lsp.hover(uri, 3, push_ch));
        assert!(
            h["contents"]["value"].as_str().unwrap().contains("push"),
            "{h}"
        );
    }

    /// plan3 phase 4 (B1+B2): the CHECK LANE through the bindgen face —
    /// a doc whose package is STAMPED (`add_def_mod` named it) compiles
    /// with its dep closure, and the checker's type errors surface with
    /// the right file and range: the bool→str assignment mismatch AND
    /// the missing record field, the diagnosis pair. An UNSTAMPED doc
    /// with the same text answers parse-only clean — today's behavior,
    /// the corpus lock's foundation.
    #[test]
    fn analyze_runs_the_checker_over_a_stamped_chain() {
        let lsp = RutLsp::new();
        // the dep: one mod-tree file mounted under its module NAME
        let dep_src = "pub struct Task {\n    id: u32;\n    title: str;\n    completed: bool;\n}\n";
        let dep_uri = "file:///ws/chk/dep/mod.rut";
        let stamped = take(lsp.add_def_mod(dep_uri, "chkdep", "", dep_src));
        assert!(
            stamped["decls"].as_array().unwrap().is_empty(),
            "{}",
            stamped
        );
        // the doc's own pkg root, stamped the same way the extension's
        // workspace indexer stamps it
        let doc = "use chkdep::{ Task };\nfn main() -> nil {\n    let mut checked: str = \" \";\n    checked = true;\n    let t = Task { title: \"x\", completed: false };\n}\n";
        let app_uri = "file:///ws/chk/app/mod.rut";
        let stamped = take(lsp.add_def_mod(app_uri, "chkapp", "", doc));
        assert!(
            stamped["decls"].as_array().unwrap().is_empty(),
            "{}",
            stamped
        );

        // the analyze lane returns the MERGED diags: both checker sites,
        // `source: "rut"`, each on its own line
        let a = take(lsp.analyze(app_uri, doc));
        let diags = a["diags"].as_array().unwrap();
        let mismatch = diags
            .iter()
            .find(|d| {
                d["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("assignment type mismatch")
            })
            .unwrap_or_else(|| panic!("no assignment-mismatch diag: {a}"));
        let missing = diags
            .iter()
            .find(|d| {
                d["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("`id` is missing")
            })
            .unwrap_or_else(|| panic!("no missing-field diag: {a}"));
        for d in [mismatch, missing] {
            assert_eq!(d["source"], "rut", "{d}");
        }
        assert_eq!(mismatch["range"]["start"]["line"], 3, "{mismatch}");
        assert_eq!(missing["range"]["start"]["line"], 4, "{missing}");

        // the parse-only fallback: the SAME text at an UNSTAMPED uri is
        // clean (no package graph assembles for it)
        let plain_uri = "file:///ws/chk/plain.rut";
        let plain = take(lsp.analyze(plain_uri, doc));
        assert!(
            plain["diags"].as_array().unwrap().is_empty(),
            "an unstamped doc must stay parse-only: {plain}"
        );
    }

    #[test]
    fn add_def_mod_stamps_the_tree_and_feeds_the_use_walk() {
        let lsp = RutLsp::new();
        // the root: `pub mod layout;` rides back as the mount walk's
        // next hop (the host reads its children from the envelope)
        let root_src = "pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n";
        let uri_root = "file:///ws/gadgets/mod.rut";
        let out = take(lsp.add_def_mod(uri_root, "gadgets", "", root_src));
        assert_eq!(
            out["decls"],
            serde_json::json!([{ "name": "layout", "vis": "pub" }, { "name": "secret", "vis": "mod" }]),
            "{}",
            out["decls"]
        );
        // the child, stamped at its mod path
        let layout_src =
            "pub struct Column {\n    w: i32;\n}\npub fn mk() -> Column { return Column { w: 1 }; }\n";
        let uri_layout = "file:///ws/gadgets/layout/mod.rut";
        let out = take(lsp.add_def_mod(uri_layout, "gadgets", "layout", layout_src));
        assert!(
            out["decls"].as_array().unwrap().is_empty(),
            "{}",
            out["decls"]
        );

        // `use gadgets::layout::⏐` — the walked module's exports (the
        // private `secret` edge walked would be empty)
        let doc = "use gadgets::layout::\nfn main() -> nil {\n}\n";
        let uri = "file:///ws/main.rut";
        take(lsp.analyze(uri, doc));
        let items = take(lsp.complete(uri, 0, 21));
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"Column"), "{labels:?}");
        assert!(labels.contains(&"mk"), "{labels:?}");
        // the root tier offers the pub mod child, never the private one
        let items = take(lsp.complete(uri, 0, 13));
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"layout"), "{labels:?}");
        assert!(
            !labels.contains(&"secret"),
            "a bare `mod` edge never crosses: {labels:?}"
        );
        assert!(
            !labels.contains(&"Column"),
            "child members stay under their path: {labels:?}"
        );
    }

    /// a mod-carrying bundle in the pack writer's shape — the wasm
    /// face mounts the rows tree additively
    fn kit_bundle() -> Vec<u8> {
        use std::collections::BTreeMap;
        let mods = BTreeMap::from([(
            "layout".to_string(),
            rut_driver::ModSource {
                path: "layout".into(),
                vis: rut_ast::ast::Vis::Pub,
                text: "pub struct Column {\n    w: i32;\n}\n".into(),
                file: "mod.rut".into(),
            },
        )]);
        let rows = rut_driver::mods::rows_json("pub mod layout;\n", &mods);
        rut_driver::bundle::write_bundle(&[
            (
                rut_driver::bundle::files::MANIFEST_NAME.into(),
                br#"{ "name": "kit" }"#.to_vec(),
            ),
            (rut_driver::mods::ROWS_NAME.into(), rows.into_bytes()),
        ])
        .unwrap()
    }

    #[test]
    fn add_bundle_mounts_a_mod_tree() {
        let lsp = RutLsp::new();
        let bytes = kit_bundle();
        let out = take(lsp.add_bundle(&bytes));
        assert_eq!(out["module"], "kit");
        assert_eq!(
            out["files"],
            serde_json::json!(["mod.rut", "layout/mod.rut"]),
            "the tree rides the file list: {}",
            out["files"]
        );
        // `use kit::layout::⏐` — the mounted child's exports
        let doc = "use kit::layout::{ Column };\nfn main() -> nil {\n    let c: Column = Column { w: 1 };\n}\n";
        let uri = "file:///ws/uses-kit.rut";
        take(lsp.analyze(uri, doc));
        let items = take(lsp.complete(uri, 0, 16));
        let labels: Vec<&str> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"Column"), "{labels:?}");
        // and the segment hover resolves through the dep tree
        let h = take(lsp.hover(uri, 0, 12)); // the `layout` segment
        let md = h["contents"]["value"].as_str().unwrap();
        assert!(md.contains("file module"), "{md}");
        assert!(md.contains("layout/mod.rut"), "{md}");
    }

    /// the extract lane — `add_bundle`'s unpack WITHOUT the index
    /// write: the file list mirrors `bundle_sources` byte-for-byte,
    /// every file spells its own mod path (the host never re-derives
    /// the mount rule), and the defs store ends the call untouched
    #[test]
    fn extract_bundle_names_the_surface_and_never_indexes() {
        let lsp = RutLsp::new();
        let origins_before: std::collections::BTreeSet<String> = lsp
            .core
            .borrow()
            .defs
            .iter()
            .map(|d| d.origin.clone())
            .collect();
        let out = take(lsp.extract_bundle(POUCH_BUNDLE));
        assert_eq!(out["module"], "pouch");
        assert!(out["namespace"].is_null());
        // the file list mirrors `bundle_sources`' own unpack
        let b = rut_lsp::deps::bundle_sources(POUCH_BUNDLE).expect("the pouch bundle unpacks");
        let paths: Vec<&str> = out["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        let want: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, want, "the envelope's files ride bundle_sources'");
        // the generated declaration entry rides, mode + mod path spelled
        // (wire v12 — the entry IS the surface the extension writes back)
        let f = &out["files"][0];
        assert_eq!(f["path"], "mod.d.rut");
        assert_eq!(f["mode"], "Decl");
        assert_eq!(f["mod_path"], "");
        assert!(
            f["src"].as_str().unwrap().contains("class Vec"),
            "the pouch entry rides"
        );
        // NO indexing side effect — the defs store ends where it started
        let origins_after: std::collections::BTreeSet<String> = lsp
            .core
            .borrow()
            .defs
            .iter()
            .map(|d| d.origin.clone())
            .collect();
        assert_eq!(
            origins_before, origins_after,
            "the extract lane never writes the index"
        );

        // a mod tree spells each file's mount path (the root `""`, each
        // child its `<path>`); a corrupt archive stays the error
        // envelope, never bytes
        let bytes = kit_bundle();
        let out = take(lsp.extract_bundle(&bytes));
        let spells: Vec<(&str, &str)> = out["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["path"].as_str().unwrap(), f["mod_path"].as_str().unwrap()))
            .collect();
        assert_eq!(spells, vec![("mod.rut", ""), ("layout/mod.rut", "layout")]);
        let junk = b"definitely not a zip";
        let err = take(lsp.extract_bundle(junk));
        assert!(
            err["error"].as_str().unwrap().contains("not a zip"),
            "{err}"
        );
    }
}
