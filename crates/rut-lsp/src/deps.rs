//! Deps — the manifest/bundle bridge between a rut project's
//! `rut.jsonc` files and the definition index. Everything here is PURE
//! (text/bytes in, values out): the faces (wasm shim, stdio server)
//! move the bytes, this module reads them — one engine, two faces, no
//! JS-side rut parsing.
//!
//! Three entries:
//! - [`dep_table`] parses one manifest into a [`DepTable`]: the
//!   module's own identity (`name`, entry sources with the mode each
//!   indexes in, `namespace`, `consts`) plus its dep rows (`deps`,
//!   `dev-deps`, `peer-deps`), each a `path` directory or a `url`
//!   bundle with the `.rut/cache/` cache path computed here. The pin
//!   is NOT the manifest's business — [`lock_pins`] reads `rut.lock`
//!   (the walk root's ledger) and [`dep_table_pinned`] stamps its rows
//!   onto the url rows.
//! - [`bundle_sources`] unpacks a `.rutbundle` into the module's OWN
//!   source files — the zip's own `rut.jsonc` names the module; the
//!   rode-along dep groups are the loader's business, not the
//!   editor's.
//! - [`index_dep`] indexes one dep source file: [`analysis::index_at`]
//!   plus `module = Some(name)`, with `namespace`/`consts` minted onto
//!   the index (a namespace-head type row, module-scope lets for the
//!   consts — the `calc`/`Math::PI` shape: the engine binds
//!   `use calc::{Math}` then routes `Math::sqrt`/`Math::PI` as qualified
//!   access, and the member machinery resolves both through the minted
//!   row).

use rut_ast::ast::Vis;
use rut_driver::bundle::files::{bundle_key, read_entry, MANIFEST_NAME};
use rut_driver::bundle::manifest::{parse_manifest, parse_manifest_compat};
use rut_driver::bundle::{parse_bundle, Manifest};
use rut_lexer::span::Span;
use rut_parser::Mode;
use sha2::{Digest, Sha256};

use crate::hover::types::{MemberSrc, TyDef, TyForm};
use crate::hover::DefIndex;

/// One entry source a dep indexes: the manifest-relative file path and
/// the mode it parses in (`entry.type` → Decl, `entry.lib`/`libs` →
/// Impl).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySource {
    pub path: String,
    pub mode: Mode,
}

/// Where a dep row's bytes live: a directory beside the manifest, or a
/// remote `.rutbundle` pinned by its sha256 (the pin lock-sourced —
/// `rut.lock` at the dep-walk root, never the manifest). The cache
/// path is computed HERE (`.rut/cache/<sha256(url)>.rutbundle`) — the
/// faces never hash URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepSource {
    Path { dir: String },
    Url { url: String, sha256: Option<String>, cache_path: String },
}

/// One dep row (`deps` / `dev-deps` / `peer-deps`). `optional` and
/// `lib` are the peer descriptor's bits (`optional = true` marks the
/// presence-mounted kind whose integration group is `lib`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepRow {
    pub name: String,
    pub source: DepSource,
    pub optional: bool,
    pub lib: Option<String>,
}

/// One parsed manifest, editor-shaped: the module's own identity plus
/// its dep rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DepTable {
    /// the manifest's `name` — what a `use` path spells
    pub name: Option<String>,
    /// the entry sources, in indexing order: the decl surface first
    /// (`entry.type`, Decl mode — or the default root `mod.d.rut` when
    /// a host pkg spells none), then the body — the root module
    /// `mod.rut` (Impl mode; the convention, not a key). An OLD
    /// published bundle's manifest (parsed through
    /// [`dep_table_compat`]) carries the repealed keys instead, mapped
    /// exactly as the loader's reader-compat law does.
    pub entries: Vec<EntrySource>,
    /// the qualified-access head (`calc`'s `Math`)
    pub namespace: Option<String>,
    /// the compiler-materialized constants (`Math::PI`), name → f64
    pub consts: Vec<(String, f64)>,
    /// `type = "host"` — the entry surface is a pure `.d.rut` and its
    /// `mod` tree mounts the Decl flavor (`NAME/mod.d.rut` children,
    /// Decl mode) instead of a lib body's `mod.rut` tree
    pub host: bool,
    /// `deps` — transitively mounted
    pub deps: Vec<DepRow>,
    /// `dev-deps` — mounted only for the pkg's own build/tests
    pub dev_deps: Vec<DepRow>,
    /// `peer-deps` — required by default, `optional` marks otherwise
    pub peer_deps: Vec<DepRow>,
}

