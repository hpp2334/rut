//! Signature help — the callee machinery, read at a call's argument
//! position (the lsp-features survey §4.4). The call SHAPE is the
//! tree's: the cursor's node walks its ancestors to the innermost
//! [`CallNode`]/[`MethodNode`], unclosed calls riding their `rparen:
//! None` shape, and the ACTIVE argument counts from the child spans —
//! the comma/paren archaeology this replaces could not see the tree
//! exactly when it fired (mid-edit); now the partial call IS a node.
//! The CALLEE resolves through the resolver family:
//!
//! * free calls by unique-name match over the recorded index params
//!   (the `call_ty` law — ambiguity is a miss, never a wrong signature);
//! * methods through the receiver's type head (a plain-ident receiver
//!   only — the chained-receiver law's honest miss stays), then the
//!   inherent impl fn via the shared `impl_method_fn` rule;
//!   own-surface hits (interface bodies, builtin/primitive surfaces) are
//!   FINAL with no recorded params → no help, so a same-named impl fn's
//!   signature can never leak through (the known-receiver-is-final law);
//! * a capitalized single-seg callee misses the free-fn lane into the
//!   CTOR fallback, gated by the FILE SCOPE (`type_decl` — an unbound
//!   or ambiguous name answers nothing, never wrong help from a
//!   foreign same-named class): the bound class's designated
//!   constructor answers with its verbatim impl signature from the
//!   class's OWN file (`is_method = false` — a ctor takes no receiver,
//!   its params ARE the call's args; the compiler's inherent-impl
//!   rule), a positional newtype with its synthesized mint label
//!   (`TyView::mint_sig`, the hover's mint line's spelling). Anything
//!   else stays a miss — no help, never wrong help.
//!
//! The signature renders verbatim (the fn view's own slice — the fn
//! name and params as written, the hover render's source). The
//! never-wrong guard is phase 3's: ambiguity, unknown receiver,
//! own-surface finality, or MORE arguments than the signature has
//! parameters — no help, never wrong help. The LIFECYCLE: a call still
//! being typed (fewer args than parameters) is the help's whole point
//! and shows the signature with the arg under the cursor highlighted —
//! through the closer's END too (the just-typed or type-overed `)`
//! reads the same hint), and the next keystroke past the closer is out
//! of the call.

use ls_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

use rut_ast::{CallNode, ExprRef, MethodNode};

use crate::resolve::View;
use crate::world::ty::Ty;
use crate::world::{FileId, World};

/// the resolved fn view behind a callee door's [`crate::resolve::Def`]
/// — the doors answer typed defs; the signature renders through the
/// view (a callee def is always a fn node, so the view is always
/// [`View::Fn`])
fn fn_view<'w>(
    w: &'w World,
    d: crate::resolve::Def,
) -> Option<crate::world::collection::FnView<'w>> {
    match d.view(w) {
        Some(View::Fn(f)) => Some(f),
        _ => None,
    }
}

