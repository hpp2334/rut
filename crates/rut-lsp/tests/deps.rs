use rut_driver::bundle::files::MANIFEST_NAME;
use rut_lsp::deps::*;
use rut_lsp::hover::TyForm;
use rut_parser::Mode;

const CALC_HOST_MANIFEST: &str = include_str!("../../../rut/calc_host/rut.jsonc");
const POUCH_BUNDLE: &[u8] = include_bytes!("../../../dist/std/pouch.rutbundle");

/// The plugin manifest's dep shape, INLINE and unpinned: the real
/// `examples/03-plugin/plugin/rut.jsonc` migrates off the retired
/// manifest `sha256` key in the docs/examples phase — the grammar
/// refuses the key today, so the editor's fixture is the shape
/// every manifest must now spell (pins live in rut.lock).
const PLUGIN_SHAPE: &str = r#"{
  "format": "rutbundle",
  "format_version": 12,
  "name": "plugin",
  "deps": {
    "server": { "path": "../server" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle" }
  }
}"#;

const POUCH_URL: &str = "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v12/dist/std/pouch.rutbundle";
const POUCH_PIN: &str = "63ddd765287243ef45f23105471ff3eae5c5a0ac1d521a3d22636745fd5e72d8";

#[test]
fn plugin_manifest_rows_path_and_url() {
    // unpinned in the manifest — the pin is the lock's row
    let t = dep_table(PLUGIN_SHAPE).expect("the plugin-shape manifest parses");
    assert_eq!(t.name.as_deref(), Some("plugin"));
    // the entry: one lib source, Impl mode
    // the body rides the src/mod.rut convention (the entry keys are repealed)
    assert_eq!(
        t.entries,
        vec![EntrySource {
            path: "src/mod.rut".into(),
            mode: Mode::Impl
        }]
    );
    // deps: the path row …
    let server = t
        .deps
        .iter()
        .find(|r| r.name == "server")
        .expect("server row");
    assert_eq!(
        server.source,
        DepSource::Path {
            dir: "../server".into()
        }
    );
    assert!(!server.optional);
    assert!(server.lib.is_none());
    // … and the url row: UNPINNED at parse (sha256 retired from the
    // grammar), the cache path computed here
    let pouch = t
        .deps
        .iter()
        .find(|r| r.name == "pouch")
        .expect("pouch row");
    match &pouch.source {
        DepSource::Url {
            url,
            sha256,
            cache_path,
        } => {
            assert_eq!(url, POUCH_URL);
            assert_eq!(sha256, &None, "the manifest is never the pin's source");
            assert_eq!(
                cache_path,
                &format!(".rut/cache/{}.rutbundle", {
                    use sha2::{Digest, Sha256};
                    let hex: String = Sha256::digest(url.as_bytes())
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    hex
                })
            );
        }
        other => panic!("pouch must be a url row: {other:?}"),
    }
    assert!(t.dev_deps.is_empty());
}

#[test]
fn the_lock_pins_the_url_rows_and_a_missing_row_stays_none() {
    // dep_table_pinned: the walk-root rut.lock's rows stamp the url
    // rows (the manifest's own grammar cannot); a url the lock does
    // not name stays None — the walk's install-recipe refusal
    let pins = lock_pins(&rut_driver::render_lock(&rut_driver::LockFile::from_rows(
        vec![rut_driver::LockRow {
            pkg: "pouch".into(),
            url: POUCH_URL.into(),
            sha256: POUCH_PIN.into(),
        }],
    )))
    .expect("the lock parses");
    let t = dep_table_pinned(PLUGIN_SHAPE, &pins).expect("parses");
    let pouch = t
        .deps
        .iter()
        .find(|r| r.name == "pouch")
        .expect("pouch row");
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
    let pouch = t
        .deps
        .iter()
        .find(|r| r.name == "pouch")
        .expect("pouch row");
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
    // the namespace + consts live on the HOST pkg's manifest
    // (calc_host); `calc` beside it is the generic lib
    let t = dep_table(CALC_HOST_MANIFEST).expect("the real calc_host manifest parses");
    assert_eq!(t.name.as_deref(), Some("calc_host"));
    // the host surface: decl mode, no body — the spelled surface
    // resolves under the `src/` shell
    assert_eq!(
        t.entries,
        vec![EntrySource {
            path: "src/calc.d.rut".into(),
            mode: Mode::Decl
        }]
    );
    assert_eq!(t.namespace.as_deref(), Some("Math"));
    let pi = t
        .consts
        .iter()
        .find(|(n, _)| n == "PI")
        .expect("PI in consts");
    assert_eq!(pi.1, std::f64::consts::PI);
    // the non-finite spelling rides as the f64 it names
    let inf = t
        .consts
        .iter()
        .find(|(n, _)| n == "INFINITY")
        .expect("INFINITY in consts");
    assert!(inf.1.is_infinite() && inf.1.is_sign_positive());
}

