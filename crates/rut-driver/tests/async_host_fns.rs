// The host future lane (phase 4): `pub host async fn` weaves its call
// sites into COLD engine-woven Future frames whose state field holds a
// HOST cell — the Completer box `__start` answers. Every fixture
// future here registers through `rut_vm::register_async!` (the ONE
// closure law) and drives on the virtual clock: await ordering,
// launch-driven concurrency, the fail path's trap at the await, the
// cancel arm (the cx `cancelled()` data path) with late results
// discarded, and the direct row marshal (`__start`/`__yield`/`__take`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rut_driver::{Module, ModuleBody, Session, compile_graph, lower_decl_module, mount_std_async, mount_std_core};
use rut_std::async_host::install_std_async;
use rut_std::logger::install_std_log;
use rut_vm::interp::{HostHooks, HostRegistry, Limits, Vm};
use rut_vm::Completer;

const RT_DECL: &str = include_str!("../../../rut/rt/rt.d.rut");

// The fixture surface: three async rows whose virtual behavior is
// spelled in the argument — `key:ms` completes at +ms (`probe`),
// fails at +ms with `kaboom:key` (`boom`), or never settles on its
// own (`hang`, the cancel lane's victim).
const FIXTURE_DECL: &str = "\
pub host async fn probe(u: str) -> str;
pub host async fn boom(u: str) -> str;
pub host async fn hang(u: str) -> str;
";

#[derive(Default)]
struct Fixture {
    /// probe: (deadline, key, completer)
    dues: RefCell<Vec<(u64, String, Completer<String>)>>,
    /// boom: (deadline, message, completer)
    fail_dues: RefCell<Vec<(u64, String, Completer<String>)>>,
    /// hang: no deadline — settles only when the test says so
    hung: RefCell<Vec<Completer<String>>>,
    /// the cancel arm's receipts
    cancels: Cell<usize>,
}

fn split_arg(u: &str) -> (String, u64) {
    match u.rsplit_once(':') {
        Some((k, ms)) => (k.to_string(), ms.parse().unwrap_or(0)),
        None => (u.to_string(), 0),
    }
}

impl Fixture {
    fn probe_start(&self, u: String) -> Completer<String> {
        let (key, ms) = split_arg(&u);
        let c = Completer::new();
        self.dues.borrow_mut().push((ms, key, c.clone()));
        c
    }
    fn boom_start(&self, u: String) -> Completer<String> {
        let (key, ms) = split_arg(&u);
        let c = Completer::new();
        let msg = format!("kaboom:{key}");
        self.fail_dues.borrow_mut().push((ms, msg, c.clone()));
        c
    }
    fn hang_start(&self, _u: String) -> Completer<String> {
        let c = Completer::new();
        self.hung.borrow_mut().push(c.clone());
        c
    }
    fn bump_cancel(&self) {
        self.cancels.set(self.cancels.get() + 1);
    }
    fn cancel_count(&self) -> usize {
        self.cancels.get()
    }
    fn complete_due(&self, now: u64) -> bool {
        let mut settled = false;
        {
            let mut dues = self.dues.borrow_mut();
            for (d, key, c) in dues.iter() {
                if *d <= now {
                    c.complete(format!("done:{key}"));
                    settled = true;
                }
            }
            dues.retain(|(d, _, _)| *d > now);
        }
        {
            let mut fds = self.fail_dues.borrow_mut();
            for (d, msg, c) in fds.iter() {
                if *d <= now {
                    c.fail(msg.clone());
                    settled = true;
                }
            }
            fds.retain(|(d, _, _)| *d > now);
        }
        settled
    }
    fn next_due(&self) -> Option<u64> {
        self.dues
            .borrow()
            .iter()
            .map(|(d, _, _)| *d)
            .chain(self.fail_dues.borrow().iter().map(|(d, _, _)| *d))
            .min()
    }
    fn has_pending(&self) -> bool {
        self.next_due().is_some()
    }
    fn settle_hung(&self, v: &str) {
        for c in self.hung.borrow().iter() {
            c.complete(v.to_string());
        }
        self.hung.borrow_mut().clear();
    }
}

