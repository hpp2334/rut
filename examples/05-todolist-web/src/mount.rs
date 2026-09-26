//! The session mount — TWO LANES over ONE closure (survey §2.4).
//!
//! THE MANIFEST (the native lane's truth): `rut/biz/` is the project
//! root — the TWO-PACKAGE law's biz side — and every package it names
//! (ui, and through ui pouch/nmapset/nmap_host) is mounted by
//! [`crate::mount::project_dir`] + `rut_driver::load_dir_session`,
//! RFC 0045's four passes run FOR REAL. The manifest, not a Rust fn,
//! is the module list.
//!
//! THE MIRROR (the wasm lane): the Session is I/O-free by law (wasm
//! hosts mount in memory), so `mount_app_session` registers the same
//! closure by hand — one `register_module` per package, the same
//! `inline`/`host_scope` values the manifests state, everything
//! `include_str!` (the rut-wasm ink/rt precedent). Nothing here knows
//! the app's SOURCE: the root's body crosses the ABI (loader.js hands
//! over `rut/biz/biz.rut`) and [`compile_app`] compiles it as the
//! root.
//!
//! [`crate::mount::mount_app_session`] is the mirror of
//! `rut/biz/rut.toml` and nothing more — `tests/mount_lane.rs` (the P3
//! proof) pins the two lanes to the same mounted-name set, the same
//! host surface, and the same compiled binary. The `limits()`/
//! `compile_app` halves below are shared by both lanes.
//!
//! THE HOST DECL SURFACE: `web.d.rut` sits FLAT at the example root,
//! outside any package directory (its pre-restructure placement, kept
//! — the two-package law counts manifests, and the host crossing is
//! not a rut package). BOTH lanes register it by hand: the embedder
//! mounts what the closure uses (the `mount_std_core` precedent).
//!
//! What died here before this file's current shape: the old
//! concatenation module (ONE inline module fabricated from the store
//! and t1 sources) — dep-kinds landed splice-dedup-by-origin and the
//! workaround's reason was gone (survey §2.5, P1/P2/P3). What the
//! TWO-PACKAGE law retired on top: the sixteen per-concept manifests —
//! t1's three, the components' eight, the store's three, the app's
//! three — merged into ui (framework) and biz (domain + app), one
//! module each, because rut's landed visibility is `pub` or
//! module-private and each layer keeps its privates private for real
//! in one file.

use rut_core::binary::Program;
use rut_driver::Module;

// ---- the packages, embedded verbatim --------------------------------
// The mirror reads the SAME files the manifest names. wasm32 has no
// filesystem, so everything rides `include_str!` — one code path for
// both lanes, the ink/pouch precedent.

/// The `web` pkg's surface, verbatim — the contract the bodies bind.
pub const WEB_D_RUT: &str = include_str!("../web.d.rut");

const POUCH_RUT: &str = include_str!("../../../rut/pouch/pouch.rut");
const NMAP_HOST_D_RUT: &str = include_str!("../../../rut/nmap_host/nmap.d.rut");
const NMAPSET_RUT: &str = include_str!("../../../rut/nmapset/nmapset.rut");

const UI_RUT: &str = include_str!("../rut/ui/ui.rut");
// ui's `entry.libs` tail (rut.toml) — the mirror spells the manifest's
// file list BY HAND (the mirror's whole job: an independent embedder
// spelling what the manifest says; mount_lane pins the two together)
const UI_STORE_RUT: &str = include_str!("../rut/ui/store.rut");
const UI_WIDGET_RUT: &str = include_str!("../rut/ui/widget.rut");
const UI_LOWERING_RUT: &str = include_str!("../rut/ui/lowering.rut");
const UI_DIFF_RUT: &str = include_str!("../rut/ui/diff.rut");
const UI_COMPONENTS_RUT: &str = include_str!("../rut/ui/components.rut");

/// ui's source, spliced the manifest's way (RFC 0041 §5): base first,
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

/// The MANIFEST LANE's mount (native only): `load_dir_session` over
/// the rut/biz project root — RFC 0045's four passes for real — plus
/// the embedder half every lane owns: the `core` prelude AND the `web`
/// host surface (the crossing is no package's dep; both lanes register
/// it by hand, the same `host_scope` the crossing's ids carry).
/// `tests/mount_lane.rs` pins the two lanes together.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_project_session() -> Result<(rut_driver::Session, String), String> {
    let (mut session, root) = rut_driver::load_dir_session(&project_dir())?;
    rut_driver::mount_std_core(&mut session);
    register_web_surface(&mut session)?;
    Ok((session, root))
}

/// Register the `web` DECL surface — the embedder half both lanes run.
/// `host_scope = "web"`: the registration prefix every crossing's host
/// id carries (RFC 0025).
pub fn register_web_surface(session: &mut rut_driver::Session) -> Result<(), String> {
    let mut web = rut_driver::lower_decl_module(WEB_D_RUT, "web.d.rut")?;
    web.host_scope = Some("web".to_string());
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
            Module { source: Some(POUCH_RUT.to_string()), ..Default::default() },
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

/// core + pouch ONLY — the fixture session: `tests/todolist_app.rs`'s
/// softfail fixture compiles with no web surface, no clock, no widgets.
pub fn mount_store_session(session: &mut rut_driver::Session) -> Result<(), String> {
    rut_driver::mount_std_core(session);
    session
        .register_module(
            "pouch",
            Module { source: Some(POUCH_RUT.to_string()), ..Default::default() },
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The app session — THE MIRROR of `rut/biz/rut.toml` (survey §2.4):
/// every package the manifest names, registered with the manifest's
/// own mount properties. The ROOT is deliberately absent: the app's
/// SOURCE crosses the ABI ([`compile_app`] registers and compiles it),
/// which is the one thing the I/O-free Session cannot fetch.
///
/// Bind `nmap_host`'s bodies with `rut_std::nmap::install_std_nmap`
/// next to [`crate::hosts::install_web_hosts`].
pub fn mount_app_session(session: &mut rut_driver::Session) -> Result<(), String> {
    mount_host_session(session)?;

    let nmap = rut_driver::lower_decl_module(NMAP_HOST_D_RUT, "nmap.d.rut")?;
    session.register_module("nmap_host", nmap).map_err(|e| e.to_string())?;
    session
        .register_module(
            "nmapset",
            Module { source: Some(NMAPSET_RUT.to_string()), inline: true, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;

    // THE UI PACKAGE — the framework: the atom store machinery, the
    // widget type, the lowering table, the keyed diff, the component
    // vocabulary — ONE module (the two-package law; inline, its
    // manifest's stated shape: generic exports splice by law). The
    // multi-lib files ride through ui_source() — the manifest's
    // `entry.libs`, spliced base-first in array order (RFC 0041 §5).
    session
        .register_module(
            "ui",
            Module { source: Some(ui_source()), inline: true, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}