#[test]
fn dev_descriptor_rows_ride() {
    let t = dep_table(
        r#"{
  "name": "json",
  "dev-deps": { "nmapset": { "path": "../nmapset" } }
}"#,
    )
    .expect("dev tables parse");
    let nmapset = t
        .dev_deps
        .iter()
        .find(|r| r.name == "nmapset")
        .expect("dev row");
    assert!(!nmapset.optional);
    assert!(nmapset.lib.is_none());
    assert_eq!(
        nmapset.source,
        DepSource::Path {
            dir: "../nmapset".into()
        }
    );
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
fn pouch_bundle_indexes_its_declaration_entry() {
    // wire v12: a published bundle carries its GENERATED declaration
    // entry — the editor's primary view. The entry parses in Decl
    // mode and spells the surface: the generic class with its
    // fields, the bodiless method signatures, the free fn.
    let b = bundle_sources(POUCH_BUNDLE).expect("the committed pouch bundle parses");
    assert_eq!(b.module, "pouch");
    assert_eq!(b.namespace, None);
    assert!(b.consts.is_empty());
    assert_eq!(
        b.files.len(),
        1,
        "files: {:?}",
        b.files.iter().map(|f| &f.path)
    );
    assert_eq!(
        b.files[0].path,
        "mod.d.rut",
        "{:?}",
        b.files.iter().map(|f| &f.path)
    );
    assert_eq!(b.files[0].mode, Mode::Decl);
    assert!(
        b.files[0].src.contains("GENERATED by rut pack"),
        "the entry: {}",
        b.files[0].src
    );
    assert!(
        b.files[0].src.contains("pub class Vec<T>"),
        "the entry: {}",
        b.files[0].src
    );
    assert!(
        b.files[0].src.contains("pub fn new() -> Self;"),
        "the entry: {}",
        b.files[0].src
    );
    assert!(
        b.files[0].src.contains("pub fn push(mut self,"),
        "the entry: {}",
        b.files[0].src
    );
    assert!(
        b.files[0].src.contains("pub fn freeze_bytes"),
        "the entry: {}",
        b.files[0].src
    );
}

#[test]
fn a_packed_v12_bundle_indexes_hover_and_completion_from_the_entry() {
    // the CDN-shaped dep lane end to end: the committed pouch
    // bundle (a compiled v12 root) unpacks to its generated
    // declaration entry; indexing it yields the hover/signature
    // facts and the member surface a consumer's editor offers —
    // from the ENTRY, one Decl parse, no binary decode
    let b = bundle_sources(POUCH_BUNDLE).expect("the committed pouch bundle parses");
    assert_eq!(b.files.len(), 1);
    let f = &b.files[0];
    assert_eq!(f.path, "mod.d.rut");
    assert_eq!(f.mode, Mode::Decl);
    let idxs = index_dep_decl_group(
        &b.module,
        "bundle:pouch/mod.d.rut",
        "",
        &f.src,
        b.namespace.as_deref(),
        &b.consts,
    );
    assert_eq!(idxs.len(), 1, "a flat entry: one module");
    let idx = &idxs[0].1;
    assert_eq!(idx.mode, Some(Mode::Decl));
    // the hover facts: the class row with its fields …
    let vec = idx.ty("Vec").expect("the class indexes");
    assert_eq!(vec.form, TyForm::Class);
    assert!(
        vec.fields.iter().any(|m| m.name == "buf"),
        "{:?}",
        vec.fields.iter().map(|m| &m.name)
    );
    assert!(vec.fields.iter().any(|m| m.name == "len"));
    // … and its member signatures — the completion surface
    let names: Vec<&str> = vec.methods.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"new"), "{names:?}");
    assert!(names.contains(&"push"), "{names:?}");
    let push = vec.methods.iter().find(|m| m.name == "push").expect("push");
    assert!(
        push.src.contains("pub fn push(mut self"),
        "the verbatim signature: {}",
        push.src
    );
    // the free fn indexes as a module-level fn with its signature
    let freeze = idx
        .fns
        .iter()
        .find(|f| f.name == "freeze_bytes")
        .expect("freeze_bytes");
    assert!(freeze.src.starts_with("fn freeze_bytes"), "{}", freeze.src);
    assert!(freeze.owner.is_none());
}

