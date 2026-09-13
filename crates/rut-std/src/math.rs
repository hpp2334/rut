//! `std:math`'s host half (RFC 0028): the native-module functions behind
//! the `f64` math surface. The integer intrinsics (`wrapping_*`,
//! `saturating_*`, `checked_*`, `abs`/`min`/`max`/`signum`) are
//! compiler-lowered and need no host body.
//!
//! The `std:math` module is mounted by the driver (`rut-driver`); a host
//! installs the bodies. An uninstalled sink traps, like any missing host
//! function.

use rut_vm::interp::Vm;
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
    ($vm:expr, $name:literal, $f:ident) => {
        $vm.register_host_fn(concat!("std:math::", $name), |_vm, a| {
            Ok(Value::F64(arg(a, 0).$f()))
        });
    };
}

macro_rules! binary {
    ($vm:expr, $name:literal, $f:ident) => {
        $vm.register_host_fn(concat!("std:math::", $name), |_vm, a| {
            Ok(Value::F64(arg(a, 0).$f(arg(a, 1))))
        });
    };
}

/// Install `std:math`'s `f64` host functions.
pub fn install_std_math(vm: &mut Vm) {
    unary!(vm, "sqrt", sqrt);
    unary!(vm, "floor", floor);
    unary!(vm, "ceil", ceil);
    unary!(vm, "round", round);
    unary!(vm, "trunc", trunc);
    unary!(vm, "exp", exp);
    unary!(vm, "ln", ln);
    unary!(vm, "log2", log2);
    unary!(vm, "log10", log10);
    unary!(vm, "sin", sin);
    unary!(vm, "cos", cos);
    unary!(vm, "tan", tan);
    unary!(vm, "asin", asin);
    unary!(vm, "acos", acos);
    unary!(vm, "atan", atan);
    unary!(vm, "sinh", sinh);
    unary!(vm, "cosh", cosh);
    unary!(vm, "tanh", tanh);

    binary!(vm, "pow", powf);
    binary!(vm, "atan2", atan2);
    binary!(vm, "hypot", hypot);
    binary!(vm, "copysign", copysign);

    vm.register_host_fn("std:math::fma", |_vm, a| {
        Ok(Value::F64(arg(a, 0).mul_add(arg(a, 1), arg(a, 2))))
    });
}
