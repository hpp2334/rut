// the async/await landing: the Future-only vocabulary.
// The weave (checkpoint brtable, the await expansion, the cancelled
// probe → drop path), the driving loop (ready + timer queues,
// drive / next_deadline / cancel), and the standard host set
// (async_engine + async_host). Trigger→completion is observed through
// the ink_host logger's recording native — the fmt batch's semantic-test
// pattern; diagnostics are pinned per message.

use std::cell::RefCell;
use std::rc::Rc;

use rut_driver::{Module, ModuleBody, Session, compile_graph, lower_decl_module, mount_std_async, mount_std_core};
use rut_vm::interp::{HostRegistry, Limits, Vm};

const INK_HOST_DECL: &str = include_str!("../../../rut/ink_host/ink_host.d.rut");

fn setup(src: &str) -> (Vm, Rc<RefCell<Vec<String>>>) {
    let mut s = Session::new();
    mount_std_core(&mut s);
    let ink_host = lower_decl_module(INK_HOST_DECL, "ink_host.d.rut").expect("the ink_host surface is valid");
    s.register_module("ink_host", ink_host).expect("mount ink_host");
    mount_std_async(&mut s);
    s.register_module("app", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() })
        .expect("register app");
    let out = compile_graph(&s, "app");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let sink = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink2 = sink.clone();
    let ctx = s.host_pkg_context();
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())));
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    hosts.verify_against(&ctx.flatten());
    let limits = Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let vm = Vm::new(Rc::new(flat), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .expect("vm");
    (vm, sink)
}

/// The host loop: drain the ready queue, wake
/// timers by advancing the virtual clock. Capped, so a stalled loop
/// fails instead of hanging.
fn run_loop(vm: &mut Vm, cap: usize) {
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
    panic!("the driving loop stalled: {} task(s) pending", vm.pending_tasks());
}

fn diags_of(src: &str) -> Vec<String> {
    let mut s = Session::new();
    mount_std_core(&mut s);
    let ink_host = lower_decl_module(INK_HOST_DECL, "ink_host.d.rut").expect("ink_host");
    s.register_module("ink_host", ink_host).expect("mount ink_host");
    mount_std_async(&mut s);
    s.register_module("app", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() })
        .expect("register app");
    compile_graph(&s, "app").diags.iter().map(|d| d.msg.clone()).collect()
}

// ---- the weave runs ----

#[test]
fn launch_runs_to_completion() {
    let src = r#"
use core::{ RunContext };
use ink_host::{ create_logger, logger_log };
use async_host::launch_future;

async fn work(cx: RunContext, log: opaque, n: u32) -> nil {
    logger_log(log, 2, "begin");
    logger_log(log, 2, f"n={n}");
    logger_log(log, 2, "end");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(work(log, 7));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["begin", "n=7", "end"]);
}

// ---- park/resume through the sleep pender ----

