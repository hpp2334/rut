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

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::str::FromStr;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot::Sender;
use futures::StreamExt;
use ls_types::Uri;

use crate::wire;
use crate::{env::JsEnv, json_envelope, locations_envelope, Core};

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
        line: u32,
        ch: u32,
        tx: Tx,
    },
    CompleteIn {
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
    SetLock {
        text: String,
        tx: Tx,
    },
    SetDirectDeps {
        names_json: String,
        tx: Tx,
    },
    /// the injected READ-ONLY host environment (the dep walk's
    /// capability — `crate::env`). This phase's whole change: the loop
    /// STORES it; no handler reads it yet (the walk is the next
    /// phase). Carried by value; the handler wraps it in the `Rc` the
    /// loop keeps.
    SetEnv {
        env: JsEnv,
        tx: Tx,
    },
    AddDefNamed {
        uri: String,
        name: String,
        src: String,
        tx: Tx,
    },
    AddDefMod {
        uri: String,
        name: String,
        mod_path: String,
        src: String,
        tx: Tx,
    },
    AddBundle {
        bytes: Vec<u8>,
        tx: Tx,
    },
    ExtractBundle {
        bytes: Vec<u8>,
        tx: Tx,
    },
    ExtractSrczip {
        bytes: Vec<u8>,
        tx: Tx,
    },
    DocLen {
        uri: String,
        tx: Tx,
    },
}

/// a handler's reply channel — one oneshot per message.
pub type Tx = Sender<Reply>;

/// what a handler answers. `Text` is the JSON envelope every query and
/// parse entry speaks; `Len` is `doc_len`'s count; `Done` is the void
/// entries' completion (the bridge awaits it for ORDERING, not payload).
pub enum Reply {
    Text(String),
    Len(u32),
    Done,
}

/// the loop's body — own the core exclusively, answer one message at a
/// time, never die on a bad handler. The injected read-only host env
/// (`set_env`) lives beside the core: `None` until the host hands one
/// over, stored (not read) this phase.
pub(crate) async fn run(core: Core, mut inbox: UnboundedReceiver<Msg>) {
    let mut core = core;
    let mut env: Option<Rc<JsEnv>> = None;
    while let Some(msg) = inbox.next().await {
        // panic isolation: one bad handler drops ONE reply channel (the
        // bridge answers its error envelope); the loop never dies. The
        // handler may leave the core half-mutated — accepted; the next
        // message sees whatever state stands.
        let _ = catch_unwind(AssertUnwindSafe(|| handle(&mut core, &mut env, msg)));
    }
}