/// The cache path for a url dep — `.rut/cache/<sha256(url)>.rutbundle`,
/// relative to the workspace root both faces share with the CLI.
pub fn cache_path(url: &str) -> String {
    let hex: String = Sha256::digest(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    format!(".rut/cache/{hex}.rutbundle")
}

/// One dep row from a parsed descriptor map. `deps` rows may be path-
/// or url-sourced; `peer-deps`/`dev-deps` descriptors are path-only by
/// grammar (a url there refused at parse) — the url arm simply never
/// fires for them. Url rows parse UNPINNED: the manifest grammar
/// retired `sha256` (an old bundle manifest's pin rides as inert
/// metadata the compat lane ignores, exactly like the loader does) —
/// the pin arrives from `rut.lock` via [`dep_table_pinned`].
fn dep_row(name: &str, desc: &std::collections::BTreeMap<String, String>) -> DepRow {
    let source = match (desc.get("path"), desc.get("url")) {
        (_, Some(url)) => DepSource::Url {
            url: url.clone(),
            sha256: None,
            cache_path: cache_path(url),
        },
        (Some(dir), _) => DepSource::Path { dir: dir.clone() },
        _ => DepSource::Path { dir: String::new() },
    };
    DepRow {
        name: name.to_string(),
        source,
        optional: desc.get("optional").map(|o| o == "true").unwrap_or(false),
        lib: desc.get("lib").cloned(),
    }
}

/// Parse one `rut.jsonc` (JSONC: comments + trailing commas legal)
/// into the editor's dep table. A malformed manifest is the error —
/// the faces surface it as a hint, never silent bytes. STRICT: the
/// repealed `entry.lib`/`entry.libs` keys refuse, and the body rides
/// the `mod.rut` convention. The bundle face parses OLD published
/// manifests through [`dep_table_compat`].
pub fn dep_table(manifest_text: &str) -> Result<DepTable, String> {
    let m: Manifest = parse_manifest(manifest_text).map_err(|e| e.to_string())?;
    dep_table_of(m)
}

/// The url → pin map one `rut.lock` spells — the pin ledger the faces
/// read at the dep-walk root and thread through [`dep_table_pinned`].
/// The lock is the pin's ONLY source (the manifest grammar retired
/// `sha256`); the map is keyed by url because that is the identity the
/// CLI's install writes rows under (one lock pins the whole closure,
/// nested path-dep url rows included). Malformed text is the error —
/// the caller (the server walk) surfaces it as a hint, never silent
/// pins.
pub fn lock_pins(
    lock_text: &str,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let lock = rut_driver::parse_lock(lock_text).map_err(|e| e.to_string())?;
    Ok(lock
        .deps()
        .iter()
        .map(|r| (r.url.clone(), r.sha256.clone()))
        .collect())
}

/// [`dep_table`] with the dep-walk root's `rut.lock` applied: every url
/// row the lock pins answers `sha256: Some(pin)`; a url with no lock
/// row stays `None` — the walk refuses it with the install recipe
/// (the editor's mirror of the loader's `Unpinned` law).
pub fn dep_table_pinned(
    manifest_text: &str,
    pins: &std::collections::BTreeMap<String, String>,
) -> Result<DepTable, String> {
    let mut t = dep_table(manifest_text)?;
    for row in t
        .deps
        .iter_mut()
        .chain(t.dev_deps.iter_mut())
        .chain(t.peer_deps.iter_mut())
    {
        if let DepSource::Url { url, sha256, .. } = &mut row.source {
            *sha256 = pins.get(url).cloned();
        }
    }
    Ok(t)
}

/// [`dep_table`] over the bundle reader's compat lane: an archive's
/// own manifest may be OLD (a published bundle is forever old — the
/// pinned CDN tags), so the repealed keys map onto Impl entries
/// exactly as the loader's reader mounts them (the lib, then the
/// `libs` splice, in listed order), REPLACING the `mod.rut`
/// convention row an old archive cannot answer.
pub fn dep_table_compat(manifest_text: &str) -> Result<DepTable, String> {
    let m: Manifest = parse_manifest_compat(manifest_text).map_err(|e| e.to_string())?;
    let mut t = dep_table_of(m.clone())?;
    if m.legacy_entry.lib.is_some() || !m.legacy_entry.libs.is_empty() {
        t.entries.retain(|e| e.mode != Mode::Impl);
        if let Some(lib) = &m.legacy_entry.lib {
            t.entries.push(EntrySource { path: bundle_key(lib)?, mode: Mode::Impl });
        }
        for lib in &m.legacy_entry.libs {
            t.entries.push(EntrySource { path: bundle_key(lib)?, mode: Mode::Impl });
        }
    }
    Ok(t)
}

fn dep_table_of(m: Manifest) -> Result<DepTable, String> {
    let mut entries = Vec::new();
    // the decl surface indexes in Decl mode; the body in Impl mode —
    // the same split `analysis::mode_of` speaks for open documents.
    // On a host pkg with no spelled surface, `mod.d.rut` beside the
    // manifest IS the root (the loader's default-root discovery
    // mirrored) — an unreadable row simply indexes nothing.
    match &m.entry.type_path {
        Some(ty) => entries.push(EntrySource { path: bundle_key(ty)?, mode: Mode::Decl }),
        None if m.pkg_type == rut_driver::bundle::PkgType::Host => {
            entries.push(EntrySource {
                path: rut_driver::decl::DECL_CHILD_FILE.to_string(),
                mode: Mode::Decl,
            });
        }
        None => {}
    }
    // the body is the ROOT MODULE — `mod.rut` beside the manifest, the
    // convention every lane reads. A surface-only dev state (or a host
    // pkg) names no body: a Lib pkg with an entry.type spelled has its
    // mod.rut probed by the callers, and an unreadable row simply
    // indexes nothing (what exists is what mounts).
    if m.pkg_type == rut_driver::bundle::PkgType::Lib {
        entries.push(EntrySource { path: "mod.rut".to_string(), mode: Mode::Impl });
    }
    let rows = |table: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>| {
        table.iter().map(|(name, desc)| dep_row(name, desc)).collect()
    };
    Ok(DepTable {
        name: m.name,
        entries,
        namespace: m.namespace,
        consts: m.consts.into_iter().collect(),
        host: m.pkg_type == rut_driver::bundle::PkgType::Host,
        deps: rows(&m.deps),
        dev_deps: rows(&m.dev_deps),
        peer_deps: rows(&m.peer_deps),
    })
}

/// One source file unpacked from a `.rutbundle`: archive-relative
/// path, text, and the mode it indexes in (the manifest's entry key
/// decided — the authoritative spelling, not the `.d.rut` convention).
/// A mounted mod child rides as `<mod path>/mod.rut`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFile {
    pub path: String,
    pub src: String,
    pub mode: Mode,
}

