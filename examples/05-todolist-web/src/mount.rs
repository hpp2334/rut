//! The run chain — TWO LANES over ONE closure (survey §2.4), both
//! spelled as offers:
//!
//! THE MANIFEST (the native lane's truth): `rut/biz/` is the project
//! root — the TWO-PACKAGE law's biz side — and every package it names
//! (ui, and through ui pouch/nmapset/nmap_host) is walked by
//! [`walk_dir`] with the primed OFFLINE REMOTE ([`std_remote`];
//! dist/std-v8 plays the wire — the PINNED artifacts, so no gate
//! networks). The four passes run FOR
//! REAL. The manifest, not a Rust fn, is the pkg list. [`probe_dir`]
//! walks a test probe the same way.
//!
//! THE MIRROR (the wasm lane): wasm has no filesystem, so
//! [`mirror_pkgs`] offers the same closure by hand — one `Pkg` per
//! package, the same mount properties the manifests state, everything
//! `include_str!`/`include_bytes!` (the rut-wasm ink/ink_host
//! precedent). Nothing here knows the app's SOURCE: the root's TREE
//! crosses the ABI (loader.js hands over the module rows wire) and
//! [`compile_app`] compiles it as the root.
//!
//! `tests/mount_lane.rs` (the P3 proof) pins the two lanes to the same
//! mounted-name set, the same host surface, and the same compiled
//! binary. The `limits()`/compile halves below are shared by both
//! lanes.
//!
//! THE HOST DECL SURFACE: `web.d.rut` lives in `web/` as a proper host
//! pkg (the shape law folded it in; it still rides NO deps row — the
//! two-package law's count is unchanged, and the host crossing is not
//! a mounted package). BOTH lanes offer it by hand: the embedder
//! offers what the closure uses — [`web_pkg`], already part of every
//! `*_run` chain here.
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
use rut_driver::{Compiled, Loaded, Pkg, RutRun};

// ---- the packages, embedded verbatim --------------------------------
// The mirror reads the SAME files the manifest names. wasm32 has no
// filesystem, so everything rides `include_str!` — one code path for
// both lanes, the ink/pouch precedent.

/// The `web` pkg's surface, verbatim — the contract the bodies bind.
pub const WEB_D_RUT: &str = include_str!("../web/web.d.rut");

const POUCH_RUT: &str = include_str!("../../../rut/pouch/mod.rut");
// the nmap host surface, from the COMMITTED CDN artifact (the v6 decl
// bundle — `dist/std/nmap_host.rutbundle`, sha256-pinned by the pins
// gate). THE BUNDLE LAW: a compiled decl surface binds like the
// hand-lowered one — the mirror's offer becomes a
// `Pkg::from_bundle`, the wasm lane's url-dep mount. pouch and
// nmapset stay verbatim sources: they are GENERIC owners, and a
// compiled bundle serves only the instantiations its own pack closure
// spelled (consumer shapes like this page's `Vec<Node>` are the
// directory lane — see docs/src/reference/bundles.md).
const NMAP_HOST_BUNDLE: &[u8] = include_bytes!("../../../dist/std/nmap_host.rutbundle");
const NMAPSET_RUT: &str = include_str!("../../../rut/nmapset/mod.rut");

// the module trees, verbatim: root + every mounted child — the
// mirror's whole job is an independent embedder spelling what the
// manifest's `mod` declarations say (mount_lane pins the two together)
const UI_ROOT: &str = include_str!("../rut/ui/mod.rut");
const UI_STORE: &str = include_str!("../rut/ui/store/mod.rut");
const UI_WIDGET: &str = include_str!("../rut/ui/widget/mod.rut");
const UI_LOWERING: &str = include_str!("../rut/ui/lowering/mod.rut");
const UI_DIFF: &str = include_str!("../rut/ui/diff/mod.rut");
const UI_COMPONENTS: &str = include_str!("../rut/ui/components/mod.rut");
const BIZ_ROOT: &str = include_str!("../rut/biz/mod.rut");
const BIZ_DOMAIN: &str = include_str!("../rut/biz/domain/mod.rut");
const BIZ_WORLD: &str = include_str!("../rut/biz/world/mod.rut");
const BIZ_APP: &str = include_str!("../rut/biz/app/mod.rut");

/// One mounted child module — the mirror's `mod NAME;` edge, spelled
/// as the loader would mount it (the vis carried faithfully).
fn child_mod(path: &str, vis: rut_ast::ast::Vis, text: &str) -> (String, rut_driver::ModSource) {
    (
        path.to_string(),
        rut_driver::ModSource { path: path.to_string(), vis, text: text.to_string() },
    )
}

