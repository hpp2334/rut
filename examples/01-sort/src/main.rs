//! 01-sort — a Rust app embedding rut.
//!
//! `sort.rut` is the application: a sorting library — five algorithms
//! (insertion / bubble / selection / quicksort / merge sort) behind one
//! `entry fn` dispatcher. This file is the embedder: compile, verify,
//! then drive the library — the host owns the session, holds the opaque
//! bank, and reads results back as one JSON array string (`serialize`);
//! `Vec<i32>` itself never crosses.

use std::future::Future;
use std::path::Path;
use std::task::{Context, Poll};

/// The embedder's ENTIRE load half — the Loader door. The project is
/// this directory; the remote policy is the project-local cache. A
/// cold start networks on its misses (the CDN, pinned by the
/// manifest); a warm start is pure cache hits.
fn load(base: &Path) -> Result<rut_driver::Loaded, rut_driver::RunError> {
    let remote = rut_native::HttpRemote::project_local(base);
    block_on(rut_native::load_path_session_with(base, &remote))
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

fn main() {
    // the manifest lane: this dir's `rut.jsonc` carries the deps, the
    // walk runs its passes and yields the pkgs — then the chain
    // (calc offered; the core prelude auto-rides), verify, drive
    let loaded = load(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("load the module dir");
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .host_pkg(rut_std::math::pkg()) // calc: .d.rut ↔ bodies, checked at the install
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the module dir");
    assert!(compiled.graph.diags.is_empty());
    rut_vm::verify::verify(compiled.graph.program.as_ref().expect("no binary emitted")).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(5_000_000),
        heap_limit_bytes: Some(8 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .compiled(compiled)
        .limits(limits)
        .build()
        .unwrap();


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

    println!("fuel used: {} of {:?}", vm.fuel_used, Some(5_000_000u64));
}
