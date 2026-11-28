//! The postfix non-null assert `e!` — the explicit-deref law's driver
//! pins. PHASE 2: the law FLIPPED — every implicit `?T → T` peel died
//! demanding the assert (ONE helper, ONE message family), so the pins
//! here pin BOTH halves:
//!
//! - the DEMAND half: each killed site errors with the family message
//!   `` `?Foo` derefs only through `!` — spell the assert (`v!`) `` —
//!   the receiver (`f.x()`, `f.x = 4`), the field read, the f-string
//!   hole, the mixed `==`, `is` on an Opt, the value funnel
//!   (`let a: Foo = f`), arithmetic, the `as` cast, the for-of
//!   iterable, the index read;
//! - the FOOTGUN fix: `f! is Foo` probes the payload and answers TRUE
//!   on a real `Foo`; the un-asserted `f is Foo` is the demand error;
//! - the KEEP half (the not-peels, must not regress): the `!= nil`
//!   guard idiom, the `?T == ?T` shared-cell compare, the implicit
//!   box (`T → ?T` cannot fail), `[?T]` element stores, `for x of
//!   [?T]` binding `?T` elements, the asserted receiver call.

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

/// THE FLIPPED DISPATCH PIN (phase 2): the receiver auto-deref is
/// GONE — `f.x()` on a `?Foo` is the demand error; the honest spelling
/// `f!.x()` answers 42. Old expectation (phase 1): `f.x()` resolved
/// through the receiver auto-deref and answered 42.
#[test]
fn the_nullable_receiver_dispatch_demands_the_assert() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    return f.x();\n\
}}\n"
    );
    let diags = compile_diags(&src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?Foo` derefs only through `!`")),
        "{diags:?}"
    );
    // the honest spelling runs
    let ok = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    return f!.x();\n\
}}\n"
    );
    assert_eq!(run_main(&ok), 42);
}

/// THE GUARD PIN (the keep list's documented idiom, §5 of the plan):
/// inside the `!= nil` guard the assert cannot trap — the author
/// spells it, the compiler does not prove it.
#[test]
fn the_nil_guard_idiom_runs() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = maybe(1);\n\
    if (f != nil) {{\n\
        let t = f!;\n\
        return t.x();\n\
    }}\n\
    return 0;\n\
}}\n"
    );
    assert_eq!(run_main(&src), 42);
}

/// THE MIXED-COMPARE PIN: `f == t` (?Foo vs Foo) is the demand error
/// at the lhs — the author spells `f! == t`. The `== nil` null-slot
/// compare stays green beside it (the guard pin above).
#[test]
fn the_mixed_compare_demands_the_assert() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    let t = Foo::mk();\n\
    if (f == t) {{ return 1; }}\n\
    return 0;\n\
}}\n"
    );
    let diags = compile_diags(&src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?Foo` derefs only through `!`")),
        "{diags:?}"
    );
}

/// THE SHARED-CELL PIN (the keep list): `?T == ?T` compares the
/// optional cells — aliased bindings compare true, no assert asked.
#[test]
fn the_nullable_compare_stays_green() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    let other = f;\n\
    if (f == other) {{ return 1; }}\n\
    return 0;\n\
}}\n"
    );
    assert_eq!(run_main(&src), 1);
}

/// THE F-STRING PIN: the hole never peels — `f"{{f}}"` on a `?i32` is
/// the demand error (the payload's assert is the author's: `f"{{f!}}"`
/// formats and a nil traps at the hole's own `!`).
#[test]
fn the_fstring_hole_demands_the_assert() {
    let src = "entry fn main() -> i32 {\n\
               let f: ?i32 = 7;\n\
               let s = f\"{f}\";\n\
               return 0;\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?i32` derefs only through `!`")),
        "{diags:?}"
    );
}

/// THE `is` FOOTGUN PIN: `f! is Foo` probes the payload cell and
/// answers TRUE on a real `Foo` (the compile-time exact fold — the
/// static type of `f!` IS `Foo`); the un-asserted `f is Foo` is the
/// demand error (it used to answer silently FALSE even on non-nil).
#[test]
fn the_asserted_is_answers_true_and_the_bare_is_demands() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    if (f! is Foo) {{ return 1; }}\n\
    return 0;\n\
}}\n"
    );
    assert_eq!(run_main(&src), 1);

    let bare = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    if (f is Foo) {{ return 1; }}\n\
    return 0;\n\
}}\n"
    );
    let diags = compile_diags(&bare);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?Foo` derefs only through `!`")),
        "{diags:?}"
    );
}

/// THE VALUE-FUNNEL PIN: `let a: Foo = f` is the demand error — the
/// payload's assert is the author's (`let a: Foo = f!`); on nil the
/// trap lands AT the assert, not statements away.
#[test]
fn the_value_position_demands_the_assert() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    let a: Foo = f;\n\
    return a.x();\n\
}}\n"
    );
    let diags = compile_diags(&src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?Foo` derefs only through `!`")),
        "{diags:?}"
    );

    let ok = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = Foo::mk();\n\
    let a: Foo = f!;\n\
    return a.x();\n\
}}\n"
    );
    assert_eq!(run_main(&ok), 42);

    // nil: the trap is AT the `!`'s own line (the author's mark)
    let nil = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let f: ?Foo = maybe(0);\n\
    let a: Foo = f!;\n\
    return a.x();\n\
}}\n"
    );
    assert_eq!(run_trap(&nil), Some("NilDeref".into()));
}

