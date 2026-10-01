//! The session mount — TWO LANES over ONE closure (survey §2.4).
//!
//! THE MANIFEST (the native lane's truth): `rut/biz/` is the project
//! root — the TWO-PACKAGE law's biz side — and every package it names
//! (ui, and through ui pouch/nmapset/nmap_host) is mounted by
//! [`crate::mount::project_dir`] through the Loader — the manifest's
//! ui row is a local `path`, the std closure rides pinned jsDelivr url
//! rows served by the PRIMED OFFLINE REMOTE ([`crate::mount::std_remote`];
//! dist/std plays the wire, no gate networks). The four passes run FOR
//! REAL. The manifest, not a Rust fn,
//! is the module list.
//!
//! THE MIRROR (the wasm lane): the Session is I/O-free by law (wasm
//! hosts mount in memory), so `mount_app_session` registers the same
//! closure by hand — one `register_module` per package, the same
//! mount properties the manifests state, everything
//! `include_str!` (the rut-wasm ink/ink_host precedent). Nothing here knows
//! the app's SOURCE: the root's body crosses the ABI (loader.js hands
//! over `rut/biz/biz.rut`) and [`compile_app`] compiles it as the
//! root.
//!
//! [`crate::mount::mount_app_session`] is the mirror of
//! `rut/biz/rut.jsonc` and nothing more — `tests/mount_lane.rs` (the P3
//! proof) pins the two lanes to the same mounted-name set, the same
//! host surface, and the same compiled binary. The `limits()`/
//! `compile_app` halves below are shared by both lanes.
//!
//! THE HOST DECL SURFACE: `web.d.rut` lives in `web/` as a proper host
//! pkg (the shape law folded it in; it still rides NO deps row — the
//! two-package law's count is unchanged, and the host crossing is not
//! a mounted package). BOTH lanes register it by hand: the embedder
//! mounts what the closure uses (the `mount_std_core` precedent).
//!
//! What died here before this file's current shape: the old
//! concatenation module (ONE module fabricated from the store
//! and t1 sources) — dep-kinds landed and the workaround's reason was
//! gone (survey §2.5, P1/P2/P3). What the
//! TWO-PACKAGE law retired on top: the sixteen per-concept manifests —
//! t1's three, the components' eight, the store's three, the app's
//! three — merged into ui (framework) and biz (domain + app), one
//! module each, because rut's landed visibility is `pub` or
//! module-private and each layer keeps its privates private for real
//! in one file.

use rut_core::binary::Program;
use rut_driver::{Module, ModuleBody};

// ---- the packages, embedded verbatim --------------------------------
// The mirror reads the SAME files the manifest names. wasm32 has no
// filesystem, so everything rides `include_str!` — one code path for
// both lanes, the ink/pouch precedent.

/// The `web` pkg's surface, verbatim — the contract the bodies bind.
pub const WEB_D_RUT: &str = include_str!("../web/web.d.rut");

const POUCH_RUT: &str = include_str!("../../../rut/pouch/pouch.rut");
// the nmap host surface, from the COMMITTED CDN artifact (the v6 decl
// bundle — `dist/std/nmap_host.rutbundle`, sha256-pinned by the pins
// gate). THE BUNDLE LAW: a compiled decl surface binds like the
// hand-lowered one — the mirror's `register_module` becomes a
// `mount_bundle_bytes`, the wasm lane's url-dep mount. pouch and
// nmapset stay verbatim sources: they are GENERIC owners, and a
// compiled bundle serves only the instantiations its own pack closure
// spelled (consumer shapes like this page's `Vec<Node>` are the
// directory lane — see docs/src/reference/bundles.md).
const NMAP_HOST_BUNDLE: &[u8] = include_bytes!("../../../dist/std/nmap_host.rutbundle");
const NMAPSET_RUT: &str = include_str!("../../../rut/nmapset/nmapset.rut");

