//! The const law: `const` is the shallow opt-out of mutability-by-
//! default. A direct reassign of a `const` binding errors (`x = ..`);
//! writes THROUGH one (`x.f = ..`) are allowed — shallow by design
//! ("reassign only"). Loop headers: `for (const t of xs)` is a fresh
//! per-iteration const binding (the loop's internal rebinding is
//! machinery, not user reassignment); `const` in a for-c header is
//! refused at parse (the induction variable is genuinely reassigned).
//! Plain `let` and unflagged loop variables are mutable — the default.
//! A const binding reassigned inside a closure hits the same error
//! through the shared compiler lane (the capture law is untouched).

use std::cell::RefCell;
use std::rc::Rc;

/// The case world: the toolchain lib these cases use (`ink`'s Logger)
/// walked from the tree as a real package. `pouch` stays out: these
/// cases need no Vec (plain arrays carry the element views).
fn world() -> Vec<rut_driver::Mounted> {
    let mut world = Vec::new();
    {
        let d = "rut/ink";
        world.extend(
            rut_native::dir_pkgs(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .join(d),
            )
            .expect("mount case libs")
            .pkgs,
        );
    }
    world
}

fn chain(src: &str) -> rut_driver::RutRun {
    // the logger use appended so the original spans stay put
    let combined = format!("{src}\nuse ink::{{Logger}};\n");
    let world = world();
    world
        .iter()
        .fold(
            rut_driver::RutRun::new().mount(
                rut_driver::Mounted::source("test", &combined)
                    .manifest(rut_driver::Manifest::lib("test", &["ink"])),
            ),
            |r, p| r.mount(p.clone()),
        )
        .entrypoint("test")
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

fn diags_of(src: &str) -> Vec<String> {
    graph_of(chain(src).compile())
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

/// Compile, flatten, verify, and run to the logger sink.
fn run_lines(src: &str) -> Vec<String> {
    let compiled = chain(src).compile().expect("compile the case");
    assert!(
        compiled.graph.diags.is_empty(),
        "unexpected diags:\n{:?}",
        compiled.graph.diags
    );
    let prog = rut_core::link::flatten(compiled.graph.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let lines = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = lines.clone();
    let ctx = rut_driver::host_pkg_ctx(&world());
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(
        &ctx,
        rut_std::logger::pkg(move |msg| sink.borrow_mut().push(msg.to_string())),
    );
    hosts.verify_against(&ctx.flatten());
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .expect("vm");
    vm.call::<_, ()>("main", ()).expect("run");
    let out = lines.borrow().clone();
    out
}
// ---- the direct lane: a `const` binding's reassign is refused ----

#[test]
fn const_reassign_is_refused() {
    let diags = diags_of(
        r#"
entry fn main() -> nil {
    const x = 1;
    x = 2;
}
"#,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("cannot reassign `x` — a `const` binding is not reassignable")),
        "want the const-law diag: {diags:?}"
    );
}

#[test]
fn const_compound_reassign_is_refused() {
    // `+=` reassigns the binding exactly as `=` does
    let diags = diags_of(
        r#"
entry fn main() -> nil {
    const total = 0;
    total += 1;
}
"#,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("cannot reassign `total` — a `const` binding is not reassignable")),
        "want the const-law diag: {diags:?}"
    );
}

// ---- shallow by design: writes THROUGH a const binding are legal ----

#[test]
fn const_field_write_runs() {
    // `t.done = true` is not the binding's own reassignment — allowed
    let lines = run_lines(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    const t = Task { done: false };
    t.done = true;
    Logger::new("law").info(f"done={t.done}");
}
"#,
    );
    assert_eq!(lines, vec!["done=true"]);
}

#[test]
fn const_index_write_runs() {
    // a Vec's element store rides the binding's handle — not a reassign
    let lines = run_lines(
        r#"
entry fn main() -> nil {
    const xs = [1, 2, 3];
    xs[1] = 20;
    Logger::new("law").info(f"{xs[1]}");
}
"#,
    );
    assert_eq!(lines, vec!["20"]);
}

#[test]
fn plain_let_reassign_stays_legal() {
    // mutability is the DEFAULT — the retired `let mut` spell is gone
    let lines = run_lines(
        r#"
entry fn main() -> nil {
    let x = 1;
    x = 2;
    x += 40;
    Logger::new("law").info(f"x={x}");
}
"#,
    );
    assert_eq!(lines, vec!["x=42"]);
}

// ---- for-of: `const` is the per-iteration binding's opt-out ----

#[test]
fn for_of_const_field_write_runs() {
    // the loop's internal rebinding is machinery; a write THROUGH the
    // const element view is author-legal (shallow)
    let lines = run_lines(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    const tasks = [Task { done: false }, Task { done: true }];
    for (const t of tasks) {
        t.done = !t.done;
    }
    let trues = 0;
    for (const t of tasks) {
        if (t.done) { trues += 1; }
    }
    Logger::new("law").info(f"trues={trues}");
}
"#,
    );
    assert_eq!(lines, vec!["trues=1"]);
}

#[test]
fn for_of_const_whole_var_reassign_is_refused() {
    // `t = Task { .. }` IS the binding's own reassignment — the const
    // element view refuses it through the shared direct lane
    let diags = diags_of(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }];
    for (const t of tasks) {
        t = Task { done: true };
    }
}
"#,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("cannot reassign `t` — a `const` binding is not reassignable")),
        "want the const-law diag: {diags:?}"
    );
}

