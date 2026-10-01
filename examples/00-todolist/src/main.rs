//! 00-todolist — a Rust app embedding rut.
//!
//! `todolist.rut` is the application: a todo-list library with full CRUD.
//! This file is the embedder: compile, verify, then drive the library
//! through its `entry fn` surface — the host owns the session, holds the
//! opaque container and the list handles, and every call crosses with
//! plain values only.

use std::collections::BTreeMap;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Poll};

/// The manifest lane's url rows seed the fetcher from the committed
/// artifacts (`dist/std/` — the seed IS the cache: the offline gates
/// never touch the network; the human `rut fetch` lane fills the same
/// shape over the wire). Path rows mount natively inside the same load;
/// the four passes (deps walk, peer gate included) run for real.
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

/// The std-only driver for the `_with` lane: the fetched futures are
/// `ready`, so one noop-waker poll settles them.
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

/// The manifest lane's mount: parse this dir's `rut.toml`, seed the
/// fetcher with every url row's committed artifact, load.
fn load_todolist_session() -> Result<(rut_driver::Session, String), String> {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest_text = std::fs::read_to_string(base.join("rut.toml"))
        .map_err(|e| format!("rut.toml: {e}"))?;
    let manifest =
        rut_driver::bundle::parse_manifest(&manifest_text).map_err(|e| e.to_string())?;
    let dist = base.join("../../dist/std");
    let mut table = BTreeMap::new();
    for desc in manifest.deps.values() {
        let Some(url) = desc.get("url") else { continue };
        let artifact = url.rsplit('/').next().unwrap_or_default();
        let bytes = std::fs::read(dist.join(artifact))
            .map_err(|e| format!("the seed is the cache — cannot read {artifact}: {e}"))?;
        table.insert(url.clone(), bytes);
    }
    block_on(rut_driver::load_dir_session_with(base, &Table(table)))
}

fn main() {
    // the manifest lane: this dir's `rut.toml` carries the deps, the
    // load mounts the closure and runs the mount passes — then the same
    // embedder half as before (mount, compile, verify, drive)
    let (mut session, root) =
        load_todolist_session().expect("load the module dir");
    // the engine prelude (`core`) mounts here; `pouch` rode the
    // manifest — a third-party pkg the app declares, which the driver
    // does not know by name
    rut_driver::mount_std_core(&mut session);
    let g = rut_driver::compile_graph(&session, &root);
    assert!(g.diags.is_empty(), "{}", g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; "));
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