/// the entries' bodies — today's `RutLsp` methods, moved verbatim into
/// sync arms over `&mut Core` (and the stored env, which only the dep
/// walk's phase will read). No sender, no await.
fn handle(core: &mut Core, env: &mut Option<Rc<JsEnv>>, msg: Msg) {
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
            // analyze_at wires related-info URIs + hover provenance, exactly
            // wired like `analysis::analyze_at`; a degenerate URI still analyzes
            let a = match Uri::from_str(&uri) {
                Ok(u) => crate::analysis::analyze_at(&u, &src, crate::analysis::mode_of(&uri)),
                Err(_) => {
                    let mut a = crate::analysis::analyze(&src, crate::analysis::mode_of(&uri));
                    a.index.origin = uri.clone();
                    a
                }
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
        Msg::Complete { uri, line, ch, tx } => {
            let answer = core.complete_stored(&uri, line, ch);
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::CompleteIn {
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
        Msg::SetLock { text, tx } => {
            let answer = match crate::deps::lock_pins(&text) {
                Ok(pins) => {
                    let n = pins.len();
                    core.lock_pins = pins;
                    json_envelope(wire::Pins { pins: n })
                }
                Err(e) => {
                    core.lock_pins = BTreeMap::new();
                    json_envelope(wire::Error::new(e))
                }
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::SetDirectDeps { names_json, tx } => {
            let answer = match serde_json::from_str::<Vec<String>>(&names_json) {
                Ok(names) => {
                    for n in names {
                        core.direct_deps.insert(n);
                    }
                    json_envelope(wire::Direct {
                        direct: core.direct_deps.len(),
                    })
                }
                Err(e) => json_envelope(wire::Error::new(e.to_string())),
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::SetEnv { env: e, tx } => {
            // the capability STORE — replace-on-reinject, exactly the
            // ledger setters' shape. No handler reads it this phase;
            // the Rc hands the walk (the next phase) a cheap clone of
            // the same cached functions.
            *env = Some(Rc::new(e));
            let _ = tx.send(Reply::Done);
        }
        Msg::AddDefNamed { uri, name, src, tx } => {
            let mut idx = crate::deps::index_dep(
                &name,
                &uri,
                &src,
                crate::analysis::mode_of(&uri),
                None,
                &[],
            );
            idx.direct_dep = core.direct_deps.contains(&name);
            match core.defs.iter().position(|d| d.origin == uri) {
                Some(slot) => core.defs[slot] = idx,
                None => core.defs.push(idx),
            }
            let _ = tx.send(Reply::Done);
        }
        Msg::AddDefMod {
            uri,
            name,
            mod_path,
            src,
            tx,
        } => {
            let mut idx = crate::deps::index_dep_mod(
                &name,
                &uri,
                &mod_path,
                &src,
                crate::analysis::mode_of(&uri),
                None,
                &[],
            );
            idx.direct_dep = core.direct_deps.contains(&name);
            let decls: Vec<wire::ModDecl> = idx
                .mods
                .iter()
                .map(|m| wire::ModDecl {
                    name: m.name.clone(),
                    vis: if matches!(m.vis, rut_ast::ast::Vis::Pub) {
                        "pub"
                    } else {
                        "mod"
                    },
                })
                .collect();
            match core.defs.iter().position(|d| d.origin == uri) {
                Some(slot) => core.defs[slot] = idx,
                None => core.defs.push(idx),
            }
            let _ = tx.send(Reply::Text(json_envelope(wire::ModDecls { decls })));
        }
        Msg::AddBundle { bytes, tx } => {
            let b = match crate::deps::bundle_sources(&bytes) {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(Reply::Text(json_envelope(
                        serde_json::json!({ "error": e }),
                    )));
                    return;
                }
            };
            let mut paths = Vec::with_capacity(b.files.len());
            let direct = core.direct_deps.contains(&b.module);
            for f in &b.files {
                let uri = format!("bundle:{}/{}", b.module, f.path);
                // a mounted mod child rides as `<path>/mod.rut` (a host tree's
                // as `<path>/mod.d.rut`, the rows lane's root as `mod.rut`) —
                // its mod path stamps the index in the file's own mode; flat
                // files keep today's shape. A DECL file is the declaration
                // entry (wire v12): ONE parse, the inline nesting stamps one
                // index per module — the consolidated group entry indexes whole.
                let idxs: Vec<_> = match crate::deps::bundle_file_mod_path(&f.path) {
                    Some(p) if f.mode == rut_parser::Mode::Decl => {
                        crate::deps::index_dep_decl_group(
                            &b.module,
                            &uri,
                            &p,
                            &f.src,
                            b.namespace.as_deref(),
                            &b.consts,
                        )
                        .into_iter()
                        .map(|(_, idx)| idx)
                        .collect()
                    }
                    Some(p) => vec![crate::deps::index_dep_mod(
                        &b.module,
                        &uri,
                        &p,
                        &f.src,
                        f.mode,
                        b.namespace.as_deref(),
                        &b.consts,
                    )],
                    None => vec![crate::deps::index_dep(
                        &b.module,
                        &uri,
                        &f.src,
                        f.mode,
                        b.namespace.as_deref(),
                        &b.consts,
                    )],
                };
                for mut idx in idxs {
                    idx.direct_dep = direct;
                    // one uri per module: the shared entry uri for the root
                    // stamp, `#<mod path>` for the nested ones (the (module,
                    // mod_path) pair is the real key; the uri only names the
                    // origin in diagnostics)
                    match core.defs.iter().position(|d| d.origin == uri) {
                        Some(slot) => core.defs[slot] = idx,
                        None => core.defs.push(idx),
                    }
                }
                paths.push(f.path.clone());
            }
            let answer = json_envelope(wire::BundleIndex {
                module: b.module,
                namespace: b.namespace,
                consts: b.consts,
                files: paths,
            });
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::ExtractBundle { bytes, tx } => {
            let answer = match crate::deps::bundle_sources(&bytes) {
                Ok(b) => {
                    let files: Vec<wire::BundleFile> =
                        b.files.iter().map(wire::BundleFile::from_source).collect();
                    json_envelope(wire::ExtractBundle {
                        module: b.module,
                        namespace: b.namespace,
                        consts: b.consts,
                        files,
                    })
                }
                Err(e) => json_envelope(serde_json::json!({ "error": e })),
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::ExtractSrczip { bytes, tx } => {
            let answer = match crate::deps::srczip_sources(&bytes) {
                Ok(b) => {
                    let files: Vec<wire::BundleFile> = b
                        .files
                        .iter()
                        .map(|f| {
                            let rel = f.path.strip_prefix("src/").unwrap_or(&f.path);
                            let mod_path = if rel == "mod.rut" || rel == "mod.d.rut" {
                                Some(String::new())
                            } else if let Some(p) = rel.strip_suffix("/mod.rut") {
                                Some(p.to_string())
                            } else if let Some(p) = rel.strip_suffix("/mod.d.rut") {
                                Some(p.to_string())
                            } else {
                                rel.strip_suffix(".rut").map(|p| p.to_string())
                            };
                            wire::BundleFile {
                                path: f.path.clone(),
                                src: f.src.clone(),
                                mode: wire::mode_str(f.mode),
                                mod_path,
                            }
                        })
                        .collect();
                    json_envelope(wire::ExtractSrczip {
                        module: b.module,
                        namespace: b.namespace,
                        consts: b.consts,
                        files,
                        // the unpack law's ingredients: the archive's own
                        // manifest (the host lays it beside the files and the
                        // pkg's own rows mount through the ordinary dir lane)
                        // and the embedded `deps/<identity>/` entries (the
                        // ride-along groups, laid under the tree; first mount
                        // wins)
                        manifest: b.manifest,
                        embedded: b
                            .embedded
                            .iter()
                            .map(|(path, src)| wire::Embedded {
                                path: path.clone(),
                                src: src.clone(),
                            })
                            .collect(),
                    })
                }
                Err(e) => json_envelope(serde_json::json!({ "error": e })),
            };
            let _ = tx.send(Reply::Text(answer));
        }
        Msg::DocLen { uri, tx } => {
            let _ = tx.send(Reply::Len(
                core.docs.get(&uri).map(|s| s.len() as u32).unwrap_or(0),
            ));
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
