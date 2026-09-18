//! The example's gate: drive the rut sorting library from the host side
//! and assert the whole session — every algorithm, edge cases, the JSON
//! round trip, the error path, and trap cleanliness. Runs under
//! `cargo test --workspace`.

use std::rc::Rc;
use rut_vm::heap::Value;

fn vm() -> (rut_vm::interp::Vm, Value) {
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
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).unwrap();
    rut_std::math::install_std_math(&mut vm);
    vm.verify_host_fns(&s.expected_host_fns()); // calc: .d.rut ↔ bodies
    let Value::Opaque(c) = vm.call("create", &[]).unwrap() else { unreachable!() };
    (vm, Value::Opaque(c))
}

const ALGOS: [&str; 5] = ["insertion", "bubble", "selection", "quick", "merge"];

fn s(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("{other:?}"),
    }
}

fn b(v: Value) -> bool {
    match v {
        Value::Bool(b) => b,
        other => panic!("{other:?}"),
    }
}

fn push_all(vm: &mut rut_vm::interp::Vm, c: &Value, xs: &[i64]) {
    for x in xs {
        vm.call("push", &[c.clone(), Value::I64(*x)]).unwrap();
    }
}

fn sort(vm: &mut rut_vm::interp::Vm, c: &Value, algo: &str) -> Value {
    vm.call("sort", &[c.clone(), Value::Str(algo.into())]).unwrap()
}

fn json(xs: &[i64]) -> String {
    format!("[{}]", xs.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "))
}

#[test]
fn every_algorithm_sorts_a_known_input() {
    for algo in ALGOS {
        let (mut vm, c) = vm();
        push_all(&mut vm, &c, &[9, -3, 5, 5, 0, 42, -7, 3]);
        assert_eq!(sort(&mut vm, &c, algo), Value::Str("".into()), "{algo}");
        assert_eq!(
            s(vm.call("serialize", &[c.clone()]).unwrap()),
            "[-7, -3, 0, 3, 5, 5, 9, 42]",
            "{algo}",
        );
        assert!(b(vm.call("is_sorted", &[c.clone()]).unwrap()), "{algo}");
    }
}

#[test]
fn edge_cases_for_every_algorithm() {
    let cases: &[(&str, &[i64])] = &[
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
                s(vm.call("serialize", &[c.clone()]).unwrap()),
                json(&want),
                "{algo} / {name}",
            );
            assert!(b(vm.call("is_sorted", &[c.clone()]).unwrap()), "{algo} / {name}");
        }
    }
}

#[test]
fn all_algorithms_agree_on_pseudo_random_input() {
    // same deterministic fill, every algorithm — one answer
    let mut expected: Option<String> = None;
    for algo in ALGOS {
        let (mut vm, c) = vm();
        vm.call("fill", &[c.clone(), Value::I64(500), Value::I64(7)]).unwrap();
        assert_eq!(vm.call("len", &[c.clone()]).unwrap(), Value::I64(500));
        sort(&mut vm, &c, algo);
        assert!(b(vm.call("is_sorted", &[c.clone()]).unwrap()), "{algo}");
        let got = s(vm.call("serialize", &[c.clone()]).unwrap());
        match &expected {
            None => expected = Some(got),
            Some(want) => assert_eq!(&got, want, "{algo} disagrees with the others"),
        }
    }
}

#[test]
fn large_input_is_monotonic_through_get() {
    let (mut vm, c) = vm();
    vm.call("fill", &[c.clone(), Value::I64(500), Value::I64(1234)]).unwrap();
    sort(&mut vm, &c, "quick");
    let mut prev = i64::MIN;
    for i in 0..500 {
        assert!(b(vm.call("has", &[c.clone(), Value::I64(i)]).unwrap()), "has({i})");
        let Value::I64(x) = vm.call("get", &[c.clone(), Value::I64(i)]).unwrap() else {
            panic!("get({i}) was not an i64");
        };
        assert!(x >= prev, "not monotonic at {i}: {x} < {prev}");
        prev = x;
    }
    // out of range reads 0, and `has` says false — no trap
    assert_eq!(vm.call("get", &[c.clone(), Value::I64(500)]).unwrap(), Value::I64(0));
    assert_eq!(vm.call("has", &[c.clone(), Value::I64(500)]).unwrap(), Value::Bool(false));
    assert_eq!(vm.call("has", &[c.clone(), Value::I64(-1)]).unwrap(), Value::Bool(false));
}

#[test]
fn unknown_algorithm_is_an_error_string() {
    let (mut vm, c) = vm();
    push_all(&mut vm, &c, &[1]);
    assert_eq!(
        sort(&mut vm, &c, "bogus"),
        Value::Str("unknown algorithm: bogus".into()),
    );
    // a failed dispatch leaves the data untouched
    assert_eq!(s(vm.call("serialize", &[c.clone()]).unwrap()), "[1]");
}

#[test]
fn wrong_container_traps_cleanly() {
    let (mut vm, c) = vm();
    drop(c);
    // passing an integer where the Opaque bank is expected is an embedder
    // mistake: a named trap, not a panic or silent zero
    let err = vm.call("sort", &[Value::I64(0), Value::Str("quick".into())]).unwrap_err();
    assert!(err.msg.contains("argument"), "{}", err.msg);
}
