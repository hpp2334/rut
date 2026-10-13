//! Block tail expressions (phase 1): the last statement without `;`
//! is the block's implicit value — Rust-style — and a trailing `if`
//! STATEMENT with an else reads as the tail too (statement-if in tail
//! position; the if ATOM in expression position is phase 2). The fn
//! body's tail counts as the return for the definite-return law, the
//! `when` arm blocks produce values, and the capture law sees closures
//! inside tails. Pins: the run shapes (implicit return, recursion,
//! scoped-binding tails, inlined tails, capture-on-tail) and the
//! diagnostics (tail type mismatch, arm disagreement, missing return
//! with a valueless tail).

use rut_vm::interp::Vm;

fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    match rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile() {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

/// one diagnostic, its message
fn one_diag(src: &str) -> String {
    let d = compiled("app", src);
    let msgs: Vec<String> = d.diags.iter().map(|d| d.msg.clone()).collect();
    assert_eq!(msgs.len(), 1, "expected exactly one diagnostic, got {msgs:?}");
    msgs[0].clone()
}

/// compile + run `main` returning i32
fn run_main(src: &str) -> i32 {
    let compiled = rut_driver::RutRun::new()
        .pkg(rut_driver::Pkg::source("app", src))
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

// ---- the tail is the value: runs ----

#[test]
fn implicit_return_fn_body_tail() {
    // 1 — the ground shape: no `return` spelled, the tail answers
    let v = run_main(
        r#"
fn five() -> i32 {
    5
}

entry fn main() -> i32 {
    five()
}
"#,
    );
    assert_eq!(v, 5);
}

#[test]
fn recursion_through_tail_position_if() {
    // 2 — the plan's `fact`: the trailing `if` STATEMENT is the body's
    // value (both arms produce); the if ATOM stays phase 2
    let v = run_main(
        r#"
fn fact(n: i32) -> i32 {
    if (n <= 1) { 1 } else { n * fact(n - 1) }
}

entry fn main() -> i32 {
    fact(6)
}
"#,
    );
    assert_eq!(v, 720);
}

#[test]
fn tail_reads_block_local_bindings() {
    // 3 — the register-lifetime law: the tail reads bindings the block
    // owns; the value moves out before the scope pops (both a computed
    // tail and a bare-path tail)
    let v = run_main(
        r#"
fn sum() -> i32 {
    let a = 20;
    let b = 22;
    a + b
}

fn bare() -> i32 {
    let x = 7;
    x
}

entry fn main() -> i32 {
    sum() + bare()
}
"#,
    );
    assert_eq!(v, 49);
}

#[test]
fn ref_typed_tail_moves_out_of_scope() {
    // 4 — a ref-repr tail binds out of the dying scope (MovRef repr)
    run_main(
        r#"
class Box2(i32);
impl Box2 { pub fn get(self) -> i32 { return self.inner; } }

fn make() -> Box2 {
    let b = Box2(40);
    b
}

entry fn main() -> i32 {
    make().get() + 2
}
"#,
    );
}

#[test]
fn tail_and_return_mix() {
    // 5 — `return` inside a nested arm, the tail answering the rest —
    // the definite-return OR-composition across while/for contexts
    let v = run_main(
        r#"
fn find(v: [i32], want: i32) -> i32 {
    for (let x of v) {
        if (x == want) { return x; }
    }
    0 - 1
}

fn mixed(c: bool) -> i32 {
    if (c) {
        while (true) {
            if (c) { return 1; }
            continue;
        }
    }
    2
}

entry fn main() -> i32 {
    find([4, 5, 6], 5) + mixed(false)
}
"#,
    );
    assert_eq!(v, 7);
}

#[test]
fn statement_if_with_returning_arms_needs_no_tail() {
    // 6 — both arms `return`: the trailing if produces NO value (the
    // `!` shape) and still terminates the body — the tail check must
    // not fire a nil mismatch
    let v = run_main(
        r#"
fn pick(c: bool) -> i32 {
    if (c) { return 1; } else { return 2; }
}

entry fn main() -> i32 {
    pick(true) + pick(false) * 10
}
"#,
    );
    assert_eq!(v, 21);
}

#[test]
fn statement_if_tail_with_one_diverging_arm() {
    // 7 — one arm returns, the other produces: the producing arm's
    // value IS the merge's value (the `!` coercion)
    let v = run_main(
        r#"
fn f(c: bool) -> i32 {
    if (c) { return 10; }
    else { 20 }
}

entry fn main() -> i32 {
    f(true) + f(false)
}
"#,
    );
    assert_eq!(v, 30);
}

#[test]
fn else_if_chain_as_tail() {
    // 8 — the chain's nested if is the value
    let v = run_main(
        r#"
fn grade(n: i32) -> i32 {
    if (n > 90) { 4 }
    else if (n > 80) { 3 }
    else { 2 }
}

entry fn main() -> i32 {
    grade(95) * 100 + grade(85) * 10 + grade(50)
}
"#,
    );
    assert_eq!(v, 432);
}

#[test]
fn lambda_block_tail_answers() {
    // 9 — the lambda BLOCK form's tail is the lambda's answer
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let g: fn() -> i32 = fn () -> i32 { 41 };
    g() + 1
}
"#,
    );
    assert_eq!(v, 42);
}

