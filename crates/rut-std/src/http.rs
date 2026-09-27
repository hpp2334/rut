//! The std HTTP lane's bodies (the rut/http redesign): the host rows of
//! `rut/http_host`, wrapped by `rut/http`'s class face (the ink
//! pattern). Three ASYNC rows ride the host future lane —
//! `register_async!` at the embedder, `Completer` answers, the take
//! marshal crosses through a phase-1 return lane — and five sync
//! readbacks ride the plain `register!` boundary:
//!
//! ```rut
//! pub host async fn http_send(c, method, url, headers, body) -> opaque;
//! pub host async fn http_body(r: opaque) -> bytes;
//! pub host async fn http_stream_next(s: opaque) -> ?bytes;
//! pub host fn http_status(r: opaque) -> i32;
//! pub host fn http_err(r: opaque) -> ?str;
//! pub host fn http_read_err(r: opaque) -> ?str;
//! pub host fn http_resp_stream(r: opaque) -> opaque;
//! ```
//!
//! The arrangement (the plan's streaming model, ruled):
//!
//! - `http_send` resolves at HEADERS. The worker thread sends the
//!   request, reads the status (+ headers), completes the send
//!   completer with the response handle, and KEEPS READING the body on
//!   the same thread — each chunk lands in the shared chunk source
//!   (`Inner`, atomics-free apart from the one Mutex the thread law
//!   allows). The response body stays UNREAD until a taker comes.
//! - `http_body` drains the remaining wire in ONE future (the one-shot
//!   lane); `http_stream_next` answers one chunk per future (nil =
//!   EOF-or-failed — the sticky read err names which);
//!   `http_resp_stream` mints the reader instantly (sync).
//! - THE ONE-SHOT LAW: body xor stream per response. A late/second
//!   taker degrades — `http_body` answers empty, a second
//!   `http_resp_stream` mints a DEAD reader whose `next` is an
//!   immediate EOF — never a trap, disclosed.
//! - Mid-read wire death is DATA: the chunk source records the sticky
//!   failure; `http_stream_next` answers nil, `http_body` the short
//!   drain, `http_read_err` (and the stream's `error()`) go non-nil.
//!   A failed completer (the `register_async!` fail lane) is NOT the
//!   mid-read path — callers never trap on network trouble.
//! - Transport failure at send: status 0 + `http_err` text (the kept
//!   law); the failure response's chunk source is born dead, so
//!   body/stream degrade to empty / immediate EOF.
//!
//! THREAD LAW: the VM stays single-threaded. Worker threads touch only
//! the Completers (atomics + a Mutex slot) and the chunk source (one
//! `Mutex<Inner>`); the response/stream rut cells are minted by the
//! rows ON the VM thread. `std::thread` appears only in the reqwest
//! lane's closures.
//!
//! Two lanes install these bodies: [`install_std_http_with`] rides an
//! injectable fixture closure keyed on method+URL(+headers/body) and
//! walks its chunk schedule on the VIRTUAL clock (the settle handle is
//! the returned [`HttpFixture`] — the test's driving loop advances the
//! clock and settles the dues); [`install_std_http`] is the reqwest
//! lane (one shared blocking client, one worker thread per request).
//! The `http` cargo feature is DEFAULT-OFF — reqwest-blocking does not
//! build on wasm32-unknown-unknown, so only native embedders opt in.

use std::collections::VecDeque;
use std::io::Read as _;
use std::sync::{Arc, Mutex};

use rut_core::types::{TypeId, TY_OPAQUE};
use rut_vm::interp::{HostRegistry, Vm};
use rut_vm::interp::Ret;
use rut_vm::{HostPayload, Opaque, OpaqueRef, Slot, Trap, TrapKind};

// ----------------------------------------------------- the payload ----

/// The chunk source shared between the response, its minted reader and
/// the worker thread: the queue the worker pushes into, the sticky
/// mid-read failure, the EOF flag, and the two one-shot lanes' pending
/// completer slots (one per read — the sequenced-Completer sibling the
/// plan spells). One Mutex; workers complete parked Completers from
/// OUTSIDE the lock (the thread law: the slot Mutex and this Mutex
/// never nest).
pub(crate) struct Inner {
    chunks: VecDeque<Vec<u8>>,
    eof: bool,
    /// the sticky mid-read failure — nil until one fails, then non-nil
    /// forever (`http_read_err` reads it; `next`'s nil means EOF *or*
    /// this)
    failed: Option<String>,
    body_waiter: Option<CompleterOf<Vec<u8>>>,
    stream_waiter: Option<CompleterOf<Option<Vec<u8>>>>,
    /// the one-shot law's bookkeeping: which lane took the body
    body_taken: bool,
    stream_minted: bool,
}

type CompleterOf<T> = rut_vm::Completer<T>;
pub(crate) type Shared = Arc<Mutex<Inner>>;

impl Inner {
    fn fresh() -> Inner {
        Inner {
            chunks: VecDeque::new(),
            eof: false,
            failed: None,
            body_waiter: None,
            stream_waiter: None,
            body_taken: false,
            stream_minted: false,
        }
    }
    /// a born-dead source: a transport-failed response's body/stream
    /// lane degrades to empty / immediate EOF
    fn dead() -> Inner {
        let mut i = Inner::fresh();
        i.eof = true;
        i
    }
}

