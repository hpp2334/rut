//! Definite-return analysis: a non-nil fn must return on every path.
//! The fall-through used to emit a bare `Ret { val: None }` and the
//! caller read a never-written ABI answer register (debug: "null
//! reference produced", release: SIGSEGV) — now the compile diagnoses
//! (`missing return: ... is `-> T` but a path reaches the end of the
//! body`) and aborts the run before anything executes. Pins: the
//! reject shapes (bare fall-through, a one-armed `if`, the lambda
//! block form, the `format_list` todolist shape) and the accepts
//! (both-arm if/else, `-> nil` fall-off, the arrow lambda, dead code
//! after a `return` — no false positive). The accepts also RUN: the
//! diagnostics are the only guard, so the clean shapes must execute.

use rut_vm::interp::Vm;

/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn one_diag(src: &str) -> String {
    let d = compiled("app", src);
    let msgs: Vec<String> = d.diags.iter().map(|d| d.msg.clone()).collect();
    assert_eq!(msgs.len(), 1, "expected exactly one diagnostic, got {msgs:?}");
    msgs[0].clone()
}

/// Compile, then execute the entry through the driver's run lane —
/// the accepts prove the law does not fire on them and the program
/// still runs to completion.
fn run_ok(src: &str) {
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
    vm.call::<_, ()>("main", ()).expect("run");
}

// ---- rejects: one diagnostic each, the message names the law ----

#[test]
fn reject_bare_fall_through_min_repro() {
    // 1 — the ground-truth repro: `-> str`, no return, the caller used
    // to read the never-written answer register
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
    assert!(msg.contains("`f`"), "got: {msg}");
    assert!(msg.contains("`-> str`"), "got: {msg}");
    assert!(
        msg.contains("but a path reaches the end of the body"),
        "got: {msg}"
    );
}

#[test]
fn reject_if_returning_on_one_branch_only() {
    // 2 — the condition-false path falls off the end
    let msg = one_diag(
        r#"
fn pick(c: bool) -> str {
    if (c) { return "a"; }
}

entry fn main() -> nil {
    let s = pick(true);
}
"#,
    );
    assert!(msg.contains("missing return"), "got: {msg}");
    assert!(msg.contains("`pick`"), "got: {msg}");
}

#[test]
fn reject_lambda_block_fall_through() {
    // 3 — the lambda BLOCK form has the same hole the fn body had
    let msg = one_diag(
        r#"
entry fn main() -> nil {
    let g: fn() -> str = fn () -> str { let x = 1; };
    let s = g();
}
"#,
    );
    assert!(msg.contains("missing return"), "got: {msg}");
    assert!(msg.contains("lambda"), "got: {msg}");
}

#[test]
fn reject_format_list_fall_through_names_it() {
    // 6 — the todolist shape (dep-free facsimile): the crashing fn is
    // named by the diagnostic
    let msg = one_diag(
        r#"
fn format_list(items: i32) -> str {
    let head = "items:";
    let n = items;
}

entry fn main() -> nil {
    let s = format_list(3);
}
"#,
    );
    assert!(msg.contains("missing return"), "got: {msg}");
    assert!(msg.contains("`format_list`"), "got: {msg}");
}

#[test]
fn reject_while_true_that_break_exits() {
    // the scan-loop refinement's boundary: a `break` bound to the
    // `while (true)` can jump past the loop, so the conservative rule
    // applies and the fn must still return after it
    let msg = one_diag(
        r#"
fn read(c: bool) -> str {
    while (true) {
        if (c) { break; }
    }
}

entry fn main() -> nil {
    let s = read(false);
}
"#,
    );
    assert!(msg.contains("missing return"), "got: {msg}");
    assert!(msg.contains("`read`"), "got: {msg}");
}

// ---- accepts: no diagnostics, and the program RUNS ----

#[test]
fn accept_if_else_both_arms_return() {
    // 8 — the merge point is unreachable: every path returns
    run_ok(
        r#"
fn pick(c: bool) -> str {
    if (c) { return "a"; } else { return "b"; }
}

entry fn main() -> nil {
    let s = pick(true);
}
"#,
    );
}

#[test]
fn accept_nil_fn_falls_off_the_end() {
    // 9 — the implicit return stays: `-> nil` fns are unchanged
    run_ok(
        r#"
fn bump(n: i32) -> nil {
    let m = n + 1;
}

entry fn main() -> nil {
    bump(1);
}
"#,
    );
}

#[test]
fn accept_lambda_forms_that_return() {
    // 11 — the arrow-expression form returns its expression; the
    // anonymous block form with an explicit `return` — both clean
    run_ok(
        r#"
entry fn main() -> nil {
    let g: fn() -> str = fn () -> str { return "x"; };
    let h: fn() -> str = () -> str => "y";
    let s = g();
    let t = h();
}
"#,
    );
}

#[test]
fn accept_while_true_scan_loop_all_exits_body() {
    // the scan-loop law (the json reader's shape — and rustc's
    // `loop {}`): a `while (true)` with no `break` bound to it never
    // reaches its end label, so the fn needs no return after it. The
    // call takes the TRUE path: the loop exits only through `return`.
    run_ok(
        r#"
fn read(c: bool) -> str {
    while (true) {
        if (c) { return "y"; }
        continue;
    }
}

entry fn main() -> nil {
    let s = read(true);
}
"#,
    );
}

#[test]
fn accept_dead_code_after_return() {
    // 12 — statements after a `return` still compile (dead code) and
    // the terminating `return` is what the analysis sees
    run_ok(
        r#"
fn dead() -> str {
    return "x";
    let y = 1;
}

entry fn main() -> nil {
    let s = dead();
}
"#,
    );
}
