//! Completions — member completion after `recv.`, bare-position
//! completion, the type tier at annotation boundaries, the use-path
//! tiers, the `::` tiers (module members by
//! tier, then the type-head lists: enum members, namespace statics, a
//! type's associated fns), and the auto-import tier, over the
//! same definition index hover uses. One rule for
//! the member list (mirrors the compiler's): **members of `T` = T's own
//! surface ∪ inherent `impl T { .. }` methods; on an interface `I` they
//! are exactly `I`'s declared signatures, use-gated when `I` is
//! foreign**. Heuristic, like hover: a
//! miss is an empty list, never wrong text.
//!
//! The use-path tiers: at `use <cursor>` the chain's module names
//! compete (every named index — a walked dep, the prelude — no index
//! is special); at `use mod::<cursor>` that module's exported public
//! names. The bare position rides the compiler's bare-name law: the
//! fn-local bindings ahead of the keyword table, then the document's
//! own decls + the chain's ambient builtin rows + the names the
//! document already `use`s — every other foreign name rides only the
//! auto-import tier (with the `use mod::Name;` insert, sorted after
//! the local items). Annotation boundaries (`let x: ⏐`, `-> T⏐`) get
//! the type tier: primitives + document/imported types, never
//! keywords, fns, or locals.

use std::collections::HashSet;

use ls_types::Range;
use rut_ast::{
    AnyHandle, Ast, ExprRef, FieldNode, IdentId, ItemRef, MethodNode, PathNode, Span, TyRef,
    UseNode,
};

use crate::hover::collection::{Chain, FnView, LetView, MemberView, TyView};
use crate::hover::types::TyForm;
use crate::mods::{self, DocMods};
use crate::semantic::LineIndex;

/// One completion item — plain data; the server maps it to LSP types.
#[derive(Debug, Clone)]
pub struct CompletionOut {
    pub label: String,
    /// declaration text shown next to the label (a signature, a field)
    pub detail: String,
    /// markdown documentation lines (may be empty)
    pub doc: Vec<String>,
    pub kind: CompletionKind,
    /// LSP `sortText` — the auto-import tier sorts after every local
    /// item (`~` prefix); `None` sorts by label as before
    pub sort_text: Option<String>,
    /// LSP `additionalTextEdits` — the auto-import insert
    /// (`use mod::Name;`), ranges over the normalized source (the same
    /// coordinate pipeline every other range rides)
    pub additional_text_edits: Vec<(Range, String)>,
}

impl CompletionOut {
    /// a plain item — no sort text, no edits (the keyword/local shape)
    fn plain(label: String, detail: String, doc: Vec<String>, kind: CompletionKind) -> Self {
        CompletionOut {
            label,
            detail,
            doc,
            kind,
            sort_text: None,
            additional_text_edits: Vec::new(),
        }
    }
    /// THE conversion — a tier row into the wire item. Every tier
    /// assembles rows; this is the one place a label allocates.
    fn of(row: &Row) -> Self {
        CompletionOut::plain(row.name().to_string(), row.detail(), row.docs(), row.kind())
    }
}

/// The wire's question set, asked of every tier's row — the name and
/// kind borrowed from the view's own node, the detail/docs rendered
/// verbatim from the spans. The tiers produce ROWS (views over the
/// decls they found); the ONLY conversion to the owned wire item is
/// [`CompletionOut::of`], at the ABI edge — one vocabulary, one
/// allocation point.
pub(crate) trait CompletionRow {
    fn name(&self) -> &str;
    fn detail(&self) -> String;
    fn docs(&self) -> Vec<String>;
    fn kind(&self) -> CompletionKind;
}

/// A binding row — the rows carry interner ids, the wire spells them;
/// the detail rides the doc's own text join
pub(crate) struct BindRow<'a> {
    pub b: &'a crate::hover::Binding,
    pub doc: &'a crate::hover::collection::MountedFile,
    /// the resolver's declared-or-inferred answer (the tier's one
    /// resolver read per row)
    pub ty: Option<String>,
}

