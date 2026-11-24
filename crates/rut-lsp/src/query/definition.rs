//! Go-to-definition and go-to-type-definition — the lsp-features survey
//! §3.3 resolution order, one pure query every host shares:
//!
//! 1. **within-file binding layer** — locals/params/iter-vars: a LOOKUP
//!    against the binding pass (`decl_ident_span` is the target),
//!    shadow-aware and scope-exit correct;
//! 2. **within-file decl layer** — fields/enum members at their exact
//!    recorded name spans, module lets, fns/types/impl methods, all
//!    keyed on the decl maps' node name spans;
//! 3. **cross-file through the use graph** — a name the document imports
//!    (`use pouch::Vec;`) resolves only inside files whose explicit
//!    module name matches the named pkg, or — for the unnamed
//!    (workspace) files — whose origin does (path segment or file
//!    stem — how `rut.jsonc` `name` works, without re-implementing the
//!    manifest loader);
//!    un-imported names answer NOTHING (the file-scope law — the
//!    one-name law's Act 2: sibling modules may share a name, so a
//!    name the file neither declares nor imports is no jump, never a
//!    foreign decl). An OWN
//!    package head — the doc's own pkg name, or one of its own mod
//!    edges (the ancestor-walk spelling, the doc's own stamp keyed) —
//!    rides
//!    the intra-package lanes instead: the head opens the resolved
//!    module's file top, a middle segment lands on its mounting edge,
//!    the leaf on the walked mod path's decl;
//! 4. **the ambient prelude** — the one embedded file's
//!    `include_str!` origin carries the true `rut/...` path
//!    (`Stamp::src_path`), so a jump lands in the real source file.
//!
//! Layering mirrors `hover` exactly (decl sites, then member position
//! with the known-receiver-is-final law, then type position, then the
//! binding pass, then the bare-name file scope) so the two queries can't
//! disagree about what an ident means. The bare-name layers (4/6/7)
//! answer through the ONE file-scope home
//! ([`file_scope`](crate::world::collection) — the queried file's own decls ∪ its use
//! edges ∪ the ambient prelude); the use-edge targeting here (layer 2
//! and typeDefinition's computed heads) is navigation, not a spelling
//! question, and keeps its own walked-path lanes.
//!
//! The plan's API surface names a third door, `get_declaration(pos)` —
//! the same walk with the surface preference inverted. CONSCIOUS
//! SUPERSESSION: the wire this face serves spells no
//! `textDocument/declaration` request (the three-lane ABI is frozen),
//! so the door has no caller and does not exist; the family's
//! body-vs-surface pick policy lives in the ONE decision point
//! (`prefer_bodies`), and the declaration flavor is
//! this walk with that policy inverted — it lands the day a face grows
//! the request, with no other change. Multiple hits come back as
//! multiple locations — the LSP-native candidate list (a peek), never a
//! wrong silent jump. A miss is an empty vector. Honest limits (the
//! survey §3.3): no visibility checking (the engine's `pub(super)`/
//! `pub(pkg)` rules are not modeled); generics resolve at the head.

use std::collections::HashMap;

use crate::semantic::Span;
use crate::world::collection::{MemberHit, MemberTarget};
use crate::world::infer::QualHit;
use crate::world::pkg::{import_map, matches_pkg};
use crate::world::types::head_of_ty;
use crate::world::{File, FileId, World};
use ls_types::Range;

/// one resolved jump target — the uri is the doc itself, a workspace
/// path/URI, or (for the embedded prelude) the true repo-relative
/// source path; the client face resolves it against the workspace
#[derive(Debug, Clone, PartialEq)]
pub struct DefLocation {
    pub uri: String,
    pub range: Range,
}

/// everything one query needs — the session's world beside the
/// queried file's id in it (the open document's own entry; its
/// buffer is the truth). `references` builds the same shape per
/// candidate file, so the matcher runs unchanged there.
pub struct Ctx<'a> {
    /// the session's world — every file the query reads
    pub w: &'a World,
    /// the queried file — its own entry in the world
    pub file: FileId,
}

impl<'a> Ctx<'a> {
    /// the queried file — its own entry in the world
    pub(crate) fn doc_file(&self) -> &'a File {
        self.w.file(self.file)
    }
}

/// byte span -> LSP range over the TARGET file's own text — a
/// location's uri is its file's (`src_path`, else the origin spelled
/// through [`origin_uri`])
pub(crate) fn loc_at(f: &File, span: Span) -> DefLocation {
    DefLocation {
        uri: f
            .stamp
            .src_path
            .clone()
            .unwrap_or_else(|| origin_uri(&f.stamp.origin)),
        range: crate::query::analysis::range(f, span),
    }
}

/// a target file's origin as a URI — workspace origins are already
/// paths or `file://` URIs; the prelude's provenance label never
/// becomes a jump target (it carries `src_path` instead)
fn origin_uri(origin: &str) -> String {
    if origin.contains("://") {
        origin.to_string()
    } else if origin.starts_with('/') {
        format!("file://{origin}")
    } else {
        origin.to_string()
    }
}

