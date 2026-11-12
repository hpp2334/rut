//! The actor seam: ONE long-lived event loop owns the session state
//! exclusively, and every ABI entry is an async RPC bridge — post a
//! [`Msg`] on the inbox, await the oneshot reply.
//!
//! The message enum is TWO TIER: [`Msg`] names the MACHINERY (which
//! kind of turn this is — a business ask, or the capability
//! lifecycle), and [`Cmd`] nests every business ask under
//! [`Msg::Cmd`]. The tier split keeps the shell business-blind:
//! [`run`] and [`handle`] route on the machinery tier only; every
//! business fact lives in a [`Cmd`] arm of [`dispatch`].
//!
//! The laws this buys (the plan's structural wins):
//!
//! - **Handlers are sync fns over the loop** — today's entry bodies,
//!   moved verbatim into [`dispatch`]'s match (plus the stored
//!   read-only host env, `crate::env`). They cannot await, so the
//!   borrow-across-await class of bug is unwritable.
//! - **A handler gets NO sender to the loop** — the reply oneshot
//!   rides IN the command, so a handler can answer but never post; the
//!   self-RPC deadlock class is unwritable. The posters are the
//!   bridges and the loop's own WAKE SOURCE (the clock capability's
//!   stored trigger — machinery, never a handler).
//! - **No walking flag, no borrow choreography** — mutations are
//!   message-serialized on the loop's FIFO; concurrent unresolved calls
//!   resolve in send order.
//! - **The actor owns the WHEN** — the loop carries a SELF-ADDRESS (a
//!   clone of its inbox sender) and the env carries one machinery
//!   capability, the clock (`Io::set_wake`/`Io::arm_wake` — the host's
//!   plain `setTimeout` behind an optional `wakeAfter`): [`Loop::defer`]
//!   queues a command under one armed window, the host calls the
//!   stored wake, and the identity-free [`Msg::Wake`] drains the
//!   queue. No clock ⇒ every deferral runs eagerly in-turn (D3).
//!   Business policy — the walk's coalescing — composes the NEUTRAL
//!   doors (`defer` + [`Loop::cancel_defer`] over pure [`DeferTask`]
//!   handles) inside the `Cmd::ReloadDeps` arm; the shell never inspects a
//!   `Cmd`.
//! - **Panics isolate to one oneshot** — `catch_unwind` keeps the loop
//!   alive; a panicked handler drops its reply channel and the bridge
//!   answers the error envelope, never a dead loop. (On the wasm32
//!   target the panic strategy is abort — the target default — so the
//!   guard's isolation is fully observable on the native pool;
//!   the structure is what ships either way.)
//!
//! Naming note: the plan spells this file `loop.rs`; `loop` is a
//! keyword, so the module carries the same content under a spellable
//! name.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::str::FromStr;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot::Sender;
use futures::StreamExt;
use ls_types::Uri;

use crate::wire;
use crate::{json_envelope, locations_envelope, SessionState};

/// the deferral window — the walk lane's debounce interval (the old JS
/// engine's 200ms, now the ACTOR's own clock read: ONE armed window
/// batches everything queued beneath it).
pub(crate) const DEBOUNCE_MS: u32 = 200;

/// the MACHINERY tier — one arm per kind of turn. The bridges'
/// business asks nest under [`Msg::Cmd`]; the capability lifecycle
/// ([`Msg::SetEnv`]) and the clock's tick ([`Msg::Wake`]) stand beside
/// it. [`handle`] matches here and never deeper.
pub enum Msg {
    /// every business ask — the only thing the bridges post
    Cmd(Cmd),
    /// the identity-free tick: an armed wake window expired — the
    /// state says what (the defer queue drains; an empty queue is a
    /// spurious no-op). Posted by the clock capability's stored wake
    /// onto the loop's own inbox; no bridge ever sends it.
    Wake,
    /// the injected READ-ONLY host environment (the dep walk's
    /// capability — `crate::env`). Carried as the [`env::Io`] trait
    /// object: the wasm bridge wraps its `JsEnv` in the `Rc` at
    /// `set_env`, native tests hand over their own `Io` — one
    /// capability lane, two constructors.
    SetEnv { env: Rc<dyn crate::env::Io>, tx: Tx },
}

