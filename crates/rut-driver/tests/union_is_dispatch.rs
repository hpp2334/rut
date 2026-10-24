//! The closed-union dispatch: `x is <prim>` folds (prims carry no
//! identity bits — the test is compile-time), and the if lowerings
//! prune the dead arm of a folded condition, which then neither
//! compiles nor typechecks. The pair is `calc`'s `sqrt<T>` shape: a
//! generic body over `T requires f32 | f64` routes per member, and the
//! live arm may name members the dead one could not (an f64-only fn in
//! the f64 arm). The interface folds and literal-true conditions keep
//! today's both-arms compile (the agreement diagnostic stays — see
//! if_expr.rs).

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

// ---- the fold itself ----

#[test]
fn prim_is_folds_both_ways() {
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let f: f32 = 1.5;
    let d: f64 = 2.5;
    (f is f32 as i32) * 1000 + (f is f64 as i32) * 100
        + (d is f64 as i32) * 10 + (d is f32 as i32)
}
"#,
    );
    assert_eq!(v, 1010);
}

#[test]
fn prim_is_folds_on_a_literal_receiver() {
    let v = run_main(
        r#"
entry fn main() -> i32 {
    (2.5f64 is f64 as i32) * 10 + (2.5f64 is f32 as i32)
}
"#,
    );
    assert_eq!(v, 10);
}

// ---- the pruning: the dead arm never typechecks ----

#[test]
fn folded_dispatch_prunes_the_dead_arm() {
    // `only64` takes f64 ONLY: the f32 member's else arm cannot
    // typecheck — pruning is what lets the member compile at all
    let v = run_main(
        r#"
fn only64(v: f64) -> f64 { v + 0.0 }

fn pick<T requires f32 | f64>(x: T) -> T {
    if (x is f32) { x } else { only64(x) }
}

entry fn main() -> i32 {
    let a = pick(2.5f32);
    let b = pick(4.5f64);
    if (a == 2.5f32 && b == 4.5) { 1 } else { 0 }
}
"#,
    );
    assert_eq!(v, 1);
}

#[test]
fn folded_dispatch_selects_through_an_else_if_chain() {
    let v = run_main(
        r#"
fn width<T requires f32 | f64>(x: T) -> i32 {
    if (x is f32) { 32 } else if (x is f64) { 64 } else { 0 }
}

entry fn main() -> i32 {
    width(1.0f32) * 100 + width(2.0f64) * 10
}
"#,
    );
    assert_eq!(v, 3840);
}

#[test]
fn folded_dispatch_in_the_value_form() {
    // the if ATOM (expression position) prunes through the same law
    let v = run_main(
        r#"
fn only64(v: f64) -> f64 { v * 2.0 }

fn pick<T requires f32 | f64>(x: T) -> f64 {
    if (x is f32) { 1.0 } else { only64(x) }
}

entry fn main() -> i32 {
    pick(9.0f64) as i32
}
"#,
    );
    assert_eq!(v, 18);
}

#[test]
fn folded_dispatch_in_the_tail_statement_form() {
    let v = run_main(
        r#"
fn only64(v: f64) -> f64 { v * 3.0 }

fn pick<T requires f32 | f64>(x: T) -> f64 {
    if (x is f32) {
        2.0
    } else {
        only64(x)
    }
}

entry fn main() -> i32 {
    pick(5.0f64) as i32
}
"#,
    );
    assert_eq!(v, 15);
}

#[test]
fn no_else_form_runs_only_the_live_arm() {
    let v = run_main(
        r#"
entry fn main() -> i32 {
    let d: f64 = 1.0;
    let mut n = 0;
    if (d is f32) { n = 10; }
    if (d is f64) { n = n + 1; }
    n
}
"#,
    );
    assert_eq!(v, 1);
}

#[test]
fn explicit_type_argument_picks_the_member() {
    let v = run_main(
        r#"
fn width<T requires f32 | f64>(x: T) -> i32 {
    if (x is f32) { 32 } else { 64 }
}

entry fn main() -> i32 {
    width<f64>(1.0) * 10 + width<f32>(2.0)
}
"#,
    );
    assert_eq!(v, 672);
}

// ---- the laws that stay ----

#[test]
fn a_union_bound_rejects_out_of_union_members() {
    let msg = one_diag(
        r#"
fn width<T requires f32 | f64>(x: T) -> i32 {
    if (x is f32) { 32 } else { 64 }
}

entry fn main() -> i32 {
    width(3)
}
"#,
    );
    assert!(msg.contains("f32") && msg.contains("f64"), "got: {msg}");
}

#[test]
fn literal_true_still_compiles_both_arms() {
    // the pruning is the `is` fold's dispatch, not a general
    // constant-folding law: the agreement diagnostic keeps firing on a
    // literal condition (if_expr.rs's pinned shape)
    let msg = one_diag(
        r#"
entry fn main() -> i32 {
    let x = if (true) { 1 } else { "s" };
    x
}
"#,
    );
    assert!(msg.contains("`if` arms must agree"), "got: {msg}");
}

#[test]
fn interface_is_keeps_the_both_arms_compile() {
    // only the PRIM fold prunes: an interface capability probe on an
    // exact receiver folds too, but the arm law is untouched there
    let msg = one_diag(
        r#"
interface Ser {
    fn encode(self) -> str;
}

class P(i32);
impl P { pub fn encode(self) -> str { "P" } }

entry fn main() -> i32 {
    let p = P(1);
    let x = if (p is Ser) { 1 } else { "s" };
    x
}
"#,
    );
    assert!(msg.contains("`if` arms must agree"), "got: {msg}");
}