/// One tier row — the view the tier door found it from. The variants
/// are the tier views themselves; the wire item is one
/// [`CompletionOut::of`] away.
pub(crate) enum Row<'a> {
    Ty(TyView<'a>),
    Fn(FnView<'a>),
    Member(MemberView<'a>),
    Let(LetView<'a>),
    Bind(BindRow<'a>),
    Kw(&'static str),
}

impl CompletionRow for Row<'_> {
    fn name(&self) -> &str {
        match self {
            Row::Ty(t) => t.name(),
            Row::Fn(f) => f.name(),
            Row::Member(m) => m.name(),
            Row::Let(l) => l.name(),
            Row::Bind(r) => r.doc.doc.ast().name(r.b.name),
            Row::Kw(k) => k,
        }
    }
    fn detail(&self) -> String {
        match self {
            Row::Ty(t) => t.head(),
            Row::Fn(f) => f.src(),
            Row::Member(m) => m.src(),
            Row::Let(l) => l.src(),
            Row::Bind(r) => match r.ty.clone() {
                Some(ty) => format!("{} — {ty}", r.b.kind.label()),
                None => r.b.kind.label().to_string(),
            },
            Row::Kw(_) => String::new(),
        }
    }
    fn docs(&self) -> Vec<String> {
        match self {
            Row::Ty(t) => t.doc_lines(),
            Row::Fn(f) => f.doc_lines(),
            Row::Member(m) => m.doc_lines(),
            Row::Let(l) => l.doc_lines(),
            Row::Bind(_) => Vec::new(),
            Row::Kw(_) => Vec::new(),
        }
    }
    fn kind(&self) -> CompletionKind {
        match self {
            Row::Ty(t) => match t.form() {
                TyForm::Interface => CompletionKind::Interface,
                _ => CompletionKind::Type,
            },
            Row::Fn(f) => {
                if f.owner().is_some() {
                    CompletionKind::Method
                } else {
                    CompletionKind::Fn
                }
            }
            Row::Member(m) => {
                if m.h.is::<rut_ast::MethodDeclNode>() {
                    CompletionKind::Method
                } else {
                    CompletionKind::Field
                }
            }
            Row::Let(_) => CompletionKind::Const,
            Row::Bind(r) => match r.b.kind {
                crate::hover::BindKind::Param => CompletionKind::Parameter,
                crate::hover::BindKind::Let | crate::hover::BindKind::IterVar => {
                    CompletionKind::Variable
                }
            },
            Row::Kw(_) => CompletionKind::Keyword,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Module,
    Type,
    Interface,
    Const,
    Fn,
    Method,
    Field,
    EnumMember,
    /// a fn-local `let` binding (or loop variable) at a bare position
    Variable,
    /// a fn-local parameter at a bare position
    Parameter,
}

/// The typing edge, from the tree: the byte before the cursor names
/// the context — a `use` ancestor (the use tiers), a member node (the
/// incomplete `recv.` shape included: the member is being typed), a
/// `::` path's tail segment (the qualified tier), a type slot at an
/// annotation boundary (the type tier), else the bare tiers with the
/// partial ident (the fence residue riding `Doc::name_at`). No token
/// scans.
enum Edge {
    /// inside a `use` statement: the use NODE (the tier door's key),
    /// the spelled path + the typed prefix
    Use {
        node: AnyHandle,
        segs: Vec<String>,
        prefix: String,
    },
    /// a `::` run's tail: the head segments + the typed prefix
    Qualified { segs: Vec<String>, prefix: String },
    /// `recv.` / `recv.na`: the member NODE (the tier door's key) +
    /// the receiver expression (the mod-head attempt's key) + the
    /// typed prefix (the incomplete method shape's absent member is
    /// the empty prefix)
    Member {
        node: AnyHandle,
        recv: ExprRef,
        prefix: String,
    },
    /// the annotation boundary — a type position (`let x: ⏐`,
    /// `fn f() -> T⏐`)
    Annotation { prefix: String },
    /// the bare tiers, with the partial ident if one is mid-typed
    Bare { prefix: String },
    /// the type slot's `.`/`::` member request (`let c: layout.Co⏐`):
    /// the head segments for the mod tier, the tail ident for the
    /// member tier — no bare fallthrough (a miss is an empty list)
    TypeDot {
        head: Vec<String>,
        tail: IdentId,
        prefix: String,
    },
}

/// Completions at `pos`: use-path tiers inside a `use` statement
/// (walking a package's mod namespaces), qualified-position tiers
/// after `modname::` in the package's own files, member items after
/// `recv.` (filtered by the typed prefix), the type tier at an
/// annotation boundary, otherwise the visible fn-local bindings ahead
/// of the keyword table plus the visible decls under the bare-name law
/// plus the auto-import tier, deduped by label. `src` is the NORMALIZED
/// source — the auto-import edits' coordinates ride the same text the
/// spans index. `doc` is the open document's file-module context (the
/// face's knowledge; the default keeps position-path completion off).
/// `direct_deps` is the open document's own dep set (its manifest's
/// rows) — the auto-import tier derives its gate from it live.
pub fn complete(
    chain: &Chain,
    pos: u32,
    src: &str,
    doc: &DocMods,
    direct_deps: &HashSet<String>,
) -> Vec<CompletionOut> {
    let head = chain.head();
    let ast = head.doc.ast();
    match typing_edge(chain, pos, src) {
        Edge::Use { node, segs, prefix } => {
            // the OWN lane first — a head that speaks for the doc's own
            // package (its own name, or an own mod edge) rides the
            // position-path machinery: the intra-package walk and the
            // visible-members gate, exactly what the compiler's own-use
            // check accepts (`use imp::⏐` offers the module's visible
            // members). A foreign head keeps the cross-package tier.
            if let Some(out) = position_path_completions(chain, doc, &segs) {
                return filtered(out, &prefix);
            }
            use_tier_out(chain, chain.use_exports(node, pos), &prefix)
        }
        // a lone `:` (or the `->` arrow) under the cursor is the
        // annotation boundary — the type tier answers it: the primitive
        // table, the document's types, and the imported foreign types
        // (no keywords, fns, or locals — values never ride a type
        // position). `Vec<⏐` stays ambiguous (`<` vs the comparison)
        // and is not this tier.
        Edge::Annotation { prefix } => filtered(type_tier_completions(chain), &prefix),
        Edge::Qualified { segs, prefix } => {
            if let Some(out) = position_path_completions(chain, doc, &segs) {
                return filtered(out, &prefix);
            }
            // the head did not resolve to a module — the `::` law's
            // other member lists ride the same trigger: enum members,
            // namespace statics, a type head's associated fns
            if let Some(out) = type_head_completions(chain, &segs) {
                return filtered(out, &prefix);
            }
            // the head resolved to nothing known — `::` in an unknown
            // context falls through to the tiers below, the typed
            // prefix riding along (the bare tier does not filter; the
            // auto-import tier does)
            let mut out = local_completions(chain, pos);
            out.extend(bare_tier_out(chain));
            out.extend(auto_import_completions(
                chain,
                ast,
                src,
                &prefix,
                direct_deps,
            ));
            let mut seen: HashSet<String> = HashSet::new();
            out.retain(|c| seen.insert(c.label.clone()));
            out
        }
        Edge::TypeDot { head, tail, prefix } => {
            if let Some(out) = position_path_completions(chain, doc, &head) {
                return filtered(out, &prefix);
            }
            member_tier_out(
                chain
                    .members_for(pos, crate::hover::collection::RecvNode::Seg(tail))
                    .unwrap_or_default(),
                &prefix,
            )
        }
        Edge::Member { node, recv, prefix } => {
            // the qualified-position spelling: positions qualify with
            // `::` — `helpers::mk(`, `let c: layout::Column` — so a
            // receiver that resolves as a MOD HEAD offers the module's
            // members (the same tier the `::` form rides). A resolved
            // head is final; an unknown one falls through to the
            // member tier
            if let ExprRef::Path(ph) = recv {
                let segs = &ph.get(ast).segs;
                if segs.len() == 1 {
                    let name = ast.name(segs[0].name).to_string();
                    if let Some(out) =
                        position_path_completions(chain, doc, std::slice::from_ref(&name))
                    {
                        return filtered(out, &prefix);
                    }
                }
            }
            // member completion shares hover's receiver inference (the
            // door types the receiver INSIDE — field reads, chained
            // calls, loop variables)
            member_tier_out(chain.members(pos, node).unwrap_or_default(), &prefix)
        }
        Edge::Bare { prefix } => {
            // the dot guard: the request sits after a `.` the tree
            // could not shape into a member node (a fence swallowed the
            // receiver). That is a MEMBER request with an unresolved
            // receiver: a miss is an empty list, never the bare
            // keyword/host-fn dump. Text-backed, so a stale sync (the
            // tail still short the dot) cannot route around it.
            if dot_context_before(src, pos) {
                return Vec::new();
            }
            // the locals tier rides AHEAD of the keyword table — the
            // name being typed is most often a binding just declared
            let mut out = local_completions(chain, pos);
            out.extend(bare_tier_out(chain));
            out.extend(auto_import_completions(
                chain,
                ast,
                src,
                &prefix,
                direct_deps,
            ));
            // one final retain-by-label over the merged list (A4): the
            // extend order IS the precedence — locals > keywords >
            // document decls > imported > auto-import — so the first
            // occurrence of a label wins and a name reachable two ways
            // (e.g. bare via a `use`, and again via the auto-import
            // tier) offers exactly one item.
            let mut seen: HashSet<String> = HashSet::new();
            out.retain(|c| seen.insert(c.label.clone()));
            out
        }
    }
}

/// the typing edge — the walk: the byte before the cursor lands the
/// probe, and the probe's own node kind (or its ancestors') names the
/// tier. A use node carries its own spans; a partial member is the
/// tree's `member: None` shape; the annotation boundary reads the one
/// punct the tree does not record (the `:`/`->` between the binding
/// head and the cursor — a text probe, the dot guard's precedent).
fn typing_edge(chain: &Chain, pos: u32, src: &str) -> Edge {
    let head = chain.head();
    let ast = head.doc.ast();
    // the byte before the cursor names the context. A STRUCTURAL
    // wrapper (the block, the fn, the root) under the cursor means the
    // byte sits in dead space — the walk looks a little further back
    // for the node the cursor is actually typing into (the blank after
    // a colon, the gap at EOF); the first non-wrapper stands.
    const CONTEXT_KINDS: [&str; 10] = [
        "LetStmt",
        "Fn",
        "Use",
        "Param",
        "FieldDecl",
        "Method",
        "Field",
        "Path",
        "TyPath",
        "Call",
    ];
    let mut p0 = None;
    for back in 1..=16u32 {
        match pos.checked_sub(back).and_then(|p| ast.node_at(p)) {
            Some(h) if matches!(h.kind_name(), "Module" | "Block") => continue,
            Some(h) if CONTEXT_KINDS.contains(&h.kind_name()) => {
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
            .member()
            .map(|(_, nsp)| partial_text(head, nsp, pos))
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
            prefix: partial_text(head, d.name_span, pos),
        };
    }
    // the qualified lane — a `::` path whose TAIL segment the cursor
    // rides (a single-seg path is a bare position; the head segments
    // are the walk)
    if let Some(p) = p0.downcast::<PathNode>() {
        let segs = &p.get(ast).segs;
        if segs.len() >= 2 {
            if let Some(i) = segs.iter().position(|s| on_name(s.name_span, pos)) {
                if i + 1 == segs.len() {
                    return Edge::Qualified {
                        segs: segs[..i]
                            .iter()
                            .map(|s| ast.name(s.name).to_string())
                            .collect(),
                        prefix: partial_text(head, segs[i].name_span, pos),
                    };
                }
            }
        }
        // the cursor strictly past the last segment — the dangling
        // `::` (the head's members tier): every seg is the walk, the
        // prefix empty. (A cursor ON the name's end is the bare
        // typing edge, not a `::` request.)
        if let Some(last) = segs.last() {
            if last.name_span.hi < pos {
                return Edge::Qualified {
                    segs: segs.iter().map(|s| ast.name(s.name).to_string()).collect(),
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
            } else if gap_spells(head, d.name_span.hi, pos, ':') {
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
                (d.name_span, d.ty)
            }),
            a.downcast::<rut_ast::FieldDeclNode>()
                .map(|h| (h.get(ast).name_span, Some(h.get(ast).ty))),
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
                    .unwrap_or(d.name_span.hi)
            } else {
                let d = fh.downcast::<rut_ast::SurfaceFnNode>()?.get(ast);
                d.params
                    .last()
                    .map(|p| p.span(ast).hi)
                    .unwrap_or(d.name_span.hi)
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

/// the name-position convention: a name ends its own position
fn on_name(sp: Span, pos: u32) -> bool {
    sp.lo <= pos && pos <= sp.hi
}

/// the typed part of `sp` — the slice up to the cursor (a cursor past
/// the span's end typed the whole name and moved on: the prefix is the
/// whole name, the tier filters on it)
fn partial_text(head: &crate::hover::collection::MountedFile, sp: Span, pos: u32) -> String {
    let end = (pos as usize).clamp(sp.lo as usize, sp.hi as usize);
    head.doc.text(Span::new(sp.lo, end as u32)).to_string()
}

/// the use tier's context, off the node's own recorded spans: a path
/// segment under the cursor walks the path BEFORE it with the partial
/// seg as the prefix; a leaf name (brace list or flat form) walks the
/// FULL path with the partial name as the prefix; past a separator
/// with nothing typed, the full path and an empty prefix.
fn use_edge(
    head: &crate::hover::collection::MountedFile,
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
        .path_spans
        .iter()
        .position(|sp| sp.lo <= pos && pos <= sp.hi);
    let before: Vec<String> = match ridden {
        Some(k) => u.path[..k]
            .iter()
            .map(|id| ast.name(*id).to_string())
            .collect(),
        None => u
            .path
            .iter()
            .zip(&u.path_spans)
            .filter(|(_, sp)| sp.hi <= pos)
            .map(|(id, _)| ast.name(*id).to_string())
            .collect(),
    };
    for sp in &u.path_spans {
        if on_name(*sp, pos) {
            return Edge::Use {
                node,
                segs: before,
                prefix: partial_text(head, *sp, pos),
            };
        }
    }
    for sp in &u.name_spans {
        if on_name(*sp, pos) {
            return Edge::Use {
                node,
                segs: before,
                prefix: partial_text(head, *sp, pos),
            };
        }
    }
    Edge::Use {
        node,
        segs: before,
        prefix: String::new(),
    }
}

/// the type slot's tail prefix — the cursor rides the slot's LAST
/// segment (a single-seg partial is the annotation tier's prefix; a
/// multi-seg path's tail is the qualified tier's, which the walk
/// answered before this lane)
fn ty_slot_prefix(
    head: &crate::hover::collection::MountedFile,
    t: TyRef,
    pos: u32,
) -> Option<String> {
    let TyRef::Path(h) = t else {
        return None;
    };
    let ast = head.doc.ast();
    let segs = &h.get(ast).segs;
    if segs.len() != 1 {
        return None; // multi-seg: the qualified lane answers
    }
    let sp = segs[0].name_span;
    on_name(sp, pos).then(|| partial_text(head, sp, pos))
}

/// the type slot's `.`-qualified spelling — positions qualify with `.`
/// (`let c: layout.Co⏐`, the editor's real spelling): the cursor past
/// the slot's tail segment with a `.` plus at most a partial ident
/// between — the same members tier the `::` form rides, headed by the
/// slot's own segments (text-backed, the dot guard's precedent: the
/// TYPE grammar spells no `.` access, so the punct lives in the
/// statement's recovery region)
fn ty_slot_dot(head: &crate::hover::collection::MountedFile, t: TyRef, pos: u32) -> Option<Edge> {
    _ty_slot_dot_impl(head, t, pos)
}

fn _ty_slot_dot_impl(
    head: &crate::hover::collection::MountedFile,
    t: TyRef,
    pos: u32,
) -> Option<Edge> {
    let TyRef::Path(h) = t else {
        return None;
    };
    let ast = head.doc.ast();
    let segs = &h.get(ast).segs;
    let last = segs.last()?;
    if last.name_span.hi >= pos {
        return None; // the cursor rides the slot itself — not a `.` run
    }
    let end = (pos as usize).min(head.doc.src().len());
    let gap = head.doc.src()[last.name_span.hi as usize..end].trim_end();
    // the `::` spelling rides the same tier (the plan's letter)
    let after = if let Some(stripped) = gap.strip_suffix("::") {
        if !stripped.is_empty() {
            return None;
        }
        ""
    } else {
        let dot = gap.rfind('.')?;
        let after = &gap[dot + 1..];
        if !after.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        after
    };
    Some(Edge::TypeDot {
        head: segs.iter().map(|s| ast.name(s.name).to_string()).collect(),
        tail: last.name,
        prefix: after.to_string(),
    })
}

/// does the text between `from` and `pos` spell the boundary punct
/// before any `=`? (the annotation probe — the tree records the
/// slots, not the punct that introduced them)
fn gap_spells(
    head: &crate::hover::collection::MountedFile,
    from: u32,
    pos: u32,
    punct: char,
) -> bool {
    if pos <= from {
        return false;
    }
    let end = (pos as usize).min(head.doc.src().len());
    let lo = (from as usize).min(end);
    let gap = head.doc.src()[lo..end].trim_end();
    // the `=` ends the binding head — a gap that crossed it is the
    // initializer's side, never an annotation boundary
    if gap.contains('=') {
        return false;
    }
    gap.rfind(punct).is_some()
}

/// the bare tiers' prefix: the partial ident the cursor sits in
/// (`Ve⏐` → `Ve`), from the tree — a path segment, a let/param name —
/// else the fence residue (`Doc::name_at`)
fn bare_prefix_walk(
    head: &crate::hover::collection::MountedFile,
    p0: rut_ast::AnyHandle,
    pos: u32,
    src: &str,
) -> String {
    let ast = head.doc.ast();
    if let Some(p) = p0.downcast::<PathNode>() {
        for s in &p.get(ast).segs {
            if on_name(s.name_span, pos) {
                return partial_text(head, s.name_span, pos);
            }
        }
    }
    if let Some(l) = p0.downcast::<rut_ast::LetStmtNode>() {
        let d = l.get(ast);
        if on_name(d.name_span, pos) {
            return partial_text(head, d.name_span, pos);
        }
        if let Some(ns) = &d.destructure {
            for (_, sp) in ns {
                if on_name(*sp, pos) {
                    return partial_text(head, *sp, pos);
                }
            }
        }
    }
    if let Some(p) = p0.downcast::<rut_ast::ParamNode>() {
        let d = p.get(ast);
        if on_name(d.name_span, pos) {
            return partial_text(head, d.name_span, pos);
        }
    }
    // the fence residue: the word under the byte before the cursor
    if p0.is::<rut_ast::ErrorNode>() {
        if let Some((sp, _)) = head.doc.name_at(pos.saturating_sub(1)) {
            let end = (pos as usize).clamp(sp.lo as usize, sp.hi as usize);
            return src[sp.lo as usize..end].to_string();
        }
    }
    String::new()
}

/// the dot context, text-backed: the source before `pos` ends with a
/// `.` — right against the cursor (`recv.⏐`) or under one trailing
/// identifier run (`recv.na⏐`). The tree walk's backstop: when the
/// tree carries no member node before the cursor (a fence swallowed
/// the receiver), this still tells a member request from a bare one.
/// `pos` past the source (a stale snapshot's tail) clamps to the end.
fn dot_context_before(src: &str, pos: u32) -> bool {
    let end = (pos as usize).min(src.len());
    let before = src[..end].trim_end();
    if before.ends_with('.') {
        return true;
    }
    let stripped = before.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
    before.len() > stripped.len() && stripped.ends_with('.')
}
/// The member tier — the enumeration and the gates live in the
/// collection door (`Chain::members`, keyed on the walk's node); this
/// maps its view rows to the wire items and applies the typed prefix
/// (presentation, at the call site).
fn member_tier_out(
    rows: Vec<crate::hover::collection::MemberRow>,
    prefix: &str,
) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for r in rows {
        match r {
            crate::hover::collection::MemberRow::Field(m)
            | crate::hover::collection::MemberRow::Method(m) => {
                push_item(&mut out, CompletionOut::of(&Row::Member(m)));
            }
            crate::hover::collection::MemberRow::ImplFn(f) => {
                push_item(&mut out, CompletionOut::of(&Row::Fn(f)));
            }
        }
    }
    filtered(out, prefix)
}

/// The locals tier at a bare position: every fn-local binding visible at
/// `pos`, under the same shadow-correct, scope-exit-correct rule the
/// resolve rides — per NAME the innermost in-scope declaration wins (an
/// inner re-decl replaces the outer inside its scope; past the block the
/// name is gone), and a binding whose own declaring ident holds the
/// cursor stays off (don't offer the name mid-typing at its own
/// declaration). Emitted ahead of the keyword table — the name being
/// typed is most often a binding just declared. The doc's own pass is
/// read internally (`Doc::bindings()` — lazy per version).
fn local_completions(chain: &Chain, pos: u32) -> Vec<CompletionOut> {
    let doc = chain.head();
    let mut best: Vec<&crate::hover::Binding> = Vec::new();
    for b in doc.doc.bindings() {
        if b.decl_ident_span.lo > pos || !(b.scope_span.lo <= pos && pos < b.scope_span.hi) {
            continue; // declared later, or its scope ended before the cursor
        }
        if b.decl_ident_span.lo <= pos && pos <= b.decl_ident_span.hi {
            continue; // the cursor sits in the declaration's own ident
        }
        match best.iter().position(|x| x.name == b.name) {
            Some(i) => {
                if best[i].decl_ident_span.lo < b.decl_ident_span.lo {
                    best[i] = b; // the inner shadow wins inside its scope
                }
            }
            None => best.push(b),
        }
    }
    let mut out: Vec<CompletionOut> = best
        .into_iter()
        .map(|b| {
            let row = Row::Bind(BindRow {
                b,
                doc,
                ty: chain.binding_ty(b),
            });
            CompletionOut::of(&row)
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// The bare tier — the keyword table (the grammar's reserved set, one
/// canonical list) ahead of the collection door's visible decls
/// (`Chain::bare`: the bare-name gates — own, ambient, used-from —
/// live inside). The label filter is the call site's.
fn bare_tier_out(chain: &Chain) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for kw in crate::semantic::RESERVED_KW {
        out.push(CompletionOut::of(&Row::Kw(kw)));
    }
    let bare = chain.bare();
    for t in bare.types {
        push_item(&mut out, CompletionOut::of(&Row::Ty(t)));
    }
    for f in bare.fns {
        push_item(&mut out, CompletionOut::of(&Row::Fn(f)));
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

// ---- the use-path tiers ----

/// The use tier — the enumeration, the walk and the pub gates live in
/// the collection door (`Chain::use_exports`, keyed on the walk's use
/// node); this maps its view rows to the wire items and applies the
/// typed prefix (presentation, at the call site).
fn use_tier_out(
    chain: &Chain,
    ux: Option<crate::hover::collection::UseExports>,
    prefix: &str,
) -> Vec<CompletionOut> {
    let Some(ux) = ux else {
        return Vec::new();
    };
    let mut out: Vec<CompletionOut> = Vec::new();
    match ux {
        crate::hover::collection::UseExports::Modules(list) => {
            for (m, origin) in list {
                push_item(
                    &mut out,
                    CompletionOut::plain(m, origin, Vec::new(), CompletionKind::Module),
                );
            }
        }
        crate::hover::collection::UseExports::Exports { pkg, path, rows } => {
            for r in rows {
                match r {
                    // the walkable children — the detail renders the
                    // mounted display from the child's mod path
                    crate::hover::collection::ExportRow::Mod(m, child) => push_item(
                        &mut out,
                        CompletionOut::plain(
                            m.name().to_string(),
                            format!(
                                "file module — {}",
                                mods::display_of(chain, Some(pkg.as_str()), &child)
                            ),
                            m.doc_lines(),
                            CompletionKind::Module,
                        ),
                    ),
                    crate::hover::collection::ExportRow::Ty(t) => {
                        push_item(&mut out, CompletionOut::of(&Row::Ty(t)));
                    }
                    crate::hover::collection::ExportRow::Fn(fv) => {
                        push_item(&mut out, CompletionOut::of(&Row::Fn(fv)));
                    }
                    crate::hover::collection::ExportRow::Let(l) => {
                        push_item(&mut out, CompletionOut::of(&Row::Let(l)));
                    }
                    // a `pub use` re-export: the imported name joined
                    // the module's surface. The cap law picks the icon
                    // until real resolution (same as the use classifier)
                    crate::hover::collection::ExportRow::ReExport(u) => {
                        let src = u.path().join("::");
                        push_item(
                            &mut out,
                            CompletionOut::plain(
                                u.name().to_string(),
                                format!("re-export of {src}::{}", u.name()),
                                Vec::new(),
                                if crate::hover::infer::is_cap(u.name()) {
                                    CompletionKind::Type
                                } else {
                                    CompletionKind::Fn
                                },
                            ),
                        );
                    }
                }
            }
            let _ = path;
        }
    }
    filtered(out, prefix)
}

// ---- the qualified-position tier (intra-package) ----

/// The type tier at an annotation boundary — `let x: ⏐`, `id: u⏐`,
/// `fn f() -> ⏐`: the parser's primitive table (the ONE
/// `PRIMITIVE_TYS` — enumerated, never a duplicated list), the open
/// document's own types, the chain's AMBIENT builtin rows, and the
/// foreign types the document already `use`s. Keywords, fns, and
/// locals stay off: values never ride a type position.
fn type_tier_completions(chain: &Chain) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for p in crate::semantic::PRIMITIVE_TYS {
        push_item(
            &mut out,
            CompletionOut::plain(
                (*p).to_string(),
                "primitive".to_string(),
                Vec::new(),
                CompletionKind::Type,
            ),
        );
    }
    let doc = chain.head();
    let imported = crate::query::definition::import_map(doc);
    for f in chain.iter() {
        let own = std::ptr::eq(f, doc);
        let used_from: HashSet<&str> = imported
            .iter()
            .filter(|(_, pkg)| crate::query::definition::matches_pkg(f, pkg))
            .map(|(name, _)| *name)
            .collect();
        for t in f.types() {
            // the same bare gate the decl tiers read: own types always,
            // foreign types when ambient or imported from this package
            if !own && !t.ambient() && !used_from.contains(t.name()) {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(
                    t.name().to_string(),
                    t.head(),
                    t.doc_lines(),
                    match t.form() {
                        TyForm::Interface => CompletionKind::Interface,
                        _ => CompletionKind::Type,
                    },
                ),
            );
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// Member completions after `<TypeHead>::` — the `::` law's member
/// lists whose head is a type, reusing the index lookup hover's member
/// lane keys on (`find_ty`): an enum head offers its members, any
/// other type head its statics and associated fns (the inherent
/// `impl T { fn .. }` rows — the owner string IS the type's name). An
/// interface head offers nothing: its signatures ride receivers,
/// never `::`. `None` = the head resolves to no indexed type (the
/// caller falls through); a resolved head is FINAL — the filtered
/// list is the answer even when empty (the known-head law, like the
/// module tier's).
fn type_head_completions(chain: &Chain, segs: &[String]) -> Option<Vec<CompletionOut>> {
    let (head, rest) = segs.split_first()?;
    let ty = chain.type_decl(head)?;
    let mut out: Vec<CompletionOut> = Vec::new();
    // a deeper walk under a type head descends nothing — types are not
    // namespaces (`Color::Red::` answers empty, final)
    if rest.is_empty() {
        match ty.form() {
            // `Color::⏐` — the declared members
            TyForm::Enum => {
                for m in ty.fields() {
                    push_item(
                        &mut out,
                        CompletionOut::plain(
                            m.name().to_string(),
                            m.src(),
                            m.doc_lines(),
                            CompletionKind::EnumMember,
                        ),
                    );
                }
            }
            // `Greeter::⏐` — interface members ride receivers
            TyForm::Interface => {}
            // `Counter::⏐` — statics first, then the associated fns
            _ => {
                for m in ty.fields() {
                    // a declared static — the verdict rides the node's
                    // own modifier chain, never a string match on the
                    // rendered slice
                    if m.is_static() {
                        push_item(
                            &mut out,
                            CompletionOut::plain(
                                m.name().to_string(),
                                m.src(),
                                m.doc_lines(),
                                CompletionKind::Field,
                            ),
                        );
                    }
                }
                for f in chain.iter() {
                    for fv in f.fns() {
                        if fv.owner().as_deref() == Some(head.as_str()) {
                            push_item(
                                &mut out,
                                CompletionOut::plain(
                                    fv.name().to_string(),
                                    fv.src(),
                                    fv.doc_lines(),
                                    CompletionKind::Fn,
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Some(out)
}

/// the leaf scope for a resolved mod path — the open document itself
/// when the path IS its own module (the pkg head resolving to the
/// root from the root), else the chain file stamped for it
fn scope_index<'a>(
    chain: &'a Chain<'a>,
    doc: &DocMods,
    pkg: Option<&str>,
    path: &str,
) -> Option<&'a crate::hover::collection::MountedFile> {
    if doc.mod_path.as_deref() == Some(path) {
        return Some(chain.head());
    }
    mods::find_mod(chain, pkg, path)
}

/// A use of the DOC that bound `head` as a member of a NAMED dep
/// (`use calc::{ math }`): answers the dep's pkg name and the walked
/// mod path (the use's own path minus the pkg head — `math`, or
/// `math/grid` for a deep binding). The dep must be a mounted, named
/// index (a module-leaf binding the engine resolves through the
/// dep-module lane); a use that bound a plain name answers nothing
/// here — its module resolution never lands.
fn use_bound_dep_module(
    chain: &Chain,
    doc_file: &crate::hover::collection::MountedFile,
    head: &str,
) -> Option<(String, String)> {
    for u in doc_file.uses() {
        if u.name() != head || u.path().is_empty() {
            continue;
        }
        // the dep must be mounted and named (a named file answers for it)
        if !chain
            .iter()
            .any(|f| f.mount.module.as_deref() == Some(u.pkg()))
        {
            continue;
        }
        // the walked module: a FLAT use's leaf IS the module
        // (`use calc::{ math }` → `math`); a qualified use's deeper
        // path spells it (`use calc::math::{ grid }` → `math/grid`
        // when the leaf itself names a module)
        let path = if u.path().len() == 1 {
            u.name().to_string()
        } else {
            u.path()[1..].join("/")
        };
        // the binding must land on a MODULE of the dep (a fn/const
        // leaf heads no path)
        if !mods::find_mod(chain, Some(u.pkg()), &path).is_some() {
            continue;
        }
        return Some((u.pkg().to_string(), path));
    }
    None
}

/// Position-path completions: after `modname::` in a package's own
/// file, the resolved module's members VISIBLE to the current file —
/// the phase-3 tier predicate (`private` reaches the declaring module
/// and descendants, `pub(super)` the parent's subtree, `pub(pkg)` the
/// whole package, `pub` everywhere) applied to every row. Declared
/// children of any edge vis ride as walkable segments (the
/// intra-package walk gates leaves, not heads). The dep-module lane
/// rides beside the own-package one: a use of this doc that bound a
/// module of a named dep (`use calc::{ math }`) heads the path into
/// the DEP's tree — the editor's mirror of the engine's
/// `math::sqrt(x)` / `math::PI` resolution. `None` = the head did not
/// resolve to a module (the caller falls through).
fn position_path_completions(
    chain: &Chain,
    doc: &DocMods,
    segs: &[String],
) -> Option<Vec<CompletionOut>> {
    let (head, rest) = segs.split_first()?;
    let doc_file = chain.head();
    let (pkg, start) = match mods::resolve_position_head(doc, doc_file, chain, head) {
        Some(start) => (doc.pkg.clone().unwrap_or_default(), start),
        None => use_bound_dep_module(chain, doc_file, head)?,
    };
    let pkg = pkg.as_str();
    let path = mods::walk_position_mods(chain, Some(pkg), &start, rest)?;
    let from = doc.mod_path.as_deref().unwrap_or("");
    let Some(scope) = scope_index(chain, doc, Some(pkg), &path) else {
        return Some(Vec::new());
    };
    let mut out: Vec<CompletionOut> = Vec::new();
    // the walkable children — any declared edge (intra-package)
    for m in scope.mods() {
        let child = mods::child_path(&path, m.name());
        push_item(
            &mut out,
            CompletionOut::plain(
                m.name().to_string(),
                format!(
                    "file module — {}",
                    mods::display_of(chain, Some(pkg), &child)
                ),
                m.doc_lines(),
                CompletionKind::Module,
            ),
        );
    }
    for t in scope.types() {
        if !mods::vis_allows(from, &path, t.vis()) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                t.name().to_string(),
                t.head(),
                t.doc_lines(),
                match t.form() {
                    TyForm::Interface => CompletionKind::Interface,
                    _ => CompletionKind::Type,
                },
            ),
        );
    }
    for f in scope.fns() {
        if f.owner().is_some() || !mods::vis_allows(from, &path, f.vis()) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                f.name().to_string(),
                f.src(),
                f.doc_lines(),
                CompletionKind::Fn,
            ),
        );
    }
    for l in scope.lets() {
        if !mods::vis_allows(from, &path, l.vis()) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                l.name().to_string(),
                l.src(),
                l.doc_lines(),
                CompletionKind::Const,
            ),
        );
    }
    // a `pub use` re-export: the imported name joined the module's
    // surface — the same tier the use-path exports ride, gated by the
    // use's declared vis (the engine's leaf gate reads the same row).
    // The cap law picks the icon until real resolution (same as the
    // use classifier)
    for u in scope.uses() {
        if !mods::vis_allows(from, &path, u.vis()) {
            continue;
        }
        let src = u.path().join("::");
        push_item(
            &mut out,
            CompletionOut::plain(
                u.name().to_string(),
                format!("re-export of {src}::{}", u.name()),
                Vec::new(),
                if crate::hover::infer::is_cap(u.name()) {
                    CompletionKind::Type
                } else {
                    CompletionKind::Fn
                },
            ),
        );
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Some(out)
}

// ---- the auto-import tier ----

/// the foreign public names (indexes whose module is named) matching
/// the bare prefix, each carrying the `use mod::Name;` insert — after
/// the local results (the caller extends bare_completions' output), so
/// `sortText` sorts them after everything already offered. Names the
/// document already imports are NOT re-offered and the `pub builtin`
/// gate keeps holding (an import offer for a gated name would complete
/// a name the compile still rejects until the use lands — the gate is
/// the compiler's ambient split, mirrored).
///
/// `direct_deps` is the open document's own dep set — its manifest's
/// `deps`/`dev-deps` rows, state-derived FACT threaded the
/// [`DocMods`](crate::mods::DocMods) way (never the state itself). The
/// direct-dep gate DERIVES here, at the read, never stamped on the
/// index: the compiler's closure check (`Session::resolve_use` →
/// `declares`) refuses `use` of a mounted-but-undeclared package (the
/// D5 diagnostic), so a TRANSITIVE-only dep stays off — its names
/// would complete into a compile the program rejects. The gate speaks
/// only on mounted dep indexes (a named module); plain workspace
/// files never ride this tier.
fn auto_import_completions(
    chain: &Chain,
    ast: &Ast,
    src: &str,
    prefix: &str,
    direct_deps: &HashSet<String>,
) -> Vec<CompletionOut> {
    // the gate itself: the ambient prelude (origin `core` — a walked
    // dep's origin is always a uri, never the bare label, so no
    // collision) or a module the document's own manifest names
    let direct = |f: &crate::hover::collection::MountedFile| {
        f.mount.origin == "core"
            || f.mount
                .module
                .as_deref()
                .is_some_and(|m| direct_deps.contains(m))
    };
    let imported: HashSet<(&str, &str)> = chain
        .head()
        .uses()
        .into_iter()
        .map(|u| (u.name(), u.pkg()))
        .collect();
    let gated: HashSet<String> = chain.iter().flat_map(|f| f.pub_gated()).collect();
    let insert = import_insert(src, ast);
    let mut out: Vec<CompletionOut> = Vec::new();
    for f in chain.iter().skip(1) {
        let Some(m) = &f.mount.module else { continue };
        if !direct(f) {
            continue; // a transitive-only dep — `use` of it is the D5 error
        }
        let m = m.as_str();
        let mut foreign = |name: &str, doc: &[String], kind: CompletionKind| {
            if imported.contains(&(name, m)) {
                return; // already imported — never re-offered
            }
            if gated.contains(name) {
                return; // the ambient split keeps holding
            }
            if !prefix.is_empty() && !name.starts_with(prefix) {
                return;
            }
            // a fresh line either way: after a use the newline leads,
            // at the body top it trails — `rut fmt` owns any tidying
            let text = if insert.1 {
                format!("\nuse {m}::{name};")
            } else {
                format!("use {m}::{name};\n")
            };
            out.push(CompletionOut {
                label: name.to_string(),
                detail: format!("{m}::{name} — import"),
                doc: doc.to_vec(),
                kind,
                sort_text: Some(format!("~{name}")),
                additional_text_edits: vec![(insert.0, text)],
            });
        };
        for t in f.types() {
            if !t.is_pub() {
                continue;
            }
            let doc = t.doc_lines();
            foreign(
                t.name(),
                &doc,
                match t.form() {
                    TyForm::Interface => CompletionKind::Interface,
                    _ => CompletionKind::Type,
                },
            );
        }
        for fv in f.fns() {
            if fv.owner().is_some() {
                continue;
            }
            let doc = fv.doc_lines();
            foreign(fv.name(), &doc, CompletionKind::Fn);
        }
        for l in f.lets() {
            let doc = l.doc_lines();
            foreign(l.name(), &doc, CompletionKind::Const);
        }
    }
    // label dedup across modules: the first module (chain order) wins,
    // so one name offers one import
    let mut seen: HashSet<String> = HashSet::new();
    out.retain(|c| seen.insert(c.label.clone()));
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// the bare-position prefix: the partial ident the cursor sits in
/// (`Ve⏐` → `Ve`), else empty
fn import_insert(src: &str, ast: &Ast) -> (Range, bool) {
    let lines = LineIndex::new(src);
    let after_use = leading_use_end(ast);
    let (offset, newline_first) = match after_use {
        Some(off) => (off, true),
        None => (body_start(src), false),
    };
    let (line, ch) = lines.position(src, offset);
    (
        Range {
            start: ls_types::Position {
                line,
                character: ch,
            },
            end: ls_types::Position {
                line,
                character: ch,
            },
        },
        newline_first,
    )
}

/// the source end of the last `use` in the leading Use run — `None`
/// when the module opens with anything else. A use item's span is
/// EXACT (v2's own-terminator law): it ends at the statement's own `;`
fn leading_use_end(ast: &Ast) -> Option<u32> {
    let mut last: Option<u32> = None;
    for item in ast.root().items(ast) {
        if let ItemRef::Use(h) = item {
            last = Some(h.span(ast).hi);
        } else {
            break; // the run ends at the first item that isn't a use
        }
    }
    last
}

/// the byte offset where the file body starts: past the leading blank
/// and `//` comment lines (a file of only comments → the end)
fn body_start(src: &str) -> u32 {
    let mut off = 0usize;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            off += line.len();
        } else {
            break;
        }
    }
    off as u32
}

/// first definition wins — the document shadows the chain
/// the call site's prefix filter + the label sort — presentation over
/// a tier's rows (the doors enumerate unfiltered)
fn filtered(mut out: Vec<CompletionOut>, prefix: &str) -> Vec<CompletionOut> {
    if !prefix.is_empty() {
        out.retain(|c| c.label.starts_with(prefix));
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

fn push_item(out: &mut Vec<CompletionOut>, c: CompletionOut) {
    if !out.iter().any(|x| x.label == c.label) {
        out.push(c);
    }
}

// ---- the LSP mapping ----

use ls_types::{
    CompletionItem, CompletionItemKind, Documentation, MarkupContent, MarkupKind, TextEdit,
};

/// plain completion data → the LSP item (kind icons, detail, docs,
/// the auto-import tier's sort text + additional edits) — the one
/// mapper every host shares
pub fn lsp_item(c: CompletionOut) -> CompletionItem {
    CompletionItem {
        label: c.label,
        kind: Some(match c.kind {
            CompletionKind::Keyword => CompletionItemKind::KEYWORD,
            CompletionKind::Module => CompletionItemKind::MODULE,
            CompletionKind::Type => CompletionItemKind::CLASS,
            CompletionKind::Interface => CompletionItemKind::INTERFACE,
            CompletionKind::Const => CompletionItemKind::CONSTANT,
            CompletionKind::Fn => CompletionItemKind::FUNCTION,
            CompletionKind::Method => CompletionItemKind::METHOD,
            CompletionKind::Field => CompletionItemKind::FIELD,
            CompletionKind::EnumMember => CompletionItemKind::ENUM_MEMBER,
            CompletionKind::Variable => CompletionItemKind::VARIABLE,
            // LSP spells no parameter kind — the variable icon carries it
            CompletionKind::Parameter => CompletionItemKind::VARIABLE,
        }),
        detail: (!c.detail.is_empty()).then_some(c.detail),
        documentation: (!c.doc.is_empty()).then(|| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: c.doc.join("\n\n"),
            })
        }),
        sort_text: c.sort_text,
        additional_text_edits: (!c.additional_text_edits.is_empty()).then(|| {
            c.additional_text_edits
                .into_iter()
                .map(|(range, new_text)| TextEdit { range, new_text })
                .collect()
        }),
        ..Default::default()
    }
}
