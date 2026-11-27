//! source-zip deps — the `.zip` transport end to end: `rut pack`'s
//! source lane (`pack_dir_zip`) embeds the whole closure under
//! `deps/<name>@<fp16>/`, the consumer's walk unpacks it once
//! (content-addressed) and mounts an ordinary directory package, and
//! an embedded `.rutbundle` dep rides through the wire lane. The pin
//! law and the pack-time tree gates hold at every door.

use std::collections::BTreeMap;
use std::path::Path;

use rut_driver::sha256_hex;
use rut_native::{load_dir_fetched, pack_dir, pack_dir_zip};

// ------------------------------------------------------------------
// the fixture world

fn scratch(tag: &str) -> rut_testkit::Scratch {
    rut_testkit::Scratch::new(&format!("zip-deps-{tag}"))
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
        .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
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
    let url = "https://cdn.example/std-v19/dist/std/util.zip";
    write(
        &app,
        "rut.jsonc",
        r#"{
  "name": "app",
  "deps": {
    "util": { "url": "https://cdn.example/std-v19/dist/std/util.zip" }
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
    let url = "https://cdn.example/std-v19/dist/std/ink.zip";
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

#[test]
fn the_bin_mode_refuses_a_src_orphan_like_the_zip_mode() {
    // F53: both pack modes gate the same tree. An undeclared `.rut`
    // under `src/` is not quietly dropped by the compiled lane.
    let root = scratch("bin-orphan");
    let util = root.join("util");
    lib_pkg(&util, "util");
    write(&util, "src/orphan.rut", "fn orphan() {}\n");
    let err = match pack_dir(&util) {
        Ok(bytes) => panic!(
            "the bin mode packed an orphan silently ({} bytes) — the file is dropped",
            bytes.len()
        ),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    std::fs::remove_file(util.join("src/orphan.rut")).unwrap();
    // clean again, the bin pack succeeds
    pack_dir(&util).expect("clean tree packs");
}

#[test]
fn a_nested_custom_root_packs_its_declared_children() {
    // F54: a host root spelled `./app/mod.d.rut` records its children
    // relative to the ROOT FILE's directory (`tool/mod.d.rut`), while
    // the gate compares against the `src/`-relative path
    // (`app/tool/mod.d.rut`). The legal layout must pack.
    let root = scratch("nested-root");
    let hh = root.join("hh");
    write(
        &hh,
        "rut.jsonc",
        r#"{ "name": "hh", "type": "host", "entry": { "type": "./app/mod.d.rut" } }"#,
    );
    write(&hh, "src/app/mod.d.rut", "pub mod tool;\n");
    write(
        &hh,
        "src/app/tool/mod.d.rut",
        "pub host fn helper() -> i32;\n",
    );
    let zip = pack_dir_zip(&hh).expect("a nested custom root is a legal layout");
    let entries = rut_driver::bundle::read_srczip(&zip).expect("read back");
    assert!(
        entries.iter().any(|(n, _)| n == "src/app/tool/mod.d.rut"),
        "{:?}",
        entries.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
}

#[test]
fn the_bin_mode_refuses_a_host_roots_orphan_like_the_zip_mode() {
    // F53, the host arm: the host surface's tree is gated in BOTH
    // modes — the bin lane's host early return must not skip it.
    let root = scratch("bin-host-orphan");
    let hh = root.join("hh");
    write(
        &hh,
        "rut.jsonc",
        r#"{ "name": "hh", "type": "host", "entry": { "type": "./app/mod.d.rut" } }"#,
    );
    write(&hh, "src/app/mod.d.rut", "pub mod tool;\n");
    write(
        &hh,
        "src/app/tool/mod.d.rut",
        "pub host fn helper() -> i32;\n",
    );
    write(&hh, "src/app/orphan.d.rut", "pub host fn stray() -> i32;\n");
    let err = match pack_dir(&hh) {
        Ok(bytes) => panic!(
            "the bin mode packed a host orphan silently ({} bytes) — the file is dropped",
            bytes.len()
        ),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    let err = pack_dir_zip(&hh).expect_err("the source lane gates the same tree");
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    std::fs::remove_file(hh.join("src/app/orphan.d.rut")).unwrap();
    // clean again, BOTH modes pack
    pack_dir(&hh).expect("clean host tree packs in bin mode");
    pack_dir_zip(&hh).expect("clean host tree packs in source mode");
}

#[test]
fn the_bin_mode_refuses_a_dir_deps_orphan_like_the_zip_mode() {
    // F53, the group trees: every PkgSource::Dir the compiled lane reads
    // is gated like the source lane's collect_tree recursion.
    let root = scratch("bin-dep-orphan");
    let helper = root.join("helper");
    lib_pkg(&helper, "helper");
    write(&helper, "src/orphan.rut", "fn orphan() {}\n");
    let app = root.join("app");
    write(
        &app,
        "rut.jsonc",
        r#"{ "name": "app", "deps": { "helper": { "path": "../helper" } } }"#,
    );
    write(
        &app,
        "src/mod.rut",
        "use helper::{answer};\nentry fn go() -> i64 { return answer(); }\n",
    );
    let err = match pack_dir(&app) {
        Ok(bytes) => panic!(
            "the bin mode packed a dep's orphan silently ({} bytes) — the file is dropped",
            bytes.len()
        ),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    let err = pack_dir_zip(&app).expect_err("the source lane gates the same tree");
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    std::fs::remove_file(helper.join("src/orphan.rut")).unwrap();
    // clean again, BOTH modes pack
    pack_dir(&app).expect("clean dep tree packs in bin mode");
    pack_dir_zip(&app).expect("clean dep tree packs in source mode");
}

#[test]
fn a_lib_documentation_surface_never_declares_a_colliding_stray() {
    // F54, law 7: the prefix arm reads the MOUNT's own root — always
    // `src/mod.rut` for a lib pkg — never the manifest's documentation
    // surface. A lib's nested `entry.type` must not whitelist a stray
    // whose path collides with a real child's record under it.
    let root = scratch("lib-surface-stray");
    let lib = root.join("libsurf");
    write(
        &lib,
        "rut.jsonc",
        r#"{ "name": "libsurf", "type": "lib", "entry": { "type": "./app/out.d.rut" } }"#,
    );
    write(&lib, "src/mod.rut", "pub mod tool;\n");
    write(
        &lib,
        "src/tool/mod.rut",
        "pub fn tool() -> i64 { return 1; }\n",
    );
    write(
        &lib,
        "src/app/out.d.rut",
        "/// documented surface, no body yet.\n",
    );
    // the clean tree (the documentation surface + the real child)
    // packs in BOTH modes — the surface file itself stays declared
    pack_dir(&lib).expect("clean lib tree packs in bin mode");
    pack_dir_zip(&lib).expect("clean lib tree packs in source mode");
    // the colliding stray: the child record `tool/mod.rut` must NOT
    // gain the documentation surface's `app/` prefix
    write(&lib, "src/app/tool/mod.rut", "fn stray() {}\n");
    let err = match pack_dir(&lib) {
        Ok(bytes) => panic!(
            "the bin mode packed a colliding stray silently ({} bytes) — the file is dropped",
            bytes.len()
        ),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    let err = pack_dir_zip(&lib).expect_err("the source lane gates the same tree");
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    // the control: a non-colliding stray refuses in both modes
    std::fs::remove_file(lib.join("src/app/tool/mod.rut")).unwrap();
    write(&lib, "src/app/tool/modx.rut", "fn stray() {}\n");
    let err = pack_dir(&lib).expect_err("a non-colliding stray refuses in bin mode");
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
    let err = pack_dir_zip(&lib).expect_err("a non-colliding stray refuses in source mode");
    assert!(
        err.to_string().contains("nothing declares this module"),
        "{err}"
    );
}
