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

#[test]
fn bundles_the_source_file_set_in_manifest_order() {
    // a package's file set: its `rut.json` byte-for-byte, the entry,
    // each `entry.libs` file in manifest order, and each `[peer-deps]`
    // `lib` group file — the exact shape the loader splices back
    let root = PathBuf::from("json");
    let mut files = MapSource::default();
    files.0.insert(
        root.join("rut.json"),
        br#"{"name": "json", "entry": {"lib": "./json.rut", "libs": ["./store.rut"]}, "peer-deps": {"pouch": {"path": "../pouch", "optional": true, "lib": "./serde_pouch.rut"}}}"#
            .to_vec(),
    );
    files.0.insert(root.join("json.rut"), b"pub fn f() -> str { return \"j\"; }\n".to_vec());
    files.0.insert(root.join("store.rut"), b"// the store half\n".to_vec());
    files.0.insert(root.join("serde_pouch.rut"), b"impl J for Vec<T>".to_vec());

    let manifest = parse_manifest(
        &String::from_utf8(files.0.get(&root.join("rut.json")).unwrap().clone()).unwrap(),
    )
    .unwrap();
    let mut out = Vec::new();
    collect_source_group(&root, &manifest, "json/", &files, &mut out).unwrap();
    let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["json/rut.json", "json/json.rut", "json/store.rut", "json/serde_pouch.rut"],
        "manifest order IS the archive order"
    );
    // the manifest rides byte-for-byte
    assert_eq!(
        out[0].1,
        files.0.get(&root.join("rut.json")).unwrap().as_slice(),
        "rut.json verbatim"
    );
}

#[test]
fn a_manifest_named_file_the_map_lacks_is_a_read_error() {
    let root = PathBuf::from("mod");
    let mut files = MapSource::default();
    files.0.insert(
        root.join("rut.json"),
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
