//! The std HTTP lane's bodies (the rut/http redesign): the host rows of
//! `rut/http_host`, wrapped by `rut/http`'s class face (the ink
//! pattern). The surface is a MOD TREE (the host-.d.rut-mod-tree
//! plan's pilot): `net/` carries the send + the response readbacks,
//! `stream/` the body lanes. Three ASYNC rows ride the host future
//! lane — `register_async!` at the embedder, `Completer` answers, the
//! take marshal crosses through a phase-1 return lane — and five sync
//! readbacks ride the plain `register!` boundary:
//!
//! ```rut
//! pub mod net;      // http_host/net/mod.d.rut
//! pub mod stream;   // http_host/stream/mod.d.rut
//!
//! // net:
//! pub host async fn http_send(c, method, url, headers, body) -> opaque;
//! pub host fn http_status(r: opaque) -> i32;
//! pub host fn http_err(r: opaque) -> ?str;
//! pub host fn http_read_err(r: opaque) -> ?str;
//! pub host fn http_resp_stream(r: opaque) -> opaque;
//! // stream:
//! pub host async fn http_body(r: opaque) -> bytes;
//! pub host async fn http_stream_next(s: opaque) -> ?bytes;
//! ```
//!
//! The rows register path-qualified — the scope-relative spellings
//! below (`"net::http_send__start"` et al, `"stream::http_body"`);
//! the pkg scope prefixes once, at the install.
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
//! Two lanes build these bodies: [`pkg_with`] rides an
//! injectable fixture closure keyed on method+URL(+headers/body) and
//! walks its chunk schedule on the VIRTUAL clock (the settle handle is
//! the returned [`HttpFixture`] — the test's driving loop advances the
//! clock and settles the dues); [`pkg`] is the reqwest
//! lane (one shared blocking client, one worker thread per request).
//! The `http` cargo feature is DEFAULT-OFF — reqwest-blocking does not
//! build on wasm32-unknown-unknown, so only native embedders opt in.

use std::collections::VecDeque;
use std::io::Read as _;
use std::sync::{Arc, Mutex};

use rut_core::types::{TypeId, TY_OPAQUE};
use rut_vm::interp::Ret;
use rut_vm::interp::{HostPkg, Vm};
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
        HttpResponse {
            status: 0,
            err: Some(msg),
            read: Arc::new(Mutex::new(Inner::dead())),
        }
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
fn install_read_rows(pkg: &mut HostPkg) {
    // the one-shot drain: the WHOLE remaining body in one future. A
    // transport-failed response, a second taker (body twice, or body
    // after a stream mint) degrade to EMPTY — the disclosed law.
    rut_vm::pkg_async_fn!(
        pkg,
        "stream::http_body",
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
                if g.failed.is_some() {
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
    rut_vm::pkg_async_fn!(
        pkg,
        "stream::http_stream_next",
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
    rut_vm::pkg_fn!(
        pkg,
        "net::http_status",
        (Opaque<HttpResponse>,) -> i32,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<i32, Trap> {
            r.with(|resp| resp.status as i32)
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "net::http_err",
        (Opaque<HttpResponse>,) -> Option<String>,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<Option<String>, Trap> {
            r.with(|resp| resp.err.clone())
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "net::http_read_err",
        (Opaque<HttpResponse>,) -> Option<String>,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<Option<String>, Trap> {
            r.with(|resp| resp.read.lock().expect("http read state").failed.clone())
        },
    );
    // the reader mint: instant, sync. The FIRST mint takes the stream
    // lane (body is now refused); a SECOND mint answers a dead reader
    // (immediate EOF) — the one-shot law's degrade, never a trap.
    rut_vm::pkg_fn!(
        pkg,
        "net::http_resp_stream",
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
    Chunk {
        src: Shared,
        chunk: Vec<u8>,
    },
    Terminal {
        src: Shared,
    },
    Fail {
        src: Shared,
        msg: String,
    },
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
        dues.push((
            0,
            Due::Send {
                reply: send_reply,
                comp: comp.clone(),
                src: Arc::clone(&src),
            },
        ));
        if let Ok(r) = &reply {
            let mut t = 0u64;
            for (i, chunk) in r.chunks.iter().enumerate() {
                if r.fail_after == Some(i) {
                    break; // the wire died before this chunk
                }
                t += 1;
                dues.push((
                    t,
                    Due::Chunk {
                        src: Arc::clone(&src),
                        chunk: chunk.clone(),
                    },
                ));
            }
            t += 1;
            match r.fail_after {
                Some(after) if after <= r.chunks.len() => dues.push((
                    t,
                    Due::Fail {
                        src: Arc::clone(&src),
                        msg: r.fail_msg.clone(),
                    },
                )),
                _ => dues.push((
                    t,
                    Due::Terminal {
                        src: Arc::clone(&src),
                    },
                )),
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
pub fn pkg_with<F>(f: F) -> (HostPkg, HttpFixture)
where
    F: Fn(&str, &str, &str, &[u8]) -> Result<FixtureReply, String> + 'static,
{
    let fixture = HttpFixture::default();
    let fx = fixture.clone();
    let mut pkg = HostPkg::new("http_host");
    rut_vm::pkg_async_fn!(
        pkg,
        "net::http_send",
        (OpaqueRef, String, String, String, Vec<u8>) -> HttpResponse,
        move |c: OpaqueRef, method: String, url: String, headers: String, body: Vec<u8>|
              -> CompleterOf<HttpResponse> {
            let _ = (c, &headers, &body); // the lanes own their client state
            let reply = f(&method, &url, &headers, &body);
            fx.arm(reply)
        },
    );
    install_read_rows(&mut pkg);
    (pkg.build(), fixture)
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
pub fn pkg() -> HostPkg {
    let mut pkg = HostPkg::new("http_host");
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            .user_agent("rgh/0.1 (+https://github.com/rut)")
            .redirect(reqwest::redirect::Policy::default())
            .build()
            .expect("the std http client builds"),
    );
    rut_vm::pkg_async_fn!(
        pkg,
        "net::http_send",
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
    install_read_rows(&mut pkg);
    pkg.build()
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
    w.complete(HttpResponse {
        status,
        err: None,
        read: Arc::clone(&src),
    });
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