/// the BUSINESS tier — one arm per ABI ask (the old flat `Msg`
/// variants, nested; the machinery's `SetEnv` is the one extraction).
/// The `tx` rides IN the command — the handler replies on it, and the
/// field set is owned data (the bridge copies the host's `&str`/`&[u8]`
/// before posting, so the future never borrows).
pub enum Cmd {
    Legend {
        tx: Tx,
    },
    Analyze {
        uri: String,
        src: String,
        tx: Tx,
    },
    Forget {
        uri: String,
        tx: Tx,
    },
    Hover {
        uri: String,
        line: u32,
        ch: u32,
        tx: Tx,
    },
    Complete {
        uri: String,
        src: String,
        line: u32,
        ch: u32,
        tx: Tx,
    },
    Definition {
        uri: String,
        line: u32,
        ch: u32,
        tx: Tx,
    },
    TypeDefinition {
        uri: String,
        line: u32,
        ch: u32,
        tx: Tx,
    },
    Inlay {
        uri: String,
        start_line: u32,
        start_ch: u32,
        end_line: u32,
        end_ch: u32,
        tx: Tx,
    },
    References {
        uri: String,
        line: u32,
        ch: u32,
        include_decl: u32,
        tx: Tx,
    },
    SignatureHelp {
        uri: String,
        line: u32,
        ch: u32,
        tx: Tx,
    },
    IndexFile {
        uri: String,
        src: String,
        tx: Tx,
    },
    ParseManifest {
        text: String,
        tx: Tx,
    },
    /// the read-only dep walk (`crate::walk`): the roots walk NOW, in
    /// one fast handler turn — no walk task, no awaits. The reply is
    /// the `WalkOut` envelope. The EAGER manual door (tests, cold
    /// boot, env-less hosts); the deferred door is [`Cmd::ReloadDeps`].
    ResolveDeps {
        roots: Vec<String>,
        tx: Tx,
    },
    /// RELOAD the deps — the level-triggered door: the walk DEFERS
    /// under the debounce window and coalesces (a later reload
    /// supersedes an earlier one, whose oneshot answers `Done` —
    /// merged, not lost). With no clock on the env the walk runs
    /// eagerly in-turn (D3): the same answer, no window.
    ReloadDeps {
        roots: Vec<String>,
        tx: Tx,
    },
    /// the deferred walk — minted ONLY by the deferral doors (the
    /// `ReloadDeps` arm's `defer`); no bridge posts it. The tx it
    /// carries is the reload's own oneshot: the answer rides the RPC's
    /// reply channel, no push lane (D2).
    WalkDeps {
        roots: Vec<String>,
        tx: Tx,
    },
    IndexModAt {
        uri: String,
        pkg: String,
        mod_path: String,
        src: String,
        tx: Tx,
    },
}

/// a handler's reply channel — one oneshot per command.
pub type Tx = Sender<Reply>;

/// what a handler answers. `Text` is the JSON envelope every query and
/// parse entry speaks; `Done` is the void entries' completion (the
/// bridge awaits it for ORDERING, not payload).
pub enum Reply {
    Text(String),
    Done,
}

/// a queued deferral's NAME — pure data (`Copy`), minted by
/// [`Loop::defer`], handed to [`Loop::cancel_defer`]. Orthogonal to
/// the queue itself: two total primitives, the
/// `setTimeout`/`clearTimeout` shape (D1 — latest-win is caller
/// policy, never shell state).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) struct DeferTask(u64);

impl DeferTask {
    /// the eager path's sentinel — `defer` returns it when no clock
    /// exists (the command dispatched in-turn, nothing is queued). It
    /// names no task, so `cancel_defer` on it always answers `None`.
    pub(crate) fn spent() -> DeferTask {
        DeferTask(u64::MAX)
    }
}

