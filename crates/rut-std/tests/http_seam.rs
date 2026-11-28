//! The seam suite: the embedder's own-backend lane over `pkg_over`.
//! A fake `HttpBackend` — in this file, the crate's test home being
//! `tests/` (the same shelf the fixture suite lives on, no
//! `#[cfg(test)]` tree) — drives the seven `http_host` rows through
//! the rut face: headers-then-chunks pumped from a real worker
//! thread, the immediate transport failure answering on the channel
//! (`(nil, SendError)` — the status field's 0 is never the face's
//! signal), the nil-use misuse pin (the marked use on the nil resp
//! side traps `NilDeref` at the `!`), the
//! one-completion law pinned EXACTLY as the `Completer` contract
//! already behaves (a double complete is LAST-WRITE-WINS — the second
//! answer overwrites the slot, the first is silently lost; never a
//! trap, never an error state), and the BUDGET: a flood backend whose
//! queue plateaus at the budget while nobody reads (real flow
//! control) and whose drain resumes when the consumer catches up.
//!
//! NO feature gate, unlike `http.rs`: the seam is feature-free (no
//! reqwest — that's the point), so every plain `cargo nextest run`
//! exercises it. The http tests mount `rut/http` as a TREE package
//! (`../../rut`) — no extension/demo lane needed; no frontend surface
//! moved (judgment recorded: the wire face and the wasm lane are
//! untouched by the seam).

use rut_driver::Mounted;
use rut_std::http::{pkg_over, HttpBackend, HttpBodyWriter, HttpRequest, HttpResponse};
use rut_std::{async_host, logger, strbuild};
use rut_vm::interp::Vm;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// this test's host pkgs — the mounted decl surface the bodies must
/// match (the tree packages, never a copied shim)
const PKG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut");

/// the probe: the FACE (the `rut/http` classes), driven as an async
/// free fn over the launcher — send at headers, the readbacks, the
/// body as the drain
const SRC: &str = r#"
use http::HttpClient;
use http_host::net::{ http_send, http_read_err, http_resp_stream };
use http_host::stream::{ http_body, http_stream_next };
use futures::{ launch_future, sleep, select2, LaunchedFutureHandle, Either2 };
use ink_host::{ create_logger, logger_log };

async fn probe(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let (resp, err) = await client.post(url).body("hi".encode()).build().send();
    if (err != nil) {
        let t: str = err!.reason;
        logger_log(log, 2, f"err={t}");
        let gone: bool = resp == nil;
        logger_log(log, 2, f"resp-nil={gone}");
    } else {
        logger_log(log, 2, f"status={resp!.status()}");
        let rerr = resp!.read_error();
        if (rerr == nil) {
            logger_log(log, 2, "read-err=nil");
        } else {
            let t: str = rerr!;
            logger_log(log, 2, f"read-err={t}");
        }
        let b = await resp!.body();
        logger_log(log, 2, f"body={b.len()}");
    }
}

async fn streamed(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let (resp, err) = await client.get(url).build().send();
    if (err != nil) {
        let t: str = err!.reason;
        logger_log(log, 2, f"stream-err={t}");
    } else {
        let s = resp!.byte_stream();
        let total: i32 = 0;
        let failed = false;
        while (true) {
            let c = await s.next();
            if (c == nil) {
                let e = s.error();
                if (e != nil) { failed = true; }
                break;
            }
            let chunk: bytes = c!;
            total += chunk.len();
        }
        logger_log(log, 2, f"stream-total={total}");
        logger_log(log, 2, f"stream-failed={failed}");
    }
}

entry fn boot(url: str) -> nil {
    let log = create_logger("t");
    launch_future(probe(log, url));
}

entry fn boot_stream(url: str) -> nil {
    let log = create_logger("t");
    launch_future(streamed(log, url));
}

// the flood probes: sleep FIRST so the test — not the flood race —
// decides when the reads begin (the VM clock is virtual; the test
// fires the sleep after observing the producer's plateau)
async fn flooded(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let (resp, err) = await client.get(url).build().send();
    if (err != nil) {
        let t: str = err!.reason;
        logger_log(log, 2, f"stream-err={t}");
    } else {
        await sleep(300);
        let s = resp!.byte_stream();
        let total: i32 = 0;
        while (true) {
            let c = await s.next();
            if (c == nil) {
                break;
            }
            let chunk: bytes = c!;
            total += chunk.len();
        }
        logger_log(log, 2, f"stream-total={total}");
    }
}