/// The response handle's payload: the status word, the send-time
/// failure text, and the shared chunk source. Minted by `http_send`'s
/// take row ON the VM thread (the thread law); the sync readbacks and
/// the reader mint ride it through `Opaque<HttpResponse>`.
pub struct HttpResponse {
    /// the HTTP status word — 0 is RESERVED for transport failure
    pub(crate) status: u16,
    /// the transport-failure text; always `None` unless status == 0
    pub(crate) err: Option<String>,
    /// the shared chunk source (the body lane and the stream mint)
    pub(crate) read: Shared,
}

impl HostPayload for HttpResponse {}

impl HttpResponse {
    fn failed(msg: String) -> HttpResponse {
        HttpResponse { status: 0, err: Some(msg), read: Arc::new(Mutex::new(Inner::dead())) }
    }
}

/// The response crosses OUT only — `http_send`'s take marshal. The box
/// is allocated here, on the VM thread (the mint law); it never binds
/// as a param (the face wraps the handle in the `Response` class).
impl Ret for HttpResponse {
    const TY: TypeId = TY_OPAQUE;
    fn rust_name() -> &'static str {
        "HttpResponse"
    }
    fn from_slot(_vm: &rut_vm::interp::Vm, _slot: Slot, _declared: TypeId) -> Result<Self, Trap> {
        Err(Trap::new(
            TrapKind::Invalid,
            "http: the response handle crosses OUT only — it is never a host-fn parameter",
        ))
    }
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        let boxed = Opaque::alloc_hosted(vm, self)?;
        boxed.handle().clone().into_slot(vm)
    }
}

/// The minted reader's payload: the same shared chunk source as its
/// response, plus the dead flag (a degraded second mint — immediate
/// EOF forever).
pub struct HttpStream {
    pub(crate) src: Shared,
    pub(crate) dead: bool,
}

impl HostPayload for HttpStream {}

// ------------------------------------------------ the chunk wakeups ---
//
// Three transitions, all "mutate under one lock, complete the parked
// waiters OUTSIDE it". Callable from any thread: the reqwest pump
// (a worker) and the fixture settle (the test loop) land here alike.

/// one more chunk arrived — a parked stream read takes the queue's
/// front (FIFO), a parked body read keeps waiting (it drains at EOF)
fn push_chunk(src: &Shared, chunk: Vec<u8>) {
    let wake = {
        let mut g = src.lock().expect("http read state");
        g.chunks.push_back(chunk);
        g.stream_waiter.take().map(|w| (w, g.chunks.pop_front()))
    };
    if let Some((w, front)) = wake {
        w.complete(front); // the queue is non-empty — always a chunk
    }
}

/// the wire died mid-read — the sticky failure lands, the stream lane
/// gets nil, the body lane the short drain (never a trap)
fn fail_source(src: &Shared, msg: String) {
    let wake = {
        let mut g = src.lock().expect("http read state");
        g.failed = Some(msg);
        // the drain is a COPY — the queued chunks stay for the stream
        // lane's later reads (they did arrive; next serves them before
        // the nil the failure answers)
        let drain: Vec<u8> = g.chunks.iter().flatten().cloned().collect();
        (g.stream_waiter.take(), g.body_waiter.take(), drain)
    };
    let (sw, bw, drain) = wake;
    if let Some(w) = sw {
        w.complete(None);
    }
    if let Some(w) = bw {
        w.complete(drain);
    }
}

/// clean EOF — the stream lane's parked read gets nil, the body lane
/// its full drain
fn end_source(src: &Shared) {
    let wake = {
        let mut g = src.lock().expect("http read state");
        g.eof = true;
        // a copy, same law: queued chunks stay readable by the stream
        // lane after the EOF flag; the body lane's drain copies them
        let drain: Vec<u8> = g.chunks.iter().flatten().cloned().collect();
        (g.stream_waiter.take(), g.body_waiter.take(), drain)
    };
    let (sw, bw, drain) = wake;
    if let Some(w) = sw {
        w.complete(None);
    }
    if let Some(w) = bw {
        w.complete(drain);
    }
}

// ---------------------------------------------------------- the rows ---

