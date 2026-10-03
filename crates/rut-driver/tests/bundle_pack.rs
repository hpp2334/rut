//! Source file-set tests: `collect_source_group` (the v4 group shape a
//! splice-needed dep rides inside a v5 bundle), entry-name
//! normalization, and the in-memory [`Source`] — the wasm/test shape of
//! the dependency injection. The compiled-bundle pack/load round trips
//! live in the sibling driver tests — this file never builds a session.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rut_driver::bundle::{bundle_key, collect_source_group, default_out_path, parse_manifest, Source};

/// The in-memory [`Source`] — a path→bytes map.
#[derive(Default)]
struct MapSource(BTreeMap<PathBuf, Vec<u8>>);

impl Source for MapSource {
    fn read(&self, p: &Path) -> Result<Vec<u8>, String> {
        self.0
            .get(p)
            .cloned()
            .ok_or_else(|| format!("cannot read {}: no such path", p.display()))
    }
}


/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Pkg::source(spec, src))
            .pkg(rut_driver::calc_pkg())
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

#[test]
fn bundles_the_source_file_set_in_manifest_order() {
    // a package's file set: its `rut.jsonc` byte-for-byte, the entry,
    // each `entry.libs` file in manifest order, and each `[peer-deps]`
    // `lib` group file — the exact shape the loader splices back
    let root = PathBuf::from("json");
    let mut files = MapSource::default();
    files.0.insert(
        root.join("rut.jsonc"),
        br#"{"name": "json", "entry": {"lib": "./json.rut", "libs": ["./store.rut"]}, "peer-deps": {"pouch": {"path": "../pouch", "optional": true, "lib": "./serde_pouch.rut"}}}"#
            .to_vec(),
    );
    files.0.insert(root.join("json.rut"), b"pub fn f() -> str { return \"j\"; }\n".to_vec());
    files.0.insert(root.join("store.rut"), b"// the store half\n".to_vec());
    files.0.insert(root.join("serde_pouch.rut"), b"impl<T> J for Vec<T>".to_vec());

    let manifest = parse_manifest(
        &String::from_utf8(files.0.get(&root.join("rut.jsonc")).unwrap().clone()).unwrap(),
    )
    .unwrap();
    let mut out = Vec::new();
    collect_source_group(&root, &manifest, "json/", &files, &mut out).unwrap();
    let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["json/rut.jsonc", "json/json.rut", "json/store.rut", "json/serde_pouch.rut"],
        "manifest order IS the archive order"
    );
    // the manifest rides byte-for-byte
    assert_eq!(
        out[0].1,
        files.0.get(&root.join("rut.jsonc")).unwrap().as_slice(),
        "rut.jsonc verbatim"
    );
}

#[test]
fn a_manifest_named_file_the_map_lacks_is_a_read_error() {
    let root = PathBuf::from("mod");
    let mut files = MapSource::default();
    files.0.insert(
        root.join("rut.jsonc"),
        br#"{"name": "mod", "entry": {"lib": "./mod.rut"}}"#.to_vec(),
    );
    let manifest = parse_manifest(r#"{"name": "mod", "entry": {"lib": "./mod.rut"}}"#).unwrap();
    let mut out = Vec::new();
    let err = collect_source_group(&root, &manifest, "", &files, &mut out).unwrap_err();
    assert!(err.contains("cannot read"), "{err}");
}

#[test]
fn entry_names_normalize_and_escape_refuses() {
    assert_eq!(bundle_key("./a.rut").unwrap(), "a.rut");
    assert_eq!(bundle_key("deps/a.rut").unwrap(), "deps/a.rut");
    let err = bundle_key("../evil.rut").unwrap_err();
    assert!(err.contains("escapes"), "{err}");
    assert!(bundle_key("/abs.rut").is_err());
}

#[test]
fn default_out_path_is_a_sibling() {
    let p = default_out_path(Path::new("demo/mod"));
    assert_eq!(p, PathBuf::from("demo/mod.rutbundle"));
}