/// A `.rutbundle`'s editable surface: the module's own name (the zip's
/// own `rut.jsonc` spells it) and its own source files. Compiled-only
/// bundles (no ridden source) yield zero files — the compiled groups
/// are the loader's truth, never guessed back into text.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleIndex {
    pub module: String,
    pub namespace: Option<String>,
    pub consts: Vec<(String, f64)>,
    pub files: Vec<BundleFile>,
}

/// The archive's own `rut.mods` rows, when the bundle carries the
/// additive envelope section: the root text (`""`) mounted with its
/// child tree through rut-driver's rows codec (the same declaration-
/// driven mount a directory runs). `Ok(None)` = no rows entry — the
/// old flat envelope, which keeps loading byte-for-byte. The flavor
/// rides the pkg: a host tree speaks Decl and mounts `mod.d.rut`
/// children, a lib body's tree the Impl `mod.rut` shape.
fn bundle_rows(
    entries: &[(String, Vec<u8>)],
    host: bool,
) -> Result<Option<(String, std::collections::BTreeMap<String, rut_driver::ModSource>)>, String> {
    let text = match read_entry(entries, rut_driver::mods::ROWS_NAME) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let (root, rows) = rut_driver::mods::parse_rows(&text)?;
    let (mode, child_file) = if host {
        (Mode::Decl, rut_driver::decl::DECL_CHILD_FILE)
    } else {
        (Mode::Impl, "mod.rut")
    };
    let mods = rut_driver::mods::mount_rows(&root, &rows, mode, child_file)?;
    Ok(Some((root, mods)))
}

/// The archive-entries child lookup for the source lane's mod mount —
/// the loader's `entry_child_lookup` three-case law mirrored over the
/// editor's decoded entries: `NAME/mod.rut` present, a same-named
/// FILE entry (`NAME.rut`), a `NAME/` prefix without its `mod.rut`,
/// or nothing. Keys are bundle-normalized exactly like the loader's.
fn entry_child_lookup(
    entries: &[(String, Vec<u8>)],
) -> impl FnMut(&str, &str) -> Result<rut_driver::ChildLookup, String> + '_ {
    move |parent: &str, name: &str| {
        let base = if parent.is_empty() {
            String::new()
        } else {
            format!("{parent}/")
        };
        let mod_key = bundle_key(&format!("{base}{name}/mod.rut"))?;
        if let Ok(text) = read_entry(entries, &mod_key) {
            return Ok(rut_driver::ChildLookup::Found { text, canon: mod_key });
        }
        if read_entry(entries, &bundle_key(&format!("{base}{name}.rut"))?).is_ok() {
            return Ok(rut_driver::ChildLookup::NotADir);
        }
        if entries.iter().any(|(k, _)| k.starts_with(&format!("{base}{name}/"))) {
            return Ok(rut_driver::ChildLookup::NoModRut);
        }
        Ok(rut_driver::ChildLookup::Missing)
    }
}

