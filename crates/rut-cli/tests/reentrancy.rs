//! Re-entrant `vm.call` (RFC 0022 §1): a host fn holds `&mut Vm` and may
//! call back into rut while the outer frame is mid-op. The nested call
//! runs on a fresh frame stack under the same budget; the outer cursor is
//! restored whether the callee returns or traps. This is what makes the
//! RFC 0023 borrow guard load-bearing: a host `with_mut` held across a
//! nested call blocks any other host borrow of the same box, even one
//! taken from rut code that the nested call runs.

use std::cell::Cell;
use std::rc::Rc;

use rut_vm::{OpaqueBox, Trap, TrapKind, Value};

const SRC: &str = r#"
import { Opaque } from "std:core";
import { Vec } from "std:collection";
import { boost, borrow_conflict, borrow_read, borrow_try, count_spin, grind, host_boom, widget_new } from "plugin:re";

// plain rut math the host calls back into
entry fn inner(x: i64) -> i64 {
    return x + 1;
}

// host fn nested-calls `inner`; the outer frame's local `x` must survive
// the round trip untouched
entry fn outer(x: i64) -> i64 {
    let y = boost(x);
    return y * 2 + x;
}

// called from INSIDE a host `with_mut` — its `borrow_try` must see the
// guard and fail
entry fn poke(b: Opaque) -> i64 {
    return borrow_try(b);
}

entry fn outer_borrow(b: Opaque) -> i64 {
    let r = borrow_conflict(b);
    return r + borrow_read(b);
}

entry fn new_widget() -> Opaque {
    return widget_new();
}

entry fn boom(i: i64) -> i64 {
    let xs = Vec<i32>.zeroed(0);
    return xs[i as i32] as i64;
}

entry fn trigger_boom(i: i64) -> i64 {
    return host_boom(i);
}

entry fn spin(n: i64) -> i64 {
    let mut s = 0i64;
    for (let i = 0i64; i < n; i += 1) {
        s += i;
    }
    return s;
}

entry fn grind_caller(n: i64) -> i64 {
    return grind(n);
}

entry fn watchdog(n: i64) -> i64 {
    return count_spin(n);
}
"#;

struct Widget {
    n: i64,
}

fn session(fuel: Option<u64>) -> rut_vm::interp::Vm {
    let mut session = rut_driver::Session::new();
    rut_driver::mount_std(&mut session);
    let f = |n: &str, ps: Vec<u32>, r: u32| (n.to_string(), ps, r);
    use rut_core::types::{TY_I64, TY_OPAQUE};
    session
        .register_module(
            "plugin:re",
            rut_driver::Module {
                spec: "plugin:re".into(),
                host_funcs: vec![
                    f("widget_new", vec![], TY_OPAQUE),
                    f("boost", vec![TY_I64], TY_I64),
                    f("borrow_try", vec![TY_OPAQUE], TY_I64),
                    f("borrow_conflict", vec![TY_OPAQUE], TY_I64),
                    f("borrow_read", vec![TY_OPAQUE], TY_I64),
                    f("host_boom", vec![TY_I64], TY_I64),
                    f("grind", vec![TY_I64], TY_I64),
                    f("count_spin", vec![TY_I64], TY_I64),
                ],
                ..Default::default()
            },
        )
        .unwrap();
    session
        .register_module(
            "app:re",
            rut_driver::Module { spec: "app:re".into(), source: Some(SRC.into()), ..Default::default() },
        )
        .unwrap();
    let g = rut_driver::compile_graph(&session, "app:re");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = g.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel,
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).unwrap()
}

