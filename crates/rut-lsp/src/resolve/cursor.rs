//! The one classification of what sits under a caret (law L3). Every
//! feature asks [`cursor`] and matches the answer; the order below is
//! binding, and only one step can match a node:
//!
//! 1. the caret's node — the boundary convention
//!    ([`crate::doc::node_at_caret`]); an error fence names
//!    nothing;
//! 2. the `Self` spellings;
//! 3. a declaring name under the caret — a node decl ([`Def::Node`])
//!    or a binding row ([`Def::Binding`]);
//! 4. a `use` edge's own path segment or leaf;
//! 5. a member node at its member position (a method call, a field
//!    access — the typing edge carries no member yet);
//! 6. a multi-segment `::` path, value or type position;
//! 7. a single name position (a lone path segment, a struct-literal
//!    label, a single type-slot segment);
//! 8. otherwise nothing — a keyword, an operator, whitespace.
//!
//! The token residue lives in `Document::name_at` — no token scans
//! here.

use rut_ast::{
    AnyHandle, ExprRef, ForCNode, ForOfNode, IdentId, LetStmtNode, ParamNode, PathNode, SelfNode,
    SelfTyNode, StructLitNode, TyPathNode, UseNode,
};

use rut_lexer::span::Span;

use super::def::{decl_ident, Def};
use crate::doc::on_name;
use crate::world::{FileId, World};

/// a member door's receiver — the node-derived spelling: the receiver
/// expression (a method/field access), or the `::` path segment
/// immediately before the member
#[derive(Clone, Copy)]
pub(crate) enum Recv {
    Expr(ExprRef),
    Seg(IdentId),
}

/// where in a `use` edge the caret sits
#[derive(Clone, Copy)]
pub(crate) enum UseAt {
    /// the path's first segment — the package head
    Head,
    /// a deeper path segment — a module being walked through
    Segment(usize),
    /// a `{ .. }` leaf name — the imported binding
    Leaf(usize),
}

/// what sits under the caret — the only classification, every feature
#[derive(Clone)]
pub(crate) enum Cursor {
    /// the caret rides a declaring name: param, let, loop var, field,
    /// enum member, module let, fn, method, type, alias, `mod` edge
    Decl(Def),
    Use {
        node: AnyHandle,
        segs: Vec<String>,
        at: UseAt,
    },
    /// a `::` path, value or type position; the last segment is the tail
    Path {
        node: AnyHandle,
        segs: Vec<String>,
        at: usize,
        is_ty: bool,
    },
    /// `recv.member`, or `recv.` with the member still being typed
    Member {
        node: AnyHandle,
        recv: Recv,
        name: Option<IdentId>,
    },
    /// a single name: a lone path segment, a struct-literal label, a
    /// type-slot segment
    Name {
        id: IdentId,
        span: Span,
        /// the slot's position flag — Act 4's completion edges key on
        /// it (the qualified/bare shapes); the hover lanes key on the
        /// name either way
        #[allow(dead_code)]
        is_ty: bool,
    },
    Selfy {
        /// the type spelling flag — the cursor carries what the tree
        /// records; hover re-derives the node kind (the two `Self`
        /// spellings share the flag, their answers differ)
        #[allow(dead_code)]
        is_ty: bool,
        span: Span,
    },
    /// a keyword, an operator, whitespace, an error fence
    Nothing,
}

/// the classification of the node the caret rides — `w` is the
/// session's world, `file` the queried file. The boundary convention:
/// the caret on the byte where a name's NEXT character would sit
/// still reads that name — whatever sits there (`(`, `;`, whitespace,
/// or the DOT that ends a receiver: `p.|bump()` answers the receiver,
/// the single-segment Name arm carrying no position check of its
/// own). The probe: the byte under the caret first; when THAT byte's
/// node names nothing (the byte sits in the statement's own span — a
/// name's end byte rides the stmt the name ends), the node the byte
/// BEFORE belongs to, the caret unchanged — the same convention, one
/// node deeper.
pub(crate) fn cursor(w: &World, file: FileId, pos: u32) -> Cursor {
    let d = w.file(file);
    let ast = d.doc.ast();
    let c = classify(w, file, ast, ast.node_at(pos), pos);
    if !matches!(c, Cursor::Nothing) {
        return c;
    }
    let back = pos.checked_sub(1).and_then(|p| ast.node_at(p));
    classify(w, file, ast, back, pos)
}

