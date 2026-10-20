//! The `if` ATOM (phase 2): `if (cond) { x } else { y }` as an
//! EXPRESSION — branch selection, the agreement diagnostic, the
//! no-`else` nil, if↔when nesting, captures inside arms, and the
//! while+break law through an arm. Phase 1's statement-tail route (the
//! trailing `if` STATEMENT as a block's value) keeps working beside it —
//! both shapes ride the same tests.

use rut_vm::interp::Vm;

/// one diagnostic, its message
fn one_diag(src: &str) -> String {
    let d = match rut_driver::RutRun::new()
        .pkg(rut_driver::Mounted::source("app", src))
        .entrypoint("app")
        .compile()
    {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    };
    let msgs: Vec<String> = d.diags.iter().map(|d| d.msg.clone()).collect();
    assert_eq!(
        msgs.len(),
        1,
        "expected exactly one diagnostic, got {msgs:?}"
    );
    msgs[0].clone()
}

/// compile + run `main` returning i32
fn run_main(src: &str) -> i32 {
    let compiled = rut_driver::RutRun::new()
        .pkg(rut_driver::Mounted::source("app", src))
        .entrypoint("app")
        .compile()
        .expect("compile");
    assert!(
        compiled.graph.diags.is_empty(),
        "diags: {:?}",
        compiled.graph.diags
    );
    let mut vm = Vm::builder().compiled(compiled).build().expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

// ---- the expression produces: runs ----

#[test]
fn branch_selection_both_ways() {
    let v = run_main(
        r#"
fn pick(c: bool) -> i32 {
    let a = if (c) { 10 } else { 20 };
    a
}

entry fn main() -> i32 {
    pick(true) * 100 + pick(false)
}
"#,
    );
    assert_eq!(v, 1020);
}

#[test]
fn arms_compute_and_scope_like_blocks() {
    // the arm's binding dies before the value lands (the register
    // lifetime law, in expression arms)
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let c = 4 > 3;
    let x = if (c) { let t = 40; t + 2 } else { 0 };
    x
}
"#,
    );
    assert_eq!(v, 42);
}

#[test]
fn else_if_chain_selects_through_the_links() {
    let v = run_main(
        r#"
fn grade(n: i32) -> i32 {
    let g = if (n > 90) { 4 } else if (n > 80) { 3 } else if (n > 70) { 2 } else { 1 };
    g
}

entry fn main() -> i32 {
    grade(95) * 100 + grade(85) * 10 + grade(50)
}
"#,
    );
    assert_eq!(v, 431);
}

#[test]
fn diverging_arm_reads_as_the_other_value() {
    // the `!` coercion: the returning arm contributes nothing, the
    // completing arm's value IS the if's (the EXPRESSION route's shape
    // of phase 1's statement-tail law)
    let v = run_main(
        r#"
fn f(c: bool) -> i32 {
    let x = if (c) { return 10; } else { 20 };
    x
}

entry fn main() -> i32 {
    f(true) + f(false)
}
"#,
    );
    assert_eq!(v, 30);
}

#[test]
fn if_expression_in_call_argument() {
    let v = run_main(
        r#"
fn twice(v: i32) -> i32 { return v + v; }

entry fn main() -> i32 {
    twice(if (2 > 1) { 5 } else { 50 })
}
"#,
    );
    assert_eq!(v, 10);
}

#[test]
fn binary_operand_if_expression() {
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let c = true;
    1 + if (c) { 2 } else { 300 }
}
"#,
    );
    assert_eq!(v, 3);
}

#[test]
fn nested_if_atoms() {
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let a = true;
    let b = false;
    let x = if (a) { let y = if (b) { 1 } else { 2 }; y } else { 3 };
    x * 10
}
"#,
    );
    assert_eq!(v, 20);
}

#[test]
fn if_and_when_nest_both_ways() {
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let c = 4 > 3;
    // if ATOM as the when scrutinee
    let w = when (if (c) { 1 } else { 0 }) {
        1 -> { 10 },
        else -> { 20 },
    };
    // the when EXPRESSION as the if's arm value (the phase-1 shape:
    // the let binds the when's arms' values)
    let i = if (c) { let q = when (2) { 2 -> { 30 }, else -> { 40 }, }; q } else { 50 };
    w + i
}
"#,
    );
    assert_eq!(v, 40);
}

#[test]
fn both_routes_in_one_program() {
    // phase 1's statement-tail if (fact) beside phase 2's expression
    // if (the label) — the routes stay independent
    let v = run_main(
        r#"
fn fact(n: i32) -> i32 {
    if (n <= 1) { 1 } else { n * fact(n - 1) }
}

entry fn main() -> i32 {
    let label = if (fact(3) == 6) { 100 } else { 200 };
    fact(5) + label
}
"#,
    );
    assert_eq!(v, 220);
}

#[test]
fn statement_if_with_trailing_semicolon_discards() {
    // the `;` decides: the statement form + `;` — the arms' values (if
    // any) are nobody's; the ASSIGNMENTS are the effect
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let mut t = 1;
    if (3 > 2) { t = 10; } else { t = 20; };
    t
}
"#,
    );
    assert_eq!(v, 10);
}

