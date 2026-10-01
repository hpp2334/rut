//! The engine's own package mounts — `core` (the prelude), `calc`
//! (the float `Math` surface), and the standard async set.

use rut_core::types::{TY_F32, TY_F64};

use crate::loader::mount_dir;
use crate::session::{Module, ModuleBody, Session};

/// The `calc` body mount note: `calc` is engine-mounted for now (the
/// float `Math` surface — host fns in `rut-std`, consts); the
/// host-pkgs plan moves it to a declared package and its rut-able
/// helpers to source (deferred with the numeric-methods phase).

/// Mount `core` — the prelude surface: the erasure
/// primitive (`opaque`), the builtin trait (`Iterator`), and
/// the compiler-lowered functions (`panic`, the
/// `str`/`bytes` natives). v1.1 removed `Option`/`Result`/`own` — use
/// sites diagnose with the removal. A
/// native module with no body: its surface is
/// [`rut_core::binary::Surface::core`], the single source of truth
/// (`rut/core/core.d.rut` mirrors it for the LSP). Builtin names are
/// ambient except the `pub builtin` spellings (the disposal pair) —
/// those resolve only through `use core::{ .. }`; each mounted row
/// keeps its ambient bit so the binding loops see the split.
/// `core` needs no `[deps]`
/// declaration: the driver mounts it unconditionally (§0.14), while
/// every other package resolves through `[deps]` or host registration.
pub fn mount_std_core(session: &mut Session) {
    // the surface is symbol-id based; the host-facing mount table is
    // string-based — `sym::text` bridges at this boundary only
    let core = rut_core::binary::Surface::core();
    let txt = |id: rut_core::IdentId| -> String {
        rut_core::sym::text(id).unwrap_or_default().to_string()
    };
    let _ = session.register_module(
        "core",
        Module {
            // each row keeps its ambient bit — the binding loops read
            // it off the synthesized surface, so the `pub builtin`
            // spellings stay import-gated end to end
            body: ModuleBody::Host {
                native_types: core.native_types.iter().map(|(n, k, a)| (txt(*n), *k, *a)).collect(),
                native_traits: core.native_traits.iter().map(|(n, k, a)| (txt(*n), *k, *a)).collect(),
                native_fns: core.native_fns.iter().map(|(n, a)| (txt(*n), *a)).collect(),
                consts: core.consts.iter().map(|c| (txt(c.name), c.ty, c.bits)).collect(),
                native_impls: core
                    .native_impls
                    .iter()
                    .map(|(t, n, i)| (*t, txt(*n), *i))
                    .collect(),
                host_funcs: vec![],
            },
            ..Default::default()
        },
    );
}

/// Mount the engine's own packages: `core` (the prelude — always;
/// nothing else is ambient) and `calc` (the `Math` float surface).
/// `pouch` and `ink` are third-party libraries in the toolchain tree
/// (`rut/pouch/`, `rut/ink/`) — a consumer declares them in its
/// manifest `[deps]`, or a native host mounts them with
/// [`mount_dir`]; nothing in the engine knows their names (the
/// host-pkgs plan §2).
pub fn mount_std(session: &mut Session) {
    mount_std_core(session);
    mount_calc(session);
}

/// Mount the standard async set: the engine rows
/// (`async_engine` — a decl module) and the typed launcher surface
/// (`async_host` — an inline rut package). Pair with
/// `rut_std::async_host::pkg()` installed through
/// `HostRegistry::install_host_pkg` before `Vm::new`; a session
/// that mounts neither simply has no launcher, and `await` stays
/// cold-poll inline.
pub fn mount_std_async(session: &mut Session) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rut")
        .canonicalize()
        .expect("the toolchain tree's rut/ dir");
    mount_dir(session, &root.join("async_engine")).expect("mount async_engine");
    mount_dir(session, &root.join("async_host")).expect("mount async_host");
}

/// Mount `calc` — a native module: `f64` host functions
/// (bodies in `rut-std`, including the float `abs`/`min`/`max`/`signum`)
/// plus their `f32` twins (`Math.sqrt_f(x: f32) -> f32` — the `_f`
/// suffix carries the width; rut has no overloading) and the `f64`
/// constants. The integer intrinsics live in `core` now — `builtin
/// impl` methods on the primitives.
pub fn mount_calc(session: &mut Session) {
    let u = |name: &str| (name.to_string(), vec![TY_F64], TY_F64, false);
    let b = |name: &str| (name.to_string(), vec![TY_F64, TY_F64], TY_F64, false);
    let uf = |name: &str| (name.to_string(), vec![TY_F32], TY_F32, false);
    let bf = |name: &str| (name.to_string(), vec![TY_F32, TY_F32], TY_F32, false);
    let host_funcs = vec![
        u("sqrt"), u("floor"), u("ceil"), u("round"), u("trunc"),
        u("exp"), u("ln"), u("log2"), u("log10"),
        u("sin"), u("cos"), u("tan"), u("asin"), u("acos"), u("atan"),
        u("sinh"), u("cosh"), u("tanh"),
        b("pow"), b("atan2"), b("hypot"), b("copysign"),
        ("fma".to_string(), vec![TY_F64, TY_F64, TY_F64], TY_F64, false),
        // the former float intrinsics — ordinary host fns now (the
        // integer intrinsics moved to core's `builtin impl` methods)
        u("abs"), b("min"), b("max"), u("signum"),
        // the f32 twins — same libm bodies in rut-std, `_f` names
        uf("sqrt_f"), uf("floor_f"), uf("ceil_f"), uf("round_f"), uf("trunc_f"),
        uf("exp_f"), uf("ln_f"), uf("log2_f"), uf("log10_f"),
        uf("sin_f"), uf("cos_f"), uf("tan_f"), uf("asin_f"), uf("acos_f"), uf("atan_f"),
        uf("sinh_f"), uf("cosh_f"), uf("tanh_f"),
        bf("pow_f"), bf("atan2_f"), bf("hypot_f"), bf("copysign_f"),
        ("fma_f".to_string(), vec![TY_F32, TY_F32, TY_F32], TY_F32, false),
        uf("abs_f"), bf("min_f"), bf("max_f"), uf("signum_f"),
    ];

    let c = |name: &str, v: f64| (name.to_string(), TY_F64, v.to_bits());
    let consts = vec![
        c("PI", std::f64::consts::PI),
        c("TAU", std::f64::consts::TAU),
        c("E", std::f64::consts::E),
        c("SQRT_2", std::f64::consts::SQRT_2),
        c("LN_2", std::f64::consts::LN_2),
        c("LN_10", std::f64::consts::LN_10),
        c("LOG2_E", std::f64::consts::LOG2_E),
        c("LOG10_E", std::f64::consts::LOG10_E),
        c("INFINITY", f64::INFINITY),
        c("NEG_INFINITY", f64::NEG_INFINITY),
        // `NAN` moved to core (`use core::{NAN}`)
        c("EPSILON", f64::EPSILON),
        c("MAX", f64::MAX),
        c("MIN", f64::MIN),
        c("MIN_POSITIVE", f64::MIN_POSITIVE),
    ];

    let _ = session.register_module(
        "calc",
        Module {
            namespace: Some("Math".to_string()),
            body: ModuleBody::Host {
                host_funcs,
                consts,
                native_types: vec![],
                native_traits: vec![],
                native_fns: vec![],
                native_impls: vec![],
            },
            ..Default::default()
        },
    );
}

