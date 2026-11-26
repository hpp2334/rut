//! A child mounts BESIDE its declaring file on every lane. A
//! single-file parent `src/m/a.rut` declaring `mod b;` mounts its
//! child at the parent FILE's directory — `src/m/b.rut` (or
//! `src/m/b/mod.rut`) — never under a directory named after the
//! parent's mod path (`src/m/a/b/…`). The rule has one home,
//! `rut_driver::mods::child_dir`; each lane's lookup reads it.
//!
//! One pin per lane: the FS walk (green — the reference), the pack
//! gate, the source-zip read, and the bundle.

use std::path::Path;

use rut_driver::Walk;

fn scratch(tag: &str) -> rut_testkit::Scratch {
    rut_testkit::Scratch::new(&format!("mount-beside-{tag}"))
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

const ROOT: &str = "pub mod m;\nuse m::a::b::{ bf };\nentry fn main() -> i32 { return bf(); }\n";
const CHILD: &str = "pub fn bf() -> i32 { return 1; }\n";

/// The plan's shape: the single-file parent `src/m/a.rut` declaring
/// `mod b;`. `dir_form` selects the directory-form child
/// `src/m/b/mod.rut` over the single-file `src/m/b.rut`.
fn proj(tag: &str, dir_form: bool) -> rut_testkit::Scratch {
    let root = scratch(tag);
    let pkg = root.join("proj");
    write(&pkg, "rut.jsonc", "{ \"name\": \"proj\" }\n");
    write(&pkg, "src/mod.rut", ROOT);
    write(&pkg, "src/m/mod.rut", "pub mod a;\n");
    write(&pkg, "src/m/a.rut", "pub mod b;\n");
    write(
        &pkg,
        if dir_form {
            "src/m/b/mod.rut"
        } else {
            "src/m/b.rut"
        },
        CHILD,
    );
    root
}

/// The child's root-relative record — the file beside `src/m/a.rut`.
fn record(dir_form: bool) -> &'static str {
    if dir_form {
        "m/b/mod.rut"
    } else {
        "m/b.rut"
    }
}

/// The source zip's entries for the same shape, written by hand so the
/// zip lane is tested on its own (not through the pack gate).
fn zip_entries(dir_form: bool) -> Vec<(String, Vec<u8>)> {
    let mut entries = vec![
        (
            "rut.jsonc".to_string(),
            b"{ \"name\": \"proj\" }\n".to_vec(),
        ),
        ("src/mod.rut".to_string(), ROOT.as_bytes().to_vec()),
        ("src/m/mod.rut".to_string(), b"pub mod a;\n".to_vec()),
        ("src/m/a.rut".to_string(), b"pub mod b;\n".to_vec()),
    ];
    entries.push((
        format!("src/{}", record(dir_form)),
        CHILD.as_bytes().to_vec(),
    ));
    entries
}

fn run(walk: Walk) -> i32 {
    let c = walk
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
        .entrypoint(&walk.root)
        .compile()
        .expect("compile the walk");
    assert!(
        c.graph.diags.is_empty(),
        "{:?}",
        c.graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
    );
    let flat = rut_core::link::flatten(c.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(rut_vm::interp::Limits {
            fuel: Some(2_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        })
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run main")
}

/// The mounted tree's mod path → file records.
fn records(walk: &Walk) -> Vec<(String, String)> {
    let pkg = walk.pkg("proj").expect("proj mounted");
    pkg.mods
        .iter()
        .map(|(k, m)| (k.clone(), m.file.clone()))
        .collect()
}

#[test]
fn the_file_lane_mounts_a_child_beside_a_single_file_parent() {
    for dir_form in [false, true] {
        let root = proj(if dir_form { "file-dir" } else { "file" }, dir_form);
        let walk = rut_native::load_dir(&root.join("proj")).expect("load the walk");
        let recs = records(&walk);
        assert!(
            recs.contains(&("m/a/b".to_string(), record(dir_form).to_string())),
            "{recs:?}"
        );
        assert_eq!(run(walk), 1, "dir_form={dir_form}");
    }
}

#[test]
fn the_pack_lane_accepts_a_child_beside_a_single_file_parent() {
    for dir_form in [false, true] {
        let root = proj(if dir_form { "pack-dir" } else { "pack" }, dir_form);
        let zip = rut_native::pack_dir_zip(&root.join("proj"))
            .expect("the pack gate accepts a mounted child");
        let entries = rut_driver::bundle::read_srczip(&zip).expect("read the packed zip");
        let want = format!("src/{}", record(dir_form));
        assert!(
            entries.iter().any(|(n, _)| *n == want),
            "the packed zip carries {want}: {entries:?}"
        );
    }
}

#[test]
fn the_source_zip_lane_mounts_a_child_beside_a_single_file_parent() {
    for dir_form in [false, true] {
        let entries = zip_entries(dir_form);
        let bytes = rut_driver::bundle::write_srczip(&entries).expect("write the zip");
        let walk = rut_driver::Mounted::from_srczip(&bytes)
            .expect("the zip lane mounts the child beside its declaring file");
        let recs = records(&walk);
        assert!(
            recs.contains(&("m/a/b".to_string(), record(dir_form).to_string())),
            "{recs:?}"
        );
        assert_eq!(run(walk), 1, "dir_form={dir_form}");
    }
}

#[test]
fn the_bundle_lane_mounts_a_child_beside_a_single_file_parent() {
    for dir_form in [false, true] {
        let root = proj(if dir_form { "bundle-dir" } else { "bundle" }, dir_form);
        let bytes = rut_native::pack_dir(&root.join("proj")).expect("pack the bundle");
        let walk = rut_driver::Bundle::parse(&bytes)
            .and_then(|b| b.walk())
            .expect("load the bundle");
        let keys: Vec<&str> = walk
            .pkg("proj")
            .expect("proj mounted")
            .mods
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["m", "m/a", "m/a/b"], "dir_form={dir_form}");
        assert_eq!(run(walk), 1, "dir_form={dir_form}");
    }
}

#[test]
fn a_child_of_a_single_file_parent_mounts_on_every_lane() {
    for dir_form in [false, true] {
        let root = proj(if dir_form { "all-dir" } else { "all" }, dir_form);
        let pkg = root.join("proj");
        // 1. the disk walk
        assert_eq!(
            run(rut_native::load_dir(&pkg).expect("disk lane")),
            1,
            "disk, dir_form={dir_form}"
        );
        // 2. the pack gate, then 3. the packed source zip
        let zip = rut_native::pack_dir_zip(&pkg).expect("pack lane");
        assert_eq!(
            run(rut_driver::Mounted::from_srczip(&zip).expect("source-zip lane")),
            1,
            "source zip, dir_form={dir_form}"
        );
        // 4. the compiled bundle
        let bundle = rut_native::pack_dir(&pkg).expect("bundle lane");
        let walk = rut_driver::Bundle::parse(&bundle)
            .and_then(|b| b.walk())
            .expect("bundle walk");
        assert_eq!(run(walk), 1, "bundle, dir_form={dir_form}");
    }
}
