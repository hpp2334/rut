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
//! - THE STREAM BUDGET: the queue is BOUNDED — once the queued bytes
//!   sit at the budget ([`HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES`],
//!   64 × 16 KiB ≈ 1 MiB) the producer PARKS on the source's condvar
//!   until readers drain below it: real TCP flow control (the pump
//!   stalls, the socket fills, the server feels it), every
//!   [`pkg_over`] backend inherits it. One poll per `await next()` is
//!   the consumer pacing — nothing reads ahead. The one-shot body
//!   drain is EXEMPT (its answer IS the whole body — gating buys
//!   nothing): taking the body lane stands the gate down.
//! - CANCEL IS THE WIRE-ABORT: a PARKED read/drain/send registers its
//!   completer's identity ([`rut_vm::Completer::id`], the Arc address)
//!   in the per-pkg [`CancelReg`]; the row family's `__cancel` arm
//!   takes the slot and marks the source with the sticky read failure
//!   `"cancelled"` — remaining readers degrade to nil +
//!   `read_error() == "cancelled"` (the data-not-a-trap lane), a
//!   budget-parked pump releases at the gate, and a pump that checks
//!   [`HttpBodyWriter::cancelled`] between chunks (the reqwest lane
//!   does) DROPS ITS RESPONSE — the connection closes. Disclosed:
//!   a blocking read already in flight still completes (cancel is
//!   between-chunk), and a pre-headers cancel of `http_send` finds no
//!   slot — best-effort, the late completion is simply never taken.
//!
//! THREAD LAW: the VM stays single-threaded. Worker threads touch only
//! the Completers (atomics + a Mutex slot) and the chunk source (one
//! `Mutex<Inner>` + the sibling condvar the pump parks on at the
//! budget — waiters complete and the budget notify fires OUTSIDE the
//! lock; the slot Mutex and this Mutex never nest). The response and
//! stream rut cells are minted by the rows ON the VM thread.
//! `std::thread` appears only in the reqwest lane's closures.
//!
//! THE SWAPPABLE-BACKEND SEAM: the lane-independent core above is the
//! public embedder API. [`HttpRequest`] carries the crossing parts,
//! [`HttpResponse::streaming`] mints the response + its
//! [`HttpBodyWriter`] at headers time, [`HttpResponse::failed`] mints
//! a transport-dead one, and [`HttpBackend`] is the ONE method an
//! embedder implements to bring their own transport (async-native on
//! THEIR runtime — rut-std stays tokio-free; Gap 5 is answered by the
//! seam, not shipped). [`pkg_over`] binds all seven `http_host` rows
//! over any backend.
//!
//! Three doors build these bodies, ALL over the one seam:
//! [`pkg_over`] rides YOUR backend; [`pkg_with`] rides an injectable
//! fixture closure keyed on method+URL(+headers/body) and walks its
//! chunk schedule on the VIRTUAL clock (the settle handle is the
//! returned [`HttpFixture`] — the test's driving loop advances the
//! clock and settles the dues); [`pkg`] is the reqwest lane (one
//! shared blocking client, one worker thread per request) — the
//! reference [`HttpBackend`]. The `http` cargo feature gates ONLY the
//! reqwest lane and is DEFAULT-OFF (reqwest-blocking does not build
//! on wasm32-unknown-unknown); the seam itself is feature-free — a
//! wasm embedder brings its own backend instead.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};

use rut_core::types::{TypeId, TY_OPAQUE};
use rut_vm::interp::Ret;
use rut_vm::interp::{HostPkg, Vm};
use rut_vm::{HostPayload, Opaque, OpaqueRef, Slot, Trap, TrapKind};

// ------------------------------------------------------- the cancel ---
//
// The `__cancel` arms' lane (the wire-abort): a PARKED host future
// (`http_send` at its headers, `http_body`/`http_stream_next` at their
// waiters) registers its completer's identity under the chunk source
// its cancel arm must stop. The completer id ([`rut_vm::Completer::id`],
// the Arc address) is the key; the mark is the sticky read failure
// `"cancelled"` — the same data-not-a-trap lane as any mid-read death,
// so remaining readers degrade to nil + `read_error() == "cancelled"`
// and a budget-parked pump releases at the gate.

/// the sticky failure text a cancel lands — `read_error()` names it
const CANCELLED: &str = "cancelled";