/// one probe node's classification at the caret `pos` — [`cursor`]'s
/// body (the node and the caret are independent: the probe may walk
/// back while the caret stays)
fn classify(w: &World, file: FileId, ast: &rut_ast::Ast, h: Option<AnyHandle>, pos: u32) -> Cursor {
    // 1. the caret's node. The fence residue names nothing.
    let Some(h) = h else {
        return Cursor::Nothing;
    };
    if h.is::<rut_ast::ErrorNode>() {
        return Cursor::Nothing;
    }
    // 2. the Self spellings — the type slot and the receiver/node
    if let Some(t) = h.downcast::<SelfTyNode>() {
        return Cursor::Selfy {
            is_ty: true,
            span: t.get(ast).span,
        };
    }
    if let Some(s) = h.downcast::<SelfNode>() {
        let sd = s.get(ast);
        return Cursor::Selfy {
            is_ty: sd.is_ty,
            span: sd.span,
        };
    }
    // 3. a declaring name under the caret — node decls and binding rows
    if let Some(c) = decl_at(w, file, ast, h, pos) {
        return c;
    }
    // 4. a use edge's own spans — the path segments, then the leaves
    if let Some(uh) = h.downcast::<UseNode>() {
        let ud = uh.get(ast);
        let segs: Vec<String> = ud
            .path
            .iter()
            .map(|n| ast.name(n.ident).to_string())
            .collect();
        for (k, seg) in ud.path.iter().enumerate() {
            if on_name(seg.span, pos) {
                return Cursor::Use {
                    node: h,
                    segs,
                    at: if k == 0 {
                        UseAt::Head
                    } else {
                        UseAt::Segment(k)
                    },
                };
            }
        }
        for (i, n) in ud.names.iter().enumerate() {
            if on_name(n.span, pos) {
                return Cursor::Use {
                    node: h,
                    segs,
                    at: UseAt::Leaf(i),
                };
            }
        }
        // the cursor rides the keyword or a brace — no name position
        return Cursor::Nothing;
    }
    // 5. a member node at its member position — a METHOD CALL or a
    //    FIELD ACCESS (a `::` path's tail is the qualified lane's
    //    member position — step 6 keeps it a Path; the walk's path arm
    //    keys the member law on the path's own receiver). The typing
    //    edge carries no member yet. The derivation is [`member_pos`]'s
    //    one body — the path shape is read by the member door, never
    //    here.
    if h.is::<rut_ast::MethodNode>() || h.is::<rut_ast::FieldNode>() {
        if let Some((member, recv)) = member_pos(ast, h, pos) {
            return Cursor::Member {
                node: h,
                recv,
                name: member,
            };
        }
    }
    // 6. a multi-segment path — value and type slots alike
    if let Some((segs_data, is_ty)) = path_segs(ast, h) {
        if segs_data.len() >= 2 {
            if let Some(k) = segs_data.iter().position(|(_, sp)| on_name(*sp, pos)) {
                return Cursor::Path {
                    node: h,
                    segs: segs_data
                        .iter()
                        .map(|(n, _)| ast.name(*n).to_string())
                        .collect(),
                    at: k,
                    is_ty,
                };
            }
            // between segments (`::`) — no name position
            return Cursor::Nothing;
        }
        // a single segment: the name position (step 7)
        let (id, sp) = segs_data[0];
        return Cursor::Name {
            id,
            span: sp,
            is_ty,
        };
    }
    // 7. a struct-literal field label — the FieldInit rides the
    //    literal as data (no node of its own)
    if let Some(sl) = h.downcast::<StructLitNode>() {
        for fi in &sl.get(ast).fields {
            if on_name(fi.name.span, pos) {
                return Cursor::Name {
                    id: fi.name.ident,
                    span: fi.name.span,
                    is_ty: false,
                };
            }
        }
        return Cursor::Nothing;
    }
    // 8. a keyword, an operator, whitespace
    Cursor::Nothing
}

/// step 3 — the declaring name the cursor rides: a node decl wraps as
/// [`Def::Node`], a binding row's decl ident as [`Def::Binding`].
/// `None` = the node is no decl (or the cursor is off its name).
fn decl_at(w: &World, file: FileId, ast: &rut_ast::Ast, h: AnyHandle, pos: u32) -> Option<Cursor> {
    // the node decls — the type decls, aliases, fns, methods, fields,
    // enum members, module lets, and the `mod` edges
    if let Some((_, sp)) = decl_ident(ast, h) {
        if on_name(sp, pos) {
            return Some(Cursor::Decl(Def::Node { file, node: h }));
        }
        return None;
    }
    // the binding layer's decls: a param/let/loop-var whose NAME the
    // cursor rides is its own declaration (params sit before their
    // scope, so the scope-containment resolve can't see them)
    let binding = |sp: Span| -> Option<Cursor> {
        if !on_name(sp, pos) {
            return None;
        }
        // the row whose decl ident the caret rides — the bindings pass
        // derives one per declared name, from this same tree
        let row = w
            .file(file)
            .doc
            .bindings()
            .iter()
            .position(|b| b.decl_ident_span == sp)?;
        Some(Cursor::Decl(Def::Binding { file, row }))
    };
    if let Some(p) = h.downcast::<ParamNode>() {
        return binding(p.get(ast).name.span);
    }
    if let Some(l) = h.downcast::<LetStmtNode>() {
        let dd = l.get(ast);
        if let Some(c) = binding(dd.name.span) {
            return Some(c);
        }
        if let Some(ns) = &dd.destructure {
            for n in ns {
                if let Some(c) = binding(n.span) {
                    return Some(c);
                }
            }
        }
        return None;
    }
    if let Some(f) = h.downcast::<ForOfNode>() {
        return binding(f.get(ast).var.span);
    }
    if let Some(f) = h.downcast::<ForCNode>() {
        return binding(f.get(ast).var.span);
    }
    None
}

