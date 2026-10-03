//! The engine's own packages as temporary Pkg constructors — `core`
//! (the prelude, auto-offered by `.compile()`), `calc` (the float
//! `Math` surface), and the standard async set.
//!
//! TEMPORARY (Phase A stopgap): these constructors exist so every
//! consumer spells the chain idiom today; the walk that feeds them
//! dies into `rut-native` next phase — `core` rides auto in
//! `.compile()`, `calc` becomes the tree pkg `rut/calc/`, and the
//! async pair arrives through the ordinary walk.

use rut_core::types::{TY_F32, TY_F64};

use crate::loader::dir_pkgs;
use crate::run::RunError;
use crate::session::{Pkg, PkgBody, Session};

/// The `calc` body mount note: `calc` is engine-mounted for now (the
/// float `Math` surface — host fns in `rut-std`, consts); the
/// host-pkgs plan moves it to a declared package and its rut-able
/// helpers to source (deferred with the numeric-methods phase).

/// The `core` prelude pkg — the erasure
/// primitive (`opaque`), the builtin trait (`Iterable`), and
/// the compiler-lowered functions (`panic`, the
/// `str`/`bytes` natives). v1.1 removed `Option`/`Result`/`own` — use
/// sites diagnose with the removal. A
/// native pkg with no body: its surface is
/// [`rut_core::binary::Surface::core`], the single source of truth
/// (`rut/core/core.d.rut` mirrors it for the LSP). Builtin names are
/// ambient except the `pub builtin` spellings (the disposal pair) —
/// those resolve only through `use core::{ .. }`; each row
/// keeps its ambient bit so the binding loops see the split.
/// `core` needs no `[deps]`
/// declaration: `.compile()` auto-offers it unconditionally (§0.14),
/// while every other package resolves through `[deps]` or host
/// registration.
pub fn core_pkg() -> Pkg {
    // the surface is symbol-id based; the host-facing mount table is
    // string-based — `sym::text` bridges at this boundary only
    let core = rut_core::binary::Surface::core();
    let txt = |id: rut_core::IdentId| -> String {
        rut_core::sym::text(id).unwrap_or_default().to_string()
    };
    Pkg {
        spec: "core".to_string(),
        // each row keeps its ambient bit — the binding loops read
        // it off the synthesized surface, so the `pub builtin`
        // spellings stay import-gated end to end
        body: PkgBody::Host {
            native_types: core.native_types.iter().map(|(n, k, a)| (txt(*n), *k, *a)).collect(),
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
    }
}

/// The engine's own packages as one mount — the packer's lane (it
/// compiles over the internal table directly). `core` (the prelude —
/// always; nothing else is ambient) and `calc` (the `Math` float
/// surface). `pouch` and `ink` are third-party libraries in the
/// toolchain tree (`rut/pouch/`, `rut/ink/`) — a consumer declares
/// them in its manifest `[deps]`, or a native host offers them with
/// [`crate::loader::dir_pkgs`]; nothing in the engine knows their
/// names (the host-pkgs plan §2).
pub(crate) fn mount_std_pkgs(session: &mut Session) {
    let core = core_pkg();
    let calc = calc_pkg();
    let _ = session.mount(core);
    let _ = session.mount(calc);
}

/// The standard async set — the engine rows (`async_host` — a decl
/// pkg) and the typed launcher surface (`futures` — an inline rut
/// package), walked from the toolchain tree. Pair with
/// `rut_std::async_host::pkg()` installed through `.host_pkg(..)`;
/// a run that offers neither simply has no launcher, and `await`
/// stays cold-poll inline.
pub fn std_async_pkgs() -> Result<Vec<Pkg>, RunError> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rut")
        .canonicalize()
        .expect("the toolchain tree's rut/ dir");
    let async_host = dir_pkgs(&root.join("async_host"))?;
    let futures = dir_pkgs(&root.join("futures"))?;
    Ok(async_host.pkgs.into_iter().chain(futures.pkgs).collect())
}

/// The `calc` pkg — a native pkg: `f64` host functions
/// (bodies in `rut-std`, including the float `abs`/`min`/`max`/`signum`)
/// plus their `f32` twins (`Math.sqrt_f(x: f32) -> f32` — the `_f`
/// suffix carries the width; rut has no overloading) and the `f64`
/// constants. The integer intrinsics live in `core` now — `builtin
/// impl` methods on the primitives.
pub fn calc_pkg() -> Pkg {
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

    Pkg {
        spec: "calc".to_string(),
        namespace: Some("Math".to_string()),
        body: PkgBody::Host {
            host_funcs,
            consts,
            native_types: vec![],
            native_fns: vec![],
            native_impls: vec![],
        },
        ..Default::default()
    }
}