/// ui's module tree, offered whole: the root module plus its five
/// mounted children (all `pub mod` — biz crosses into each).
pub fn ui_pkg() -> Pkg {
    let mods: std::collections::BTreeMap<String, rut_driver::ModSource> = [
        child_mod("store", rut_ast::ast::Vis::Pub, UI_STORE),
        child_mod("widget", rut_ast::ast::Vis::Pub, UI_WIDGET),
        child_mod("lowering", rut_ast::ast::Vis::Pub, UI_LOWERING),
        child_mod("diff", rut_ast::ast::Vis::Pub, UI_DIFF),
        child_mod("components", rut_ast::ast::Vis::Pub, UI_COMPONENTS),
    ]
    .into();
    Pkg { spec: "ui".into(), mods, ..Pkg::source("ui", UI_ROOT) }
}

/// biz's module tree, offered whole: the root module (the law header +
/// the `mod` declarations) plus its three children (`domain`/`world`
/// pub — the probe's use paths cross — `app` private). This is the
/// handed-over ABI TREE both the native mirror (mount_lane) and the
/// wasm lane (loader.js's rows wire) compose.
pub fn biz_pkg() -> Pkg {
    let mods: std::collections::BTreeMap<String, rut_driver::ModSource> = [
        child_mod("domain", rut_ast::ast::Vis::Pub, BIZ_DOMAIN),
        child_mod("world", rut_ast::ast::Vis::Pub, BIZ_WORLD),
        child_mod("app", rut_ast::ast::Vis::Self_, BIZ_APP),
    ]
    .into();
    Pkg { spec: "app".into(), mods, ..Pkg::source("app", BIZ_ROOT) }
}

/// The rut/ project root — the manifest lane's walk point. Native
/// only: the wasm lane has no filesystem and uses the mirror.
#[cfg(not(target_arch = "wasm32"))]
pub fn project_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rut").join("biz")
}

/// A probe's module dir — `tests/<probe>/`. Each probe under tests/ is
/// its own rut program (its own `rut.jsonc`), the manifest lane's truth
/// for the test suites exactly like `rut/` is for the app.
#[cfg(not(target_arch = "wasm32"))]
pub fn probe_dir(probe: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(probe)
}

/// The `web` host surface as a pkg — the embedder half both lanes
/// offer (the registration scope IS the package name: `web::*`, the
/// prefix every crossing's host id carries).
pub fn web_pkg() -> Result<Pkg, String> {
    Ok(rut_driver::lower_decl_module(WEB_D_RUT, "web.d.rut")?.named("web"))
}

/// THE MIRROR — every package `rut/biz/rut.jsonc` names, offered by
/// hand (the wasm lane; also mount_lane's comparison world): `web`
/// (the crossing surface) + `pouch` + `nmap_host` (the committed CDN
// artifact — the url-dep mount's in-memory twin) + `nmapset` + `ui`.
/// The ROOT is deliberately absent: the app's SOURCE crosses the ABI
/// ([`compile_app`] offers and compiles it).
pub fn mirror_pkgs() -> Result<Vec<Pkg>, String> {
    let nmap_host = rut_driver::Pkg::from_bundle(NMAP_HOST_BUNDLE)
        .map_err(|e| e.to_string())?
        .pkgs;
    let mut pkgs = vec![web_pkg()?, Pkg::source("pouch", POUCH_RUT)];
    pkgs.extend(nmap_host);
    pkgs.push(Pkg::source("nmapset", NMAPSET_RUT));
    pkgs.push(ui_pkg());
    Ok(pkgs)
}

/// The mirror as a run: the mirror pkgs + nmap's bodies (the listener
/// table rides the val-column row `HashMap<str, i64>` — a mounted decl
/// pkg's rows demand bodies).
pub fn mirror_run() -> Result<RutRun, String> {
    let mut run = RutRun::new();
    for p in mirror_pkgs()? {
        run = run.pkg(p);
    }
    Ok(run.host_pkg(rut_std::nmap::pkg()))
}

