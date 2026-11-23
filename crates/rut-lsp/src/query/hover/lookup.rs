//! Lookup — what's under the cursor, answered by the generic walk:
//! `node_at` → `ancestors` → `downcast`. The cursor's node names the
//! lane: a decl node at its own name position (field, enum member,
//! module let, `mod` edge), a use edge (its own path segments), a
//! member node (the door derives member + receiver from the node), a
//! path's tail segment, a capitalized/primitive name, a local binding
//! (`Doc::bindings()` — read internally), a module let, or a unique fn
//! name. The token residue lives in `Doc::name_at` — no token scans
//! here.

use super::render::{
    binding_markdown, render_candidates, render_fn_hits, render_let, render_member, render_mod,
    render_module_target, render_primitive, render_ty,
};
use crate::semantic::Span;
use crate::world::collection::{
    enclosing_type, Chain, FnView, MemberHit, MemberTarget, ModView, MountedFile, TyView,
};
use crate::world::infer::is_cap;
use crate::world::mods::DocMods;
use crate::world::types::TyForm;
use rut_ast::{
    AnyHandle, EnumMemberNode, FieldDeclNode, FieldNode, MethodNode, ModDeclNode, ModuleLetNode,
    ParamNode, PathNode, SelfNode, SelfTyNode, UseNode,
};

/// the name-position convention the old token scan kept: a name ends
/// its own position (the cursor sitting on the ident's last byte still
/// reads it)
fn on_name(sp: Span, pos: u32) -> bool {
    sp.lo <= pos && pos <= sp.hi
}

/// The hover result: markdown body + the span it decorates.
pub struct HoverOut {
    pub markdown: String,
    pub span: Span,
}

/// The chain is the world: the open document first (its fresh entry),
/// then the ambient prelude, then the rest of the workspace. `doc` is
/// the open document's file-module context (the face's knowledge): an
/// own-package use-path segment hover resolves through it, the default
/// keeps the cross-package lanes only.
pub fn hover(chain: &Chain, doc: &DocMods, pos: u32) -> Option<HoverOut> {
    let d = chain.head();
    let ast = d.doc.ast();
    // the cursor's node — the byte under it, else the byte just before
    // (a cursor sitting on a token's end reads that token — the old
    // scan's boundary convention)
    let h = ast
        .node_at(pos)
        .or_else(|| pos.checked_sub(1).and_then(|p| ast.node_at(p)))?;
    // the fence residue: a garbage region names nothing
    if h.is::<rut_ast::ErrorNode>() {
        return None;
    }

    // the receiver/type spellings — the tree's own kinds (the
    // enclosing-type walk is HOME-scoped: the only file whose spans
    // index the cursor's offsets)
    if let Some(t) = h.downcast::<SelfTyNode>() {
        let sd = t.get(ast);
        let ty = enclosing_type(std::iter::once(d), sd.span.lo)?;
        return Some(HoverOut {
            markdown: render_ty(&ty),
            span: sd.span,
        });
    }
    if let Some(s) = h.downcast::<SelfNode>() {
        let sd = s.get(ast);
        if sd.is_ty {
            return None; // the `Self` value spelling names no value
        }
        let ty = enclosing_type(std::iter::once(d), sd.span.lo)?;
        return Some(HoverOut {
            markdown: render_ty(&ty),
            span: sd.span,
        });
    }

    // decl sites — the decl node whose NAME position the cursor rides:
    // a field decl, an enum member, a module let, or a `mod NAME;`
    // namespace edge (a use site never matches). Rendering is
    // TREE-LOCAL: `ast.parent(decl)` is the owning type, and the
    // head/docs/verbatim read from the nodes + `text(span)` joins.
    if let Some(md_span) = decl_site_hover(chain, h, pos) {
        return Some(HoverOut {
            markdown: md_span.0,
            span: md_span.1,
        });
    }

    // use-path segments — the cursor rides one of the document's `use
    // pkg::a::b::{ .. }` edges' own path spans: hover the resolved
    // module through the dep tree (a miss is no hover)
    if let Some(md_span) = use_segment_hover(chain, doc, h, pos) {
        return Some(HoverOut {
            markdown: md_span.0,
            span: md_span.1,
        });
    }

    // member position: the walk found a member node and the cursor
    // rides its member (the door derives member + receiver + receiver
    // type INSIDE). A resolved receiver never falls through to the
    // name search: a miss there is a use-gated interface member or a
    // genuine miss — either way the member answer is final.
    if member_position(ast, h, pos) {
        match chain.member(pos, h, doc) {
            MemberHit::Found(target) => {
                return Some(HoverOut {
                    markdown: render_target(target),
                    span: node_span(ast, h, pos),
                })
            }
            MemberHit::None => return None,
            MemberHit::UnknownReceiver => {} // fall through to the name search
        }
    }

    // the name lanes key on the node's own recorded name — a path
    // segment, a param name, a local let's name. The (id, span) pair
    // threads through: the span the lanes decorate IS the mapped name
    // span (one name_at read per hover)
    let (name_id, name_span) = name_at(ast, h, pos)?;
    let name = ast.name(name_id);
    {
        // the FILE SCOPE's answers (the one home — the flat doors
        // delegate to `Chain::file_scope`): a capitalized name
        // resolves through the queried file's own decls, its use
        // edges, and the ambient prelude — a name the file never
        // binds answers nothing below, never a foreign decl
        if is_cap(name) || crate::semantic::is_primitive_ty(name) {
            if let Some(ty) = chain.type_decl(name) {
                return Some(HoverOut {
                    markdown: render_ty(&ty),
                    span: name_span,
                });
            }
            // M7 — the numeric/bool primitives have no surface decl to
            // index; the static blurb is the answer
            if let Some(md) = render_primitive(name) {
                return Some(HoverOut {
                    markdown: md,
                    span: name_span,
                });
            }
        }

        // identifier hover — the binding layer's shadow-correct resolve
        // (params, lets, loop variables; decl and use sites alike)
        if let Some(md) = binding_hover(chain, pos, name_id) {
            return Some(HoverOut {
                markdown: md,
                span: name_span,
            });
        }

        // module-let use sites — the file scope's unique match
        if let Some(md) = module_let_hover(chain, name) {
            return Some(HoverOut {
                markdown: md,
                span: name_span,
            });
        }

        // bare name: the file scope's fn match (free fns, methods);
        // the scope bound the name to more than one of its own fns →
        // the candidates list (the mid-edit/legal-ambiguity path)
        let hits: Vec<FnView> = chain.functions(name);
        match hits.as_slice() {
            [] => {}
            [one] => {
                return Some(HoverOut {
                    markdown: render_fn_hits(one),
                    span: name_span,
                })
            }
            _ => {
                return Some(HoverOut {
                    markdown: render_candidates(&hits),
                    span: name_span,
                })
            }
        }
    }
    None
}

