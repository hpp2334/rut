//! 00-todolist — a Rust app embedding rut.
//!
//! `todolist.rut` is the application: a todo-list library with full CRUD.
//! This file is the embedder: compile, verify, then drive the library
//! through its `entry fn` surface — the host owns the session, holds the
//! opaque container and the list handles, and every call crosses with
//! plain values only (RFC 0023 §2).

use std::rc::Rc;

fn main() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/todolist.rut")).unwrap();
    // the app's libs: core+calc (engine) and `pouch` — a third-party pkg
    // the app declares, mounted from the toolchain tree (the driver
    // does not know its name)
    let mut session = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut session);
    rut_driver::mount_dir(
        &mut session,
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/pouch")),
    )
    .expect("mount pouch");
    let out = rut_driver::compile_module_in(&mut session, &src, rut_parser::Mode::Impl, "todolist");
    assert!(out.diags.is_empty());
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), rut_vm::interp::HostRegistry::new()).unwrap();

    // the session: container in, handles out, values back — typed
    let c: rut_vm::OpaqueRef = vm.call("createContainer", ()).unwrap();
    let list: u32 = vm.call("create", (c.clone(),)).unwrap();

    let mut add = |title: &str| -> i32 {
        vm.call::<_, i32>("add", (c.clone(), list, title)).unwrap()
    };
    let rfc = add("write the RFC");
    let vmf = add("implement the VM");
    let demo = add("ship the demo");
    println!("added: #{rfc}, #{vmf}, #{demo}");

    vm.call::<_, bool>("set_done", (c.clone(), list, vmf, true)).unwrap();
    println!("set_done(#{vmf}, true)");

    println!("title_of(#{vmf}) = {:?}", vm.call::<_, String>("title_of", (c.clone(), list, vmf)).unwrap());
    println!("title_of(42) = {:?}", vm.call::<_, String>("title_of", (c.clone(), list, 42)).unwrap());

    println!("remove(#{rfc}) = {:?}", vm.call::<_, bool>("remove", (c.clone(), list, rfc)).unwrap());
    println!("remove(#{rfc}) again = {:?}", vm.call::<_, bool>("remove", (c.clone(), list, rfc)).unwrap());

    let text: String = vm.call("render", (c.clone(), list)).unwrap();
    println!("{text}");

    // a second list in the same container — handles are independent
    let other: u32 = vm.call("create", (c.clone(),)).unwrap();
    let n: i32 = vm.call("len", (c.clone(), other)).unwrap();
    println!("second list #{other}: {n} todos");

    println!("fuel used: {} of {:?}", vm.fuel_used, limits.fuel);
}