/// The lane-independent rows: the body drain, the stream read, and the
/// five sync readbacks. Both lanes register these — only `http_send`
/// differs (a worker thread vs the virtual clock).
fn install_read_rows(hosts: &mut HostRegistry) {
    // the one-shot drain: the WHOLE remaining body in one future. A
    // transport-failed response, a second taker (body twice, or body
    // after a stream mint) degrade to EMPTY — the disclosed law.
    rut_vm::register_async!(
        hosts,
        "http_host::http_body",
        (Opaque<HttpResponse>,) -> Vec<u8>,
        move |r: Opaque<HttpResponse>| -> CompleterOf<Vec<u8>> {
            let comp = CompleterOf::new();
            let mut immediate: Option<Vec<u8>> = None;
            r.with(|resp| {
                let mut g = resp.read.lock().expect("http read state");
                if resp.err.is_some() || g.body_taken || g.stream_minted {
                    immediate = Some(Vec::new());
                    return;
                }
                g.body_taken = true;
                if let Some(_) = g.failed {
                    immediate = Some(g.chunks.drain(..).flatten().collect::<Vec<u8>>()); // the short drain
                    return;
                }
                if g.eof {
                    immediate = Some(g.chunks.drain(..).flatten().collect());
                    return;
                }
                g.body_waiter = Some(comp.clone());
            })
            .expect("http_body: the response handle is live");
            if let Some(bytes) = immediate {
                comp.complete(bytes);
            }
            comp
        },
    );
    // the stream read: one chunk per future; nil = EOF-or-failed (the
    // sticky read err names which). A degraded mint answers an
    // immediate EOF. A failed completer is NOT the mid-read path.
    rut_vm::register_async!(
        hosts,
        "http_host::http_stream_next",
        (Opaque<HttpStream>,) -> Option<Vec<u8>>,
        move |s: Opaque<HttpStream>| -> CompleterOf<Option<Vec<u8>>> {
            let comp = CompleterOf::new();
            let mut immediate: Option<Option<Vec<u8>>> = None;
            s.with(|st| {
                if st.dead {
                    immediate = Some(None);
                    return;
                }
                let mut g = st.src.lock().expect("http read state");
                if let Some(chunk) = g.chunks.pop_front() {
                    immediate = Some(Some(chunk));
                    return;
                }
                if g.failed.is_some() {
                    immediate = Some(None);
                    return;
                }
                if g.eof {
                    immediate = Some(None);
                    return;
                }
                g.stream_waiter = Some(comp.clone());
            })
            .expect("http_stream_next: the stream handle is live");
            if let Some(ans) = immediate {
                comp.complete(ans);
            }
            comp
        },
    );
    // the sync readbacks (the kept law: 0 is the transport verdict)
    rut_vm::register!(
        hosts,
        "http_host::http_status",
        (Opaque<HttpResponse>,) -> i32,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<i32, Trap> {
            Ok(r.with(|resp| resp.status as i32)?)
        },
    );
    rut_vm::register!(
        hosts,
        "http_host::http_err",
        (Opaque<HttpResponse>,) -> Option<String>,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<Option<String>, Trap> {
            Ok(r.with(|resp| resp.err.clone())?)
        },
    );
    rut_vm::register!(
        hosts,
        "http_host::http_read_err",
        (Opaque<HttpResponse>,) -> Option<String>,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<Option<String>, Trap> {
            Ok(r.with(|resp| resp.read.lock().expect("http read state").failed.clone())?)
        },
    );
    // the reader mint: instant, sync. The FIRST mint takes the stream
    // lane (body is now refused); a SECOND mint answers a dead reader
    // (immediate EOF) — the one-shot law's degrade, never a trap.
    rut_vm::register!(
        hosts,
        "http_host::http_resp_stream",
        (Opaque<HttpResponse>,) -> OpaqueRef,
        |vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<OpaqueRef, Trap> {
            let (src, dead) = r.with(|resp| {
                let mut g = resp.read.lock().expect("http read state");
                if g.body_taken || g.stream_minted {
                    (Arc::clone(&resp.read), true)
                } else {
                    g.stream_minted = true;
                    (Arc::clone(&resp.read), false)
                }
            })?;
            let boxed = Opaque::alloc_hosted(vm, HttpStream { src, dead })?;
            Ok(boxed.handle().clone())
        },
    );
}

// ------------------------------------------------------ the fixture ---
//
// The fixture lane: the request answers on the virtual clock. The
// `HttpFixture` handle is the test's side — the driving loop settles
// the dues (`settle(now_ms)`) and advances to `next_due()`, exactly
// like the engine fixture lanes (the async_host_fns shape). Chunk
// arrival is deterministic: the send at tick 0, chunk i at tick i+1,
// the terminal (EOF or the mid-read failure) one tick after the last.

/// The fixture's recorded reply: the status word and the CHUNK PLAN —
/// the body cut into exactly these pieces, released one per tick (the
/// deterministic small chunks the streaming tests assert on).
pub struct FixtureReply {
    pub status: u16,
    pub chunks: Vec<Vec<u8>>,
    /// a mid-read wire death: after this many released chunks the
    /// source fails with `fail_msg` (the rest never arrives)
    pub fail_after: Option<usize>,
    pub fail_msg: String,
}

impl Clone for FixtureReply {
    fn clone(&self) -> Self {
        FixtureReply {
            status: self.status,
            chunks: self.chunks.clone(),
            fail_after: self.fail_after,
            fail_msg: self.fail_msg.clone(),
        }
    }
}

impl FixtureReply {
    /// the whole body as ONE chunk (the buffered shape)
    pub fn ok(status: u16, body: &[u8]) -> FixtureReply {
        FixtureReply {
            status,
            chunks: vec![body.to_vec()],
            fail_after: None,
            fail_msg: String::new(),
        }
    }
    /// the body as an explicit chunk plan
    pub fn chunked(status: u16, chunks: Vec<Vec<u8>>) -> FixtureReply {
        FixtureReply {
            status,
            chunks,
            fail_after: None,
            fail_msg: String::new(),
        }
    }
    /// chunks up to `after` arrive, then the wire dies with `msg`
    pub fn failing(status: u16, chunks: Vec<Vec<u8>>, after: usize, msg: &str) -> FixtureReply {
        FixtureReply {
            status,
            chunks,
            fail_after: Some(after),
            fail_msg: msg.to_string(),
        }
    }
}

enum Due {
    Send {
        reply: Result<FixtureReply, String>,
        comp: CompleterOf<HttpResponse>,
        src: Shared,
    },
    Chunk { src: Shared, chunk: Vec<u8> },
    Terminal { src: Shared },
    Fail { src: Shared, msg: String },
}

#[derive(Default)]
struct FixtureState {
    dues: std::cell::RefCell<Vec<(u64, Due)>>,
}

/// The fixture lane's settle handle: the test loop's side of the
/// virtual clock. Clone = another handle on the same dues.
#[derive(Clone, Default)]
pub struct HttpFixture(std::rc::Rc<FixtureState>);

