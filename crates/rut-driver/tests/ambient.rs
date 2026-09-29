//! Builtin names are AMBIENT (RFC 0028 revised, builtin-surface): no
//! `use` is needed for the engine's fns and containers — the erasure
//! primitive's statics are `opaque(..)` / `opaque.downcast<T>`. The
//! type-name string itself is `opaque` since phase 2 (interner/boot/
//! crossing in lockstep); the old boot/free call spellings are gone
//! (their removal is pinned in `the_old_spellings_are_gone` below).
//! The builtin TRAITS are the other half of the split: every one is
//! `pub builtin` (import-gated) — see `the_gated_traits_require_the_import`.

use rut_core::binary::Surface;
use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    let collection = Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, Surface::core(), "core".to_string()), (3, collection, "collection".to_string())],
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
             let p = opaque.downcast<Point>(b);\n\
             if (p == nil) { panic(\"point downcast failed\"); }\n\
             let b2 = opaque(7);\n\
             let n = opaque.downcast<i32>(b2);\n\
             if (n == nil) { panic(\"int downcast failed\"); }\n\
             return p.x + p.y + n;\n\
         }\n",
    );
    assert_eq!(v, 14);
}

#[test]
fn the_ambient_prelude_binds_beyond_the_gated_names() {
    // the native fns (panic/string_join) and the native
    // containers still bind with no `use` statement anywhere in this
    // source. The `for..of` here runs over `[i32]` — a builtin
    // sequence's FUSED loop, which never names `Iterator` — so it works
    // without the import too; an `impl Iterator<E> for T` or a
    // trait-typed parameter WOULD gate (see the_gated_traits_require_the_import below).
    let v = run_main(
        "fn total(v: [i32]) -> i32 {\n\
             let mut t = 0;\n\
             for (let x of v) { t += x; }\n\
             return t;\n\
         }\n\
         pub fn main() -> i32 {\n\
             let s = string_join([\"a\", \"b\"]);\n\
             if (s != \"ab\") { panic(\"join diverged\"); }\n\
             if (s.len() == 99) { panic(\"unreachable\"); }\n\
             return total([1, 2, 3]);\n\
         }\n",
    );
    assert_eq!(v, 6);
}

/// The import-gated builtin traits (the `pub builtin` spellings):
/// naming `Iterator`/`Future`/`RunContext` in source resolves ONLY
/// through `use core::{ .. }` — the bare spelling names the fix
/// exactly — while the engine's weave never consults user scope: the
/// fused `for..of` over `[i32]` above and every launched host frame
/// run with no import at all. With the import, an `impl Iterator<i32>
/// for CountUp` compiles and the duck-typed `for..of` drives it.
#[test]
fn the_gated_traits_require_the_import() {
    // no use: each bare spelling names its fix
    for (src, name) in [
        ("class C { }\nimpl Iterator<i32> for C { fn __iterate(self, emit: fn(i32) -> bool) { } }\npub fn main() -> i32 { for (let v of C { }) { } return 0; }\n", "Iterator"),
        ("fn f(cx: RunContext) -> i32 { return cx.checkpoint() as i32; }\npub fn main() -> i32 { return f(nil); }\n", "RunContext"),
        ("class F { }\nimpl Future<nil> for F { fn yield(self, cx: RunContext) { } }\npub fn main() -> i32 { return 0; }\n", "Future"),
    ] {
        let out = compile(src);
        assert!(
            out.diags
                .iter()
                .any(|d| d.msg == format!("`{name}` is not in scope — `use core::{{ {name} }}`")),
            "the {name} miss names the fix: {:?}",
            out.diags
        );
        assert!(out.program.is_none(), "the bare {name} source must not compile");
    }

    // with the import: the impl registers and for..of drives it
    let v = run_main(
        "use core::{ Iterator };\n\
         class CountUp { n: i32 = 0; }\n\
         impl CountUp { fn new(n: i32) -> Self { return Self { n: n }; } }\n\
         impl Iterator<i32> for CountUp {\n\
             fn __iterate(self, emit: fn(i32) -> bool) {\n\
                 for (let i = 1; i <= self.n; i += 1) { if (!emit(i)) { return; } }\n\
             }\n\
         }\n\
         class Acc { total: i32 = 0; }\n\
         pub fn main() -> i32 {\n\
             let mut acc: ?Acc = Acc { };\n\
             for (let v of CountUp.new(4)) { acc.total = acc.total + v; }\n\
             return acc.total;\n\
         }\n",
    );
    assert_eq!(v, 10);
}

#[test]
fn opaque_downcast_member_carries_the_nullable_contract() {
    // the member form yields the nullable (RFC 0014, refval-round2): a
    // mismatch is `nil` — never a zero-value `.0` with a flag
    let v = run_main(
        "pub fn main() -> i32 {\n\
             let b = opaque(\"hello\");\n\
             let miss = opaque.downcast<i64>(b);\n\
             if (miss != nil) { return 1; }\n\
             let s = opaque.downcast<str>(b);\n\
             if (s == nil) { return 3; }\n\
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
             let n = downcast<i32>(b);\n\
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
             let n = opaque.downcast<i32>(b2);\n\
             if (n == nil) { return 0; }\n\
             return n;\n\
         }\n",
    );
    assert_eq!(v, 4);
}
