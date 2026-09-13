//! Host-constructed `Opaque` boxes (RFC 0023/0026): the embedder news any
//! `'static` Rust value behind an `Opaque`, borrows it back typed, and rut
//! sees only the box. RFC 0014 semantics must hold on the rut side —
//! `o is Opaque` is `true`, `downcast<T>` is `None` for every `T` (never a
//! trap), `own` shares identity — and the payload's `Drop` runs when the
//! box's rc hits 0 (RFC 0016 §3).
//!
//! Note the drop-timing flow passes the box straight between host fns:
//! a wrapper-class INSTANCE also works (the `Logger` pattern) but today's
//! ref discipline does not recursively release record fields, so a box
//! stored in a class field lives until VM end (RFC 0016 §3, known edge).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use rut_vm::{OpaqueBox, Trap, Value};

const SRC: &str = r#"
import { Opaque, downcast, own } from "std:core";
import { store_new, store_set, store_get, store_size } from "plugin:boxes";

// the wrapper class (the Logger pattern, RFC 0028): one Opaque field,
// one host fn call per method — the Rust payload never leaks into rut
class Store {
    h: Opaque;
    fn adopt(h: Opaque) -> Self { return Self { h: h }; }
    fn set(mut self, k: str, v: i64) -> unit { store_set(self.h, k, v); }
    fn get(self, k: str) -> i64 { return store_get(self.h, k); }
}

entry fn make() -> Opaque { return store_new(); }
entry fn put(c: Opaque, k: str, v: i64) -> unit { store_set(c, k, v); }
entry fn get(c: Opaque, k: str) -> i64 { return store_get(c, k); }
entry fn size(c: Opaque) -> i64 { return store_size(c); }
entry fn wrap_put(c: Opaque, k: str, v: i64) -> unit {
    let s = Store.adopt(c);
    s.set(k, v);
}
entry fn wrap_get(c: Opaque, k: str) -> i64 {
    return Store.adopt(c).get(k);
}
entry fn erase_laws(c: Opaque) -> bool {
    // RFC 0014 on a host payload box: it is an Opaque and nothing more
    // specific — no rut type recovers from it, checked, never a trap
    return c is Opaque && downcast<i64>(c).is_none() && downcast<str>(c).is_none();
}
entry fn share(c: Opaque) -> Opaque {
    // own on a host box shares the box (no re-box, identity preserved)
    return own(c);
}
"#;

struct Store {
    map: HashMap<String, Value>,
    dropped: Rc<Cell<bool>>,
}

impl Drop for Store {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}

struct Widget {
    _x: i64,
}

fn session() -> rut_vm::interp::Vm {
    let mut session = rut_driver::Session::new();
    rut_driver::mount_std(&mut session);
    let ty = |n: &str| (n.to_string(), Vec::new(), rut_core::types::TY_OPAQUE);
    session
        .register_module(
            "plugin:boxes",
            rut_driver::Module {
                spec: "plugin:boxes".into(),
                host_funcs: vec![
                    ty("store_new"),
                    ("store_set".into(), vec![rut_core::types::TY_OPAQUE, rut_core::types::TY_STR, rut_core::types::TY_I64], rut_core::types::TY_UNIT),
                    ("store_get".into(), vec![rut_core::types::TY_OPAQUE, rut_core::types::TY_STR], rut_core::types::TY_I64),
                    ("store_size".into(), vec![rut_core::types::TY_OPAQUE], rut_core::types::TY_I64),
                ],
                ..Default::default()
            },
        )
        .unwrap();
    session
        .register_module(
            "app:boxes",
            rut_driver::Module { spec: "app:boxes".into(), source: Some(SRC.into()), ..Default::default() },
        )
        .unwrap();
    let g = rut_driver::compile_graph(&session, "app:boxes");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = g.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).unwrap()
}

/// Bind the `plugin:boxes` bodies over a real `HashMap` payload.
fn install(vm: &mut rut_vm::interp::Vm, dropped: &Rc<Cell<bool>>) {
    let dropped = dropped.clone();
    vm.register_host_fn("plugin:boxes::store_new", move |vm, _args| {
        let b = OpaqueBox::alloc(vm, Store { map: HashMap::new(), dropped: dropped.clone() })?;
        Ok(b.into_value())
    });
    vm.register_host_fn("plugin:boxes::store_set", |_vm, args| {
        let b = OpaqueBox::<Store>::from_value(&args[0])?;
        let Value::Str(k) = &args[1] else { return Err(Trap::new(rut_vm::TrapKind::Invalid, "arg 1: expected a string")) };
        let Value::I64(v) = args[2] else { return Err(Trap::new(rut_vm::TrapKind::Invalid, "arg 2: expected an integer")) };
        b.with_mut(|s| {
            s.map.insert(k.clone(), Value::I64(v));
        })?;
        Ok(Value::Unit)
    });
    vm.register_host_fn("plugin:boxes::store_get", |_vm, args| {
        let b = OpaqueBox::<Store>::from_value(&args[0])?;
        let Value::Str(k) = &args[1] else { return Err(Trap::new(rut_vm::TrapKind::Invalid, "arg 1: expected a string")) };
        Ok(b.with(|s| s.map.get(k).cloned())?.unwrap_or(Value::I64(-1)))
    });
    vm.register_host_fn("plugin:boxes::store_size", |_vm, args| {
        let b = OpaqueBox::<Store>::from_value(&args[0])?;
        Ok(Value::I64(b.with(|s| s.map.len() as i64)?))
    });
}

