//! Builtin names are AMBIENT (RFC 0028 revised, builtin-surface phase 1):
//! no `use` is needed for the engine's names — the erasure primitive's
//! statics resolve under the boot spelling (`Opaque.new`, free
//! `downcast<T>`) AND the new surface spelling (`opaque.new`,
//! `opaque.downcast<T>`). The old `use core::{ .. }` imports stay legal
//! (redundant, not an error) until the phase-2 sweep deletes them.

use rut_core::binary::Surface;
use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    let collection = Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, Surface::core()), (3, collection)],
    )
}

/// Compile, flatten (RFC 0035 §1), verify, and run a single-module
/// `main` returning i32.
fn run_main(src: &str) -> i32 {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

#[test]
fn builtins_resolve_with_no_use_statement() {
    // every name here is a core builtin — none is imported
    let v = run_main(
        "struct Point { x: i32; y: i32 }\n\
         pub fn main() -> i32 {\n\
             let b = Opaque.new(Point { x: 3, y: 4 });\n\
             let (p, ok) = downcast<Point>(b);\n\
             assert(ok);\n\
             let b2 = opaque.new(7);\n\
             let (n, ok2) = opaque.downcast<i32>(b2);\n\
             assert(ok2);\n\
             return p.x + p.y + n;\n\
         }\n",
    );
    assert_eq!(v, 14);
}

#[test]
fn opaque_downcast_member_carries_the_tuple_contract() {
    // the member form keeps the free fn's `(T, bool)` semantics: a false
    // `.1` leaves `.0` at the type's zero value (RFC 0014)
    let v = run_main(
        "pub fn main() -> i32 {\n\
             let b = opaque.new(\"hello\");\n\
             let (miss, ok) = opaque.downcast<i64>(b);\n\
             if (ok) { return 1; }\n\
             if (miss != 0) { return 2; }\n\
             let (s, ok2) = opaque.downcast<str>(b);\n\
             if (!ok2) { return 3; }\n\
             return s.len() as i32;\n\
         }\n",
    );
    assert_eq!(v, 5);
}

#[test]
fn the_old_use_imports_stay_legal() {
    // phase-1 grace: `use core::{ Opaque, downcast }` is redundant now,
    // not an error — the phase-2 sweep deletes the imports
    let v = run_main(
        "use core::{ Opaque, downcast };\n\
         pub fn main() -> i32 {\n\
             let b = Opaque.new(9);\n\
             let (n, ok) = downcast<i32>(b);\n\
             if (!ok) { return 0; }\n\
             return n;\n\
         }\n",
    );
    assert_eq!(v, 9);
}

#[test]
fn opaque_type_position_resolves_under_both_spellings() {
    let v = run_main(
        "fn keep(o: Opaque) -> Opaque { return o; }\n\
         fn keep2(o: opaque) -> opaque { return o; }\n\
         pub fn main() -> i32 {\n\
             let b = keep(opaque.new(4));\n\
             let b2 = keep2(b);\n\
             let (n, ok) = opaque.downcast<i32>(b2);\n\
             if (!ok) { return 0; }\n\
             return n;\n\
         }\n",
    );
    assert_eq!(v, 4);
}