/// the registry entry's map type — the Weak back-link on the source
/// spells it too (the resolve sites' cleanup bridge)
type RegMap = HashMap<usize, Shared>;

/// The per-pkg cancel registry: completer id → the source whose pump
/// must stop when that future's frame is cancelled. One per built pkg
/// (`pkg_send_with_reg` mints it; the rows' closures and the lane that
/// completes the send share the clone). Entries live ONLY while the
/// future is parked: inserted at the park, removed at the cancel arm —
/// or at the natural resolve, through the source's [`Source::forget`]
/// back-link (a resolved future's frame retires at its take, so its
/// cancel arm can never fire again).
#[derive(Clone, Default)]
pub(crate) struct CancelReg(Arc<Mutex<RegMap>>);

impl CancelReg {
    /// register a parked future's source. Stamps the source's back-link
    /// (first insert wins) so the resolve sites can retire the entry.
    pub(crate) fn insert(&self, id: usize, src: &Shared) {
        let _ = src.back.set(Arc::downgrade(&self.0));
        self.0
            .lock()
            .expect("http cancel registry")
            .insert(id, Arc::clone(src));
    }
    /// the cancel arm: take the slot and mark what it names. A miss is
    /// the disclosed best-effort law (the future was never parked, or
    /// already resolved — pre-headers cancel of `http_send`).
    fn take(&self, id: usize) -> Option<Shared> {
        self.0.lock().expect("http cancel registry").remove(&id)
    }
    /// retire the slot without marking (the pump's own end — the
    /// reqwest lane's and the fetch lane's alike: EOF, a mid-read
    /// failure, or the wire-abort's between-chunk drop all land here).
    #[cfg(any(feature = "http", all(feature = "http-wasm", target_arch = "wasm32")))]
    pub(crate) fn remove(&self, id: usize) {
        self.0.lock().expect("http cancel registry").remove(&id);
    }
}

/// the three async rows' shared `__cancel` arm: the registry slot for
/// the cancelled future's completer names the source — the sticky
/// `cancelled` failure lands (readers degrade, the gate releases), the
/// reqwest pump drops its response at its next between-chunk check.
/// A miss is the disclosed best-effort law: never a trap.
fn abort_parked(reg: &CancelReg, id: usize) {
    if let Some(src) = reg.take(id) {
        fail_source(&src, CANCELLED.to_string());
    }
}

// ----------------------------------------------------- the payload ----

/// The chunk source's state, shared between the response, its minted
/// reader and the worker thread: the queue the worker pushes into, its
/// byte weight (`queued` — what the budget gate reads), the sticky
/// mid-read failure, the EOF flag, and the two one-shot lanes' pending
/// completer slots (one per read — the sequenced-Completer sibling the
/// plan spells). One Mutex; workers complete parked Completers from
/// OUTSIDE the lock (the thread law: the slot Mutex and this Mutex
/// never nest).
pub(crate) struct Inner {
    /// the arrived chunks — `queued` is their byte weight, kept in
    /// step at every push / pop / drain (the budget gate's number)
    chunks: VecDeque<Vec<u8>>,
    queued: usize,
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
/// The chunk source's cells: the state mutex and the SIBLING condvar
/// the pump lane parks on while the queue sits at the budget (the
/// wait releases the mutex; the notify fires after it — the thread
/// law). The alias is the shared handle everything holds. `back` is
/// the cancel registry this source was first registered under (the
/// read lanes' resolve sites retire their entries through it; Weak —
/// the registry dies with its pkg's bindings, a dead one skips the
/// cleanup).
pub(crate) struct Source {
    g: Mutex<Inner>,
    room: Condvar,
    back: OnceLock<Weak<Mutex<RegMap>>>,
}

pub(crate) type Shared = Arc<Source>;

impl Source {
    fn fresh() -> Shared {
        Arc::new(Source {
            g: Mutex::new(Inner::fresh()),
            room: Condvar::new(),
            back: OnceLock::new(),
        })
    }
    fn dead() -> Shared {
        Arc::new(Source {
            g: Mutex::new(Inner::dead()),
            room: Condvar::new(),
            back: OnceLock::new(),
        })
    }
    /// the one lock (the expect is the house panic text)
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.g.lock().expect("http read state")
    }
    /// retire the registry slot of a completer that resolved the
    /// NATURAL way (a parked read this source just woke): its frame
    /// takes the answer and retires, so the cancel arm can never fire
    /// again — the slot goes with it.
    fn forget(&self, id: usize) {
        if let Some(reg) = self.back.get().and_then(|w| w.upgrade()) {
            reg.lock().expect("http cancel registry").remove(&id);
        }
    }
}