/// the deepest node's own recorded name position — the interner id AND
/// the exact name span the hover decorates: a path segment, a param
/// name, a local let's name (destructured members included), a
/// struct-literal label, a use leaf, a type slot's segment, or a decl
/// node's declared name. The name lanes and the hover span read this
/// ONE mapping, so the decoration is always the hovered token (the
/// pre-phase `t.span` law), never the decl's whole extent.
fn name_at(ast: &rut_ast::Ast, h: AnyHandle, pos: u32) -> Option<(rut_ast::IdentId, Span)> {
    // member access (`recv.m(...)`, `recv.f`): the cursor rides the
    // member token — the recorded `name_span`/`member()` span — which
    // is what the hover decorates (the tuple-index spelling `.0` rides
    // the int token's own span)
    if let Some(m) = h.downcast::<MethodNode>() {
        let d = m.get(ast);
        return d
            .member
            .filter(|n| n.span.lo <= pos && pos <= n.span.hi)
            .map(|n| (n.ident, n.span));
    }
    if let Some(f) = h.downcast::<FieldNode>() {
        let d = f.get(ast);
        return on_name(d.name.span, pos).then_some((d.name.ident, d.name.span));
    }
    if let Some(p) = h.downcast::<PathNode>() {
        for s in &p.get(ast).segs {
            if on_name(s.name.span, pos) {
                return Some((s.name.ident, s.name.span));
            }
        }
        return None;
    }
    if let Some(p) = h.downcast::<ParamNode>() {
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
    // a use edge's LEAF names are name positions (the imported name
    // resolves through the chain like any use site)
    if let Some(u) = h.downcast::<UseNode>() {
        let d = u.get(ast);
        for n in &d.names {
            if on_name(n.span, pos) {
                return Some((n.ident, n.span));
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
    // the decl nodes: the cursor on a declared NAME is a name position
    // (the type/fn/let/mod lanes answer through the chain)
    macro_rules! decl_name {
        ($t:ty) => {
            if let Some(d) = h.downcast::<$t>() {
                let dd = d.get(ast);
                return on_name(dd.name.span, pos).then_some((dd.name.ident, dd.name.span));
            }
        };
    }
    decl_name!(rut_ast::ClassNode);
    decl_name!(rut_ast::StructNode);
    decl_name!(rut_ast::InterfaceNode);
    decl_name!(rut_ast::EnumNode);
    decl_name!(rut_ast::BuiltinTyNode);
    decl_name!(rut_ast::BuiltinPrimitiveNode);
    decl_name!(rut_ast::SurfaceStructNode);
    decl_name!(rut_ast::AliasNode);
    decl_name!(rut_ast::MethodDeclNode);
    decl_name!(rut_ast::FnNode);
    decl_name!(rut_ast::ModDeclNode);
    decl_name!(rut_ast::ModuleLetNode);
    None
}

/// the span the hover decorates — the node's own name position when it
/// records one (the pre-phase `t.span` law), else the node's span
fn node_span(ast: &rut_ast::Ast, h: AnyHandle, pos: u32) -> Span {
    name_at(ast, h, pos).map_or_else(|| ast.span_of(h), |(_, sp)| sp)
}

/// is the cursor on a member lane's node at its member position?
fn member_position(ast: &rut_ast::Ast, h: AnyHandle, pos: u32) -> bool {
    if let Some(m) = h.downcast::<rut_ast::MethodNode>() {
        let d = m.get(ast);
        return d
            .member
            .is_some_and(|n| n.span.lo <= pos && pos <= n.span.hi);
    }
    if let Some(f) = h.downcast::<rut_ast::FieldNode>() {
        let d = f.get(ast);
        return d.name.span.lo <= pos && pos <= d.name.span.hi;
    }
    if let Some(p) = h.downcast::<PathNode>() {
        let segs = &p.get(ast).segs;
        if segs.len() >= 2 {
            if let Some(i) = segs
                .iter()
                .position(|s| s.name.span.lo <= pos && pos <= s.name.span.hi)
            {
                return i + 1 == segs.len();
            }
        }
    }
    false
}

/// a local binding's hover — the shadow-correct resolve, the type
/// declared-or-inferred (the resolver reads the doc's bindings)
fn binding_hover(chain: &Chain<'_>, pos: u32, id: rut_ast::IdentId) -> Option<String> {
    let d = chain.head();
    // the shadow-correct resolve — or the DECL site: the cursor rides a
    // row's declaring ident (a param head sits before its scope body,
    // so the scope-containment resolve can't see it; definition's
    // decl→itself law, mirrored for the hover)
    let b = rut_ast::resolve_binding(d.doc.bindings(), pos, id).or_else(|| {
        d.doc.bindings().iter().find(|b| {
            b.name == id && {
                let sp = b.decl_ident_span;
                sp.lo <= pos && pos <= sp.hi
            }
        })
    })?;
    let ty = chain.binding_ty(b);
    let name = d.doc.ast().name(b.name);
    Some(binding_markdown(b, name, ty))
}

/// the decl node at the cursor — the name-position downcasts, the
/// owning type read off `ast.parent` (tree-local rendering, no chain
/// sweep). Returns the rendered markdown + the hover span.
fn decl_site_hover(chain: &Chain<'_>, h: AnyHandle, pos: u32) -> Option<(String, Span)> {
    let d = chain.head();
    let ast = d.doc.ast();
    if let Some(fh) = h.downcast::<FieldDeclNode>() {
        let fd = fh.get(ast);
        if on_name(fd.name.span, pos) {
            // the owning type: the decl's parent — a type body node
            let parent = ast.parent(h)?;
            let t = TyView { file: d, h: parent };
            let md = match t.form() {
                TyForm::Enum => render_ty(&t),
                _ => render_member(
                    &crate::world::collection::MemberView { file: d, ty: t, h },
                    None,
                ),
            };
            return Some((md, fd.name.span));
        }
        return None;
    }
    if let Some(eh) = h.downcast::<EnumMemberNode>() {
        let ed = eh.get(ast);
        if on_name(ed.name.span, pos) {
            // `Color::Red`'s decl shows the enum, matching the use site
            let parent = ast.parent(h)?;
            let t = TyView { file: d, h: parent };
            return Some((render_ty(&t), ed.name.span));
        }
        return None;
    }
    if let Some(lh) = h.downcast::<ModuleLetNode>() {
        let ld = lh.get(ast);
        if on_name(ld.name.span, pos) {
            let l = crate::world::collection::LetView { file: d, h };
            return Some((render_let(&l), ld.name.span));
        }
        return None;
    }
    if let Some(mh) = h.downcast::<ModDeclNode>() {
        let md_node = mh.get(ast);
        if on_name(md_node.name.span, pos) {
            let m = ModView { file: d, h };
            let child = crate::world::mods::child_path(
                crate::world::mods::mod_path_of(d),
                ast.name(md_node.name.ident),
            );
            let mounted = mounted_child(chain, d, &child);
            return Some((
                render_mod(
                    d,
                    &m,
                    &crate::world::mods::display_of(chain, d.mount.module.as_deref(), &child),
                    mounted.map(|c| c.mods().len()),
                ),
                md_node.name.span,
            ));
        }
    }
    None
}

/// the chain file a mod edge's child resolves to — the (module,
/// mod_path) pair when the owner is named; an unnamed owner (an open
/// workspace file) answers any file stamped at the child's mod path
/// (the dep walk's twins — display only: the child count)
fn mounted_child<'a>(
    chain: &'a Chain<'a>,
    owner: &MountedFile,
    child: &str,
) -> Option<&'a MountedFile> {
    match crate::world::mods::find_mod(chain, owner.mount.module.as_deref(), child) {
        Some(c) => Some(c),
        None if owner.mount.module.is_none() => chain
            .iter()
            .find(|i| crate::world::mods::mod_path_of(i) == child),
        None => None,
    }
}

/// a use-path SEGMENT hover: the cursor rides one of the use node's own
/// path segs (the node's recorded `path` spans — no sweep) — the
/// resolved module (walked through the pub edges; the pkg head answers
/// the root module) renders kind + mounted file + child count.
///
/// An OWN package head — the doc's own pkg name, or one of its own
/// mod edges — commits to the intra-package walk (any declared edge;
/// the doc's own rows read for its own scope). A non-walking path is
/// no hover, never wrong text.
fn use_segment_hover(
    chain: &Chain<'_>,
    doc: &DocMods,
    h: AnyHandle,
    pos: u32,
) -> Option<(String, Span)> {
    let uh = h.downcast::<UseNode>()?;
    let di = chain.head();
    let ast = di.doc.ast();
    let u_node = uh.get(ast);
    let segs: Vec<String> = u_node
        .path
        .iter()
        .map(|n| ast.name(n.ident).to_string())
        .collect();
    for (k, seg) in u_node.path.iter().enumerate() {
        if !on_name(seg.span, pos) {
            continue;
        }
        let pkg = segs[0].clone();
        // the OWN lane — committed once the head speaks for the
        // doc's own package: `k == 0` answers the resolved start
        // module, deeper segments walk intra-package
        if let Some(start) = crate::world::mods::resolve_position_head(doc, di, chain, &pkg) {
            let path = if k == 0 {
                start
            } else {
                crate::world::mods::walk_position_mods(
                    chain,
                    doc.pkg.as_deref(),
                    &start,
                    &segs[1..=k],
                )?
            };
            let target = if doc.mod_path.as_deref() == Some(path.as_str()) {
                di
            } else {
                crate::world::mods::find_mod(chain, doc.pkg.as_deref(), &path)?
            };
            return Some((render_module_target(target, &path), seg.span));
        }
        let walked = if k == 0 {
            Some(String::new())
        } else {
            crate::world::mods::walk_use_mods(chain, &pkg, &segs[1..=k])
        };
        let path = walked?;
        let target = crate::world::mods::find_mod(chain, Some(&pkg), &path)?;
        return Some((render_module_target(target, &path), seg.span));
    }
    None
}

/// module-let use sites: the queried file's unique match renders the
/// decl; ambiguity stays a miss (the file scope's honesty rule)
fn module_let_hover(chain: &Chain<'_>, name: &str) -> Option<String> {
    chain.module_let(name).map(|l| render_let(&l))
}

/// the markdown a resolved member target renders to (hover's half of
/// the shared resolver; definition uses the target's spans instead)
pub(crate) fn render_target(t: MemberTarget) -> String {
    match t {
        MemberTarget::Member(_, m, via) => render_member(&m, via),
        MemberTarget::EnumMember(ty, _) => render_ty(&ty),
        MemberTarget::ImplFn(f) => render_fn_hits(&f),
        MemberTarget::IfaceMember(_, m) => render_member(&m, None),
    }
}
