//! Path loading — module directories (`rut.toml`) and `.rutbundle`
//! archives (RFC 0038): pack/load round trips, determinism, and the
//! refusal gates (unknown layout version, `[deps]`, corruption).

use std::path::Path;

use rut_driver::{load_bundle_session, load_dir_session, load_path_session, pack_dir};

/// A two-file module in a temp dir: `entry.rut` includes `./inc.rut`.
fn make_dir(base: &Path) -> std::path::PathBuf {
    let dir = base.join("mod");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rut.toml"),
        // bundle-shaped: the keys a `rut pack` needs are already there,
        // and directory loading ignores them (RFC 0038 §2)
        "format = \"rutbundle\"\nformat_version = 1\nname = \"app:mod\"\nentry.lib = \"./entry.rut\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("entry.rut"),
        "import { seven } from \"./inc.rut\";\nfn main() -> i32 { return seven(); }\n",
    )
    .unwrap();
    std::fs::write(dir.join("inc.rut"), "pub fn seven() -> i32 { return 7; }\n").unwrap();
    dir
}

fn linked_binary(session: &rut_driver::Session, root: &str) -> Vec<u8> {
    let g = rut_driver::compile_graph(session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    rut_core::binary::encode(&g.program.expect("linked program"))
}

#[test]
fn bundle_round_trip_matches_the_directory() {
    let base = std::env::temp_dir().join(format!("rut-bundle-test-{}", std::process::id()));
    let dir = make_dir(&base);

    // the directory form
    let (session, root) = load_dir_session(&dir).unwrap();
    let from_dir = linked_binary(&session, &root);

    // the packed form: same sources, same linked bytes
    let bytes = pack_dir(&dir).unwrap();
    let bundle_path = base.join("mod.rutbundle");
    std::fs::write(&bundle_path, &bytes).unwrap();
    let (session, root) = load_bundle_session(&bundle_path).unwrap();
    let from_bundle = linked_binary(&session, &root);
    assert_eq!(from_dir, from_bundle, "dir and bundle compile identically");

    // dispatch agrees on both forms
    let (s1, r1) = load_path_session(&dir).unwrap();
    let (s2, r2) = load_path_session(&bundle_path).unwrap();
    assert_eq!(linked_binary(&s1, &r1), from_dir);
    assert_eq!(linked_binary(&s2, &r2), from_dir);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn packing_is_deterministic_and_multi_file() {
    let base = std::env::temp_dir().join(format!("rut-bundle-det-{}", std::process::id()));
    let dir = make_dir(&base);
    let a = pack_dir(&dir).unwrap();
    let b = pack_dir(&dir).unwrap();
    assert_eq!(a, b, "same dir => byte-identical bundle");
    // both sources are in the archive, manifest first
    let entries = rut_driver::parse_bundle(&a).unwrap();
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["rut.toml", "entry.rut", "inc.rut"]);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn in_memory_bytes_load_without_a_file() {
    let base = std::env::temp_dir().join(format!("rut-bundle-mem-{}", std::process::id()));
    let dir = make_dir(&base);
    let bytes = pack_dir(&dir).unwrap();
    let (session, root) = rut_driver::load_bundle_bytes(&bytes, Path::new("mem")).unwrap();
    assert_eq!(root, "app:mod");
    assert!(rut_driver::compile_graph(&session, &root).program.is_some());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn refusals() {
    let base = std::env::temp_dir().join(format!("rut-bundle-ref-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let src = "fn main() -> i32 { return 7; }\n".as_bytes();

    // no rut.toml entry at all
    let not_a_bundle = rut_driver::write_bundle(&[("x.rut".into(), src.to_vec())]).unwrap();
    let err = rut_driver::load_bundle_bytes(&not_a_bundle, Path::new("a")).unwrap_err();
    assert!(err.contains("rut.toml"), "{err}");

    // unknown format_version — refused before anything else is read
    let manifest = "format = \"rutbundle\"\nformat_version = 2\nname = \"app:x\"\nentry.lib = \"./x.rut\"\n";
    let bad_version = rut_driver::write_bundle(&[
        ("rut.toml".into(), manifest.as_bytes().to_vec()),
        ("x.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&bad_version, Path::new("b")).unwrap_err();
    assert!(err.contains("format_version"), "{err}");

    // missing `format = "rutbundle"`
    let manifest = "format_version = 1\nname = \"app:x\"\nentry.lib = \"./x.rut\"\n";
    let no_format = rut_driver::write_bundle(&[
        ("rut.toml".into(), manifest.as_bytes().to_vec()),
        ("x.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&no_format, Path::new("c")).unwrap_err();
    assert!(err.contains("rutbundle"), "{err}");

    // `[deps]` — one module per bundle in v1
    let manifest =
        "format = \"rutbundle\"\nformat_version = 1\nname = \"app:x\"\nentry.lib = \"./x.rut\"\n[deps]\n\"lib:m\" = { path = \"../m\" }\n";
    let with_deps = rut_driver::write_bundle(&[
        ("rut.toml".into(), manifest.as_bytes().to_vec()),
        ("x.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&with_deps, Path::new("d")).unwrap_err();
    assert!(err.contains("one module"), "{err}");

    // surface/ir payloads are a later format_version
    let manifest =
        "format = \"rutbundle\"\nformat_version = 1\nname = \"app:x\"\nentry.type = \"./x.d.rut\"\n";
    let with_type = rut_driver::write_bundle(&[
        ("rut.toml".into(), manifest.as_bytes().to_vec()),
        ("x.d.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&with_type, Path::new("e")).unwrap_err();
    assert!(err.contains("sources only"), "{err}");

    // a missing include is a load error naming the entry
    let manifest = "format = \"rutbundle\"\nformat_version = 1\nname = \"app:x\"\nentry.lib = \"./x.rut\"\n";
    let missing_inc = rut_driver::write_bundle(&[
        ("rut.toml".into(), manifest.as_bytes().to_vec()),
        ("x.rut".into(), b"import { y } from \"./gone.rut\";\n".to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&missing_inc, Path::new("f")).unwrap_err();
    assert!(err.contains("gone.rut"), "{err}");

    // corruption: flip a payload byte, the CRC check fires
    let dir = make_dir(&base);
    let mut bytes = pack_dir(&dir).unwrap();
    let at = 30 + "name = \"app:mod\"\nentry.lib = \"./entry.rut\"\n".len();
    bytes[at] ^= 0x01;
    let err = rut_driver::load_bundle_bytes(&bytes, Path::new("g")).unwrap_err();
    assert!(err.contains("CRC"), "{err}");

    // `..` include escapes are refused
    let manifest = "format = \"rutbundle\"\nformat_version = 1\nname = \"app:x\"\nentry.lib = \"./x.rut\"\n";
    let escape = rut_driver::write_bundle(&[
        ("rut.toml".into(), manifest.as_bytes().to_vec()),
        ("x.rut".into(), b"import { y } from \"../evil.rut\";\n".to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&escape, Path::new("h")).unwrap_err();
    assert!(err.contains("escapes"), "{err}");

    // a loose .rut file is not a path-form module
    let loose = base.join("loose.rut");
    std::fs::write(&loose, src).unwrap();
    assert!(load_path_session(&loose).is_err());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn pack_requires_bundle_shaped_manifest() {
    // a bare directory manifest packs to a bundle no loader would accept,
    // so `pack_dir` refuses up front and says what to add
    let base = std::env::temp_dir().join(format!("rut-bundle-shape-{}", std::process::id()));
    let dir = base.join("mod");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rut.toml"), "name = \"app:mod\"\nentry.lib = \"./m.rut\"\n").unwrap();
    std::fs::write(dir.join("m.rut"), "pub fn f() -> i32 { return 0; }\n").unwrap();
    let err = pack_dir(&dir).unwrap_err();
    assert!(err.contains("bundle-shaped"), "{err}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn pack_refuses_deps_and_surface_entries() {
    let base = std::env::temp_dir().join(format!("rut-bundle-packref-{}", std::process::id()));
    let dir = base.join("consumer");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rut.toml"),
        "format = \"rutbundle\"\nformat_version = 1\nname = \"app:main\"\nentry.lib = \"./entry.rut\"\n[deps]\n\"lib:m\" = { path = \"../m\" }\n",
    )
    .unwrap();
    std::fs::write(dir.join("entry.rut"), "fn main() -> i32 { return 0; }\n").unwrap();
    let err = pack_dir(&dir).unwrap_err();
    assert!(err.contains("one module"), "{err}");

    std::fs::write(
        dir.join("rut.toml"),
        "format = \"rutbundle\"\nformat_version = 1\nname = \"app:surf\"\nentry.type = \"./surf.d.rut\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("surf.d.rut"), "pub fn f() -> i32;\n").unwrap();
    let err = pack_dir(&dir).unwrap_err();
    assert!(err.contains("sources only"), "{err}");

    let _ = std::fs::remove_dir_all(&base);
}
