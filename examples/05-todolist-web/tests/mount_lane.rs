//! P3 of the appkit-retirement proof (restructure survey §2.5/§5.2):
//! THE TWO LANES AGREE. The native lane walks the MANIFEST
//! (`load_dir_with` over `rut/` — the manifest is the truth); the
//! wasm lane offers the MIRROR (`mirror_pkgs` — one `Pkg` per package,
//! because wasm has no filesystem and the app's source crosses the
//! ABI). The mirror cannot drift: this test pins the two closures to
//! the same offered-name set, the same host-fn surface, and — the
//! strongest form — the same compiled binary. (P1 is every green
//! suite here: the packages offer SEPARATELY, no concatenation
//! anywhere; P2 lives in `app_law.rs`.)

use std::collections::BTreeSet;

use todolist_web::mount;

#[test]
fn the_manifest_lane_and_the_mirror_lane_agree() {
    // the manifest: the rut/ project root and its closure — the walk's
    // own yield, `web` offered (the embedder half)
    let manifest = mount::project().expect("the rut/ project mounts");
    assert_eq!(manifest.root, "app", "the project root is the app");

    // the mirror: the same closure, offered by hand
    let mirror = mount::mirror_pkgs().expect("the mirror mounts");

    // same offered-name set. The mirror holds every package BUT the
    // root — the app's source crosses the ABI (loader.js fetches
    // rut/app/app.rut), which is the one thing an in-memory offer
    // cannot fetch.
    let mut manifest_names: BTreeSet<String> =
        manifest.pkgs.iter().map(|p| p.spec.clone()).collect();
    let mut mirror_names: BTreeSet<String> =
        mirror.iter().map(|p| p.spec.clone()).collect();
    // the `web` surface rides NEITHER lane's walk — both offer it by
    // hand (the embedder half); the mirror's app source crosses the ABI
    manifest_names.insert("web".to_string());
    mirror_names.insert("app".to_string());
    assert_eq!(
        manifest_names, mirror_names,
        "the mirror drifted from the manifest — mirror_pkgs must offer \
         exactly rut/biz/rut.jsonc's closure (minus the ABI root)"
    );

    // same host surface: the `.d.rut` pkg rides the manifest (its
    // registration scope IS the pkg name), and the mirror states the
    // same rows
    assert_eq!(
        mount::expected_host_fns(&manifest),
        rut_driver::declared_host_fns(&mirror),
        "the two lanes declare different host surfaces"
    );

    // and the strongest form: the same compiled binary. Both lanes
    // compile the same closure from the same root source — identical
    // programs is what "the manifest is the truth, the mirror is its
    // mirror" means when it is TRUE.
    let from_manifest = mount::verified(&mount::compile_walk(&manifest).expect("the manifest lane compiles"))
        .expect("the manifest lane's binary verifies");
    let src = mount::biz_source();
    let from_mirror = mount::verified(
        &mount::compile_app(mount::mirror_run().expect("the mirror mounts"), &src)
            .expect("the mirror lane compiles"),
    )
    .expect("the mirror lane's binary verifies");
    assert_eq!(
        rut_core::binary::encode(&from_manifest),
        rut_core::binary::encode(&from_mirror),
        "the two lanes compiled different programs — the mirror drifted"
    );
}
