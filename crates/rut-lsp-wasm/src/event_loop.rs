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
//!   self-RPC deadlock class is unwritable. Only the bridges post (and
//!   in this phase only the bridges exist).
//! - **No walking flag, no borrow choreography** — mutations are
//!   message-serialized on the loop's FIFO; concurrent unresolved calls
//!   resolve in send order.
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

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::str::FromStr;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot::Sender;
use futures::StreamExt;
use ls_types::Uri;

use crate::wire;
use crate::{json_envelope, locations_envelope, SessionState};

/// the MACHINERY tier — one arm per kind of turn. The bridges'
/// business asks nest under [`Msg::Cmd`]; the capability lifecycle
/// ([`Msg::SetEnv`]) stands beside it. [`handle`] matches here and
/// never deeper.
pub enum Msg {
    /// every business ask — the only thing the bridges post
    Cmd(Cmd),
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
    AddDef {
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
    /// the `WalkOut` envelope.
    ResolveDeps {
        roots: Vec<String>,
        tx: Tx,
    },
    AddDefMod {
        uri: String,
        name: String,
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

/// the loop's whole self — the business state (opaque to the shell)
/// beside the machinery's own facts: the stored read-only host env
/// (`None` until the host hands one over, read only by the dep walk).
/// `run` owns it exclusively; [`handle`] and [`dispatch`] borrow it
/// whole.
struct Loop {
    /// the editing session's accumulated facts — every business arm's
    /// reads and writes land here
    state: SessionState,
    /// the injected host env — `set_env`'s store (replace-on-reinject)
    env: Option<Rc<dyn crate::env::Io>>,
}

/// the loop's body — own the [`Loop`] exclusively, answer one message
/// at a time, never die on a bad handler.
pub(crate) async fn run(state: SessionState, mut inbox: UnboundedReceiver<Msg>) {
    let mut l = Loop { state, env: None };
    while let Some(msg) = inbox.next().await {
        // panic isolation: one bad handler drops ONE reply channel (the
        // bridge answers its error envelope); the loop never dies. The
        // handler may leave the state half-mutated — accepted; the next
        // message sees whatever state stands.
        let _ = catch_unwind(AssertUnwindSafe(|| handle(&mut l, msg)));
    }
}

/// the shell's turn — MACHINERY only, business-blind: a business ask
/// delegates to [`dispatch`]; the capability lane stores the env and
/// answers. No business fact is inspected here.
fn handle(l: &mut Loop, msg: Msg) {
    match msg {
        Msg::Cmd(c) => dispatch(l, c),
        Msg::SetEnv { env, tx } => {
            // the capability STORE — replace-on-reinject, exactly the
            // ledger setters' shape. The walk (ResolveDeps below) is
            // the one reader; the Rc hands it the same cached
            // functions the host injected.
            l.env = Some(env);
            let _ = tx.send(Reply::Done);
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
            // on the keystroke)
            l.state.docs.insert(uri.clone(), src);
            let answer = l.state.complete_stored(&uri, line, ch);
            let _ = tx.send(Reply::Text(answer));
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
        Cmd::AddDef { uri, src, tx } => {
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
            // the read-only walk: one fast handler turn over the
            // stored env — no walk task, no awaits. An env-less session
            // answers the loud error envelope, never a silent empty
            // walk (the demo path never calls this entry).
            let answer = match l.env.as_ref() {
                None => json_envelope(wire::Error::new(
                    "no host environment — the dep walk reads through the injected env \
                     (load(bytes, env) / set_env)",
                )),
                Some(io) => json_envelope(wire::WalkOut::from(&crate::walk::resolve(
                    &mut l.state,
                    io.as_ref(),
                    &roots,
                ))),
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Cmd::AddDefMod {
            uri,
            name,
            mod_path,
            src,
            tx,
        } => {
            // the host's scan-index lane carries no manifest — no consts
            let decls = l.state.add_def_mod_at(&uri, &name, &mod_path, &src, &[]);
            let _ = tx.send(Reply::Text(json_envelope(wire::ModDecls { decls })));
        }
    }
}

/// the executor seam: the wasm face parks the loop on the JS microtask
/// queue (`spawn_local`); native ABI tests drive it on a `LocalPool`
/// they own (`RutLsp::spawn_native`). Answers the inbox handle the
/// bridges post on.
pub(crate) fn spawn(state: SessionState) -> UnboundedSender<Msg> {
    let (tx, rx) = unbounded::<Msg>();
    #[cfg(target_family = "wasm")]
    wasm_bindgen_futures::spawn_local(run(state, rx));
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
