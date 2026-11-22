//! The typing edge — the walk that names the completion context from
//! the tree: the byte before the cursor lands the probe, and the
//! probe's own node kind (or its ancestors') names the tier — the
//! Edge verdict the dispatcher matches. No token scans.

use rut_ast::{AnyHandle, Ast, FieldNode, MethodNode, NodeKindId, PathNode, UseNode};

use crate::world::collection::Chain;

use super::{
    bare_prefix_walk, gap_spells, on_name, partial_text, ty_slot_dot, ty_slot_prefix, Edge,
};

/// the typing edge — the walk: the byte before the cursor lands the
/// probe, and the probe's own node kind (or its ancestors') names the
/// tier. A use node carries its own spans; a partial member is the
/// tree's `member: None` shape; the annotation boundary reads the one
/// punct the tree does not record (the `:`/`->` between the binding
/// head and the cursor — a text probe, the dot guard's precedent).
pub(super) fn typing_edge(chain: &Chain, pos: u32, src: &str) -> Edge {
    let head = chain.head();
    let ast = head.doc.ast();
    // the byte before the cursor names the context. A STRUCTURAL
    // wrapper (the block, the fn, the root) under the cursor means the
    // byte sits in dead space — the walk looks a little further back
    // for the node the cursor is actually typing into (the blank after
    // a colon, the gap at EOF); the first non-wrapper stands.
    const CONTEXT_KINDS: [NodeKindId; 10] = [
        NodeKindId::LetStmt,
        NodeKindId::Fn,
        NodeKindId::Use,
        NodeKindId::Param,
        NodeKindId::FieldDecl,
        NodeKindId::Method,
        NodeKindId::Field,
        NodeKindId::Path,
        NodeKindId::TyPath,
        NodeKindId::Call,
    ];
    let mut p0 = None;
    for back in 1..=16u32 {
        match pos.checked_sub(back).and_then(|p| ast.node_at(p)) {
            Some(h) if matches!(h.kind, NodeKindId::Module | NodeKindId::Block) => continue,
            Some(h) if CONTEXT_KINDS.contains(&h.kind) => {
                p0 = Some(h);
                break;
            }
            Some(h) => {
                p0 = Some(h);
                break;
            }
            None => continue,
        }
    }
    let Some(p0) = p0 else {
        return Edge::Bare {
            prefix: String::new(),
        };
    };
    // the use tier — a Use ancestor (uses are ITEMS: only a probe
    // inside the use statement itself has one)
    for a in ast.ancestors(p0) {
        if let Some(u) = a.downcast::<UseNode>() {
            return use_edge(head, ast, a, u.get(ast), pos);
        }
        if a.is::<rut_ast::ModuleNode>() {
            break;
        }
    }
    // the member lane — a method call (the partial shape rides its
    // absent member as the empty prefix) or a field access
    if let Some(m) = p0.downcast::<MethodNode>() {
        let d = m.get(ast);
        let prefix = d
            .member
            .map(|n| partial_text(head, n.span, pos))
            .unwrap_or_default();
        return Edge::Member {
            node: p0,
            recv: d.recv,
            prefix,
        };
    }
    if let Some(f) = p0.downcast::<FieldNode>() {
        let d = f.get(ast);
        return Edge::Member {
            node: p0,
            recv: d.recv,
            prefix: partial_text(head, d.name.span, pos),
        };
    }
    // the qualified lane — a `::` path whose TAIL segment the cursor
    // rides (a single-seg path is a bare position; the head segments
    // are the walk)
    if let Some(p) = p0.downcast::<PathNode>() {
        let segs = &p.get(ast).segs;
        if segs.len() >= 2 {
            if let Some(i) = segs.iter().position(|s| on_name(s.name.span, pos)) {
                if i + 1 == segs.len() {
                    return Edge::Qualified {
                        segs: segs[..i]
                            .iter()
                            .map(|s| ast.name(s.name.ident).to_string())
                            .collect(),
                        prefix: partial_text(head, segs[i].name.span, pos),
                    };
                }
            }
        }
        // the cursor strictly past the last segment — the dangling
        // `::` (the head's members tier): every seg is the walk, the
        // prefix empty. (A cursor ON the name's end is the bare
        // typing edge, not a `::` request.)
        if let Some(last) = segs.last() {
            if last.name.span.hi < pos {
                return Edge::Qualified {
                    segs: segs
                        .iter()
                        .map(|s| ast.name(s.name.ident).to_string())
                        .collect(),
                    prefix: String::new(),
                };
            }
        }
    }
    // the annotation boundary — the let's (or fn ret's) type slot: the
    // cursor rides the slot's tail segment (the partial type), or the
    // slot is ABSENT with the `:`/`->` spelled between the head and the
    // cursor
    for a in ast.ancestors(p0) {
        if a.is::<rut_ast::ModuleNode>() {
            break;
        }
        if let Some(l) = a.downcast::<rut_ast::LetStmtNode>() {
            let d = l.get(ast);
            if let Some(t) = d.ty {
                if let Some(prefix) = ty_slot_prefix(head, t, pos) {
                    return Edge::Annotation { prefix };
                }
                if let Some(e) = ty_slot_dot(head, t, pos) {
                    return e;
                }
            } else if gap_spells(head, d.name.span.hi, pos, ':') {
                return Edge::Annotation {
                    prefix: String::new(),
                };
            }
        }
        // params and fields annotate like lets (`fn f(x: ⏐)`,
        // `struct S { id: u⏐ }`); the gap probe keys on the node's own
        // name span
        let slots = [
            a.downcast::<rut_ast::ParamNode>().map(|h| {
                let d = h.get(ast);
                (d.name.span, d.ty)
            }),
            a.downcast::<rut_ast::FieldDeclNode>()
                .map(|h| (h.get(ast).name.span, Some(h.get(ast).ty))),
        ];
        for (name_span, ty) in slots.into_iter().flatten() {
            if let Some(t) = ty {
                if let Some(prefix) = ty_slot_prefix(head, t, pos) {
                    return Edge::Annotation { prefix };
                }
                if let Some(e) = ty_slot_dot(head, t, pos) {
                    return e;
                }
            } else if gap_spells(head, name_span.hi, pos, ':') {
                return Edge::Annotation {
                    prefix: String::new(),
                };
            }
        }
        // fn rets ride the same slot lanes; an ABSENT ret with the
        // arrow spelled (the gap between the param list and the
        // cursor) is the arrow boundary
        let fn_lane = |h: Option<rut_ast::AnyHandle>| -> Option<Edge> {
            let fh = h?;
            let ret = if let Some(f) = fh.downcast::<rut_ast::FnNode>() {
                f.get(ast).ret
            } else {
                fh.downcast::<rut_ast::SurfaceFnNode>()?.get(ast).ret
            };
            let anchor = if let Some(f) = fh.downcast::<rut_ast::FnNode>() {
                let d = f.get(ast);
                d.params
                    .last()
                    .map(|p| p.span(ast).hi)
                    .unwrap_or(d.name.span.hi)
            } else {
                let d = fh.downcast::<rut_ast::SurfaceFnNode>()?.get(ast);
                d.params
                    .last()
                    .map(|p| p.span(ast).hi)
                    .unwrap_or(d.name.span.hi)
            };
            if let Some(t) = ret {
                if let Some(prefix) = ty_slot_prefix(head, t, pos) {
                    return Some(Edge::Annotation { prefix });
                }
                if let Some(e) = ty_slot_dot(head, t, pos) {
                    return Some(e);
                }
            } else if anchor < pos {
                let end = (pos as usize).min(head.doc.src().len());
                let gap = head.doc.src()[anchor as usize..end].trim_end();
                if gap.ends_with("->") && !gap.contains('{') {
                    return Some(Edge::Annotation {
                        prefix: String::new(),
                    });
                }
            }
            None
        };
        for h in [
            a.downcast::<rut_ast::FnNode>().map(|h| h.any()),
            a.downcast::<rut_ast::SurfaceFnNode>().map(|h| h.any()),
        ] {
            if let Some(e) = fn_lane(h) {
                return e;
            }
        }
    }
    Edge::Bare {
        prefix: bare_prefix_walk(head, p0, pos, src),
    }
}

