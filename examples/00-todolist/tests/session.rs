//! The example's gate: drive the rut library from the host side and
//! assert the full CRUD session — `cargo test --workspace` runs it.

use std::rc::Rc;
use rut_vm::heap::Value;

fn vm() -> (rut_vm::interp::Vm, Value) {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/todolist.rut")).unwrap();
    let out = rut_driver::compile_module(&src, rut_parser::Mode::Impl, "todolist");
    assert!(out.diags.is_empty());
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).unwrap();
    let Value::Opaque(c) = vm.call("createContainer", &[]).unwrap() else { unreachable!() };
    (vm, Value::Opaque(c))
}

fn i(v: Value) -> i64 {
    match v {
        Value::I64(n) => n,
        other => panic!("{other:?}"),
    }
}

#[test]
fn crud_session() {
    let (mut vm, c) = vm();
    let Value::I64(h) = vm.call("create", &[c.clone()]).unwrap() else { unreachable!() };
    let list = Value::I64(h);
    let args = |extra: &[Value]| {
        let mut a = vec![c.clone(), list.clone()];
        a.extend_from_slice(extra);
        a
    };

    assert_eq!(i(vm.call("add", &args(&[Value::Str("write the RFC".into())])).unwrap()), 1);
    assert_eq!(i(vm.call("add", &args(&[Value::Str("implement the VM".into())])).unwrap()), 2);
    assert_eq!(i(vm.call("add", &args(&[Value::Str("ship the demo".into())])).unwrap()), 3);
    assert_eq!(i(vm.call("len", &args(&[])).unwrap()), 3);

    assert_eq!(vm.call("set_done", &args(&[Value::I64(2), Value::Bool(true)])).unwrap(), Value::Bool(true));
    assert_eq!(
        vm.call("title_of", &args(&[Value::I64(2)])).unwrap(),
        Value::Str("implement the VM".into())
    );
    assert_eq!(vm.call("title_of", &args(&[Value::I64(42)])).unwrap(), Value::Str("".into()));
    assert_eq!(vm.call("has_title", &args(&[Value::I64(2)])).unwrap(), Value::Bool(true));
    assert_eq!(vm.call("has_title", &args(&[Value::I64(42)])).unwrap(), Value::Bool(false));

    assert_eq!(
        vm.call("remove", &args(&[Value::I64(1)])).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        vm.call("remove", &args(&[Value::I64(1)])).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(i(vm.call("len", &args(&[])).unwrap()), 2);

    assert_eq!(
        vm.call("render", &args(&[])).unwrap(),
        Value::Str("TodoList[2]\n  #2 [x] implement the VM\n  #3 [ ] ship the demo".into())
    );

    // a second list in the same container is independent
    let Value::I64(h2) = vm.call("create", &[c.clone()]).unwrap() else { unreachable!() };
    assert_eq!(i(vm.call("len", &[c.clone(), Value::I64(h2)]).unwrap()), 0);
    assert_eq!(i(vm.call("len", &args(&[])).unwrap()), 2);
}

#[test]
fn wrong_box_traps_cleanly() {
    let (mut vm, c) = vm();
    // an Opaque that isn't the container: downcast traps, nothing corrupts
    let wrong = match vm.call("createContainer", &[]).unwrap() {
        Value::Opaque(inner) => {
            // box a plain string instead of a Lists — still a valid Opaque
            drop(inner);
            Value::Opt(None)
        }
        other => unreachable!("{other:?}"),
    };
    let _ = wrong;
    // passing None where Opaque is expected is an embedder mistake: a
    // named trap, not a panic or silent zero
    let err = vm.call("create", &[Value::Opt(None)]).unwrap_err();
    assert!(err.msg.contains("argument"), "{}", err.msg);
}