/// `textDocument/signatureHelp` at an LSP position (already converted to
/// a byte offset). `None` when there is no call to help with, or when
/// the callee does not resolve to exactly one known signature.
pub fn signature_help(w: &World, file: FileId, pos: u32) -> Option<SignatureHelp> {
    let d = w.file(file);
    let ast = d.doc.ast();
    // the cursor's node — the byte under it, else the byte just before
    // (a cursor sitting on a token's end reads that token) — then the
    // innermost call ancestor: nested calls each answer at their own
    // depth, and a control paren (`if (`) is no call at all — its
    // condition simply walks up PAST it
    let probe = crate::semantic::node_at_caret(&d.doc, pos)?;
    // the cursor ON a callee's name belongs to the ENCLOSING call
    // (the old paren scan's adjacency: pos before the callee's `(` is
    // not inside its arg list) — a callee whose own name covers pos is
    // skipped in favor of the next call up
    let call = ast.ancestors(probe).find(|h| is_call_at(h, ast, pos));
    let call = match call {
        Some(c) => {
            // a closed call answers while the cursor still rides it —
            // through the closer's END (the just-typed / type-overed
            // `)` reads the same hint) — and the NEXT keystroke past
            // it is out of the call
            let inside = if let Some(m) = c.downcast::<MethodNode>() {
                m.get(ast).rparen.is_none_or(|rp| pos <= rp.hi)
            } else if let Some(cc) = c.downcast::<CallNode>() {
                cc.get(ast).rparen.is_none_or(|rp| pos <= rp.hi)
            } else {
                unreachable!("the ancestor probe found a call kind")
            };
            if !inside {
                return None;
            }
            c
        }
        None => {
            // the DECL site: the cursor on a fn/method's own name or
            // param list answers with THAT signature (the active slot
            // rides the param the cursor names)
            return signature_help_at_decl(w, file, probe, pos);
        }
    };

    // the callee + the args, from the node
    let (fndef, params, fndef_doc, args, is_method) = if let Some(m) = call.downcast::<MethodNode>()
    {
        let md = m.get(ast);
        let name = md.member?.ident;
        // the receiver must be ONE plain identifier (`c`, `Circle`) —
        // a chained receiver (`c.grown(2).grown(`) or a complex one is
        // an honest miss (the known-receiver law the tests pin); `self`
        // and the other keywords are not receivers here either
        let recv_id = match md.recv {
            ExprRef::Path(ph) => {
                let segs = &ph.get(ast).segs;
                (segs.len() == 1).then(|| segs[0].name.ident)
            }
            _ => None,
        }?;
        let recv_text = ast.name(recv_id);
        if crate::semantic::is_keyword(recv_text) {
            return None;
        }
        let ty_name = crate::world::collection::named_recv(w, file, pos, recv_id)?;
        // own-surface finality: `None` here is FINAL — no help, never
        // a same-named impl fn's signature
        let fndef = fn_view(
            w,
            crate::world::collection::impl_method_fn(w, file, &ty_name, ast.name(name))?,
        )?;
        (
            fndef.src(),
            fndef.params(),
            fndef.doc_lines(),
            md.args.clone(),
            true,
        )
    } else {
        let cd = call.downcast::<CallNode>()?.get(ast);
        // free fn by unique match (the call_ty law: ambiguity is a
        // miss); the callee must be a plain single-seg name
        let callee_id = match cd.callee {
            ExprRef::Path(ph) => {
                let segs = &ph.get(ast).segs;
                (segs.len() == 1).then(|| segs[0].name.ident)
            }
            _ => None,
        }?;
        let name = ast.name(callee_id);
        // the ctor fallback lane — a single capitalized callee
        // names a class in call position. The FILE SCOPE gates the
        // whole fallback: the queried file must bind the class
        // (`type_decl`) — an unbound or ambiguous name answers
        // nothing (the compiler refuses both; hover's candidates
        // lane answers the ambiguity), never wrong help from a
        // foreign same-named class. The newtype answers through
        // the mint arm (never the marker sweep — the engine's
        // `Ctx::class_ctors` law: a newtype never registers; the
        // synthesized label mentions the param name so the
        // alignment check holds); the designated ctor answers with
        // its verbatim impl signature from the class's OWN file
        // (the compiler's inherent-impl rule), `is_method = false`
        // (a ctor takes no receiver — its params ARE the call's
        // args). Anything else stays a miss — no help, never wrong
        // help.
        if let Some(one) =
            crate::world::collection::unique_free_fn(w, file, name).and_then(|d| fn_view(w, d))
        {
            (
                one.src(),
                one.params(),
                one.doc_lines(),
                cd.args.clone(),
                false,
            )
        } else {
            if !crate::world::infer::is_cap(name) {
                return None;
            }
            let t = crate::world::collection::type_decl(w, file, name)?;
            if t.is_newtype() {
                let label = t.mint_sig()?;
                (label, t.mint_params(), Vec::new(), cd.args.clone(), false)
            } else {
                let f = crate::world::collection::constructor_of(t)?;
                (f.src(), f.params(), f.doc_lines(), cd.args.clone(), false)
            }
        }
    };

    // render: the verbatim signature, params as written
    let (label, pieces) = signature_parts(&fndef, &params, is_method)?;

    // the ACTIVE parameter from the child spans: the argument the
    // cursor rides (closed on both ends — a cursor at an arg's start
    // or end reads that arg), else the separators before it (the
    // depth-0 comma count this replaces). A call still being typed has
    // fewer args than params — that IS the help's moment; MORE
    // arguments than params is a mismatch → no help
    let separators = args.iter().filter(|a| a.span(ast).hi < pos).count() as u32;
    let ride = args
        .iter()
        .position(|a| {
            let sp = a.span(ast);
            sp.lo <= pos && pos <= sp.hi
        })
        .map(|i| i as u32)
        .unwrap_or(separators);
    let active_parameter = if params.is_empty() {
        if separators > 0 {
            return None; // arguments where none belong
        }
        None
    } else {
        if ride >= params.len() as u32 {
            return None;
        }
        Some(ride)
    };

    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: doc_markdown(&fndef_doc),
            parameters: Some(
                pieces
                    .into_iter()
                    .map(|p| ParameterInformation {
                        label: ParameterLabel::Simple(p),
                        documentation: None,
                    })
                    .collect(),
            ),
            active_parameter,
        }],
        active_signature: Some(0),
        active_parameter,
    })
}