/// Mount the world, register the fixture rows through `register_async!`
/// (both macro forms — `hang` carries the abort closure), and pass the
/// boot join: `expected_host_fns` must have EXPANDED the async
/// rows into their families, or the verify panics here.
fn setup(src: &str) -> (Vm, Rc<RefCell<Vec<String>>>, Rc<Fixture>) {
    let mut s = Session::new();
    mount_std_core(&mut s);
    let mut rt = lower_decl_module(RT_DECL, "rt.d.rut").expect("the rt surface is valid");
    rt.host_scope = Some("rt:log".to_string());
    s.register_module("rt", rt).expect("mount rt");
    mount_std_async(&mut s);
    let fixture =
        lower_decl_module(FIXTURE_DECL, "fixture.d.rut").expect("the fixture surface is valid");
    s.register_module("fixture", fixture).expect("mount fixture");
    s.register_module("app", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() })
        .expect("register app");
    let out = compile_graph(&s, "app");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let sink = Rc::new(RefCell::new(Vec::<String>::new()));
    let mut hosts = HostRegistry::new();
    let sink2 = sink.clone();
    install_std_log(&mut hosts, move |m| sink2.borrow_mut().push(m.to_string()));
    install_std_async(&mut hosts);
    let fx = Rc::new(Fixture::default());
    let fx2 = fx.clone();
    rut_vm::register_async!(hosts, "fixture::probe", (String,) -> String,
        move |u: String| -> Completer<String> { fx2.probe_start(u) });
    let fx3 = fx.clone();
    rut_vm::register_async!(hosts, "fixture::boom", (String,) -> String,
        move |u: String| -> Completer<String> { fx3.boom_start(u) });
    let fx4 = fx.clone();
    let fx5 = fx.clone();
    rut_vm::register_async!(hosts, "fixture::hang", (String,) -> String,
        move |u: String| -> Completer<String> { fx4.hang_start(u) },
        move |_c: Completer<String>| { fx5.bump_cancel() });
    // the boot join over the EXPANDED expectations — the decl
    // grammar spells one row, the embedder binds five bodies
    hosts.verify_against(&s.expected_host_fns());
    let limits = Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let vm = Vm::new(Rc::new(flat), &limits, HostHooks::default(), hosts).expect("vm");
    (vm, sink, fx)
}

/// The host loop over the virtual clock: drain
/// the ready queue, settle the fixture's due completions, advance to
/// the next due/deadline. Capped, so a stalled loop fails instead of
/// hanging.
fn run_loop(vm: &mut Vm, fx: &Fixture, cap: usize) {
    for _ in 0..cap {
        vm.run_ready().expect("run_ready");
        if vm.pending_tasks() == 0 && !fx.has_pending() {
            return;
        }
        // settle anything due at the current clock; a settle spins the
        // poll without advancing (the next run_ready sees it)
        if fx.complete_due(vm.now_ms()) {
            continue;
        }
        let next = [fx.next_due(), vm.next_deadline()]
            .into_iter()
            .flatten()
            .filter(|&d| d > vm.now_ms())
            .min();
        match next {
            Some(d) => vm.set_now(d),
            None => vm.set_now(vm.now_ms() + 1),
        }
    }
    panic!("the driving loop stalled: {} task(s) pending", vm.pending_tasks());
}

// ---- await ordering ----

#[test]
fn await_orders_after_completion() {
    let src = r#"
use core::{ RunContext };
use rt::{ create_logger, logger_log };
use async_host::launch_future;
use fixture::probe;

async fn job(cx: RunContext, log: opaque, u: str) -> nil {
    logger_log(log, 2, f"before:{u}");
    await probe(u);
    logger_log(log, 2, f"after:{u}");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(job(log, "a:10"));
}
"#;
    let (mut vm, sink, fx) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["before:a:10", "after:a:10"],
        "the await parked until the completer settled"
    );
    assert_eq!(vm.now_ms(), 10, "the clock advanced to the fixture's due");
}

// ---- launch-driven concurrency ----

