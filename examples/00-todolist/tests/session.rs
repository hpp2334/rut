//! The example's gate: drive the rut library from the host side and
//! assert the full CRUD session — `cargo test --workspace` runs it.

use std::rc::Rc;
use rut_vm::OpaqueRef;

fn vm() -> (rut_vm::interp::Vm, OpaqueRef) {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/todolist.rut")).unwrap();
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    rut_driver::mount_dir(
        &mut s,
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/pouch")),
    )
    .expect("mount pouch");
    let out = rut_driver::compile_module_in(&mut s, &src, rut_parser::Mode::Impl, "todolist");
    assert!(out.diags.is_empty());
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), rut_vm::interp::HostRegistry::new()).unwrap();
    let c: OpaqueRef = vm.call_typed("createContainer", ()).unwrap();
    (vm, c)
}

#[test]
fn crud_session() {
    let (mut vm, c) = vm();
    let list: u32 = vm.call_typed("create", (c.clone(),)).unwrap();

    assert_eq!(
        vm.call_typed::<_, i32>("add", (c.clone(), list, "write the RFC")).unwrap(),
        1
    );
    assert_eq!(
        vm.call_typed::<_, i32>("add", (c.clone(), list, "implement the VM")).unwrap(),
        2
    );
    assert_eq!(
        vm.call_typed::<_, i32>("add", (c.clone(), list, "ship the demo")).unwrap(),
        3
    );
    assert_eq!(vm.call_typed::<_, i32>("len", (c.clone(), list)).unwrap(), 3);

    assert_eq!(
        vm.call_typed::<_, bool>("set_done", (c.clone(), list, 2i32, true)).unwrap(),
        true
    );
    assert_eq!(
        vm.call_typed::<_, String>("title_of", (c.clone(), list, 2i32)).unwrap(),
        "implement the VM"
    );
    assert_eq!(
        vm.call_typed::<_, String>("title_of", (c.clone(), list, 42i32)).unwrap(),
        ""
    );
    assert_eq!(vm.call_typed::<_, bool>("has_title", (c.clone(), list, 2i32)).unwrap(), true);
    assert_eq!(vm.call_typed::<_, bool>("has_title", (c.clone(), list, 42i32)).unwrap(), false);

    assert_eq!(vm.call_typed::<_, bool>("remove", (c.clone(), list, 1i32)).unwrap(), true);
    assert_eq!(vm.call_typed::<_, bool>("remove", (c.clone(), list, 1i32)).unwrap(), false);
    assert_eq!(vm.call_typed::<_, i32>("len", (c.clone(), list)).unwrap(), 2);

    assert_eq!(
        vm.call_typed::<_, String>("render", (c.clone(), list)).unwrap(),
        "TodoList[2]\n  #2 [x] implement the VM\n  #3 [ ] ship the demo"
    );

    // a second list in the same container is independent
    let h2: u32 = vm.call_typed("create", (c.clone(),)).unwrap();
    assert_eq!(vm.call_typed::<_, i32>("len", (c.clone(), h2)).unwrap(), 0);
    assert_eq!(vm.call_typed::<_, i32>("len", (c.clone(), list)).unwrap(), 2);
}

#[test]
fn wrong_box_traps_cleanly() {
    let (mut vm, c) = vm();
    let _ = c;
    // passing a bool where the Opaque container is expected is an
    // embedder mistake: a named trap, not a panic or silent zero
    let err = vm.call_typed::<_, u32>("create", (false,)).unwrap_err();
    assert!(err.msg.contains("argument"), "{}", err.msg);
}