fn install(vm: &mut rut_vm::interp::Vm, invocations: &Rc<Cell<u32>>) {
    let invocations = invocations.clone();
    vm.register_host_fn("plugin:re::widget_new", |vm, _args| {
        let b = OpaqueBox::alloc(vm, Widget { n: 0 })?;
        Ok(b.into_value())
    });
    vm.register_host_fn("plugin:re::boost", |vm, args| {
        let Value::I64(x) = args[0] else { panic!("boost: i64 arg") };
        let v = vm.call("inner", &[Value::I64(x)])?;
        let Value::I64(y) = v else { panic!("inner: i64 result") };
        Ok(Value::I64(y + 1))
    });
    vm.register_host_fn("plugin:re::borrow_try", |_vm, args| {
        let b = OpaqueBox::<Widget>::from_value(&args[0])?;
        Ok(Value::I64(b.with(|w| w.n)?))
    });
    vm.register_host_fn("plugin:re::borrow_read", |_vm, args| {
        let b = OpaqueBox::<Widget>::from_value(&args[0])?;
        Ok(Value::I64(b.with(|w| w.n)?))
    });
    vm.register_host_fn("plugin:re::borrow_conflict", |vm, args| {
        let b = OpaqueBox::<Widget>::from_value(&args[0])?;
        // hold the mutable borrow ACROSS a nested vm.call — rut code that
        // runs inside must not be able to borrow the same box
        let r = b.with_mut(|w| {
            w.n += 1;
            vm.call("poke", &[args[0].clone()])
        });
        match r {
            // poke ran without touching the box (not this program's shape)
            Ok(Ok(v)) => Ok(v),
            // poke trapped on the guard — caught here, the outer rut frame
            // keeps going (marker -100)
            Ok(Err(_t)) => Ok(Value::I64(-100)),
            // the with_mut itself failed
            Err(t) => Err(t),
        }
    });
    vm.register_host_fn("plugin:re::host_boom", |vm, args| {
        let Value::I64(i) = args[0] else { panic!("host_boom: i64 arg") };
        let v = vm.call("boom", &[Value::I64(i)])?;
        Ok(v)
    });
    vm.register_host_fn("plugin:re::grind", |vm, args| {
        let Value::I64(n) = args[0] else { panic!("grind: i64 arg") };
        // the catch-and-refuel pattern for nested budget traps: the host
        // owns the retry, the outer frame never sees the trap
        match vm.call("spin", &[Value::I64(n)]) {
            Ok(v) => Ok(v),
            Err(t) if t.kind == TrapKind::OutOfFuel => {
                vm.add_fuel(2_000_000);
                vm.call("spin", &[Value::I64(n)])
            }
            Err(t) => Err(t),
        }
    });
    vm.register_host_fn("plugin:re::count_spin", move |vm, args| {
        let Value::I64(n) = args[0] else { panic!("count_spin: i64 arg") };
        invocations.set(invocations.get() + 1);
        vm.call("spin", &[Value::I64(n)])
    });
}

fn as_i64(v: Value) -> i64 {
    let Value::I64(x) = v else { unreachable!("{v:?}") };
    x
}

#[test]
fn nested_call_returns_and_outer_locals_survive() {
    let invocations = Rc::new(Cell::new(0));
    let mut vm = session(Some(1_000_000));
    install(&mut vm, &invocations);
    // boost nested-calls inner(x)=x+1 and adds 1; outer computes y*2+x
    // with its OWN x — 7*2+5
    assert_eq!(as_i64(vm.call("outer", &[Value::I64(5)]).unwrap()), 19);
}

#[test]
fn borrow_guard_blocks_rut_running_inside_with_mut() {
    let invocations = Rc::new(Cell::new(0));
    let mut vm = session(Some(1_000_000));
    install(&mut vm, &invocations);
    let b = vm.call("new_widget", &[]).unwrap();
    // borrow_conflict: with_mut(+1) -> nested `poke` -> borrow_try's
    // `with` fails on the guard (-100 marker) and the mutation stands (n=1)
    assert_eq!(as_i64(vm.call("outer_borrow", &[b]).unwrap()), -99);
}

#[test]
fn nested_trap_propagates_and_vm_stays_usable() {
    let invocations = Rc::new(Cell::new(0));
    let mut vm = session(Some(1_000_000));
    install(&mut vm, &invocations);
    // boom index-OOBs inside the nested call; the trap unwinds the nested
    // frame only and surfaces at the embedder with its kind intact
    let err = vm.call("trigger_boom", &[Value::I64(0)]).unwrap_err();
    assert_eq!(err.kind, TrapKind::IndexOutOfBounds, "got: {}", err.msg);
    // the parked outer frame does not block later calls — a fresh entry
    // nests over it cleanly
    assert_eq!(as_i64(vm.call("inner", &[Value::I64(41)]).unwrap()), 42);
}

#[test]
fn nested_budget_trap_catch_add_fuel_retry() {
    let invocations = Rc::new(Cell::new(0));
    // spin(60_000) needs far more than 30k fuel; grind catches the
    // nested OutOfFuel, refuels, and retries — the outer frame never
    // learns the budget tripped
    let mut vm = session(Some(30_000));
    install(&mut vm, &invocations);
    let n = 60_000i64;
    let want = n * (n - 1) / 2;
    assert_eq!(as_i64(vm.call("grind_caller", &[Value::I64(n)]).unwrap()), want);
}

#[test]
fn resume_reruns_the_host_op_after_propagated_out_of_fuel() {
    let invocations = Rc::new(Cell::new(0));
    // the nested OutOfFuel propagates past count_spin to the embedder;
    // the vm parks AT the host op, so resume() re-runs count_spin (its
    // invocation count proves the re-run) rather than skipping the call
    let mut vm = session(Some(20_000));
    install(&mut vm, &invocations);
    let n = 50_000i64;
    let want = n * (n - 1) / 2;
    let err = vm.call("watchdog", &[Value::I64(n)]).unwrap_err();
    assert_eq!(err.kind, TrapKind::OutOfFuel);
    assert_eq!(invocations.get(), 1);
    vm.add_fuel(2_000_000);
    assert_eq!(as_i64(vm.resume().unwrap()), want);
    assert_eq!(invocations.get(), 2);
}
