//! The typed host boundary: `Vm::call` in Rust
//! types — `Value`/`Slot` never leave the crate. Mirrors e2e's
//! entry-driven shape but through the typed API.

/// The toolchain libs a single-file case declares by use (`ink`,
/// `pouch`) — walked from the tree as real packages.
fn case_world() -> Vec<rut_driver::Pkg> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."); // repo root
    let mut world = Vec::new();
    for d in ["rut/ink", "rut/pouch"] {
        world.extend(
            rut_native::dir_pkgs(&root.join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                .pkgs,
        );
    }
    world.push(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"));
    world
}

fn compile(src: &str, module: &str) -> rut_driver::Compiled {
    let combined = format!("{src}\nuse ink::{{Logger}};\n");
    match rut_driver::RutRun::new()
        .pkgs(&rut_driver::Loaded { pkgs: case_world(), root: String::new() })
        .pkg(rut_driver::Pkg::source(module, &combined))
        .host_pkg(rut_std::logger::pkg(|_msg| {}))
        .host_pkg(rut_std::math::pkg())
        .entrypoint(module)
        .compile()
    {
        Ok(c) => c,
        Err(e) => rut_driver::Compiled {
            graph: rut_driver::GraphOutput {
                diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
                program: None,
            },
            hosts: rut_vm::interp::HostRegistry::new(),
        },
    }
}

fn entry_vm(src: &str) -> rut_vm::interp::Vm {
    let out = compile(src, "m");
    assert!(
        out.graph.diags.is_empty(),
        "unexpected diags:\n{}",
        rut_lexer::diag::render_diags(&format!("{src}\nuse ink::{{Logger}};\n"), &out.graph.diags)
    );
    rut_vm::verify::verify(out.graph.program.as_ref().expect("program")).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // the bindings, BEFORE the Vm — offer = declare, the install rides
    // the chain's registry
    let vm = rut_vm::interp::Vm::builder()
        .compiled(out)
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .build()
        .expect("vm");
    vm
}

const SRC: &str = r#"
use pouch::{ Vec };
struct Row { id: i32; }
struct Box { rows: Vec<Row>; }

entry fn num() -> i64 { return 42; }
entry fn half() -> f64 { return 1.5; }
entry fn flag() -> bool { return true; }
entry fn greet() -> str { return "hey"; }
entry fn echo_bytes(v: bytes) -> bytes { return v; }
entry fn pair(v: i32) -> (i32, bool) {
    return when (v > 0) { true -> (v, true), else -> (0, false) };
}
entry fn triple() -> (i64, f64, str) { let a: i64 = 1; let b: f64 = 2.5; return (a, b, "three"); }
entry fn nothing() -> nil { return; }
entry fn make() -> opaque { let b: ?Box = Box { rows: Vec.new() }; return opaque(b); }
entry fn put(c: opaque) -> u32 {
    let b = opaque.downcast<?Box>(c);
    b.rows.push(Row { id: 1 });
    return b.rows.len() as u32;
}
"#;

#[test]
fn typed_prims_and_tuples() {
    let mut vm = entry_vm(SRC);
    assert_eq!(vm.call::<_, i64>("num", ()).unwrap(), 42);
    assert_eq!(vm.call::<_, f64>("half", ()).unwrap(), 1.5);
    assert_eq!(vm.call::<_, bool>("flag", ()).unwrap(), true);
    assert_eq!(vm.call::<_, ()>("nothing", ()).unwrap(), ());
    assert_eq!(vm.call::<_, (i32, bool)>("pair", (7i32,)).unwrap(), (7, true));
    assert_eq!(vm.call::<_, (i32, bool)>("pair", (-1i32,)).unwrap(), (0, false));
    let (a, b, c) = vm.call::<_, (i64, f64, String)>("triple", ()).unwrap();
    assert_eq!((a, b, c.as_str()), (1, 2.5, "three"));
}

#[test]
fn typed_str_and_bytes() {
    let mut vm = entry_vm(SRC);
    // an owned copy — the explicit "I keep this data" shape
    assert_eq!(vm.call::<_, String>("greet", ()).unwrap(), "hey");
    let back = vm
        .call::<_, Vec<u8>>("echo_bytes", (vec![1u8, 2, 250],))
        .unwrap();
    assert_eq!(back, vec![1, 2, 250]);
}

#[test]
fn typed_opaque_roundtrip() {
    let mut vm = entry_vm(SRC);
    let c: rut_vm::OpaqueRef = vm.call::<_, rut_vm::OpaqueRef>("make", ()).unwrap();
    let n: u32 = vm.call("put", (c.clone(),)).unwrap();
    assert_eq!(n, 1);
    let n: u32 = vm.call("put", (c,)).unwrap();
    assert_eq!(n, 2); // the same box, mutated through the boundary
}

#[test]
fn typed_wrong_shape_is_a_named_trap() {
    let mut vm = entry_vm(SRC);
    // rut says `str`, the embedder asked for i64 — the trap names both
    let err = vm.call::<_, i64>("greet", ()).unwrap_err();
    assert!(err.msg.contains("boundary:"), "{}", err.msg);
    assert!(err.msg.contains("`str`") && err.msg.contains("`i64`"), "{}", err.msg);
    // arity: the raw call's check still guards the typed wrapper
    let err = vm.call::<_, (i32, bool)>("pair", ()).unwrap_err();
    assert!(err.msg.contains("args"), "{}", err.msg);
}

// ---- the entry-err shape (err-channel phase 3): `(?T, err)` crosses
// under the ORIGINAL entry rule — the nullable's element answers it, no
// err carve-out — and decodes NIL-FLATTENED: the null slot is `None` /
// `Value::Nil`, a some-slot is the payload. The exactly-one-non-nil
// channel convention is the CALLER's, documented, not the boundary's. ----

const OPT_ERR: &str = r#"
entry fn find(b: bool) -> (?i64, str) {
    if (b) { return (7, ""); }
    return (nil, "not found");
}
entry fn give_str(b: bool) -> (?str, str) {
    if (b) { return ("payload", ""); }
    return (nil, "boom");
}
"#;

#[test]
fn opt_err_pairs_cross_and_decode_nil_flattened() {
    let mut vm = entry_vm(OPT_ERR);
    // the value channel live: (payload, "") — exactly-one-non-nil
    let (v, e): (Option<i64>, String) = vm.call("find", (true,)).unwrap();
    assert_eq!((v, e.as_str()), (Some(7), ""));
    // the err channel live: (nil, why) — the value component is nil and
    // reads back as `None`, rut-nil and VM-nil indistinguishable
    let (v, e): (Option<i64>, String) = vm.call("find", (false,)).unwrap();
    assert_eq!((v, e.as_str()), (None, "not found"));
    // a ref-repr element (`?str`) boxes and unboxes the same way
    let (s, e): (Option<String>, String) = vm.call("give_str", (true,)).unwrap();
    assert_eq!((s.as_deref(), e.as_str()), (Some("payload"), ""));
    let (s, e): (Option<String>, String) = vm.call("give_str", (false,)).unwrap();
    assert_eq!((s.as_deref(), e.as_str()), (None, "boom"));
}

#[test]
fn the_driver_result_decodes_positionally() {
    let mut vm = entry_vm(OPT_ERR);
    // `Ret for Value`: the whole crossing as one tagged value — the raw
    // shape envelope-style decoders read, `?T` already flattened
    let v: rut_vm::Value = vm.call("find", (false,)).unwrap();
    match v {
        rut_vm::Value::Tuple(parts) => {
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0], rut_vm::Value::Nil);
            assert_eq!(parts[1], rut_vm::Value::Str("not found".into()));
        }
        other => panic!("expected the pair shape, got {other:?}"),
    }
}

