//! The mut-binding law for loop variables: the header's `mut` is the
//! write law for BOTH `for` forms. A for-of element view (`for (let
//! mut x of xs)`) gates field writes and whole-var reassignment
//! through `x`; a for-c induction variable (`for (let mut i = ...)`)
//! gates every assignment to `i` — the update clause included, it is
//! author code. Reads never gate; the shared element (for-of yields
//! the stored one, shared by ref) and the capture law's promotion
//! machinery are unchanged. A plain loop variable's diagnostic names
//! the loop header as the fix.

use std::cell::RefCell;
use std::rc::Rc;

/// The case world: the toolchain lib these cases use (`ink`'s Logger)
/// walked from the tree as a real package. `pouch` stays out: its own
/// un-mut for-c headers are a phase-3 corpus site under the new law,
/// and these cases need no Vec (plain arrays carry the element views).
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
            rut_driver::RutRun::new().pkg(
                rut_driver::Mounted::source("test", &combined)
                    .manifest(rut_driver::Manifest::lib("test", &["ink"])),
            ),
            |r, p| r.pkg(p.clone()),
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
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(hosts)
        .build()
        .expect("vm");
    vm.call::<_, ()>("main", ()).expect("run");
    let out = lines.borrow().clone();
    out
}
// ---- for-of: the header's `mut` gates writes through the element ----

#[test]
fn for_of_mut_element_write_compiles_and_runs() {
    // the todo `toggle` shape: the write hits the STORED element (the
    // shared element, shared by ref), so the read-back loop sees it
    let lines = run_lines(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }, Task { done: true }];
    for (let mut t of tasks) {
        t.done = !t.done;
    }
    let mut trues = 0;
    for (let t of tasks) {
        if (t.done) { trues += 1; }
    }
    Logger::new("law").info(f"trues={trues}");
}
"#,
    );
    assert_eq!(lines, vec!["trues=1"]);
}

#[test]
fn closure_capture_of_plain_let_for_of_var_write_diagnoses() {
    // the capture inherits the binding's (absent) mut: the write
    // through the captured var is refused inside the closure frame too
    let diags = diags_of(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }];
    for (let t of tasks) {
        let flip = fn () -> bool {
            t.done = true;
            return t.done;
        };
        flip();
    }
}
"#,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("mut") && d.contains("binding")),
        "want a mut-binding law diag for the captured plain loop var: {diags:?}"
    );
}

#[test]
fn for_of_plain_let_field_write_diagnoses() {
    let diags = diags_of(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }];
    for (let t of tasks) {
        t.done = true;
    }
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains(
            "assignment through `t` requires `mut` in the loop header: `for (let mut t of ...)` (the mut-binding law)"
        )),
        "want the loop-header law diag: {diags:?}"
    );
}

#[test]
fn for_of_plain_let_whole_var_reassign_diagnoses() {
    let diags = diags_of(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }];
    for (let t of tasks) {
        t = Task { done: true };
    }
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains(
            "assignment to `t` requires `mut` in the loop header: `for (let mut t of ...)` (the mut-binding law)"
        )),
        "want the loop-header law diag: {diags:?}"
    );
}

// ---- for-c: every assignment to the induction var needs the header's mut ----

#[test]
fn for_c_update_clause_without_mut_diagnoses() {
    // `i += 1` is author code: the same law as a body assignment
    let diags = diags_of(
        r#"
entry fn main() -> nil {
    for (let i = 0; i < 3; i += 1) {
        Logger::new("law").info(f"{i}");
    }
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains(
            "assignment to `i` requires `mut` in the loop header: `for (let mut i = ...)` (the mut-binding law)"
        )),
        "want the loop-header law diag: {diags:?}"
    );
}

#[test]
fn for_c_body_assignment_without_mut_diagnoses() {
    // the update slot stays read-only; the BODY assigns `i`
    let diags = diags_of(
        r#"
entry fn main() -> nil {
    let mut n = 3;
    for (let i = 0; n > 0; i) {
        i = i + 1;
        n -= 1;
    }
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains(
            "assignment to `i` requires `mut` in the loop header: `for (let mut i = ...)` (the mut-binding law)"
        )),
        "want the loop-header law diag: {diags:?}"
    );
}

