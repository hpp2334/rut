// The structured-competition surface (the select/completer landing):
// `select2` / `select_all` (first-ready over engine-minted race frames;
// the losers cancelled by library law) and `completer` (the
// manually-resolvable future primitive). Runs observe through the ink
// logger's recording native — the async_basics pattern; every race
// drives through the standard host loop (ready queue + virtual clock).

use std::cell::RefCell;
use std::rc::Rc;


use rut_vm::interp::{HostRegistry, Limits, Vm};




/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Pkg::source(spec, src))
            .pkg(rut_driver::calc_pkg())
            .entrypoint(spec)
            .compile(),
    )
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

fn setup(src: &str) -> (Vm, Rc<RefCell<Vec<String>>>) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut loaded = rut_driver::dir_pkgs(&root.join("rut/ink")).expect("mount ink");
    loaded.pkgs.extend(rut_driver::std_async_pkgs().expect("the async pair walks"));
    loaded.pkgs.push(rut_driver::Pkg::source("app", src));
    let expected = rut_driver::declared_host_fns(&loaded.pkgs);
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint("app")
        .compile()
        .unwrap();
    assert!(compiled.graph.diags.is_empty(), "diags: {:?}", compiled.graph.diags);
    let prog = compiled.graph.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let sink = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink2 = sink.clone();
    let ctx = rut_driver::host_pkg_ctx(&loaded.pkgs);
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())));
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    hosts.verify_against(&expected);
    let limits = Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let vm = Vm::builder().program(Rc::new(flat)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(hosts).build()
        .expect("vm");
    (vm, sink)
}

/// The host loop: drain the ready queue, wake timers by advancing the
/// virtual clock. Capped, so a stalled loop fails instead of hanging.
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

// ---- select2 ----