/// the world restricted to the pkg that declares `name` — the
/// computed-head lane's gate (typeDefinition's value-reached type
/// heads; the spelling lanes answer through the file scope instead).
/// An import the world cannot satisfy falls back to the flat world
/// (navigation for a computed head: better a broad answer than none)
fn chain_for<'a>(
    w: &'a World,
    file: FileId,
    imported: &HashMap<&str, &str>,
    name: &str,
) -> Vec<&'a File> {
    match imported.get(name) {
        Some(pkg) => {
            let filtered: Vec<&File> = w
                .chain(file)
                .map(|id| w.file(id))
                .filter(|f| matches_pkg(f, pkg))
                .collect();
            if filtered.is_empty() {
                w.chain(file).map(|id| w.file(id)).collect()
            } else {
                filtered
            }
        }
        None => w.chain(file).map(|id| w.file(id)).collect(),
    }
}

/// a type's definition target span — the declaring ident (the node's
/// own recorded name span); the whole decl is the fallback when the
/// name span is missing on a degenerate parse
pub(crate) fn ty_span(t: &crate::world::collection::TyView) -> Span {
    t.name_span().unwrap_or(t.span())
}

pub(crate) fn fn_span(f: &crate::world::collection::FnView) -> Span {
    f.name_span().unwrap_or(f.span())
}

// ---- definition ----

/// `textDocument/definition` — the ident at `pos` to its declaration.
/// Empty when nothing resolves (keywords, unknown names, primitives
/// with no surface decl).
///
/// The resolver family's bodies-flavored door: body-vs-surface is the
/// pick policy INSIDE (`prefer_bodies` — the mounted-over-embed rule,
/// then the body over the `.d.rut` surface). The plan's declaration
/// flavor (`get_declaration`) is consciously superseded — see the
/// module doc: the ABI spells no declaration request, and the pick
/// policy's single decision point makes the inverted walk a local
/// change when one arrives.
pub fn get_definition(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    get_definition_targets(ctx, pos)
}

