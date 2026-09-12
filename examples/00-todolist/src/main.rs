//! 00-todolist — a Rust app embedding rut.
//!
//! `todolist.rut` is the application: a todo-list library with full CRUD.
//! This file is the embedder: compile, verify, then drive the library
//! through its `entry fn` surface — the host owns the session, holds the
//! opaque container and the list handles, and every call crosses with
//! plain values only (RFC 0023 §2).

use std::rc::Rc;
use rut_vm::heap::Value;

fn main() {
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
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks { print: None }).unwrap();

    // the session: container in, handles out, values back
    let Value::Opaque(c) = vm.call("createContainer", &[]).unwrap() else { unreachable!() };
    let Value::I64(list) = vm.call("create", &[Value::Opaque(c.clone())]).unwrap() else { unreachable!() };

    let mut add = |title: &str| -> i64 {
        match vm.call("add", &[Value::Opaque(c.clone()), Value::I64(list), Value::Str(title.into())]).unwrap() {
            Value::I64(id) => id,
            v => unreachable!("{v:?}"),
        }
    };
    let rfc = add("write the RFC");
    let vmf = add("implement the VM");
    let demo = add("ship the demo");
    println!("added: #{rfc}, #{vmf}, #{demo}");

    vm.call("set_done", &[Value::Opaque(c.clone()), Value::I64(list), Value::I64(vmf), Value::Bool(true)]).unwrap();
    println!("set_done(#{vmf}, true)");

    println!("title_of(#{vmf}) = {:?}", vm.call("title_of", &[Value::Opaque(c.clone()), Value::I64(list), Value::I64(vmf)]).unwrap());
    println!("title_of(42) = {:?}", vm.call("title_of", &[Value::Opaque(c.clone()), Value::I64(list), Value::I64(42)]).unwrap());

    println!("remove(#{rfc}) = {:?}", vm.call("remove", &[Value::Opaque(c.clone()), Value::I64(list), Value::I64(rfc)]).unwrap());
    println!("remove(#{rfc}) again = {:?}", vm.call("remove", &[Value::Opaque(c.clone()), Value::I64(list), Value::I64(rfc)]).unwrap());

    let Value::Str(text) = vm.call("render", &[Value::Opaque(c.clone()), Value::I64(list)]).unwrap() else { unreachable!() };
    println!("{text}");

    // a second list in the same container — handles are independent
    let Value::I64(other) = vm.call("create", &[Value::Opaque(c.clone())]).unwrap() else { unreachable!() };
    let Value::I64(n) = vm.call("len", &[Value::Opaque(c.clone()), Value::I64(other)]).unwrap() else { unreachable!() };
    println!("second list #{other}: {n} todos");

    println!("fuel used: {} of {:?}", vm.fuel_used, limits.fuel);
}