/// the neutral defer lane: a FIFO under ONE armed window — nothing
/// dropped, nothing reordered, no `Cmd` shape ever inspected (the
/// shell is business-blind). Coalescing is NOT here: the CALLER
/// composes `defer` + `cancel_defer` (D1).
#[derive(Default)]
struct Sched {
    /// queued deferrals in arrival order
    queue: VecDeque<(DeferTask, Cmd)>,
    /// the next task id — monotone, so a stale handle never names a
    /// fresh task
    next: u64,
    /// is a wake window armed? One window batches the whole queue; the
    /// tick clears it
    armed: bool,
}

/// the loop's whole self — the business state (opaque to the shell)
/// beside the machinery's own facts: the stored read-only host env
/// (`None` until the host hands one over, read only by the dep walk),
/// the neutral defer queue, and the SELF-ADDRESS (a clone of the
/// inbox sender — the wake source's captured target). `run` owns it
/// exclusively; [`handle`] and [`dispatch`] borrow it whole.
struct Loop {
    /// the editing session's accumulated facts — every business arm's
    /// reads and writes land here
    state: SessionState,
    /// the injected host env — `set_env`'s store (replace-on-reinject)
    env: Option<Rc<dyn crate::env::Io>>,
    /// the neutral defer lane
    sched: Sched,
    /// the loop's self-address: a clone of the inbox sender the
    /// `SetEnv` arm captures into the wake source, so the clock's tick
    /// re-enters THIS loop (case 9: each instance's wake posts into
    /// its own inbox)
    self_tx: UnboundedSender<Msg>,
}

/// the loop's body — own the [`Loop`] exclusively, answer one message
/// at a time, never die on a bad handler.
pub(crate) async fn run(
    state: SessionState,
    self_tx: UnboundedSender<Msg>,
    mut inbox: UnboundedReceiver<Msg>,
) {
    let mut l = Loop {
        state,
        env: None,
        sched: Sched::default(),
        self_tx,
    };
    while let Some(msg) = inbox.next().await {
        // panic isolation: one bad handler drops ONE reply channel (the
        // bridge answers its error envelope); the loop never dies. The
        // handler may leave the state half-mutated — accepted; the next
        // message sees whatever state stands.
        let _ = catch_unwind(AssertUnwindSafe(|| handle(&mut l, msg)));
    }
}

/// the shell's turn — MACHINERY only, business-blind: a business ask
/// delegates to [`dispatch`]; the tick drains the defer queue; the
/// capability lane installs the env (and its wake source) and answers.
/// No business fact is inspected here.
fn handle(l: &mut Loop, msg: Msg) {
    match msg {
        Msg::Cmd(c) => dispatch(l, c),
        Msg::Wake => l.on_wake(),
        Msg::SetEnv { env, tx } => {
            // the capability STORE — replace-on-reinject, exactly the
            // ledger setters' shape. The WAKE SOURCE installs first:
            // the clock's trigger is a closure over this loop's
            // self-address (it posts the identity-free `Wake` on the
            // inbox, so only the actor's own loop sees the tick), then
            // the store — every reader (the walk in `dispatch`, the
            // clock in `arm_window`) reads the same capability.
            let self_tx = l.self_tx.clone();
            let wake: crate::env::WakeFn = Rc::new(move || {
                let _ = self_tx.unbounded_send(Msg::Wake);
            });
            env.set_wake(wake);
            l.env = Some(env);
            let _ = tx.send(Reply::Done);
        }
    }
}

impl Loop {
    /// the neutral door — enqueue under the window; the handle names
    /// the task for cancel. Nothing dropped, order kept. No clock ⇒
    /// the command dispatches NOW (D3) and the spent sentinel comes
    /// back.
    fn defer(&mut self, cmd: Cmd) -> DeferTask {
        if !self.arm_window() {
            dispatch(self, cmd); // the no-clock degrade — in-turn
            return DeferTask::spent();
        }
        let t = DeferTask(self.sched.next);
        self.sched.next += 1;
        self.sched.queue.push_back((t, cmd));
        t
    }