#[test]
fn two_awaits_run_concurrently_and_settle_by_deadline() {
    let src = r#"
use core::{ RunContext };
use rt::{ create_logger, logger_log };
use async_host::launch_future;
use fixture::probe;

async fn job(cx: RunContext, log: opaque, u: str) -> nil {
    logger_log(log, 2, f"before:{u}");
    await probe(u);
    logger_log(log, 2, f"done:{u}");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(job(log, "b:5"));
    launch_future(job(log, "a:20"));
}
"#;
    let (mut vm, sink, fx) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["before:b:5", "before:a:20", "done:b:5", "done:a:20"],
        "both bodies launched before either settled; completions land by deadline"
    );
    assert_eq!(vm.now_ms(), 20);
}

// ---- fire and forget: a launched host future settles, answer discarded ----

#[test]
fn a_launched_host_future_settles_without_an_awaiter() {
    let src = r#"
use rt::create_logger;
use async_host::launch_future;
use fixture::probe;

pub fn main() -> nil {
    let _log = create_logger("t");
    launch_future(probe("ff:15"));
}
"#;
    let (mut vm, _sink, fx) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, &fx, 100);
    assert_eq!(vm.now_ms(), 15, "the launched frame drove to its completion");
    assert_eq!(vm.pending_tasks(), 0, "the retired frame left the poll set");
}

// ---- the fail path: the completer's message traps at the await ----

#[test]
fn a_failed_completer_traps_with_its_message() {
    let src = r#"
use core::{ RunContext };
use rt::{ create_logger, logger_log };
use async_host::launch_future;
use fixture::boom;

async fn victim(cx: RunContext, log: opaque, u: str) -> nil {
    await boom(u);
    logger_log(log, 2, "unreachable");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(victim(log, "x:7"));
}
"#;
    let (mut vm, sink, fx) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    let mut trapped = None;
    for _ in 0..100 {
        match vm.run_ready() {
            Err(t) => {
                trapped = Some(t);
                break;
            }
            Ok(_) => {}
        }
        fx.complete_due(vm.now_ms());
        match fx.next_due() {
            Some(d) if d > vm.now_ms() => vm.set_now(d),
            _ => vm.set_now(vm.now_ms() + 1),
        }
    }
    let t = trapped.expect("the failed completer traps the loop");
    assert!(t.msg.contains("kaboom:x"), "the fail message rides the trap: {}", t.msg);
    assert!(
        !sink.borrow().iter().any(|l| l == "unreachable"),
        "the await never resumes past a failure"
    );
}

// ---- cancellation: the cx data path maps to the cancel arm ----

#[test]
fn cancel_maps_the_data_path_to_the_arm_and_discards_late_results() {
    let src = r#"
use core::{ RunContext };
use rt::{ create_logger, logger_log };
use async_host::{ launch_future, sleep, LaunchedFutureHandle };
use fixture::hang;

async fn victim(cx: RunContext, log: opaque) -> nil {
    let buf: ?str = "held";
    await hang("x");
    logger_log(log, 2, "unreachable");
}

async fn killer(cx: RunContext, log: opaque, h: LaunchedFutureHandle<nil>) -> nil {
    await sleep(10);
    let ok = h.abort();
    if (ok) { logger_log(log, 2, "killer:aborted"); }
}

pub fn main() -> nil {
    let log = create_logger("t");
    let v = launch_future(victim(log));
    launch_future(killer(log, v));
}
"#;
    let (mut vm, sink, fx) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    // drive until the task abort has retired the victim: the ready
    // queue and the timers drain, and only the hung host future stays
    // pending on its Completer
    for _ in 0..200 {
        vm.run_ready().expect("run_ready");
        if vm.pending_tasks() == 1 {
            break;
        }
        match vm.next_deadline() {
            Some(d) if d > vm.now_ms() => vm.set_now(d),
            _ => vm.set_now(vm.now_ms() + 1),
        }
    }
    assert!(
        sink.borrow().iter().any(|l| l == "killer:aborted"),
        "the killer ran: {:?}",
        sink.borrow()
    );
    assert_eq!(
        vm.pending_tasks(),
        1,
        "the victim retired; the hung host future is the one still parked"
    );
    // the task is gone; the host frame is the poll set's business —
    // cancel it through the cx data path and the arm fires
    let frame = vm.first_host_pending().expect("the hung future is reachable");
    assert!(vm.cancel(frame), "a parked host future cancels");
    vm.run_ready().expect("the cancel spin drives the launched frame");
    assert_eq!(fx.cancel_count(), 1, "the abort closure fired exactly once");
    vm.run_ready().expect("the next spin retires the poll set's entry");
    assert_eq!(vm.pending_tasks(), 0, "the cancelled host frame left the set");
    // the late result: the worker completes after the cancel — the
    // disclosed best-effort law says it is discarded, never trapped
    fx.settle_hung("late");
    vm.run_ready().expect("the late settle spins clean");
    assert_eq!(vm.pending_tasks(), 0);
    assert!(!sink.borrow().iter().any(|l| l == "unreachable"));
}