impl HttpFixture {
    /// fire every due at or before `now_ms`; true when anything fired
    /// (the loop spins the poll without advancing, the async_host_fns
    /// shape)
    pub fn settle(&self, now_ms: u64) -> bool {
        let mut fired = false;
        let mut dues = self.0.dues.borrow_mut();
        let mut rest = Vec::new();
        for (at, due) in dues.drain(..) {
            if at <= now_ms {
                fired = true;
                match due {
                    Due::Send { reply, comp, src } => match reply {
                        Err(msg) => comp.complete(HttpResponse::failed(msg)),
                        Ok(r) => comp.complete(HttpResponse {
                            status: r.status,
                            err: None,
                            read: Arc::clone(&src),
                        }),
                    },
                    Due::Chunk { src, chunk } => push_chunk(&src, chunk),
                    Due::Terminal { src } => end_source(&src),
                    Due::Fail { src, msg } => fail_source(&src, msg),
                }
            } else {
                rest.push((at, due));
            }
        }
        *dues = rest;
        fired
    }
    /// the earliest unsettled due (the loop's next clock stop)
    pub fn next_due(&self) -> Option<u64> {
        self.0.dues.borrow().iter().map(|(at, _)| *at).min()
    }
    pub fn has_pending(&self) -> bool {
        self.next_due().is_some()
    }
    /// arm a request's schedule: the send at 0, chunk i at i+1, the
    /// terminal one tick after the last released chunk
    fn arm(&self, reply: Result<FixtureReply, String>) -> CompleterOf<HttpResponse> {
        let comp = CompleterOf::new();
        let src: Shared = Arc::new(Mutex::new(Inner::fresh()));
        let mut dues = self.0.dues.borrow_mut();
        let send_reply = reply.clone();
        dues.push((0, Due::Send { reply: send_reply, comp: comp.clone(), src: Arc::clone(&src) }));
        if let Ok(r) = &reply {
            let mut t = 0u64;
            for (i, chunk) in r.chunks.iter().enumerate() {
                if r.fail_after == Some(i) {
                    break; // the wire died before this chunk
                }
                t += 1;
                dues.push((t, Due::Chunk { src: Arc::clone(&src), chunk: chunk.clone() }));
            }
            t += 1;
            match r.fail_after {
                Some(after) if after <= r.chunks.len() => {
                    dues.push((t, Due::Fail { src: Arc::clone(&src), msg: r.fail_msg.clone() }))
                }
                _ => dues.push((t, Due::Terminal { src: Arc::clone(&src) })),
            }
        }
        comp
    }
}

/// Install the fixture lane over an injectable request closure: `f`
/// maps (method, url, headers, body) to the recorded reply — or
/// `Err(message)` for a transport failure, which lands as status 0
/// with the message in `http_err` and a born-dead body. The fixture
/// map's keys ARE the assertions: an unknown URL is a LOUD transport
/// failure, never a pass-through. Returns the settle handle the test's
/// driving loop walks.
pub fn install_std_http_with<F>(hosts: &mut HostRegistry, f: F) -> HttpFixture
where
    F: Fn(&str, &str, &str, &[u8]) -> Result<FixtureReply, String> + 'static,
{
    let fixture = HttpFixture::default();
    let fx = fixture.clone();
    rut_vm::register_async!(
        hosts,
        "http_host::http_send",
        (OpaqueRef, String, String, String, Vec<u8>) -> HttpResponse,
        move |c: OpaqueRef, method: String, url: String, headers: String, body: Vec<u8>|
              -> CompleterOf<HttpResponse> {
            let _ = (c, &headers, &body); // the lanes own their client state
            let reply = f(&method, &url, &headers, &body);
            fx.arm(reply)
        },
    );
    install_read_rows(hosts);
    fixture
}

// ----------------------------------------------------- the reqwest ----

/// Install the reqwest lane: one shared blocking client (this UA,
/// redirects on — the default policy's ten hops; gzip/brotli arrive
/// with the cargo features), one worker thread PER REQUEST.
/// `http_send` resolves at HEADERS — the worker completes the send
/// completer the moment the status is known, then keeps reading the
/// (still unread) response body into the chunk source on the same
/// thread. Transport failure maps to status 0 + the message (the kept
/// law). Requires the `http` cargo feature.
#[cfg(feature = "http")]
pub fn install_std_http(hosts: &mut HostRegistry) {
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            .user_agent("rgh/0.1 (+https://github.com/rut)")
            .redirect(reqwest::redirect::Policy::default())
            .build()
            .expect("the std http client builds"),
    );
    rut_vm::register_async!(
        hosts,
        "http_host::http_send",
        (OpaqueRef, String, String, String, Vec<u8>) -> HttpResponse,
        move |c: OpaqueRef, method: String, url: String, headers: String, body: Vec<u8>|
              -> CompleterOf<HttpResponse> {
            let _ = c;
            let comp = CompleterOf::new();
            let w = comp.clone();
            let client = Arc::clone(&client);
            // the worker owns the request end to end: send, complete at
            // HEADERS, then pump the body into the chunk source. The
            // cancel arm is the disclosed no-op — the thread runs to its
            // blocking completion and the late result is simply never
            // taken.
            std::thread::spawn(move || {
                send_once(&client, &method, &url, &headers, &body, &w);
            });
            comp
        },
    );
    install_read_rows(hosts);
}

