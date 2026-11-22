//! The std HTTP lane's wasm door (the fetch lane): [`pkg_fetch`] binds
//! the seven `http_host` rows over the browser window's `fetch` — the
//! playground's transport, and the swappable-backend seam's second
//! reference backend beside the reqwest lane (which stays native).
//!
//! THE SHAPE (the seam contract — answer `out` exactly once, then
//! drive the writer, never block the VM thread):
//!
//! - `http_send` resolves at HEADERS: one `spawn_local` task awaits
//!   `window.fetch(..)`, reads the status, mints
//!   [`HttpResponse::streaming`], registers the send future's cancel
//!   slot (BEFORE the completion — a loser-cancel in the resolve
//!   window must find it, the reqwest lane's order), completes the
//!   send completer, and keeps pumping the body on the same task.
//! - The body rides the stream's DEFAULT reader, ONE read per step —
//!   each step suspends at its `read().await`, so the JS microtask
//!   queue drains between steps and the VM thread never blocks. The
//!   VM itself only runs inside the host's synchronous run/resume
//!   exports (JS run-to-completion), so a chunk that lands while a
//!   leg is parked waits for the page's next pump leg — the
//!   net-parked run lane (the wasm engine's park/resume shape).
//! - THE BUDGET GATE, wasm flavor: the native gate parks the
//!   producer on the source's condvar — there is no condvar park on
//!   the JS thread (it would block the whole world). The pump checks
//!   [`HttpBodyWriter::parked_at_budget`] between reads and PARKS by
//!   re-arming itself one macrotask at a time ([`tick`]); the
//!   guest's one-pop-per-await pacing drains below the budget and
//!   the next tick resumes the pump. Real backpressure all the same:
//!   a parked pump issues no reads, the stream's internal buffer
//!   fills, the browser slows the socket. The predicate is checked
//!   immediately before each read and nothing else runs on this
//!   single JS thread between that check and the push — so the
//!   condvar gate below [`HttpBodyWriter::push_chunk`] is never
//!   reached here.
//! - CANCEL is the same wire-abort: the between-read
//!   [`HttpBodyWriter::cancelled`] probe drops the response (the
//!   reader releases, the connection closes) and every pump end —
//!   EOF, mid-read failure, or the cancel drop — retires the send
//!   slot through the pkg's [`CancelReg`].
//! - NO WINDOW (node, the smoke): the immediate status-0 transport
//!   failure — no JS call, no panic. The born-dead source answers
//!   the body/stream lanes with the disclosed degrades, so an
//!   offline run completes synchronously inside the first run
//!   envelope.
//!
//! Feature `http-wasm`, wasm32 only: the JS trio is a target-gated
//! dependency and the module is cfg'd to the arch, so native builds —
//! feature on or off — never compile it. The lane-independent rows
//! (the body drain, the stream read, the five sync readbacks) are the
//! seam's shared installer ([`crate::http::pkg_send_with_reg`]) — no
//! second grammar, the same law the reqwest lane runs.

use std::sync::Arc;

use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use rut_vm::interp::HostPkg;
use rut_vm::Completer;

use crate::http::{
    pkg_send_with_reg, CancelReg, HttpBackend, HttpBodyWriter, HttpRequest, HttpResponse,
};

// ------------------------------------------------------- the backend ---

/// The fetch backend: the window's `fetch`, driven on `spawn_local`
/// tasks. Holds the pkg's cancel registry — the send slot's wire-abort
/// bookkeeping, shared with the row arms exactly like the reqwest
/// lane's. No client state: each send is one `fetch` call and the
/// browser owns connection pooling.
pub struct FetchBackend {
    reg: CancelReg,
}

impl HttpBackend for FetchBackend {
    fn send(&self, req: &HttpRequest, out: Completer<HttpResponse>) {
        let Some(window) = web_sys::window() else {
            // the no-DOM host (node, the smoke): the immediate
            // status-0 transport failure — the kept law, no JS call,
            // no panic
            out.complete(HttpResponse::failed(
                "fetch: no window on this host (the network is unavailable)".to_string(),
            ));
            return;
        };
        let reg = self.reg.clone();
        let req = req.clone();
        wasm_bindgen_futures::spawn_local(async move {
            send_fetch(window, req, out, reg).await;
        });
    }
}

/// The wasm flavor of the reqwest lane's `pkg()`: install the seven
/// `http_host` rows over the browser's `fetch`. `http_send` resolves
/// at HEADERS; the body pumps through the stream's default reader one
/// read per step. Transport failure maps to status 0 + the message
/// (the kept law); a window-less host answers the immediate failure.
/// Requires the `http-wasm` cargo feature on wasm32.
pub fn pkg_fetch() -> HostPkg {
    let reg = CancelReg::default();
    let backend = Arc::new(FetchBackend { reg: reg.clone() });
    pkg_send_with_reg(
        move |req: &HttpRequest, out: Completer<HttpResponse>| backend.send(req, out),
        reg,
    )
}

// -------------------------------------------------- the send + pump ----