fn get_definition_targets(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    let w = ctx.w;
    let file = ctx.file;
    let doc = ctx.doc_file();
    let ast = doc.doc.ast();
    // the cursor's node — the byte under it, else the byte just before
    // (the token scan's boundary convention). The fence residue names
    // nothing.
    let Some(h) = cursor_node(ast, pos) else {
        return Vec::new();
    };
    if h.is::<rut_ast::ErrorNode>() {
        return Vec::new();
    }
    // the `Self` spellings name no declaration of their own (the
    // keyword law the token scan carried)
    if h.is::<rut_ast::SelfNode>() || h.is::<rut_ast::SelfTyNode>() {
        return Vec::new();
    }

    // 1. decl sites — the cursor rides a decl node's OWN name span
    //    (field / enum member / module let / fn decl / `mod` edge —
    //    free fns, impl methods, `[constructor]`s alike): the
    //    declaration IS the answer (go-to-def on a decl → itself)
    if let Some(name_span) = decl_site_span(ast, h, pos) {
        return vec![loc_at(doc, name_span)];
    }

    // 1b. the mod-edge click — the cursor rides a `mod NAME;` edge's
    //     own name span: the mounted child's file top (the
    //     own-package lanes; an unstamped doc or an unmounted child is
    //     a miss, never a wrong jump)
    if let Some(loc) = mod_edge_target(ctx, h, pos) {
        return vec![loc];
    }

    // 2. use-statement names and path segments — the survey's
    //    use-graph edge: ctrl+click on `Vec` inside `use pouch::Vec;`
    //    jumps to the exporting module's decl (strictly inside the
    //    named pkg, now strictly inside the walked mod path too), a
    //    MOD PATH segment jumps to the `mod NAME;` edge that mounts
    //    it, and the PKG HEAD jumps to the pkg's root file at the
    //    file top
    if let Some(locs) = use_site_targets(ctx, h, pos) {
        return locs;
    }

    // 3. member position — the walk found a member node (or a path's
    //    tail segment) and the cursor rides its member: the door
    //    derives member + receiver INSIDE. A resolved receiver never
    //    falls through (the use-gate law hover established), an
    //    unknown one keeps the bare-name fallback
    if member_name_span(ast, h, pos).is_some() {
        match crate::world::collection::member(w, file, pos, h) {
            MemberHit::Found(target) => {
                return member_locations(&target);
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through to the name layers
        }
    }

    // 3b. the qualified-path lane — `m::…::x` where the head opens a
    //     module the queried file binds: the walked module's own file
    //     scope answers (the module namespace lane). AUTHORITATIVE
    //     for a qualified TAIL: an unbound head is an empty answer,
    //     and the bare-name layers below never read a qualified
    //     spelling's tail (a qualified name is not the doc's own
    //     bare name)
    if let Some(locs) = qualified_tail_locations(ctx, ast, h, pos) {
        return locs;
    }

    // the name lanes key on the node's own recorded name — a path
    // segment, a param name, a local let's name
    let Some((name_id, _)) = node_name(ast, h, pos) else {
        return Vec::new();
    };
    let name = ast.name(name_id).to_string();

    // 4. type position — the file scope's spelled type (the ONE home):
    //    the file's own decl, its use-bound decl, or the ambient
    //    prelude row; a name the file never binds is no jump
    if crate::world::infer::is_cap(&name) || crate::semantic::is_primitive_ty(&name) {
        let tys = crate::world::collection::file_scope(w, file, &name).types;
        if !tys.is_empty() {
            return tys.iter().map(|t| loc_at(t.file, ty_span(t))).collect();
        }
        // a primitive token with no surface decl (i32, bool …) is a
        // miss — never a wrong jump
        if crate::semantic::is_primitive_ty(&name) {
            return Vec::new();
        }
    }

    // 5. the binding layer — locals/params/iter-vars, shadow-correct
    //    (the doc's rows read internally); the declaring ident span is
    //    the span-first contract
    if let Some(b) = rut_ast::resolve_binding(doc.doc.bindings(), pos, name_id) {
        return vec![loc_at(doc, b.decl_ident_span)];
    }

    // an AMBIGUOUS name — two distinct bindings, a one-name violation
    // the checker refuses — jumps NOWHERE at a use site (never a pick
    // between them; the hover lists the candidates). Decl sites
    // answered at layer 1, locals at layer 5 — both exact.
    if crate::world::collection::one_name_refused(w, file, &name) {
        return Vec::new();
    }

    // 6. module-let use sites — the file scope's lets (the door),
    //    every hit (the candidate list)
    let scope = crate::world::collection::file_scope(w, file, &name);
    if !scope.lets.is_empty() {
        return scope
            .lets
            .iter()
            .map(|l| loc_at(l.file, l.name_span().unwrap_or(l.span())))
            .collect();
    }

    // 7. fn names — the file scope's fns (free fns and impl methods),
    //    every hit (a peek, not a guess)
    scope
        .fns
        .iter()
        .map(|f| loc_at(f.file, fn_span(f)))
        .collect()
}

/// `textDocument/typeDefinition` — the survey prices this cheap and it
/// is: resolve the expression's type head (the receiver rules — member
/// declared types, binding types, module lets), then jump to that
/// type's declaration. The same layers, one step further.
pub fn get_type_definition(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    get_type_definition_targets(ctx, pos)
}

fn get_type_definition_targets(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    let w = ctx.w;
    let file = ctx.file;
    let doc = ctx.doc_file();
    let ast = doc.doc.ast();
    let Some(h) = cursor_node(ast, pos) else {
        return Vec::new();
    };
    if h.is::<rut_ast::ErrorNode>() || h.is::<rut_ast::SelfNode>() || h.is::<rut_ast::SelfTyNode>()
    {
        return Vec::new();
    }
    let imported = import_map(w, file);

    // member position: the member's DECLARED type — `w.item` (a
    // `?Circle` field) jumps to the `Circle` decl; a resolved receiver
    // is final (the use-gate law)
    if let Some(name_span) = member_name_span(ast, h, pos) {
        let _ = name_span;
        match crate::world::collection::member(w, file, pos, h) {
            MemberHit::Found(target) => {
                let ty_text = match &target {
                    MemberTarget::Member(_, m, _)
                    | MemberTarget::EnumMember(_, m)
                    | MemberTarget::IfaceMember(_, m) => m.ty(),
                    MemberTarget::ImplFn(f) => f.ret(),
                };
                return ty_text
                    .as_deref()
                    .map(head_of_ty)
                    .and_then(|head| ty_name_locations(ctx, &imported, &head))
                    .unwrap_or_default();
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through
        }
    }

    // the name lanes key on the node's own recorded name
    let Some((name_id, _)) = node_name(ast, h, pos) else {
        return Vec::new();
    };
    let name = ast.name(name_id).to_string();

    // a type token: its own declaration (definition's answer). A type
    // the one-name law refuses — beside a same-named decl or leaf —
    // answers definition's way: its own decl site is ITSELF, every
    // other spelling of the name jumps nowhere (never a pick between
    // the bindings)
    if crate::world::infer::is_cap(&name) || crate::semantic::is_primitive_ty(&name) {
        if crate::world::collection::one_name_refused(w, file, &name) {
            return decl_site_span(ast, h, pos)
                .map(|sp| vec![loc_at(doc, sp)])
                .unwrap_or_default();
        }
        if let Some(locs) = ty_name_locations(ctx, &imported, &name) {
            return locs;
        }
        if crate::semantic::is_primitive_ty(&name) {
            return Vec::new();
        }
    }

    // locals / module lets — their type head's declaration
    if let Some(b) = rut_ast::resolve_binding(doc.doc.bindings(), pos, name_id) {
        let files: Vec<&File> = w.chain(file).map(|id| w.file(id)).collect();
        let qual = |segs: &[String], is_ty: bool| -> Option<QualHit> {
            let scope = crate::world::collection::qualified_scope(w, file, segs, is_ty)?;
            Some(if is_ty {
                QualHit::Ty(scope.types.first()?.name().to_string())
            } else {
                QualHit::Fn(scope.fns.into_iter().next()?)
            })
        };
        return crate::world::infer::binding_ty(doc, &files, b, &qual)
            .as_deref()
            .map(head_of_ty)
            .and_then(|head| ty_name_locations(ctx, &imported, &head))
            .unwrap_or_default();
    }
    match crate::world::infer::module_let_ty(files_of(w, file), &name) {
        Some(head) => ty_name_locations(ctx, &imported, &head).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// the query chain as a plain file window — the inference pass's
/// shape (the queried file first, the world after)
pub(crate) fn files_of(w: &World, file: FileId) -> Vec<&File> {
    w.chain(file).map(|id| w.file(id)).collect()
}

// ---- the cursor walk ----

/// the cursor's node — the byte under it, else the byte just before
/// (a cursor sitting on a token's end reads that token — the old
/// token scan's boundary convention)
pub(crate) fn cursor_node(ast: &rut_ast::Ast, pos: u32) -> Option<rut_ast::AnyHandle> {
    ast.node_at(pos)
        .or_else(|| pos.checked_sub(1).and_then(|p| ast.node_at(p)))
}

/// the name-position convention the old token scan kept: a name ends
/// its own position
pub(crate) fn on_name(sp: Span, pos: u32) -> bool {
    sp.lo <= pos && pos <= sp.hi
}

/// the name span of the decl node the cursor rides — the self-jump
/// lane (the declaration IS the answer)
fn decl_site_span(ast: &rut_ast::Ast, h: rut_ast::AnyHandle, pos: u32) -> Option<Span> {
    macro_rules! decl {
        ($t:ty) => {
            if let Some(d) = h.downcast::<$t>() {
                let dd = d.get(ast);
                return on_name(dd.name.span, pos).then_some(dd.name.span);
            }
        };
    }
    decl!(rut_ast::FieldDeclNode);
    decl!(rut_ast::EnumMemberNode);
    decl!(rut_ast::ModuleLetNode);
    decl!(rut_ast::FnNode);
    decl!(rut_ast::MethodDeclNode);
    // a type decl's own name (class/struct/interface/enum/builtin/
    // host/alias) is the declaration too — the doc's own row, never
    // the file scope's layers: a file the one-name law refuses (the
    // type beside a same-named decl) still jumps the decl under the
    // cursor to ITSELF, where the scope's layers answer nothing
    decl!(rut_ast::ClassNode);
    decl!(rut_ast::StructNode);
    decl!(rut_ast::InterfaceNode);
    decl!(rut_ast::EnumNode);
    decl!(rut_ast::BuiltinTyNode);
    decl!(rut_ast::BuiltinPrimitiveNode);
    decl!(rut_ast::SurfaceStructNode);
    decl!(rut_ast::AliasNode);
    // the binding layer's decls: a param/let/loop-var whose NAME the
    // cursor rides is its own declaration (params sit before their
    // scope, so the scope-containment resolve can't see them)
    if let Some(d) = h.downcast::<rut_ast::ParamNode>() {
        let dd = d.get(ast);
        return on_name(dd.name.span, pos).then_some(dd.name.span);
    }
    if let Some(d) = h.downcast::<rut_ast::LetStmtNode>() {
        let dd = d.get(ast);
        if on_name(dd.name.span, pos) {
            return Some(dd.name.span);
        }
        if let Some(ns) = &dd.destructure {
            for n in ns {
                if on_name(n.span, pos) {
                    return Some(n.span);
                }
            }
        }
        return None;
    }
    if let Some(d) = h.downcast::<rut_ast::ForOfNode>() {
        let dd = d.get(ast);
        return on_name(dd.var.span, pos).then_some(dd.var.span);
    }
    if let Some(d) = h.downcast::<rut_ast::ForCNode>() {
        let dd = d.get(ast);
        return on_name(dd.var.span, pos).then_some(dd.var.span);
    }
    None
}

/// the (name id, name span) the cursor's node records — a path
/// segment, a param name, a let's name (destructured members
/// included), a loop variable
pub(crate) fn node_name(
    ast: &rut_ast::Ast,
    h: rut_ast::AnyHandle,
    pos: u32,
) -> Option<(rut_core::IdentId, Span)> {
    if let Some(p) = h.downcast::<rut_ast::PathNode>() {
        for s in &p.get(ast).segs {
            if on_name(s.name.span, pos) {
                return Some((s.name.ident, s.name.span));
            }
        }
        return None;
    }
    if let Some(p) = h.downcast::<rut_ast::ParamNode>() {
        let d = p.get(ast);
        return on_name(d.name.span, pos).then_some((d.name.ident, d.name.span));
    }
    if let Some(l) = h.downcast::<rut_ast::LetStmtNode>() {
        let d = l.get(ast);
        if on_name(d.name.span, pos) {
            return Some((d.name.ident, d.name.span));
        }
        if let Some(ns) = &d.destructure {
            for n in ns {
                if on_name(n.span, pos) {
                    return Some((n.ident, n.span));
                }
            }
        }
        return None;
    }
    if let Some(f) = h.downcast::<rut_ast::ForOfNode>() {
        let d = f.get(ast);
        return on_name(d.var.span, pos).then_some((d.var.ident, d.var.span));
    }
    if let Some(f) = h.downcast::<rut_ast::ForCNode>() {
        let d = f.get(ast);
        return on_name(d.var.span, pos).then_some((d.var.ident, d.var.span));
    }
    // struct-literal field labels are name positions (the FieldInit
    // rides the literal as data — no node of its own)
    if let Some(sl) = h.downcast::<rut_ast::StructLitNode>() {
        for fi in &sl.get(ast).fields {
            if on_name(fi.name.span, pos) {
                return Some((fi.name.ident, fi.name.span));
            }
        }
        return None;
    }
    // type positions are name positions too (`x: i32|` — the slot's
    // own path node)
    if let Some(t) = h.downcast::<rut_ast::TyPathNode>() {
        for seg in &t.get(ast).segs {
            if on_name(seg.name.span, pos) {
                return Some((seg.name.ident, seg.name.span));
            }
        }
        return None;
    }
    // the type-decl nodes: the cursor on the declared NAME is a name
    // position like any other (the decl-site self-jump answers fields,
    // lets, fns, and mod edges — types and aliases ride here)
    // the decl nodes: the cursor on the declared NAME is a name
    // position (definition's decl-site lane answers first; references
    // keys its targets on these)
    macro_rules! decl_node {
        ($t:ty) => {
            if let Some(d) = h.downcast::<$t>() {
                let dd = d.get(ast);
                return on_name(dd.name.span, pos).then_some((dd.name.ident, dd.name.span));
            }
        };
    }
    decl_node!(rut_ast::FieldDeclNode);
    decl_node!(rut_ast::EnumMemberNode);
    decl_node!(rut_ast::ModuleLetNode);
    decl_node!(rut_ast::MethodDeclNode);
    macro_rules! ty_decl {
        ($t:ty) => {
            if let Some(d) = h.downcast::<$t>() {
                let dd = d.get(ast);
                return on_name(dd.name.span, pos).then_some((dd.name.ident, dd.name.span));
            }
        };
    }
    ty_decl!(rut_ast::ClassNode);
    ty_decl!(rut_ast::StructNode);
    ty_decl!(rut_ast::InterfaceNode);
    ty_decl!(rut_ast::EnumNode);
    ty_decl!(rut_ast::BuiltinTyNode);
    ty_decl!(rut_ast::BuiltinPrimitiveNode);
    ty_decl!(rut_ast::SurfaceStructNode);
    ty_decl!(rut_ast::AliasNode);
    ty_decl!(rut_ast::MethodDeclNode);
    ty_decl!(rut_ast::FnNode);
    ty_decl!(rut_ast::ModDeclNode);
    ty_decl!(rut_ast::ModuleLetNode);
    None
}

/// the member name span the cursor rides — the member lane's position
/// test (a method call, a field access, or a path's tail segment)
fn member_name_span(ast: &rut_ast::Ast, h: rut_ast::AnyHandle, pos: u32) -> Option<Span> {
    if let Some(m) = h.downcast::<rut_ast::MethodNode>() {
        let d = m.get(ast);
        return d
            .member
            .and_then(|n| on_name(n.span, pos).then_some(n.span));
    }
    if let Some(f) = h.downcast::<rut_ast::FieldNode>() {
        let d = f.get(ast);
        return on_name(d.name.span, pos).then_some(d.name.span);
    }
    if let Some(p) = h.downcast::<rut_ast::PathNode>() {
        let segs = &p.get(ast).segs;
        if segs.len() >= 2 {
            if let Some(i) = segs.iter().position(|s| on_name(s.name.span, pos)) {
                if i + 1 == segs.len() {
                    return Some(segs[i].name.span);
                }
            }
        }
    }
    None
}

/// the qualified-path position — the cursor rides a MULTI-segment
/// path (value paths and type-slot paths alike)
struct QualifiedPos {
    /// the segs as spelled
    segs: Vec<String>,
    /// the riding segment's index
    k: usize,
    /// a type-slot path (`x: a::B`) rather than a value path
    is_ty: bool,
}

/// the qualified-path position of the cursor, if it rides one
fn qualified_at(ast: &rut_ast::Ast, h: rut_ast::AnyHandle, pos: u32) -> Option<QualifiedPos> {
    let path = h.downcast::<rut_ast::PathNode>().map(|p| {
        let d = p.get(ast);
        (
            d.segs
                .iter()
                .map(|s| (ast.name(s.name.ident).to_string(), s.name.span))
                .collect::<Vec<_>>(),
            false,
        )
    });
    let ty = h.downcast::<rut_ast::TyPathNode>().map(|t| {
        let d = t.get(ast);
        (
            d.segs
                .iter()
                .map(|s| (ast.name(s.name.ident).to_string(), s.name.span))
                .collect::<Vec<_>>(),
            true,
        )
    });
    let (segs, is_ty) = path.or(ty)?;
    if segs.len() < 2 {
        return None;
    }
    let k = segs.iter().position(|(_, sp)| on_name(*sp, pos))?;
    Some(QualifiedPos {
        segs: segs.into_iter().map(|(n, _)| n).collect(),
        k,
        is_ty,
    })
}

/// the qualified tail's locations — `Some(vec![])` is the
/// authoritative empty (an unbound head opens nothing; the bare-name
/// layers never run on a qualified tail); `None` = not a qualified
/// position. A HEAD or MID-segment cursor names a MODULE: a module the
/// walk lands on jumps to its mounting `mod NAME;` edge (the use
/// path's segment law, mirrored), a package's own name to its root
/// file top; a segment the walk cannot land keeps the pre-existing
/// layers (hover's own fall-through).
fn qualified_tail_locations(
    ctx: &Ctx,
    ast: &rut_ast::Ast,
    h: rut_ast::AnyHandle,
    pos: u32,
) -> Option<Vec<DefLocation>> {
    let q = qualified_at(ast, h, pos)?;
    if q.k + 1 != q.segs.len() {
        return module_segment_target(ctx, &q.segs[..=q.k], q.is_ty);
    }
    let Some(scope) = crate::world::collection::qualified_scope(ctx.w, ctx.file, &q.segs, q.is_ty)
    else {
        return Some(Vec::new());
    };
    let mut hits: Vec<(&File, Span)> = Vec::new();
    if q.is_ty {
        for t in &scope.types {
            hits.push((t.file, ty_span(t)));
        }
    } else {
        for t in &scope.types {
            hits.push((t.file, ty_span(t)));
        }
        for l in &scope.lets {
            hits.push((l.file, l.name_span().unwrap_or(l.span())));
        }
        for f in &scope.fns {
            hits.push((f.file, fn_span(f)));
        }
    }
    Some(hits.into_iter().map(|(f, sp)| loc_at(f, sp)).collect())
}

/// a qualified path's head/mid segment — the module the prefix names:
/// its mounting edge, or the package root's file top
fn module_segment_target(ctx: &Ctx, segs: &[String], is_ty: bool) -> Option<Vec<DefLocation>> {
    let w = ctx.w;
    let target = crate::world::collection::qualified_module(w, ctx.file, segs, is_ty)?;
    let path = crate::resolve::mods::mod_path_of(w.file(target));
    if path.is_empty() {
        // the package's own name — the root module, at its file top
        // (`Span::new(0, 0)`, the pkg-head convention)
        return Some(vec![loc_at(w.file(target), Span::new(0, 0))]);
    }
    // a `mod NAME;` edge in the parent module's file — inside the
    // TARGET's own package (the queried file's own entry when the
    // parent is the open document)
    let name = path.rsplit('/').next()?;
    let parent_path = rut_lir::check::parent_of(path);
    let parent = if w.file(ctx.file).stamp.module.as_deref()
        == w.file(target).stamp.module.as_deref()
        && w.file(ctx.file).stamp.mod_path.as_deref() == Some(parent_path)
    {
        ctx.file
    } else {
        crate::resolve::mods::find_mod(w, w.file(target).stamp.module.as_deref(), parent_path)?
    };
    let edge = w
        .file(parent)
        .mods()
        .into_iter()
        .find(|m| m.name() == name)?;
    Some(vec![loc_at(
        w.file(parent),
        edge.name_span().unwrap_or(edge.span()),
    )])
}

/// a type head's declaration location(s) — typeDefinition's
/// computed-head lane: the type name here was REACHED THROUGH A VALUE
/// (a member's declared type, a binding's inferred head, a module
/// let's), not spelled at the cursor, so the file-scope law does not
/// gate it — the use graph restricts the search (the pkg the name is
/// imported from, else the flat world), and `None` when the name has
/// no indexed decl anywhere (a bare miss, kept distinct from an empty
/// hit list). Spelled type tokens answer through the file scope
/// (layer 4 — [`file_scope`](crate::world::collection)).
fn ty_name_locations(
    ctx: &Ctx,
    imported: &HashMap<&str, &str>,
    name: &str,
) -> Option<Vec<DefLocation>> {
    let scoped = chain_for(ctx.w, ctx.file, imported, name);
    let hits: Vec<(&File, Span)> = scoped
        .iter()
        .filter_map(|f| f.ty(name).map(|t| (*f, ty_span(&t))))
        .collect();
    // the body over the decl surface (`prefer_bodies`), so a
    // both-files dep's type lands in the body here too
    let hits = crate::world::collection::prefer_bodies(&hits);
    match hits.as_slice() {
        [] => None,
        _ => Some(hits.iter().map(|(f, sp)| loc_at(f, *sp)).collect()),
    }
}

/// where an imported name declares — strictly inside the named pkg's
/// files AND inside the walked mod path (`use pouch::a::b::{ C }`
/// resolves C only in pouch's module `a/b`; a walk the world cannot
/// satisfy falls back to the flat pkg — better a broad answer than
/// none, the `chain_for` law). Only `pub` rows bind a cross-package
/// leaf (the compiler's leaf gate — a private or `pub(pkg)` row the
/// use line refuses), and the queried file itself never exports to
/// its own import (layer 1 speaks for it).
fn use_graph_targets<'a>(
    w: &'a World,
    file: FileId,
    pkg: &str,
    mod_path: Option<&str>,
    name: &str,
) -> Vec<(&'a File, Span)> {
    let in_scope: Vec<FileId> = match mod_path {
        Some(p) => {
            let scoped: Vec<FileId> = w
                .chain(file)
                .filter(|id| {
                    matches_pkg(w.file(*id), pkg)
                        && crate::resolve::mods::mod_path_of(w.file(*id)) == p
                })
                .collect();
            if scoped.is_empty() {
                w.chain(file)
                    .filter(|id| matches_pkg(w.file(*id), pkg))
                    .collect()
            } else {
                scoped
            }
        }
        None => w
            .chain(file)
            .filter(|id| matches_pkg(w.file(*id), pkg))
            .collect(),
    };
    let mut out: Vec<(&File, Span)> = Vec::new();
    for id in in_scope {
        if id == file {
            continue;
        }
        let f = w.file(id);
        if let Some(t) = f.ty(name).filter(|t| t.vis() == rut_ast::Vis::Pub) {
            out.push((f, ty_span(&t)));
        }
        if let Some(l) = f
            .lets()
            .into_iter()
            .find(|l| l.name() == name && l.vis() == rut_ast::Vis::Pub)
        {
            out.push((f, l.name_span().unwrap_or(l.span())));
        }
        for fv in f
            .fns()
            .into_iter()
            .filter(|f| f.name() == name && f.owner().is_none() && f.vis() == rut_ast::Vis::Pub)
        {
            out.push((f, fn_span(&fv)));
        }
    }
    // the body over the decl surface (`prefer_bodies`): a pkg indexing
    // both its `.d.rut` and its `mod.rut` resolves the leaf from the
    // body — the surface is the fallback when it is all there is
    crate::world::collection::prefer_bodies(&out)
}

/// the pkg's ROOT file — where a pkg-head click lands. Among the
/// world's `matches_pkg` files only the root-shaped ones count: the
/// lib root `mod.rut` (`mod_path == Some("")` — how a dep walk /
/// bundle loader stamps the root) first, then the flat roots
/// (`mod_path == None` — how a host `calc.d.rut` and a flat dep's
/// root file mount), tie-broken body-over-decl (the
/// `prefer_bodies` law). Equals once more tie-broken toward the
/// MOUNTED closure: a scheme'd origin (`file://`, `bundle:`) is a real
/// artifact this workspace pinned, the scheme-less one an ambient
/// embed answering for a name nothing mounted — the mount wins, the
/// embed is the fallback. No root-shaped file → `None` — an honest
/// miss, never a wrong jump.
fn pkg_root_index(w: &World, file: FileId, pkg: &str) -> Option<FileId> {
    // the queried file is no package's root by a path coincidence (a
    // document that IS the root rides the own lane before this door)
    let mut roots: Vec<FileId> = w
        .chain(file)
        .filter(|id| {
            *id != file
                && matches_pkg(w.file(*id), pkg)
                && crate::resolve::mods::mod_path_of(w.file(*id)).is_empty()
        })
        .collect();
    roots.sort_by_key(|id| {
        let f = w.file(*id);
        (
            f.stamp.mod_path.is_none(),
            f.stamp.mode == crate::semantic::Mode::Decl,
            !f.stamp.origin.contains("://"),
        )
    });
    roots.into_iter().next()
}

/// the use-statement layer: the cursor rides one of the use node's own
/// recorded spans — a leaf name (jump to the exporting decl through the
/// walked mod path), the pkg head (jump to the pkg's root file at the
/// file top), or a middle path segment (jump to the `mod NAME;` edge
/// that mounts it). An OWN package head — the doc's own pkg name, or
/// one of its own mod edges (the ancestor-walk spelling) — commits to
/// the intra-package lanes: the head opens the resolved module's file
/// top, a middle segment lands on its mounting edge, and a dead end is
/// an honest empty, never a foreign fallback.
fn use_site_targets(ctx: &Ctx, h: rut_ast::AnyHandle, pos: u32) -> Option<Vec<DefLocation>> {
    let uh = h.downcast::<rut_ast::UseNode>()?;
    let w = ctx.w;
    let file = ctx.file;
    let doc = ctx.doc_file();
    let ast = doc.doc.ast();
    let u_node = uh.get(ast);
    let segs: Vec<String> = u_node
        .path
        .iter()
        .map(|n| ast.name(n.ident).to_string())
        .collect();
    // the leaf name
    for n in &u_node.names {
        if !on_name(n.span, pos) {
            continue;
        }
        let name = ast.name(n.ident);
        // the one-name refusal: a leaf the file binds twice is a use
        // line the compiler refuses — it jumps nowhere (the bare-name
        // lanes answer nothing for it either)
        if crate::world::collection::one_name_refused(w, file, name) {
            return Some(Vec::new());
        }
        let pkg = segs[0].clone();
        // the OWN lane — the walk is the intra-package one (any
        // declared edge; the doc's own rows read for its own scope)
        if let Some(start) = own_lane_start(ctx, &pkg) {
            let walked = crate::resolve::mods::position_walk(
                w,
                w.file(file).stamp.module.as_deref(),
                file,
                &start,
                &segs[1..],
            );
            return Some(match walked {
                Some(p) => own_leaf_targets(ctx, &p, name),
                None => Vec::new(),
            });
        }
        let walked = if u_node.path.len() == 1 {
            Some(String::new())
        } else {
            crate::resolve::mods::use_walk(w, &pkg, file, &segs[1..])
        };
        let targets = use_graph_targets(w, file, &pkg, walked.as_deref(), name);
        if !targets.is_empty() {
            return Some(targets.into_iter().map(|(f, sp)| loc_at(f, sp)).collect());
        }
        // a MODULE leaf (`use calc::{ math }`): the walked module's
        // `pub` child opens at its file top — the pkg-head convention
        if let Some(p) = walked {
            if let Some((child, _)) = crate::resolve::mods::use_module_leaf(w, &pkg, &p, name) {
                return Some(vec![loc_at(w.file(child), Span::new(0, 0))]);
            }
        }
        return Some(Vec::new());
    }
    // a path segment — the pkg head included: `k == 0` opens the
    // pkg's ROOT file at the file top (`Span::new(0, 0)`, the
    // convention a pkg-root row uses)
    for (k, seg) in u_node.path.iter().enumerate() {
        if !on_name(seg.span, pos) {
            continue;
        }
        let pkg = segs[0].clone();
        // the OWN lane — `k == 0` opens the resolved module's file
        // top (the root for the pkg name, the child file for an own
        // mod edge), deeper segments the mounting edge
        if let Some(start) = own_lane_start(ctx, &pkg) {
            return Some(if k == 0 {
                match own_scope(ctx, &start) {
                    Some(f) => vec![loc_at(w.file(f), Span::new(0, 0))],
                    None => Vec::new(),
                }
            } else {
                match crate::resolve::mods::position_walk(
                    w,
                    w.file(file).stamp.module.as_deref(),
                    file,
                    &start,
                    &segs[1..=k],
                ) {
                    Some(p) => own_edge_target(ctx, &p)
                        .map(|l| vec![l])
                        .unwrap_or_default(),
                    None => Vec::new(),
                }
            });
        }
        if k == 0 {
            return Some(
                pkg_root_index(w, file, &pkg)
                    .map(|f| vec![loc_at(w.file(f), Span::new(0, 0))])
                    .unwrap_or_default(),
            );
        }
        let walked = crate::resolve::mods::use_walk(w, &pkg, file, &segs[1..=k])?;
        let parent =
            crate::resolve::mods::find_mod(w, Some(&pkg), rut_lir::check::parent_of(&walked))?;
        let edge_name = ast.name(u_node.path[k].ident);
        let edge = w
            .file(parent)
            .mods()
            .into_iter()
            .find(|m| m.name() == edge_name)?;
        return Some(vec![loc_at(
            w.file(parent),
            edge.name_span().unwrap_or(edge.span()),
        )]);
    }
    None
}

/// the OWN-package lanes' start path for a use head, when the head
/// speaks for the doc's own package at all: the package's own name
/// starts at the root module, any other spelling resolves through the
/// position head walk (the doc's own rows first, then its ancestors —
/// the compiler's bare-head law). `None` = a foreign head; the caller
/// rides the cross-package lanes.
fn own_lane_start(ctx: &Ctx, head: &str) -> Option<String> {
    crate::resolve::mods::position_head(ctx.w, ctx.file, head)
}

/// the scope file for a resolved own mod path — the queried file's
/// own entry when the path IS its own module, else the world file
/// stamped for the `(pkg, path)` pair. `None` = nothing speaks for
/// the path (an unmounted module — the callers answer empty).
fn own_scope(ctx: &Ctx, walked: &str) -> Option<FileId> {
    let w = ctx.w;
    if w.file(ctx.file).stamp.mod_path.as_deref() == Some(walked) {
        return Some(ctx.file);
    }
    crate::resolve::mods::find_mod(w, w.file(ctx.file).stamp.module.as_deref(), walked)
}

/// the own-lane leaf: the walked mod path's scope answers the
/// declaring type / module-let / free-fn spans (the use-graph's
/// collector, scoped to the one module) — each row VISIBLE from the
/// importing file (`rut_lir::check::vis_crosses`: "import is no
/// visibility bypass" —
/// a leaf the use line refuses jumps nowhere), the module's `pub use`
/// rows never followed (the compiler's binding refuses a re-export
/// import)
fn own_leaf_targets(ctx: &Ctx, walked: &str, name: &str) -> Vec<DefLocation> {
    let Some(f) = own_scope(ctx, walked) else {
        return Vec::new();
    };
    let w = ctx.w;
    let from = w.file(ctx.file).stamp.mod_path.as_deref().unwrap_or("");
    let allows = |vis: rut_ast::Vis| rut_lir::check::vis_crosses(from, walked, vis);
    let mut hits: Vec<(&File, Span)> = Vec::new();
    let scope = w.file(f);
    if let Some(t) = scope.ty(name).filter(|t| allows(t.vis())) {
        hits.push((scope, ty_span(&t)));
    }
    if let Some(l) = scope
        .lets()
        .into_iter()
        .find(|l| l.name() == name && allows(l.vis()))
    {
        hits.push((scope, l.name_span().unwrap_or(l.span())));
    }
    for fv in scope
        .fns()
        .into_iter()
        .filter(|f| f.name() == name && f.owner().is_none() && allows(f.vis()))
    {
        hits.push((scope, fn_span(&fv)));
    }
    hits.into_iter().map(|(f, sp)| loc_at(f, sp)).collect()
}

/// the own-lane interior segment: the walked path's mounting `mod
/// NAME;` edge in the parent module's file — the foreign lane's
/// target law, keyed on the own package
fn own_edge_target(ctx: &Ctx, walked: &str) -> Option<DefLocation> {
    let parent_path = rut_lir::check::parent_of(walked);
    let name = walked.rsplit('/').next()?;
    let parent = own_scope(ctx, parent_path)?;
    let edge = ctx
        .w
        .file(parent)
        .mods()
        .into_iter()
        .find(|m| m.name() == name)?;
    Some(loc_at(
        ctx.w.file(parent),
        edge.name_span().unwrap_or(edge.span()),
    ))
}

/// the `mod NAME;` edge click — the cursor rides one of the doc's own
/// edges' recorded name spans: the child's mod path is
/// `child_path(doc's mod path, NAME)` and the jump opens the mounted
/// child at its file top (`Span::new(0, 0)`, the pkg-head convention).
/// Honest misses: an unstamped doc answers nothing; a child the world
/// does not carry an empty jump; the `(pkg, mod_path)` pair keys the
/// match — another package's same-path child never leaks in.
fn mod_edge_target(ctx: &Ctx, h: rut_ast::AnyHandle, pos: u32) -> Option<DefLocation> {
    let mh = h.downcast::<rut_ast::ModDeclNode>()?;
    let w = ctx.w;
    let doc = ctx.doc_file();
    let ast = doc.doc.ast();
    let m = mh.get(ast);
    on_name(m.name.span, pos).then_some(())?;
    let name = ast.name(m.name.ident);
    let from = w.file(ctx.file).stamp.mod_path.as_deref()?;
    let child = rut_lir::check::child_path(from, name);
    let mounted =
        crate::resolve::mods::find_mod(w, w.file(ctx.file).stamp.module.as_deref(), &child)?;
    Some(loc_at(w.file(mounted), Span::new(0, 0)))
}

fn member_locations(target: &MemberTarget) -> Vec<DefLocation> {
    match target {
        MemberTarget::Member(ty, m, _)
        | MemberTarget::EnumMember(ty, m)
        | MemberTarget::IfaceMember(ty, m) => match m.name_span() {
            Some(sp) => vec![loc_at(ty.file, sp)],
            // a member whose name span is missing on a degenerate parse
            // has no span to jump to — an empty answer, never a wrong
            // jump
            None => Vec::new(),
        },
        MemberTarget::ImplFn(f) => vec![loc_at(f.file, fn_span(f))],
    }
}
