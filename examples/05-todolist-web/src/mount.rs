//! The session mount — TWO LANES over ONE closure (survey §2.4).
//!
//! THE MANIFEST (the native lane's truth): `rut/` is a project —
//! `rut/rut.toml` names the root (`app`, `./app/app.rut`) and every
//! sub-package, and [`crate::mount::project_dir`] +
//! `rut_driver::load_dir_session` mounts the whole closure with RFC
//! 0045's four passes (the deps walk, the root's dev-deps, the peer
//! gate, compile) run FOR REAL. The manifest, not a Rust fn, is the
//! module list.
//!
//! THE MIRROR (the wasm lane): the Session is I/O-free by law (wasm
//! hosts mount in memory), so `mount_app_session` registers the same
//! closure by hand — one `register_module` per package, the same
//! `inline`/`host_scope` values the manifests state, everything
//! `include_str!` (the rut-wasm ink/rt precedent). Nothing here knows
//! the app's SOURCE: the root's body crosses the ABI (loader.js hands
//! over `rut/app/app.rut`) and [`compile_app`] compiles it as the root.
//!
//! [`crate::mount::mount_app_session`] is the mirror of `rut/rut.toml`
//! and nothing more — `tests/mount_lane.rs` (the P3 proof) pins the two
//! lanes to the same mounted-name set, the same host surface, and the
//! same compiled binary. The `limits()`/`compile_app` halves below are
//! shared by both lanes.
//!
//! What died here: the old concatenation module — ONE inline module
//! fabricated from the store and t1 sources because the pre-dedup
//! graph spliced each use's transitive sources per use (both rode
//! pouch; a consumer importing both would define Vec twice).
//! dep-kinds landed splice-dedup-by-origin, the restructure mounts the
//! packages SEPARATELY (each with its own manifest-stated
//! composition), and the workaround's reason is gone with it. The
//! retirement proof: P1 (the separate mounts compile — every green
//! suite here), P2 (the law gate asserts the retired name survives
//! nowhere in `src/` — this file included), P3 (the lane equality
//! above) — `tests/app_law.rs` and `tests/mount_lane.rs`.

use rut_core::binary::Program;
use rut_driver::Module;

// ---- the packages, embedded verbatim --------------------------------
// The mirror reads the SAME files the manifest names. wasm32 has no
// filesystem, so everything rides `include_str!` — one code path for
// both lanes, the ink/pouch precedent.

/// The `web` pkg's surface, verbatim — the contract the bodies bind.
pub const WEB_D_RUT: &str = include_str!("../rut/web/web.d.rut");

const POUCH_RUT: &str = include_str!("../../../rut/pouch/pouch.rut");
const NMAP_HOST_D_RUT: &str = include_str!("../../../rut/nmap_host/nmap.d.rut");
const NMAPSET_RUT: &str = include_str!("../../../rut/nmapset/nmapset.rut");

const WIDGET_RUT: &str = include_str!("../rut/t1/widget/widget.rut");
const LOWERING_RUT: &str = include_str!("../rut/t1/lowering/lowering.rut");
const T1_RUT: &str = include_str!("../rut/t1/core/t1.rut");
const ATOM_RUT: &str = include_str!("../rut/store/atom/atom.rut");
const DERIVED_RUT: &str = include_str!("../rut/store/derived/derived.rut");
const TODOS_RUT: &str = include_str!("../rut/store/todos/todos.rut");

const COL_RUT: &str = include_str!("../rut/components/col/col.rut");
const ROW_RUT: &str = include_str!("../rut/components/row/row.rut");
const TEXT_RUT: &str = include_str!("../rut/components/text/text.rut");
const BUTTON_RUT: &str = include_str!("../rut/components/button/button.rut");
const CHECKBOX_RUT: &str = include_str!("../rut/components/checkbox/checkbox.rut");
const FIELD_RUT: &str = include_str!("../rut/components/field/field.rut");
const SPACER_RUT: &str = include_str!("../rut/components/spacer/spacer.rut");
const CARD_RUT: &str = include_str!("../rut/components/card/card.rut");

const TODO_LIST_RUT: &str = include_str!("../rut/app/todo_list/todo_list.rut");
const TODO_ROW_RUT: &str = include_str!("../rut/app/todo_row/todo_row.rut");

/// The rut/ project root — the manifest lane's mount point. Native
/// only: the wasm lane has no filesystem and uses the mirror.
#[cfg(not(target_arch = "wasm32"))]
pub fn project_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rut")
}

