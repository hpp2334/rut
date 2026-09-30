//! The example's gate: drive the rut library from the host side and
//! assert the full CRUD session — `cargo test --workspace` runs it.

use std::rc::Rc;
use rut_vm::OpaqueRef;

fn vm() -> (rut_vm::interp::Vm, OpaqueRef) {
    // the manifest lane: `rut.toml` carries the deps, the load mounts
    // the closure — then the same embedder half as before
    let (mut s, root) =
        rut_driver::load_dir_session(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("load the module dir");
    rut_driver::mount_std_core(&mut s);
    let g = rut_driver::compile_graph(&s, &root);
    assert!(g.diags.is_empty());
    let prog = g.program.expect("no binary emitted");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), rut_vm::interp::HostRegistry::new()).unwrap();
    let c: OpaqueRef = vm.call("createContainer", ()).unwrap();
    (vm, c)
}

#[test]
fn crud_session() {
    let (mut vm, c) = vm();
    let list: u32 = vm.call("create", (c.clone(),)).unwrap();

    assert_eq!(
        vm.call::<_, i32>("add", (c.clone(), list, "write the RFC")).unwrap(),
        1
    );
    assert_eq!(
        vm.call::<_, i32>("add", (c.clone(), list, "implement the VM")).unwrap(),
        2
    );
    assert_eq!(
        vm.call::<_, i32>("add", (c.clone(), list, "ship the demo")).unwrap(),
        3
    );
    assert_eq!(vm.call::<_, i32>("len", (c.clone(), list)).unwrap(), 3);

    assert_eq!(
        vm.call::<_, bool>("set_done", (c.clone(), list, 2i32, true)).unwrap(),
        true
    );
    assert_eq!(
        vm.call::<_, String>("title_of", (c.clone(), list, 2i32)).unwrap(),
        "implement the VM"
    );
    assert_eq!(
        vm.call::<_, String>("title_of", (c.clone(), list, 42i32)).unwrap(),
        ""
    );
    assert_eq!(vm.call::<_, bool>("has_title", (c.clone(), list, 2i32)).unwrap(), true);
    assert_eq!(vm.call::<_, bool>("has_title", (c.clone(), list, 42i32)).unwrap(), false);

    assert_eq!(vm.call::<_, bool>("remove", (c.clone(), list, 1i32)).unwrap(), true);
    assert_eq!(vm.call::<_, bool>("remove", (c.clone(), list, 1i32)).unwrap(), false);
    assert_eq!(vm.call::<_, i32>("len", (c.clone(), list)).unwrap(), 2);

    assert_eq!(
        vm.call::<_, String>("render", (c.clone(), list)).unwrap(),
        "TodoList[2]\n  #2 [x] implement the VM\n  #3 [ ] ship the demo"
    );

    // a second list in the same container is independent
    let h2: u32 = vm.call("create", (c.clone(),)).unwrap();
    assert_eq!(vm.call::<_, i32>("len", (c.clone(), h2)).unwrap(), 0);
    assert_eq!(vm.call::<_, i32>("len", (c.clone(), list)).unwrap(), 2);
}

#[test]
fn wrong_box_traps_cleanly() {
    let (mut vm, c) = vm();
    let _ = c;
    // passing a bool where the opaque container is expected is an
    // embedder mistake: a named trap, not a panic or silent zero
    let err = vm.call::<_, u32>("create", (false,)).unwrap_err();
    assert!(err.msg.contains("argument"), "{}", err.msg);
}