#[test]
fn a_non_crossable_nullable_still_rejects() {
    // no carve-out: `?Bag` fails exactly as `Bag` does — the nullable's
    // ELEMENT answers the crossing rule (Bag is a class: a cell type,
    // never a crossing currency)
    let src = "class Bag { f: fn(i64) -> i64; }\nentry fn bad() -> (?Bag, str) { return (nil, \"x\"); }\n";
    let out = compile(src, "m");
    assert!(
        out.graph.diags.iter().any(|d| d.msg.contains("only primitives")),
        "expected the crossing diagnostic, got {:?}",
        out.graph.diags.iter().map(|d| &d.msg).collect::<Vec<_>>()
    );
}

// ---- host-fn payload boxes through the typed boundary ----

#[test]
fn typed_host_box_roundtrip() {
    // the host boxes a Rust payload, hands it in as `opaque`, reads the
    // SAME box back — the payload never crosses as data
    let src = r#"
entry fn ident(o: opaque) -> opaque { return o; }
"#;
    let mut vm = entry_vm(src);
    let boxed = rut_vm::Opaque::alloc(&mut vm, vec!["a".to_string(), "b".to_string()])
        .expect("alloc host box");
    let back: rut_vm::Opaque<Vec<String>> =
        vm.call("ident", (boxed,)).expect("typed crossing");
    assert_eq!(back.with(|v| v.clone()).unwrap(), vec!["a".to_string(), "b".to_string()]);
    // the payload type token is checked, never UB
    let wrong: Result<rut_vm::Opaque<u64>, _> = vm.call("ident", (back,));
    assert!(wrong.is_err(), "a `Vec<String>` box must not read as `u64`");
}
