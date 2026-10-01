//! 01-sort — a Rust app embedding rut.
//!
//! `sort.rut` is the application: a sorting library — five algorithms
//! (insertion / bubble / selection / quicksort / merge sort) behind one
//! `entry fn` dispatcher. This file is the embedder: compile, verify,
//! then drive the library — the host owns the session, holds the opaque
//! bank, and reads results back as one JSON array string (`serialize`);
//! `Vec<i32>` itself never crosses.

use std::collections::BTreeMap;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Poll};

/// The manifest lane's url rows seed the fetcher from the committed
/// artifacts (`dist/std/` — the seed IS the cache: the offline gates
/// never touch the network). Path rows mount natively inside the same
/// load; the four passes (deps walk, peer gate included) run for real.
struct Table(BTreeMap<String, Vec<u8>>);

impl rut_driver::DepRemote for Table {
    fn fetch(
        &self,
        url: &str,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Vec<u8>, rut_driver::RemoteError>> + '_>> {
        let r = match self.0.get(url) {
            Some(bytes) => Ok(bytes.clone()),
            None => Err(rut_driver::RemoteError::new(format!("no seeded bytes for {url}"))),
        };
        Box::pin(std::future::ready(r))
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
fn load_sort_session() -> Result<(rut_driver::Session, String), String> {
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
        .map_err(|e| e.to_string())
}

fn main() {
    // the manifest lane: this dir's `rut.toml` carries the deps, the
    // load mounts the closure and runs the mount passes — then the same
    // embedder half as before (mount, compile, verify, drive)
    let (mut session, root) =
        load_sort_session().expect("load the module dir");
    // the app's libs: core+calc (engine) — `calc` is ambient, and
    // `pouch` rode the manifest
    rut_driver::mount_std(&mut session);
    let g = rut_driver::compile_graph(&session, &root);
    assert!(g.diags.is_empty());
    let prog = g.program.expect("no binary emitted");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(5_000_000),
        heap_limit_bytes: Some(8 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let ctx = session.host_pkg_context();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.verify_against(&session.expected_host_fns()); // calc: .d.rut ↔ bodies
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap();


    // the session: one bank, exact values in, JSON out — typed
    let c: rut_vm::OpaqueRef = vm.call("create", ()).unwrap();
    let ser = |vm: &mut rut_vm::interp::Vm| -> String {
        vm.call::<_, String>("serialize", (c.clone(),)).unwrap()
    };

    // an exact host-supplied input, sorted by hand-picked algorithms
    for x in [5i32, 2, 9, 2] {
        vm.call::<_, ()>("push", (c.clone(), x)).unwrap();
    }
    println!("pushed: {}", ser(&mut vm));
    vm.call::<_, String>("sort", (c.clone(), "insertion")).unwrap();
    println!("after insertion: {}", ser(&mut vm));

    // the full sweep: same deterministic input, every algorithm, fuel
    // per call (the session runs under budgets)
    println!("fill(16, seed=42), each algorithm:");
    for algo in ["insertion", "bubble", "selection", "quick", "merge"] {
        vm.call::<_, ()>("fill", (c.clone(), 16u32, 42u32)).unwrap();
        let before = vm.fuel_used;
        vm.call::<_, String>("sort", (c.clone(), algo)).unwrap();
        let sorted: bool = vm.call("is_sorted", (c.clone(),)).unwrap();
        println!("  {algo:<9} {sorted}  fuel {:>6}  {}", vm.fuel_used - before, ser(&mut vm));
    }

    // unknown names are values, not traps
    println!("sort(bogus) = {:?}", vm.call::<_, String>("sort", (c.clone(), "bogus")).unwrap());

    println!("fuel used: {} of {:?}", vm.fuel_used, limits.fuel);
}
