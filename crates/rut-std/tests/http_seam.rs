//! The seam suite: the embedder's own-backend lane over `pkg_over`.
//! A fake `HttpBackend` — in this file, the crate's test home being
//! `tests/` (the same shelf the fixture suite lives on, no
//! `#[cfg(test)]` tree) — drives the seven `http_host` rows through
//! the rut face: headers-then-chunks pumped from a real worker
//! thread, the immediate transport failure (status 0), the
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// this test's host pkgs — the mounted decl surface the bodies must
/// match (the tree packages, never a copied shim)
const PKG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut");

/// the probe: the FACE (the `rut/http` classes), driven as an async
/// free fn over the launcher — send at headers, the readbacks, the
/// body as the drain
const SRC: &str = r#"
use http::HttpClient;
use futures::launch_future;
use futures::sleep;
use ink_host::{ create_logger, logger_log };

async fn probe(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.post(url, "hi".encode()).build().send();
    logger_log(log, 2, f"status={resp.status()}");
    let terr = resp.transport_error();
    if (terr == nil) {
        logger_log(log, 2, "err=nil");
    } else {
        let t: str = terr;
        logger_log(log, 2, f"err={t}");
    }
    let rerr = resp.read_error();
    if (rerr == nil) {
        logger_log(log, 2, "read-err=nil");
    } else {
        let t: str = rerr;
        logger_log(log, 2, f"read-err={t}");
    }
    let b = await resp.body();
    logger_log(log, 2, f"body={b.len()}");
}

async fn streamed(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.get(url).build().send();
    let s = resp.byte_stream();
    let total: i32 = 0;
    let failed = false;
    while (true) {
        let c = await s.next();
        if (c == nil) {
            let e = s.error();
            if (e != nil) { failed = true; }
            break;
        }
        let chunk: bytes = c;
        total += chunk.len();
    }
    logger_log(log, 2, f"stream-total={total}");
    logger_log(log, 2, f"stream-failed={failed}");
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
    let resp = await client.get(url).build().send();
    await sleep(300);
    let s = resp.byte_stream();
    let total: i32 = 0;
    while (true) {
        let c = await s.next();
        if (c == nil) {
            break;
        }
        let chunk: bytes = c;
        total += chunk.len();
    }
    logger_log(log, 2, f"stream-total={total}");
}

async fn drained(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.get(url).build().send();
    await sleep(300);
    let b = await resp.body();
    logger_log(log, 2, f"body={b.len()}");
}

entry fn boot_flood(url: str) -> nil {
    let log = create_logger("t");
    launch_future(flooded(log, url));
}

entry fn boot_drain(url: str) -> nil {
    let log = create_logger("t");
    launch_future(drained(log, url));
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
    /// pushes of `chunk_bytes` each, as fast as the writer accepts —
    /// the BUDGET is the only pacing
    Flood { chunk_bytes: usize, chunks: usize },
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
            Script::Flood {
                chunk_bytes,
                chunks,
            } => {
                // HEADERS, then the flood: push as fast as the writer
                // accepts — past the budget the gate paces the pump
                let (resp, w) = HttpResponse::streaming(200);
                out.complete(resp);
                let pushed = Arc::clone(&self.pushed);
                let (chunk_bytes, chunks) = (*chunk_bytes, *chunks);
                std::thread::spawn(move || {
                    for _ in 0..chunks {
                        w.push_chunk(vec![0u8; chunk_bytes]);
                        pushed.fetch_add(1, Ordering::SeqCst);
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

fn boot(script: Script) -> (Vm, Rc<std::cell::RefCell<Vec<String>>>, Arc<FakeBackend>) {
    let backend = Arc::new(FakeBackend {
        script,
        seen: Mutex::new(Vec::new()),
        pushed: Arc::new(AtomicUsize::new(0)),
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
        Mounted::source("app", SRC)
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

/// the flood script: 4× the budget in 16 KiB pump chunks — far past
/// the bound, still well under the VM heap limit when drained whole
fn flood_script() -> Script {
    let chunk_bytes = 16 * 1024;
    let chunks = 4 * (HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES / chunk_bytes);
    Script::Flood {
        chunk_bytes,
        chunks,
    }
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
fn a_seam_backends_immediate_failure_lands_as_status_zero() {
    let (mut vm, sink, _backend) = boot(Script::Dead("seam: no route to host".to_string()));
    vm.call::<_, ()>("boot", ("seam://dead",)).unwrap();
    pump(&mut vm);
    assert_eq!(
        *sink.borrow(),
        vec![
            "status=0",
            "err=seam: no route to host",
            "read-err=nil",
            "body=0"
        ],
        "the failed mint: status 0 + the err text; the body lane degrades to empty"
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
        vec!["status=200", "err=nil", "read-err=nil", "body=2"],
        "the SECOND answer won (status 200, err nil) — the failed first is gone; \
         the second answer's writer still drove the body"
    );
}
