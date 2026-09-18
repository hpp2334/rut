//! Re-entrant `vm.call` (RFC 0022 §1): a host fn holds `&mut Vm` and may
//! call back into rut while the outer frame is mid-op. The nested call
//! runs on a fresh frame stack under the same budget; the outer cursor is
//! restored whether the callee returns or traps. This is what makes the
//! RFC 0023 borrow guard load-bearing: a host `with_mut` held across a
//! nested call blocks any other host borrow of the same box, even one
//! taken from rut code that the nested call runs.

use std::cell::Cell;
use std::rc::Rc;

use rut_vm::{OpaqueBox, OpaqueRef, Trap, TrapKind};
use rut_vm::interp::Vm;

const SRC: &str = r#"
use core::{ Opaque };
use pouch::{ Vec };
use re::{ boost, borrow_conflict, borrow_read, borrow_try, count_spin, grind, host_boom, widget_new };

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

type ExpectedHostFns = std::collections::BTreeMap<
    String,
    (Vec<rut_core::types::TypeId>, rut_core::types::TypeId),
>;

fn session(fuel: Option<u64>, invocations: &Rc<Cell<u32>>) -> rut_vm::interp::Vm {
    let mut session = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut session);
    // the source uses `pouch` — a third-party pkg, mounted from the tree
    rut_driver::mount_dir(
        &mut session,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/pouch"),
    )
    .expect("mount pouch");
    // `re` — this test's own host pkg, declared in tests/data/re
    rut_driver::mount_dir(
        &mut session,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/re"),
    )
    .expect("mount re");
    let expected = session.expected_host_fns();
    session
        .register_module(
            "app_re",
            rut_driver::Module { spec: "app_re".into(), source: Some(SRC.into()), ..Default::default() },
        )
        .unwrap();
    let g = rut_driver::compile_graph(&session, "app_re");
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
    // bindings BEFORE the Vm (RFC 0025): install + contract + boot
    let mut hosts = rut_vm::interp::HostRegistry::new();
    install(&mut hosts, &invocations);
    hosts.verify_against(&expected); // tests/data/re/re.d.rut ↔ the bodies
    let vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap();
    vm
}

fn install(hosts: &mut rut_vm::interp::HostRegistry, invocations: &Rc<Cell<u32>>) {
    let invocations = invocations.clone();
    rut_vm::register!(hosts, "re::widget_new", () -> OpaqueRef, |vm: &mut Vm| -> Result<OpaqueRef, Trap> {
        let b = OpaqueBox::alloc(vm, Widget { n: 0 })?;
        Ok(b.handle().clone())
    });
    rut_vm::register!(hosts, "re::boost", (i64,) -> i64, |vm: &mut Vm, x: i64| -> Result<i64, Trap> {
        let y: i64 = vm.call_typed("inner", (x,))?;
        Ok(y + 1)
    });
    rut_vm::register!(hosts, "re::borrow_try", (OpaqueBox<Widget>,) -> i64, |_vm: &mut Vm, b: OpaqueBox<Widget>| b.with(|w| w.n));
    rut_vm::register!(hosts, "re::borrow_read", (OpaqueBox<Widget>,) -> i64, |_vm: &mut Vm, b: OpaqueBox<Widget>| b.with(|w| w.n));
    rut_vm::register!(hosts, "re::borrow_conflict", (OpaqueBox<Widget>,) -> i64, |vm: &mut Vm, b: OpaqueBox<Widget>| -> Result<i64, Trap> {
        // hold the mutable borrow ACROSS a nested vm.call — rut code that
        // runs inside must not be able to borrow the same box
        let r = b.with_mut(|w| {
            w.n += 1;
            vm.call_typed::<_, i64>("poke", (b.handle().clone(),))
        });
        match r {
            // poke ran without touching the box (not this program's shape)
            Ok(Ok(v)) => Ok(v),
            // poke trapped on the guard — caught here, the outer rut frame
            // keeps going (marker -100)
            Ok(Err(_t)) => Ok(-100),
            // the with_mut itself failed
            Err(t) => Err(t),
        }
    });
    rut_vm::register!(hosts, "re::host_boom", (i64,) -> i64, |vm: &mut Vm, i: i64| vm.call_typed::<_, i64>("boom", (i,)));
    rut_vm::register!(hosts, "re::grind", (i64,) -> i64, |vm: &mut Vm, n: i64| -> Result<i64, Trap> {
        // the catch-and-refuel pattern for nested budget traps: the host
        // owns the retry, the outer frame never sees the trap
        match vm.call_typed::<_, i64>("spin", (n,)) {
            Ok(v) => Ok(v),
            Err(t) if t.kind == TrapKind::OutOfFuel => {
                vm.add_fuel(2_000_000);
                vm.call_typed::<_, i64>("spin", (n,))
            }
            Err(t) => Err(t),
        }
    });
    rut_vm::register!(hosts, "re::count_spin", (i64,) -> i64, move |vm: &mut Vm, n: i64| -> Result<i64, Trap> {
        invocations.set(invocations.get() + 1);
        vm.call_typed::<_, i64>("spin", (n,))
    });
}

