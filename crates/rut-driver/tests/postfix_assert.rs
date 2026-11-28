//! The postfix non-null assert `e!` — the explicit-deref law's driver
//! pins. The `!` lands ADDITIVE (phase 1): all implicit peels stay
//! legal, nothing demands the `!` yet. The pins:
//!
//! - green: `let t = f!; t.x()` on a non-nil `?Foo` runs to 42 — the
//!   assert peels ONE `?` layer, the plain receiver needs nothing;
//! - nil: `f!` on nil traps `NilDeref` AT the assert — main's one
//!   payload peel is the assert's own `GetF { field: 0 }`, emitted at
//!   the `f!` form's own span;
//! - `!` on a non-nullable diagnoses (`` `!` asserts non-nil — `i32`
//!   has no `?` to peel ``);
//! - `f! = v` is refused by the assign-target validation family (an
//!   `Assert` node is not an assignable target);
//! - the exact-nullable receiver dispatch is UNAFFECTED: `f.x()` on a
//!   non-nil `?Foo` still resolves through the receiver auto-deref
//!   (`lir/call/method.rs`'s receiver law — phase 1 touches nothing
//!   beside it).

use std::rc::Rc;

/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .mount(rut_driver::Mounted::source(spec, src))
            .entrypoint(spec)
            .compile(),
    )
}

fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    }
}

const HEAD: &str = "\
class Foo {}

impl Foo {
    fn mk() -> Foo { return Foo {}; }
    fn x(self) -> i32 { return 42; }
}

fn maybe(v: i32) -> ?Foo {
    if (v > 0) { return Foo::mk(); }
    return nil;
}
";

/// Compile, verify, and run `main` — the i32 return.
fn run_main(src: &str) -> i32 {
    let out = compiled("app_main", src);
    assert!(
        out.diags.is_empty(),
        "{}",
        out.diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp::Vm::builder()
        .program(Rc::new(flat))
        .limits(limits)
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm")
        .call::<_, i32>("main", ())
        .expect("run")
}

/// Run `main`, answering the trap name (None = clean run).
fn run_trap(src: &str) -> Option<String> {
    let out = compiled("app_main", src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(flat))
        .limits(limits)
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    match vm.call::<_, i32>("main", ()) {
        Ok(_) => None,
        Err(t) => Some(t.name()),
    }
}

/// Compile only — the rendered diagnostics (negative pins).
fn compile_diags(src: &str) -> Vec<String> {
    let out = compiled("app_main", src);
    out.diags.iter().map(|d| format!("{d:?}")).collect()
}

/// THE GREEN PIN: the assert peels one `?` layer, then the plain
/// receiver runs — `let t = f!; t.x()` answers 42, no other peel in
/// sight.
#[test]
fn the_assert_peels_one_layer_and_the_plain_receiver_runs() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = maybe(1);\n\
    let t = f!;\n\
    return t.x();\n\
}}\n"
    );
    assert_eq!(run_main(&src), 42);
}

/// THE NIL PIN: `f!` on nil traps `NilDeref` AT the assert — and the
/// IR proves the site: main's ONLY `GetF { field: 0 }` is the assert's
/// own emission, and its span row names the `f!` form's own bytes.
#[test]
fn nil_traps_nilderef_at_the_asserts_own_site() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = maybe(0);\n\
    let t = f!;\n\
    return t.x();\n\
}}\n"
    );
    assert_eq!(run_trap(&src), Some("NilDeref".into()));

    // the site: exactly one payload peel in main, at the `f!` form
    let out = compiled("app_main", &src);
    let prog = out.program.expect("linked program");
    let main_fn = prog
        .funcs
        .iter()
        .find(|f| prog.interner.name(f.name) == "main")
        .expect("main in the program");
    let mut getfs: Vec<u32> = Vec::new();
    for (pc, op) in main_fn.code.iter().enumerate() {
        if let rut_core::ops::Op::GetF { field: 0, .. } = op {
            getfs.push(main_fn.spans[pc].1);
        }
    }
    assert_eq!(getfs.len(), 1, "the assert is the ONE payload peel");
    let form = src.find("f!").expect("the `f!` form") as u32;
    assert_eq!(getfs[0], form, "the peel emits at the assert's own span");
}

/// THE NON-NULLABLE PIN: `!` on a plain `i32` has no `?` to peel — the
/// plan's message family, one diagnostic.
#[test]
fn the_assert_on_a_non_nullable_diagnoses() {
    let src = "entry fn main() -> i32 {\n\
               let x = 5;\n\
               let y = x!;\n\
               return y;\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`!` asserts non-nil") && d.contains("has no `?` to peel")),
        "{diags:?}"
    );
}

/// THE ASSIGN-TARGET PIN: `f! = v` is refused — an `Assert` node is
/// not an assignable target (the validation family's law).
#[test]
fn an_assert_is_not_an_assign_target() {
    let src = "entry fn main() -> i32 {\n\
               let f: ?i32 = nil;\n\
               let v = 3;\n\
               f! = v;\n\
               return 0;\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("invalid assignment target")),
        "{diags:?}"
    );
}

/// THE DISPATCH GUARD: the exact-nullable receiver law is UNAFFECTED —
/// `f.x()` on a non-nil `?Foo` still resolves through the receiver
/// auto-deref and answers 42 (phase 1 adds the assert beside it; the
/// implicit receiver peel stays legal until the law flips).
#[test]
fn the_nullable_receiver_dispatch_is_unaffected() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    return f.x();\n\
}}\n"
    );
    assert_eq!(run_main(&src), 42);
}
