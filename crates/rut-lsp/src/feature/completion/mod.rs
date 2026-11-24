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
//! The use-path tiers: at `use <cursor>` the world's nameable module
//! names compete (the document's own manifest rows + the ambient
//! prelude — mounting is transitive, nameability declaration-scoped);
//! at `use mod::<cursor>` that module's exported public names. The
//! bare position rides the compiler's bare-name law: the
//! fn-local bindings ahead of the keyword table, then the document's
//! own decls + the world's ambient builtin rows + the names the
//! document already `use`s — every other foreign name rides only the
//! auto-import tier (with the `use mod::Name;` insert, sorted after
//! the local items). Annotation boundaries (`let x: ⏐`, `-> T⏐`) get
//! the type tier: primitives + document/imported types, never
//! keywords, fns, or locals. The `mod NAME;` name slot (item level
//! only) is its own tier: the module files beside the open document,
//! probed on DISK through the env — the loader's own spellings,
//! nothing indexed.
//!
//! The seams: `edge` classifies the typing edge from the tree;
//! `tiers` answers the bare/locals/member/use tiers; `paths` the
//! qualified/annotation tiers; `auto_import` the foreign-name tier
//! with the `use` insert; `rows` is the row vocabulary and the one
//! wire mapping. Children read the parent's private items — the
//! surface stays `complete` + `CompletionOut`.

mod auto_import;
mod edge;
mod mod_name;
mod paths;
mod rows;
mod tiers;

pub use rows::{lsp_item, CompletionOut};

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

use std::collections::HashSet;

use rut_ast::{AnyHandle, ExprRef, IdentId, Span, TyRef};

use crate::env::Io;
use crate::resolve::cursor::{self, Cursor};
use crate::world::{File, FileId, World};

use crate::resolve::{members, members_for, use_exports};
use auto_import::auto_import_completions;
use edge::typing_edge;
use paths::{position_path_completions, type_head_completions, type_tier_completions};
use rows::filtered;
use tiers::{bare_tier_out, local_completions, member_tier_out, mod_name_tier_out, use_tier_out};

