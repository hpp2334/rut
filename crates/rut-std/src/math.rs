//! `calc`'s host half (RFC 0028): the native-module functions behind
//! the `f64` math surface — the float primitives and the float
//! `abs`/`min`/`max`/`signum` helpers (the integer numeric methods are
//! core's `builtin impl` methods, compiler-lowered — no host body).
//!
//! The `calc` module is mounted by the driver (`rut-driver`); a host
//! installs the bodies. An uninstalled sink traps, like any missing host
//! function. Bindings are TYPED (RFC 0025): the signatures match
//! `calc.d.rut`, and `Vm::verify_host_fns` checks the contract at load
//! time.

use rut_core::types::TY_F64;
use rut_vm::interp::HostRegistry;
use rut_vm::Value;

/// Read the `i`-th `f64` argument; a non-float is `NaN` (the signature is
/// enforced by the verifier, so this is defensive).
fn arg(a: &[Value], i: usize) -> f64 {
    match a.get(i) {
        Some(Value::F64(x)) => *x,
        _ => f64::NAN,
    }
}

macro_rules! unary {
    ($hosts:expr, $name:literal, $f:ident) => {
        $hosts.register(
            concat!("calc::", $name),
            vec![rut_core::types::TY_F64],
            rut_core::types::TY_F64,
            |_vm, a| Ok(Value::F64(arg(a, 0).$f())),
        );
    };
}

macro_rules! binary {
    ($hosts:expr, $name:literal, $f:ident) => {
        $hosts.register(
            concat!("calc::", $name),
            vec![rut_core::types::TY_F64, rut_core::types::TY_F64],
            rut_core::types::TY_F64,
            |_vm, a| Ok(Value::F64(arg(a, 0).$f(arg(a, 1)))),
        );
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

    // the float helpers — ordinary host fns (the int forms are core's
    // `builtin impl` methods now); NaN comparisons are false, so
    // `min`/`max` keep the operand `b` on NaN
    unary!(hosts, "abs", abs);
    binary!(hosts, "min", min);
    binary!(hosts, "max", max);
    // JS `Math.sign` semantics (the old intrinsic's law): ±0 stay 0,
    // NaN passes through as itself — Rust's `f64::signum` would map
    // +0.0 to 1.0
    hosts.register("calc::signum", vec![TY_F64], TY_F64, |_vm, a| {        let x = arg(a, 0);
        let v = if x > 0.0 {
            1.0
        } else if x < 0.0 {
            -1.0
        } else if x == 0.0 {
            0.0
        } else {
            x // NaN
        };
        Ok(Value::F64(v))
    });

    hosts.register("calc::fma", vec![TY_F64, TY_F64, TY_F64], TY_F64, |_vm, a| {
        Ok(Value::F64(arg(a, 0).mul_add(arg(a, 1), arg(a, 2))))
    });
}
