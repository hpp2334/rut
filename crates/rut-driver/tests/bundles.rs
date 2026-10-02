//! Path loading — module directories (`rut.jsonc`) and `.rutbundle`
//! archives: a v7 bundle is the COMPILED contract (`.rutc` binaries +
//! scope ledger + mixed source groups), and the refusals gate it
//! (version, corruption, missing groups).

use std::path::Path;

use rut_driver::bundle::FsSource;
use rut_driver::{load_bundle_session, load_bundle_bytes, load_dir_session, load_path_session, pack_dir};

/// The make_dir manifest's text — also the corruption test's offset
/// reference (the first zip entry's payload).
fn make_dir_manifest() -> String {
    // bundle-shaped: the keys a `rut pack` needs are already there,
    // and directory loading ignores them (layout v9 — the compiled
    // format; the packer emits v9 and the loader reads v9 only)
    r#"{"format": "rutbundle", "format_version": 9, "name": "mod", "entry": {"lib": "./mod.rut"}}"#.to_string()
}

/// A one-file module in a temp dir (one directory, one entry file).
fn make_dir(base: &Path) -> std::path::PathBuf {
    let dir = base.join("mod");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rut.jsonc"), make_dir_manifest()).unwrap();
    std::fs::write(
        dir.join("mod.rut"),
        "pub fn seven() -> i32 { return 7; }\nfn main() -> i32 { return seven(); }\n",
    )
    .unwrap();
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
    let (session, root) = load_dir_session(&dir, &FsSource).unwrap();
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
fn in_memory_bytes_load_without_a_file() {
    let base = std::env::temp_dir().join(format!("rut-bundle-mem-{}", std::process::id()));
    let dir = make_dir(&base);
    let bytes = pack_dir(&dir).unwrap();
    let (session, root) = load_bundle_bytes(&bytes, Path::new("mem")).unwrap();
    assert_eq!(root, "mod");
    // the root mounts as a compiled body — the decoded `.rutc`
    assert!(matches!(
        session.resolve("mod").unwrap().body,
        rut_driver::ModuleBody::Compiled(_)
    ));
    assert!(rut_driver::compile_graph(&session, &root).program.is_some());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn refusals() {
    let base = std::env::temp_dir().join(format!("rut-bundle-ref-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let src = "entry fn main() -> i32 { return 7; }\n".as_bytes();

    // no rut.jsonc entry at all
    let not_a_bundle = rut_driver::bundle::write_bundle(&[("x.rut".into(), src.to_vec())]).unwrap();
    let err = rut_driver::load_bundle_bytes(&not_a_bundle, Path::new("a")).unwrap_err().to_string();
    assert!(err.contains("rut.jsonc"), "{err}");

    // pre-9 layouts are refused by the one-line version gate — the
    // same refusal an OLDER loader applies to a version it does not
    // know, before reading anything else (refuse, never guess). The
    // retired wire (≤8: the rut.json name, 7/8 versions) refuses
    // LOUDLY here: that IS the no-compat law — re-pack the directory.
    // 99+ is the reserved band, the same loud refusal.
    for v in [1u8, 2, 3, 4, 5, 6, 7, 8, 99, 100] {
        let manifest = format!(
            r#"{{"format": "rutbundle", "format_version": {v}, "name": "x", "entry": {{"lib": "./x.rut"}}}}"#
        );
        let old = rut_driver::bundle::write_bundle(&[
            ("rut.jsonc".into(), manifest.as_bytes().to_vec()),
            ("x.rut".into(), src.to_vec()),
        ])
        .unwrap();
        let err = rut_driver::load_bundle_bytes(&old, Path::new("b")).unwrap_err().to_string();
        assert!(err.contains("format_version"), "{err}");
        assert!(err.contains("reads bundle format_version 9 (compiled) and 10 (decl) only"), "{err}");
        assert!(err.contains("re-pack the directory"), "{err}");
        if v <= 8 {
            assert!(err.contains("pre-JSONC wire is retired"), "{err}");
        }
    }

    // the retired ENTRY NAME: a bundle whose root manifest still rides
    // as `rut.json` (the pre-9 spelling) gets the pointed
    // re-pack refusal — no fallback lane reads it
    let old_name = rut_driver::bundle::write_bundle(&[
        (
            "rut.json".into(),
            br#"{"format": "rutbundle", "format_version": 7, "name": "x", "entry": {"lib": "./x.rut"}}"#
                .to_vec(),
        ),
        ("x.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&old_name, Path::new("d"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("no `rut.jsonc` entry"), "{err}");
    assert!(err.contains("found `rut.json`"), "{err}");
    assert!(err.contains("predates wire 9"), "{err}");
    assert!(err.contains("re-pack the directory"), "{err}");

    // missing `format = "rutbundle"`
    let manifest = r#"{"format_version": 9, "name": "x", "entry": {"lib": "./x.rut"}}"#;
    let no_format = rut_driver::bundle::write_bundle(&[
        ("rut.jsonc".into(), manifest.as_bytes().to_vec()),
        ("x.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&no_format, Path::new("c")).unwrap_err().to_string();
    assert!(err.contains("rutbundle"), "{err}");

    // corruption: flip a payload byte, the CRC check fires (gate 1 —
    // before any manifest is read)
    let dir = make_dir(&base);
    let mut bytes = pack_dir(&dir).unwrap();
    let at = 30 + make_dir_manifest().len();
    bytes[at] ^= 0x01;
    let err = rut_driver::load_bundle_bytes(&bytes, Path::new("h")).unwrap_err().to_string();
    assert!(err.contains("CRC"), "{err}");

    // a pack whose declared surface file is missing refuses loudly
    // (the surface is a lib pkg's declared `entry.type` — a doc surface;
    // no host rows: those live only in `type = "host"` pkgs)
    let dir = make_dir(&base);
    std::fs::write(dir.join("surface.d.rut"), "/// the pkg's surface doc.\n").unwrap();
    std::fs::write(
        dir.join("rut.jsonc"),
        r#"{"format": "rutbundle", "format_version": 9, "name": "mod", "entry": {"lib": "./mod.rut", "type": "./surface.d.rut"}}"#,
    )
    .unwrap();
    std::fs::remove_file(dir.join("surface.d.rut")).unwrap();
    let err = pack_dir(&dir).unwrap_err().to_string();
    assert!(err.contains("surface.d.rut"), "{err}");

    // a loose .rut file is not a path-form module
    let loose = base.join("loose.rut");
    std::fs::write(&loose, src).unwrap();
    assert!(load_path_session(&loose).is_err());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn the_declared_kind_dispatches_in_bundle_groups_too() {
    // the bundle-group twins of the directory loader's kind dispatch:
    // a lib group whose surface declares host fns refuses at load
    // (the lib-surface law), and a surface-only lib group mounts as a
    // decl unit
    let base = std::env::temp_dir().join(format!("rut-bundle-kind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);

    // the dep `s`: a HOST pkg (declared), packable as a source group
    let s = base.join("s");
    std::fs::create_dir_all(&s).unwrap();
    std::fs::write(
        s.join("rut.jsonc"),
        r#"{"name": "s", "type": "host", "entry": {"type": "./s.d.rut"}}"#,
    )
    .unwrap();
    std::fs::write(s.join("s.d.rut"), "pub host fn ping(x: i32) -> i32;\n").unwrap();
    // the consumer: m (+ transitively s)
    let m = base.join("m");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(
        m.join("rut.jsonc"),
        r#"{"format": "rutbundle", "format_version": 9, "name": "main", "entry": {"lib": "./main.rut"}, "deps": {"s": {"path": "../s"}}}"#,
    )
    .unwrap();
    std::fs::write(
        m.join("main.rut"),
        "entry fn main() -> i32 { return 7; }\n",
    )
    .unwrap();

    let bytes = pack_dir(&m).unwrap();

    // TWIN 1 — the same group re-spelled `type = "lib"` (and nothing
    // else changed): the group's surface now rides the LIB arm, and its
    // host rows refuse at load, naming the fix
    let entries = rut_driver::bundle::parse_bundle(&bytes).unwrap();
    let lib_manifest =
        r#"{"name": "s", "type": "lib", "entry": {"type": "./s.d.rut"}}"#.as_bytes().to_vec();
    let respelled: Vec<(String, Vec<u8>)> = entries
        .iter()
        .map(|(n, b)| {
            if n == "s/rut.jsonc" {
                (n.clone(), lib_manifest.clone())
            } else {
                (n.clone(), b.clone())
            }
        })
        .collect();
    let respelled_bytes = rut_driver::bundle::write_bundle(&respelled).unwrap();
    let err = rut_driver::load_bundle_bytes(&respelled_bytes, Path::new("respelled")).unwrap_err().to_string();
    assert!(err.contains("s/s.d.rut"), "{err}");
    assert!(err.contains("`host fn ping`"), "{err}");
    assert!(err.contains("`type = \"host\"`"), "{err}");

    // TWIN 2 — the surface-only dev state packs and loads as a decl
    // unit: a lib dep with a clean surface and no body
    let s2 = base.join("s2");
    std::fs::create_dir_all(&s2).unwrap();
    std::fs::write(
        s2.join("rut.jsonc"),
        r#"{"name": "s", "type": "lib", "entry": {"type": "./s.d.rut"}}"#,
    )
    .unwrap();
    std::fs::write(s2.join("s.d.rut"), "/// documented surface, no body yet.\n").unwrap();
    let m2 = base.join("m2");
    std::fs::create_dir_all(&m2).unwrap();
    std::fs::write(
        m2.join("rut.jsonc"),
        r#"{"format": "rutbundle", "format_version": 9, "name": "main", "entry": {"lib": "./main.rut"}, "deps": {"s": {"path": "../s2"}}}"#,
    )
    .unwrap();
    std::fs::write(
        m2.join("main.rut"),
        "entry fn main() -> i32 { return 7; }\n",
    )
    .unwrap();
    let dev_bytes = pack_dir(&m2).unwrap();
    let (session, root) = rut_driver::load_bundle_bytes(&dev_bytes, Path::new("dev")).unwrap();
    assert_eq!(root, "main");
    let sf = session.resolve("s").expect("s mounted");
    assert!(
        matches!(
            sf.body,
            rut_driver::ModuleBody::Source { is_decl: true, .. }
        ),
        "the surface-only group is a decl unit: {:?}",
        sf.body
    );
    let g = rut_driver::compile_graph(&session, "main");
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    assert!(g.program.is_some());

    // and the declared host spelling still mounts as a host body (the
    // kind is honored from the field, pack to load)
    let (session, _) = load_bundle_bytes(&bytes, Path::new("host")).unwrap();
    assert!(matches!(
        session.resolve("s").expect("s mounted").body,
        rut_driver::ModuleBody::Host { .. }
    ));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn packs_the_dep_graph_and_loads_it_by_name() {
    // a v5 bundle embeds the whole `[deps]` closure — linkable pkgs as
    // compiled groups, host pkgs as source groups — and the loader
    // resolves groups by NAME (the deps' `path` keys are directory-time
    // only)
    let base = std::env::temp_dir().join(format!("rut-bundle-deps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);

    // the dep: a linkable source pkg → a compiled group
    let m = base.join("m");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(m.join("rut.jsonc"), r#"{"name": "m", "entry": {"lib": "./m.rut"}}"#).unwrap();
    std::fs::write(m.join("m.rut"), "pub fn four() -> i32 { return 4; }\n").unwrap();
    // the dep's dep: a host pkg (declaration-only surface) → source
    let s = base.join("s");
    std::fs::create_dir_all(&s).unwrap();
    std::fs::write(
        s.join("rut.jsonc"),
        r#"{"name": "s", "type": "host", "entry": {"type": "./s.d.rut"}}"#,
    )
    .unwrap();
    std::fs::write(s.join("s.d.rut"), "pub host fn ping(x: i32) -> i32;\n").unwrap();
    // m uses s
    std::fs::write(
        m.join("rut.jsonc"),
        r#"{"name": "m", "entry": {"lib": "./m.rut"}, "deps": {"s": {"path": "../s"}}}"#,
    )
    .unwrap();

    // the consumer: m (+ transitively s)
    let main = base.join("consumer");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::write(
        main.join("rut.jsonc"),
        r#"{"format": "rutbundle", "format_version": 9, "name": "main", "entry": {"lib": "./entry.rut"}, "deps": {"m": {"path": "../m"}}}"#,
    )
    .unwrap();
    std::fs::write(
        main.join("entry.rut"),
        "use m::{ four };\nentry fn main() -> i32 { return four(); }\n",
    )
    .unwrap();

    let bytes = pack_dir(&main).unwrap();
    let entries = rut_driver::bundle::parse_bundle(&bytes).unwrap();
    let mut names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["m/m.rutc", "m/rut.jsonc", "main.rutc", "rut.jsonc", "rut.scopes", "s/rut.jsonc", "s/s.d.rut"],
        "the closure rides compiled, the host pkg as source"
    );

    // determinism with deps too
    assert_eq!(bytes, pack_dir(&main).unwrap(), "same dir => byte-identical bundle");

    // the packed form compiles like the directory form
    let bundle = base.join("main.rutbundle");
    std::fs::write(&bundle, &bytes).unwrap();
    let (session, root) = load_path_session(&bundle).unwrap();
    assert_eq!(root, "main");
    assert!(matches!(
        session.resolve("m").expect("m mounted").body,
        rut_driver::ModuleBody::Compiled(_)
    ));
    let sf = session.resolve("s").expect("s mounted (transitively)");
    assert!(
        matches!(sf.body, rut_driver::ModuleBody::Host { .. }),
        "the host pkg rode along as a host body"
    );
    let g = rut_driver::compile_graph(&session, "main");
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    assert!(g.program.is_some());

    // a v5 bundle missing a declared dep group is a load error naming it
    let stripped: Vec<(String, Vec<u8>)> = entries
        .iter()
        .filter(|(n, _)| !n.starts_with("m/"))
        .cloned()
        .collect();
    let bytes = rut_driver::bundle::write_bundle(&stripped).unwrap();
    let err = rut_driver::load_bundle_bytes(&bytes, Path::new("stripped")).unwrap_err().to_string();
    assert!(err.contains("missing its `m` dependency group"), "{err}");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn the_directory_manifest_speaks_jsonc() {
    // the JSONC leniency through the LOADER lane: `//` and `/* */`
    // comments and trailing commas in a directory's `rut.jsonc` — the
    // same value laws underneath
    let base = std::env::temp_dir().join(format!("rut-jsonc-dir-{}", std::process::id()));
    let dir = base.join("jsonc");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rut.jsonc"),
        r#"// mod — the JSONC manifest
{
  // the header prose
  "format": "rutbundle",
  "format_version": 9, /* the compiled layout */
  "name": "mod",
  "entry": {
    "lib": "./mod.rut", // trailing prose
  },
}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("mod.rut"),
        "pub fn seven() -> i32 { return 7; }\nfn main() -> i32 { return seven(); }\n",
    )
    .unwrap();
    let (_, root) = rut_driver::load_dir_session(&dir, &FsSource).unwrap();
    assert_eq!(root, "mod");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_directory_still_holding_rut_json_gets_the_pointed_refusal() {
    // the retired name: no fallback lane reads it — the refusal names
    // the file and the cutover recipe
    let base = std::env::temp_dir().join(format!("rut-old-name-{}", std::process::id()));
    let dir = base.join("old");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rut.json"), make_dir_manifest()).unwrap();
    std::fs::write(
        dir.join("mod.rut"),
        "pub fn seven() -> i32 { return 7; }\nfn main() -> i32 { return seven(); }\n",
    )
    .unwrap();
    let err = rut_driver::load_dir_session(&dir, &FsSource).unwrap_err().to_string();
    assert!(err.contains("rut.json"), "{err}");
    assert!(err.contains("rut.jsonc"), "{err}");
    assert!(err.contains("re-name the file or re-pack the directory"), "{err}");
    let _ = std::fs::remove_dir_all(&base);
}