// ---- the no-else nil ----

#[test]
fn no_else_produces_nil_and_runs_the_arm() {
    // the implicit else arm is the nil value; the then arm's effect
    // (the assignment) is the observable
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let c = 3 > 2;
    let mut t = 0;
    let n = if (c) { t = 5; };
    let d = if (!c) { t = 50; };
    n;
    t
}
"#,
    );
    assert_eq!(v, 5);
}

// ---- capture law through the arms ----

#[test]
fn capture_inside_if_arms() {
    // the plan's `|| if (c) { x } else { 0 }` shape: the closures read
    // through the if ATOM's arms — the capture scan must cross both
    // arms (a missed scan resolves the name nil and the compile fails
    // with a width error); each arm observed once
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let a = 7;
    let b = 0 - 9;
    let g = fn () -> i32 { let y = if (a > 3) { a } else { 0 - a }; y };
    let h = fn () -> i32 { let z = if (b > 3) { b } else { 0 - b }; z };
    g() * 10 + h()
}
"#,
    );
    assert_eq!(v, 79);
}

#[test]
fn captured_and_reassigned_binding_promotes_through_an_arm() {
    // the promotion half: `b` is captured (inside the then arm) AND
    // reassigned AND ref-headed — one shared cell, the closure reads
    // the NEW value
    let v = run_main(
        r#"
class Box2(i32);
impl Box2 { pub fn get(self) -> i32 { return self.inner; } }

entry fn main() -> i32 {
    let mut b = Box2(1);
    let g = fn () -> i32 { let r = if (true) { b } else { Box2(0 - 1) }; r.get() };
    b = Box2(2);
    g()
}
"#,
    );
    assert_eq!(v, 2);
}

#[test]
fn ref_typed_arms_merge_through_the_shared_slot() {
    // both arms produce refs; the merge moves through the shared
    // out-register with the ref repr (MovRef)
    let v = run_main(
        r#"
class Box2(i32);
impl Box2 { pub fn get(self) -> i32 { return self.inner; } }

entry fn main() -> i32 {
    let a = Box2(30);
    let b = Box2(12);
    let c = true;
    let picked = if (c) { a } else { b };
    picked.get()
}
"#,
    );
    assert_eq!(v, 30);
}

// ---- the while+break law crosses the arms ----

#[test]
fn break_inside_an_arm_binds_to_the_loop() {
    // the break lives in the if ATOM's then arm — binds_break must
    // cross it or `while_never_falls_through` would misread the loop
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let mut n = 0;
    while (n < 10) {
        n = n + 1;
        let done = if (n >= 3) { break; } else { 0 };
        done;
    }
    n
}
"#,
    );
    assert_eq!(v, 3);
}

#[test]
fn break_through_an_arm_keeps_the_missing_return_law() {
    // the silent-wrong mode: if the walk misses the break inside the
    // arm, the loop reads as never-falls-through and the missing
    // `return` stops being diagnosed
    let msg = one_diag(
        r#"
fn f() -> i32 {
    while (true) {
        let x = if (true) { break; } else { 3 };
        x;
    }
}

entry fn main() -> i32 {
    f()
}
"#,
    );
    assert!(msg.contains("missing return"), "got: {msg}");
}

// ---- diagnostics ----

#[test]
fn reject_disagreeing_arms() {
    let msg = one_diag(
        r#"
entry fn main() -> i32 {
    let x = if (true) { 1 } else { "s" };
    x
}
"#,
    );
    assert!(msg.contains("`if` arms must agree"), "got: {msg}");
    assert!(msg.contains("`i32` vs `str`"), "got: {msg}");
}

#[test]
fn reject_producing_then_without_else() {
    // no `else` types the whole `nil` — a producing then arm disagrees
    let msg = one_diag(
        r#"
entry fn main() -> i32 {
    let c = true;
    let x = if (c) { 1 };
    x
}
"#,
    );
    assert!(msg.contains("`if` arms must agree"), "got: {msg}");
    assert!(msg.contains("`i32` vs `nil`"), "got: {msg}");
}

#[test]
fn no_else_statement_form_still_needs_the_return() {
    // the plain statement form (statement-leading, no else): unchanged
    // phase-1 law — the body can fall through
    let msg = one_diag(
        r#"
fn f(c: bool) -> i32 {
    if (c) { 1 }
}

entry fn main() -> i32 {
    f(true)
}
"#,
    );
    assert!(msg.contains("missing return"), "got: {msg}");
}

#[test]
fn non_bool_condition_still_hints() {
    // today's law: the condition hint is `bool`, the mismatch diagnoses
    // at the comparison/literal site (hint-only, per the statement form)
    let msg = one_diag(
        r#"
entry fn main() -> i32 {
    let x = if (3) { 1 } else { 2 };
    x;
}
"#,
    );
    assert!(!msg.is_empty(), "an int condition must not pass silently");
}