#[test]
fn the_decl_entry_indexes_whole() {
    // the v12 consumption: ONE Decl parse of the consolidated entry,
    // the inline nesting stamping one index per module — hover
    // facts from every level land with their mod paths
    let src = "\
pub mod net {
    pub host fn connect(url: str) -> opaque;
}
pub host fn legacy_ping() -> bool;
";
    let idxs = index_dep_decl_group("myhost", "bundle:myhost/mod.d.rut", "", src, None, &[]);
    let paths: Vec<&str> = idxs.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(paths, vec!["", "net"], "{paths:?}");
    let (root, net) = (&idxs[0].1, &idxs[1].1);
    // the root's own rows + the inline edge
    assert!(root.fns.iter().any(|f| f.name == "legacy_ping"));
    assert_eq!(root.mods.len(), 1);
    assert_eq!(root.mods[0].name, "net");
    // the child's rows, stamped at their mod path
    assert_eq!(net.mod_path.as_deref(), Some("net"));
    assert!(net.fns.iter().any(|f| f.name == "connect"));
    // one parse, two modules: the origins stay distinct (one per
    // index — the defs store's key)
    assert_eq!(idxs[0].1.origin, "bundle:myhost/mod.d.rut");
    assert_eq!(idxs[1].1.origin, "bundle:myhost/mod.d.rut#net");
}

#[test]
fn not_a_bundle_is_the_error() {
    let err = bundle_sources(b"definitely not a zip").unwrap_err();
    assert!(err.contains("not a zip"), "{err}");
}