#[test]
fn for_of_plain_let_writes_stay_legal() {
    // the unflagged loop variable is mutable — field writes AND whole-
    // var reassignment, both legal under the default
    let lines = run_lines(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }];
    for (let t of tasks) {
        t.done = true;
        Logger::new("law").info(f"t.done={t.done}");
    }
    let v = 1;
    for (let w of [v]) {
        Logger::new("law").info(f"w={w}");
    }
}
"#,
    );
    assert_eq!(lines, vec!["t.done=true", "w=1"]);
}

// ---- for-c: `const` in the header is refused at parse ----

#[test]
fn for_c_const_header_is_refused() {
    // the induction variable is genuinely reassigned by the loop —
    // the dedicated diag
    let diags = diags_of(
        r#"
entry fn main() -> nil {
    for (const i = 0; i < 3; i += 1) {
        Logger::new("law").info(f"{i}");
    }
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains("reassigned by the loop")),
        "want the for-c const refusal: {diags:?}"
    );
}

#[test]
fn for_c_plain_header_runs_end_to_end() {
    // the induction var is loop machinery: the update clause's write
    // is not user reassignment and never gates
    let lines = run_lines(
        r#"
entry fn main() -> nil {
    let total = 0;
    for (let i = 0; i < 4; i += 1) {
        total += i;
    }
    Logger::new("law").info(f"total={total}");
}
"#,
    );
    assert_eq!(lines, vec!["total=6"]);
}

#[test]
fn for_c_body_assignment_stays_legal() {
    // the BODY assigning the induction var is legal too (mutable default)
    let lines = run_lines(
        r#"
entry fn main() -> nil {
    let n = 3;
    let total = 0;
    for (let i = 0; n > 0; i) {
        i = i + 1;
        total += i;
        n -= 1;
    }
    Logger::new("law").info(f"total={total}");
}
"#,
    );
    assert_eq!(lines, vec!["total=6"]);
}

// ---- the capture law × const: a closure's reassign hits the same lane ----

#[test]
fn closure_capture_of_const_reassign_is_refused() {
    // the capture inherits the const flag; the closure frame's write
    // reassigns the binding — the shared compiler lane refuses it
    let diags = diags_of(
        r#"
entry fn main() -> nil {
    const base = 10;
    let bump = fn () -> i32 {
        base = base + 1;
        return base;
    };
    bump();
}
"#,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("cannot reassign `base` — a `const` binding is not reassignable")),
        "want the const-law diag: {diags:?}"
    );
}

#[test]
fn closure_capture_of_const_write_through_stays_legal() {
    // a captured const record's field write through the closure is the
    // shallow lane, not a reassign — legal, and it lands on the shared
    // cell (the capture law is untouched)
    let lines = run_lines(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    const t = Task { done: false };
    let flip = fn () -> bool {
        t.done = true;
        return t.done;
    };
    flip();
    Logger::new("law").info(f"done={t.done}");
}
"#,
    );
    assert_eq!(lines, vec!["done=true"]);
}

// ---- the emit-closure form: the const flag rides the for_of_sigs map ----

#[test]
fn user_iterable_for_of_const_reassign_is_refused() {
    // a user `[iterable]` desugars to the synthetic emit closure
    // (`FnKey::ForOfEmit`): the header's const must cross the
    // monomorphization queue inside the sig and gate this frame's
    // reassignment the same way the fused form does
    let diags = diags_of(
        r#"
class Bag { }
impl Bag {
    [iterable] fn iterate(self, emit: fn(i32) -> bool) {
        emit(1);
    }
}
entry fn main() -> nil {
    for (const v of Bag { }) {
        v = 3;
    }
}
"#,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("cannot reassign `v` — a `const` binding is not reassignable")),
        "want the const-law diag: {diags:?}"
    );
}

#[test]
fn user_iterable_for_of_const_write_through_runs() {
    let lines = run_lines(
        r#"
class Bag { }
impl Bag {
    [iterable] fn iterate(self, emit: fn(i32) -> bool) {
        emit(1);
        emit(2);
    }
}
entry fn main() -> nil {
    for (const v of Bag { }) {
        Logger::new("law").info(f"v={v}");
    }
}
"#,
    );
    assert_eq!(lines, vec!["v=1", "v=2"]);
}

