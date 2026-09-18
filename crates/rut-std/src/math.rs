//! `calc`'s host half (RFC 0028): the native-module functions behind
//! the `f64` math surface — the float primitives and the float
//! `abs`/`min`/`max`/`signum` helpers (the integer numeric methods are
//! core's `builtin impl` methods, compiler-lowered — no host body).
//!
//! The `calc` module is mounted by the driver (`rut-driver`); a host
//! installs the bodies. Bindings are the MAGIC shape (RFC 0023/0025):
//! the closure's Rust parameter types ARE the `.d.rut` row — the
//! signature is derived and checked against `calc.d.rut` at load time.
//! These are the crossing-hot bindings: the adapter inlines to one
//! indirect call, the body reads its `f64`s straight out of the slots.

use rut_vm::interp::{HostRegistry, Vm};

macro_rules! unary {
    ($hosts:expr, $name:literal, $f:ident) => {
        $hosts.register::<_, (f64,), f64, _>(concat!("calc::", $name), |_vm: &mut Vm, x: f64| x.$f());
    };
}

macro_rules! binary {
    ($hosts:expr, $name:literal, $f:ident) => {
        $hosts.register::<_, (f64, f64), f64, _>(concat!("calc::", $name), |_vm: &mut Vm, a: f64, b: f64| a.$f(b));
    };
}

/// Install `calc`'s `f64` host functions.
pub fn install_std_math(hosts: &mut HostRegistry) {
    unary!(hosts, "sqrt", sqrt);
    unary!(hosts, "floor", floor);
    unary!(hosts, "ceil", ceil);
    unary!(hosts, "round", round);
    unary!(hosts, "trunc", trunc);
    unary!(hosts, "exp", exp);
    unary!(hosts, "ln", ln);
    unary!(hosts, "log2", log2);
    unary!(hosts, "log10", log10);
    unary!(hosts, "sin", sin);
    unary!(hosts, "cos", cos);
    unary!(hosts, "tan", tan);
    unary!(hosts, "asin", asin);
    unary!(hosts, "acos", acos);
    unary!(hosts, "atan", atan);
    unary!(hosts, "sinh", sinh);
    unary!(hosts, "cosh", cosh);
    unary!(hosts, "tanh", tanh);

    binary!(hosts, "pow", powf);
    binary!(hosts, "atan2", atan2);
    binary!(hosts, "hypot", hypot);
    binary!(hosts, "copysign", copysign);

    unary!(hosts, "abs", abs);
    binary!(hosts, "min", min);
    binary!(hosts, "max", max);

    // JS `Math.sign` semantics (the old intrinsic's law): ±0 stay 0,
    // NaN passes through as itself — Rust's `f64::signum` would map
    // +0.0 to 1.0
    hosts.register::<_, (f64,), f64, _>("calc::signum", |_vm: &mut Vm, x: f64| if x > 0.0 {
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
}