#[test]
fn nested_call_returns_and_outer_locals_survive() {
    let invocations = Rc::new(Cell::new(0));
    let mut vm = session(Some(1_000_000), &invocations);
    
    // boost nested-calls inner(x)=x+1 and adds 1; outer computes y*2+x
    // with its OWN x — 7*2+5
    assert_eq!(vm.call_typed::<_, i64>("outer", (5,)).unwrap(), 19);
}

#[test]
fn borrow_guard_blocks_rut_running_inside_with_mut() {
    let invocations = Rc::new(Cell::new(0));
    let mut vm = session(Some(1_000_000), &invocations);
    
    let b: OpaqueRef = vm.call_typed("new_widget", ()).unwrap();
    // borrow_conflict: with_mut(+1) -> nested `poke` -> borrow_try's
    // `with` fails on the guard (-100 marker) and the mutation stands (n=1)
    assert_eq!(vm.call_typed::<_, i64>("outer_borrow", (b,)).unwrap(), -99);
}

#[test]
fn nested_trap_propagates_and_vm_stays_usable() {
    let invocations = Rc::new(Cell::new(0));
    let mut vm = session(Some(1_000_000), &invocations);
    
    // boom index-OOBs inside the nested call; the trap unwinds the nested
    // frame only and surfaces at the embedder with its kind intact
    let err = vm.call_typed::<_, i64>("trigger_boom", (0,)).unwrap_err();
    assert_eq!(err.kind, TrapKind::IndexOutOfBounds, "got: {}", err.msg);
    // the parked outer frame does not block later calls — a fresh entry
    // nests over it cleanly
    assert_eq!(vm.call_typed::<_, i64>("inner", (41,)).unwrap(), 42);
}

#[test]
fn nested_budget_trap_catch_add_fuel_retry() {
    let invocations = Rc::new(Cell::new(0));
    // spin(60_000) needs far more than 30k fuel; grind catches the
    // nested OutOfFuel, refuels, and retries — the outer frame never
    // learns the budget tripped
    let mut vm = session(Some(30_000), &invocations);
    
    let n = 60_000i64;
    let want = n * (n - 1) / 2;
    assert_eq!(vm.call_typed::<_, i64>("grind_caller", (n,)).unwrap(), want);
}

#[test]
fn resume_reruns_the_host_op_after_propagated_out_of_fuel() {
    let invocations = Rc::new(Cell::new(0));
    // the nested OutOfFuel propagates past count_spin to the embedder;
    // the vm parks AT the host op, so resume() re-runs count_spin (its
    // invocation count proves the re-run) rather than skipping the call
    let mut vm = session(Some(20_000), &invocations);
    
    let n = 50_000i64;
    let want = n * (n - 1) / 2;
    let err = vm.call_typed::<_, i64>("watchdog", (n,)).unwrap_err();
    assert_eq!(err.kind, TrapKind::OutOfFuel);
    assert_eq!(invocations.get(), 1);
    vm.add_fuel(2_000_000);
    assert_eq!(vm.resume_typed::<i64>().unwrap(), want);
    assert_eq!(invocations.get(), 2);
}