/// dist/std-v8 plays the wire: the pinned artifacts prime ONE offline
/// cache per process (`DepRemote::write` is the stand-in for the GET —
/// the ONCE-gated priming is the 03-plugin precedent), and every
/// manifest-lane walk rides an `HttpRemote::offline` over it — a miss
/// NEVER touches the network, so the gate cannot network by
/// construction. The pin's bytes are the committed bytes
/// (`pack-std --check` green); the CDN is only the human lane. The
/// cache root lives in the tmp dir: a throwaway, not the project-local
/// `.rut/cache` a `rut run` would warm.
#[cfg(not(target_arch = "wasm32"))]
fn std_remote() -> rut_native::HttpRemote {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    rut_native::HttpRemote::offline(ROOT.get_or_init(|| {
        let root = std::env::temp_dir()
            .join(format!("rut-05-todolist-web-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // every manifest this example loads with deps — the app's two
        // packages and the two probes that name the std closure
        // (harness/softfail are dep-free) — walked for url rows exactly
        // like the loader's own prefetch walks
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let dist = base.join("../../dist/std-v8");
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
        let warmer = rut_native::HttpRemote::offline(&root);
        for url in &urls {
            let artifact = url.rsplit('/').next().unwrap_or_default();
            let bytes = std::fs::read(dist.join(artifact)).unwrap_or_else(|e| {
                panic!("the committed artifact is the cache — {artifact}: {e}")
            });
            rut_native::DepRemote::write(&warmer, url, &bytes).expect("prime the cache");
        }
        root
    }))
}

/// The manifest lane's directory walk: [`rut_native::load_dir_with`]
/// over [`std_remote`] — the same embedder shape the other examples
/// spell, with the offline flavor swapped in. One noop-waker poll
/// settles the READY futures: every url row is a primed cache hit, so
/// nothing ever awaits the wire.
#[cfg(not(target_arch = "wasm32"))]
pub fn walk_dir(dir: &std::path::Path) -> Result<Loaded, String> {
    use std::future::Future as _;
    let remote = std_remote();
    let fut = rut_native::load_dir_with(dir, &remote);
    let mut fut = std::pin::pin!(fut);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match fut.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => return v.map_err(|e| e.to_string()),
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// THE MANIFEST LANE — the rut/biz project root walked for real (the
/// four passes), answered as the yield plus its root name. Offer with
/// [`offer`], compile with [`compile_walk`].
#[cfg(not(target_arch = "wasm32"))]
pub fn project() -> Result<Loaded, String> {
    walk_dir(&project_dir())
}

/// A probe's manifest lane — `tests/<probe>/` walked for real, the
/// probe's own manifest naming its deps (the sibling ui row stays a
/// local `path`; the std rows are pinned jsDelivr urls; `web` rides no
/// row). The softfail fixture is the deliberate exception: no crossings
/// is its point, so its test loads bare.
#[cfg(not(target_arch = "wasm32"))]
pub fn probe(probe: &str) -> Result<Loaded, String> {
    walk_dir(&probe_dir(probe))
}

/// The embedder half every lane owns, as chain steps: the walk's pkgs
/// + the `web` surface + nmap's bodies WHEN the closure declares the
/// `nmap_host` scope (the listener table rides the val-column row
/// `HashMap<str, i64>` — a mounted decl pkg's rows demand bodies;
/// probes without the scope skip the install, the blanket law's gate).
/// The core prelude auto-offers at `.compile()`.
pub fn offer(mut run: RutRun, loaded: &Loaded) -> RutRun {
    run = run.pkgs(loaded);
    run = match web_pkg() {
        Ok(w) => run.pkg(w),
        Err(_) => run, // unreachable: the surface is include_str!'d
    };
    if loaded.pkgs.iter().any(|p| p.spec == "nmap_host") {
        run = run.host_pkg(rut_std::nmap::pkg());
    }
    run
}

/// The declared host rows of a walked world + the `web` surface — the
/// `.d.rut` ↔ binding contract's expected table (`verify_against`).
pub fn expected_host_fns(loaded: &Loaded) -> rut_vm::interp::ExpectedHostFns {
    let mut pkgs: Vec<Pkg> = loaded.pkgs.clone();
    if let Ok(w) = web_pkg() {
        pkgs.push(w);
    }
    rut_driver::declared_host_fns(&pkgs)
}

/// Compile a run with a handed-in root pkg — the mirror lane's compile
/// half for the ABI tree (the pkg's root module is its body, its
/// `mod` children its tree). Diags are the error text; the compiled
/// run comes back (its registry carries every `.host_pkg` install).
pub fn compile_root_pkg(run: RutRun, pkg: Pkg) -> Result<Compiled, String> {
    let spec = pkg.spec.clone();
    let compiled = run
        .pkg(pkg)
        .entrypoint(&spec)
        .compile()
        .map_err(|e| format!("the app does not compile: {e}"))?;
    checked(compiled)
}

/// [`compile_root_pkg`] over the biz tree.
pub fn compile_app(run: RutRun) -> Result<Compiled, String> {
    compile_root_pkg(run, biz_pkg())
}

/// Compile a walked world from its root pkg — THE MANIFEST LANE's
/// compile half ([`offer`] runs inside).
pub fn compile_walk(loaded: &Loaded) -> Result<Compiled, String> {
    let compiled = offer(RutRun::new(), loaded)
        .entrypoint(&loaded.root)
        .compile()
        .map_err(|e| format!("the app does not compile: {e}"))?;
    checked(compiled)
}

/// The shared tail of both compile halves: diags are the error text,
/// and the binary must decode and verify before it is a program.
fn checked(compiled: Compiled) -> Result<Compiled, String> {
    if !compiled.graph.diags.is_empty() {
        return Err(format!(
            "the app does not compile: {}",
            compiled.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; ")
        ));
    }
    if compiled.graph.program.is_none() {
        return Err("the app compiled to no binary".to_string());
    }
    Ok(compiled)
}

/// The binary half of a compiled run: encode → decode → verify — the
/// same bytes a run lane would cross.
pub fn verified(compiled: &Compiled) -> Result<Program, String> {
    let prog = compiled.graph.program.as_ref().expect("checked() ran");
    let bin = rut_core::binary::encode(prog);
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