// ---- async for-of: the const flag threads the weave ----

#[test]
fn async_for_of_const_var_runs() {
    let (mut vm, sink) = async_setup(
        r#"
use ink_host::{ create_logger, logger_log };
use futures::launch_future;

async fn flip_all(log: opaque, flags: [bool]) -> nil {
    let flipped = 0;
    for (const v of flags) {
        if (v) { flipped += 1; }
    }
    logger_log(log, 2, f"flipped={flipped}");
}

entry fn main() -> nil {
    let log = create_logger("t");
    launch_future(flip_all(log, [false, true]));
}
"#,
    );
    vm.call::<_, ()>("main", ()).expect("main");
    run_async_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["flipped=1"]);
}

#[test]
fn async_for_of_const_reassign_diagnoses() {
    let diags = async_diags_of(
        r#"
use ink_host::{ create_logger, logger_log };
use futures::launch_future;

async fn break_all(log: opaque, flags: [bool]) -> nil {
    for (const v of flags) {
        v = true;
    }
}

entry fn main() -> nil {
    let log = create_logger("t");
    launch_future(break_all(log, [false, true]));
}
"#,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("cannot reassign `v` — a `const` binding is not reassignable")),
        "want the const-law diag: {diags:?}"
    );
}

#[test]
fn async_for_of_plain_let_write_stays_legal() {
    let (mut vm, sink) = async_setup(
        r#"
use ink_host::{ create_logger, logger_log };
use futures::launch_future;

async fn flip_all(log: opaque, flags: [bool]) -> nil {
    for (let v of flags) {
        v = !v;
    }
    logger_log(log, 2, "flipped");
}

entry fn main() -> nil {
    let log = create_logger("t");
    launch_future(flip_all(log, [false, true]));
}
"#,
    );
    vm.call::<_, ()>("main", ()).expect("main");
    run_async_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["flipped"]);
}

// ---- reads never gate ----

#[test]
fn read_only_loops_stay_legal() {
    // the for-of var is read only; the for-c var never assigned
    // (the `while`-style shape: the condition carries the progress)
    let lines = run_lines(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }, Task { done: true }];
    let trues = 0;
    for (let t of tasks) {
        if (t.done) { trues += 1; }
    }
    const n = 3;
    let total = 0;
    let i = 0;
    while (n > 0) {
        total += i;
        i += 1;
        n;
        break;
    }
    Logger::new("law").info(f"trues={trues}");
}
"#,
    );
    assert_eq!(lines, vec!["trues=1"]);
}

const INK_HOST_DECL: &str = include_str!("../../../rut/ink_host/src/ink_host.d.rut");

fn async_pkgs(src: &str) -> Vec<rut_driver::Mounted> {
    let ink_host = rut_driver::lower_decl_module(
        INK_HOST_DECL,
        &std::collections::BTreeMap::new(),
        "ink_host.d.rut",
    )
    .expect("the ink_host surface is valid")
    .named("ink_host");
    let mut pkgs = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    pkgs.push(ink_host);
    pkgs.push(
        rut_driver::Mounted::source("app", src)
            .manifest(rut_driver::Manifest::lib("app", &["futures", "ink_host"])),
    );
    pkgs
}

fn async_setup(src: &str) -> (rut_vm::interp::Vm, Rc<RefCell<Vec<String>>>) {
    let pkgs = async_pkgs(src);
    let expected = rut_driver::declared_host_fns(&pkgs);
    let compiled = rut_driver::Walk {
        pkgs: pkgs.clone(),
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "diags: {:?}",
        compiled.graph.diags
    );
    let prog = compiled.graph.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let sink = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink2 = sink.clone();
    let ctx = rut_driver::host_pkg_ctx(&pkgs);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(
        &ctx,
        rut_std::logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())),
    );
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    hosts.verify_against(&expected);
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(flat))
        .limits(limits)
        .hosts(hosts)
        .build()
        .expect("vm");
    (vm, sink)
}

fn async_diags_of(src: &str) -> Vec<String> {
    let pkgs = async_pkgs(src);
    rut_driver::Walk {
        pkgs,
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap()
    .graph
    .diags
    .iter()
    .map(|d| d.msg.clone())
    .collect()
}

/// The async driving loop: drain the ready queue, wake timers by
/// advancing the virtual clock (capped — a stalled loop fails).
fn run_async_loop(vm: &mut rut_vm::interp::Vm, cap: usize) {
    for _ in 0..cap {
        vm.run_ready().expect("run_ready");
        match vm.next_deadline() {
            Some(d) => vm.set_now(d),
            None => {
                if vm.pending_tasks() == 0 {
                    return;
                }
            }
        }
    }
    panic!(
        "the driving loop stalled: {} task(s) pending",
        vm.pending_tasks()
    );
}