// ---- the ANSWER lane: await lifts the completed value ----

#[test]
fn await_delivers_the_host_answer() {
    let src = r#"
use core::{ RunContext };
use rt::{ create_logger, logger_log };
use async_host::launch_future;
use fixture::probe;

async fn job(cx: RunContext, log: opaque, u: str) -> nil {
    let v = await probe(u);
    logger_log(log, 2, f"got:{v}");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(job(log, "k:10"));
}
"#;
    let (mut vm, sink, fx) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["got:done:k"],
        "the await lifted the completed frame's answer"
    );
}

#[test]
fn await_delivers_through_user_frames_and_type_checks() {
    // the rgh shape: a user async fn awaits another user async fn whose
    // body awaits the HOST row — the value crosses two answer lanes
    let src = r#"
use core::{ RunContext };
use rt::{ create_logger, logger_log };
use async_host::launch_future;
use fixture::probe;

async fn inner(cx: RunContext, log: opaque, u: str) -> str {
    logger_log(log, 2, "inner:start");
    let v = await probe(u);
    logger_log(log, 2, "inner:got");
    return v;
}

async fn outer(cx: RunContext, log: opaque, u: str) -> str {
    logger_log(log, 2, "outer:start");
    let v = await inner(log, u);
    logger_log(log, 2, f"outer:{v}");
    return v;
}

async fn job(cx: RunContext, log: opaque, u: str) -> nil {
    logger_log(log, 2, "job:start");
    let v = await outer(log, u);
    logger_log(log, 2, f"job:{v}");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(job(log, "n:5"));
}
"#;
    let (mut vm, sink, fx) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["job:start", "outer:start", "inner:start", "inner:got", "outer:done:n", "job:done:n"],
        "the answer crossed the user frame's answer lane, then the outer's"
    );
}

// ---- the rows, driven directly: the marshal lane end to end ----

#[test]
fn the_rows_drive_directly_through_call_host_row() {
    let src = r#"
use core::{ RunContext };
use async_host::launch_future;
use fixture::probe;

async fn job(cx: RunContext, u: str) -> nil {
    await probe(u);
}

pub fn main() -> nil {
    launch_future(job("direct:25"));
}
"#;
    use rut_vm::Value;
    let (mut vm, _sink, fx) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    // a fresh state cell, minted straight through the start row
    let cell = vm
        .call_host_row("fixture::probe__start", &[Value::Str("k:0".into())])
        .expect("start answers the state cell");
    let Value::Opaque(cell) = cell else {
        panic!("the start row answers the state cell box, got {cell:?}");
    };
    // the resumption probe answers pending before the settle...
    let p = vm
        .call_host_row(
            "fixture::probe__yield",
            &[Value::Opaque(cell.clone()), Value::Opaque(cell.clone())],
        )
        .expect("yield probes");
    assert_eq!(p, Value::I64(rut_vm::PENDING as i64));
    // ...and a take before the settle is the misuse trap
    let early = vm.call_host_row("fixture::probe__take", &[Value::Opaque(cell.clone())]);
    assert!(
        early.err().map(|t| t.msg.contains("still pending")).unwrap_or(false),
        "a pending take traps"
    );
    // settle through the fixture's due lane (deadline 0)
    assert!(fx.complete_due(0), "the fixture settled");
    // the probe flips to ready, the take marshals ON the VM thread
    let p = vm
        .call_host_row(
            "fixture::probe__yield",
            &[Value::Opaque(cell.clone()), Value::Opaque(cell.clone())],
        )
        .expect("yield probes");
    assert_eq!(p, Value::I64(rut_vm::READY as i64));
    let ans = vm
        .call_host_row("fixture::probe__take", &[Value::Opaque(cell.clone())])
        .expect("take");
    assert_eq!(ans, Value::Str("done:k".into()), "the answer crossed through the phase-1 return lane");
    // the decl row itself never runs — the weave mints the future
    let err = vm
        .call_host_row("fixture::probe", &[Value::Str("k".into())])
        .err()
        .expect("the decl row traps");
    assert!(
        err.msg.contains("called through its decl row"),
        "the decl row is the teaching trap: {}",
        err.msg
    );
}