const UI_RUT: &str = include_str!("../rut/ui/ui.rut");
// ui's `entry.libs` tail (rut.jsonc) — the mirror spells the manifest's
// file list BY HAND (the mirror's whole job: an independent embedder
// spelling what the manifest says; mount_lane pins the two together)
const UI_STORE_RUT: &str = include_str!("../rut/ui/store.rut");
const UI_WIDGET_RUT: &str = include_str!("../rut/ui/widget.rut");
const UI_LOWERING_RUT: &str = include_str!("../rut/ui/lowering.rut");
const UI_DIFF_RUT: &str = include_str!("../rut/ui/diff.rut");
const UI_COMPONENTS_RUT: &str = include_str!("../rut/ui/components.rut");

/// ui's source, spliced the manifest's way: base first,
/// then `entry.libs` in array order, '\n'-joined — ONE module.
pub fn ui_source() -> String {
    let mut src = String::from(UI_RUT);
    for part in [UI_STORE_RUT, UI_WIDGET_RUT, UI_LOWERING_RUT, UI_DIFF_RUT, UI_COMPONENTS_RUT] {
        src.push('\n');
        src.push_str(part);
    }
    src
}

/// biz's source, spliced the same way (biz.rut + its `entry.libs`
/// tail) — the handed-over ABI source both the native mirror
/// (mount_lane) and the wasm lane (loader.js) compose.
pub fn biz_source() -> String {
    let mut src = String::from(BIZ_RUT);
    for part in [DOMAIN_RUT, WORLD_RUT, APP_RUT] {
        src.push('\n');
        src.push_str(part);
    }
    src
}
const BIZ_RUT: &str = include_str!("../rut/biz/biz.rut");
const DOMAIN_RUT: &str = include_str!("../rut/biz/domain.rut");
const WORLD_RUT: &str = include_str!("../rut/biz/world.rut");
const APP_RUT: &str = include_str!("../rut/biz/app.rut");

/// The rut/ project root — the manifest lane's mount point. Native
/// only: the wasm lane has no filesystem and uses the mirror.
#[cfg(not(target_arch = "wasm32"))]
pub fn project_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rut").join("biz")
}

/// The MANIFEST LANE's mount (native only): the rut/biz project root
/// through the Loader — the four passes for real, the url rows served
/// by the primed offline remote ([`std_remote`]) — plus
/// the embedder half every lane owns: the `core` prelude AND the `web`
/// host surface (the crossing is no package's dep; both lanes register
/// it by hand — the registration scope IS the pkg name (`web::*`).
/// `tests/mount_lane.rs` pins the two lanes together.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_project_session() -> Result<(rut_driver::Session, String), String> {
    let (mut session, root) = load_dir_with_std(&project_dir())?;
    rut_driver::mount_std_core(&mut session);
    register_web_surface(&mut session)?;
    Ok((session, root))
}

/// A probe's module dir — `tests/<probe>/`. Each probe under tests/ is
/// its own rut program (its own `rut.jsonc`), the manifest lane's truth
/// for the test suites exactly like `rut/` is for the app.
#[cfg(not(target_arch = "wasm32"))]
pub fn probe_dir(probe: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(probe)
}

/// The PROBE lanes' mount (native only): the Loader over
/// `tests/<probe>/` — the probe's own manifest names its deps (the
/// sibling ui row stays a local `path`; the std rows are pinned
/// jsDelivr urls; `web` rides no row) — plus the embedder half every lane owns:
/// the `core` prelude AND the `web` host surface. The softfail fixture
/// is the deliberate exception: no crossings is its point, so it loads
/// its dir bare + `mount_std_core` (its test spells that shape).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_probe_session(probe: &str) -> Result<(rut_driver::Session, String), String> {
    let (mut session, root) = load_dir_with_std(&probe_dir(probe))?;
    rut_driver::mount_std_core(&mut session);
    register_web_surface(&mut session)?;
    Ok((session, root))
}