#[test]
fn inlined_fn_body_tail_answers() {
    // 10 — the small-fn inline splices the body: the tail must move
    // into the inline result slot (the old path left the slot unwritten)
    let v = run_main(
        r#"
fn five() -> i32 {
    5
}

entry fn main() -> i32 {
    five() + five()
}
"#,
    );
    assert_eq!(v, 10);
}

#[test]
fn capture_inside_tail_stays_linked() {
    // 11 — the capture-law walk must cross the tail: the closure whose
    // body IS a tail still captures `b` (the silent-wrong mode was a
    // capture scan that stopped at the last `;`)
    let v = run_main(
        r#"
class Box2(i32);
impl Box2 { pub fn get(self) -> i32 { return self.inner; } }

entry fn main() -> i32 {
    let b = Box2(40);
    let f = fn () -> i32 { b.get() + 2 };
    f()
}
"#,
    );
    assert_eq!(v, 42);
}

#[test]
fn captured_and_reassigned_binding_promotes_through_tail() {
    // 11b — the promotion half of the capture law: the binding is
    // captured (inside a tail) AND reassigned AND ref-headed, so both
    // frames share one cell — the closure reads the NEW value
    let v = run_main(
        r#"
class Box2(i32);
impl Box2 { pub fn get(self) -> i32 { return self.inner; } }

entry fn main() -> i32 {
    let mut b = Box2(1);
    let f = fn () -> i32 { b.get() };
    b = Box2(2);
    f()
}
"#,
    );
    assert_eq!(v, 2);
}

#[test]
fn when_block_arms_produce_values() {
    // 12 — `when` arm blocks are value blocks; mixed expr/block arms
    // unify under the same law
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let x = 1;
    let w = when (x) {
        0 -> { 10 },
        1 -> { 11 },
        else -> { 12 },
    };
    let mixed = when (x) {
        0 -> 50,
        else -> { 60 },
    };
    w * 100 + mixed
}
"#,
    );
    assert_eq!(v, 1160);
}

#[test]
fn when_arm_block_locals_die_before_the_value_lands() {
    // 12b — the arm block's binding is scoped; the arm's value moves
    // out of the dying scope (the register-lifetime law in arm blocks)
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let x = 0;
    let w = when (x) {
        0 -> { let t = 40; t },
        else -> { 2 },
    };
    w + 2
}
"#,
    );
    assert_eq!(v, 42);
}

#[test]
fn when_nil_arm_blocks_stay_statement_blocks() {
    // 13 — the pre-existing shape: nil-context block arms
    // (`-> { a(); b(); }`) compile as statements, values discarded
    let v = run_main(
        r#"
class Cell { total: i32 = 0; }

entry fn main() -> i32 {
    let c = Cell { };
    let mut total = 0;
    when (true) {
        true -> { total = total + 9; },
        else -> { total = total + 0; },
    }
    total + 10
}
"#,
    );
    assert_eq!(v, 19);
}

// ---- diagnostics ----

#[test]
fn reject_tail_type_mismatch() {
    let msg = one_diag(
        r#"
fn f() -> i32 {
    let x = 1;
    "s"
}

entry fn main() -> i32 {
    f()
}
"#,
    );
    assert!(msg.contains("tail type mismatch"), "got: {msg}");
    assert!(msg.contains("`i32` expected"), "got: {msg}");
    assert!(msg.contains("`str` is the body's tail"), "got: {msg}");
}

#[test]
fn reject_value_tail_in_nil_fn() {
    let msg = one_diag(
        r#"
fn f() -> nil {
    1
}

entry fn main() -> nil {
    f();
}
"#,
    );
    assert!(msg.contains("tail type mismatch"), "got: {msg}");
    assert!(msg.contains("`nil` expected"), "got: {msg}");
}

#[test]
fn reject_if_tail_arms_disagree() {
    let msg = one_diag(
        r#"
fn f(c: bool) -> i32 {
    if (c) { 1 } else { "s" }
}

entry fn main() -> i32 {
    f(true)
}
"#,
    );
    assert!(msg.contains("`if` arms must agree"), "got: {msg}");
    assert!(msg.contains("`i32` vs `str`"), "got: {msg}");
}

#[test]
fn reject_if_tail_without_else_still_missing_return() {
    // the no-else trailing if is the statement form — the block has no
    // value and the body can fall through
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
    assert!(msg.contains("`f`"), "got: {msg}");
}

#[test]
fn reject_lambda_block_tail_mismatch() {
    let msg = one_diag(
        r#"
entry fn main() -> nil {
    let g: fn() -> i32 = fn () -> i32 { "s" };
    let n = g();
}
"#,
    );
    assert!(msg.contains("tail type mismatch"), "got: {msg}");
}

#[test]
fn reject_bare_fall_through_still_diagnoses() {
    // the law the tails build on: statements only, no tail — unchanged
    let msg = one_diag(
        r#"
fn f() -> str {
    let x = 1;
}

entry fn main() -> nil {
    let s = f();
}
"#,
    );
    assert!(msg.contains("missing return"), "got: {msg}");
}
