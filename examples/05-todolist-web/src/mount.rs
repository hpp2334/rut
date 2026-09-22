//! The session mount, shared by both lanes (survey §5.3's boot shape):
//! std `core` (ambient, no host fns), `pouch` embedded as inline source
//! (the wasm-embeds-pouch precedent — wasm has no filesystem, and one
//! code path keeps the twin honest), and the `web` decl surface lowered
//! from `web.d.rut` with `host_scope = "web"` (the `rt` mounting
//! precedent). Nothing here knows the app.

use rut_driver::Module;

/// The `web` pkg's surface, verbatim — the contract the bodies bind.
pub const WEB_D_RUT: &str = include_str!("../web.d.rut");

const POUCH_RUT: &str = include_str!("../../../rut/pouch/pouch.rut");

/// Mount `core` + `pouch` + the `web` host surface. `core` is mounted
/// unconditionally by the engine anyway; `calc` is NOT mounted — the
/// crossings need no float surface (add `mount_calc` +
/// `rut_std::math::install_std_math` with the app that wants it).
pub fn mount_host_session(session: &mut rut_driver::Session) -> Result<(), String> {
    rut_driver::mount_std_core(session);
    session
        .register_module(
            "pouch",
            Module { source: Some(POUCH_RUT.to_string()), ..Default::default() },
        )
        .map_err(|e| e.to_string())?;
    let mut web = rut_driver::lower_decl_module(WEB_D_RUT, "web.d.rut")?;
    web.host_scope = Some("web".to_string()); // the registration prefix: `web::<name>`
    session.register_module("web", web).map_err(|e| e.to_string())?;
    Ok(())
}

/// Compile the app source against a mounted session and hand back the
/// verified binary program. Compile diags are a boot error, never a
/// partial run — the page fails loud before any DOM exists.
pub fn compile_app(
    session: &mut rut_driver::Session,
    src: &str,
) -> Result<rut_core::binary::Program, String> {
    let out = rut_driver::compile_module_in(session, src, rut_parser::Mode::Impl, "app");
    if !out.diags.is_empty() {
        let msgs: Vec<String> = out.diags.iter().map(|d| d.msg.clone()).collect();
        return Err(format!("the app does not compile: {}", msgs.join("; ")));
    }
    let bin = out.binary.ok_or("the app compiled to no binary")?;
    let prog = rut_core::binary::decode(&bin).map_err(|e| format!("binary decode: {e}"))?;
    rut_vm::verify::verify(&prog).map_err(|e| format!("verify: {e}"))?;
    Ok(prog)
}

/// The twin's budgets (survey §5.5 — the 00-todolist numbers; each turn
/// is bounded, so latency never feeds fuel).
pub fn limits() -> rut_vm::interp::Limits {
    rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    }
}

// ---- the phase-2 app mounts -------------------------------------------------
// `mount_host_session` above stays app-blind (the phase-1 contract). The
// app's session adds three packages, all embedded — wasm32 has no
// filesystem, so everything rides `include_str!` (the rut-wasm +
// ink/pouch precedent, one code path for both lanes):
//
//   * `nmap_host` — the native key-table surface (RFC 0025 decl), its
//     bodies bound per lane by `rut_std::nmap::install_std_nmap`;
//   * `nmapset` — the keyed-collection pkg, source-inlined (`inline`,
//     the ink treatment: a generic-class module splices into its
//     consumer);
//   * `appkit` — the app unit's ONE splice: the "server" half
//     (`store.rut`) and the widget framework (`t1.rut`) offered as a
//     single inline module. WHY ONE: the graph splices each use's
//     transitive sources per use, with no cross-use dedup — and store
//     and t1 both ride pouch (t1 also rides nmapset), so a consumer
//     importing both would define Vec twice. Spliced together they
//     define everything once, and the unit's bound surface carries the
//     host crossings the lowering needs. Both files stay verbatim and
//     byte-untouched; the app imports `appkit::{ ... }` — the biz law
//     (no crossing import), gated in `tests/app_law.rs`.

pub const NMAP_HOST_D_RUT: &str = include_str!("../../../rut/nmap_host/nmap.d.rut");
pub const NMAPSET_RUT: &str = include_str!("../../../rut/nmapset/nmapset.rut");
pub const STORE_RUT: &str = include_str!("../store.rut");
pub const T1_RUT: &str = include_str!("../t1.rut");

/// core + pouch ONLY — the store's own session: `tests/store.rs` drives
/// the "server" half with no web surface and no clock mounted at all.
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

/// The app session: everything [`mount_host_session`] mounts, plus the
/// keyed-collection pkg and the store module. Bind `nmap_host`'s bodies
/// with `rut_std::nmap::install_std_nmap` next to [`crate::hosts::install_web_hosts`].
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
    session
        .register_module(
            "appkit",
            // ONE inline module = the app unit's one splice: the store
            // (the "server") + the widget framework, verbatim, in that
            // order. The app imports this and nothing else — see the
            // mount note above for why they ride together.
            Module {
                source: Some(format!("{STORE_RUT}\n{T1_RUT}")),
                inline: true,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