/// Unpack a `.rutbundle`'s OWN sources: CRC-verified entries, the
/// archive's `rut.jsonc` naming the module, then the root module —
/// the `mod.rut` entry beside the manifest (an OLD published bundle
/// rides at its legacy `entry.lib` spelling, mapped by
/// [`dep_table_compat`] — the loader's reader-compat law mirrored) —
/// plus the `rut.mods` rows entry when the bundle carries
/// one (the additive envelope section: the tree as path-keyed rows),
/// else the file-entry mount the loader's source lane runs. Rode-along
/// dep groups (`<pkg>/…` prefixes) never ride — they are the loader's
/// business. A missing entry file (a compiled-only bundle whose source
/// did not ride) is skipped, not an error: the editor indexes what
/// exists. A declared child whose file is missing is the loud mount
/// error — declarations are the graph.
pub fn bundle_sources(bytes: &[u8]) -> Result<BundleIndex, String> {
    let entries = parse_bundle(bytes).map_err(|e| e.to_string())?;
    let manifest_text = read_entry(&entries, MANIFEST_NAME)
        .map_err(|_| format!("no `{MANIFEST_NAME}` entry — not a rut bundle"))?;
    let table = dep_table_compat(&manifest_text)?;
    let module = table
        .name
        .clone()
        .ok_or_else(|| format!("`{MANIFEST_NAME}` has no `name`"))?;
    let mut files = Vec::new();
    // the decl surface first (entry.type — Decl mode), verbatim
    if let Some(e) = table.entries.iter().find(|e| e.mode == Mode::Decl) {
        if let Ok(src) = read_entry(&entries, &e.path) {
            files.push(BundleFile { path: e.path.clone(), src, mode: e.mode });
        }
    }
    // the tree: rows entry first (the additive envelope — the pack
    // writer's shape), else the file-entry mount over the root module.
    // The HOST flavor's root already rode the decl-surface push above
    // (the entry.type surface) — only its children join here, each a
    // Decl file at `<mod path>/mod.d.rut`; a lib tree's root re-rides
    // as `mod.rut` beside its Impl children.
    let tree: Option<(Option<BundleFile>, Vec<BundleFile>)> = match bundle_rows(&entries, table.host)? {
        Some((root_text, mods)) => {
            let (child_file, mode) = if table.host {
                (rut_driver::decl::DECL_CHILD_FILE, Mode::Decl)
            } else {
                ("mod.rut", Mode::Impl)
            };
            let children = mods
                .into_iter()
                .map(|(path, m)| BundleFile {
                    path: format!("{path}/{child_file}"),
                    src: m.text,
                    mode,
                })
                .collect();
            if table.host {
                Some((None, children))
            } else {
                Some((
                    Some(BundleFile { path: "mod.rut".to_string(), src: root_text, mode: Mode::Impl }),
                    children,
                ))
            }
        }
        None if table.host => None, // a host surface carries no lib body
        None => {
            // the source lane: root = the `mod.rut` entry (the
            // post-repeal convention), or the OLD envelope's legacy
            // lib — dep_table_compat mapped it onto the Impl entries
            let root = match table.entries.iter().find(|e| e.mode == Mode::Impl) {
                Some(e) => match read_entry(&entries, &e.path) {
                    Ok(src) => Some(BundleFile { path: e.path.clone(), src, mode: e.mode }),
                    Err(_) => None,
                },
                None => match read_entry(&entries, "mod.rut") {
                    Ok(src) => Some(BundleFile { path: "mod.rut".to_string(), src, mode: Mode::Impl }),
                    Err(_) => None,
                },
            };
            match root {
                Some(root) => {
                    let mounted = rut_driver::mount_mod_children(
                        &root.src,
                        &format!("\u{0}{module}"),
                        Mode::Impl,
                        "mod.rut",
                        &mut entry_child_lookup(&entries),
                    )?;
                    let children = mounted
                        .into_iter()
                        .map(|(path, m)| BundleFile {
                            path: format!("{path}/mod.rut"),
                            src: m.text,
                            mode: Mode::Impl,
                        })
                        .collect();
                    Some((Some(root), children))
                }
                None => None,
            }
        }
    };
    if let Some((root, children)) = tree {
        if let Some(root) = root {
            files.push(root);
        }
        files.extend(children);
    } else {
        // no body rode (a compiled-only or surface-only bundle) — the
        // remaining entry files still index, exactly as before
        for e in &table.entries {
            if e.mode != Mode::Impl {
                continue;
            }
            if let Ok(src) = read_entry(&entries, &e.path) {
                files.push(BundleFile { path: e.path.clone(), src, mode: e.mode });
            }
        }
    }
    Ok(BundleIndex { module, namespace: table.namespace, consts: table.consts, files })
}

/// a bundle file's mod path: the rows lane's root rides as `mod.rut`
/// (`Some("")`), a mounted child as `<path>/mod.rut` (a host tree's
/// as `<path>/mod.d.rut`), the host default root as `mod.d.rut`
/// (`Some("")` — the twin of the lib root's spelling) — `Some(path)`;
/// anything else is today's flat shape (`None`) — shared by both faces
/// (the stdio walk and the wasm shim index identically)
pub fn bundle_file_mod_path(path: &str) -> Option<String> {
    if let Some(p) = path.strip_suffix("/mod.rut") {
        return Some(p.to_string());
    }
    if let Some(p) = path.strip_suffix(format!("/{}", rut_driver::decl::DECL_CHILD_FILE).as_str()) {
        return Some(p.to_string());
    }
    if path == "mod.rut" || path == rut_driver::decl::DECL_CHILD_FILE {
        return Some(String::new());
    }
    None
}

