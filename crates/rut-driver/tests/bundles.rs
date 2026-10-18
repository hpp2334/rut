//! Path loading — module directories (`rut.jsonc`) and `.rutbundle`
//! archives: a v11 bundle is the COMPILED contract (`mod.rutc` binaries
//! + scope ledger + `deps/` identity groups), and the refusals gate it
//! (version, corruption, missing groups).

use std::path::Path;

use rut_driver::Mounted;
use rut_native::{load_bundle_session, load_dir, load_path_session, pack_dir};

mod common;
use common::{dep_dir, pkg_named};



/// The make_dir manifest's text — also the corruption test's offset
/// reference (the first zip entry's payload).

/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Mounted::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            // the root DECLARES its calc use (the structural form; the
            // offer stays the walked tree pkg)
            .pkg(
                rut_driver::Mounted::source(spec, src)
                    .manifest(rut_driver::Manifest::lib(spec, &["calc"])),
            )
            .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
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

fn make_dir_manifest() -> String {
    // bundle-shaped: the keys a `rut pack` needs are already there,
    // and directory loading ignores them (layout 12 — the current
    // wire; the packer emits 12 and the loader reads 12 plus the old
    // 11 lane forever)
    r#"{"format": "rutbundle", "format_version": 12, "name": "mod"}"#.to_string()
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

fn linked_binary(loaded: &rut_driver::Walk) -> Vec<u8> {
    let g = loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    rut_core::binary::encode(&g.graph.program.expect("linked program"))
}

#[test]
fn bundle_round_trip_matches_the_directory() {
    let base = std::env::temp_dir().join(format!("rut-bundle-test-{}", std::process::id()));
    let dir = make_dir(&base);

    // the directory form
    let loaded = load_dir(&dir).unwrap();
    let from_dir = linked_binary(&loaded);

    // the packed form: same sources, same linked bytes
    let bytes = pack_dir(&dir).unwrap();
    let bundle_path = base.join("mod.rutbundle");
    std::fs::write(&bundle_path, &bytes).unwrap();
    let loaded = load_bundle_session(&bundle_path).unwrap();
    let from_bundle = linked_binary(&loaded);
    assert_eq!(from_dir, from_bundle, "dir and bundle compile identically");

    // dispatch agrees on both forms
    let s1 = load_path_session(&dir).unwrap();
    let s2 = load_path_session(&bundle_path).unwrap();
    assert_eq!(linked_binary(&s1), from_dir);
    assert_eq!(linked_binary(&s2), from_dir);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn in_memory_bytes_load_without_a_file() {
    let base = std::env::temp_dir().join(format!("rut-bundle-mem-{}", std::process::id()));
    let dir = make_dir(&base);
    let bytes = pack_dir(&dir).unwrap();
    let loaded = rut_driver::Mounted::from_bundle(&bytes).unwrap();
    assert_eq!(loaded.root, "mod");
    // the root mounts as a compiled body — the decoded `.rutc`
    assert!(matches!(
        loaded.pkg("mod").unwrap().body,
        rut_driver::MountedBody::Compiled(_)
    ));
    assert!(
        loaded
            .pkgs
            .iter()
            .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
            .entrypoint(&loaded.root)
            .compile()
            .unwrap()
            .graph
            .program
            .is_some()
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn refusals() {
    let base = std::env::temp_dir().join(format!("rut-bundle-ref-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let src = "entry fn main() -> i32 { return 7; }\n".as_bytes();

    // no rut.jsonc entry at all
    let not_a_bundle = rut_driver::bundle::write_bundle(&[("x.rut".into(), src.to_vec())]).unwrap();
    let err = rut_driver::Mounted::from_bundle(&not_a_bundle).unwrap_err().to_string();
    assert!(err.contains("rut.jsonc"), "{err}");

    // anything outside the wire family refuses at the one-line version
    // gate — the same refusal an OLDER loader applies to a version it
    // does not know, before reading anything else (refuse, never
    // guess). ONE uniform re-pack recipe: the old wires, the reserved
    // band, anything. (11 stays readable — the old lane, a published
    // bundle is forever old.)
    for v in [1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10, 13, 99, 100] {
        let manifest = format!(
            r#"{{"format": "rutbundle", "format_version": {v}, "name": "x", "entry": {{"lib": "./x.rut"}}}}"#
        );
        let old = rut_driver::bundle::write_bundle(&[
            ("rut.jsonc".into(), manifest.as_bytes().to_vec()),
            ("x.rut".into(), src.to_vec()),
        ])
        .unwrap();
        let err = rut_driver::Mounted::from_bundle(&old).unwrap_err().to_string();
        assert!(err.contains("format_version"), "{err}");
        assert!(err.contains("reads bundle format_version 12 (and the old 11 lane) only"), "{err}");
        assert!(err.contains("re-pack the directory"), "{err}");
    }

    // a manifest riding under the retired ENTRY NAME falls to the
    // generic refusal: the reader routes on `rut.jsonc` only — no
    // `rut.jsonc` entry, not a rut bundle
    let old_name = rut_driver::bundle::write_bundle(&[
        (
            "rut.json".into(),
            br#"{"format": "rutbundle", "format_version": 11, "name": "x", "entry": {"lib": "./x.rut"}}"#
                .to_vec(),
        ),
        ("x.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::Mounted::from_bundle(&old_name)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no `rut.jsonc` entry"), "{err}");
    assert!(err.contains("not a rut bundle"), "{err}");

    // missing `format = "rutbundle"`
    let manifest = r#"{"format_version": 11, "name": "x", "entry": {"lib": "./x.rut"}}"#;
    let no_format = rut_driver::bundle::write_bundle(&[
        ("rut.jsonc".into(), manifest.as_bytes().to_vec()),
        ("x.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::Mounted::from_bundle(&no_format).unwrap_err().to_string();
    assert!(err.contains("rutbundle"), "{err}");

    // corruption: flip a payload byte, the CRC check fires (gate 1 —
    // before any manifest is read)
    let dir = make_dir(&base);
    let mut bytes = pack_dir(&dir).unwrap();
    let at = 30 + make_dir_manifest().len();
    bytes[at] ^= 0x01;
    let err = rut_driver::Mounted::from_bundle(&bytes).unwrap_err().to_string();
    assert!(err.contains("CRC"), "{err}");

    // a pack whose declared surface file is missing refuses loudly
    // (the surface is a lib pkg's declared `entry.type` — a doc surface;
    // no host rows: those live only in `type = "host"` pkgs)
    let dir = make_dir(&base);
    std::fs::write(dir.join("surface.d.rut"), "/// the pkg's surface doc.\n").unwrap();
    std::fs::write(
        dir.join("rut.jsonc"),
        r#"{"format": "rutbundle", "format_version": 12, "name": "mod", "entry": {"type": "./surface.d.rut"}}"#,
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
        r#"{"format": "rutbundle", "format_version": 12, "name": "main", "deps": {"s": {"path": "../s"}}}"#,
    )
    .unwrap();
    std::fs::write(
        m.join("mod.rut"),
        "entry fn main() -> i32 { return 7; }\n",
    )
    .unwrap();

    let bytes = pack_dir(&m).unwrap();

    // TWIN 1 — the same group re-spelled `type = "lib"` (and nothing
    // else changed): the surface rides the LIB arm at load, and its
    // host rows refuse, naming the fix. The twin packs FRESH — a
    // doctored in-archive manifest would refuse earlier, at the
    // fingerprint-over-content law (the dirname would no longer hash
    // to its group).
    let s_lib = base.join("s_lib");
    std::fs::create_dir_all(&s_lib).unwrap();
    std::fs::write(
        s_lib.join("rut.jsonc"),
        r#"{"name": "s", "type": "lib", "entry": {"type": "./s.d.rut"}}"#,
    )
    .unwrap();
    std::fs::write(s_lib.join("s.d.rut"), "pub host fn ping(x: i32) -> i32;\n").unwrap();
    let m_lib = base.join("m_lib");
    std::fs::create_dir_all(&m_lib).unwrap();
    std::fs::write(
        m_lib.join("rut.jsonc"),
        r#"{"format": "rutbundle", "format_version": 12, "name": "main", "deps": {"s": {"path": "../s_lib"}}}"#,
    )
    .unwrap();
    std::fs::write(
        m_lib.join("mod.rut"),
        "entry fn main() -> i32 { return 7; }\n",
    )
    .unwrap();
    let lib_bytes = pack_dir(&m_lib);
    let err = lib_bytes.unwrap_err().to_string();
    assert!(err.contains("s.d.rut"), "{err}");
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
        r#"{"format": "rutbundle", "format_version": 12, "name": "main", "deps": {"s": {"path": "../s2"}}}"#,
    )
    .unwrap();
    std::fs::write(
        m2.join("mod.rut"),
        "entry fn main() -> i32 { return 7; }\n",
    )
    .unwrap();
    let dev_bytes = pack_dir(&m2).unwrap();
    let session = rut_driver::Mounted::from_bundle(&dev_bytes).unwrap();
    let sf = pkg_named(&session, "s");
    assert!(
        matches!(
            sf.body,
            rut_driver::MountedBody::Source { is_decl: true, .. }
        ),
        "the surface-only group is a decl unit: {:?}",
        sf.body
    );
    let g = session
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
        .entrypoint(&session.root)
        .compile()
        .unwrap();
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    assert!(g.graph.program.is_some());

    // and the declared host spelling still mounts as a host body (the
    // kind is honored from the field, pack to load)
    let session = Mounted::from_bundle(&bytes).unwrap();
    assert!(matches!(
        pkg_named(&session, "s").body,
        rut_driver::MountedBody::Host { .. }
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
    std::fs::write(m.join("rut.jsonc"), r#"{"name": "m"}"#).unwrap();
    std::fs::write(m.join("mod.rut"), "pub fn four() -> i32 { return 4; }\n").unwrap();
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
        r#"{"name": "m", "deps": {"s": {"path": "../s"}}}"#,
    )
    .unwrap();

    // the consumer: m (+ transitively s)
    let main = base.join("consumer");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::write(
        main.join("rut.jsonc"),
        r#"{"format": "rutbundle", "format_version": 12, "name": "main", "deps": {"m": {"path": "../m"}}}"#,
    )
    .unwrap();
    std::fs::write(
        main.join("mod.rut"),
        "use m::{ four };\nentry fn main() -> i32 { return four(); }\n",
    )
    .unwrap();

    let bytes = pack_dir(&main).unwrap();
    let entries = rut_driver::bundle::parse_bundle(&bytes).unwrap();
    // v12: the root's own files at the archive root (canonical names —
    // the GENERATED declaration entry beside the binary), each dep
    // group under its content identity (a compiled group rides its own
    // generated entry; the host group its consolidated one)
    let names: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
    let m_dir = dep_dir(&names, "m");
    let s_dir = dep_dir(&names, "s");
    let mut sorted = names.clone();
    sorted.sort();
    let mut want = vec![
        "mod.d.rut".to_string(),
        "mod.rutc".to_string(),
        "rut.jsonc".to_string(),
        "rut.scopes".to_string(),
        format!("{m_dir}/mod.d.rut"),
        format!("{m_dir}/mod.rutc"),
        format!("{m_dir}/rut.jsonc"),
        format!("{s_dir}/mod.d.rut"),
        format!("{s_dir}/rut.jsonc"),
    ];
    want.sort();
    assert_eq!(
        sorted, want,
        "the closure rides compiled, the host pkg as source, all under deps/<identity>/"
    );

    // determinism with deps too
    assert_eq!(bytes, pack_dir(&main).unwrap(), "same dir => byte-identical bundle");

    // the packed form compiles like the directory form
    let bundle = base.join("main.rutbundle");
    std::fs::write(&bundle, &bytes).unwrap();
    let session = load_path_session(&bundle).unwrap();
    assert_eq!(session.root, "main");
    assert!(matches!(
        session.pkg("main").expect("m mounted").body,
        rut_driver::MountedBody::Compiled(_)
    ));
    let sf = pkg_named(&session, "s");
    assert!(
        matches!(sf.body, rut_driver::MountedBody::Host { .. }),
        "the host pkg rode along as a host body"
    );
    // the first-mount edge + the structural manifest: a group records
    // the ARCHIVE ROOT as its puller, and every archive pkg rides its
    // own in-archive manifest
    let mf = pkg_named(&session, "m");
    assert_eq!(sf.mounted_by.as_deref(), Some("main"), "the archive root pulled its groups");
    assert_eq!(mf.mounted_by.as_deref(), Some("main"));
    assert!(session.pkg("main").unwrap().mounted_by.is_none(), "the bundle root has no parent");
    assert_eq!(mf.manifest.name.as_deref(), Some("m"));
    assert!(mf.manifest.deps.contains_key("s"), "the group's own dep rows ride");
    let g = session
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
        .entrypoint(&session.root)
        .compile()
        .unwrap();
    assert!(g.graph.diags.is_empty(), "{:?}", g.graph.diags);
    assert!(g.graph.program.is_some());

    // a v11 bundle missing a declared dep group is a load error naming it
    let stripped: Vec<(String, Vec<u8>)> = entries
        .iter()
        .filter(|(n, _)| !n.starts_with(&format!("{m_dir}/")))
        .cloned()
        .collect();
    let bytes = rut_driver::bundle::write_bundle(&stripped).unwrap();
    let err = rut_driver::Mounted::from_bundle(&bytes).unwrap_err().to_string();
    assert!(err.contains("missing its `m` dependency group"), "{err}");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn entry_keys_admit_identities_under_deps() {
    // THE @-ADMISSION LAW (wire v11): a group's entries ride at
    // `deps/<name>@<fp16>/…` — `@` is not in the spec alphabet, so an
    // identity can never collide with a real package name, and the
    // entry-key law admits it inside the `deps/` component: the key
    // normalizer answers it, and the container writes + reads the
    // entry back byte-for-byte.
    let key = "deps/util@0123456789abcdef/mod.rutc";
    assert_eq!(
        rut_driver::bundle::bundle_key(key).unwrap().as_str(),
        key,
        "an identity dirname passes the entry-key law",
    );
    // the traversal laws keep biting around it
    assert!(rut_driver::bundle::bundle_key("deps/../util@0123456789abcdef/mod.rutc").is_err());
    assert!(rut_driver::bundle::bundle_key("/deps/util@0123456789abcdef/mod.rutc").is_err());
    let bytes = rut_driver::bundle::write_bundle(&[(
        key.to_string(),
        b"RUTC-payload".to_vec(),
    )])
    .expect("the container writes identity entries");
    let parsed = rut_driver::bundle::parse_bundle(&bytes).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].0, key);
    assert_eq!(parsed[0].1, b"RUTC-payload");
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
  "format_version": 11, /* the one wire number */
  "name": "mod", /* beside the value */
  "style": { "indent": "2", }, // a trailing comma, the JSONC way
}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("mod.rut"),
        "pub fn seven() -> i32 { return 7; }\nfn main() -> i32 { return seven(); }\n",
    )
    .unwrap();
    let loaded = rut_native::load_dir(&dir).unwrap();
    let root = loaded.root;
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
    let err = rut_native::load_dir(&dir).unwrap_err().to_string();
    assert!(err.contains("rut.json"), "{err}");
    assert!(err.contains("rut.jsonc"), "{err}");
    assert!(err.contains("re-name the file or re-pack the directory"), "{err}");
    let _ = std::fs::remove_dir_all(&base);
}