/// The reqwest worker: build the request (canonical method, the
/// "\n"-joined `Name: value` header block, bytes with empty = none —
/// the disclosed empty-vs-none merge), execute, complete the send at
/// HEADERS, then pump the body until EOF or wire death.
#[cfg(feature = "http")]
fn send_once(
    client: &reqwest::blocking::Client,
    method: &str,
    url: &str,
    headers: &str,
    body: &[u8],
    w: &CompleterOf<HttpResponse>,
) {
    let send = (|| -> Result<reqwest::blocking::Response, String> {
        let m = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| format!("bad method `{method}`: {e}"))?;
        let mut req = client.request(m, url);
        for line in headers.split('\n') {
            if line.trim().is_empty() {
                continue;
            }
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("bad header line `{line}` (want `Name: value`)"))?;
            req = req.header(name.trim(), value.trim());
        }
        if !body.is_empty() {
            req = req.body(body.to_vec());
        }
        req.send().map_err(|e| e.to_string())
    })();
    let mut resp = match send {
        Err(msg) => {
            w.complete(HttpResponse::failed(msg));
            return;
        }
        Ok(resp) => resp,
    };
    let status = resp.status().as_u16();
    let src: Shared = Arc::new(Mutex::new(Inner::fresh()));
    // HEADERS — the send resolves here; the body stays unread
    w.complete(HttpResponse { status, err: None, read: Arc::clone(&src) });
    // the pump: this thread keeps reading the wire into the chunk
    // source. A fixed 16 KiB buffer — bounded memory end to end, and
    // each read lands as one stream chunk.
    let mut buf = [0u8; 16 * 1024];
    loop {
        match resp.read(&mut buf) {
            Ok(0) => {
                end_source(&src);
                return;
            }
            Ok(n) => push_chunk(&src, buf[..n].to_vec()),
            Err(e) => {
                fail_source(&src, e.to_string());
                return;
            }
        }
    }
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// this test's host pkgs — the mounted decl surface the bodies must
    /// match (RFC 0025)
    const PKG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut");

    /// the probe: the FACE (the `rut/http` classes), driven as an async
    /// free fn over the launcher — the same shape the rgh brain rides.
    /// The awaits deliver (the answer lane): send at headers, body as
    /// the drain, stream reads per chunk.
    const SRC: &str = r#"
use http::{ HttpClient, ClientQueryMethod };
use http_host::{ http_status, http_err, http_read_err };
use async_host::launch_future;
use rt::{ create_logger, logger_log };

async fn probe(cx: RunContext, log: opaque, url: str) -> nil {
    let client = HttpClient.new();
    let resp = await client.post(url, "hi".encode()).build().send(cx);
    logger_log(log, 2, f"status={resp.status()}");
    // the readbacks are ?str — nil-check, then deref, then log (the
    // rgh deref pattern; a ?str never interpolates)
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
    let b = await resp.body(cx);
    logger_log(log, 2, f"body={b.len()}");
}

