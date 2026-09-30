//! `calc`'s host half: the native-module functions behind
//! the float math surface — `f64` members and their `f32` twins
//! (`sqrt_f` …; the `_f` suffix carries the width, rut has no
//! overloading) — the float primitives and the float
//! `abs`/`min`/`max`/`signum` helpers (the integer numeric methods are
//! core's `builtin impl` methods, compiler-lowered — no host body).
//!
//! The `calc` module is mounted by the driver (`rut-driver`); a host
//! installs the bodies. Bindings are the MAGIC shape:
//! the closure's Rust parameter types ARE the `.d.rut` row — the
//! signature is derived and checked against `calc.d.rut` at install
//! time. These are the crossing-hot bindings: the adapter inlines to
//! one indirect call, the body reads its floats straight out of the
//! slots.

use rut_vm::interp::{HostPkg, Vm};

macro_rules! unary {
    ($pkg:expr, $name:literal, $suffix:literal, $f:ident, $t:ty) => {
        $pkg.register::<_, ($t,), $t, _>(concat!($name, $suffix), |_vm: &mut Vm, x: $t| x.$f());
    };
}

macro_rules! binary {
    ($pkg:expr, $name:literal, $suffix:literal, $f:ident, $t:ty) => {
        $pkg.register::<_, ($t, $t), $t, _>(concat!($name, $suffix), |_vm: &mut Vm, a: $t, b: $t| a.$f(b));
    };
}

/// one macro, both widths — the f32 row is the same body under the
/// `_f`-suffixed name
macro_rules! for_width {
    ($mac:ident, $pkg:expr, $name:literal, $f:ident) => {
        $mac!($pkg, $name, "", $f, f64);
        $mac!($pkg, $name, "_f", $f, f32);
    };
}

/// Build `calc`'s float host pkg — both widths.
pub fn pkg() -> HostPkg {
    let mut pkg = HostPkg::new("calc");
    for_width!(unary, pkg, "sqrt", sqrt);
    for_width!(unary, pkg, "floor", floor);
    for_width!(unary, pkg, "ceil", ceil);
    for_width!(unary, pkg, "round", round);
    for_width!(unary, pkg, "trunc", trunc);
    for_width!(unary, pkg, "exp", exp);
    for_width!(unary, pkg, "ln", ln);
    for_width!(unary, pkg, "log2", log2);
    for_width!(unary, pkg, "log10", log10);
    for_width!(unary, pkg, "sin", sin);
    for_width!(unary, pkg, "cos", cos);
    for_width!(unary, pkg, "tan", tan);
    for_width!(unary, pkg, "asin", asin);
    for_width!(unary, pkg, "acos", acos);
    for_width!(unary, pkg, "atan", atan);
    for_width!(unary, pkg, "sinh", sinh);
    for_width!(unary, pkg, "cosh", cosh);
    for_width!(unary, pkg, "tanh", tanh);

    for_width!(binary, pkg, "pow", powf);
    for_width!(binary, pkg, "atan2", atan2);
    for_width!(binary, pkg, "hypot", hypot);
    for_width!(binary, pkg, "copysign", copysign);

    for_width!(unary, pkg, "abs", abs);
    for_width!(binary, pkg, "min", min);
    for_width!(binary, pkg, "max", max);

    // JS `Math.sign` semantics (the old intrinsic's law): ±0 stay 0,
    // NaN passes through as itself — Rust's `signum` would map +0.0
    // to 1.0. Same law at both widths.
    pkg.register::<_, (f64,), f64, _>("signum", |_vm: &mut Vm, x: f64| if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else if x == 0.0 {
        0.0
    } else {
        x // NaN
    });
    pkg.register::<_, (f32,), f32, _>("signum_f", |_vm: &mut Vm, x: f32| if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else if x == 0.0 {
        0.0
    } else {
        x // NaN
    });

    pkg.register::<_, (f64, f64, f64), f64, _>(
        "fma",
        |_vm: &mut Vm, a: f64, b: f64, c: f64| a.mul_add(b, c),
    );
    pkg.register::<_, (f32, f32, f32), f32, _>(
        "fma_f",
        |_vm: &mut Vm, a: f32, b: f32, c: f32| a.mul_add(b, c),
    );
    pkg.build()
}
