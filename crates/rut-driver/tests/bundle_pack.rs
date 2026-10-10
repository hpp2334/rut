//! Source file-set tests: `collect_source_group` (the v4 group shape a
//! splice-needed dep rides inside a v5 bundle), entry-name
//! normalization, and the string-keyed in-memory reader — the
//! wasm/test shape of the dependency injection. The compiled-bundle
//! pack/load round trips live in the sibling driver tests — this file
//! never builds a session.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rut_driver::bundle::{bundle_key, parse_manifest, parse_manifest_compat};
use rut_driver::pack::{collect_source_group, PackRead};
use rut_native::default_out_path;

/// The in-memory reader — a key→bytes map behind the world's
/// `(dir key, rel)` door: the join is the test's own key math, exactly
/// what `FsSource` does for the real filesystem.
fn map_read(files: &BTreeMap<String, Vec<u8>>) -> PackRead {
    let files = files.clone();
    Rc::new(move |dir: &str, rel: &str| {
        let key = format!("{dir}/{rel}");
        files
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("cannot read {key}: no such path"))
    })
}

#[test]
fn bundles_the_source_file_set_in_manifest_order() {
    // a FLAT package's file set: its `rut.jsonc` byte-for-byte and the
    // root module (`mod.rut`) — the post-repeal group shape — plus each
    // `[peer-deps]` `lib` group file
    let mut files = BTreeMap::new();
    files.insert(
        "json/rut.jsonc".to_string(),
        br#"{"name": "json", "peer-deps": {"pouch": {"path": "../pouch", "optional": true, "lib": "./serde_pouch.rut"}}}"#
            .to_vec(),
    );
    files.insert("json/mod.rut".to_string(), b"pub fn f() -> str { return \"j\"; }\n".to_vec());
    files.insert("json/serde_pouch.rut".to_string(), b"impl<T> J for Vec<T>".to_vec());

    let manifest = parse_manifest(
        &String::from_utf8(files["json/rut.jsonc"].clone()).unwrap(),
    )
    .unwrap();
    let mut out = Vec::new();
    collect_source_group("json", &manifest, &Default::default(), "json/", &map_read(&files), &mut out).unwrap();
    let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["json/rut.jsonc", "json/mod.rut", "json/serde_pouch.rut"],
        "the manifest rides first, then the root module, then the peer groups"
    );
    // the manifest rides byte-for-byte
    assert_eq!(
        out[0].1,
        files["json/rut.jsonc"].as_slice(),
        "rut.jsonc verbatim"
    );

    // the OLD envelope's group shape (a published archive being
    // re-packed): the compat manifest maps the repealed keys, and the
    // legacy files ride in manifest order — the reader-compat law's
    // writer twin
    let mut old = BTreeMap::new();
    old.insert(
        "json/rut.jsonc".to_string(),
        br#"{"name": "json", "entry": {"lib": "./json.rut", "libs": ["./store.rut"]}, "peer-deps": {"pouch": {"path": "../pouch", "optional": true, "lib": "./serde_pouch.rut"}}}"#
            .to_vec(),
    );
    old.insert("json/json.rut".to_string(), b"pub fn f() -> str { return \"j\"; }\n".to_vec());
    old.insert("json/store.rut".to_string(), b"// the store half\n".to_vec());
    old.insert("json/serde_pouch.rut".to_string(), b"impl<T> J for Vec<T>".to_vec());
    let legacy = parse_manifest_compat(
        &String::from_utf8(old["json/rut.jsonc"].clone()).unwrap(),
    )
    .unwrap();
    let mut out2 = Vec::new();
    collect_source_group("json", &legacy, &Default::default(), "json/", &map_read(&old), &mut out2).unwrap();
    let names2: Vec<&str> = out2.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names2,
        vec!["json/rut.jsonc", "json/json.rut", "json/store.rut", "json/serde_pouch.rut"],
        "manifest order IS the archive order"
    );
}

#[test]
fn a_manifest_named_file_the_map_lacks_is_a_read_error() {
    let mut files = BTreeMap::new();
    files.insert(
        "mod/rut.jsonc".to_string(),
        br#"{"name": "mod"}"#.to_vec(),
    );
    let manifest = parse_manifest(r#"{"name": "mod"}"#).unwrap();
    let mut out = Vec::new();
    let err = collect_source_group("mod", &manifest, &Default::default(), "", &map_read(&files), &mut out).unwrap_err();
    assert!(err.contains("has no entry"), "{err}");
    assert!(err.contains("the root module is `mod.rut` beside the manifest"), "{err}");
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
