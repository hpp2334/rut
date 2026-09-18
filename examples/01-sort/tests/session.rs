//! The example's gate: drive the rut sorting library from the host side
//! and assert the whole session — every algorithm, edge cases, the JSON
//! round trip, the error path, and trap cleanliness. Runs under
//! `cargo test --workspace`.

use std::rc::Rc;
use rut_vm::OpaqueRef;

fn vm() -> (rut_vm::interp::Vm, OpaqueRef) {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/sort.rut")).unwrap();
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std(&mut s);
    rut_driver::mount_dir(
        &mut s,
        &std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/pouch")),
    )
    .expect("mount pouch");
    let out = rut_driver::compile_module_in(&mut s, &src, rut_parser::Mode::Impl, "sort");
    assert!(out.diags.is_empty());
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(5_000_000),
        heap_limit_bytes: Some(8 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::math::install_std_math(&mut hosts);
    hosts.verify_against(&s.expected_host_fns()); // calc: .d.rut ↔ bodies
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap();

    let c: OpaqueRef = vm.call_typed("create", ()).unwrap();
    (vm, c)
}

const ALGOS: [&str; 5] = ["insertion", "bubble", "selection", "quick", "merge"];

fn push_all(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef, xs: &[i32]) {
    for x in xs {
        vm.call_typed::<_, ()>("push", (c.clone(), *x)).unwrap();
    }
}

fn serialize(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef) -> String {
    vm.call_typed("serialize", (c.clone(),)).unwrap()
}

fn is_sorted(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef) -> bool {
    vm.call_typed("is_sorted", (c.clone(),)).unwrap()
}

fn has(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef, i: i32) -> bool {
    vm.call_typed("has", (c.clone(), i)).unwrap()
}

fn get(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef, i: i32) -> i32 {
    vm.call_typed("get", (c.clone(), i)).unwrap()
}

fn sort(vm: &mut rut_vm::interp::Vm, c: &OpaqueRef, algo: &str) -> String {
    vm.call_typed("sort", (c.clone(), algo)).unwrap()
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
        vm.call_typed::<_, ()>("fill", (c.clone(), 500u32, 7u32)).unwrap();
        assert_eq!(vm.call_typed::<_, i32>("len", (c.clone(),)).unwrap(), 500);
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
    vm.call_typed::<_, ()>("fill", (c.clone(), 500u32, 1234u32)).unwrap();
    sort(&mut vm, &c, "quick");
    let mut prev = i32::MIN;
    for i in 0..500 {
        assert!(has(&mut vm, &c, i), "has({i})");
        let x: i32 = get(&mut vm, &c, i);
        assert!(x >= prev, "not monotonic at {i}: {x} < {prev}");
        prev = x;
    }
    // out of range reads 0, and `has` says false — no trap
    assert_eq!(vm.call_typed::<_, i32>("get", (c.clone(), 500i32)).unwrap(), 0);
    assert_eq!(vm.call_typed::<_, bool>("has", (c.clone(), 500i32)).unwrap(), false);
    assert_eq!(vm.call_typed::<_, bool>("has", (c.clone(), -1i32)).unwrap(), false);
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
    // passing an integer where the Opaque bank is expected is an embedder
    // mistake: a named trap, not a panic or silent zero
    let err = vm.call_typed::<_, String>("sort", (0i64, "quick")).unwrap_err();
    assert!(err.msg.contains("argument"), "{}", err.msg);
}