/// The manifest lane's directory load: the Loader (its default source
/// IS the filesystem) over [`std_remote`] — the same embedder shape
/// src/main.rs of the other examples spells, with the offline flavor
/// swapped in. One noop-waker poll settles the READY futures: every
/// url row is a primed cache hit, so nothing ever awaits the wire.
#[cfg(not(target_arch = "wasm32"))]
fn load_dir_with_std(
    dir: &std::path::Path,
) -> Result<(rut_driver::Session, String), String> {
    use std::future::Future;
    let fut = rut_driver::Loader::new(dir).dep_remote(std_remote()).build().load();
    let mut fut = std::pin::pin!(fut);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match fut.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => return v.map_err(|e| e.to_string()),
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// dist/std plays the wire: the committed artifacts prime ONE offline
/// cache per process (`DepRemote::write` is the stand-in for the GET —
/// the ONCE-gated priming is the 03-plugin precedent), and every
/// manifest-lane load rides an `HttpRemote::offline` over it — a miss
/// NEVER touches the network, so the gate cannot network by
/// construction. The pin's bytes are the committed bytes
/// (`pack-std --check` green); the CDN is only the human lane. The
/// cache root lives in the tmp dir: a throwaway, not the project-local
/// `.rut/cache` a `rut run` would warm.
#[cfg(not(target_arch = "wasm32"))]
fn std_remote() -> rut_driver::HttpRemote {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    rut_driver::HttpRemote::offline(ROOT.get_or_init(|| {
        let root = std::env::temp_dir()
            .join(format!("rut-05-todolist-web-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // every manifest this example loads with deps — the app's two
        // packages and the two probes that name the std closure
        // (harness/softfail are dep-free) — walked for url rows exactly
        // like the loader's own prefetch walks
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let dist = base.join("../../dist/std");
        let mut urls = std::collections::BTreeSet::new();
        for rel in [
            "rut/biz/rut.jsonc",
            "rut/ui/rut.jsonc",
            "tests/store_probe/rut.jsonc",
            "tests/t1_harness/rut.jsonc",
        ] {
            let Ok(text) = std::fs::read_to_string(base.join(rel)) else { continue };
            let Ok(manifest) = rut_driver::bundle::parse_manifest(&text) else { continue };
            for desc in manifest.deps.values() {
                if let Some(url) = desc.get("url") {
                    urls.insert(url.clone());
                }
            }
        }
        let warmer = rut_driver::HttpRemote::offline(&root);
        for url in &urls {
            let artifact = url.rsplit('/').next().unwrap_or_default();
            let bytes = std::fs::read(dist.join(artifact)).unwrap_or_else(|e| {
                panic!("the committed artifact is the cache — {artifact}: {e}")
            });
            rut_driver::DepRemote::write(&warmer, url, &bytes).expect("prime the cache");
        }
        root
    }))
}

/// Register the `web` DECL surface — the embedder half both lanes run.
/// The registration scope IS the package name (`web::*`: the prefix
/// every crossing's host id carries).
pub fn register_web_surface(session: &mut rut_driver::Session) -> Result<(), String> {
    let web = rut_driver::lower_decl_module(WEB_D_RUT, "web.d.rut")?;
    session.register_module("web", web).map_err(|e| e.to_string())
}

/// Mount `core` + `pouch` + the `web` host surface. `core` is mounted
/// unconditionally by the engine anyway; `calc` is NOT mounted — the
/// crossings need no float surface.
pub fn mount_host_session(session: &mut rut_driver::Session) -> Result<(), String> {
    rut_driver::mount_std_core(session);
    session
        .register_module(
            "pouch",
            Module { body: ModuleBody::Source { text: POUCH_RUT.to_string(), is_decl: false }, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;
    register_web_surface(session)?;
    Ok(())
}

/// Compile a handed-in root source against a mounted session under an
/// explicit spec, and hand back the verified binary program — the
/// mirror lane's compile half for hand-over sources.
pub fn compile_root(
    session: &mut rut_driver::Session,
    src: &str,
    spec: &str,
) -> Result<Program, String> {
    let out = rut_driver::compile_module_in(session, src, rut_parser::Mode::Impl, spec);
    verified(out.diags.iter().map(|d| d.msg.clone()).collect(), out.binary)
}

pub fn compile_app(
    session: &mut rut_driver::Session,
    src: &str,
) -> Result<Program, String> {
    compile_root(session, src, "app")
}

/// Compile a session's already-mounted graph from its root spec — THE
/// MANIFEST LANE's compile half.
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_manifest(session: &rut_driver::Session, root: &str) -> Result<Program, String> {
    let out = rut_driver::compile_graph(session, root);
    match out.program {
        Some(p) => verified(
            out.diags.iter().map(|d| d.msg.clone()).collect(),
            Some(rut_core::binary::encode(&p)),
        ),
        None => verified(
            out.diags.iter().map(|d| d.msg.clone()).collect(),
            None,
        ),
    }
}

/// The shared tail of both compile halves: diags are the error text,
/// the binary decodes and verifies before it is a program.
fn verified(msgs: Vec<String>, binary: Option<Vec<u8>>) -> Result<Program, String> {
    if !msgs.is_empty() {
        return Err(format!("the app does not compile: {}", msgs.join("; ")));
    }
    let bin = binary.ok_or("the app compiled to no binary")?;
    let prog = rut_core::binary::decode(&bin).map_err(|e| format!("binary decode: {e}"))?;
    rut_vm::verify::verify(&prog).map_err(|e| format!("verify: {e}"))?;
    Ok(prog)
}

/// The twin's budgets (survey §5.5 — the 00-todolist numbers; each turn
/// is bounded, so latency never feeds fuel).
pub fn limits() -> rut_vm::interp::Limits {
    rut_vm::interp::Limits {
        // fuel is DISABLED for this example: fuel is an embedding-host
        // scheduling slice (a turn that outlives one parks and waits
        // for the host to resume), and this page's pump runs turns to
        // completion — a slice cap here only means long journeys die
        // mid-turn the moment the board grows past the budget (the
        // crash the long-journey tests pin). The heap limit stays: the
        // resource guard that matters is memory, not instructions.
        fuel: None,
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    }
}

/// The app session — THE MIRROR of `rut/biz/rut.jsonc` (survey §2.4):
/// every package the manifest names, registered with the manifest's
/// own mount properties. The ROOT is deliberately absent: the app's
/// SOURCE crosses the ABI ([`compile_app`] registers and compiles it),
/// which is the one thing the I/O-free Session cannot fetch.
///
/// Install `nmap_host`'s bodies with `rut_std::nmap::pkg()` next to
/// [`crate::hosts::install_web_hosts`].
pub fn mount_app_session(session: &mut rut_driver::Session) -> Result<(), String> {
    mount_host_session(session)?;

    // the nmap host surface, from the CDN artifact (the v6 decl bundle)
    // — the wasm lane's url-dep mount; the registration scope IS the
    // package name the bundle answers to
    let mounted = rut_driver::mount_bundle_bytes(session, NMAP_HOST_BUNDLE).map_err(|e| e.to_string())?;
    debug_assert_eq!(mounted, "nmap_host");
    session
        .register_module(
            "nmapset",
            Module { body: ModuleBody::Source { text: NMAPSET_RUT.to_string(), is_decl: false }, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;

    // THE UI PACKAGE — the framework: the atom store machinery, the
    // widget type, the lowering table, the keyed diff, the component
    // vocabulary — ONE module (the two-package law). The
    // multi-lib files ride through ui_source() — the manifest's
    // `entry.libs`, spliced base-first in array order.
    session
        .register_module(
            "ui",
            Module { body: ModuleBody::Source { text: ui_source(), is_decl: false }, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}
