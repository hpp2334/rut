//! The example's gate: drive the rut library from the host side and
//! assert the full CRUD session — `cargo test --workspace` runs it.

use std::future::Future;
use std::path::Path;
use std::task::{Context, Poll};
use rut_vm::OpaqueRef;

/// dist/std plays the wire: the committed artifacts prime an OFFLINE
/// remote (`DepRemote::write` is the stand-in for the GET), so this
/// gate cannot network by construction — no env, no set_var races.
/// The Loader + the warmed remote are the same embedder shape
/// src/main.rs spells, with the offline flavor swapped in.
fn warm(base: &Path) -> rut_native::HttpRemote {
    // a unique root per call: the tests run in parallel threads, and a
    // shared root would race the prime (remove/write/rename)
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let root = std::env::temp_dir()
        .join(format!("rut-00-todolist-cache-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let remote = rut_native::HttpRemote::offline(&root);
    let manifest_text = std::fs::read_to_string(base.join("rut.jsonc")).expect("rut.jsonc");
    let manifest =
        rut_driver::bundle::parse_manifest(&manifest_text).expect("parse rut.jsonc");
    let dist = base.join("../../dist/std");
    for desc in manifest.deps.values() {
        let Some(url) = desc.get("url") else { continue };
        let artifact = url.rsplit('/').next().unwrap_or_default();
        let bytes = std::fs::read(dist.join(artifact))
            .unwrap_or_else(|e| panic!("the committed artifact is the cache — {artifact}: {e}"));
        rut_native::DepRemote::write(&remote, url, &bytes).expect("prime the cache");
    }
    remote
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

fn load() -> rut_driver::Loaded {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let remote = warm(base);
    block_on(rut_native::load_path_session_with(base, &remote)).expect("load the module dir")
}

fn vm() -> (rut_vm::interp::Vm, OpaqueRef) {
    // the manifest lane: `rut.jsonc` carries the deps (pouch rides its
    // CDN bundle, pinned), the walk yields the pkgs — then the chain
    // (the core prelude auto-offers at `.compile()`)
    let loaded = load();
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the module dir");
    assert!(compiled.graph.diags.is_empty());
    rut_vm::verify::verify(compiled.graph.program.as_ref().expect("no binary emitted")).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .compiled(compiled)
        .limits(limits)
        .build()
        .unwrap();
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