#[test]
fn park_and_resume_through_sleep() {
    let src = r#"
use core::{ RunContext };
use ink_host::{ create_logger, logger_log };
use async_host::{ launch_future, sleep };

async fn tick(cx: RunContext, log: opaque) -> nil {
    for (let i = 0; i < 3; i += 1) {
        logger_log(log, 2, f"tick{i}");
        await sleep(10);
    }
    logger_log(log, 2, "done");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(tick(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["tick0", "tick1", "tick2", "done"]);
    // the clock advanced: three 10ms sleeps
    assert_eq!(vm.now_ms(), 30);
}

// ---- nested awaits ----

#[test]
fn nested_awaits() {
    let src = r#"
use core::{ RunContext };
use ink_host::{ create_logger, logger_log };
use async_host::{ launch_future, sleep };

async fn inner(cx: RunContext, log: opaque, tag: str) -> nil {
    logger_log(log, 2, f"{tag}:enter");
    await sleep(5);
    logger_log(log, 2, f"{tag}:exit");
}

async fn outer(cx: RunContext, log: opaque) -> nil {
    await inner(log, "a");
    await inner(log, "b");
    logger_log(log, 2, "outer:done");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(outer(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 200);
    assert_eq!(
        *sink.borrow(),
        vec!["a:enter", "a:exit", "b:enter", "b:exit", "outer:done"]
    );
}

// ---- cancellation: abort → the probe at the checkpoint → the drop path ----
//
// The full arc, orchestrated in-rut (the receipt lives in rut): the
// victim parks on a 60ms sleep; the killer waits 10ms, aborts the
// victim's receipt, and re-aborts. The victim's resumed probe fires
// the drop path; the second abort answers false (the state retired).

#[test]
fn abort_after_park_runs_the_drop_path_then_reports_false() {
    let src = r#"
use core::{ RunContext };
use ink_host::{ create_logger, logger_log };
use async_host::{ launch_future, sleep, LaunchedFutureHandle };

async fn victim(cx: RunContext, log: opaque) -> nil {
    let buf: ?str = "held";
    logger_log(log, 2, "victim:park");
    await sleep(60);
    logger_log(log, 2, "victim:unreachable");
}

async fn killer(cx: RunContext, log: opaque, h: LaunchedFutureHandle<nil>) -> nil {
    await sleep(10);
    let ok = h.abort();
    if (ok) { logger_log(log, 2, "killer:aborted"); }
    // let the loop drive the flagged victim: its drop path retires it
    await sleep(5);
    let ok2 = h.abort();
    if (ok2) { logger_log(log, 2, "killer:twice"); } else { logger_log(log, 2, "killer:second-false"); }
}

pub fn main() -> nil {
    let log = create_logger("t");
    let v = launch_future(victim(log));
    launch_future(killer(log, v));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 200);
    assert_eq!(
        *sink.borrow(),
        vec![
            "victim:park",
            "killer:aborted",
            "killer:second-false",
        ]
    );
    // the second abort answering false IS the drop-path observation:
    // the victim's state retired at its checkpoint's drop path before
    // the killer's next wake. The victim's 60ms sleep and the killer's
    // 10ms + 5ms sleeps all armed; the loop's final advance lands on
    // the last deadline (the killer's tail sleep armed after the clock
    // had jumped to 60)
    assert_eq!(vm.now_ms(), 65);
}

// ---- Disposal at the cancellation checkpoint ----

/// A dispose-implementing local that logs through the ink_host logger when
/// the engine releases it — the Disposal analog of the old on_drop
/// observation closures.
const DROPLOG: &str = r#"
class DropLog { log: opaque; }
impl DropLog {
    fn new(log: opaque) -> Self { return Self { log: log }; }
}
impl Disposal for DropLog {
    fn dispose(mut self, cx: DisposalContext) {
        logger_log(self.log, 2, "dropped:buf");
    }
}

class DropTag { log: opaque; tag: str; }
impl DropTag {
    fn new(log: opaque, tag: str) -> Self { return Self { log: log, tag: tag }; }
}
impl Disposal for DropTag {
    fn dispose(mut self, cx: DisposalContext) {
        logger_log(self.log, 2, self.tag);
    }
}
"#;

#[test]
fn abort_after_park_disposes_locals_at_the_checkpoint() {
    let src = format!(
        r#"{DROPLOG}
use core::{{ Disposal, DisposalContext, RunContext }};
use ink_host::{{ create_logger, logger_log }};
use async_host::{{ launch_future, sleep, LaunchedFutureHandle }};

async fn victim(cx: RunContext, log: opaque) -> nil {{
    let buf = DropLog.new(log);
    logger_log(log, 2, "victim:park");
    await sleep(60);
    logger_log(log, 2, "victim:unreachable");
}}

async fn killer(cx: RunContext, log: opaque, h: LaunchedFutureHandle<nil>) -> nil {{
    await sleep(10);
    let ok = h.abort();
    if (ok) {{ logger_log(log, 2, "killer:aborted"); }}
    // let the loop drive the flagged victim: its checkpoint drop path
    // releases the local, and the drive-end drain runs its dispose
    await sleep(5);
    let ok2 = h.abort();
    if (ok2) {{ logger_log(log, 2, "killer:twice"); }} else {{ logger_log(log, 2, "killer:second-false"); }}
}}

pub fn main() -> nil {{
    let log = create_logger("t");
    let v = launch_future(victim(log));
    launch_future(killer(log, v));
}}
"#
    );
    let (mut vm, sink) = setup(&src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 200);
    assert_eq!(
        *sink.borrow(),
        vec![
            "victim:park",
            "killer:aborted",
            "dropped:buf", // the dispose drives before the killer's next wake
            "killer:second-false",
        ]
    );
}

#[test]
fn dispose_locals_fire_in_reverse_order_at_the_checkpoint() {
    let src = format!(
        r#"{DROPLOG}
use core::{{ Disposal, DisposalContext, RunContext }};
use ink_host::{{ create_logger, logger_log }};
use async_host::{{ launch_future, sleep, LaunchedFutureHandle }};

async fn victim(cx: RunContext, log: opaque) -> nil {{
    let a = DropTag.new(log, "drop:a");
    let b = DropTag.new(log, "drop:b");
    await sleep(60);
    logger_log(log, 2, "unreachable");
}}

async fn killer(cx: RunContext, log: opaque, h: LaunchedFutureHandle<nil>) -> nil {{
    await sleep(30);
    let ok = h.abort();
    if (ok) {{ logger_log(log, 2, "killer:aborted"); }} else {{ logger_log(log, 2, "killer:late"); }}
}}

pub fn main() -> nil {{
    let log = create_logger("t");
    let v = launch_future(victim(log));
    launch_future(killer(log, v));
}}
"#
    );
    let (mut vm, sink) = setup(&src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 200);
    // `a` is declared before `b`, so at the resume arm's drop path `b`
    // releases first (reverse binding order) and the LIFO drain runs its
    // dispose first — after the victim crossed its park
    let lines = sink.borrow().clone();
    let pa = lines.iter().position(|l| l == "drop:a").expect("drop:a ran");
    let pb = lines.iter().position(|l| l == "drop:b").expect("drop:b ran");
    assert!(pb < pa, "reverse declaration order: b before a, got {lines:?}");
    assert!(!lines.iter().any(|l| l == "unreachable"));
    assert!(lines.iter().any(|l| l == "killer:aborted"));
}

// ---- a fresh (never driven) task aborts before its body runs ----

#[test]
fn abort_before_first_drive_never_runs_the_body() {
    let src = r#"
use core::{ RunContext };
use ink_host::{ create_logger, logger_log };
use async_host::launch_future;

async fn job(cx: RunContext, log: opaque) -> nil {
    logger_log(log, 2, "body-ran");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(job(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    // the launched frame sits in the queue; cancel it BEFORE any drive
    let task = vm_first_ready(&mut vm);
    assert!(vm.cancel(task), "abort on a fresh task answers true");
    vm.run_ready().expect("drive the flagged task");
    assert!(sink.borrow().is_empty(), "the s0 probe saw the flag — the body never ran");
}

/// The queued task slot — the test-side stand-in for the receipt's
/// frame edge (the VM's queue owns the launched frame).
fn vm_first_ready(vm: &mut Vm) -> rut_vm::Slot {
    vm.first_ready()
}

// ---- the cx protocol read from a body ----

#[test]
fn the_cx_cancelled_probe_answers_in_a_live_body() {
    let src = r#"
use core::{ RunContext };
use ink_host::{ create_logger, logger_log };
use async_host::launch_future;

async fn work(cx: RunContext, log: opaque) -> nil {
    if (cx.cancelled()) { logger_log(log, 2, "flagged"); } else { logger_log(log, 2, "live"); }
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(work(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["live"]);
}

// ---- the sealed mint recovers as a live Future ----

// The mint crosses SEALED (`__sleep -> opaque`): the rut face's
// `opaque.downcast<Future<nil>>` recovers it, and the recovered frame
// drives, parks, resumes, and aborts like any other.

#[test]
fn a_bound_sleep_future_drives_and_aborts_through_the_box() {
    let src = r#"
use core::{ Future, RunContext };
use ink_host::{ create_logger, logger_log };
use async_host::{ launch_future, sleep, LaunchedFutureHandle };

async fn parker(cx: RunContext, log: opaque) -> nil {
    // the annotated binding: the downcast's `?Future<nil>` answer
    // derefs into a `Future<nil>` binding — the one-consume surface
    let s: Future<nil> = sleep(40);
    logger_log(log, 2, "parked");
    await s;
    logger_log(log, 2, "unreachable");
}

async fn killer(cx: RunContext, log: opaque, h: LaunchedFutureHandle<nil>) -> nil {
    await sleep(10);
    let ok = h.abort();
    if (ok) { logger_log(log, 2, "aborted-the-parked"); }
}

pub fn main() -> nil {
    let log = create_logger("t");
    let h = launch_future(parker(log));
    launch_future(killer(log, h));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 200);
    assert_eq!(
        *sink.borrow(),
        vec!["parked", "aborted-the-parked"],
        "the recovered sleep future parked; the abort fired before the 40ms wake"
    );
    // the clock's final position is the victim's armed 40ms deadline:
    // the abort retired the frame at 10ms, but the park's armed timer
    // still owns its map entry — the wake fires and drives a retired
    // frame (a no-op), exactly the abort_after_park law
    assert_eq!(vm.now_ms(), 40);
}

// ---- fuel accounting per drive step ----

#[test]
fn fuel_is_charged_per_drive_step() {
    let src = r#"
use core::{ RunContext };
use ink_host::{ create_logger, logger_log };
use async_host::launch_future;

async fn work(cx: RunContext, log: opaque) -> nil {
    logger_log(log, 2, "ran");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(work(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    let before = vm.fuel_used;
    vm.run_ready().expect("the drive");
    let after = vm.fuel_used;
    assert!(after > before, "a drive step charges fuel: {before} -> {after}");
    assert_eq!(*sink.borrow(), vec!["ran"]);
}