    /// cancel — orthogonal to enqueue, total over pure data:
    /// `Some(cmd)` when still queued (the CALLER answers it); `None`
    /// when already drained / never queued / already cancelled.
    /// Idempotent, never panics, never double-answers.
    fn cancel_defer(&mut self, t: DeferTask) -> Option<Cmd> {
        let i = self.sched.queue.iter().position(|(k, _)| *k == t)?;
        Some(self.sched.queue.remove(i)?.1)
    }

    /// the tick: the window expired — drain ALL queued work, in order
    /// (a batch, not a pop). An empty queue is a spurious tick: a
    /// no-op, never a crash.
    fn on_wake(&mut self) {
        self.sched.armed = false;
        for (_, cmd) in std::mem::take(&mut self.sched.queue) {
            dispatch(self, cmd);
        }
    }

    /// true ⇒ the command queues under an armed window; false ⇒ run
    /// now. The FIRST deferral in a window arms the clock; every later
    /// one rides the same window (one `arm_wake` per batch).
    fn arm_window(&mut self) -> bool {
        if self.sched.armed {
            return true;
        }
        match self.env.as_ref().map(|io| io.arm_wake(DEBOUNCE_MS)) {
            Some(true) => {
                self.sched.armed = true;
                true
            }
            // no env / no clock / the arm refused ⇒ eager (D3)
            _ => false,
        }
    }
}

