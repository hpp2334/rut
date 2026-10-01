//! The example's gate: drive the rut library from the host side and
//! assert the full CRUD session — `cargo test --workspace` runs it.

use std::collections::BTreeMap;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Poll};
use rut_vm::OpaqueRef;

/// The manifest lane's url rows seed the fetcher from the committed
/// artifacts (the seed IS the cache — the gates never touch the
/// network; see src/main.rs for the annotated shape).
struct Table(BTreeMap<String, Vec<u8>>);

impl rut_driver::DepFetch for Table {
    fn dep_fetch(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, String>> {
        let r = match self.0.get(url) {
            Some(bytes) => Ok(bytes.clone()),
            None => Err(format!("no seeded bytes for {url}")),
        };
        std::future::ready(r)
    }
}

fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn load_session() -> (rut_driver::Session, String) {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest_text =
        std::fs::read_to_string(base.join("rut.toml")).expect("rut.toml");
    let manifest =
        rut_driver::bundle::parse_manifest(&manifest_text).expect("parse rut.toml");
    let dist = base.join("../../dist/std");
    let mut table = BTreeMap::new();
    for desc in manifest.deps.values() {
        let Some(url) = desc.get("url") else { continue };
        let artifact = url.rsplit('/').next().unwrap_or_default();
        let bytes = std::fs::read(dist.join(artifact))
            .unwrap_or_else(|e| panic!("the seed is the cache — {artifact}: {e}"));
        table.insert(url.clone(), bytes);
    }
    block_on(rut_driver::load_dir_session_with(base, &Table(table)))
        .expect("load the module dir")
}

fn vm() -> (rut_vm::interp::Vm, OpaqueRef) {
    // the manifest lane: `rut.toml` carries the deps (pouch rides its
    // CDN bundle, pinned), the load mounts the closure — then the same
    // embedder half as before
    let (mut s, root) = load_session();
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