async fn drained(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let (resp, err) = await client.get(url).build().send();
    if (err != nil) {
        let t: str = err!.reason;
        logger_log(log, 2, f"body-err={t}");
    } else {
        await sleep(300);
        let b = await resp!.body();
        logger_log(log, 2, f"body={b.len()}");
    }
}

entry fn boot_flood(url: str) -> nil {
    let log = create_logger("t");
    launch_future(flooded(log, url));
}

entry fn boot_drain(url: str) -> nil {
    let log = create_logger("t");
    launch_future(drained(log, url));
}

// ---- the cancel probes (the wire-abort) ------------------------------
//
// The probes drive the `http_host` ROWS directly: the cancel lane's
// wire-abort fires when the HOST frame itself is cancelled — the
// futures package's module surface (a `select2` loser, or
// `LaunchedFutureHandle::abort` over the launched row future). A
// woven wrapper cancelled above its await only clears the pending
// edge (the await-park law): its host child runs to completion — the
// disclosed best-effort lane.

// A: the flood shape — the drain row future is launched and parks;
// the abort marks the source; the budget-parked pump releases, sees
// the mark at its between-chunk check and stops; the taken drain lane
// degrades every later reader (nil + the named error).
async fn flood_abort(log: opaque, url: str) -> nil {
    let r: opaque = await http_send(opaque(nil), "GET", url, "", "".encode());
    await sleep(300);
    let h = launch_future(http_body(r));
    await sleep(1);
    let ok = h.abort();
    logger_log(log, 2, f"abort={ok}");
    // the cancel arm fires at the cancelled frame's NEXT turn — the
    // checkpoint lets it land before the readbacks
    await sleep(1);
    let s: opaque = http_resp_stream(r);
    let c = await http_stream_next(s);
    if (c == nil) {
        logger_log(log, 2, "next=nil");
    } else {
        logger_log(log, 2, "next=chunk");
    }
    let re = http_read_err(r);
    if (re == nil) {
        logger_log(log, 2, "read-err=nil");
    } else {
        let t: str = re!;
        logger_log(log, 2, f"read-err={t}");
    }
}

// B: the select2 loser — the read row future parks against a trickle,
// the sleep wins the race, and the engine's loser law cancels the HOST
// frame: the row's `__cancel` arm fires, the worker stops between
// chunks, the next read degrades — never a trap.
async fn read_loser(log: opaque, url: str) -> nil {
    let r: opaque = await http_send(opaque(nil), "GET", url, "", "".encode());
    let s: opaque = http_resp_stream(r);
    let first = await http_stream_next(s);
    let n: i32 = -1;
    if (first != nil) {
        let v: bytes = first!;
        n = v.len();
    }
    logger_log(log, 2, f"first={n}");
    let w = await select2(http_stream_next(s), sleep(1));
    logger_log(log, 2, f"read-lost={w.is_b()}");
    let second = await http_stream_next(s);
    if (second == nil) {
        logger_log(log, 2, "second=nil");
    } else {
        logger_log(log, 2, "second=chunk");
    }
    let re = http_read_err(r);
    if (re == nil) {
        logger_log(log, 2, "read-err=nil");
    } else {
        let t: str = re!;
        logger_log(log, 2, f"read-err={t}");
    }
}

// C: the explicit abort — the launched drain row future is parked on
// the trickle; `LaunchedFutureHandle::abort` retires it (true) and the
// cancel arm marks the source; the taken lane degrades the late taker
// to empty and the error names the cancel.
async fn drain_abort(log: opaque, url: str) -> nil {
    let r: opaque = await http_send(opaque(nil), "GET", url, "", "".encode());
    let h = launch_future(http_body(r));
    await sleep(1);
    let ok = h.abort();
    logger_log(log, 2, f"abort={ok}");
    // the cancel arm fires at the cancelled frame's NEXT turn
    await sleep(1);
    let b = await http_body(r);
    logger_log(log, 2, f"late-body={b.len()}");
    let re = http_read_err(r);
    if (re == nil) {
        logger_log(log, 2, "read-err=nil");
    } else {
        let t: str = re!;
        logger_log(log, 2, f"read-err={t}");
    }
}