/// Completions at `pos`: use-path tiers inside a `use` statement
/// (walking a package's mod namespaces), qualified-position tiers
/// after `modname::` in the package's own files, member items after
/// `recv.` (filtered by the typed prefix), the type tier at an
/// annotation boundary, otherwise the visible fn-local bindings ahead
/// of the keyword table plus the visible decls under the bare-name law
/// plus the auto-import tier, deduped by label. The document's own
/// normalized text (the spans' currency — the auto-import edits ride
/// it) is the file's. The mod-name slot's FILE facts derive from the
/// file's own entry: the directory from its origin (`file://` uris
/// only), the flavor from its stamp — no dir / no env, and the slot
/// answers empty, never the bare dump. `direct_deps` is the open
/// document's own dep set (its manifest's rows) — the auto-import tier
/// derives its gate from it live. `io` is the stored host env (the
/// loop's capability) — the mod-name slot's disk probe reads it.
pub fn complete(
    w: &World,
    file: FileId,
    pos: u32,
    direct_deps: &HashSet<String>,
    io: Option<&dyn Io>,
) -> Vec<CompletionOut> {
    let head = w.file(file);
    let ast = head.doc.ast();
    let src = head.doc.src();
    match typing_edge(w, file, pos, src) {
        Edge::Use { node, segs, prefix } => {
            // the OWN lane first — a head that speaks for the doc's own
            // package (its own name, or an own mod edge) rides the
            // position-path machinery: the intra-package walk and the
            // visible-members gate, exactly what the compiler's own-use
            // check accepts (`use imp::⏐` offers the module's visible
            // members). A foreign head keeps the cross-package tier.
            if let Some(out) = position_path_completions(w, file, &segs) {
                return filtered(out, &prefix);
            }
            use_tier_out(w, use_exports(w, file, node, pos, direct_deps), &prefix)
        }
        // a lone `:` (or the `->` arrow) under the cursor is the
        // annotation boundary — the type tier answers it: the primitive
        // table, the document's types, and the imported foreign types
        // (no keywords, fns, or locals — values never ride a type
        // position). `Vec<⏐` stays ambiguous (`<` vs the comparison)
        // and is not this tier.
        Edge::Annotation { prefix } => filtered(type_tier_completions(w, file), &prefix),
        Edge::Qualified { segs, prefix } => {
            if let Some(out) = position_path_completions(w, file, &segs) {
                return filtered(out, &prefix);
            }
            // the head did not resolve to a module — the `::` law's
            // other member lists ride the same trigger: enum members,
            // namespace statics, a type head's associated fns (the
            // member-vis dial rides the site context too)
            if let Some(out) = type_head_completions(w, file, &segs) {
                return filtered(out, &prefix);
            }
            // the head resolved to nothing known — `::` in an unknown
            // context falls through to the tiers below, the typed
            // prefix riding along (the bare tier does not filter; the
            // auto-import tier does)
            let mut out = local_completions(w, file, pos);
            out.extend(bare_tier_out(w, file));
            out.extend(auto_import_completions(
                w,
                file,
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
            if let Some(out) = position_path_completions(w, file, &head) {
                return filtered(out, &prefix);
            }
            member_tier_out(
                members_for(w, file, pos, crate::resolve::cursor::Recv::Seg(tail))
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
                    let name = ast.name(segs[0].name.ident).to_string();
                    if let Some(out) =
                        position_path_completions(w, file, std::slice::from_ref(&name))
                    {
                        return filtered(out, &prefix);
                    }
                }
            }
            // member completion shares hover's receiver inference (the
            // door types the receiver INSIDE — field reads, chained
            // calls, loop variables); the member-vis dial reads the
            // site context
            member_tier_out(members(w, file, pos, node).unwrap_or_default(), &prefix)
        }
        // FINAL: the mod-name slot answers module candidates only — a
        // miss is an empty list, never the bare keyword dump (the use
        // tier's law). The tier reads no stamp: none is
        // involved, the disk beside the document is the answer.
        Edge::ModName { node, prefix } => mod_name_tier_out(w, file, io, node, &prefix),
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
            let mut out = local_completions(w, file, pos);
            out.extend(bare_tier_out(w, file));
            out.extend(auto_import_completions(
                w,
                file,
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

/// The typing edge, from the tree: the byte before the cursor names
/// the context — a `use` ancestor (the use tiers), a member node (the
/// incomplete `recv.` shape included: the member is being typed), a
/// `::` path's tail segment (the qualified tier), a type slot at an
/// annotation boundary (the type tier), else the bare tiers with the
/// partial ident (the fence residue riding `Doc::name_at`). The three
/// on-name shapes are the ONE classification's
/// ([`crate::resolve::cursor`]); the between-tokens probes — the
/// annotation boundary, the mod-name slot, the dangling `::`, the
/// cursor past a member — are completion's own. No token scans.
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
    /// the `mod NAME;` name slot — the node under the cursor (the decl
    /// being typed, or the `mod` fence) + the typed prefix
    ModName { node: AnyHandle, prefix: String },
    /// the type slot's `.`/`::` member request (`let c: layout.Co⏐`):
    /// the head segments for the mod tier, the tail ident for the
    /// member tier — no bare fallthrough (a miss is an empty list)
    TypeDot {
        head: Vec<String>,
        tail: IdentId,
        prefix: String,
    },
}

/// the typed part of `sp` — the slice up to the cursor (a cursor past
/// the span's end typed the whole name and moved on: the prefix is the
/// whole name, the tier filters on it)
fn partial_text(head: &File, sp: Span, pos: u32) -> String {
    let end = (pos as usize).clamp(sp.lo as usize, sp.hi as usize);
    head.doc.text(Span::new(sp.lo, end as u32)).to_string()
}

/// the type slot's tail prefix — the cursor rides the slot's LAST
/// segment (a single-seg partial is the annotation tier's prefix; a
/// multi-seg path's tail is the qualified tier's, which the walk
/// answered before this lane)
fn ty_slot_prefix(head: &File, t: TyRef, pos: u32) -> Option<String> {
    let TyRef::Path(h) = t else {
        return None;
    };
    let ast = head.doc.ast();
    let segs = &h.get(ast).segs;
    if segs.len() != 1 {
        return None; // multi-seg: the qualified lane answers
    }
    let sp = segs[0].name.span;
    crate::doc::on_name(sp, pos).then(|| partial_text(head, sp, pos))
}

/// the type slot's `.`-qualified spelling — positions qualify with `.`
/// (`let c: layout.Co⏐`, the editor's real spelling): the cursor past
/// the slot's tail segment with a `.` plus at most a partial ident
/// between — the same members tier the `::` form rides, headed by the
/// slot's own segments (text-backed, the dot guard's precedent: the
/// TYPE grammar spells no `.` access, so the punct lives in the
/// statement's recovery region)
fn ty_slot_dot(head: &File, t: TyRef, pos: u32) -> Option<Edge> {
    let TyRef::Path(h) = t else {
        return None;
    };
    let ast = head.doc.ast();
    let segs = &h.get(ast).segs;
    let last = segs.last()?;
    if last.name.span.hi >= pos {
        return None; // the cursor rides the slot itself — not a `.` run
    }
    let end = (pos as usize).min(head.doc.src().len());
    let gap = head.doc.src()[last.name.span.hi as usize..end].trim_end();
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
        head: segs
            .iter()
            .map(|s| ast.name(s.name.ident).to_string())
            .collect(),
        tail: last.name.ident,
        prefix: after.to_string(),
    })
}

/// does the text between `from` and `pos` spell the boundary punct
/// before any `=`? (the annotation probe — the tree records the
/// slots, not the punct that introduced them)
fn gap_spells(head: &File, from: u32, pos: u32, punct: char) -> bool {
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

/// the bare tiers' prefix — the typed part of the name the ONE
/// classification rides: a lone path segment, a declaring name (a
/// let/param/loop-var mid-typed), a path segment (a `::` head the
/// qualified lane did not take) — the doc's `text` from the cursor's
/// name span start to `pos`; the fence residue keeps `Doc::name_at`
/// (the word under the byte before the cursor, when the probe landed
/// on an error fence)
fn bare_prefix(
    w: &World,
    head: &File,
    ast: &rut_ast::Ast,
    p0: rut_ast::AnyHandle,
    pos: u32,
    c: &Cursor,
    src: &str,
) -> String {
    let on = |sp: Span| partial_text(head, sp, pos);
    if std::env::var("RUT_EDGE_DEBUG").is_ok() {
        let kind = match c {
            Cursor::Name { .. } => "Name",
            Cursor::Decl(_) => "Decl",
            Cursor::Path { .. } => "Path",
            Cursor::Nothing => "Nothing",
            _ => "other",
        };
        let nb = w
            .file(w.id_of(head))
            .doc
            .bindings()
            .iter()
            .map(|b| (ast.name(b.name), b.decl_ident_span.lo, b.decl_ident_span.hi))
            .collect::<Vec<_>>();
        eprintln!(
            "bare debug: pos={pos} cursor={kind} p0={:?} bindings={nb:?}",
            p0.kind
        );
    }
    match c {
        Cursor::Name { span, .. } => on(*span),
        Cursor::Decl(d) => on(d.name_span(w)),
        Cursor::Path { node, at, .. } => on(cursor::path_seg(ast, *node, *at).1),
        // the fence residue: the word under the byte before the cursor
        _ if p0.is::<rut_ast::ErrorNode>() => {
            if let Some((sp, _)) = head.doc.name_at(pos.saturating_sub(1)) {
                let end = (pos as usize).clamp(sp.lo as usize, sp.hi as usize);
                return src[sp.lo as usize..end].to_string();
            }
            String::new()
        }
        _ => String::new(),
    }
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