// ---- the thread law: a REAL worker completes across the boundary ----

#[test]
fn a_worker_thread_completes_and_the_poll_lane_drives_it() {
    // the thread row needs its own surface+registration, so build the
    // session by hand instead of `setup`. Its start closure spawns
    // std::thread — `std::thread` lives in the EMBEDDER closure only;
    // rut-vm touches atomics + the Mutex slot.
    let mut s = Session::new();
    mount_std_core(&mut s);
    let mut rt = lower_decl_module(RT_DECL, "rt.d.rut").expect("rt");
    rt.host_scope = Some("rt:log".to_string());
    s.register_module("rt", rt).expect("mount rt");
    mount_std_async(&mut s);
    let fixture = lower_decl_module(
        &format!("{FIXTURE_DECL}pub host async fn wall(u: str) -> str;\n"),
        "fixture.d.rut",
    )
    .expect("fixture");
    s.register_module("fixture", fixture).expect("mount fixture");
    let app = r#"
use core::{ RunContext };
use rt::{ create_logger, logger_log };
use async_host::launch_future;
use fixture::wall;

async fn job(cx: RunContext, log: opaque, u: str) -> nil {
    logger_log(log, 2, f"wall:before:{u}");
    await wall(u);
    logger_log(log, 2, f"wall:after:{u}");
}

pub fn main() -> nil {
    let log = create_logger("t");
    launch_future(job(log, "t"));
}
"#;
    s.register_module("app", Module { body: ModuleBody::Source { text: app.into(), is_decl: false }, ..Default::default() })
        .expect("register app");
    let out = compile_graph(&s, "app");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let sink = Rc::new(RefCell::new(Vec::<String>::new()));
    let mut hosts = HostRegistry::new();
    let sink2 = sink.clone();
    install_std_log(&mut hosts, move |m| sink2.borrow_mut().push(m.to_string()));
    install_std_async(&mut hosts);
    let fx = Rc::new(Fixture::default());
    let fx2 = fx.clone();
    rut_vm::register_async!(hosts, "fixture::probe", (String,) -> String,
        move |u: String| -> Completer<String> { fx2.probe_start(u) });
    let fx3 = fx.clone();
    rut_vm::register_async!(hosts, "fixture::boom", (String,) -> String,
        move |u: String| -> Completer<String> { fx3.boom_start(u) });
    let fx4 = fx.clone();
    rut_vm::register_async!(hosts, "fixture::hang", (String,) -> String,
        move |u: String| -> Completer<String> { fx4.hang_start(u) });
    rut_vm::register_async!(hosts, "fixture::wall", (String,) -> String,
        move |u: String| -> Completer<String> {
            let c = Completer::new();
            let w = c.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(30));
                w.complete(format!("walled:{u}"));
            });
            c
        });
    hosts.verify_against(&s.expected_host_fns());
    let limits = Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = Vm::new(Rc::new(flat), &limits, HostHooks::default(), hosts).expect("vm");
    vm.call::<_, ()>("main", ()).expect("main");
    // wall-clock polling: the embedder loop spins; the worker thread
    // settles the completer from another thread
    for _ in 0..200 {
        vm.run_ready().expect("run_ready");
        if sink.borrow().iter().any(|l| l == "wall:after:t") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        *sink.borrow(),
        vec!["wall:before:t", "wall:after:t"],
        "the worker thread's completion woke the await through the poll lane"
    );
}
