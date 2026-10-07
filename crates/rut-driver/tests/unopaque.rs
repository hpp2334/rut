//! `unopaque<T>(b) -> T` — the erasure box's inverse: every kind's
//! round-trip (`opaque(x)` → `unopaque<T>` → x), the alias law through
//! the recovery, and the mismatch discipline (a compile-time refusal
//! where the seal law already knows — interface objects are never
//! boxed; the `BadUnbox` trap naming both types everywhere else).

/// Compile, flatten, verify, and run a single-module `main` returning i32.
use rut_parser::Mode;

fn run_main(src: &str) -> i32 {
    let out = rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(prog))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

/// Compile and return the diagnostics (the refusal lane).
fn diags_of(src: &str) -> Vec<String> {
    let out = rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
    );
    out.diags.iter().map(|d| d.msg.clone()).collect()
}

#[test]
fn every_kind_round_trips_through_the_box() {
    // i32, f64, bool, str, bytes, a record, a tuple — box, recover,
    // compare: the recovered value IS the sealed one
    let src = "\
struct Point { x: i32; y: i32; }
entry fn main() -> i32 {
    let bi = opaque(7);
    let vi: i32 = unopaque<i32>(bi);
    let bf = opaque(2.5f64);
    let vf: f64 = unopaque<f64>(bf);
    let bb = opaque(true);
    let vb: bool = unopaque<bool>(bb);
    let bs = opaque(\"hi\");
    let vs: str = unopaque<str>(bs);
    let bz = opaque(bytes.from([1u8, 2u8, 3u8]));
    let vz: bytes = unopaque<bytes>(bz);
    let bp = opaque(Point { x: 3, y: 4 });
    let vp: Point = unopaque<Point>(bp);
    let bt = opaque((1, 2));
    let vt: (i32, i32) = unopaque<(i32, i32)>(bt);
    let acc: i32 = vi + (vf as i32) + (vb as i32) + (vs == \"hi\") as i32
        + (vz.len() as i32) + vp.x + vp.y + (vt.0 + vt.1);
    return acc;
}
";
    assert_eq!(run_main(src), 7 + 2 + 1 + 1 + 3 + 3 + 4 + 3);
}

#[test]
fn a_record_recovery_shares_the_source_cell() {
    // the alias law through the inverse: the recovery writes through
    // to the source (one cell, everywhere), like downcast's
    let src = "\
struct Point { x: i32; y: i32; }
entry fn main() -> i32 {
    let mut p = Point { x: 1, y: 2 };
    let b = opaque(p);
    let mut back: Point = unopaque<Point>(b);
    back.x = 99;
    return p.x;
}
";
    assert_eq!(run_main(src), 99);
}

#[test]
fn a_mismatch_is_the_bad_unbox_trap() {
    // the erased content is a runtime fact: the box holds i32, f64 is
    // recovered — the trap names both types
    let src = "\
entry fn main() -> i32 {
    let b = opaque(7);
    let v: f64 = unopaque<f64>(b);
    return v as i32;
}
";
    let out = rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
    );
    assert!(out.diags.is_empty(), "the mismatch is a runtime fact: {:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(prog))
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    let err = vm.call::<_, i32>("main", ()).expect_err("the mismatch traps");
    let msg = format!("{err:?}");
    assert!(msg.contains("BadUnbox"), "{msg}");
    assert!(msg.contains("i32") && msg.contains("f64"), "the trap names both types: {msg}");
}

#[test]
fn an_interface_type_argument_is_a_compile_error() {
    // knowable at compile time: interface objects are never boxed (the
    // seal law refuses them), so the recovery can never match — the
    // ONE sealed Future<T> exception rides the async lane
    let src = "\
interface Shape { fn area(self) -> f64; }
struct Sq { s: f64 }
impl Sq { pub fn area(self) -> f64 { return self.s * self.s; } }
entry fn main() -> i32 {
    let b = opaque(5);
    let s: Shape = unopaque<Shape>(b);
    return 0;
}
";
    let ds = diags_of(src);
    assert!(
        ds.iter().any(|d| d.contains("unopaque") && d.contains("CONCRETE")),
        "the seal law refuses the type argument: {ds:?}"
    );
}

#[test]
fn unopaque_takes_an_opaque_box_only() {
    // a non-box operand diagnoses — the erasure is a spelled value
    let ds = diags_of(
        "entry fn main() -> i32 {\n    let v: i32 = unopaque<i32>(7);\n    return v;\n}\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("unopaque takes an `opaque` box")),
        "{ds:?}"
    );
}
