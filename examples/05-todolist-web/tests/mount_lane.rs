//! P3 of the appkit-retirement proof (restructure survey §2.5/§5.2):
//! THE TWO LANES AGREE. The native lane mounts the MANIFEST
//! (`load_dir_session` over `rut/` — the manifest is the truth); the
//! wasm lane mounts the MIRROR (`mount_app_session` — one
//! `register_module` per package, because the Session is I/O-free and
//! the app's source crosses the ABI). The mirror cannot drift: this
//! test pins the two closures to the same mounted-name set, the same
//! host-fn surface, and — the strongest form — the same compiled
//! binary. (P1 is every green suite here: the packages mount
//! SEPARATELY, no concatenation anywhere; P2 lives in `app_law.rs`.)

use std::collections::BTreeSet;

use todolist_web::mount;

#[test]
fn the_manifest_lane_and_the_mirror_lane_agree() {
    // the manifest: the rut/ project root and its closure — through
    // the lane's own mount helper, core included (the embedder half)
    let (manifest, root) = mount::load_project_session()
        .expect("the rut/ project mounts");
    assert_eq!(root, "app", "the project root is the app");

    // the mirror: the same closure, registered by hand
    let mut mirror = rut_driver::Session::new();
    mount::mount_app_session(&mut mirror).expect("the mirror mounts");

    // same mounted-name set. The mirror holds every package BUT the
    // root — the app's source crosses the ABI (loader.js fetches
    // rut/app/app.rut), which is the one thing an I/O-free Session
    // cannot fetch.
    let manifest_names: BTreeSet<String> =
        manifest.modules().map(|(s, _)| s.clone()).collect();
    let mut mirror_names: BTreeSet<String> =
        mirror.modules().map(|(s, _)| s.clone()).collect();
    mirror_names.insert("app".to_string());
    assert_eq!(
        manifest_names, mirror_names,
        "the mirror drifted from the manifest — mount_app_session must \
         register exactly rut/biz/rut.toml's closure (minus the ABI root)"
    );

    // same host surface: the `.d.rut` pkg and its scope ride the
    // manifest's host_scope, and the mirror states the same value
    assert_eq!(
        manifest.expected_host_fns(),
        mirror.expected_host_fns(),
        "the two lanes declare different host surfaces"
    );

    // and the strongest form: the same compiled binary. Both lanes
    // compile the same closure from the same root source — identical
    // programs is what "the manifest is the truth, the mirror is its
    // mirror" means when it is TRUE.
    let from_manifest = mount::compile_manifest(&manifest, &root)
        .expect("the manifest lane compiles");
    let src = mount::biz_source();
    let from_mirror = mount::compile_app(&mut mirror, &src)
        .expect("the mirror lane compiles");
    assert_eq!(
        rut_core::binary::encode(&from_manifest),
        rut_core::binary::encode(&from_mirror),
        "the two lanes compiled different programs — the mirror drifted"
    );
}