#[test]
fn srczip_rides_the_unpack_ingredients() {
    // the real std artifact: the zip's own pkg indexes src-only,
    // while the embedded `deps/ink_host@…/` group rides home whole —
    // the manifest text beside the files, the group's entries under
    // it (the unpack law: the ordinary dir lane mounts the pkg's own
    // path rows INTO the embedded groups, first mount wins)
    let b = srczip_sources(include_bytes!("../../../dist/std/ink.zip"))
        .expect("the ink source zip indexes");
    assert_eq!(b.module, "ink");
    assert!(
        b.files.iter().all(|f| f.path.starts_with("src/")),
        "the own index stays src-only: {:?}",
        b.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    assert!(
        b.files.iter().any(|f| f.path == "src/mod.rut"),
        "the root module rides"
    );
    // the archive's own manifest, verbatim — the walk lays it beside
    // the files and parses the pkg's own rows from it
    assert!(
        b.manifest.contains("\"name\": \"ink\""),
        "the own manifest text: {}",
        b.manifest
    );
    // the embedded group: both its entries (the mini manifest + the
    // host surface), verbatim archive-relative paths
    let paths: Vec<_> = b.embedded.iter().map(|(p, _)| p.as_str()).collect();
    assert!(
        paths.contains(&"deps/ink_host@7e9a5d738fbd9797/rut.jsonc"),
        "{paths:?}"
    );
    assert!(
        paths.contains(&"deps/ink_host@7e9a5d738fbd9797/src/ink_host.d.rut"),
        "{paths:?}"
    );
    let group_manifest = &b
        .embedded
        .iter()
        .find(|(p, _)| p.ends_with("/rut.jsonc"))
        .expect("the group manifest rides")
        .1;
    assert!(
        group_manifest.contains("\"name\": \"ink_host\""),
        "{group_manifest}"
    );
    // a compiled bundle's ride-alongs are compiled — never text —
    // so the unpack ingredients stay empty there (unchanged law)
    let pouch = bundle_sources(POUCH_BUNDLE).expect("pouch indexes");
    assert!(pouch.embedded.is_empty(), "{:?}", pouch.embedded);
}

#[test]
fn index_dep_names_the_module_and_mints_the_namespace() {
    let src = "\
pub host fn sqrt(x: f64) -> f64;
";
    let consts = vec![("PI".to_string(), std::f64::consts::PI)];
    let idx = index_dep(
        "calc",
        "file:///deps/calc.d.rut",
        src,
        Mode::Decl,
        Some("Math"),
        &consts,
    );
    // the explicit name rides the index …
    assert_eq!(idx.module.as_deref(), Some("calc"));
    assert_eq!(idx.origin, "file:///deps/calc.d.rut");
    // … the parse mode rides too (the definition layer's
    // body-over-decl preference reads it) …
    assert_eq!(idx.mode, Some(Mode::Decl));
    // … the surface indexes …
    assert!(idx
        .fns
        .iter()
        .any(|f| f.name == "sqrt" && f.owner.is_none()));
    // … the namespace head mints as a type row carrying the fns as
    // methods and the consts as fields — the Math::sqrt/Math::PI shape
    let math = idx.ty("Math").expect("the namespace head mints");
    assert_eq!(math.form, TyForm::Namespace);
    assert!(math.is_pub);
    assert!(math.methods.iter().any(|m| m.name == "sqrt"));
    let pi = math
        .fields
        .iter()
        .find(|m| m.name == "PI")
        .expect("PI as a field");
    assert_eq!(pi.ty.as_deref(), Some("f64"));
    // … and the consts mint module-scope lets (the bare-`PI` form)
    let pi = idx
        .lets
        .iter()
        .find(|l| l.name == "PI")
        .expect("PI as a module let");
    assert_eq!(pi.ty.as_deref(), Some("f64"));
    assert!(pi.src.starts_with("let PI = 3.141592653589793"));
}

#[test]
fn index_dep_without_namespace_mints_nothing() {
    let idx = index_dep(
        "pouch",
        "file:///deps/pouch.rut",
        "class Vec<T> {\n}\n",
        Mode::Impl,
        None,
        &[],
    );
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
    let root = index_dep_mod(
        "gadgets",
        "file:///deps/gadgets/mod.rut",
        "",
        "pub mod layout;\n",
        Mode::Impl,
        None,
        &[],
    );
    assert_eq!(root.mod_path.as_deref(), Some(""));
    assert_eq!(root.mods.len(), 1);
    assert_eq!(root.mods[0].name, "layout");
}

/// THE HOP-1 MEMBER LAW: a dep source's inherent impl methods ride
/// the dep index (`impl Logger { pub fn info(self, …) }` → `fns`
/// with `owner: Some("Logger")`), so a consumer's `log.info(…)`
/// click resolves THROUGH `member_target`'s impl fallback to the
/// dep index's fn — the ink shape that stranded every scaffold's
/// method jump on the ambient repo-relative answer.
#[test]
fn dep_impl_methods_ride_the_dep_index() {
    const INK: &str = concat!(
        "use ink_host::{ create_logger, logger_log };\n",
        "pub class Logger {\n",
        "    internal: opaque;\n",
        "}\n",
        "impl Logger {\n",
        "    pub fn info(self, msg: str) -> nil { logger_log(self.internal, 1, msg); }\n",
        "}\n",
    );
    let idx = index_dep_mod(
        "ink",
        "file:///deps/ink/src/mod.rut",
        "",
        INK,
        Mode::Impl,
        None,
        &[],
    );
    let info = idx.fns.iter().find(|f| f.name == "info");
    assert_eq!(
        info.map(|f| f.owner.as_deref()),
        Some(Some("Logger")),
        "impl methods ride the dep index: {:?}",
        idx.fns
            .iter()
            .map(|f| (f.name.as_str(), f.owner.as_deref()))
            .collect::<Vec<_>>()
    );
    // the class body is fields-only; the method answers ONLY through
    // the impl fallback
    let ty = idx.ty("Logger").expect("the Logger ty rides");
    assert!(ty.methods.iter().all(|m| m.name != "info"));
    // the full member resolution, the definition lane's own path:
    // receiver `log` (ctor-inferred Logger) → `.info` → the dep fn
    let consumer = "use ink::{ Logger };\nentry fn main() {\n    let log = Logger(\"hello\");\n    log.info(\"hi\");\n}\n";
    let lines: Vec<&str> = consumer.lines().collect();
    let line = lines
        .iter()
        .position(|l| l.contains(".info"))
        .expect("the .info line") as u32;
    let ch = lines[line as usize].find(".info").unwrap() as u32 + 1;
    let hits = rut_lsp::analysis::definition_at(
        "file:///ws/main.rut",
        consumer,
        &[idx],
        &rut_lsp::mods::DocMods::default(),
        line,
        ch,
    );
    assert_eq!(hits.len(), 1, "the .info click resolves: {hits:?}");
    assert_eq!(hits[0].uri, "file:///deps/ink/src/mod.rut");
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
                file: "mod.rut".into(),
            },
        ),
        (
            "layout/grid".to_string(),
            rut_driver::ModSource {
                path: "layout/grid".into(),
                vis: rut_ast::ast::Vis::Pub,
                text: "pub struct Cell {\n    x: i32;\n}\n".into(),
                file: "mod.rut".into(),
            },
        ),
    ]);
    let rows = rut_driver::mods::rows_json(
        "pub mod layout;\npub fn tag() -> i32 { return 1; }\n",
        &mods,
    );
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
    assert_eq!(
        paths,
        vec!["mod.rut", "layout/mod.rut", "layout/grid/mod.rut"]
    );
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
        (
            "layout/mod.rut".into(),
            b"pub fn span() -> i32 { return 1; }\n".to_vec(),
        ),
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
    assert_eq!(
        bundle_file_mod_path("layout/mod.rut").as_deref(),
        Some("layout")
    );
    assert_eq!(
        bundle_file_mod_path("layout/grid/mod.rut").as_deref(),
        Some("layout/grid")
    );
    // the host tree's Decl children spell the same mod paths, and
    // the bare `mod.d.rut` IS the host root — `Some("")`, the twin
    // of the lib root's spelling
    assert_eq!(bundle_file_mod_path("mod.d.rut").as_deref(), Some(""));
    assert_eq!(
        bundle_file_mod_path("net/mod.d.rut").as_deref(),
        Some("net")
    );
    assert_eq!(
        bundle_file_mod_path("net/tls/mod.d.rut").as_deref(),
        Some("net/tls")
    );
    assert_eq!(bundle_file_mod_path("pouch.rut"), None);
    assert_eq!(
        bundle_file_mod_path("myhost.d.rut"),
        None,
        "a surface root is a flat file"
    );
}