async fn streamed(cx: RunContext, log: opaque, url: str) -> nil {
    let client = HttpClient.new();
    let resp = await client.get(url).build().send(cx);
    let s = resp.byte_stream();
    let mut total: i32 = 0;
    let mut failed = false;
    while (true) {
        let c = await s.next(cx);
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
"#;

    fn boot(
        fixture: impl Fn(&str, &str, &str, &[u8]) -> Result<FixtureReply, String> + 'static,
    ) -> (Vm, Rc<std::cell::RefCell<Vec<String>>>, HttpFixture) {
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std_core(&mut session);
        rut_driver::mount_std_async(&mut session);
        rut_driver::mount_dir(&mut session, &std::path::Path::new(PKG_DIR).join("rt"))
            .expect("mount rt");
        rut_driver::mount_dir(&mut session, &std::path::Path::new(PKG_DIR).join("http_host"))
            .expect("mount http_host");
        rut_driver::mount_dir(&mut session, &std::path::Path::new(PKG_DIR).join("http"))
            .expect("mount http");
        session
            .register_module(
                "app",
                rut_driver::Module {
                    spec: "app".into(),
                    source: Some(SRC.into()),
                    ..Default::default()
                },
            )
            .expect("register the probe");
        let g = rut_driver::compile_graph(&session, "app");
        assert!(
            g.diags.is_empty(),
            "{}",
            g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
        );
        let prog = g.program.expect("compile");
        rut_vm::verify::verify(&prog).unwrap();
        let limits = rut_vm::interp::Limits {
            fuel: Some(4_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        };
        let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut hosts = rut_vm::interp::HostRegistry::new();
        let sink2 = sink.clone();
        crate::logger::install_std_log(&mut hosts, move |m| sink2.borrow_mut().push(m.to_string()));
        crate::async_host::install_std_async(&mut hosts);
        let fx = install_std_http_with(&mut hosts, fixture);
        hosts.verify_against(&session.expected_host_fns()); // the decl ↔ the bodies
        let vm = rut_vm::interp::Vm::new(
            Rc::new(prog),
            &limits,
            rut_vm::interp::HostHooks::default(),
            hosts,
        )
        .unwrap();
        (vm, sink, fx)
    }

    /// the driving loop over the virtual clock: drain the ready queue,
    /// settle the fixture's dues, advance to the next due/deadline
    fn run_loop(vm: &mut Vm, fx: &HttpFixture, cap: usize) {
        for _ in 0..cap {
            vm.run_ready().expect("run_ready");
            if vm.pending_tasks() == 0 && !fx.has_pending() {
                return;
            }
            if fx.settle(vm.now_ms()) {
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

    #[test]
    fn the_face_sends_posts_and_drains() {
        let seen = Rc::new(std::cell::RefCell::new(String::new()));
        let seen2 = seen.clone();
        let (mut vm, sink, fx) = boot(move |method, url, _h, body| {
            seen2
                .borrow_mut()
                .push_str(&format!("{method} {url} body={}\n", String::from_utf8_lossy(body)));
            if url == "fixture://ok" {
                Ok(FixtureReply::ok(200, b"hello rut"))
            } else {
                Err(format!("rgh-test: unexpected url: {url}"))
            }
        });
        vm.call::<_, ()>("boot", ("fixture://ok",)).unwrap();
        run_loop(&mut vm, &fx, 100);
        assert_eq!(
            *sink.borrow(),
            vec!["status=200", "err=nil", "read-err=nil", "body=9"],
            "send resolves at headers, body drains through the answer lane"
        );
        assert_eq!(
            seen.borrow().as_str(),
            "POST fixture://ok body=hi\n",
            "the builder crossed method+url+body verbatim"
        );
    }

    #[test]
    fn transport_failure_reserves_status_zero_and_degrades() {
        let (mut vm, sink, fx) = boot(|_m, url, _h, _b| {
            Err(format!("boom: {url} failed"))
        });
        vm.call::<_, ()>("boot", ("fixture://bad",)).unwrap();
        run_loop(&mut vm, &fx, 100);
        assert_eq!(
            *sink.borrow(),
            vec!["status=0", "err=boom: fixture://bad failed", "read-err=nil", "body=0"],
            "status 0 + the err text; the body lane degrades to empty"
        );
    }

    #[test]
    fn http_error_statuses_are_not_transport_failures() {
        let (mut vm, sink, fx) = boot(|_m, _u, _h, _b| Ok(FixtureReply::ok(404, b"nope")));
        vm.call::<_, ()>("boot", ("fixture://404",)).unwrap();
        run_loop(&mut vm, &fx, 100);
        assert_eq!(
            *sink.borrow(),
            vec!["status=404", "err=nil", "read-err=nil", "body=4"],
            "a real status (4xx included) keeps err nil and the body intact"
        );
    }

    #[test]
    fn the_stream_lane_walks_deterministic_chunks_to_eof() {
        let (mut vm, sink, fx) = boot(|_m, _u, _h, _b| {
            Ok(FixtureReply::chunked(
                200,
                vec![b"ab".to_vec(), b"cde".to_vec(), b"f".to_vec()],
            ))
        });
        vm.call::<_, ()>("boot_stream", ("fixture://chunks",)).unwrap();
        run_loop(&mut vm, &fx, 100);
        assert_eq!(
            *sink.borrow(),
            vec!["stream-total=6", "stream-failed=false"],
            "chunk per next, EOF nil, the sticky err stays clean"
        );
    }

    #[test]
    fn a_mid_read_death_is_data_never_a_trap() {
        let (mut vm, sink, fx) = boot(|_m, _u, _h, _b| {
            Ok(FixtureReply::failing(
                200,
                vec![b"ab".to_vec(), b"cd".to_vec(), b"ef".to_vec()],
                2,
                "connection reset by peer",
            ))
        });
        vm.call::<_, ()>("boot_stream", ("fixture://die",)).unwrap();
        run_loop(&mut vm, &fx, 100);
        assert_eq!(
            *sink.borrow(),
            vec!["stream-total=4", "stream-failed=true"],
            "two chunks, then nil; the sticky read err is set (both readbacks read one fact)"
        );
    }

    #[test]
    fn the_body_lane_answers_the_short_drain_after_a_mid_read_death() {
        let src = r#"
use http::HttpClient;
use async_host::launch_future;
use rt::{ create_logger, logger_log };

async fn probe(cx: RunContext, log: opaque, url: str) -> nil {
    let client = HttpClient.new();
    let resp = await client.get(url).build().send(cx);
    let b = await resp.body(cx);
    logger_log(log, 2, f"drain={b.len()}");
    logger_log(log, 2, f"read-err={resp.read_error()}");
}

entry fn boot(url: str) -> nil {
    let log = create_logger("t");
    launch_future(probe(log, url));
}
"#;
        // splice the probe source over the default one and re-boot by hand
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std_core(&mut session);
        rut_driver::mount_std_async(&mut session);
        let pkg = std::path::Path::new(PKG_DIR);
        for d in ["rt", "http_host", "http"] {
            rut_driver::mount_dir(&mut session, &pkg.join(d)).expect("mount pkg");
        }
        session
            .register_module("app", rut_driver::Module { spec: "app".into(), source: Some(src.into()), ..Default::default() })
            .expect("register");
        let g = rut_driver::compile_graph(&session, "app");
        assert!(g.diags.is_empty(), "{:?}", g.diags);
        let prog = g.program.expect("compile");
        rut_vm::verify::verify(&prog).unwrap();
        let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut hosts = rut_vm::interp::HostRegistry::new();
        let sink2 = sink.clone();
        crate::logger::install_std_log(&mut hosts, move |m| sink2.borrow_mut().push(m.to_string()));
        crate::async_host::install_std_async(&mut hosts);
        let fx = install_std_http_with(&mut hosts, |_m, _u, _h, _b| {
            Ok(FixtureReply::failing(200, vec![b"xy".to_vec(), b"zw".to_vec()], 1, "eof in chunk"))
        });
        hosts.verify_against(&session.expected_host_fns());
        let limits = rut_vm::interp::Limits {
            fuel: Some(4_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        };
        let mut vm = rut_vm::interp::Vm::new(
            Rc::new(prog),
            &limits,
            rut_vm::interp::HostHooks::default(),
            hosts,
        )
        .unwrap();
        vm.call::<_, ()>("boot", ("fixture://short",)).unwrap();
        run_loop(&mut vm, &fx, 100);
        assert_eq!(
            *sink.borrow(),
            vec!["drain=2", "read-err=eof in chunk"],
            "one chunk arrived, then the wire died — the drain answers the short body"
        );
    }

    #[test]
    fn the_one_shot_law_degrades_second_takers() {
        let src = r#"
use http::HttpClient;
use async_host::launch_future;
use rt::{ create_logger, logger_log };

async fn probe(cx: RunContext, log: opaque, url: str) -> nil {
    let client = HttpClient.new();
    let resp = await client.get(url).build().send(cx);
    // body first, THEN a stream mint: the mint degrades to a dead
    // reader — immediate EOF, empty error
    let b = await resp.body(cx);
    logger_log(log, 2, f"body={b.len()}");
    let s = resp.byte_stream();
    let c = await s.next(cx);
    if (c == nil) {
        logger_log(log, 2, "late-next-nil=true");
    } else {
        logger_log(log, 2, "late-next-nil=false");
    }
    let le = s.error();
    if (le == nil) {
        logger_log(log, 2, "late-err=nil");
    } else {
        let t: str = le;
        logger_log(log, 2, f"late-err={t}");
    }
}

// the other order: stream first, then body — the drain degrades to
// empty
async fn probe2(cx: RunContext, log: opaque, url: str) -> nil {
    let client = HttpClient.new();
    let resp = await client.get(url).build().send(cx);
    let s = resp.byte_stream();
    let c = await s.next(cx);
    let mut n: i32 = -1;
    if (c != nil) {
        let v: bytes = c;
        n = v.len();
    }
    logger_log(log, 2, f"first-next={n}");
    let b = await resp.body(cx);
    logger_log(log, 2, f"late-body={b.len()}");
}

entry fn boot(url: str) -> nil {
    let log = create_logger("t");
    launch_future(probe(log, url));
    launch_future(probe2(log, url));
}
"#;
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std_core(&mut session);
        rut_driver::mount_std_async(&mut session);
        let pkg = std::path::Path::new(PKG_DIR);
        for d in ["rt", "http_host", "http"] {
            rut_driver::mount_dir(&mut session, &pkg.join(d)).expect("mount pkg");
        }
        session
            .register_module("app", rut_driver::Module { spec: "app".into(), source: Some(src.into()), ..Default::default() })
            .expect("register");
        let g = rut_driver::compile_graph(&session, "app");
        assert!(g.diags.is_empty(), "{:?}", g.diags);
        let prog = g.program.expect("compile");
        rut_vm::verify::verify(&prog).unwrap();
        let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut hosts = rut_vm::interp::HostRegistry::new();
        let sink2 = sink.clone();
        crate::logger::install_std_log(&mut hosts, move |m| sink2.borrow_mut().push(m.to_string()));
        crate::async_host::install_std_async(&mut hosts);
        let fx = install_std_http_with(&mut hosts, |_m, _u, _h, _b| {
            Ok(FixtureReply::chunked(200, vec![b"ab".to_vec(), b"cd".to_vec()]))
        });
        hosts.verify_against(&session.expected_host_fns());
        let limits = rut_vm::interp::Limits {
            fuel: Some(4_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        };
        let mut vm = rut_vm::interp::Vm::new(
            Rc::new(prog),
            &limits,
            rut_vm::interp::HostHooks::default(),
            hosts,
        )
        .unwrap();
        vm.call::<_, ()>("boot", ("fixture://oneshot",)).unwrap();
        run_loop(&mut vm, &fx, 200);
        // the two launched futures interleave — the LAWS are
        // per-response, so compare as a set
        let mut got = sink.borrow().clone();
        got.sort();
        assert_eq!(
            got,
            vec![
                "body=4",             // probe: the drain took the whole body
                "first-next=2",       // probe2: the stream lane took chunk one
                "late-body=0",        // the late DRAIN degrades to empty
                "late-err=nil",       // degraded, not failed
                "late-next-nil=true", // the late mint is a dead reader
            ],
            "body xor stream: whichever lane took first wins, the second degrades"
        );
    }

    #[test]
    fn the_builder_walks_every_verb_and_header() {
        let src = r#"
use http::{ HttpClient, ClientQueryMethod };
use async_host::launch_future;
use rt::{ create_logger, logger_log };

async fn probe(cx: RunContext, log: opaque) -> nil {
    let client = HttpClient.new();
    let _ = await client.get("fixture://get").build().send(cx);
    let _ = await client.post("fixture://post", "p".encode()).build().send(cx);
    let _ = await client.put("fixture://put", "p".encode()).build().send(cx);
    let _ = await client.patch("fixture://patch", "p".encode()).build().send(cx);
    let _ = await client.del("fixture://delete").build().send(cx);
    let _ = await client.request()
        .method(ClientQueryMethod.Put)
        .url("fixture://manual")
        .header("Accept", "application/json")
        .header("X-Trace", "t-1")
        .body("m".encode())
        .build()
        .send(cx);
    logger_log(log, 2, "all-sent");
}

entry fn boot() -> nil {
    let log = create_logger("t");
    launch_future(probe(log));
}
"#;
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std_core(&mut session);
        rut_driver::mount_std_async(&mut session);
        let pkg = std::path::Path::new(PKG_DIR);
        for d in ["rt", "http_host", "http"] {
            rut_driver::mount_dir(&mut session, &pkg.join(d)).expect("mount pkg");
        }
        session
            .register_module("app", rut_driver::Module { spec: "app".into(), source: Some(src.into()), ..Default::default() })
            .expect("register");
        let g = rut_driver::compile_graph(&session, "app");
        assert!(g.diags.is_empty(), "{:?}", g.diags);
        let prog = g.program.expect("compile");
        rut_vm::verify::verify(&prog).unwrap();
        let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut hosts = rut_vm::interp::HostRegistry::new();
        let sink2 = sink.clone();
        crate::logger::install_std_log(&mut hosts, move |m| sink2.borrow_mut().push(m.to_string()));
        crate::async_host::install_std_async(&mut hosts);
        let seen = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let seen2 = seen.clone();
        let fx = install_std_http_with(&mut hosts, move |method, url, headers, body| {
            seen2.borrow_mut().push(format!(
                "{method} {url} headers=[{headers}] body={}",
                String::from_utf8_lossy(body)
            ));
            Ok(FixtureReply::ok(200, b""))
        });
        hosts.verify_against(&session.expected_host_fns());
        let limits = rut_vm::interp::Limits {
            fuel: Some(4_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        };
        let mut vm = rut_vm::interp::Vm::new(
            Rc::new(prog),
            &limits,
            rut_vm::interp::HostHooks::default(),
            hosts,
        )
        .unwrap();
        vm.call::<_, ()>("boot", ()).unwrap();
        run_loop(&mut vm, &fx, 400);
        assert_eq!(*sink.borrow(), vec!["all-sent"]);
        assert_eq!(
            *seen.borrow(),
            vec![
                "GET fixture://get headers=[] body=",
                "POST fixture://post headers=[] body=p",
                "PUT fixture://put headers=[] body=p",
                "PATCH fixture://patch headers=[] body=p",
                "DELETE fixture://delete headers=[] body=",
                "PUT fixture://manual headers=[Accept: application/json\nX-Trace: t-1] body=m",
            ],
            "the five verbs are build sugars; headers accumulate Name: value lines"
        );
    }

    #[test]
    fn the_decl_rows_join_both_lanes() {
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std_core(&mut session);
        let pkg = std::path::Path::new(PKG_DIR);
        rut_driver::mount_dir(&mut session, &pkg.join("http_host")).expect("mount http_host");
        let expected = session.expected_host_fns();
        // the fixture lane binds the whole family over the virtual clock
        let mut hosts = rut_vm::interp::HostRegistry::new();
        let _fx = install_std_http_with(&mut hosts, |_m, _u, _h, _b| Ok(FixtureReply::ok(200, b"")));
        hosts.verify_against(&expected); // panics on drift
        // the reqwest lane binds the SAME rows — no network at bind time
        let mut hosts = rut_vm::interp::HostRegistry::new();
        install_std_http(&mut hosts);
        hosts.verify_against(&expected);
    }

    #[test]
    fn a_worker_thread_streams_through_the_reqwest_lane() {
        // the REAL lane over a local TCP server: send resolves, the
        // stream walks the chunks, the sticky read err stays clean —
        // the thread law end to end
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf); // the request head (small)
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabc").unwrap();
            sock.write_all(b"def").unwrap(); // the rest after the headers
            sock.flush().unwrap();
        });
        let url = format!("http://{addr}/stream");
        // reuse the standard boot with the REAL lane bolted on
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std_core(&mut session);
        rut_driver::mount_std_async(&mut session);
        let pkg = std::path::Path::new(PKG_DIR);
        for d in ["rt", "http_host", "http"] {
            rut_driver::mount_dir(&mut session, &pkg.join(d)).expect("mount pkg");
        }
        let stream_src = r#"
use http::HttpClient;
use async_host::launch_future;
use rt::{ create_logger, logger_log };

async fn streamed(cx: RunContext, log: opaque, url: str) -> nil {
    let client = HttpClient.new();
    let resp = await client.get(url).build().send(cx);
    logger_log(log, 2, f"status={resp.status()}");
    let s = resp.byte_stream();
    let mut total: i32 = 0;
    let mut failed = false;
    while (true) {
        let c = await s.next(cx);
        if (c == nil) {
            let e = s.error();
            if (e != nil) { failed = true; }
            break;
        }
        let chunk: bytes = c;
        total += chunk.len();
    }
    logger_log(log, 2, f"total={total}");
    logger_log(log, 2, f"failed={failed}");
}

entry fn boot_stream(url: str) -> nil {
    let log = create_logger("t");
    launch_future(streamed(log, url));
}
"#;
        session
            .register_module("app", rut_driver::Module { spec: "app".into(), source: Some(stream_src.into()), ..Default::default() })
            .expect("register");
        let g = rut_driver::compile_graph(&session, "app");
        assert!(g.diags.is_empty(), "{:?}", g.diags);
        let prog = g.program.expect("compile");
        rut_vm::verify::verify(&prog).unwrap();
        let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut hosts = rut_vm::interp::HostRegistry::new();
        let sink2 = sink.clone();
        crate::logger::install_std_log(&mut hosts, move |m| sink2.borrow_mut().push(m.to_string()));
        crate::async_host::install_std_async(&mut hosts);
        install_std_http(&mut hosts); // the reqwest lane
        hosts.verify_against(&session.expected_host_fns());
        let limits = rut_vm::interp::Limits {
            fuel: Some(4_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        };
        let mut vm = rut_vm::interp::Vm::new(
            Rc::new(prog),
            &limits,
            rut_vm::interp::HostHooks::default(),
            hosts,
        )
        .unwrap();
        vm.call::<_, ()>("boot_stream", (url,)).unwrap();
        // wall-clock pump: the worker thread settles the completers
        for _ in 0..2000 {
            vm.run_ready().expect("run_ready");
            if vm.pending_tasks() == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        server.join().expect("the server thread");
        assert_eq!(
            *sink.borrow(),
            vec!["status=200", "total=6", "failed=false"],
            "the send resolved at headers; the pump walked the whole body"
        );
    }
}