/// Index one dep source file: the ordinary [`analysis::index_at`] pass
/// over `src` (origin stamped with `uri`, mode per the manifest), the
/// module NAMED, and the manifest's `namespace`/`consts` minted on —
/// a `TyForm::Namespace` type row for the head (the module's free fns
/// as methods, the consts as fields: the qualified `Math::sqrt`/`Math::PI`
/// shape resolves through the member machinery), module-scope lets for
/// the consts (the bare `PI` form the compiler binds as an extern
/// const once the pkg is used).
pub fn index_dep(
    name: &str,
    uri: &str,
    src: &str,
    mode: Mode,
    namespace: Option<&str>,
    consts: &[(String, f64)],
) -> DefIndex {
    // the ordinary index pass — but the MODE comes from the manifest's
    // entry key (the authoritative spelling), not the `.d.rut`
    // filename convention `analysis::mode_of` speaks for open documents
    let normalized = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&normalized);
    let (ast, _) = rut_parser::parse(&normalized, mode);
    let mut idx = crate::hover::index(&normalized, &ast, &toks);
    idx.origin = uri.to_string();
    idx.module = Some(name.to_string());
    // the parse mode rides the index — the definition layer's
    // body-over-decl preference reads it (the manifest's entry key is
    // the authoritative spelling, not the `.d.rut` filename convention
    // `analysis::mode_of` speaks for open documents)
    idx.mode = Some(mode);
    if let Some(head) = namespace {
        // the module's free fns become the head's methods — the
        // qualified-access surface the engine routes by the bound
        // namespace
        let methods: Vec<MemberSrc> = idx
            .fns
            .iter()
            .filter(|f| f.owner.is_none())
            .map(|f| MemberSrc {
                name: f.name.clone(),
                src: f.src.clone(),
                ty: f.ret.clone(),
                name_span: f.name_span,
                doc: f.doc.clone(),
                line: f.line,
            })
            .collect();
        let fields: Vec<MemberSrc> = consts
            .iter()
            .map(|(n, v)| MemberSrc {
                name: n.clone(),
                src: format!("{n} = {:?}", v),
                ty: Some("f64".to_string()),
                name_span: None,
                doc: vec!["manifest const — compiler-materialized".to_string()],
                line: 1,
            })
            .collect();
        idx.types.push(TyDef {
            name: head.to_string(),
            name_span: None,
            form: TyForm::Namespace,
            vis: Vis::Pub,
            is_pub: true,
            ambient: false,
            generics: Vec::new(),
            fields,
            methods,
            alias_target: None,
            doc: vec![format!(
                "the module's namespace head — `use {}::{{ {head} }}` binds it; members are reached qualified",
                name
            )],
            span: Span::new(0, 0),
            line: 1,
        });
    }
    for (n, v) in consts {
        idx.lets.push(crate::hover::LetDef {
            name: n.clone(),
            name_span: None,
            ty: Some("f64".to_string()),
            src: format!("let {n} = {:?}", v),
            doc: vec!["manifest const — compiler-materialized".to_string()],
            vis: Vis::Pub,
            span: Span::new(0, 0),
            line: 1,
        });
    }
    idx
}