/// the DECL-site signature help: the cursor rides a fn/method's own
/// name or param list — THAT signature answers (the recorded params as
/// written; the paren itself = the first slot). A surface fn's view
/// resolves through the same index the call lane reads.
fn signature_help_at_decl(
    w: &World,
    file: FileId,
    probe: rut_ast::AnyHandle,
    pos: u32,
) -> Option<SignatureHelp> {
    let d = w.file(file);
    let ast = d.doc.ast();
    for a in ast.ancestors(probe) {
        if a.is::<rut_ast::ModuleNode>() {
            return None;
        }
        // the fn/method the cursor rides — its name position or param
        // region decides; the view (recorded params, verbatim slice)
        // comes from the chain
        let (name, nsp, param_spans, is_method) = if let Some(f) = a.downcast::<rut_ast::FnNode>() {
            let fd = f.get(ast);
            (fd.name.ident, fd.name.span, fd.params.clone(), false)
        } else if let Some(m) = a.downcast::<rut_ast::MethodDeclNode>() {
            let md = m.get(ast);
            (md.name.ident, md.name.span, md.params.clone(), true)
        } else {
            continue;
        };
        let on_slot = crate::semantic::on_name(nsp, pos)
            || param_spans
                .iter()
                .any(|pm| pm.span(ast).lo <= pos && pos <= pm.span(ast).hi);
        if !on_slot {
            continue;
        }
        let name_text = ast.name(name);
        let (view, is_method) = if is_method {
            // the impl fn's view — the owner derives from the parent
            // impl's target (an interface decl surfaces FINAL: no
            // recorded params, no help)
            let owner = ast.parent(a).and_then(|p| {
                if let Some(t) = p.downcast::<rut_ast::ImplNode>() {
                    let td = t.get(ast);
                    return Some(Ty::of(ast, td.target).head());
                }
                p.downcast::<rut_ast::InterfaceNode>()
                    .map(|t| ast.name(t.get(ast).name.ident).to_string())
            })?;
            (
                crate::world::collection::impl_method_fn(w, file, &owner, name_text)
                    .and_then(|dd| fn_view(w, dd))?,
                true,
            )
        } else {
            let one = crate::world::collection::unique_free_fn(w, file, name_text)
                .and_then(|dd| fn_view(w, dd))?;
            (one, false)
        };
        let params = view.params();
        let (label, pieces) = signature_parts(&view.src(), &params, is_method)?;
        let active_parameter = if params.is_empty() {
            None
        } else {
            let ride = param_spans
                .iter()
                .position(|pm| {
                    let sp = pm.span(ast);
                    sp.lo <= pos && pos <= sp.hi
                })
                .map(|i| i as u32)
                .unwrap_or(0);
            (ride < params.len() as u32).then_some(ride)
        };
        return Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: doc_markdown(&view.doc_lines()),
                parameters: Some(
                    pieces
                        .into_iter()
                        .map(|p| ParameterInformation {
                            label: ParameterLabel::Simple(p),
                            documentation: None,
                        })
                        .collect(),
                ),
                active_parameter,
            }],
            active_signature: Some(0),
            active_parameter,
        });
    }
    None
}

/// is `h` the call node the cursor rides — a call/method whose CALLEE
/// name does not itself cover `pos` (on the callee's name = the
/// enclosing call's argument region)
fn is_call_at(h: &rut_ast::AnyHandle, ast: &rut_ast::Ast, pos: u32) -> bool {
    let on_callee = if let Some(c) = h.downcast::<CallNode>() {
        match c.get(ast).callee {
            ExprRef::Path(ph) => ph
                .get(ast)
                .segs
                .iter()
                .any(|s| s.name.span.lo <= pos && pos <= s.name.span.hi),
            _ => false,
        }
    } else if let Some(m) = h.downcast::<MethodNode>() {
        m.get(ast)
            .member
            .is_some_and(|n| n.span.lo <= pos && pos <= n.span.hi)
    } else {
        return false;
    };
    !on_callee
}

/// the verbatim signature as the label, plus each parameter's as-written
/// piece (a substring of the label, per the LSP contract). `self` (or
/// `mut self`) never surfaces — the recorded params exclude it, and the
/// surviving pieces must line up with them or there is no help.
fn signature_parts(src: &str, params: &[String], is_method: bool) -> Option<(String, Vec<String>)> {
    let open = src.find('(')?;
    let mut depth = 0i32;
    let mut close = None;
    for (off, c) in src[open..].char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + off);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let section = &src[open + 1..close];
    // top-level commas only — a tuple type inside a param (`-> (a, b)`
    // callers, `HashMap<i32, str>` generics) must not split
    let mut pieces: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut paren = 0i32;
    let mut angle = 0i32;
    for c in section.chars() {
        match c {
            '(' | '[' => {
                paren += 1;
                cur.push(c);
            }
            ')' | ']' => {
                paren -= 1;
                cur.push(c);
            }
            '<' => {
                angle += 1;
                cur.push(c);
            }
            '>' => {
                angle -= 1;
                cur.push(c);
            }
            ',' if paren == 0 && angle == 0 => {
                pieces.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        pieces.push(cur.trim().to_string());
    }
    if is_method {
        let first = pieces.first()?;
        if first != "self" {
            return None; // the recorded params exclude self — a mismatch is degenerate
        }
        pieces.remove(0);
    }
    if pieces.len() != params.len() {
        return None;
    }
    // each piece must mention its recorded param name — the recorded
    // names come from the AST, the pieces from the verbatim slice; if
    // they disagree the parse is degenerate and the honest answer is none
    for (name, piece) in params.iter().zip(&pieces) {
        if !piece
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == name)
        {
            return None;
        }
    }
    Some((src.to_string(), pieces))
}

/// the fn's doc lines as the signature documentation (markdown), only
/// when there are any
fn doc_markdown(doc: &[String]) -> Option<Documentation> {
    if doc.is_empty() {
        return None;
    }
    Some(Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: doc.join("\n"),
    }))
}