/// the use tier's context, off the node's own recorded spans: a path
/// segment under the cursor walks the path BEFORE it with the partial
/// seg as the prefix; a leaf name (brace list or flat form) walks the
/// FULL path with the partial name as the prefix; past a separator
/// with nothing typed, the full path and an empty prefix.
fn use_edge(
    head: &crate::world::collection::MountedFile,
    ast: &Ast,
    node: AnyHandle,
    u: &UseNode,
    pos: u32,
) -> Edge {
    // the walk carries the path BEFORE the segment the cursor rides
    // (the riding segment is the typed prefix — `use po⏐` is the
    // module-NAME tier; a sync that ran past the incomplete leaf can
    // leave later tokens in the path — the statement's recovery — and
    // they are not the walk)
    let ridden = u
        .path
        .iter()
        .position(|n| n.span.lo <= pos && pos <= n.span.hi);
    let before: Vec<String> = match ridden {
        Some(k) => u.path[..k]
            .iter()
            .map(|n| ast.name(n.ident).to_string())
            .collect(),
        None => u
            .path
            .iter()
            .filter(|n| n.span.hi <= pos)
            .map(|n| ast.name(n.ident).to_string())
            .collect(),
    };
    for n in u.path.iter().chain(&u.names) {
        if on_name(n.span, pos) {
            return Edge::Use {
                node,
                segs: before,
                prefix: partial_text(head, n.span, pos),
            };
        }
    }
    Edge::Use {
        node,
        segs: before,
        prefix: String::new(),
    }
}