#[test]
fn a_host_manifest_without_entry_type_discovers_the_default_root() {
    // the lib-parity amendment: `src/mod.d.rut` under the `src/`
    // shell is the auto-discovered root — the editor's dep table
    // carries it as the Decl entry exactly like a spelled surface
    let t = dep_table(r#"{ "name": "h", "type": "host" }"#).expect("parses");
    assert!(t.host);
    assert_eq!(
        t.entries,
        vec![EntrySource {
            path: "src/mod.d.rut".into(),
            mode: Mode::Decl
        }]
    );
    // a spelled surface still wins — under the shell as well
    let t = dep_table(r#"{ "name": "h", "type": "host", "entry": { "type": "./h.d.rut" } }"#)
        .expect("parses");
    assert_eq!(
        t.entries,
        vec![EntrySource {
            path: "src/h.d.rut".into(),
            mode: Mode::Decl
        }]
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
                file: "mod.d.rut".into(),
            },
        ),
        (
            "net/tls".to_string(),
            rut_driver::ModSource {
                path: "net/tls".into(),
                vis: rut_ast::ast::Vis::Pub,
                text: "pub host fn handshake(c: opaque) -> opaque;\n".into(),
                file: "mod.d.rut".into(),
            },
        ),
    ]);
    let rows =
        rut_driver::mods::rows_json("pub mod net;\npub host fn legacy_ping() -> bool;\n", &mods);
    rut_driver::bundle::write_bundle(&[
        (
            MANIFEST_NAME.into(),
            br#"{ "name": "myhost", "type": "host", "entry": { "type": "./myhost.d.rut" } }"#
                .to_vec(),
        ),
        // the spelled surface rides the ONE canonical wire name —
        // the archive is name-free (the pack writer's shape)
        (
            rut_driver::decl::DECL_CHILD_FILE.into(),
            b"pub mod net;\npub host fn legacy_ping() -> bool;\n".to_vec(),
        ),
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
            ("mod.d.rut", Mode::Decl),
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
    let t =
        dep_table(r#"{ "name": "myhost", "type": "host", "entry": { "type": "./myhost.d.rut" } }"#)
            .expect("the host manifest parses");
    assert!(t.host, "type = host marks the table");
    assert_eq!(
        t.entries,
        vec![EntrySource {
            path: "src/myhost.d.rut".into(),
            mode: Mode::Decl
        }],
        "the surface indexes in Decl mode; no lib body row"
    );
    let surface_only =
        dep_table(r#"{ "name": "s", "type": "lib", "entry": { "type": "./s.d.rut" } }"#)
            .expect("the lib manifest parses");
    assert!(!surface_only.host, "a lib manifest stays unmarked");
}