fn as_i64(v: Value) -> i64 {
    let Value::I64(x) = v else { unreachable!("{v:?}") };
    x
}

#[test]
fn host_boxes_hold_any_rust_type() {
    let dropped = Rc::new(Cell::new(false));
    let mut vm = session();
    install(&mut vm, &dropped);

    // news from the host, hold the handle across calls — the data lives
    // in the VM heap as long as either side holds a reference
    let held = vm.call("make", &[]).unwrap();
    let Value::Opaque(_) = &held else { unreachable!("{held:?}") };

    // the wrapper class drives it too — and its record pins the box (the
    // record-field discipline above), so the wrapper gets its own box
    vm.call("wrap_put", &[held.clone(), Value::Str("via-wrapper".into()), Value::I64(1)]).unwrap();
    assert_eq!(as_i64(vm.call("wrap_get", &[held.clone(), Value::Str("via-wrapper".into())]).unwrap()), 1);

    // a second box for the handle-lifecycle half of the test
    let held = vm.call("make", &[]).unwrap();
    vm.call("put", &[held.clone(), Value::Str("key-3".into()), Value::I64(41)]).unwrap();
    vm.call("put", &[held.clone(), Value::Str("key-7".into()), Value::I64(7)]).unwrap();
    assert_eq!(as_i64(vm.call("get", &[held.clone(), Value::Str("key-3".into())]).unwrap()), 41);
    assert_eq!(as_i64(vm.call("size", &[held.clone()]).unwrap()), 2);
    assert_eq!(as_i64(vm.call("get", &[held.clone(), Value::Str("missing".into())]).unwrap()), -1);

    // RFC 0014 erasure laws on the rut side
    let Value::Bool(true) = vm.call("erase_laws", &[held.clone()]).unwrap() else {
        unreachable!("erase_laws");
    };

    // own shares identity: writes through the copy are visible through
    // the original, and the handles are `==`
    let shared = vm.call("share", &[held.clone()]).unwrap();
    vm.call("put", &[shared.clone(), Value::Str("via-own".into()), Value::I64(9)]).unwrap();
    assert_eq!(as_i64(vm.call("get", &[held.clone(), Value::Str("via-own".into())]).unwrap()), 9);
    match (&shared, &held) {
        (Value::Opaque(a), Value::Opaque(b)) => assert_eq!(a, b),
        _ => unreachable!(),
    }

    // a wrong-type borrow is a checked error naming both sides
    let err = OpaqueBox::<Widget>::from_value(&held).err().expect("wrong type must be rejected");
    assert!(err.msg.contains("Store") && err.msg.contains("Widget"), "{}", err.msg);

    // a non-handle is rejected too
    let err = OpaqueBox::<Widget>::from_value(&Value::I64(5)).err().expect("non-handle must be rejected");
    assert!(err.msg.contains("expected an Opaque handle"), "{}", err.msg);

    // the borrow guard (RFC 0023 §2): a nested exclusive borrow is a
    // checked trap, and a shared borrow is excluded while `&mut` is out
    let b = OpaqueBox::<Store>::from_value(&held).unwrap();
    b.with_mut(|_| {
        assert!(b.with(|_| {}).is_err(), "shared borrow must be excluded under &mut");
        let again = b.with_mut(|_| {});
        assert!(again.unwrap_err().msg.contains("borrowed by an outer host call"));
    })
    .unwrap();
    b.with(|_| {}).expect("the guard clears when the closure returns");

    // rc-0 (RFC 0016 §3): this box's only references are the three
    // handles — the payload's Drop runs with the last one, VM alive or
    // not. (A box STORED in a wrapper-class field lives until VM end —
    // record fields are not recursively released today.)
    drop(held);
    drop(shared);
    assert!(!dropped.get(), "b still holds");
    drop(b);
    assert!(dropped.get(), "payload Drop must run deterministically at rc-0");
    drop(vm);
    assert!(dropped.get());
}
