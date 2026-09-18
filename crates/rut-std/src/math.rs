//! `calc`'s host half (RFC 0028): the native-module functions behind
//! the float math surface — `f64` members and their `f32` twins
//! (`sqrt_f` …; the `_f` suffix carries the width, rut has no
//! overloading) — the float primitives and the float
//! `abs`/`min`/`max`/`signum` helpers (the integer numeric methods are
//! core's `builtin impl` methods, compiler-lowered — no host body).
//!
//! The `calc` module is mounted by the driver (`rut-driver`); a host
//! installs the bodies. Bindings are the MAGIC shape (RFC 0023/0025):
//! the closure's Rust parameter types ARE the `.d.rut` row — the
//! signature is derived and checked against `calc.d.rut` at load time.
//! These are the crossing-hot bindings: the adapter inlines to one
//! indirect call, the body reads its floats straight out of the slots.

use rut_vm::interp::{HostRegistry, Vm};

macro_rules! unary {
    ($hosts:expr, $name:literal, $suffix:literal, $f:ident, $t:ty) => {
        $hosts.register::<_, ($t,), $t, _>(concat!("calc::", $name, $suffix), |_vm: &mut Vm, x: $t| x.$f());
    };
}

macro_rules! binary {
    ($hosts:expr, $name:literal, $suffix:literal, $f:ident, $t:ty) => {
        $hosts.register::<_, ($t, $t), $t, _>(concat!("calc::", $name, $suffix), |_vm: &mut Vm, a: $t, b: $t| a.$f(b));
    };
}

/// one macro, both widths — the f32 row is the same body under the
/// `_f`-suffixed name
macro_rules! for_width {
    ($mac:ident, $hosts:expr, $name:literal, $f:ident) => {
        $mac!($hosts, $name, "", $f, f64);
        $mac!($hosts, $name, "_f", $f, f32);
    };
}

/// Install `calc`'s float host functions — both widths.
pub fn install_std_math(hosts: &mut HostRegistry) {
    for_width!(unary, hosts, "sqrt", sqrt);
    for_width!(unary, hosts, "floor", floor);
    for_width!(unary, hosts, "ceil", ceil);
    for_width!(unary, hosts, "round", round);
    for_width!(unary, hosts, "trunc", trunc);
    for_width!(unary, hosts, "exp", exp);
    for_width!(unary, hosts, "ln", ln);
    for_width!(unary, hosts, "log2", log2);
    for_width!(unary, hosts, "log10", log10);
    for_width!(unary, hosts, "sin", sin);
    for_width!(unary, hosts, "cos", cos);
    for_width!(unary, hosts, "tan", tan);
    for_width!(unary, hosts, "asin", asin);
    for_width!(unary, hosts, "acos", acos);
    for_width!(unary, hosts, "atan", atan);
    for_width!(unary, hosts, "sinh", sinh);
    for_width!(unary, hosts, "cosh", cosh);
    for_width!(unary, hosts, "tanh", tanh);

    for_width!(binary, hosts, "pow", powf);
    for_width!(binary, hosts, "atan2", atan2);
    for_width!(binary, hosts, "hypot", hypot);
    for_width!(binary, hosts, "copysign", copysign);

    for_width!(unary, hosts, "abs", abs);
    for_width!(binary, hosts, "min", min);
    for_width!(binary, hosts, "max", max);

    // JS `Math.sign` semantics (the old intrinsic's law): ±0 stay 0,
    // NaN passes through as itself — Rust's `signum` would map +0.0
    // to 1.0. Same law at both widths.
    hosts.register::<_, (f64,), f64, _>("calc::signum", |_vm: &mut Vm, x: f64| if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else if x == 0.0 {
        0.0
    } else {
        x // NaN
    });
    hosts.register::<_, (f32,), f32, _>("calc::signum_f", |_vm: &mut Vm, x: f32| if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else if x == 0.0 {
        0.0
    } else {
        x // NaN
    });

    hosts.register::<_, (f64, f64, f64), f64, _>(
        "calc::fma",
        |_vm: &mut Vm, a: f64, b: f64, c: f64| a.mul_add(b, c),
    );
    hosts.register::<_, (f32, f32, f32), f32, _>(
        "calc::fma_f",
        |_vm: &mut Vm, a: f32, b: f32, c: f32| a.mul_add(b, c),
    );
}