#[test]
fn select2_answers_the_first_ready() {
    let src = r#"
use ink_host::{ create_logger, logger_log };
use futures::{ launch_future, sleep, select2, Either2 };

async fn slow(log: opaque) -> str {
    await sleep(500);
    return "slow";
}

async fn quick(log: opaque) -> str {
    await sleep(5);
    return "quick";
}

async fn raced(log: opaque) -> nil {
    let winner = await select2(slow(log), quick(log));
    if (winner.is_a()) {
        logger_log(log, 2, f"slow: {winner.a_value()}");
    } else {
        logger_log(log, 2, f"quick: {winner.b_value()}");
    }
}

entry fn main() -> nil {
    let log = create_logger("t");
    launch_future(raced(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["quick: quick"]);
    // the race answered at the winner's deadline; the loser's far timer
    // drains afterwards, so the clock ends at the loser's deadline
    assert_eq!(vm.now_ms(), 500);
}

#[test]
fn select2_types_the_two_sides_distinctly() {
    // `select2<str, nil>`: the timeout side answers nil — the winner's
    // type is the fn signature's Either2<T, U>, no casts anywhere.
    let src = r#"
use ink_host::{ create_logger, logger_log };
use futures::{ launch_future, sleep, select2, Either2 };

async fn fetch(log: opaque) -> str {
    await sleep(500);
    return "payload";
}

async fn fetch_or_timeout(log: opaque) -> nil {
    let winner = await select2(fetch(log), sleep(50));
    if (winner.is_a()) {
        logger_log(log, 2, f"got {winner.a_value()}");
    } else {
        logger_log(log, 2, "timeout");
    }
}

entry fn main() -> nil {
    let log = create_logger("t");
    launch_future(fetch_or_timeout(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["timeout"]);
}

#[test]
fn select2_cancels_the_loser_through_its_drop_path() {
    // the loser's frame is cancelled at its checkpoint: its locals
    // release in reverse binding order, the Disposal hook fires, and
    // the loser's continuation NEVER runs.
    let src = r#"
use core::{ DisposalContext };
use ink::{ Logger };
use ink_host::{ create_logger, logger_log };
use futures::{ launch_future, sleep, select2, Either2 };

class Tracked {
    n: i32 = 0;
}
impl Tracked {
    [disposal] fn dispose_tracked(mut self, cx: DisposalContext) {
        Logger.new("t").info(f"gone {self.n}");
    }
}

async fn loser(log: opaque, n: i32) -> nil {
    let t = Tracked { n: n };
    await sleep(60_000);
    logger_log(log, 2, "loser finished");   // must never run
}

async fn winner(log: opaque) -> nil {
    await sleep(5);
    logger_log(log, 2, "won");
}

async fn raced(log: opaque) -> nil {
    let w = await select2(loser(log, 7), winner(log));
    logger_log(log, 2, "raced done");
}

entry fn main() -> nil {
    let log = create_logger("t");
    launch_future(raced(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    // the loser's drop fires (its cancelled probe runs the Disposal
    // hook), the race continues, and the loser's own continuation is
    // never reached. The drop's position relative to the continuation
    // is queue timing, not a law — presence is.
    let lines = sink.borrow().clone();
    assert!(lines.contains(&"gone 7".to_string()), "lines: {lines:?}");
    assert!(lines.contains(&"won".to_string()), "lines: {lines:?}");
    assert!(lines.contains(&"raced done".to_string()), "lines: {lines:?}");
    assert!(!lines.contains(&"loser finished".to_string()), "lines: {lines:?}");
}

// ---- select_all ----

#[test]
fn select_all_answers_the_winner_index_and_value() {
    let src = r#"
use core::{ Future, DisposalContext };
use ink::{ Logger };
use ink_host::{ create_logger, logger_log };
use futures::{ launch_future, sleep, select_all };

class Tracked {
    tag: str;
}
impl Tracked {
    [disposal] fn dispose_tracked(mut self, cx: DisposalContext) {
        Logger.new("t").info(f"cancelled {self.tag}");
    }
}

async fn worker(log: opaque, ms: u32, tag: str) -> str {
    let t = Tracked { tag: tag };
    await sleep(ms);
    return tag;
}

async fn raced(log: opaque) -> nil {
    let workers: [Future<str>] = [worker(log, 300, "a"), worker(log, 10, "b"), worker(log, 200, "c")];
    let (i, v) = await select_all(workers);
    logger_log(log, 2, f"winner {i}:{v}");
}

entry fn main() -> nil {
    let log = create_logger("t");
    launch_future(raced(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    // the winner's index is 1 (worker "b") and its value rides the
    // tuple; the two losers' drop paths fire. The cancel order relative
    // to the woken continuation is queue timing, not a law.
    let lines = sink.borrow().clone();
    assert!(lines.contains(&"winner 1:b".to_string()), "lines: {lines:?}");
    assert!(lines.contains(&"cancelled a".to_string()), "lines: {lines:?}");
    assert!(lines.contains(&"cancelled c".to_string()), "lines: {lines:?}");
}

// ---- the completer ----

#[test]
fn completer_resolves_a_parked_awaiter() {
    // the manual mint: the awaiter parks on the cold future; the
    // resolution right settles it from "callback" side; stability on a
    // second resolve (false — already retired).
    let src = r#"
use core::{ Future };
use ink_host::{ create_logger, logger_log };
use futures::{ launch_future, sleep, completer, Completer };

async fn waiter(log: opaque, f: Future<str>) -> nil {
    let v = await f;
    logger_log(log, 2, f"got {v}");
}

async fn settle_later(log: opaque, done: Completer<str>) -> nil {
    await sleep(10);
    let first = done.resolve("settled");
    let second = done.resolve("again");
    logger_log(log, 2, f"resolve {first} then {second}");
}

entry fn main() -> nil {
    let log = create_logger("t");
    let (f, done) = completer<str>();
    launch_future(waiter(log, f));
    launch_future(settle_later(log, done));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["resolve true then false", "got settled"]);
}

#[test]
fn completer_answers_before_the_await() {
    // already-done at the probe: the await falls straight through
    let src = r#"
use core::{ Future };
use ink_host::{ create_logger, logger_log };
use futures::{ launch_future, completer };

async fn waiter(log: opaque, f: Future<u32>, done: Completer<u32>) -> nil {
    done.resolve(41);
    let v = await f;
    logger_log(log, 2, f"answer {v}");
}

entry fn main() -> nil {
    let log = create_logger("t");
    let (f, done) = completer<u32>();
    launch_future(waiter(log, f, done));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["answer 41"]);
}

#[test]
fn completer_feeds_a_select_race() {
    // the callback→async composition: a PLAIN fn returning a Future
    // (no async block), raced against a sleep through select2.
    let src = r#"
use core::{ Future };
use ink_host::{ create_logger, logger_log };
use futures::{ launch_future, sleep, select2, completer, Completer, Either2 };

// the callback side: settle the future when the "response" arrives
fn fake_fetch(tag: str) -> Future<str> {
    let (f, done) = completer<str>();
    launch_future(settle_later(done, tag));
    return f;
}

async fn settle_later(done: Completer<str>, tag: str) -> nil {
    await sleep(10);
    done.resolve(tag);
}

async fn fetch_or_timeout(log: opaque) -> nil {
    let winner = await select2(fake_fetch("payload"), sleep(5_000));
    if (winner.is_a()) {
        logger_log(log, 2, f"got {winner.a_value()}");
    } else {
        logger_log(log, 2, "timeout");
    }
}

entry fn main() -> nil {
    let log = create_logger("t");
    launch_future(fetch_or_timeout(log));
}
"#;
    let (mut vm, sink) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    run_loop(&mut vm, 100);
    assert_eq!(*sink.borrow(), vec!["got payload"]);
}
