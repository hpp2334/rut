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
//!   `dev-deps`, `peer-deps`), each a `path` directory or a pinned
//!   `url` bundle with the `.rut/cache/` cache path computed here.
//! - [`bundle_sources`] unpacks a `.rutbundle` into the module's OWN
//!   source files — the zip's own `rut.jsonc` names the module; the
//!   rode-along dep groups are the loader's business, not the
//!   editor's.
//! - [`index_dep`] indexes one dep source file: [`analysis::index_at`]
//!   plus `module = Some(name)`, with `namespace`/`consts` minted onto
//!   the index (a namespace-head type row, module-scope lets for the
//!   consts — the `calc`/`Math.PI` shape: the engine binds
//!   `use calc::{Math}` then routes `Math.sqrt`/`Math.PI` as qualified
//!   access, and the member machinery resolves both through the minted
//!   row).

use rut_ast::ast::Vis;
use rut_driver::bundle::files::{bundle_key, read_entry, MANIFEST_NAME};
use rut_driver::bundle::manifest::parse_manifest;
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
/// remote `.rutbundle` pinned by its sha256. The cache path is
/// computed HERE (`.rut/cache/<sha256(url)>.rutbundle`) — the faces
/// never hash URLs.
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
    /// (`entry.type`, Decl mode), then the body (`entry.lib`, then
    /// `entry.libs` in listed order, Impl mode)
    pub entries: Vec<EntrySource>,
    /// the qualified-access head (`calc`'s `Math`)
    pub namespace: Option<String>,
    /// the compiler-materialized constants (`Math.PI`), name → f64
    pub consts: Vec<(String, f64)>,
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
/// or url-sourced (the pin beside the url); `peer-deps`/`dev-deps`
/// descriptors are path-only by grammar (a url there refused at parse)
/// — the url arm simply never fires for them.
fn dep_row(name: &str, desc: &std::collections::BTreeMap<String, String>) -> DepRow {
    let source = match (desc.get("path"), desc.get("url")) {
        (_, Some(url)) => DepSource::Url {
            url: url.clone(),
            sha256: desc.get("sha256").cloned(),
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
/// the faces surface it as a hint, never silent bytes.
pub fn dep_table(manifest_text: &str) -> Result<DepTable, String> {
    let m: Manifest = parse_manifest(manifest_text).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    // the decl surface indexes in Decl mode; the body in Impl mode —
    // the same split `analysis::mode_of` speaks for open documents
    if let Some(ty) = &m.entry.type_path {
        entries.push(EntrySource { path: bundle_key(ty)?, mode: Mode::Decl });
    }
    if let Some(lib) = &m.entry.lib {
        entries.push(EntrySource { path: bundle_key(lib)?, mode: Mode::Impl });
    }
    for lib in &m.entry.libs {
        entries.push(EntrySource { path: bundle_key(lib)?, mode: Mode::Impl });
    }
    let rows = |table: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>| {
        table.iter().map(|(name, desc)| dep_row(name, desc)).collect()
    };
    Ok(DepTable {
        name: m.name,
        entries,
        namespace: m.namespace,
        consts: m.consts.into_iter().collect(),
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
/// old flat envelope, which keeps loading byte-for-byte.
fn bundle_rows(
    entries: &[(String, Vec<u8>)],
) -> Result<Option<(String, std::collections::BTreeMap<String, rut_driver::ModSource>)>, String> {
    let text = match read_entry(entries, rut_driver::mods::ROWS_NAME) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let (root, rows) = rut_driver::mods::parse_rows(&text)?;
    let mods = rut_driver::mods::mount_rows(&root, &rows)?;
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
/// the `entry.lib` file while the transitional key stands, else the
/// `mod.rut` entry beside the manifest (the loader's dual-read
/// mirrored) — plus the `rut.mods` rows entry when the bundle carries
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
    let table = dep_table(&manifest_text)?;
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
    // writer's shape), else the file-entry mount over the root module
    let tree: Option<(BundleFile, Vec<BundleFile>)> = match bundle_rows(&entries)? {
        Some((root_text, mods)) => {
            let children = mods
                .into_iter()
                .map(|(path, m)| BundleFile {
                    path: format!("{path}/mod.rut"),
                    src: m.text,
                    mode: Mode::Impl,
                })
                .collect();
            Some((
                BundleFile { path: "mod.rut".to_string(), src: root_text, mode: Mode::Impl },
                children,
            ))
        }
        None => {
            // the source lane: root = entry.lib while the transitional
            // key stands, else the `mod.rut` entry beside the manifest
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
                    Some((root, children))
                }
                None => None,
            }
        }
    };
    if let Some((root, children)) = tree {
        files.push(root);
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
/// (`Some("")`), a mounted child as `<path>/mod.rut` (`Some(path)`);
/// anything else is today's flat shape (`None`) — shared by both
/// faces (the stdio walk and the wasm shim index identically)
pub fn bundle_file_mod_path(path: &str) -> Option<String> {
    match path.strip_suffix("/mod.rut") {
        Some(p) => Some(p.to_string()),
        None if path == "mod.rut" => Some(String::new()),
        None => None,
    }
}

/// Index one dep source file: the ordinary [`analysis::index_at`] pass
/// over `src` (origin stamped with `uri`, mode per the manifest), the
/// module NAMED, and the manifest's `namespace`/`consts` minted on —
/// a `TyForm::Namespace` type row for the head (the module's free fns
/// as methods, the consts as fields: the qualified `Math.sqrt`/`Math.PI`
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

    const PLUGIN_MANIFEST: &str = include_str!("../../../examples/03-plugin/plugin/rut.jsonc");
    const CALC_MANIFEST: &str = include_str!("../../../rut/calc/rut.jsonc");
    const POUCH_BUNDLE: &[u8] = include_bytes!("../../../dist/std/pouch.rutbundle");

    #[test]
    fn plugin_manifest_rows_path_and_url() {
        let t = dep_table(PLUGIN_MANIFEST).expect("the real plugin manifest parses");
        assert_eq!(t.name.as_deref(), Some("plugin"));
        // the entry: one lib source, Impl mode
        assert_eq!(t.entries, vec![EntrySource { path: "plugin.rut".into(), mode: Mode::Impl }]);
        // deps: the path row …
        let server = t.deps.iter().find(|r| r.name == "server").expect("server row");
        assert_eq!(server.source, DepSource::Path { dir: "../server".into() });
        assert!(!server.optional);
        assert!(server.lib.is_none());
        // … and the pinned url row with the cache path computed here
        let pouch = t.deps.iter().find(|r| r.name == "pouch").expect("pouch row");
        match &pouch.source {
            DepSource::Url { url, sha256, cache_path } => {
                assert_eq!(url, "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v8/dist/std/pouch.rutbundle");
                assert_eq!(
                    sha256.as_deref(),
                    Some("d0bc36bb385bcc14e76d6275c92613edeab2d46ae0d162f7104282caef1a12cd")
                );
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
  "entry": { "lib": "./json.rut" },
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
        // the manifest's entry.lib — the ONLY own source; `.rutc` and
        // the scope ledger never ride, neither do dep groups
        assert_eq!(b.files.len(), 1, "files: {:?}", b.files.iter().map(|f| &f.path));
        assert_eq!(b.files[0].path, "pouch.rut");
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
        // … the surface indexes …
        assert!(idx.fns.iter().any(|f| f.name == "sqrt" && f.owner.is_none()));
        // … the namespace head mints as a type row carrying the fns as
        // methods and the consts as fields — the Math.sqrt/Math.PI shape
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
        assert_eq!(bundle_file_mod_path("pouch.rut"), None);
    }
}
