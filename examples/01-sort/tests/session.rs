//! The example's gate: drive the rut sorting library from the host side
//! and assert the whole session — every algorithm, edge cases, the JSON
//! round trip, the error path, and trap cleanliness. Runs under
//! `cargo test --workspace`.

use std::future::Future;
use std::path::Path;
use std::task::{Context, Poll};
use rut_vm::OpaqueRef;

/// dist/std-v8 plays the wire: the PINNED artifacts (the published
/// std-v8 bytes the manifest rows name) prime an OFFLINE
/// remote (`DepRemote::write` is the stand-in for the GET), so this
/// gate cannot network by construction — no env, no set_var races.
fn warm(base: &Path) -> rut_native::HttpRemote {
    // a unique root per call: the tests run in parallel threads, and a
    // shared root would race the prime (remove/write/rename)
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let root = std::env::temp_dir()
        .join(format!("rut-01-sort-cache-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let remote = rut_native::HttpRemote::offline(&root);
    let manifest_text = std::fs::read_to_string(base.join("rut.jsonc")).expect("rut.jsonc");
    let manifest =
        rut_driver::bundle::parse_manifest(&manifest_text).expect("parse rut.jsonc");
    let dist = base.join("../../dist/std-v8");
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
    // (calc offered; the core prelude auto-rides)
    let loaded = load();
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .host_pkg(rut_std::math::pkg()) // calc: .d.rut ↔ bodies, checked at the install
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the module dir");
    assert!(
        compiled.graph.diags.is_empty(),
        "{}",
        compiled.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
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

    let c: OpaqueRef = vm.call("create", ()).unwrap();
    (vm, c)
}

const ALGOS: [&str; 5] = ["insertion", "bubble", "selection", "quick", "merge"];

fn push_all(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef, xs: &[i32]) {
    for x in xs {
        vm.call::<_, ()>("push", (c.clone(), *x)).unwrap();
    }
}

fn serialize(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef) -> String {
    vm.call("serialize", (c.clone(),)).unwrap()
}

fn is_sorted(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef) -> bool {
    vm.call("is_sorted", (c.clone(),)).unwrap()
}

fn has(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef, i: i32) -> bool {
    vm.call("has", (c.clone(), i)).unwrap()
}

fn get(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef, i: i32) -> i32 {
    vm.call("get", (c.clone(), i)).unwrap()
}

fn sort(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef, algo: &str) -> String {
    vm.call("sort", (c.clone(), algo)).unwrap()
}

fn json(xs: &[i32]) -> String {
    format!("[{}]", xs.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "))
}

#[test]
fn every_algorithm_sorts_a_known_input() {
    for algo in ALGOS {
        let (mut vm, c) = vm();
        push_all(&mut vm, &c, &[9, -3, 5, 5, 0, 42, -7, 3]);
        assert_eq!(sort(&mut vm, &c, algo), "", "{algo}");
        assert_eq!(
            serialize(&mut vm, &c),
            "[-7, -3, 0, 3, 5, 5, 9, 42]",
            "{algo}",
        );
        assert!(is_sorted(&mut vm, &c), "{algo}");
    }
}

#[test]
fn edge_cases_for_every_algorithm() {
    let cases: &[(&str, &[i32])] = &[
        ("empty", &[]),
        ("single", &[7]),
        ("duplicates", &[4, 4, 4, 1, 1]),
        ("already sorted", &[1, 2, 3, 4, 5]),
        ("reverse", &[5, 4, 3, 2, 1]),
        ("negatives", &[-5, 3, -1, 0, -9, 2]),
    ];
    for algo in ALGOS {
        for (name, input) in cases {
            let (mut vm, c) = vm();
            push_all(&mut vm, &c, input);
            sort(&mut vm, &c, algo);
            let mut want = input.to_vec();
            want.sort_unstable();
            assert_eq!(
                serialize(&mut vm, &c),
                json(&want),
                "{algo} / {name}",
            );
            assert!(is_sorted(&mut vm, &c), "{algo} / {name}");
        }
    }
}

#[test]
fn all_algorithms_agree_on_pseudo_random_input() {
    // same deterministic fill, every algorithm — one answer
    let mut expected: Option<String> = None;
    for algo in ALGOS {
        let (mut vm, c) = vm();
        vm.call::<_, ()>("fill", (c.clone(), 500u32, 7u32)).unwrap();
        assert_eq!(vm.call::<_, i32>("len", (c.clone(),)).unwrap(), 500);
        sort(&mut vm, &c, algo);
        assert!(is_sorted(&mut vm, &c), "{algo}");
        let got = serialize(&mut vm, &c);
        match &expected {
            None => expected = Some(got),
            Some(want) => assert_eq!(&got, want, "{algo} disagrees with the others"),
        }
    }
}

#[test]
fn large_input_is_monotonic_through_get() {
    let (mut vm, c) = vm();
    vm.call::<_, ()>("fill", (c.clone(), 500u32, 1234u32)).unwrap();
    sort(&mut vm, &c, "quick");
    let mut prev = i32::MIN;
    for i in 0..500 {
        assert!(has(&mut vm, &c, i), "has({i})");
        let x: i32 = get(&mut vm, &c, i);
        assert!(x >= prev, "not monotonic at {i}: {x} < {prev}");
        prev = x;
    }
    // out of range reads 0, and `has` says false — no trap
    assert_eq!(vm.call::<_, i32>("get", (c.clone(), 500i32)).unwrap(), 0);
    assert_eq!(vm.call::<_, bool>("has", (c.clone(), 500i32)).unwrap(), false);
    assert_eq!(vm.call::<_, bool>("has", (c.clone(), -1i32)).unwrap(), false);
}

#[test]
fn unknown_algorithm_is_an_error_string() {
    let (mut vm, c) = vm();
    push_all(&mut vm, &c, &[1]);
    assert_eq!(sort(&mut vm, &c, "bogus"), "unknown algorithm: bogus");
    // a failed dispatch leaves the data untouched
    assert_eq!(serialize(&mut vm, &c), "[1]");
}

#[test]
fn wrong_container_traps_cleanly() {
    let (mut vm, c) = vm();
    drop(c);
    // passing an integer where the opaque bank is expected is an embedder
    // mistake: a named trap, not a panic or silent zero
    let err = vm.call::<_, String>("sort", (0i64, "quick")).unwrap_err();
    assert!(err.msg.contains("argument"), "{}", err.msg);
}
