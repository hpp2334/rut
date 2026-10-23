//! source-zip deps — the `.zip` transport end to end: `rut pack`'s
//! source lane (`pack_dir_zip`) embeds the whole closure under
//! `deps/<name>@<fp16>/`, the consumer's walk unpacks it once
//! (content-addressed) and mounts an ordinary directory package, and
//! an embedded `.rutbundle` dep rides through the wire lane. The pin
//! law and the pack-time tree gates hold at every door.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rut_driver::sha256_hex;
use rut_native::{load_dir_fetched, pack_dir, pack_dir_zip};

// ------------------------------------------------------------------
// the fixture world

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-zip-deps-{tag}-{}", std::process::id()));
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

/// A lib pkg over the auto core: one pub fn the consumer calls.
fn lib_pkg(dir: &Path, name: &str) {
    write(dir, "rut.jsonc", &format!(r#"{{ "name": "{name}" }}"#));
    write(
        dir,
        "src/mod.rut",
        &format!("pub fn answer() -> i64 {{ return {name}_answer(); }}\nfn {name}_answer() -> i64 {{ return 42; }}\n"),
    );
}

/// A host pkg (a pure `.d.rut` surface) — the zip rides it without a
/// `src/mod.rut`.
fn host_pkg(dir: &Path, name: &str) {
    write(
        dir,
        "rut.jsonc",
        &format!(
            r#"{{ "name": "{name}", "type": "host", "entry": {{ "type": "./{name}.d.rut" }} }}"#
        ),
    );
    write(
        dir,
        &format!("src/{name}.d.rut"),
        "pub host fn ping(x: i32) -> i32;\n",
    );
}

fn run_entry<R: rut_vm::interp::Ret>(loaded: rut_driver::Walk, entry: &str) -> R {
    let g = loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(
        g.graph.diags.is_empty(),
        "{:?}",
        g.graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let flat = rut_core::link::flatten(g.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, R>(entry, ()).expect("run")
}

// ------------------------------------------------------------------
// the lanes

#[test]
fn a_source_zip_dep_packs_mounts_and_runs() {
    let root = scratch("e2e");
    // the dep, packed as a source zip
    let util = root.join("util");
    lib_pkg(&util, "util");
    let zip = pack_dir_zip(&util).expect("pack the dep as a zip");
    // the zip's shape: the manifest + src/ + the closure under deps/
    let entries = rut_driver::bundle::read_srczip(&zip).expect("read back");
    assert!(
        entries.iter().any(|(n, _)| n == "src/mod.rut"),
        "{:?}",
        entries.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    // the consumer: the zip by url, pinned, loaded from the fetched map
    let app = root.join("app");
    let url = "https://cdn.example/std-v14/dist/std/util.zip";
    write(
        &app,
        "rut.jsonc",
        r#"{
  "name": "app",
  "deps": {
    "util": { "url": "https://cdn.example/std-v14/dist/std/util.zip" }
  }
}"#,
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{answer};\nentry fn go() -> i64 { return answer(); }\n",
    );
    let zip_sha = sha256_hex(&zip);
    write(
        &app,
        "rut.lock",
        &format!(
            r#"{{ "format": "rutlock", "format_version": 1, "deps": [{{ "pkg": "util", "url": "{url}", "sha256": "{zip_sha}" }}] }}"#
        ),
    );
    let mut map = BTreeMap::new();
    map.insert(url.to_string(), zip.clone());
    let loaded = load_dir_fetched(&app, &map).expect("mount the zip dep");
    assert_eq!(loaded.root, "app");
    let answer: i64 = run_entry(loaded, "go");
    assert_eq!(answer, 42);
}

#[test]
fn a_packed_zip_embeds_its_path_deps_and_the_consumer_sees_them() {
    let root = scratch("embed");
    // ink's shape: a lib over a host surface, the host a path dep
    let host = root.join("ink_host");
    host_pkg(&host, "ink_host");
    let ink = root.join("ink");
    write(
        &ink,
        "rut.jsonc",
        r#"{
  // ink — the lib over the host surface (a path dep, the dev shape)
  "format": "rutbundle",
  "format_version": 12,
  "name": "ink",
  "deps": {
    "ink_host": { "path": "../ink_host" }
  }
}"#,
    );
    write(
        &ink,
        "src/mod.rut",
        "pub fn tag() -> str { return \"ink\"; }\n",
    );
    let zip = pack_dir_zip(&ink).expect("pack");
    let entries = rut_driver::bundle::read_srczip(&zip).expect("read back");
    // the host dep rode under its identity dir, and the rewritten
    // manifest points inside
    assert!(
        entries
            .iter()
            .any(|(n, _)| n.starts_with("deps/ink_host@") && n.ends_with("/src/ink_host.d.rut")),
        "{:?}",
        entries.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    let manifest = entries
        .iter()
        .find(|(n, _)| n == "rut.jsonc")
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .unwrap();
    assert!(
        manifest.contains(r#""path": "deps/ink_host@"#),
        "the row points inside: {manifest}"
    );
    assert!(
        !manifest.contains("../ink_host") && !manifest.contains("rutbundle\""),
        "the external path and the wire rows drop: {manifest}"
    );
    // the consumer runs it (the embedded host surface mounts through
    // the identity dir)
    let app = root.join("app");
    let url = "https://cdn.example/std-v14/dist/std/ink.zip";
    write(
        &app,
        "rut.jsonc",
        &format!(r#"{{ "name": "app", "deps": {{ "ink": {{ "url": "{url}" }} }} }}"#),
    );
    write(
        &app,
        "src/mod.rut",
        "use ink::{tag};\nentry fn go() -> str { return tag(); }\n",
    );
    let zip_sha = sha256_hex(&zip);
    write(
        &app,
        "rut.lock",
        &format!(
            r#"{{ "format": "rutlock", "format_version": 1, "deps": [{{ "pkg": "ink", "url": "{url}", "sha256": "{zip_sha}" }}] }}"#
        ),
    );
    let mut map = BTreeMap::new();
    map.insert(url.to_string(), zip.clone());
    let loaded = load_dir_fetched(&app, &map).expect("mount");
    let tag: String = run_entry(loaded, "go");
    assert_eq!(tag, "ink");
}

#[test]
fn a_rutbundle_url_dep_embeds_as_ir_and_loads_through_the_wire_lane() {
    let root = scratch("ir");
    // the dep, packed as a compiled bundle
    let util = root.join("util");
    lib_pkg(&util, "util");
    let bundle = pack_dir(&util).expect("pack the dep as a rutbundle");
    // the consumer packs it into ITS zip — the IR file rides verbatim
    let app = root.join("app");
    let url = "https://cdn.example/dist/util.rutbundle";
    write(
        &app,
        "rut.jsonc",
        &format!(r#"{{ "name": "app", "deps": {{ "util": {{ "url": "{url}" }} }} }}"#),
    );
    write(
        &app,
        "src/mod.rut",
        "use util::{answer};\npub fn the_answer() -> i64 { return answer(); }\n",
    );
    let bundle_sha = sha256_hex(&bundle);
    write(
        &app,
        "rut.lock",
        &format!(
            r#"{{ "format": "rutlock", "format_version": 1, "deps": [{{ "pkg": "util", "url": "{url}", "sha256": "{bundle_sha}" }}] }}"#
        ),
    );
    let mut map = BTreeMap::new();
    map.insert(url.to_string(), bundle);
    let zip = rut_native::pack_dir_zip_fetched(&app, &map).expect("pack the consumer's zip");
    let entries = rut_driver::bundle::read_srczip(&zip).expect("read back");
    assert!(
        entries
            .iter()
            .any(|(n, _)| n.starts_with("deps/util@") && n.ends_with("/util.rutbundle")),
        "the IR file rides under the dep's identity: {:?}",
        entries.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    // consuming THAT zip mounts the IR dir through the wire lane
    let fan = root.join("fan");
    let zip_url = "https://cdn.example/dist/app.zip";
    write(
        &fan,
        "rut.jsonc",
        &format!(r#"{{ "name": "fan", "deps": {{ "app": {{ "url": "{zip_url}" }} }} }}"#),
    );
    write(
        &fan,
        "src/mod.rut",
        "use app::{the_answer};\nentry fn go() -> i64 { return the_answer(); }\n",
    );
    let zip_sha = sha256_hex(&zip);
    write(
        &fan,
        "rut.lock",
        &format!(
            r#"{{ "format": "rutlock", "format_version": 1, "deps": [{{ "pkg": "app", "url": "{zip_url}", "sha256": "{zip_sha}" }}] }}"#
        ),
    );
    let mut map2 = BTreeMap::new();
    map2.insert(zip_url.to_string(), zip);
    let loaded = load_dir_fetched(&fan, &map2).expect("mount the zip, then the IR file inside");
    let answer: i64 = run_entry(loaded, "go");
    assert_eq!(answer, 42);
}

#[test]
fn the_same_zip_packs_byte_identically() {
    let root = scratch("det");
    let util = root.join("util");
    lib_pkg(&util, "util");
    let a = pack_dir_zip(&util).expect("pack");
    let b = pack_dir_zip(&util).expect("pack again");
    assert_eq!(a, b, "same input ⇒ byte-identical zip");
}

#[test]
fn a_pin_mismatch_refuses_at_the_mount_door() {
    let root = scratch("pin");
    let util = root.join("util");
    lib_pkg(&util, "util");
    let zip = pack_dir_zip(&util).expect("pack");
    let app = root.join("app");
    let url = "https://cdn.example/dist/util.zip";
    write(
        &app,
        "rut.jsonc",
        &format!(r#"{{ "name": "app", "deps": {{ "util": {{ "url": "{url}" }} }} }}"#),
    );
    write(
        &app,
        "src/mod.rut",
        "entry fn main() -> i64 { return 1; }\n",
    );
    write(
        &app,
        "rut.lock",
        &format!(
            r#"{{ "format": "rutlock", "format_version": 1, "deps": [{{ "pkg": "util", "url": "{url}", "sha256": "{}" }}] }}"#,
            "0".repeat(64)
        ),
    );
    let mut map = BTreeMap::new();
    map.insert(url.to_string(), zip);
    let err = load_dir_fetched(&app, &map).expect_err("the pin is law");
    assert!(err.to_string().contains("sha256 pin mismatch"), "{err}");
}

#[test]
fn the_pack_tree_gates_refuse_stray_and_undeclared_rut_code() {
    let root = scratch("gates");
    let util = root.join("util");
    lib_pkg(&util, "util");
    // a stray `.rut` beside the manifest — outside the src shell
    write(&util, "stray.rut", "fn stray() {}\n");
    let err = pack_dir_zip(&util).expect_err("gate A");
    assert!(
        err.to_string()
            .contains("rut code rides only under a `src/` shell"),
        "{err}"
    );
    std::fs::remove_file(util.join("stray.rut")).unwrap();
    // an undeclared module under src/ — nothing mounts it
    write(&util, "src/orphan.rut", "fn orphan() {}\n");
    let err = pack_dir_zip(&util).expect_err("gate B");
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    std::fs::remove_file(util.join("src/orphan.rut")).unwrap();
    // clean again, the pack succeeds
    pack_dir_zip(&util).expect("clean tree packs");
}
