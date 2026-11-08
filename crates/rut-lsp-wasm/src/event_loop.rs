//! The actor seam: ONE long-lived event loop owns the language core
//! exclusively, and every ABI entry is an async RPC bridge — post a
//! [`Msg`] on the inbox, await the oneshot reply.
//!
//! The laws this buys (the plan's structural wins):
//!
//! - **Handlers are sync fns over `&mut Core`** — today's entry bodies,
//!   moved verbatim into [`handle`]'s match (plus the stored read-only
//!   host env, `crate::env`). They cannot await, so the
//!   borrow-across-await class of bug is unwritable.
//! - **A handler gets NO sender to the loop** — the reply oneshot rides
//!   IN the message, so a handler can answer but never post; the
//!   self-RPC deadlock class is unwritable. Only the bridges post (and
//!   in this phase only the bridges exist).
//! - **No walking flag, no borrow choreography** — mutations are
//!   message-serialized on the loop's FIFO; concurrent unresolved calls
//!   resolve in send order.
//! - **Panics isolate to one oneshot** — `catch_unwind` keeps the loop
//!   alive; a panicked handler drops its reply channel and the bridge
//!   answers the error envelope, never a dead core. (On the wasm32
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
use crate::{json_envelope, locations_envelope, Core};

/// one ABI call in flight. The `tx` rides IN the message — the handler
/// replies on it, and the field set is owned data (the bridge copies the
/// host's `&str`/`&[u8]` before posting, so the future never borrows).
pub enum Msg {
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
    /// the injected READ-ONLY host environment (the dep walk's
    /// capability — `crate::env`). Carried as the [`env::Io`] trait
    /// object: the wasm bridge wraps its `JsEnv` in the `Rc` at
    /// `set_env`, native tests hand over their own `Io` — one
    /// capability lane, two constructors.
    SetEnv {
        env: Rc<dyn crate::env::Io>,
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

/// a handler's reply channel — one oneshot per message.
pub type Tx = Sender<Reply>;

/// what a handler answers. `Text` is the JSON envelope every query and
/// parse entry speaks; `Done` is the void entries' completion (the
/// bridge awaits it for ORDERING, not payload).
pub enum Reply {
    Text(String),
    Done,
}

/// the loop's body — own the core exclusively, answer one message at a
/// time, never die on a bad handler. The injected read-only host env
/// (`set_env`) lives beside the core: `None` until the host hands one
/// over, read only by the dep walk.
pub(crate) async fn run(core: Core, mut inbox: UnboundedReceiver<Msg>) {
    let mut core = core;
    let mut env: Option<Rc<dyn crate::env::Io>> = None;
    while let Some(msg) = inbox.next().await {
        // panic isolation: one bad handler drops ONE reply channel (the
        // bridge answers its error envelope); the loop never dies. The
        // handler may leave the core half-mutated — accepted; the next
        // message sees whatever state stands.
        let _ = catch_unwind(AssertUnwindSafe(|| handle(&mut core, &mut env, msg)));
    }
}

/// the entries' bodies — sync arms over `&mut Core` (and the stored
/// env, which only the dep walk reads). No sender, no await.
fn handle(core: &mut Core, env: &mut Option<Rc<dyn crate::env::Io>>, msg: Msg) {
    match msg {
        Msg::Legend { tx } => {
            let _ = tx.send(Reply::Text(json_envelope(serde_json::json!(
                crate::semantic::TokenType::legend()
            ))));
        }
        Msg::Analyze { uri, src, tx } => {
            // open / FULL-sync change: the doc stores FIRST (the analyze
            // lane's store — the queries below read the same text)
            core.docs.insert(uri.clone(), src.clone());
            // analyze_at stamps related-info URIs + hover provenance at
            // construction; a degenerate URI still analyzes (unstamped
            // twin below)
            let a = match Uri::from_str(&uri) {
                Ok(u) => crate::analysis::analyze_at(&u, &src),
                Err(_) => crate::analysis::analyze_unstamped(&uri, &src),
            };
            let diags = {
                let dm = core.doc_mods_of(&uri);
                let groups = if dm.pkg.is_some() {
                    crate::check::groups_from_defs(&core.defs)
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
        Msg::Forget { uri, tx } => {
            core.docs.remove(&uri);
            let _ = tx.send(Reply::Done);
        }
        Msg::Hover { uri, line, ch, tx } => {
            let answer = match core.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let doc = core.doc_mods_of(&uri);
                    let out = crate::analysis::hover_at(&uri, &src, &core.defs, &doc, line, ch);
                    json_envelope(serde_json::json!(out))
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::Complete {
            uri,
            src,
            line,
            ch,
            tx,
        } => {
            // the doc-sync entry: the CURRENT text stores FIRST (see the
            // bridge's doc — the analyze lane debounces, completion fires
            // on the keystroke)
            core.docs.insert(uri.clone(), src);
            let answer = core.complete_stored(&uri, line, ch);
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::Definition { uri, line, ch, tx } => {
            let answer = match core.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let doc = core.doc_mods_of(&uri);
                    let locs =
                        crate::analysis::definition_at(&uri, &src, &core.defs, &doc, line, ch);
                    locations_envelope(&locs)
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::TypeDefinition { uri, line, ch, tx } => {
            let answer = match core.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let doc = core.doc_mods_of(&uri);
                    let locs =
                        crate::analysis::type_definition_at(&uri, &src, &core.defs, &doc, line, ch);
                    locations_envelope(&locs)
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::Inlay {
            uri,
            start_line,
            start_ch,
            end_line,
            end_ch,
            tx,
        } => {
            let answer = match core.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let hints = crate::analysis::inlay_hints_at(
                        &uri,
                        &src,
                        &core.defs,
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
        Msg::References {
            uri,
            line,
            ch,
            include_decl,
            tx,
        } => {
            let answer = match core.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let doc = core.doc_mods_of(&uri);
                    let locs = crate::analysis::references_at(
                        &uri,
                        &src,
                        &core.defs,
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
        Msg::SignatureHelp { uri, line, ch, tx } => {
            let answer = match core.docs.get(&uri).cloned() {
                None => "null".to_string(),
                Some(src) => {
                    let out = crate::analysis::signature_help_at(&uri, &src, &core.defs, line, ch);
                    json_envelope(serde_json::json!(out))
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::AddDef { uri, src, tx } => {
            let idx = crate::analysis::index_at(&uri, &src);
            match core.defs.iter().position(|d| d.origin == uri) {
                Some(slot) => core.defs[slot] = idx,
                None => core.defs.push(idx),
            }
            let _ = tx.send(Reply::Done);
        }
        Msg::ParseManifest { text, tx } => {
            let answer = match crate::deps::dep_table_pinned(&text, &core.lock_pins) {
                Ok(t) => json_envelope(wire::DepTable::from(&t)),
                Err(e) => json_envelope(wire::Error::new(e)),
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::SetEnv { env: e, tx } => {
            // the capability STORE — replace-on-reinject, exactly the
            // ledger setters' shape. The walk (ResolveDeps below) is
            // the one reader; the Rc hands it the same cached
            // functions the host injected.
            *env = Some(e);
            let _ = tx.send(Reply::Done);
        }
        Msg::ResolveDeps { roots, tx } => {
            // the read-only walk: one fast handler turn over the
            // stored env — no walk task, no awaits. An env-less core
            // answers the loud error envelope, never a silent empty
            // walk (the demo path never calls this entry).
            let answer = match env.as_ref() {
                None => json_envelope(wire::Error::new(
                    "no host environment — the dep walk reads through the injected env \
                     (load(bytes, env) / set_env)",
                )),
                Some(io) => json_envelope(wire::WalkOut::from(&crate::walk::resolve(
                    core,
                    io.as_ref(),
                    &roots,
                ))),
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::AddDefMod {
            uri,
            name,
            mod_path,
            src,
            tx,
        } => {
            let decls = core.add_def_mod_at(&uri, &name, &mod_path, &src);
            let _ = tx.send(Reply::Text(json_envelope(wire::ModDecls { decls })));
        }
    }
}

/// the executor seam: the wasm face parks the loop on the JS microtask
/// queue (`spawn_local`); native ABI tests drive it on a `LocalPool`
/// they own (`RutLsp::spawn_native`). Answers the inbox handle the
/// bridges post on.
pub(crate) fn spawn(core: Core) -> UnboundedSender<Msg> {
    let (tx, rx) = unbounded::<Msg>();
    #[cfg(target_family = "wasm")]
    wasm_bindgen_futures::spawn_local(run(core, rx));
    // A native build has no executor for a plain `RutLsp::new()` — the
    // ABI face belongs to the wasm; native tests go through
    // `spawn_native`, which parks the loop on a pool IT hands back.
    // Here the inbox drops and every bridge answers the loop-gone
    // envelope — the honest dead-loop shape.
    #[cfg(not(target_family = "wasm"))]
    drop((core, rx));
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