/// a path node's segments as (ident, span) pairs beside the
/// type-position flag — a [`PathNode`] (value) or a [`TyPathNode`]
/// (type slot). `None` = the node is no path.
fn path_segs(ast: &rut_ast::Ast, h: AnyHandle) -> Option<(Vec<(IdentId, Span)>, bool)> {
    if let Some(p) = h.downcast::<PathNode>() {
        let d = p.get(ast);
        return Some((
            d.segs.iter().map(|s| (s.name.ident, s.name.span)).collect(),
            false,
        ));
    }
    if let Some(t) = h.downcast::<TyPathNode>() {
        let d = t.get(ast);
        return Some((
            d.segs.iter().map(|s| (s.name.ident, s.name.span)).collect(),
            true,
        ));
    }
    None
}

/// the (ident, span) of the path segment the cursor rides
pub(crate) fn path_seg(ast: &rut_ast::Ast, h: AnyHandle, at: usize) -> (IdentId, Span) {
    let (segs, _) = path_segs(ast, h).expect("a path cursor rides a path node");
    segs[at]
}

/// the (member, receiver) pair a member-position node carries — a
/// method call's member (the typing edge rides its absent member), a
/// field access's name, or a `::` path's tail segment (the tail IS
/// the member lane — the walk's path arm keys the door on it). One
/// body for every reader: the classification above (its member step
/// reads the method/field shapes only), the member door, the member
/// tier. `None` when the node is no member position or the cursor is
/// not on its member segment (a path's head is a name position).
pub(crate) fn member_pos(
    ast: &rut_ast::Ast,
    h: AnyHandle,
    pos: u32,
) -> Option<(Option<IdentId>, Recv)> {
    if let Some(m) = h.downcast::<rut_ast::MethodNode>() {
        let d = m.get(ast);
        // the cursor rides the member name (the token-scan's
        // boundary convention: a name ends its own position) — or
        // the shape is the TYPING EDGE (absent member, `recv.`):
        // a member position with nothing resolved yet
        return match d.member {
            None => Some((None, Recv::Expr(d.recv))),
            Some(n) => {
                let on_member = n.span.lo <= pos && pos <= n.span.hi;
                on_member.then_some((Some(n.ident), Recv::Expr(d.recv)))
            }
        };
    }
    if let Some(f) = h.downcast::<rut_ast::FieldNode>() {
        let d = f.get(ast);
        let on_member = d.name.span.lo <= pos && pos <= d.name.span.hi;
        return if on_member {
            Some((Some(d.name.ident), Recv::Expr(d.recv)))
        } else {
            None
        };
    }
    if let Some(p) = h.downcast::<PathNode>() {
        let segs = &p.get(ast).segs;
        if segs.len() >= 2 {
            if let Some(i) = segs
                .iter()
                .position(|s| s.name.span.lo <= pos && pos <= s.name.span.hi)
            {
                if i + 1 == segs.len() {
                    // the tail segment IS the member lane
                    return Some((Some(segs[i].name.ident), Recv::Seg(segs[i - 1].name.ident)));
                }
            }
        }
    }
    None
}

/// the span the cursor's lane decorates: the declaring name, the
/// ridden use segment or leaf, the path segment, the member token,
/// the name position, the Self spelling
pub(crate) fn cursor_span(w: &World, file: FileId, c: &Cursor) -> Span {
    let ast = w.file(file).doc.ast();
    match c {
        Cursor::Decl(def) => def.name_span(w),
        Cursor::Use { node, at, .. } => {
            let u = node.downcast::<UseNode>().expect("a use cursor");
            let ud = u.get(ast);
            match at {
                UseAt::Head => ud.path[0].span,
                UseAt::Segment(k) => ud.path[*k].span,
                UseAt::Leaf(i) => ud.names[*i].span,
            }
        }
        Cursor::Path { node, at, .. } => path_seg(ast, *node, *at).1,
        Cursor::Member { node, .. } => member_span(ast, *node),
        Cursor::Name { span, .. } => *span,
        Cursor::Selfy { span, .. } => *span,
        Cursor::Nothing => Span::new(0, 0),
    }
}

/// the member token's span on a member-position node — the call's
/// member name, the field access's name, or a path's tail segment
pub(crate) fn member_span(ast: &rut_ast::Ast, h: AnyHandle) -> Span {
    if let Some(m) = h.downcast::<rut_ast::MethodNode>() {
        let d = m.get(ast);
        if let Some(n) = d.member {
            return n.span;
        }
    }
    if let Some(f) = h.downcast::<rut_ast::FieldNode>() {
        return f.get(ast).name.span;
    }
    if let Some(p) = h.downcast::<PathNode>() {
        let segs = &p.get(ast).segs;
        if let Some(last) = segs.last() {
            return last.name.span;
        }
    }
    ast.span_of(h)
}