impl Inner {
    fn fresh() -> Inner {
        Inner {
            chunks: VecDeque::new(),
            queued: 0,
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
    /// the budget gate's predicate: the queue sits at/over the budget
    /// (the gate bounds PUSHES, not chunk sizes — one push may take the
    /// queue one chunk past; the next parks until readers drain below)
    fn over_budget(&self) -> bool {
        self.queued >= HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES
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
    /// Mint a response at HEADERS time plus the writer that drives
    /// its body: a backend completes the send completer with the
    /// response half, then pumps the body through the writer half
    /// (from any thread). The body stays UNREAD until a taker comes —
    /// the one-shot law starts here.
    pub fn streaming(status: u16) -> (HttpResponse, HttpBodyWriter) {
        let src: Shared = Source::fresh();
        (
            HttpResponse {
                status,
                err: None,
                read: Arc::clone(&src),
            },
            HttpBodyWriter { src },
        )
    }

    /// Mint a transport-failed response: status 0 + the message (the
    /// kept law); the chunk source is born dead, so the body/stream
    /// lanes degrade to empty / immediate EOF.
    pub fn failed(msg: String) -> HttpResponse {
        HttpResponse {
            status: 0,
            err: Some(msg),
            read: Source::dead(),
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

// -------------------------------------------------------- the seam ---
//
// The embedder's own-backend API: the crossing parts, the body
// writer, the ONE-method transport trait, and the pkg door that binds
// all seven rows over any backend. Feature-free by law — a wasm
// embedder brings its own transport instead of reqwest.

/// The crossing parts of a request — exactly what the VM lane hands a
/// backend. `headers` is the builder's `"\n"`-joined `Name: value`
/// block (empty = none); `body` is the raw bytes (empty = none — the
/// disclosed empty-vs-none merge).
#[derive(Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: String,
    pub body: Vec<u8>,
}

/// The body driver a backend keeps after answering at headers:
/// `push_chunk` feeds one chunk (a parked stream read takes the
/// queue's front immediately, FIFO), `end` is the clean EOF, `fail`
/// the sticky mid-read death (the queued chunks stay readable — they
/// did arrive). Callable from ANY thread; one Mutex inside, never
/// nested with a Completer slot (the thread law). The terminal flags
/// are sticky — the read side answers EOF/failed from the first
/// terminal on.
///
/// THE BOUNDED LANE: `push_chunk` is the budget gate's home — past
/// [`HttpBodyWriter::DEFAULT_STREAM_BUDGET_BYTES`] queued it PARKS the
/// calling thread on the source's condvar until readers drain below
/// the budget (real TCP flow control: the pump stalls, the socket
/// fills, the server feels it). MAY BLOCK — the worker thread only
/// (the backend contract's never-block-the-VM law). The one-shot body
/// drain stands the gate down (its answer IS the whole body).
pub struct HttpBodyWriter {
    src: Shared,
}

impl HttpBodyWriter {
    /// the stream budget: the queue bound the pump gate enforces —
    /// 64 × 16 KiB chunks ≈ 1 MiB. Past it [`HttpBodyWriter::push_chunk`]
    /// parks the producer until readers drain below; the one-shot body
    /// lane is exempt.
    pub const DEFAULT_STREAM_BUDGET_BYTES: usize = 64 * 16 * 1024;

    /// one more chunk arrived — BOUNDED: while the queue sits at the
    /// budget and the one-shot body lane hasn't taken over (its answer
    /// IS the whole body — gating buys nothing there), the caller
    /// parks on the source's condvar until readers drain below it. A
    /// terminal seen under the lock (a watchdog `end`/`fail` from
    /// another thread) releases the park. Consumer pacing propagates
    /// end to end: one poll per `await next()` pops one chunk, the pop
    /// notifies, the pump re-checks.
    pub fn push_chunk(&self, chunk: Vec<u8>) {
        {
            let mut g = self.src.lock();
            while g.over_budget() && !g.body_taken && !g.eof && g.failed.is_none() {
                g = self.src.room.wait(g).expect("http read state");
            }
        }
        push_chunk(&self.src, chunk);
    }

    /// clean EOF — the body lane drains, the stream lane sees nil
    pub fn end(&self) {
        end_source(&self.src);
    }

    /// the wire died mid-read — the sticky failure lands (data,
    /// never a trap)
    pub fn fail(&self, msg: String) {
        fail_source(&self.src, msg);
    }

    /// the wire-abort probe a pump lane checks BETWEEN chunks: true
    /// once a cancel marked this source (the `__cancel` arm's sticky
    /// failure). The reqwest pump drops its response on true — the
    /// connection closes with the cancelled frame. A blocking read
    /// already in flight still completes first (cancel is
    /// between-chunk — the disclosed law).
    pub fn cancelled(&self) -> bool {
        self.src.lock().failed.as_deref() == Some(CANCELLED)
    }

    /// THE BUDGET GATE, wasm flavor: true while a `push_chunk` would
    /// park the producer on the condvar gate — the queue at the
    /// budget, the one-shot body lane not taken over, no terminal
    /// yet. The JS thread has no condvar (a park there blocks the
    /// whole world), so the fetch pump checks this BETWEEN reads and
    /// parks by re-arming itself one macrotask at a time; the guest's
    /// one-pop-per-await pacing drains below the budget and the
    /// parked pump resumes. Feature-free — it reads the same cells
    /// the condvar gate does, so every backend sees one predicate.
    pub fn parked_at_budget(&self) -> bool {
        let g = self.src.lock();
        g.over_budget() && !g.body_taken && !g.eof && g.failed.is_none()
    }

    /// the chunk source this writer drives — the cancel registry's
    /// insert target (the send slot retires through it at every pump
    /// end, the reqwest lane's bookkeeping shared by the fetch lane;
    /// the reqwest pump reaches the field directly — same module).
    #[cfg(all(feature = "http-wasm", target_arch = "wasm32"))]
    pub(crate) fn source(&self) -> &Shared {
        &self.src
    }
}

/// The embedder's transport: the ONE method a custom backend
/// implements. The contract:
///
/// - answer `out` EXACTLY ONCE — at HEADERS, completing with
///   [`HttpResponse::streaming`]'s response half, or with
///   [`HttpResponse::failed`] on transport trouble;
/// - then drive the minted [`HttpBodyWriter`] from any thread;
/// - NEVER block the VM thread: do the wire work on your own
///   threads/tasks — the completer and the writer (atomics + Mutex
///   slots) are the only shared state (the thread law).
///
/// A double complete is NOT loud — the `Completer`'s disclosed
/// contract is LAST-WRITE-WINS (the second answer overwrites the
/// slot, the first is silently lost; a late answer after a take is
/// never taken). The seam suite pins exactly that; "exactly once"
/// above is the discipline, not an enforced invariant.
pub trait HttpBackend: Send + Sync + 'static {
    fn send(&self, req: &HttpRequest, out: rut_vm::Completer<HttpResponse>);
}

// ------------------------------------------------ the chunk wakeups ---
//
// Three transitions, all "mutate under one lock, complete the parked
// waiters OUTSIDE it". Callable from any thread: the reqwest pump
// (a worker) and the fixture settle (the test loop) land here alike.

/// one more chunk arrived — a parked stream read takes the queue's
/// front (FIFO), a parked body read keeps waiting (it drains at EOF).
/// UNGATED on purpose: the fixture's virtual-clock lane rides this
/// path ON the VM thread (a park here would deadlock the driving
/// loop); the BUDGET gate lives on [`HttpBodyWriter::push_chunk`] —
/// every real backend's lane.
fn push_chunk(src: &Shared, chunk: Vec<u8>) {
    let wake = {
        let mut g = src.lock();
        let n = chunk.len();
        g.chunks.push_back(chunk);
        g.queued += n;
        g.stream_waiter.take().and_then(|w| {
            g.chunks.pop_front().map(|front| {
                g.queued -= front.len();
                (w, front)
            })
        })
    };
    if let Some((w, front)) = wake {
        // the parked read resolves — its cancel slot retires with it
        // (the frame takes this chunk and retires; the arm can't fire)
        src.forget(w.id());
        w.complete(Some(front)); // the queue is non-empty — always a chunk
    }
}

/// the wire died mid-read — the sticky failure lands, the stream lane
/// gets nil, the body lane the short drain (never a trap)
fn fail_source(src: &Shared, msg: String) {
    let wake = {
        let mut g = src.lock();
        g.failed = Some(msg);
        // the drain is a COPY — the queued chunks stay for the stream
        // lane's later reads (they did arrive; next serves them before
        // the nil the failure answers)
        let drain: Vec<u8> = g.chunks.iter().flatten().cloned().collect();
        (g.stream_waiter.take(), g.body_waiter.take(), drain)
    };
    let (sw, bw, drain) = wake;
    // release a pump parked at the budget (a terminal stands the gate
    // down — it re-checks under the lock and moves on)
    src.room.notify_all();
    if let Some(w) = sw {
        src.forget(w.id());
        w.complete(None);
    }
    if let Some(w) = bw {
        src.forget(w.id());
        w.complete(drain);
    }
}

/// clean EOF — the stream lane's parked read gets nil, the body lane
/// its full drain
fn end_source(src: &Shared) {
    let wake = {
        let mut g = src.lock();
        g.eof = true;
        // a copy, same law: queued chunks stay readable by the stream
        // lane after the EOF flag; the body lane's drain copies them
        let drain: Vec<u8> = g.chunks.iter().flatten().cloned().collect();
        (g.stream_waiter.take(), g.body_waiter.take(), drain)
    };
    let (sw, bw, drain) = wake;
    // release a pump parked at the budget (same law as the failure)
    src.room.notify_all();
    if let Some(w) = sw {
        src.forget(w.id());
        w.complete(None);
    }
    if let Some(w) = bw {
        src.forget(w.id());
        w.complete(drain);
    }
}

// ---------------------------------------------------------- the rows ---

/// The lane-independent rows: the body drain, the stream read, and the
/// five sync readbacks. Both lanes register these — only `http_send`
/// differs (a worker thread vs the virtual clock). `reg` is the pkg's
/// cancel registry: a PARKED read registers its completer under it and
/// its row rides the 5-arg abort form — the `__cancel` arm marks the
/// source (readers degrade to nil + the named error, the gate
/// releases, the pump stops between chunks).
fn install_read_rows(pkg: &mut HostPkg, reg: &CancelReg) {
    // the one-shot drain: the WHOLE remaining body in one future. A
    // transport-failed response, a second taker (body twice, or body
    // after a stream mint) degrade to EMPTY — the disclosed law.
    let reg_start = reg.clone();
    let start_body = move |r: Opaque<HttpResponse>| -> CompleterOf<Vec<u8>> {
        let reg = reg_start.clone();
        let comp = CompleterOf::new();
        let mut immediate: Option<Vec<u8>> = None;
        // did this call TAKE the body lane (vs degrade)? only a
        // take stands the budget gate down
        let mut took = false;
        // did the drain PARK (the EOF/failure is still on the wire)?
        // a parked drain is the one that registers its cancel slot
        let mut parked = false;
        let mut src: Option<Shared> = None;
        r.with(|resp| {
            let mut g = resp.read.lock();
            src = Some(Arc::clone(&resp.read));
            if resp.err.is_some() || g.body_taken || g.stream_minted {
                immediate = Some(Vec::new());
                return;
            }
            g.body_taken = true;
            took = true;
            if g.failed.is_some() {
                let short: Vec<u8> = g.chunks.drain(..).flatten().collect();
                g.queued -= short.len();
                immediate = Some(short); // the short drain
                return;
            }
            if g.eof {
                let rest: Vec<u8> = g.chunks.drain(..).flatten().collect();
                g.queued -= rest.len();
                immediate = Some(rest);
                return;
            }
            g.body_waiter = Some(comp.clone());
            parked = true;
        })
        .expect("http_body: the response handle is live");
        if took {
            // the body lane took over: STAND THE GATE DOWN (its
            // answer IS the whole body — gating buys nothing) and
            // release a pump parked at the budget — the drain
            // waits for EOF, and the parked pump is the only one
            // who can reach it
            if let Some(s) = src.as_ref() {
                s.room.notify_all();
            }
        }
        if parked {
            if let Some(s) = src.as_ref() {
                reg.insert(comp.id(), s);
            }
        }
        if let Some(bytes) = immediate {
            comp.complete(bytes);
        }
        comp
    };
    let abort_body = {
        let reg = reg.clone();
        move |c: CompleterOf<Vec<u8>>| abort_parked(&reg, c.id())
    };
    rut_vm::pkg_async_fn!(
        pkg,
        "stream::http_body",
        (Opaque<HttpResponse>,) -> Vec<u8>,
        start_body,
        abort_body,
    );
    // the stream read: one chunk per future; nil = EOF-or-failed (the
    // sticky read err names which). A degraded mint answers an
    // immediate EOF. A failed completer is NOT the mid-read path.
    // ONE POLL PER AWAIT: exactly one pop here — the consumer's pace
    // is the pump's pace (the pop notifies the parked pump).
    let reg_start = reg.clone();
    let start_next = move |s: Opaque<HttpStream>| -> CompleterOf<Option<Vec<u8>>> {
        let reg = reg_start.clone();
        let comp = CompleterOf::new();
        let mut immediate: Option<Option<Vec<u8>>> = None;
        let mut popped = false;
        let mut parked = false;
        let mut src: Option<Shared> = None;
        s.with(|st| {
            if st.dead {
                immediate = Some(None);
                return;
            }
            src = Some(Arc::clone(&st.src));
            let mut g = st.src.lock();
            if let Some(chunk) = g.chunks.pop_front() {
                g.queued -= chunk.len();
                popped = true;
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
            parked = true;
        })
        .expect("http_stream_next: the stream handle is live");
        if popped {
            // a chunk left the queue — the parked pump re-checks
            // the budget (real flow control: one read, one push's
            // worth of room)
            if let Some(s) = src.as_ref() {
                s.room.notify_all();
            }
        }
        if parked {
            if let Some(s) = src.as_ref() {
                reg.insert(comp.id(), s);
            }
        }
        if let Some(ans) = immediate {
            comp.complete(ans);
        }
        comp
    };
    let abort_next = {
        let reg = reg.clone();
        move |c: CompleterOf<Option<Vec<u8>>>| abort_parked(&reg, c.id())
    };
    rut_vm::pkg_async_fn!(
        pkg,
        "stream::http_stream_next",
        (Opaque<HttpStream>,) -> Option<Vec<u8>>,
        start_next,
        abort_next,
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
            r.with(|resp| resp.read.lock().failed.clone())
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
                let mut g = resp.read.lock();
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

// ------------------------------------------------- the one installer —

/// The send row over ANY answering closure — the ONE installer every
/// door rides ([`pkg_over`], [`pkg_with`], [`pkg`], and the wasm
/// fetch lane's `pkg_fetch` over in [`crate::http_wasm`]). `send`
/// receives the crossing parts and the completer; it answers exactly
/// once and then drives the writer. The decl-row arity (the opaque
/// cx + the four crossing values) lives HERE once — no lane re-spells
/// it. `reg` is the pkg's cancel registry, captured by every row's
/// abort arm; the send lane's own slot is registered by whoever
/// completes the send completer: the reqwest pump at headers, the
/// fetch pump at headers, the fixture's settle on the VM thread. A
/// custom [`HttpBackend`] that never registers keeps the disclosed
/// best-effort law for its lane.
pub(crate) fn pkg_send_with_reg<S>(send: S, reg: CancelReg) -> HostPkg
where
    S: Fn(&HttpRequest, rut_vm::Completer<HttpResponse>) + 'static,
{
    let mut pkg = HostPkg::new("http_host");
    let abort_send = {
        let reg = reg.clone();
        move |c: CompleterOf<HttpResponse>| abort_parked(&reg, c.id())
    };
    rut_vm::pkg_async_fn!(
        pkg,
        "net::http_send",
        (OpaqueRef, String, String, String, Vec<u8>) -> HttpResponse,
        move |c: OpaqueRef, method: String, url: String, headers: String, body: Vec<u8>|
              -> CompleterOf<HttpResponse> {
            let _ = c;
            let comp = CompleterOf::new();
            send(
                &HttpRequest {
                    method,
                    url,
                    headers,
                    body,
                },
                comp.clone(),
            );
            comp
        },
        abort_send,
    );
    install_read_rows(&mut pkg, &reg);
    pkg.build()
}

/// Bind ALL seven `http_host` rows over any backend: `http_send`
/// marshals the crossing parts into an [`HttpRequest`], hands the
/// backend the send future's completer, and answers cold — the
/// backend's own thread/task completes it at headers (via
/// [`HttpResponse::streaming`]) or with [`HttpResponse::failed`], then
/// drives the writer; the five sync readbacks and the two body lanes
/// are the shared rows. The decl↔body join is pinned in
/// `tests/http.rs` (`the_decl_rows_join_both_lanes`).
pub fn pkg_over(backend: Arc<dyn HttpBackend>) -> HostPkg {
    pkg_send_with_reg(move |req, out| backend.send(req, out), CancelReg::default())
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
    /// the pkg's cancel registry — the send settle registers the send
    /// future's slot under it (the read lanes share the same registry
    /// through their rows' closures)
    reg: CancelReg,
}

/// The fixture lane's settle handle: the test loop's side of the
/// virtual clock. Clone = another handle on the same dues.
#[derive(Clone, Default)]
pub struct HttpFixture(std::rc::Rc<FixtureState>);

impl HttpFixture {
    /// the settle handle over the pkg's cancel registry — the Send
    /// settle registers the send future's slot under it
    fn with_reg(reg: CancelReg) -> HttpFixture {
        HttpFixture(std::rc::Rc::new(FixtureState {
            dues: std::cell::RefCell::new(Vec::new()),
            reg,
        }))
    }
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
                        Ok(r) => {
                            // the send future's cancel slot, registered
                            // BEFORE the completion (a loser-cancel in
                            // the resolve window must find it)
                            self.0.reg.insert(comp.id(), &src);
                            comp.complete(HttpResponse {
                                status: r.status,
                                err: None,
                                read: Arc::clone(&src),
                            });
                        }
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
    /// arm a request's schedule: the send at 0 (answering the GIVEN
    /// completer — the seam's `send` contract), chunk i at i+1, the
    /// terminal one tick after the last released chunk
    fn arm(&self, reply: Result<FixtureReply, String>, comp: CompleterOf<HttpResponse>) {
        let src: Shared = Source::fresh();
        let mut dues = self.0.dues.borrow_mut();
        // the body schedule is built from a borrow; the send due moves
        // `reply` and lands first, so the tick-0 send still fires
        // before its tick-i chunks when a settle covers both
        let mut later: Vec<(u64, Due)> = Vec::new();
        if let Ok(r) = &reply {
            let mut t = 0u64;
            for (i, chunk) in r.chunks.iter().enumerate() {
                if r.fail_after == Some(i) {
                    break; // the wire died before this chunk
                }
                t += 1;
                later.push((
                    t,
                    Due::Chunk {
                        src: Arc::clone(&src),
                        chunk: chunk.clone(),
                    },
                ));
            }
            t += 1;
            match r.fail_after {
                Some(after) if after <= r.chunks.len() => later.push((
                    t,
                    Due::Fail {
                        src: Arc::clone(&src),
                        msg: r.fail_msg.clone(),
                    },
                )),
                _ => later.push((
                    t,
                    Due::Terminal {
                        src: Arc::clone(&src),
                    },
                )),
            }
        }
        dues.push((0, Due::Send { reply, comp, src }));
        dues.extend(later);
    }
}

/// Install the fixture lane over an injectable request closure: `f`
/// maps (method, url, headers, body) to the recorded reply — or
/// `Err(message)` for a transport failure, which lands as status 0
/// with the message in `http_err` and a born-dead body. The fixture
/// map's keys ARE the assertions: an unknown URL is a LOUD transport
/// failure, never a pass-through. Returns the settle handle the test's
/// driving loop walks. Rides the ONE seam installer: the closure
/// answers ON the VM thread (the virtual-clock lane spawns no
/// workers); the schedule fires through [`HttpFixture::settle`].
pub fn pkg_with<F>(f: F) -> (HostPkg, HttpFixture)
where
    F: Fn(&str, &str, &str, &[u8]) -> Result<FixtureReply, String> + 'static,
{
    let reg = CancelReg::default();
    let fixture = HttpFixture::with_reg(reg.clone());
    let fx = fixture.clone();
    let pkg = pkg_send_with_reg(
        move |req: &HttpRequest, out: CompleterOf<HttpResponse>| {
            let reply = f(&req.method, &req.url, &req.headers, &req.body);
            fx.arm(reply, out);
        },
        reg,
    );
    (pkg, fixture)
}

// ----------------------------------------------------- the reqwest ----

/// The reqwest reference backend (the `http` cargo feature): one
/// shared blocking client, one worker thread PER REQUEST — send,
/// complete at HEADERS, keep pumping the body into the chunk source
/// on the same thread.
#[cfg(feature = "http")]
struct ReqwestBackend {
    client: Arc<reqwest::blocking::Client>,
    /// the pkg's cancel registry — the pump registers the send
    /// future's slot at headers (the wire-abort mark's target) and
    /// retires it at the pump's own end
    reg: CancelReg,
}

#[cfg(feature = "http")]
impl HttpBackend for ReqwestBackend {
    fn send(&self, req: &HttpRequest, out: rut_vm::Completer<HttpResponse>) {
        let client = Arc::clone(&self.client);
        let req = req.clone();
        let reg = self.reg.clone();
        // the worker owns the request end to end: send, complete at
        // HEADERS, then pump the body into the chunk source. The
        // cancel arm marks the source through the registry — the pump
        // checks it between chunks and drops the response (the
        // connection closes: wire-abort). A cancel before the headers
        // finds no slot: the disclosed best-effort law — the thread
        // runs to its blocking completion and the late result is
        // simply never taken.
        std::thread::spawn(move || send_once(&client, &req, &out, &reg));
    }
}

/// Install the reqwest lane: the reference [`HttpBackend`] over one
/// shared blocking client (this UA, redirects on — the default
/// policy's ten hops; gzip/brotli arrive with the cargo features).
/// `http_send` resolves at HEADERS — the worker completes the send
/// completer the moment the status is known, then keeps reading the
/// (still unread) response body into the chunk source on the same
/// thread. Transport failure maps to status 0 + the message (the kept
/// law). Requires the `http` cargo feature.
#[cfg(feature = "http")]
pub fn pkg() -> HostPkg {
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            .user_agent("rgh/0.1 (+https://github.com/rut)")
            .redirect(reqwest::redirect::Policy::default())
            .build()
            .expect("the std http client builds"),
    );
    let reg = CancelReg::default();
    let backend = Arc::new(ReqwestBackend {
        client,
        reg: reg.clone(),
    });
    pkg_send_with_reg(move |req, out| backend.send(req, out), reg)
}

/// The reqwest worker: build the request (canonical method, the
/// "\n"-joined `Name: value` header block, bytes with empty = none —
/// the disclosed empty-vs-none merge), execute, complete the send at
/// HEADERS via [`HttpResponse::streaming`], then pump the writer
/// until EOF or wire death. `reg` carries the pkg's cancel registry:
/// the send future's slot registers at headers (before the completion
/// — a loser-cancel in the resolve window must find it) and retires
/// at the pump's end; a mark from the cancel arm stops the pump at
/// its next between-chunk check — the response drops, the connection
/// closes (wire-abort).
#[cfg(feature = "http")]
fn send_once(
    client: &reqwest::blocking::Client,
    req: &HttpRequest,
    w: &rut_vm::Completer<HttpResponse>,
    reg: &CancelReg,
) {
    use std::io::Read as _;
    let send = (|| -> Result<reqwest::blocking::Response, String> {
        let m = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| format!("bad method `{}`: {e}", req.method))?;
        let mut rq = client.request(m, &req.url);
        for line in req.headers.split('\n') {
            if line.trim().is_empty() {
                continue;
            }
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("bad header line `{line}` (want `Name: value`)"))?;
            rq = rq.header(name.trim(), value.trim());
        }
        if !req.body.is_empty() {
            rq = rq.body(req.body.clone());
        }
        rq.send().map_err(|e| e.to_string())
    })();
    let mut resp = match send {
        Err(msg) => {
            w.complete(HttpResponse::failed(msg));
            return;
        }
        Ok(resp) => resp,
    };
    // HEADERS — the send resolves here; the body stays unread
    let (answer, body) = HttpResponse::streaming(resp.status().as_u16());
    reg.insert(w.id(), &body.src);
    w.complete(answer);
    // the pump: this thread keeps reading the wire into the writer. A
    // fixed 16 KiB buffer — bounded memory end to end, and each read
    // lands as one stream chunk. The writer's BUDGET gate backs the
    // pressure all the way out: at the budget the park stalls this
    // loop, the socket fills, the server feels it (real flow control).
    // Between chunks the CANCEL probe runs: a marked source ends the
    // pump here — `resp` drops, the connection closes (wire-abort).
    let mut buf = [0u8; 16 * 1024];
    loop {
        if body.cancelled() {
            reg.remove(w.id());
            return;
        }
        match resp.read(&mut buf) {
            Ok(0) => {
                reg.remove(w.id());
                body.end();
                return;
            }
            Ok(n) => body.push_chunk(buf[..n].to_vec()),
            Err(e) => {
                reg.remove(w.id());
                body.fail(e.to_string());
                return;
            }
        }
    }
}