/// The send task: fetch → HEADERS → complete → pump. The failure
/// paths all land the same way — [`HttpResponse::failed`] into the
/// send completer (status 0, the born-dead body) — never a JS panic
/// and never a trap.
async fn send_fetch(
    window: web_sys::Window,
    req: HttpRequest,
    out: Completer<HttpResponse>,
    reg: CancelReg,
) {
    // the request: canonical verb, the "\n"-joined `Name: value` block,
    // bytes with empty = none (the crossing's disclosed merge)
    let init = web_sys::RequestInit::new();
    init.set_method(&req.method);
    let headers = match web_sys::Headers::new() {
        Ok(h) => h,
        Err(e) => {
            out.complete(HttpResponse::failed(format!("fetch headers: {e:?}")));
            return;
        }
    };
    for line in req.headers.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            out.complete(HttpResponse::failed(format!(
                "bad header line `{line}` (want `Name: value`)"
            )));
            return;
        };
        if headers.append(name.trim(), value.trim()).is_err() {
            out.complete(HttpResponse::failed(format!(
                "bad header `{name}` (the browser refused it)"
            )));
            return;
        }
    }
    init.set_headers(&headers);
    if !req.body.is_empty() {
        init.set_body(&js_sys::Uint8Array::from(req.body.as_slice()).into());
    }
    let sent = window.fetch_with_str_and_init(&req.url, &init);
    let answered = match JsFuture::from(sent).await {
        Ok(v) => v,
        Err(e) => {
            out.complete(HttpResponse::failed(format!("fetch failed: {e:?}")));
            return;
        }
    };
    let resp: web_sys::Response = match answered.dyn_into() {
        Ok(r) => r,
        Err(_) => {
            out.complete(HttpResponse::failed(
                "fetch answered something that is not a Response".to_string(),
            ));
            return;
        }
    };
    // HEADERS — the send resolves here; the body stays unread
    let (answer, writer) = HttpResponse::streaming(resp.status());
    // the send future's cancel slot, registered BEFORE the completion
    // (a loser-cancel in the resolve window must find it — the reqwest
    // lane's order)
    reg.insert(out.id(), writer.source());
    out.complete(answer);
    let body = match resp.body() {
        Some(stream) => match web_sys::ReadableStreamDefaultReader::new(&stream) {
            Ok(reader) => reader,
            Err(e) => {
                reg.remove(out.id());
                writer.fail(format!("stream reader: {e:?}"));
                return;
            }
        },
        // a null-body response (204, HEAD): headers only, clean EOF
        None => {
            reg.remove(out.id());
            writer.end();
            return;
        }
    };
    pump(body, writer, out.id(), reg).await;
}

/// The body pump: the default reader, ONE read per step — each step
/// suspends at its `read().await` (the JS microtask queue drains
/// between steps), pushes through the SAME writer the send minted,
/// and re-checks the two probes between steps: the wire-abort
/// (cancel drops the response — the connection closes) and the
/// budget gate (a park re-arms on a later macrotask — [`tick`]).
/// Every end retires the send slot through the registry.
async fn pump(
    reader: web_sys::ReadableStreamDefaultReader,
    writer: HttpBodyWriter,
    send_id: usize,
    reg: CancelReg,
) {
    loop {
        // the between-chunk cancel probe: a marked source ends the
        // pump here — the reader releases, the connection closes
        if writer.cancelled() {
            reg.remove(send_id);
            return;
        }
        // the budget gate, wasm flavor: park without reading — one
        // macrotask per tick, resumed by the guest's next pop (the
        // no-condvar law; see the module header)
        if writer.parked_at_budget() {
            let _ = tick().await; // the value is nothing — the wait is the point
            continue;
        }
        let step = match JsFuture::from(reader.read()).await {
            Ok(v) => v,
            Err(e) => {
                reg.remove(send_id);
                writer.fail(format!("stream read failed: {e:?}"));
                return;
            }
        };
        // the read result: `{ value: Uint8Array | undefined, done:
        // bool }` — read through the property keys (the result shape
        // is the JS contract; no binding dependency beyond Reflect)
        let done = js_sys::Reflect::get(&step, &js_str("done"))
            .ok()
            .and_then(|d| d.as_bool())
            .unwrap_or(false);
        if done {
            reg.remove(send_id);
            writer.end();
            return;
        }
        let chunk = js_sys::Reflect::get(&step, &js_str("value")).ok();
        match chunk {
            Some(v) if v.is_instance_of::<js_sys::Uint8Array>() => {
                writer.push_chunk(js_sys::Uint8Array::from(v).to_vec());
            }
            // a done-less result with no payload is an EOF spelling
            _ => {
                reg.remove(send_id);
                writer.end();
                return;
            }
        }
    }
}

/// One macrotask, as a future — the parked pump's clock: a promise
/// resolved through `window.setTimeout(.., 0)`. A self-requeuing
/// MICROTASK would starve every macrotask (the runner's pump legs
/// would never fire), so the park rides the timer queue.
fn tick() -> JsFuture {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let Some(window) = web_sys::window() else {
            return; // unreachable from the pump: the send had a window
        };
        let cb = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref::<js_sys::Function>(),
            0,
        );
    });
    JsFuture::from(promise)
}

/// the one-string helper — `Reflect::get`'s key spelling
fn js_str(s: &str) -> JsValue {
    JsValue::from_str(s)
}