/// The store pkg's own directory — `tests/store.rs` mounts it as the
/// ROOT (a manifest session of one package + pouch), DOM-free: no web
/// surface, no clock, no nmap in the session at all.
#[cfg(not(target_arch = "wasm32"))]
pub fn todos_pkg_dir() -> std::path::PathBuf {
    project_dir().join("store").join("todos")
}

/// The MANIFEST LANE's mount (native only): `load_dir_session` over
/// the rut/ project root — RFC 0045's four passes for real — plus the
/// embedder half every lane owns: the `core` prelude (the 03-plugin
/// host precedent: the manifest resolves the packages; the embedder
/// mounts what the closure uses). The wasm counterpart is
/// [`mount_app_session`]; `tests/mount_lane.rs` pins the two together.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_project_session() -> Result<(rut_driver::Session, String), String> {
    let (mut session, root) = rut_driver::load_dir_session(&project_dir())?;
    rut_driver::mount_std_core(&mut session);
    Ok((session, root))
}

/// [`load_project_session`] at package scale: the todos pkg alone (its
/// manifest closure is pouch) — the store suite's DOM-free world.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_todos_session() -> Result<(rut_driver::Session, String), String> {
    let (mut session, root) = rut_driver::load_dir_session(&todos_pkg_dir())?;
    rut_driver::mount_std_core(&mut session);
    Ok((session, root))
}

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

/// Compile a handed-in root source against a mounted session under an
/// explicit spec, and hand back the verified binary program — the
/// mirror lane's compile half for hand-over sources: `compile_app` is
/// the ABI shape of this (the page's root is always spec `app`); the
/// twin harness compiles under its own spec over the same closure.
/// Compile diags are a boot error, never a partial run — the page
/// fails loud before any DOM exists.
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
/// MANIFEST LANE's compile half: `load_dir_session` registered the
/// root's source from disk, this compiles the closure and hands back
/// the verified binary program.
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
        fuel: Some(1_000_000),
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

/// The app session — THE MIRROR of `rut/rut.toml` (survey §2.4): every
/// package the manifest names, registered with the manifest's own
/// mount properties. The ROOT is deliberately absent: the app's SOURCE
/// crosses the ABI ([`compile_app`] registers and compiles it), which
/// is the one thing the I/O-free Session cannot fetch. `tests/mount_lane.rs`
/// proves this closure and the manifest's agree — names, host surface,
/// binary.
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

    // t1, three packages: the type (linked), the lowering (linked),
    // the core (inline — its manifest's stated shape, see rut.toml)
    session
        .register_module("widget", Module { source: Some(WIDGET_RUT.to_string()), ..Default::default() })
        .map_err(|e| e.to_string())?;
    session
        .register_module(
            "lowering",
            Module { source: Some(LOWERING_RUT.to_string()), ..Default::default() },
        )
        .map_err(|e| e.to_string())?;
    session
        .register_module(
            "t1",
            Module { source: Some(T1_RUT.to_string()), inline: true, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;

    // the store packages (survey §4 — the atom store, phase 2): the
    // cell layer (atom: Rail + Atom<T> + StrAtom), the derived layer
    // (derived: the declared DAG + Seen), the domain (todos: the
    // atoms as fields, counts$ the derived, refresh() the flush) —
    // all inline, exactly what their manifests state
    session
        .register_module(
            "atom",
            Module { source: Some(ATOM_RUT.to_string()), inline: true, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;
    session
        .register_module(
            "derived",
            Module { source: Some(DERIVED_RUT.to_string()), inline: true, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;
    session
        .register_module(
            "todos",
            Module { source: Some(TODOS_RUT.to_string()), inline: true, ..Default::default() },
        )
        .map_err(|e| e.to_string())?;

    // the component vocabulary — one package per kind
    let components: &[(&str, &str)] = &[
        ("col", COL_RUT),
        ("row", ROW_RUT),
        ("text", TEXT_RUT),
        ("button", BUTTON_RUT),
        ("checkbox", CHECKBOX_RUT),
        ("field", FIELD_RUT),
        ("spacer", SPACER_RUT),
        ("card", CARD_RUT),
    ];
    for (spec, src) in components {
        session
            .register_module(spec, Module { source: Some(src.to_string()), ..Default::default() })
            .map_err(|e| e.to_string())?;
    }

    // the two view builders
    session
        .register_module(
            "todo_list",
            Module { source: Some(TODO_LIST_RUT.to_string()), ..Default::default() },
        )
        .map_err(|e| e.to_string())?;
    session
        .register_module(
            "todo_row",
            Module { source: Some(TODO_ROW_RUT.to_string()), ..Default::default() },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}
