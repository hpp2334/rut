//! Multi-lib entries: a package authored as several
//! `.rut` files — the base `entry.lib` plus an ordered `entry.libs`
//! tail — spliced into ONE module: one namespace, one visibility
//! scope, manifest array order as the canonical splice order.
//!
//! Pinned here:
//! - one namespace: names private to one file resolve from another
//!   (a lib file calls a base fn; the base calls a lib fn), and a
//!   consumer imports a pub name a LIB file declares;
//! - the splice order: the source body is base, then libs in array
//!   order, '\n'-joined — the determinism law (never a dir listing);
//! - the manifest laws: `libs` without `lib`, a `.d.rut` element, a
//!   duplicated file — each one loud error;
//! - bundles: the multi-lib pkg compiles to its `.rutc` (the splice
//!   already happened at pack), so the archive carries the binary, not
//!   the files; the packed form compiles identically to the directory

use rut_driver::bundle::FsSource;
use rut_driver::{load_bundle_bytes, load_dir_session, pack_dir};
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-multilib-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

const BASE: &str = "// kid.rut — the base\nfn base_part() -> i32 { return 20; }\n";
const LIB_B: &str = "// part_b.rut — the first lib: uses the base's private fn\npub fn lib_sum() -> i32 { return base_part() + tail_part(); }\n";
const LIB_C: &str = "// part_c.rut — the second lib\nfn tail_part() -> i32 { return 3; }\n";

/// kid/ + consumer/ under one scratch root; `libs` spelled or omitted
/// per test, bundle keys per `version` (None = plain directory pkg).
fn world(tag: &str, libs: Option<&str>, version: Option<u64>) -> PathBuf {
    let root = scratch(tag);
    let kid = root.join("kid");
    let bundle_keys = match version {
        None => String::new(),
        Some(v) => format!("format = \"rutbundle\"\nformat_version = {v}\n"),
    };
    let libs_row = libs
        .map(|l| format!("entry.libs = [{l}]\n"))
        .unwrap_or_default();
    write(
        &kid,
        "rut.toml",
        &format!(
            "{bundle_keys}name = \"kid\"\nentry.lib = \"./kid.rut\"\n{libs_row}"
        ),
    );
    write(&kid, "kid.rut", BASE);
    write(&kid, "part_b.rut", LIB_B);
    write(&kid, "part_c.rut", LIB_C);
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "{bundle_keys}name = \"app\"\nentry.lib = \"./entry.rut\"\n[deps]\nkid = {{ path = \"../kid\" }}\n"
        ),
    );
    write(
        &app,
        "entry.rut",
        "use kid::{lib_sum};\npub fn main() -> i32 { return lib_sum(); }\n",
    );
    root
}

fn run_main(session: rut_driver::Session, root: &str) -> i64 {
    let g = rut_driver::compile_graph(&session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run main").into()
}

#[test]
fn multi_lib_is_one_module_in_manifest_order() {
    // c before b in the array: the splice order is the LIST's, never
    // alphabetical, never a directory listing
    let root = world("order", Some("\"./part_c.rut\", \"./part_b.rut\""), None);
    let (session, app_root) = load_dir_session(&root.join("app"), &FsSource).expect("load");
    let kid = session.resolve("kid").expect("kid mounted");
    let rut_driver::ModuleBody::Source { text: src, .. } = &kid.body else {
        panic!("kid has a source body");
    };
    assert_eq!(
        src.as_str(),
        format!("{BASE}\n{LIB_C}\n{LIB_B}"),
        "the splice is base, then libs in manifest order, '\\n'-joined"
    );
    // one namespace + the consumer's import of a LIB-declared pub fn
    assert_eq!(run_main(session, &app_root), 23);
}

#[test]
fn manifest_libs_laws_are_loud() {
    // `libs` without `lib` — the base is what the tail is a tail ON
    let root = world("nolib", Some("\"./part_b.rut\""), None);
    let kid = root.join("kid");
    let text = std::fs::read_to_string(kid.join("rut.toml")).unwrap();
    std::fs::write(&kid.join("rut.toml"), text.replacen("entry.lib = \"./kid.rut\"\n", "", 1)).unwrap();
    let err = load_dir_session(&kid, &FsSource).unwrap_err().to_string();
    assert!(err.contains("needs `entry.lib`"), "{err}");

    // a `.d.rut` element — a decl surface is not a body
    let root = world("declrut", Some("\"./part_b.rut\", \"./x.d.rut\""), None);
    let err = load_dir_session(&root.join("kid"), &FsSource).unwrap_err().to_string();
    assert!(err.contains("`.d.rut`"), "{err}");

    // the base named again in the tail — the splice would duplicate it
    let root = world("dup", Some("\"./part_b.rut\", \"./kid.rut\""), None);
    let err = load_dir_session(&root.join("kid"), &FsSource).unwrap_err().to_string();
    assert!(err.contains("twice"), "{err}");

    // a missing file — the exact path, never a silent skip
    let root = world("gone", Some("\"./gone.rut\""), None);
    let err = load_dir_session(&root.join("kid"), &FsSource).unwrap_err().to_string();
    assert!(err.contains("gone.rut"), "{err}");
}

#[test]
fn multi_lib_packs_compiled_and_loads_identically() {
    let root = world("v5", Some("\"./part_b.rut\", \"./part_c.rut\""), Some(5));
    let bytes = pack_dir(&root.join("app")).expect("pack");
    // determinism: same dir ⇒ byte-identical bundle
    assert_eq!(bytes, pack_dir(&root.join("app")).unwrap());
    // the archive carries the compiled binaries — the splice already
    // happened at pack time, so the lib FILES do not ride
    let names: Vec<String> =
        rut_driver::bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    for key in ["app.rutc", "rut.scopes", "kid/kid.rutc"] {
        assert!(names.contains(&key.to_string()), "the bundle must carry `{key}`: {names:?}");
    }
    assert!(!names.iter().any(|n| n.ends_with(".rut")), "no source rides a linkable pkg: {names:?}");
    let (session, app_root) = load_bundle_bytes(&bytes, Path::new("mem")).expect("load");
    assert_eq!(run_main(session.clone(), &app_root), 23);

    // the packed form compiles to the SAME linked binary as the directory
    let (dir_session, dir_root) = load_dir_session(&root.join("app"), &FsSource).expect("dir load");
    let linked = |s: &rut_driver::Session, r: &str| {
        let g = rut_driver::compile_graph(s, r);
        assert!(g.diags.is_empty(), "{:?}", g.diags);
        rut_core::binary::encode(&g.program.expect("linked"))
    };
    assert_eq!(linked(&dir_session, &dir_root), linked(&session, &app_root));
}