/// [`index_dep`] with the file's mod path stamped — one mounted
/// module of a mod-carrying dep (the dep walk / bundle loader call
/// this per file; `mod_path` `""` is the pkg root's `mod.rut`).
pub fn index_dep_mod(
    name: &str,
    uri: &str,
    mod_path: &str,
    src: &str,
    mode: Mode,
    namespace: Option<&str>,
    consts: &[(String, f64)],
) -> DefIndex {
    let mut idx = index_dep(name, uri, src, mode, namespace, consts);
    idx.mod_path = Some(mod_path.to_string());
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALC_MANIFEST: &str = include_str!("../../../rut/calc/rut.jsonc");
    const POUCH_BUNDLE: &[u8] = include_bytes!("../../../dist/std/pouch.rutbundle");

    /// The plugin manifest's dep shape, INLINE and unpinned: the real
    /// `examples/03-plugin/plugin/rut.jsonc` migrates off the retired
    /// manifest `sha256` key in the docs/examples phase — the grammar
    /// refuses the key today, so the editor's fixture is the shape
    /// every manifest must now spell (pins live in rut.lock).
    const PLUGIN_SHAPE: &str = r#"{
  "format": "rutbundle",
  "format_version": 10,
  "name": "plugin",
  "deps": {
    "server": { "path": "../server" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/pouch.rutbundle" }
  }
}"#;

    const POUCH_URL: &str = "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/pouch.rutbundle";
    const POUCH_PIN: &str = "1bbd861905b6cc09435833d18122f0740be14b7b4bc306821634fb88d38e6f83";

    #[test]
    fn plugin_manifest_rows_path_and_url() {
        // unpinned in the manifest — the pin is the lock's row
        let t = dep_table(PLUGIN_SHAPE).expect("the plugin-shape manifest parses");
        assert_eq!(t.name.as_deref(), Some("plugin"));
        // the entry: one lib source, Impl mode
        // the body rides the mod.rut convention (the entry keys are repealed)
        assert_eq!(
            t.entries,
            vec![EntrySource { path: "mod.rut".into(), mode: Mode::Impl }]
        );
        // deps: the path row …
        let server = t.deps.iter().find(|r| r.name == "server").expect("server row");
        assert_eq!(server.source, DepSource::Path { dir: "../server".into() });
        assert!(!server.optional);
        assert!(server.lib.is_none());
        // … and the url row: UNPINNED at parse (sha256 retired from the
        // grammar), the cache path computed here
        let pouch = t.deps.iter().find(|r| r.name == "pouch").expect("pouch row");
        match &pouch.source {
            DepSource::Url { url, sha256, cache_path } => {
                assert_eq!(url, POUCH_URL);
                assert_eq!(sha256, &None, "the manifest is never the pin's source");
                assert_eq!(
                    cache_path,
                    &format!(".rut/cache/{}.rutbundle", {
                        use sha2::{Digest, Sha256};
                        let hex: String = Sha256::digest(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
                        hex
                    })
                );
            }
            other => panic!("pouch must be a url row: {other:?}"),
        }
        assert!(t.dev_deps.is_empty() && t.peer_deps.is_empty());
    }

    #[test]
    fn the_lock_pins_the_url_rows_and_a_missing_row_stays_none() {
        // dep_table_pinned: the walk-root rut.lock's rows stamp the url
        // rows (the manifest's own grammar cannot); a url the lock does
        // not name stays None — the walk's install-recipe refusal
        let pins = lock_pins(&rut_driver::render_lock(&rut_driver::LockFile::from_rows(
            vec![rut_driver::LockRow { pkg: "pouch".into(), url: POUCH_URL.into(), sha256: POUCH_PIN.into() }],
        )))
        .expect("the lock parses");
        let t = dep_table_pinned(PLUGIN_SHAPE, &pins).expect("parses");
        let pouch = t.deps.iter().find(|r| r.name == "pouch").expect("pouch row");
        assert_eq!(
            pouch.source,
            DepSource::Url {
                url: POUCH_URL.into(),
                sha256: Some(POUCH_PIN.into()),
                cache_path: cache_path(POUCH_URL),
            },
            "the pin is lock-sourced"
        );
        // a url the lock has no row for: None — never a guessed pin
        let t = dep_table_pinned(PLUGIN_SHAPE, &std::collections::BTreeMap::new()).expect("parses");
        let pouch = t.deps.iter().find(|r| r.name == "pouch").expect("pouch row");
        assert!(
            matches!(&pouch.source, DepSource::Url { sha256: None, .. }),
            "no lock row ⇒ unpinned: {:?}",
            pouch.source
        );
    }

    #[test]
    fn a_malformed_lock_is_the_error_not_silent_pins() {
        let err = lock_pins(r#"{"format": "rutlock", "format_version": 1, "deps": [{"pkg": "p", "url": "https://x/p.rutbundle", "sha256": "nothex"}]}"#)
            .unwrap_err();
        assert!(err.contains("64 hex digits"), "{err}");
    }

    #[test]
    fn calc_manifest_carries_namespace_and_consts() {
        let t = dep_table(CALC_MANIFEST).expect("the real calc manifest parses");
        assert_eq!(t.name.as_deref(), Some("calc"));
        // the host surface: decl mode, no body
        assert_eq!(t.entries, vec![EntrySource { path: "calc.d.rut".into(), mode: Mode::Decl }]);
        assert_eq!(t.namespace.as_deref(), Some("Math"));
        let pi = t.consts.iter().find(|(n, _)| n == "PI").expect("PI in consts");
        assert_eq!(pi.1, 3.141592653589793);
        // the non-finite spelling rides as the f64 it names
        let inf = t.consts.iter().find(|(n, _)| n == "INFINITY").expect("INFINITY in consts");
        assert!(inf.1.is_infinite() && inf.1.is_sign_positive());    }

    #[test]
    fn peer_descriptor_bits_ride() {
        let t = dep_table(
            r#"{
  "name": "json",
  "peer-deps": {
    "pouch": { "path": "../pouch", "optional": true, "lib": "./serde_pouch.rut" }
  },
  "dev-deps": { "nmapset": { "path": "../nmapset" } }
}"#,
        )
        .expect("peer/dev tables parse");
        let pouch = t.peer_deps.iter().find(|r| r.name == "pouch").expect("pouch peer");
        assert!(pouch.optional);
        assert_eq!(pouch.lib.as_deref(), Some("./serde_pouch.rut"));
        assert_eq!(pouch.source, DepSource::Path { dir: "../pouch".into() });
        let nmapset = t.dev_deps.iter().find(|r| r.name == "nmapset").expect("dev row");
        assert!(!nmapset.optional);
        assert!(nmapset.lib.is_none());
    }

    #[test]
    fn cache_path_hashes_the_url_not_the_bytes() {
        // the filename is sha256 of the URL STRING — deterministic,
        // computable before any fetch
        assert_eq!(
            cache_path("https://example.com/pouch.rutbundle"),
            ".rut/cache/455bf83475bf92ac5309d7daba5c66889231960d0a71c97600608b1d3b595034.rutbundle"
        );
    }

    #[test]
    fn a_bad_manifest_is_the_error() {
        let err = dep_table("{\"name\": six}").unwrap_err();
        assert!(err.starts_with("line 1: "), "{err}");
    }

    #[test]
    fn pouch_bundle_unpacks_its_own_source() {
        let b = bundle_sources(POUCH_BUNDLE).expect("the committed pouch bundle parses");
        assert_eq!(b.module, "pouch");
        assert_eq!(b.namespace, None);
        assert!(b.consts.is_empty());
        // the root module — the ONLY own source; `.rutc` and the scope
        // ledger never ride, neither do dep groups
        assert_eq!(b.files.len(), 1, "files: {:?}", b.files.iter().map(|f| &f.path));
        assert_eq!(b.files[0].path, "mod.rut");
        assert_eq!(b.files[0].mode, Mode::Impl);
        assert!(b.files[0].src.contains("class Vec"), "the real pouch source rides");
    }

    #[test]
    fn not_a_bundle_is_the_error() {
        let err = bundle_sources(b"definitely not a zip").unwrap_err();
        assert!(err.contains("not a zip"), "{err}");
    }

    #[test]
    fn index_dep_names_the_module_and_mints_the_namespace() {
        let src = "\
pub host fn sqrt(x: f64) -> f64;
";
        let consts = vec![("PI".to_string(), 3.141592653589793f64)];
        let idx = index_dep("calc", "file:///deps/calc.d.rut", src, Mode::Decl, Some("Math"), &consts);
        // the explicit name rides the index …
        assert_eq!(idx.module.as_deref(), Some("calc"));
        assert_eq!(idx.origin, "file:///deps/calc.d.rut");
        // … the parse mode rides too (the definition layer's
        // body-over-decl preference reads it) …
        assert_eq!(idx.mode, Some(Mode::Decl));
        // … the surface indexes …
        assert!(idx.fns.iter().any(|f| f.name == "sqrt" && f.owner.is_none()));
        // … the namespace head mints as a type row carrying the fns as
        // methods and the consts as fields — the Math::sqrt/Math::PI shape
        let math = idx.ty("Math").expect("the namespace head mints");
        assert_eq!(math.form, TyForm::Namespace);
        assert!(math.is_pub);
        assert!(math.methods.iter().any(|m| m.name == "sqrt"));
        let pi = math.fields.iter().find(|m| m.name == "PI").expect("PI as a field");
        assert_eq!(pi.ty.as_deref(), Some("f64"));
        // … and the consts mint module-scope lets (the bare-`PI` form)
        let pi = idx.lets.iter().find(|l| l.name == "PI").expect("PI as a module let");
        assert_eq!(pi.ty.as_deref(), Some("f64"));
        assert!(pi.src.starts_with("let PI = 3.141592653589793"));
    }

    #[test]
    fn index_dep_without_namespace_mints_nothing() {
        let idx = index_dep("pouch", "file:///deps/pouch.rut", "class Vec<T> {\n}\n", Mode::Impl, None, &[]);
        assert_eq!(idx.module.as_deref(), Some("pouch"));
        assert!(idx.ty("Math").is_none());
        assert!(idx.lets.is_empty());
    }

    #[test]
    fn index_dep_mod_stamps_the_mod_path() {
        let idx = index_dep_mod(
            "gadgets",
            "file:///deps/gadgets/layout/mod.rut",
            "layout",
            "pub struct Column {\n    w: i32;\n}\n",
            Mode::Impl,
            None,
            &[],
        );
        assert_eq!(idx.module.as_deref(), Some("gadgets"));
        assert_eq!(idx.mod_path.as_deref(), Some("layout"));
        // the recorded mod edges ride the parsed rows too
        let root = index_dep_mod("gadgets", "file:///deps/gadgets/mod.rut", "", "pub mod layout;\n", Mode::Impl, None, &[]);
        assert_eq!(root.mod_path.as_deref(), Some(""));
        assert_eq!(root.mods.len(), 1);
        assert_eq!(root.mods[0].name, "layout");
    }

    /// a mod-carrying bundle in the pack writer's shape: manifest + the
    /// additive `rut.mods` rows entry (the tree as path-keyed rows)
    fn kit_rows_bundle() -> Vec<u8> {
        use std::collections::BTreeMap;
        let mods = BTreeMap::from([
            (
                "layout".to_string(),
                rut_driver::ModSource {
                    path: "layout".into(),
                    vis: rut_ast::ast::Vis::Pub,
                    text: "pub mod grid;\npub struct Column {\n    w: i32;\n}\n".into(),
                },
            ),
            (
                "layout/grid".to_string(),
                rut_driver::ModSource {
                    path: "layout/grid".into(),
                    vis: rut_ast::ast::Vis::Pub,
                    text: "pub struct Cell {\n    x: i32;\n}\n".into(),
                },
            ),
        ]);
        let rows = rut_driver::mods::rows_json("pub mod layout;\npub fn tag() -> i32 { return 1; }\n", &mods);
        rut_driver::bundle::write_bundle(&[
            (MANIFEST_NAME.into(), br#"{ "name": "kit" }"#.to_vec()),
            (rut_driver::mods::ROWS_NAME.into(), rows.into_bytes()),
        ])
        .unwrap()
    }

    #[test]
    fn a_mod_bundle_unpacks_the_rows_tree() {
        let bytes = kit_rows_bundle();
        let b = bundle_sources(&bytes).expect("the kit bundle unpacks");
        assert_eq!(b.module, "kit");
        assert_eq!(b.namespace, None);
        // the root + each child, path-keyed
        let paths: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["mod.rut", "layout/mod.rut", "layout/grid/mod.rut"]);
        assert!(b.files[0].src.contains("pub mod layout;"));
        assert!(b.files[1].src.contains("struct Column"));
        assert!(b.files[2].src.contains("struct Cell"));
        assert!(b.files.iter().all(|f| f.mode == Mode::Impl));
    }

    #[test]
    fn a_source_lane_bundle_mounts_the_file_entries() {
        // no rows entry — the loader's source lane: the `mod.rut` entry
        // beside the manifest is the root, its decls mount the children
        let bytes = rut_driver::bundle::write_bundle(&[
            (MANIFEST_NAME.into(), br#"{ "name": "kit" }"#.to_vec()),
            ("mod.rut".into(), b"pub mod layout;\n".to_vec()),
            ("layout/mod.rut".into(), b"pub fn span() -> i32 { return 1; }\n".to_vec()),
        ])
        .unwrap();
        let b = bundle_sources(&bytes).expect("the source-lane bundle unpacks");
        let paths: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["mod.rut", "layout/mod.rut"]);
        assert!(b.files[1].src.contains("fn span"));
    }

    #[test]
    fn bundle_file_mod_paths_spell_the_tree() {
        assert_eq!(bundle_file_mod_path("mod.rut").as_deref(), Some(""));
        assert_eq!(bundle_file_mod_path("layout/mod.rut").as_deref(), Some("layout"));
        assert_eq!(bundle_file_mod_path("layout/grid/mod.rut").as_deref(), Some("layout/grid"));
        // the host tree's Decl children spell the same mod paths, and
        // the bare `mod.d.rut` IS the host root — `Some("")`, the twin
        // of the lib root's spelling
        assert_eq!(bundle_file_mod_path("mod.d.rut").as_deref(), Some(""));
        assert_eq!(bundle_file_mod_path("net/mod.d.rut").as_deref(), Some("net"));
        assert_eq!(bundle_file_mod_path("net/tls/mod.d.rut").as_deref(), Some("net/tls"));
        assert_eq!(bundle_file_mod_path("pouch.rut"), None);
        assert_eq!(bundle_file_mod_path("myhost.d.rut"), None, "a surface root is a flat file");
    }

    #[test]
    fn a_host_manifest_without_entry_type_discovers_the_default_root() {
        // the lib-parity amendment: `mod.d.rut` beside the manifest is
        // the auto-discovered root — the editor's dep table carries it
        // as the Decl entry exactly like a spelled surface
        let t = dep_table(r#"{ "name": "h", "type": "host" }"#).expect("parses");
        assert!(t.host);
        assert_eq!(
            t.entries,
            vec![EntrySource { path: "mod.d.rut".into(), mode: Mode::Decl }]
        );
        // a spelled surface still wins
        let t = dep_table(
            r#"{ "name": "h", "type": "host", "entry": { "type": "./h.d.rut" } }"#,
        )
        .expect("parses");
        assert_eq!(
            t.entries,
            vec![EntrySource { path: "h.d.rut".into(), mode: Mode::Decl }]
        );
    }

    /// a host-tree bundle in the pack writer's shape: a `type = "host"`
    /// manifest + the root `.d.rut` entry + the additive `rut.mods`
    /// rows entry (the Decl tree as path-keyed rows)
    fn host_tree_bundle() -> Vec<u8> {
        use std::collections::BTreeMap;
        let mods = BTreeMap::from([
            (
                "net".to_string(),
                rut_driver::ModSource {
                    path: "net".into(),
                    vis: rut_ast::ast::Vis::Pub,
                    text: "pub mod tls;\npub host fn connect(url: str) -> opaque;\n".into(),
                },
            ),
            (
                "net/tls".to_string(),
                rut_driver::ModSource {
                    path: "net/tls".into(),
                    vis: rut_ast::ast::Vis::Pub,
                    text: "pub host fn handshake(c: opaque) -> opaque;\n".into(),
                },
            ),
        ]);
        let rows = rut_driver::mods::rows_json("pub mod net;\npub host fn legacy_ping() -> bool;\n", &mods);
        rut_driver::bundle::write_bundle(&[
            (
                MANIFEST_NAME.into(),
                br#"{ "name": "myhost", "type": "host", "entry": { "type": "./myhost.d.rut" } }"#.to_vec(),
            ),
            ("myhost.d.rut".into(), b"pub mod net;\npub host fn legacy_ping() -> bool;\n".to_vec()),
            (rut_driver::mods::ROWS_NAME.into(), rows.into_bytes()),
        ])
        .unwrap()
    }

    #[test]
    fn a_host_tree_bundle_unpacks_the_decl_flavor() {
        let b = bundle_sources(&host_tree_bundle()).expect("the host bundle unpacks");
        assert_eq!(b.module, "myhost");
        // the root surface rode the decl entry; the children are Decl
        // files at `<mod path>/mod.d.rut`
        let files: Vec<(&str, Mode)> = b.files.iter().map(|f| (f.path.as_str(), f.mode)).collect();
        assert_eq!(
            files,
            vec![
                ("myhost.d.rut", Mode::Decl),
                ("net/mod.d.rut", Mode::Decl),
                ("net/tls/mod.d.rut", Mode::Decl),
            ],
            "{files:?}"
        );
        assert!(b.files[0].src.contains("pub mod net;"));
        assert!(b.files[1].src.contains("connect"));
        assert!(b.files[2].src.contains("handshake"));
    }

    #[test]
    fn a_host_manifest_marks_the_table() {
        let t = dep_table(
            r#"{ "name": "myhost", "type": "host", "entry": { "type": "./myhost.d.rut" } }"#,
        )
        .expect("the host manifest parses");
        assert!(t.host, "type = host marks the table");
        assert_eq!(
            t.entries,
            vec![EntrySource { path: "myhost.d.rut".into(), mode: Mode::Decl }],
            "the surface indexes in Decl mode; no lib body row"
        );
        let surface_only =
            dep_table(r#"{ "name": "s", "type": "lib", "entry": { "type": "./s.d.rut" } }"#)
                .expect("the lib manifest parses");
        assert!(!surface_only.host, "a lib manifest stays unmarked");
    }
}
