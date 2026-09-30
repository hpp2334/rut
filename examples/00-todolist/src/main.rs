//! 00-todolist — a Rust app embedding rut.
//!
//! `todolist.rut` is the application: a todo-list library with full CRUD.
//! This file is the embedder: compile, verify, then drive the library
//! through its `entry fn` surface — the host owns the session, holds the
//! opaque container and the list handles, and every call crosses with
//! plain values only.

use std::rc::Rc;

fn main() {
    // the manifest lane: this dir's `rut.toml` carries the deps, the
    // load mounts the closure and runs the mount passes — then the same
    // embedder half as before (mount, compile, verify, drive)
    let (mut session, root) =
        rut_driver::load_dir_session(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("load the module dir");
    // the engine prelude (`core`) mounts here; `pouch` rode the
    // manifest — a third-party pkg the app declares, which the driver
    // does not know by name
    rut_driver::mount_std_core(&mut session);
    let g = rut_driver::compile_graph(&session, &root);
    assert!(g.diags.is_empty());
    let prog = g.program.expect("no binary emitted");
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