/// the business tier's turn — the entries' bodies, verbatim: sync arms
/// over the loop's state (and the stored env, which only the dep walk
/// reads). No sender, no await.
fn dispatch(l: &mut Loop, cmd: Cmd) {
    match cmd {
        Cmd::Legend { tx } => {
            let _ = tx.send(Reply::Text(json_envelope(serde_json::json!(
                crate::semantic::TokenType::legend()
            ))));
        }
        Cmd::Analyze { uri, src, tx } => {
            // open / FULL-sync change: the doc stores FIRST (the analyze
            // lane's store — the queries below read the same text)
            l.state.docs.insert(uri.clone(), src.clone());
            // analyze_at stamps related-info URIs + hover provenance at
            // construction; a degenerate URI still analyzes (unstamped
            // twin below)
            let a = match Uri::from_str(&uri) {
                Ok(u) => crate::analysis::analyze_at(&u, &src),
                Err(_) => crate::analysis::analyze_unstamped(&uri, &src),
            };
            // the envelope's rows move to the wire field by field (a
            // plain record — no consuming ceremony)
            let diags = {
                let dm = l.state.doc_mods_of(&uri);
                let groups = if dm.pkg.is_some() {
                    crate::check::groups_from_defs(&l.state.defs)
                } else {
                    Vec::new()
                };
                crate::check::merged_doc_diags(&uri, &src, dm.pkg, dm.mod_path, groups, a.diags)
            };
            let answer = json_envelope(wire::Analysis {
                diags,
                // SemanticTokens serializes `data` to the flat u32 wire format
                // (the same bytes a stdio face would put on the wire)
                tokens: ls_types::SemanticTokens {
                    result_id: None,
                    data: a.tokens,
                },
                symbols: a.symbols,
            });
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::Forget { uri, tx } => {
            l.state.docs.remove(&uri);
            let _ = tx.send(Reply::Done);
        }
        Cmd::Hover { uri, line, ch, tx } => {
            let answer = match l.state.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let doc = l.state.doc_mods_of(&uri);
                    let out = crate::analysis::hover_at(&uri, &src, &l.state.defs, &doc, line, ch);
                    json_envelope(serde_json::json!(out))
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::Complete {
            uri,
            src,
            line,
            ch,
            tx,
        } => {
            // the doc-sync entry: the CURRENT text stores FIRST (see the
            // bridge's doc — the analyze lane debounces, completion fires
            // on the keystroke). The typed body; the envelope writes
            // HERE, at the reply boundary (hover's arm's own shape)
            l.state.docs.insert(uri.clone(), src);
            let items = l.state.complete_stored(&uri, line, ch);
            let _ = tx.send(Reply::Text(json_envelope(serde_json::json!(items))));
        }
        Cmd::Definition { uri, line, ch, tx } => {
            let answer = match l.state.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let doc = l.state.doc_mods_of(&uri);
                    let locs =
                        crate::analysis::definition_at(&uri, &src, &l.state.defs, &doc, line, ch);
                    locations_envelope(&locs)
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::TypeDefinition { uri, line, ch, tx } => {
            let answer = match l.state.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let doc = l.state.doc_mods_of(&uri);
                    let locs = crate::analysis::type_definition_at(
                        &uri,
                        &src,
                        &l.state.defs,
                        &doc,
                        line,
                        ch,
                    );
                    locations_envelope(&locs)
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::Inlay {
            uri,
            start_line,
            start_ch,
            end_line,
            end_ch,
            tx,
        } => {
            let answer = match l.state.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let hints = crate::analysis::inlay_hints_at(
                        &uri,
                        &src,
                        &l.state.defs,
                        ls_types::Range {
                            start: ls_types::Position {
                                line: start_line,
                                character: start_ch,
                            },
                            end: ls_types::Position {
                                line: end_line,
                                character: end_ch,
                            },
                        },
                    );
                    json_envelope(serde_json::json!(hints))
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::References {
            uri,
            line,
            ch,
            include_decl,
            tx,
        } => {
            let answer = match l.state.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let doc = l.state.doc_mods_of(&uri);
                    let locs = crate::analysis::references_at(
                        &uri,
                        &src,
                        &l.state.defs,
                        &doc,
                        line,
                        ch,
                        include_decl != 0,
                    );
                    locations_envelope(&locs)
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::SignatureHelp { uri, line, ch, tx } => {
            let answer = match l.state.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let out =
                        crate::analysis::signature_help_at(&uri, &src, &l.state.defs, line, ch);
                    json_envelope(serde_json::json!(out))
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::IndexFile { uri, src, tx } => {
            let idx = crate::analysis::index_at(&uri, &src);
            match l.state.defs.iter().position(|d| d.origin == uri) {
                Some(slot) => l.state.defs[slot] = idx,
                None => l.state.defs.push(idx),
            }
            let _ = tx.send(Reply::Done);
        }
        Cmd::ParseManifest { text, tx } => {
            let answer = match crate::deps::dep_table_pinned(&text, &l.state.lock_pins) {
                Ok(t) => json_envelope(wire::DepTable::from(&t)),
                Err(e) => json_envelope(wire::Error::new(e)),
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::ResolveDeps { roots, tx } => {
            // the eager manual door — tests, cold boot, env-less hosts.
            // The walk body is the SAME one the deferred door runs (D9);
            // the envelope writes here, at the reply boundary
            let _ = tx.send(Reply::Text(match walk_now(l, &roots) {
                Ok(w) => json_envelope(w),
                Err(e) => json_envelope(e),
            }));
        }
        Cmd::ReloadDeps { roots, tx } => {
            // the ONLY place coalescing is chosen (D1): the lane
            // composes the neutral doors over its remembered handle —
            // level-triggered, latest wins. `defer` may have run the
            // walk eagerly (no clock): the spent sentinel it returned
            // names nothing, and the answer already rode this arm's tx.
            let t = l.defer(Cmd::WalkDeps { roots, tx });
            if let Some(prev) = l.state.walk_task.replace(t) {
                // cancel ⇒ Some: the queued walk never fires — answer
                // its reload `Done` (superseded, merged into the newer;
                // the LAST reload's promise carries the world).
                // cancel ⇒ None: the previous walk already fired and
                // answered its own reload — nothing to do, a
                // double-answer is unwritable.
                if let Some(Cmd::WalkDeps { tx: old, .. }) = l.cancel_defer(prev) {
                    let _ = old.send(Reply::Done);
                }
            }
        }
        Cmd::WalkDeps { roots, tx } => {
            // minted ONLY by the deferral doors: the tick drains it
            // (or the no-clock eager path dispatched it in-turn);
            // the envelope writes here, at the reply boundary
            let _ = tx.send(Reply::Text(match walk_now(l, &roots) {
                Ok(w) => json_envelope(w),
                Err(e) => json_envelope(e),
            }));
        }
        Cmd::IndexModAt {
            uri,
            pkg,
            mod_path,
            src,
            tx,
        } => {
            let decls = l.state.index_mod_at(&uri, &pkg, &mod_path, &src);
            let _ = tx.send(Reply::Text(json_envelope(wire::ModDecls { decls })));
        }
    }
}

/// the walk BODY both doors share (D9 — one implementation, two
/// doors): the eager `Cmd::ResolveDeps` and the deferred
/// `Cmd::WalkDeps` run exactly this — one fast handler turn over the
/// stored env, no walk task, no awaits. TYPED: the calling arms own
/// the envelope; an env-less session answers the loud error value,
/// never a silent empty walk.
fn walk_now(l: &mut Loop, roots: &[String]) -> Result<wire::WalkOut, wire::Error> {
    match l.env.as_ref() {
        None => Err(wire::Error::new(
            "no host environment — the dep walk reads through the injected env \
             (load(bytes, env) / set_env)",
        )),
        Some(io) => Ok(wire::WalkOut::from(&crate::walk::resolve(
            &mut l.state,
            io.as_ref(),
            roots,
        ))),
    }
}

/// the executor seam: the wasm face parks the loop on the JS microtask
/// queue (`spawn_local`); native ABI tests drive it on a `LocalPool`
/// they own (`RutLsp::spawn_native`). Answers the inbox handle the
/// bridges post on.
pub(crate) fn spawn(state: SessionState) -> UnboundedSender<Msg> {
    let (tx, rx) = unbounded::<Msg>();
    #[cfg(target_family = "wasm")]
    wasm_bindgen_futures::spawn_local(run(state, tx.clone(), rx));
    // A native build has no executor for a plain `RutLsp::new()` — the
    // ABI face belongs to the wasm; native tests go through
    // `spawn_native`, which parks the loop on a pool IT hands back.
    // Here the inbox drops and every bridge answers the loop-gone
    // envelope — the honest dead-loop shape.
    #[cfg(not(target_family = "wasm"))]
    drop((state, rx));
    tx
}

/// the channel pair behind [`spawn`] — `spawn_native` parks the loop
/// itself and needs the receiver (native targets only).
#[cfg(not(target_family = "wasm"))]
pub(crate) fn pair() -> (UnboundedSender<Msg>, UnboundedReceiver<Msg>) {
    unbounded::<Msg>()
}

/// the answer a bridge gives when the loop is gone (the page is tearing
/// down, the pool dropped): the error envelope the JS face already
/// narrows on — never a hang, never a partial answer.
pub(crate) fn loop_gone() -> String {
    json_envelope(wire::Error::new("the rut-lsp event loop is gone"))
}

// ---- the machinery's own gate — the NEUTRAL-QUEUE and CANCEL laws
// with dummy cmds, driven directly on the loop (native, deterministic;
// D8: the fake clock STORES the wake source and RECORDS arms, the
// test fires the stored Rc by hand — zero timers) ----
#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::{Io, WakeFn};
    use futures::FutureExt;
    use std::cell::RefCell;

    /// the machinery fake: reads always miss (no dummy cmd walks), the
    /// clock records arms and stores the wake source. `clock: false`
    /// spells the capability-less env (D3's degrade world).
    #[derive(Default)]
    struct ClockIo {
        clock: bool,
        arms: RefCell<Vec<u32>>,
        wake: RefCell<Option<WakeFn>>,
    }

    impl ClockIo {
        /// with the clock capability
        fn ticking() -> ClockIo {
            ClockIo {
                clock: true,
                ..Default::default()
            }
        }
    }

    impl Io for ClockIo {
        fn read(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn read_dir(&self, _path: &str) -> Option<Vec<String>> {
            None
        }
        fn is_dir(&self, _path: &str) -> bool {
            false
        }
        fn notify(&self, _message: &str, _is_error: bool) {}
        fn set_wake(&self, wake: WakeFn) {
            *self.wake.borrow_mut() = Some(wake);
        }
        fn arm_wake(&self, ms: u32) -> bool {
            if !self.clock {
                return false; // the host object carried no wakeAfter
            }
            self.arms.borrow_mut().push(ms);
            true
        }
    }

    /// a bare loop (no env) — the caller owns the inbox pair
    fn bare_loop() -> (Loop, UnboundedReceiver<Msg>) {
        let (tx, rx) = unbounded::<Msg>();
        (
            Loop {
                state: SessionState::boot(),
                env: None,
                sched: Sched::default(),
                self_tx: tx,
            },
            rx,
        )
    }

    /// a loop with the ticking clock installed through the REAL SetEnv
    /// lane — the wake-source install is the plumbing under test too
    fn clocked_loop() -> (Loop, Rc<ClockIo>, UnboundedReceiver<Msg>) {
        let (mut l, rx) = bare_loop();
        let io = Rc::new(ClockIo::ticking());
        let (stx, srx) = futures::channel::oneshot::channel();
        handle(
            &mut l,
            Msg::SetEnv {
                env: io.clone(),
                tx: stx,
            },
        );
        assert!(
            matches!(srx.now_or_never(), Some(Ok(Reply::Done))),
            "SetEnv answers Done"
        );
        (l, io, rx)
    }

    #[test]
    fn set_env_installs_the_wake_source_on_the_self_address() {
        // the full plumbing: SetEnv hands the env a closure that posts
        // the identity-free Wake on the loop's OWN inbox — not a host
        // callback lane, exactly the self-address
        let (_l, io, mut rx) = clocked_loop();
        let wake = io.wake.borrow().clone().expect("the wake source installed");
        wake();
        assert!(
            matches!((&mut rx.next()).now_or_never(), Some(Some(Msg::Wake))),
            "firing the stored wake posts Msg::Wake on the inbox"
        );
    }

    #[test]
    fn the_neutral_queue_never_drops_or_reorders() {
        let (mut l, io, mut rx) = clocked_loop();
        // two DISTINCT dummy cmds: Analyze stores the doc, Forget
        // removes it — the FIFO law is observable in the end state (a
        // reversed drain would leave the doc in)
        let uri = "file:///sched/fifo.rut".to_string();
        let (a_tx, mut a_rx) = futures::channel::oneshot::channel();
        let (b_tx, mut b_rx) = futures::channel::oneshot::channel();
        l.defer(Cmd::Analyze {
            uri: uri.clone(),
            src: "fn main() -> nil {\n}\n".to_string(),
            tx: a_tx,
        });
        l.defer(Cmd::Forget { uri, tx: b_tx });
        // queued, not dispatched: no reply, no doc — and ONE armed
        // window covers the whole batch
        assert!((&mut a_rx).now_or_never().is_none() && (&mut b_rx).now_or_never().is_none());
        assert!(!l.state.docs.contains_key("file:///sched/fifo.rut"));
        assert_eq!(l.sched.queue.len(), 2);
        assert_eq!(*io.arms.borrow(), vec![DEBOUNCE_MS], "one window arms once");
        // the stored wake fires the tick: BOTH dispatch, in order
        let wake = io.wake.borrow().clone().expect("the wake source");
        wake();
        assert!(matches!(
            (&mut rx.next()).now_or_never(),
            Some(Some(Msg::Wake))
        ));
        l.on_wake();
        assert!(matches!(
            (&mut a_rx).now_or_never(),
            Some(Ok(Reply::Text(_)))
        ));
        assert!(matches!((&mut b_rx).now_or_never(), Some(Ok(Reply::Done))));
        // Analyze ran BEFORE Forget (reversed, the doc would remain)
        assert!(!l.state.docs.contains_key("file:///sched/fifo.rut"));
        assert!(l.sched.queue.is_empty() && !l.sched.armed);
    }

    #[test]
    fn cancel_is_total_and_idempotent() {
        let (mut l, _io, _rx) = clocked_loop();
        // still queued — cancel hands the cmd BACK (the CALLER answers
        // it; the test holds it, so nothing answers yet)
        let (tx, mut rx) = futures::channel::oneshot::channel();
        let t = l.defer(Cmd::Legend { tx });
        let cancelled = l.cancel_defer(t);
        assert!(matches!(cancelled, Some(Cmd::Legend { .. })));
        // idempotent — a second cancel answers None, never a double answer
        assert!(l.cancel_defer(t).is_none());
        // the cancelled cmd NEVER dispatched, not even at the tick
        l.on_wake();
        assert!(
            (&mut rx).now_or_never().is_none(),
            "a cancelled cmd never runs while its caller holds it"
        );
        // the caller dropping it unanswered is the canceled-oneshot
        // shape (the bridge's error envelope) — never the loop's voice
        drop(cancelled);
        assert!(matches!((&mut rx).now_or_never(), Some(Err(_))));
        // after the tick drained: the same handle answers None (its cmd
        // already fired and answered its own caller)
        let (tx2, mut rx2) = futures::channel::oneshot::channel();
        let t2 = l.defer(Cmd::Legend { tx: tx2 });
        l.on_wake();
        assert!(matches!(
            (&mut rx2).now_or_never(),
            Some(Ok(Reply::Text(_)))
        ));
        assert!(l.cancel_defer(t2).is_none());
        // the eager-path sentinel names no task; neither does any
        // never-queued id
        assert!(l.cancel_defer(DeferTask::spent()).is_none());
        assert!(l.cancel_defer(DeferTask(9999)).is_none());
    }

    #[test]
    fn a_spurious_wake_is_a_noop() {
        let (mut l, _io, _rx) = clocked_loop();
        l.on_wake(); // an empty queue: no crash, nothing dispatched
        assert!(l.sched.queue.is_empty() && !l.sched.armed);
    }

    #[test]
    fn no_env_and_no_clock_dispatch_eagerly() {
        // no env at all: defer runs the cmd NOW and hands back the
        // spent sentinel (D3)
        let (mut l, _rx) = bare_loop();
        let (a_tx, mut a_rx) = futures::channel::oneshot::channel();
        let t = l.defer(Cmd::Legend { tx: a_tx });
        assert_eq!(t, DeferTask::spent());
        assert!(matches!(
            (&mut a_rx).now_or_never(),
            Some(Ok(Reply::Text(_)))
        ));
        assert!(l.sched.queue.is_empty());

        // an env WITHOUT the clock (arm_wake ⇒ false): same degrade —
        // the biz arms are identical either way, the queue never grows
        let (mut l2, _rx2) = bare_loop();
        let io = Rc::new(ClockIo::default()); // clock: false
        let (stx, _srx) = futures::channel::oneshot::channel();
        handle(&mut l2, Msg::SetEnv { env: io, tx: stx });
        let (b_tx, mut b_rx) = futures::channel::oneshot::channel();
        let t2 = l2.defer(Cmd::Legend { tx: b_tx });
        assert_eq!(t2, DeferTask::spent());
        assert!(matches!(
            (&mut b_rx).now_or_never(),
            Some(Ok(Reply::Text(_)))
        ));
        assert!(l2.sched.queue.is_empty());
    }
}