#[test]
fn for_c_mut_runs_end_to_end() {
    let lines = run_lines(
        r#"
entry fn main() -> nil {
    let mut total = 0;
    for (let mut i = 0; i < 4; i += 1) {
        total += i;
    }
    Logger::new("law").info(f"total={total}");
}
"#,
    );
    assert_eq!(lines, vec!["total=6"]);
}

// ---- the capture law × mut: a closure writes through the captured var ----

#[test]
fn closure_capture_of_mut_for_of_var_writes_through_the_shared_cell() {
    // the lambda's write through the captured `t` must land on the
    // stored element: the capture pools the loop var's shared cell
    // (the promotion machinery is unchanged — only the write law's
    // exemption went away, and the capture inherits the header's mut)
    let lines = run_lines(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }];
    for (let mut t of tasks) {
        let flip = fn () -> bool {
            t.done = true;
            return t.done;
        };
        flip();
    }
    let mut trues = 0;
    for (let t of tasks) {
        if (t.done) { trues += 1; }
    }
    Logger::new("law").info(f"trues={trues}");
}
"#,
    );
    assert_eq!(lines, vec!["trues=1"]);
}

// ---- the emit-closure form: the flag rides the for_of_sigs map ----

#[test]
fn user_iterable_for_of_mut_threads_the_emit_closure() {
    // a user `[iterable]` desugars to the synthetic emit closure
    // (`FnKey::ForOfEmit`): the header's mut must cross the
    // monomorphization queue inside the sig and gate this frame's
    // writes the same way the fused form does
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
    for (let mut v of Bag { }) {
        v = v * 10;
        Logger::new("law").info(f"v={v}");
    }
}
"#,
    );
    assert_eq!(lines, vec!["v=10", "v=20"]);
}

#[test]
fn user_iterable_for_of_plain_let_write_diagnoses() {
    let diags = diags_of(
        r#"
class Bag { }
impl Bag {
    [iterable] fn iterate(self, emit: fn(i32) -> bool) {
        emit(1);
    }
}
entry fn main() -> nil {
    for (let v of Bag { }) {
        v = 3;
    }
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains(
            "assignment to `v` requires `mut` in the loop header: `for (let mut v of ...)` (the mut-binding law)"
        )),
        "want the loop-header law diag: {diags:?}"
    );
}

// ---- async for-of: the flag threads the weave ----

#[test]
fn async_for_of_mut_var_runs() {
    let (mut vm, sink) = async_setup(
        r#"
use ink_host::{ create_logger, logger_log };
use futures::launch_future;

async fn flip_all(log: opaque, flags: [bool]) -> nil {
    for (let mut v of flags) {
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

#[test]
fn async_for_of_plain_let_write_diagnoses() {
    let diags = async_diags_of(
        r#"
use ink_host::{ create_logger, logger_log };
use futures::launch_future;

async fn break_all(log: opaque, flags: [bool]) -> nil {
    for (let v of flags) {
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
        diags.iter().any(|d| d.contains(
            "assignment to `v` requires `mut` in the loop header: `for (let mut v of ...)` (the mut-binding law)"
        )),
        "want the loop-header law diag: {diags:?}"
    );
}

// ---- reads never gate: plain loop variables stay legal ----

#[test]
fn read_only_loops_stay_legal_without_mut() {
    // the for-of var is read only; the for-c var never assigned
    // (the `while`-style shape: the condition carries the progress)
    let lines = run_lines(
        r#"
struct Task { done: bool }
entry fn main() -> nil {
    let tasks = [Task { done: false }, Task { done: true }];
    let mut trues = 0;
    for (let t of tasks) {
        if (t.done) { trues += 1; }
    }
    let mut n = 3;
    let mut total = 0;
    for (let i = 0; n > 0; i) {
        total += i;
        n -= 1;
    }
    Logger::new("law").info(f"trues={trues} total={total}");
}
"#,
    );
    assert_eq!(lines, vec!["trues=1 total=0"]);
}

// ---- the async lane's harness (the async_basics shape) ----

const INK_HOST_DECL: &str = include_str!("../../../rut/ink_host/ink_host.d.rut");

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
    .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
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
        .hooks(rut_vm::interp::HostHooks::default())
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
    .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
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
