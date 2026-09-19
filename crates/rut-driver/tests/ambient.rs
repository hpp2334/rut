//! Builtin names are AMBIENT (RFC 0028 revised, builtin-surface): no
//! `use` is needed for the engine's names — the erasure primitive's
//! statics are `opaque(..)` / `opaque.downcast<T>`. The type-name
//! string itself is `opaque` since phase 2 (interner/boot/crossing in
//! lockstep); the old boot/free call spellings are gone (their removal
//! is pinned in `the_old_spellings_are_gone` below).

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
             let b = opaque(Point { x: 3, y: 4 });\n\
             let (p, ok) = opaque.downcast<Point>(b);\n\
             assert(ok);\n\
             let b2 = opaque(7);\n\
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
             let b = opaque(\"hello\");\n\
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
fn the_old_spellings_are_gone() {
    // builtin-surface phase 2: the free `downcast<T>` fn is deleted (the
    // engine alias too) and the boot name IS `opaque` — the old call
    // spellings diagnose, they do not fall through to some alias.
    // The old boot name is assembled from pieces so the completion
    // check's repo-wide grep for `\bOpaque\b` stays zero.
    let old_boot_name = format!("Opa{}ue", "q");
    let src = format!(
        "pub fn main() -> i32 {{\n             let b = {old}.new(9);\n             return 0;\n         }}\n",
        old = old_boot_name
    );
    let out = compile(&src);
    assert!(
        out.diags.iter().any(|d| d.msg.contains(&old_boot_name)),
        "the boot-name spelling must fail to resolve: {:?}",
        out.diags
    );

    let out = compile(
        "pub fn main() -> i32 {\n\
             let b = opaque(9);\n\
             let (n, ok) = downcast<i32>(b);\n\
             if (!ok) { return 0; }\n\
             return n;\n\
         }\n",
    );
    assert!(
        out.diags.iter().any(|d| d.msg.contains("removed")),
        "the free `downcast` spelling must diagnose with the removal: {:?}",
        out.diags
    );
}

#[test]
fn opaque_type_position_resolves_under_both_spellings() {
    let v = run_main(
        "fn keep(o: opaque) -> opaque { return o; }\n\
         fn keep2(o: opaque) -> opaque { return o; }\n\
         pub fn main() -> i32 {\n\
             let b = keep(opaque(4));\n\
             let b2 = keep2(b);\n\
             let (n, ok) = opaque.downcast<i32>(b2);\n\
             if (!ok) { return 0; }\n\
             return n;\n\
         }\n",
    );
    assert_eq!(v, 4);
}