/// THE FIELD-WRITE PIN: `f.x = 4` through a nullable head is the
/// demand error; `f!.x = 4` writes the shared record.
#[test]
fn the_field_write_demands_the_assert() {
    let src = "class P { pub x: i32; }\n\
               entry fn main() -> i32 {\n\
               let f: ?P = P { x: 0 };\n\
               f.x = 4;\n\
               return 0;\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?P` derefs only through `!`")),
        "{diags:?}"
    );

    let ok = "class P { pub x: i32; }\n\
             entry fn main() -> i32 {\n\
             let f: ?P = P { x: 0 };\n\
             f!.x = 4;\n\
             return f!.x;\n\
             }\n";
    assert_eq!(run_main(ok), 4);
}

/// THE FIELD-READ PIN: `f.x` through a nullable receiver is the
/// demand error (`f!.x` reads the payload).
#[test]
fn the_field_read_demands_the_assert() {
    let src = "class P { pub x: i32; }\n\
               entry fn main() -> i32 {\n\
               let f: ?P = P { x: 7 };\n\
               return f.x;\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?P` derefs only through `!`")),
        "{diags:?}"
    );
}

/// THE ARITHMETIC PIN: `m + 1` on `m: ?i32` is the demand error
/// (`m! + 1` computes at the payload).
#[test]
fn arithmetic_demands_the_assert() {
    let src = "entry fn main() -> i32 {\n\
               let m: ?i32 = 7;\n\
               return m + 1;\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?i32` derefs only through `!`")),
        "{diags:?}"
    );

    let ok = "entry fn main() -> i32 {\n\
              let m: ?i32 = 7;\n\
              return m! + 1;\n\
              }\n";
    assert_eq!(run_main(ok), 8);
}

/// THE CAST PIN: the `as` operand never peels — `m as i64` on
/// `m: ?i32` is the demand error (`m! as i64` casts the payload).
#[test]
fn the_cast_operand_demands_the_assert() {
    let src = "entry fn main() -> i64 {\n\
               let m: ?i32 = 7;\n\
               return m as i64;\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`?i32` derefs only through `!`")),
        "{diags:?}"
    );

    let ok = "entry fn main() -> i64 {\n\
              let m: ?i32 = 7;\n\
              return m! as i64;\n\
              }\n";
    let out = compiled("app_main", ok);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

/// THE FOR-OF PIN: the iterable never peels — `for (let x of f)` on a
/// `?[i32]` is the demand error; `for (let x of f!)` iterates the
/// payload. A `[?i32]` array's elements bind `?i32` — the load
/// yields the optional, no peel (the keep half beside it).
#[test]
fn the_for_of_iterable_demands_the_assert() {
    let src = "entry fn main() -> i32 {\n\
               let f: ?[i32] = [1, 2, 3];\n\
               let sum = 0;\n\
               for (let x of f) { sum += x; }\n\
               return sum;\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("derefs only through `!`") && d.contains("?Array<i32>")),
        "{diags:?}"
    );

    let ok = "entry fn main() -> i32 {\n\
              let f: ?[i32] = [1, 2, 3];\n\
              let sum = 0;\n\
              for (let x of f!) { sum += x; }\n\
              return sum;\n\
              }\n";
    assert_eq!(run_main(ok), 6);

    // the mirror shape: `[?i32]` elements load as `?i32` — the guard
    // + assert idiom is the honest read (the keep list's last bullet)
    let elems = "entry fn main() -> i32 {\n\
                 let xs: [?i32] = [nil; 3];\n\
                 xs[0] = 5;\n\
                 xs[1] = 7;\n\
                 let sum = 0;\n\
                 for (let e of xs) {\n\
                     if (e != nil) { sum += e!; }\n\
                 }\n\
                 return sum;\n\
                 }\n";
    assert_eq!(run_main(elems), 12);
}

/// THE INDEX PIN: the indexee never peels — `t[0]` on `?[i32]` is the
/// demand error; `t![0]` reads the payload (and `t![0] = 9` writes it).
#[test]
fn the_indexee_demands_the_assert() {
    let src = "entry fn main() -> i32 {\n\
               let t: ?[i32] = [1, 2, 3];\n\
               return t[0];\n\
               }\n";
    let diags = compile_diags(src);
    assert!(
        diags
            .iter()
            .any(|d| d.contains("derefs only through `!`") && d.contains("?Array<i32>")),
        "{diags:?}"
    );

    let ok = "entry fn main() -> i32 {\n\
              let t: ?[i32] = [1, 2, 3];\n\
              t![0] = 9;\n\
              return t![0];\n\
              }\n";
    assert_eq!(run_main(ok), 9);
}

/// THE BOXING PIN (the keep list): `T → ?T` stays implicit — it
/// cannot fail, no spelling asked, and the nil-slot compare beside it
/// stays the guard.
#[test]
fn boxing_stays_implicit() {
    let src = format!(
        "{HEAD}\n\
entry fn main() -> i32 {{\n\
    let s: ?Foo = Foo::mk();\n\
    if (s == nil) {{ return 0; }}\n\
    return 1;\n\
}}\n"
    );
    assert_eq!(run_main(&src), 1);
}