// D: the pre-headers send loser — the send row future loses the race
// before the headers arrive (no cancel slot yet): the disclosed
// best-effort law. The arm fires, finds no slot, and the walk
// continues; the late completion lands in the retired frame and is
// never taken. No trap.
async fn send_loser(log: opaque, url: str) -> nil {
    let w = await select2(http_send(opaque(nil), "GET", url, "", "".encode()), sleep(1));
    logger_log(log, 2, f"send-lost={w.is_b()}");
    await sleep(2);
    logger_log(log, 2, "done");
}

entry fn boot_flood_abort(url: str) -> nil {
    let log = create_logger("t");
    launch_future(flood_abort(log, url));
}

entry fn boot_read_loser(url: str) -> nil {
    let log = create_logger("t");
    launch_future(read_loser(log, url));
}

entry fn boot_drain_abort(url: str) -> nil {
    let log = create_logger("t");
    launch_future(drain_abort(log, url));
}

entry fn boot_send_loser(url: str) -> nil {
    let log = create_logger("t");
    launch_future(send_loser(log, url));
}
"#;

/// the misuse probe: the channel's err side bound and IGNORED, the nil
/// resp side asserted and USED — the law's loud exits. An UN-marked
/// `resp.body()` does not COMPILE (the demand diag is the first
/// loudness); the marked `resp!` on the nil side is the `NilDeref`
/// trap at the `!`'s own site.
const TRAP_SRC: &str = r#"
use http::HttpClient;
use futures::launch_future;
use ink_host::{ create_logger, logger_log };

async fn nil_use(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let (resp, err) = await client.post(url).body("hi".encode()).build().send();
    if (err != nil) {
        let t: str = err!.reason;
        logger_log(log, 2, f"err={t}");
    }
    let b = await resp!.body();
    logger_log(log, 2, f"body={b.len()}");
}

entry fn boot_nil_use(url: str) -> nil {
    let log = create_logger("t");
    launch_future(nil_use(log, url));
}
"#;

/// the fake backend's script — what `send` answers, and how the body
/// is driven afterwards
enum Script {
    /// answer at HEADERS now, then pump the chunks + clean EOF from a
    /// WORKER THREAD (the never-blocks-the-VM law exercised for real)
    Stream {
        status: u16,
        chunks: Vec<&'static str>,
    },
    /// answer at HEADERS, then FLOOD from a worker thread: `chunks`
    /// pushes of `chunk_bytes` each, one `beat_ms` pause before each
    /// push (0 = as fast as the writer accepts — the BUDGET is the
    /// only pacing). The worker checks `HttpBodyWriter::cancelled`
    /// after each completed push and stops on the mark; a beat keeps
    /// the post-release sprint from racing the test's clock advance.
    Flood {
        chunk_bytes: usize,
        chunks: usize,
        beat_ms: u64,
    },
    /// answer at HEADERS, then TRICKLE: chunk 0 now, each further
    /// chunk one `gap_ms` beat later — a consumer PARKS between
    /// arrivals. The worker checks `HttpBodyWriter::cancelled`
    /// between chunks (the reqwest pump's shared contract) and stops
    /// on the mark. The gap must outlive the test's clock advance
    /// (the abort lands inside the first beat even under load).
    Trickle {
        status: u16,
        chunks: Vec<&'static str>,
        gap_ms: u64,
    },
    /// answer at HEADERS only after `delay_ms` — the pre-headers
    /// window a `http_send` cancel rides (the disclosed best-effort)
    Late { delay_ms: u64 },
    /// answer the transport failure NOW (status 0, born-dead body)
    Dead(String),
    /// answer TWICE — the violating backend the one-completion pin
    /// rides (last-write-wins per the Completer's own contract)
    DoubleComplete,
}

