//! Path loading — module directories (`rut.toml`) and `.rutbundle`
//! archives: a v5 bundle is the COMPILED contract (`.rutc` binaries +
//! scope ledger + mixed source groups), and the refusals gate it
//! (version, corruption, missing groups).

use std::path::Path;

use rut_driver::bundle::FsSource;
use rut_driver::{load_bundle_session, load_bundle_bytes, load_dir_session, load_path_session, pack_dir};

/// A one-file module in a temp dir (one directory, one entry file).
fn make_dir(base: &Path) -> std::path::PathBuf {
    let dir = base.join("mod");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rut.toml"),
        // bundle-shaped: the keys a `rut pack` needs are already there,
        // and directory loading ignores them (layout v5 — the compiled
        // format; the packer emits v5 and the loader reads v5 only)
        "format = \"rutbundle\"\nformat_version = 5\nname = \"mod\"\nentry.lib = \"./mod.rut\"\n",
    )
    .unwrap();
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
    let src = "fn main() -> i32 { return 7; }\n".as_bytes();

    // no rut.toml entry at all
    let not_a_bundle = rut_driver::bundle::write_bundle(&[("x.rut".into(), src.to_vec())]).unwrap();
    let err = rut_driver::load_bundle_bytes(&not_a_bundle, Path::new("a")).unwrap_err().to_string();
    assert!(err.contains("rut.toml"), "{err}");

    // v1–v4 layouts are refused by the one-line version gate — the
    // same refusal an OLDER loader applies to a version it does not
    // know, before reading anything else (refuse, never guess)
    for v in [1u8, 2, 3, 4, 7, 99] {
        let manifest = format!(
            "format = \"rutbundle\"\nformat_version = {v}\nname = \"x\"\nentry.lib = \"./x.rut\"\n"
        );
        let old = rut_driver::bundle::write_bundle(&[
            ("rut.toml".into(), manifest.as_bytes().to_vec()),
            ("x.rut".into(), src.to_vec()),
        ])
        .unwrap();
        let err = rut_driver::load_bundle_bytes(&old, Path::new("b")).unwrap_err().to_string();
        assert!(err.contains("format_version"), "{err}");
        assert!(err.contains("reads bundle format_version 5 (compiled) and 6 (decl) only"), "{err}");
        assert!(err.contains("re-pack the directory"), "{err}");
    }

    // missing `format = "rutbundle"`
    let manifest = "format_version = 5\nname = \"x\"\nentry.lib = \"./x.rut\"\n";
    let no_format = rut_driver::bundle::write_bundle(&[
        ("rut.toml".into(), manifest.as_bytes().to_vec()),
        ("x.rut".into(), src.to_vec()),
    ])
    .unwrap();
    let err = rut_driver::load_bundle_bytes(&no_format, Path::new("c")).unwrap_err().to_string();
    assert!(err.contains("rutbundle"), "{err}");

    // corruption: flip a payload byte, the CRC check fires (gate 1 —
    // before any manifest is read)
    let dir = make_dir(&base);
    let mut bytes = pack_dir(&dir).unwrap();
    let at = 30 + "name = \"mod\"\nentry.lib = \"./mod.rut\"\n".len();
    bytes[at] ^= 0x01;
    let err = rut_driver::load_bundle_bytes(&bytes, Path::new("h")).unwrap_err().to_string();
    assert!(err.contains("CRC"), "{err}");

    // a pack whose declared surface file is missing refuses loudly
    // (the surface is a lib pkg's declared `entry.type` — a doc surface;
    // no host rows: those live only in `type = "host"` pkgs)
    let dir = make_dir(&base);
    std::fs::write(dir.join("surface.d.rut"), "/// the pkg's surface doc.\n").unwrap();
    std::fs::write(
        dir.join("rut.toml"),
        "format = \"rutbundle\"\nformat_version = 5\nname = \"mod\"\nentry.lib = \"./mod.rut\"\nentry.type = \"./surface.d.rut\"\n",
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
        s.join("rut.toml"),
        "name = \"s\"\ntype = \"host\"\nentry.type = \"./s.d.rut\"\n",
    )
    .unwrap();
    std::fs::write(s.join("s.d.rut"), "pub host fn ping(x: i32) -> i32;\n").unwrap();
    // the consumer: m (+ transitively s)
    let m = base.join("m");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(
        m.join("rut.toml"),
        "format = \"rutbundle\"\nformat_version = 5\nname = \"main\"\nentry.lib = \"./main.rut\"\n[deps]\ns = { path = \"../s\" }\n",
    )
    .unwrap();
    std::fs::write(
        m.join("main.rut"),
        "pub fn main() -> i32 { return 7; }\n",
    )
    .unwrap();

    let bytes = pack_dir(&m).unwrap();

    // TWIN 1 — the same group re-spelled `type = "lib"` (and nothing
    // else changed): the group's surface now rides the LIB arm, and its
    // host rows refuse at load, naming the fix
    let entries = rut_driver::bundle::parse_bundle(&bytes).unwrap();
    let lib_manifest =
        "name = \"s\"\ntype = \"lib\"\nentry.type = \"./s.d.rut\"\n".as_bytes().to_vec();
    let respelled: Vec<(String, Vec<u8>)> = entries
        .iter()
        .map(|(n, b)| {
            if n == "s/rut.toml" {
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
        s2.join("rut.toml"),
        "name = \"s\"\ntype = \"lib\"\nentry.type = \"./s.d.rut\"\n",
    )
    .unwrap();
    std::fs::write(s2.join("s.d.rut"), "/// documented surface, no body yet.\n").unwrap();
    let m2 = base.join("m2");
    std::fs::create_dir_all(&m2).unwrap();
    std::fs::write(
        m2.join("rut.toml"),
        "format = \"rutbundle\"\nformat_version = 5\nname = \"main\"\nentry.lib = \"./main.rut\"\n[deps]\ns = { path = \"../s2\" }\n",
    )
    .unwrap();
    std::fs::write(
        m2.join("main.rut"),
        "pub fn main() -> i32 { return 7; }\n",
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
    std::fs::write(m.join("rut.toml"), "name = \"m\"\nentry.lib = \"./m.rut\"\n").unwrap();
    std::fs::write(m.join("m.rut"), "pub fn four() -> i32 { return 4; }\n").unwrap();
    // the dep's dep: a host pkg (declaration-only surface) → source
    let s = base.join("s");
    std::fs::create_dir_all(&s).unwrap();
    std::fs::write(
        s.join("rut.toml"),
        "name = \"s\"\ntype = \"host\"\nentry.type = \"./s.d.rut\"\n",
    )
    .unwrap();
    std::fs::write(s.join("s.d.rut"), "pub host fn ping(x: i32) -> i32;\n").unwrap();
    // m uses s
    std::fs::write(
        m.join("rut.toml"),
        "name = \"m\"\nentry.lib = \"./m.rut\"\n[deps]\ns = { path = \"../s\" }\n",
    )
    .unwrap();

    // the consumer: m (+ transitively s)
    let main = base.join("consumer");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::write(
        main.join("rut.toml"),
        "format = \"rutbundle\"\nformat_version = 5\nname = \"main\"\nentry.lib = \"./entry.rut\"\n[deps]\nm = { path = \"../m\" }\n",
    )
    .unwrap();
    std::fs::write(
        main.join("entry.rut"),
        "use m::{ four };\npub fn main() -> i32 { return four(); }\n",
    )
    .unwrap();

    let bytes = pack_dir(&main).unwrap();
    let entries = rut_driver::bundle::parse_bundle(&bytes).unwrap();
    let mut names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["m/m.rutc", "m/rut.toml", "main.rutc", "rut.scopes", "rut.toml", "s/rut.toml", "s/s.d.rut"],
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