/// the in-file fake `HttpBackend`: it records every request it is
/// handed (the crossing parts — the test's assertions) and answers
/// per its script
struct FakeBackend {
    script: Script,
    seen: Mutex<Vec<String>>,
    /// the flood lane's completed-push counter — a push parked on the
    /// budget hasn't landed, so the plateau/drain pins read it
    pushed: Arc<AtomicUsize>,
    /// set when a worker exited on the cancel mark's between-chunk
    /// check (the wire-abort pin: the pump stopped early)
    stopped_early: Arc<AtomicBool>,
}

impl HttpBackend for FakeBackend {
    fn send(&self, req: &HttpRequest, out: rut_vm::Completer<HttpResponse>) {
        self.seen.lock().expect("seen").push(format!(
            "{} {} headers=[{}] body={}",
            req.method,
            req.url,
            req.headers,
            String::from_utf8_lossy(&req.body)
        ));
        match &self.script {
            Script::Stream { status, chunks } => {
                // HEADERS — the send resolves here; the writer then
                // crosses to a worker thread (any thread is the law)
                let (resp, w) = HttpResponse::streaming(*status);
                out.complete(resp);
                let chunks: Vec<Vec<u8>> = chunks.iter().map(|c| c.as_bytes().to_vec()).collect();
                std::thread::spawn(move || {
                    for c in chunks {
                        w.push_chunk(c);
                    }
                    w.end();
                });
            }
            Script::Trickle {
                status,
                chunks,
                gap_ms,
            } => {
                let (resp, w) = HttpResponse::streaming(*status);
                out.complete(resp);
                let chunks: Vec<Vec<u8>> = chunks.iter().map(|c| c.as_bytes().to_vec()).collect();
                let gap = std::time::Duration::from_millis(*gap_ms);
                let stopped = Arc::clone(&self.stopped_early);
                let pushed = Arc::clone(&self.pushed);
                std::thread::spawn(move || {
                    for (i, c) in chunks.into_iter().enumerate() {
                        if i > 0 {
                            std::thread::sleep(gap);
                        }
                        // the between-chunk wire-abort probe
                        if w.cancelled() {
                            stopped.store(true, Ordering::SeqCst);
                            return;
                        }
                        w.push_chunk(c);
                        pushed.fetch_add(1, Ordering::SeqCst);
                    }
                    w.end();
                });
            }
            Script::Late { delay_ms } => {
                let delay = std::time::Duration::from_millis(*delay_ms);
                std::thread::spawn(move || {
                    std::thread::sleep(delay);
                    let (resp, w) = HttpResponse::streaming(200);
                    out.complete(resp);
                    w.push_chunk(b"late".to_vec());
                    w.end();
                });
            }
            Script::Flood {
                chunk_bytes,
                chunks,
                beat_ms,
            } => {
                // HEADERS, then the flood: push as fast as the writer
                // accepts (the beat paces it when nonzero) — past the
                // budget the gate paces the pump
                let (resp, w) = HttpResponse::streaming(200);
                out.complete(resp);
                let pushed = Arc::clone(&self.pushed);
                let stopped = Arc::clone(&self.stopped_early);
                let (chunk_bytes, chunks, beat_ms) = (*chunk_bytes, *chunks, *beat_ms);
                let beat = std::time::Duration::from_millis(beat_ms);
                std::thread::spawn(move || {
                    for _ in 0..chunks {
                        if !beat.is_zero() {
                            std::thread::sleep(beat);
                        }
                        w.push_chunk(vec![0u8; chunk_bytes]);
                        pushed.fetch_add(1, Ordering::SeqCst);
                        // the between-chunk wire-abort probe: a cancel
                        // released this pump's budget park AND ended it
                        if w.cancelled() {
                            stopped.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                    w.end();
                });
            }
            Script::Dead(msg) => {
                out.complete(HttpResponse::failed(msg.clone()));
            }
            Script::DoubleComplete => {
                let (first, w) = HttpResponse::streaming(200);
                // the FIRST answer (a failed response) lands, then the
                // SECOND overwrites it — the pinned contract
                out.complete(HttpResponse::failed("first".to_string()));
                out.complete(first);
                w.push_chunk(b"ok".to_vec());
                w.end();
            }
        }
    }
}

fn boot_src(
    src: &str,
    script: Script,
) -> (Vm, Rc<std::cell::RefCell<Vec<String>>>, Arc<FakeBackend>) {
    let backend = Arc::new(FakeBackend {
        script,
        seen: Mutex::new(Vec::new()),
        pushed: Arc::new(AtomicUsize::new(0)),
        stopped_early: Arc::new(AtomicBool::new(false)),
    });
    let mut world = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    let tree = std::path::Path::new(PKG_DIR);
    for d in ["ink_host", "http_host", "http"] {
        world.extend(
            rut_native::dir_pkgs(&tree.join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                .pkgs,
        );
    }
    world.push(
        Mounted::source("app", src)
            // the app declares the heads it names (the nameability
            // law — mounted-but-undeclared is D5)
            .manifest(rut_driver::Manifest::lib(
                "app",
                &["http", "http_host", "futures", "async_host", "ink_host"],
            )),
    );
    let compiled = rut_driver::Walk {
        pkgs: world.clone(),
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
        "{}",
        compiled
            .graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let prog = compiled.graph.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let sink2 = sink.clone();
    hosts.install_host_pkg(
        &ctx,
        logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())),
    );
    hosts.install_host_pkg(&ctx, async_host::pkg());
    // the http face's responses build through the strbuild pkg's
    // StringBuilder — its `strbuild_host` rows are declared here, so
    // the bodies install alongside the async set
    hosts.install_host_pkg(&ctx, strbuild::pkg());
    hosts.install_host_pkg(&ctx, pkg_over(backend.clone()));
    hosts.verify_against(&ctx.flatten()); // the decl ↔ the bodies
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .unwrap();
    (vm, sink, backend)
}

/// the standard probe source (`SRC`) over a script
fn boot(script: Script) -> (Vm, Rc<std::cell::RefCell<Vec<String>>>, Arc<FakeBackend>) {
    boot_src(SRC, script)
}

/// the wall-clock pump (the reqwest lane's shape): the backend's
/// worker threads settle the completers; drain until quiet
fn pump(vm: &mut Vm) {
    for _ in 0..4000 {
        vm.run_ready().expect("run_ready");
        if vm.pending_tasks() == 0 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!(
        "the seam pump stalled: {} task(s) pending",
        vm.pending_tasks()
    );
}

/// move the virtual clock to the earliest deadline and release it into
/// the ready queue (the expiry mechanism — run_ready never walks the
/// timer map)
fn advance(vm: &mut Vm) {
    let due = vm.next_deadline().expect("a deadline is armed");
    vm.set_now(due);
    let _ = vm.next_deadline();
}

/// drive the probes' tail sleeps home: advance the clock (and spin the
/// ready queue) until nothing is pending, then let the wall-clock
/// worker threads settle
fn drive_to_rest(vm: &mut Vm, cap: usize) {
    for _ in 0..cap {
        vm.run_ready().expect("run_ready");
        if vm.pending_tasks() == 0 {
            return;
        }
        if vm.next_deadline().is_some() {
            advance(vm);
        }
    }
    pump(vm);
}

/// the flood script: 4× the budget in 16 KiB pump chunks — far past
/// the bound, still well under the VM heap limit when drained whole
fn flood_script() -> Script {
    let chunk_bytes = 16 * 1024;
    let chunks = 4 * (HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES / chunk_bytes);
    Script::Flood {
        chunk_bytes,
        chunks,
        beat_ms: 0,
    }
}

/// the flood script for the wire-abort pin: one pump beat per push so
/// the post-release sprint cannot race the test's clock advance — the
/// budget-parked push releases at the mark and the very next
/// between-chunk check ends the pump
fn flood_script_slow() -> Script {
    let chunk_bytes = 16 * 1024;
    let chunks = 4 * (HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES / chunk_bytes);
    Script::Flood {
        chunk_bytes,
        chunks,
        beat_ms: 1,
    }
}

/// the worker threads are WALL-clock while the probes are virtual: wait
/// for the between-chunk exit on the mark (the wire-abort pin)
fn wait_stopped_early(backend: &FakeBackend) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !backend.stopped_early.load(Ordering::SeqCst) {
        assert!(
            std::time::Instant::now() < deadline,
            "the pump never stopped on the between-chunk mark"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    backend.pushed.load(Ordering::SeqCst)
}

/// run_ready turns until the probe's `sleep` has armed its deadline,
/// so the test — not the flood race — decides when the reads begin
/// (the VM clock is virtual; nothing fires until `set_now`)
fn wait_sleep_armed(vm: &mut Vm) {
    for _ in 0..500 {
        vm.run_ready().expect("run_ready");
        if vm.next_deadline().is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the probe never armed its sleep");
}

/// the plateau proof: wait until `pushed` reaches `expect` and STAYS
/// put across a beat — nobody is reading, so a past-budget push
/// cannot land (it is parked inside `push_chunk`); growth past the
/// plateau means the budget gate leaked
fn wait_plateau(pushed: &AtomicUsize, expect: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "the flood never plateaued at {expect} (pushed={})",
            pushed.load(Ordering::SeqCst)
        );
        if pushed.load(Ordering::SeqCst) == expect {
            std::thread::sleep(std::time::Duration::from_millis(150));
            assert_eq!(
                pushed.load(Ordering::SeqCst),
                expect,
                "queued bytes kept growing past the budget while nobody read"
            );
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn the_budget_bounds_the_flood_and_the_drain_recovers() {
    let (mut vm, sink, backend) = boot(flood_script());
    vm.call::<_, ()>("boot_flood", ("seam://flood",)).unwrap();
    wait_sleep_armed(&mut vm);
    // the plateau: the producer stops at exactly the budget while
    // nobody reads — bounded queue, not an unbounded VecDeque
    let chunk_bytes = 16 * 1024;
    let plateau = HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES / chunk_bytes;
    wait_plateau(&backend.pushed, plateau);
    // resume: the consumer catches up, every parked push releases,
    // every byte arrives
    let due = vm.next_deadline().expect("the sleep is armed");
    vm.set_now(due);
    // next_deadline is the expiry mechanism — run_ready never walks
    // the timer map, so the due sleep moves to the ready queue here
    let _ = vm.next_deadline();
    pump(&mut vm);
    let chunks = 4 * plateau;
    assert_eq!(
        *sink.borrow(),
        vec![format!("stream-total={}", chunks * chunk_bytes)],
        "the drain walked every chunk to EOF after the plateau"
    );
    assert_eq!(
        backend.pushed.load(Ordering::SeqCst),
        chunks,
        "the gate released every parked push once the consumer drained"
    );
}

#[test]
fn the_body_drain_is_exempt_the_whole_body_lands_in_one_future() {
    let (mut vm, sink, backend) = boot(flood_script());
    vm.call::<_, ()>("boot_drain", ("seam://drain",)).unwrap();
    wait_sleep_armed(&mut vm);
    // park the pump FIRST: the flood sits parked at the budget when
    // the one-shot body lane takes over. Its stand-down notify must
    // release the pump — the drain waits for EOF, and the parked pump
    // is the only one who can reach it. Lose the notify (or the
    // body_taken exemption) and this test deadlocks into the pump's
    // stall panic.
    let chunk_bytes = 16 * 1024;
    let plateau = HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES / chunk_bytes;
    wait_plateau(&backend.pushed, plateau);
    let due = vm.next_deadline().expect("the sleep is armed");
    vm.set_now(due);
    let _ = vm.next_deadline();
    pump(&mut vm);
    let chunks = 4 * plateau;
    assert_eq!(
        *sink.borrow(),
        vec![format!("body={}", chunks * chunk_bytes)],
        "the whole 4× budget arrived in ONE future — the drain lane is exempt from the gate"
    );
    assert_eq!(
        backend.pushed.load(Ordering::SeqCst),
        chunks,
        "the exempted pump pushed everything once the body lane took over"
    );
}

#[test]
fn a_seam_backend_streams_headers_then_chunks_from_a_worker_thread() {
    let (mut vm, sink, backend) = boot(Script::Stream {
        status: 200,
        chunks: vec!["he", "llo"],
    });
    vm.call::<_, ()>("boot_stream", ("seam://chunks",)).unwrap();
    pump(&mut vm);
    assert_eq!(
        *sink.borrow(),
        vec!["stream-total=5", "stream-failed=false"],
        "send resolved at headers; the worker-thread pump walked the chunks to EOF"
    );
    assert_eq!(
        *backend.seen.lock().expect("seen"),
        vec!["GET seam://chunks headers=[] body="],
        "the backend received the crossing parts verbatim"
    );
}

#[test]
fn a_seam_backends_immediate_failure_answers_on_the_channel() {
    let (mut vm, sink, _backend) = boot(Script::Dead("seam: no route to host".to_string()));
    vm.call::<_, ()>("boot", ("seam://dead",)).unwrap();
    pump(&mut vm);
    assert_eq!(
        *sink.borrow(),
        vec!["err=seam: no route to host", "resp-nil=true"],
        "the failed mint answers on the channel: err carries the scripted \
         reason text, resp is the nil side — the status field's 0 is \
         never read by the face"
    );
}

#[test]
fn a_nil_response_used_after_a_failed_send_traps() {
    // the misuse pin: the pair bound, the err side IGNORED, the nil
    // resp side asserted and used. The VM answers NilDeref AT the
    // `!`'s own site (ops.rs:432 — "never a silent read"); an
    // UN-marked `resp.body()` would not even compile (the demand diag
    // is the law's first loudness). The CLI's `trap:` + exit-1 lane is
    // that same fact's program face.
    let (mut vm, sink, _backend) =
        boot_src(TRAP_SRC, Script::Dead("seam: no route to host".to_string()));
    vm.call::<_, ()>("boot_nil_use", ("seam://dead",)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let trap = loop {
        assert!(
            std::time::Instant::now() < deadline,
            "the nil resp use never trapped"
        );
        match vm.run_ready() {
            Ok(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
            Err(t) => break t,
        }
    };
    assert_eq!(
        trap.name(),
        "NilDeref",
        "the marked use on the nil resp side traps NilDeref at the `!`"
    );
    assert_eq!(
        *sink.borrow(),
        vec!["err=seam: no route to host"],
        "the err side crossed on the channel and was ignored — the misuse \
         the law makes loud"
    );
}

#[test]
fn double_complete_is_last_write_wins_the_pinned_contract() {
    // the ONE-COMPLETION LAW, verified and pinned: the Completer's
    // own contract is NOT loud — `complete` overwrites the slot, so a
    // violating backend's SECOND answer wins and its first is
    // silently lost (and a complete after the take is never taken —
    // the disclosed late-answer law). No trap, no error state. The
    // trait's "exactly once" is the discipline; this test pins what
    // the cell actually does.
    let (mut vm, sink, _backend) = boot(Script::DoubleComplete);
    vm.call::<_, ()>("boot", ("seam://double",)).unwrap();
    pump(&mut vm);
    assert_eq!(
        *sink.borrow(),
        vec!["status=200", "read-err=nil", "body=2"],
        "the SECOND answer won (status 200, err nil — the guard fell \
         through) — the failed first is gone; \
         the second answer's writer still drove the body"
    );
}

// ---- the wire-abort (the __cancel arms) ------------------------------

#[test]
fn the_cancel_releases_the_parked_pump_and_marks_the_source() {
    // the flood shape: the drain is launched against a flood parked at
    // the budget; the abort marks the source — the parked pump
    // RELEASES at the gate and STOPS at its between-chunk check (one
    // released push, then silence); the one-shot degrade answers the
    // dead reader nil and `read_error()` names the cancel.
    let (mut vm, sink, backend) = boot(flood_script_slow());
    vm.call::<_, ()>("boot_flood_abort", ("seam://cancel",))
        .unwrap();
    wait_sleep_armed(&mut vm); // sleep(300) armed
    let chunk_bytes = 16 * 1024;
    let plateau = HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES / chunk_bytes;
    wait_plateau(&backend.pushed, plateau); // the pump parks inside push #257
    advance(&mut vm); // sleep(300) fires: the drain launches and parks
    wait_sleep_armed(&mut vm); // sleep(1) armed
    advance(&mut vm); // sleep(1) fires: the abort
    drive_to_rest(&mut vm, 200); // the arm's turn + the readback sleeps
    assert_eq!(
        *sink.borrow(),
        vec!["abort=true", "next=nil", "read-err=cancelled"],
        "the abort retired the drain; the source is marked; the dead reader \
         degrades to nil with the named error — never a trap"
    );
    // the budget-parked push released at the mark; the body-taken
    // stand-down let a few more pushes land in the sprint before the
    // between-chunk check saw the mark — the flood never completed
    let stopped_at = wait_stopped_early(&backend);
    let chunks = 4 * (HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES / chunk_bytes);
    assert!(
        stopped_at < chunks,
        "the pump stopped at {stopped_at} of {chunks} — the wire-abort ended \
         the flood early"
    );
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(
        backend.pushed.load(Ordering::SeqCst),
        stopped_at,
        "the pump stayed stopped — the mark is sticky"
    );
}

#[test]
fn the_select2_loser_read_cancels_through_the_engine_law() {
    // the futures package's loser law with a HOST child: the second
    // read parks against a trickle, the sleep wins the select2, the
    // loser's cascade fires the row's `__cancel` arm — the worker
    // stops between chunks and every later read degrades.
    let (mut vm, sink, backend) = boot(Script::Trickle {
        status: 200,
        chunks: vec!["he", "llo", "!"],
        gap_ms: 1000,
    });
    vm.call::<_, ()>("boot_read_loser", ("seam://loser",))
        .unwrap();
    wait_sleep_armed(&mut vm); // the race's sleep(1) armed
    advance(&mut vm); // the sleep wins; the read loses
    pump(&mut vm);
    assert_eq!(
        *sink.borrow(),
        vec![
            "first=2",
            "read-lost=true",
            "second=nil",
            "read-err=cancelled",
        ],
        "the parked read lost the race and was cancelled; the next read \
         answers nil with the sticky `cancelled` error — never a trap"
    );
    let stopped_at = wait_stopped_early(&backend);
    assert!(
        stopped_at <= 1,
        "only the pre-landed chunk arrived — the trickle stopped on the mark \
         (0 when the mark beat the worker's first scheduling)"
    );
}

#[test]
fn an_explicit_abort_of_a_launched_drain_marks_the_source() {
    // `LaunchedFutureHandle::abort` over a launched drain: the receipt
    // answers true (the frame was parked), the cascade fires the
    // cancel arm, the taken lane degrades the late taker to empty,
    // and the error names the cancel.
    let (mut vm, sink, backend) = boot(Script::Trickle {
        status: 200,
        chunks: vec!["he", "llo"],
        gap_ms: 1000,
    });
    vm.call::<_, ()>("boot_drain_abort", ("seam://abort",))
        .unwrap();
    wait_sleep_armed(&mut vm);
    advance(&mut vm); // sleep(1) fires: the abort
    drive_to_rest(&mut vm, 200); // the arm's turn + the readback sleep
    assert_eq!(
        *sink.borrow(),
        vec!["abort=true", "late-body=0", "read-err=cancelled"],
        "the abort retired the parked drain; the one-shot law degrades the \
         late taker; the sticky read error names the cancel"
    );
    let stopped_at = wait_stopped_early(&backend);
    assert!(
        stopped_at <= 1,
        "only the pre-landed chunk arrived — the trickle stopped on the mark \
         (0 when the mark beat the worker's first scheduling)"
    );
}

#[test]
fn a_pre_headers_send_loser_stays_best_effort() {
    // the send future loses the race BEFORE the headers arrive: no
    // cancel slot exists yet — the disclosed best-effort law. The
    // worker's late completion lands in the retired frame and is
    // never taken; the walk continues, no trap.
    let (mut vm, sink, _backend) = boot(Script::Late { delay_ms: 300 });
    vm.call::<_, ()>("boot_send_loser", ("seam://late",))
        .unwrap();
    wait_sleep_armed(&mut vm);
    advance(&mut vm); // the sleep wins; the send loses pre-headers
    wait_sleep_armed(&mut vm); // the walk's sleep(2) armed
    advance(&mut vm);
    pump(&mut vm);
    assert_eq!(
        *sink.borrow(),
        vec!["send-lost=true", "done"],
        "the pre-headers loser degrades silently; the late completion is \
         discarded — never a trap"
    );
}
