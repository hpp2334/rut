//! The member law's resolver family — where a member of a receiver
//! actually declares. One rule for every host: hover, definition, and
//! completion share [`member`] and the doors beneath it (the receiver
//! resolver, the bindings read internally, the own-surface → enum →
//! inherent-impl → interface gates with the use-both rule).
//! [`MemberHit`] is the verdict shape every consumer matches on.

use std::collections::HashSet;

use rut_ast::{
    AnyHandle, ExprRef, FieldNode, IdentId, Lit, MethodDeclNode, MethodNode, PathNode, UseNode, Vis,
};

use super::{FnView, MemberView, TyView};
use crate::semantic::Span;
use crate::world::bindings::{BindKind, Binding};
use crate::world::infer::QualHit;
use crate::world::types::TyForm;
use crate::world::{File, FileId, World};

/// a member door's receiver — the node-derived spelling: the receiver
/// expression (a method/field access), or the `::` path segment
/// immediately before the member
#[derive(Clone, Copy)]
pub(crate) enum RecvNode {
    Expr(ExprRef),
    Seg(IdentId),
}

/// THE MEMBER DOOR — where a member of the receiver actually
/// declares, gated by the member-vis law. Doors consume NODES, not
/// derived facts: the caller hands the node the cursor's walk found
/// (a method call, a field access, or a `::` path's tail segment);
/// the member + receiver derive INSIDE, the receiver's type
/// resolves INSIDE (self → the enclosing type, literals →
/// primitives, paths → the bindings — read internally, never a
/// caller-run pass — then module lets), and the member law (own
/// surface → enum members → inherent impl methods → interface
/// signatures) with every gate (the use-both rule, then the
/// member-vis dial against the queried file — the site context)
/// lives here. One rule for every host — hover, definition, and
/// completion share it. The signature-help lane rides
/// [`impl_method_fn`] UNGATED: a
/// call site spelling a refused member is already a panel error.
pub(crate) fn member<'a>(w: &'a World, file: FileId, pos: u32, node: AnyHandle) -> MemberHit<'a> {
    match member_hit(w, file, pos, node) {
        // the member-vis gate: a resolved target the site cannot
        // see is a MISS — hover/definition agree with the compile
        MemberHit::Found(target) if !target_visible(w, file, &target) => MemberHit::None,
        hit => hit,
    }
}

/// the ungated resolve — the member law's walk (the gated
/// [`member`] wraps this)
fn member_hit<'a>(w: &'a World, file: FileId, pos: u32, node: AnyHandle) -> MemberHit<'a> {
    let ast = w.file(file).doc.ast();
    // the member + the receiver, from the node itself — the
    // cursor's own segment decides for a path (the head segment is
    // a name position, never a member lane)
    let derived = member_at(w, file, pos, node);
    let Some((member, recv)) = derived else {
        return MemberHit::UnknownReceiver;
    };
    let Some(member) = member else {
        // the incomplete shape (`recv.` at the typing edge): the
        // member is being typed — nothing to resolve yet
        return MemberHit::UnknownReceiver;
    };
    let member_text = ast.name(member);
    // the receiver's type head: the bindings' resolve, else the
    // expression door (a chained receiver — `m::mk(41).bump()` —
    // types through its call's written ret, the same door the
    // completion tier rides)
    let Some(ty_name) = recv_ty_name(w, file, pos, recv).or_else(|| match recv {
        RecvNode::Expr(e) => expr_ty(w, file, e).map(|t| crate::world::types::head_of_ty(&t)),
        _ => None,
    }) else {
        return MemberHit::UnknownReceiver;
    };
    // an interface receiver: the declared signatures ARE the members —
    // satisfaction is structural (a type qualifies by having the
    // members), so no impl lookup rides this path. The use-both gate:
    // a foreign interface's members resolve only when this document
    // names the interface in a `use`; an interface declared here is
    // in scope natively
    if let Some(iface) = iface_decl(w, file, &ty_name) {
        if !std::ptr::eq(iface.file, w.file(file))
            && !used_ifaces(w.file(file)).contains(iface.name())
        {
            return MemberHit::None;
        }
        if let Some(m) = iface
            .methods()
            .into_iter()
            .find(|m| m.name() == member_text)
        {
            return MemberHit::Found(MemberTarget::IfaceMember(iface, m));
        }
        return MemberHit::None;
    }
    // the receiver's type head is a COMPUTED string (the inferred
    // `RequestBuilder` behind `let b = builder()`), not a spelling
    // the file binds — the file scope answers first, the doc's
    // import pkgs the fallback ([`type_via_imports`])
    let ty = match super::type_decl(w, file, &ty_name)
        .or_else(|| super::type_via_imports(w, file, &ty_name))
    {
        Some(t) => t,
        None => return MemberHit::UnknownReceiver,
    };
    // own surface first
    if let Some(m) = ty.methods().into_iter().find(|m| m.name() == member_text) {
        return MemberHit::Found(MemberTarget::Member(ty, m, None));
    }
    if ty.form() == TyForm::Enum {
        // `Color::Red` — the member IS an enum member; hover shows the enum
        if let Some(m) = ty.fields().into_iter().find(|f| f.name() == member_text) {
            return MemberHit::Found(MemberTarget::EnumMember(ty, m));
        }
        return MemberHit::None;
    }
    if let Some(m) = ty.fields().into_iter().find(|f| f.name() == member_text) {
        return MemberHit::Found(MemberTarget::Member(ty, m, None));
    }
    // inherent impl-block methods — where methods live since type
    // bodies went fields-only; the owner string IS the target
    // type's name. The mounted-over-embed law rides
    // ([`type_decl`]'s): the world may carry the same owner twice —
    // the dep walk's real origins and the ambient surface answering
    // for a name nothing mounted — and the mount wins
    // the sweep stays inside the resolved type's OWN package (an
    // inherent impl lives in its type's module — a same-named type
    // in another package owns its own methods, never these)
    let ty_pkg = ty.file.stamp.module.as_deref();
    let mut hits: Vec<(&File, FnView<'a>)> = Vec::new();
    for id in w.chain(file) {
        let f = w.file(id);
        if f.stamp.module.as_deref() != ty_pkg {
            continue;
        }
        for fv in f.fns() {
            if fv.name() == member_text && fv.owner().as_deref() == Some(ty_name.as_str()) {
                hits.push((f, fv));
            }
        }
    }
    // the mounted-over-embed pick over the same hits (one home)
    match super::prefer_mounted(&hits).into_iter().next() {
        Some(f) => MemberHit::Found(MemberTarget::ImplFn(f)),
        None => MemberHit::None,
    }
}

/// the resolved target's half of the member-vis gate — the same
/// law the completion tier applies per row: enum variants are the
/// enum lane's rows (the compiler never dials them) and interface
/// signatures already answered behind the use-both gate; struct +
/// host-struct fields and the builtin/primitive surfaces carry no
/// dial (the compiler's field law has no struct dial; the surfaces
/// are compiler-lowered). Class members and impl fns gate.
fn target_visible(w: &World, file: FileId, target: &MemberTarget<'_>) -> bool {
    match target {
        MemberTarget::EnumMember(..) | MemberTarget::IfaceMember(..) => true,
        MemberTarget::Member(ty, m, _) => match ty.form() {
            TyForm::Class => member_dial_allows(w, file, m.file, m.vis()),
            _ => true,
        },
        MemberTarget::ImplFn(f) => member_dial_allows(w, file, f.file, f.vis()),
    }
}

/// the member dial — may the queried file see a member declared
/// in `decl` with visibility `vis`? The site context is the
/// queried file's own stamp: cross-package — the decl file names a
/// package the site is not in, and a `None` site pkg is foreign to
/// every named pkg — only `pub` passes, the compiler's carried-row
/// strictness; same package (the unnamed files included) the
/// `vis_crosses` predicate verbatim, already the position-path
/// tier's gate.
pub(crate) fn member_dial_allows(w: &World, site: FileId, decl_file: &File, vis: Vis) -> bool {
    if let Some(p) = decl_file.stamp.module.as_deref() {
        if w.file(site).stamp.module.as_deref() != Some(p) {
            return vis == Vis::Pub;
        }
    }
    crate::world::mods::vis_allows(
        w.file(site).stamp.mod_path.as_deref().unwrap_or(""),
        crate::world::mods::mod_path_of(decl_file),
        vis,
    )
}

/// the (member, receiver) pair a member-position node carries —
/// `None` when the node is not a member position or the cursor is
/// not on its member segment (a path's head is a name position)
pub(crate) fn member_at(
    w: &World,
    file: FileId,
    pos: u32,
    node: AnyHandle,
) -> Option<(Option<IdentId>, RecvNode)> {
    let ast = w.file(file).doc.ast();
    if let Some(m) = node.downcast::<MethodNode>() {
        let d = m.get(ast);
        // the cursor rides the member name (the token-scan's
        // boundary convention: a name ends its own position) — or
        // the shape is the TYPING EDGE (absent member, `recv.`):
        // a member position with nothing resolved yet
        return match d.member {
            None => Some((None, RecvNode::Expr(d.recv))),
            Some(n) => {
                let on_member = n.span.lo <= pos && pos <= n.span.hi;
                on_member.then_some((Some(n.ident), RecvNode::Expr(d.recv)))
            }
        };
    }
    if let Some(f) = node.downcast::<FieldNode>() {
        let d = f.get(ast);
        let on_member = d.name.span.lo <= pos && pos <= d.name.span.hi;
        return if on_member {
            Some((Some(d.name.ident), RecvNode::Expr(d.recv)))
        } else {
            None
        };
    }
    if let Some(p) = node.downcast::<PathNode>() {
        let segs = &p.get(ast).segs;
        if segs.len() >= 2 {
            if let Some(i) = segs
                .iter()
                .position(|s| s.name.span.lo <= pos && pos <= s.name.span.hi)
            {
                if i + 1 == segs.len() {
                    // the tail segment IS the member lane
                    return Some((
                        Some(segs[i].name.ident),
                        RecvNode::Seg(segs[i - 1].name.ident),
                    ));
                }
            }
        }
    }
    None
}

/// the receiver's type name — the node-derived spelling: the
/// receiver value (`self` → the enclosing type), a path segment
/// (capitalized and primitive names spell themselves, locals ride
/// the bindings — read internally, never a caller-run pass — then
/// module lets)
pub(crate) fn recv_ty_name(w: &World, file: FileId, pos: u32, recv: RecvNode) -> Option<String> {
    let ast = w.file(file).doc.ast();
    match recv {
        RecvNode::Expr(e) => match e {
            ExprRef::Self_(h) if !h.get(ast).is_ty => {
                // the enclosing type lives in the HOME file — the
                // only file whose spans index the cursor's
                // offsets (a foreign world file's byte spans are
                // its own text's coordinates and would steal the
                // answer whenever they overlap)
                enclosing_type(std::iter::once(w.file(file)), pos).map(|t| t.name().to_string())
            }
            ExprRef::Path(ph) => {
                let segs = &ph.get(ast).segs;
                if segs.len() != 1 {
                    return None;
                }
                named_recv(w, file, pos, segs[0].name.ident)
            }
            ExprRef::Lit(h) => match h.get(ast).lit {
                // every string-literal form is a `str` receiver
                Lit::Str(_) | Lit::RawStr(_) => Some("str".to_string()),
                _ => None,
            },
            ExprRef::FStr(_) => Some("str".to_string()),
            _ => None,
        },
        RecvNode::Seg(id) => named_recv(w, file, pos, id),
    }
}

/// a plain name receiver — capitalized and primitive spellings
/// name themselves; anything else is the bindings' resolve (the
/// declared-or-inferred head) then the module lets. A QUALIFIED
/// annotation (`c: m::Counter`) resolves through the TYPE
/// ladder's qualified lane to its declaring view; the view's bare
/// name then reaches the type through the computed-head lane
/// ([`type_via_imports`](super::type_via_imports))
pub(crate) fn named_recv(w: &World, file: FileId, pos: u32, id: IdentId) -> Option<String> {
    let doc = w.file(file);
    let text = doc.doc.ast().name(id);
    if crate::world::infer::is_cap(text) || crate::semantic::is_primitive_ty(text) {
        return Some(text.to_string());
    }
    let b = rut_ast::resolve_binding(doc.doc.bindings(), pos, id)?;
    binding_ty(w, file, b)
        .map(|t| {
            qualified_ty_name(w, file, &t).unwrap_or_else(|| crate::world::types::head_of_ty(&t))
        })
        .or_else(|| {
            let files: Vec<&File> = w.chain(file).map(|id| w.file(id)).collect();
            crate::world::infer::module_let_ty(files, text)
        })
}

/// a qualified TYPE annotation's resolution — `m::Counter` as
/// written resolves through the TYPE ladder's qualified lane (the
/// own family — `resolve_qualified_ty`'s law) to its declaring
/// view; the view's bare name is the type head the member lane
/// then resolves. `None` when the spelling is unqualified or the
/// lane misses — `head_of_ty` keeps the old shape then.
pub(crate) fn qualified_ty_name(w: &World, file: FileId, text: &str) -> Option<String> {
    if !text.contains("::") {
        return None;
    }
    let mut segs: Vec<String> = text.split("::").map(|s| s.trim().to_string()).collect();
    // the tail's generic args ride the substitution, not the walk
    // — the lane matches the DECLARED name
    if let Some(last) = segs.last_mut() {
        if let Some(i) = last.find('<') {
            *last = last[..i].to_string();
        }
    }
    super::qualified_scope(w, file, &segs, true)?
        .types
        .first()
        .map(|t| t.name().to_string())
}

/// a binding row's type — declared-or-inferred, the world as the
/// inference window (the hover/tooltip text for a local). A loop
/// variable's iterated ELEMENT is the answer (`[Point]` iterates
/// `Point`), the survey's for-of rule
pub(crate) fn binding_ty(w: &World, file: FileId, b: &Binding) -> Option<String> {
    let head = w.file(file);
    if let Some(sp) = b.ty_span {
        return Some(head.doc.text(sp).to_string());
    }
    // a destructured row takes its TUPLE element positionally
    if let (Some(i), Some(ExprRef::Tuple(th))) = (b.destructure_index, b.init) {
        let ast = head.doc.ast();
        if let Some(&elem) = th.get(ast).elems.get(i) {
            return expr_ty(w, file, elem);
        }
    }
    let ty = b.init.and_then(|init| expr_ty(w, file, init))?;
    if matches!(b.kind, BindKind::IterVar) {
        if let Some(elem) = crate::world::infer::elem_ty(&ty) {
            return Some(elem);
        }
    }
    Some(ty)
}

/// the resolver family's expression door — the display-side type
/// of `e`, the world as the file window (the receiver resolver
/// generalized: literals → primitives, paths → the bindings (read
/// internally) and module let, calls → the callee's written ret,
/// member chains → the member lanes). The hook rides the value
/// ladder with the queried file's OWN stamp — the
/// ancestor's edge and the own-pkg head resolve exactly as the
/// spelling lanes resolve them: a module-QUALIFIED callee
/// (`m::mk(..)`) resolves through the module-namespace lane (the
/// one home —
/// [`qualified_scope`](super::qualified_scope)) to its fn, and a BARE
/// callee through the file scope (the one home —
/// [`functions`](super::functions): what the doc binds, never the flat
/// unique-match a same-named fn elsewhere would break); the
/// written ret prices the call either way
pub(crate) fn expr_ty<'a>(w: &'a World, file: FileId, e: ExprRef) -> Option<String> {
    let files: Vec<&'a File> = w.chain(file).map(|id| w.file(id)).collect();
    let qual = |segs: &[String], is_ty: bool| -> Option<QualHit<'a>> {
        if !is_ty && segs.len() == 1 {
            return super::functions(w, file, &segs[0])
                .into_iter()
                .find(|f| f.owner().is_none())
                .map(QualHit::Fn);
        }
        let scope = super::qualified_scope(w, file, segs, is_ty)?;
        Some(if is_ty {
            QualHit::Ty(scope.types.first()?.name().to_string())
        } else {
            QualHit::Fn(scope.fns.into_iter().next()?)
        })
    };
    crate::world::infer::expr_ty(w.file(file), &files, e, &qual)
}

/// the inherent impl-block fn `impl ty_name { fn member(..) }` —
/// the callee door's method half (the old `impl_method_fn` rule's
/// one home). Own-surface methods (interface bodies,
/// builtin/primitive surfaces) have no recorded params: a hit there
/// is FINAL (the known-receiver-is-final law) and yields `None`,
/// never a wrong signature from a same-named impl fn in another
/// file.
pub(crate) fn impl_method_fn<'a>(
    w: &'a World,
    file: FileId,
    ty_name: &str,
    member: &str,
) -> Option<FnView<'a>> {
    for id in w.chain(file) {
        let i = w.file(id);
        if let Some(t) = i.ty(ty_name) {
            if t.methods().iter().any(|m| m.name() == member) {
                return None;
            }
        }
        for f in i.fns() {
            if f.name() == member && f.owner().as_deref() == Some(ty_name) {
                return Some(f);
            }
        }
    }
    None
}

/// the class's designated constructor — `impl Logger {
/// [constructor] pub fn new(..) -> Self }` — the ctor hover /
/// signature-help lanes' shared door. The sweep is the RESOLVED
/// class's OWN FILE's, never a name-keyed flat sweep over the
/// world (the compiler's inherent-impl rule: an inherent impl must
/// live in its class's own module — module ↔ file is 1:1 — so a
/// same-named class in another package can never donate a ctor,
/// and a cross-module `[constructor]` impl is NOT a ctor of that
/// class). First hit in the file's own source order (the checker
/// enforces one `[constructor]` per class). A newtype never
/// answers here — the checker refuses the marker on a newtype
/// (the engine's `Ctx::class_ctors` law: a newtype never
/// registers; its call surface already IS the mint), so the
/// newtype arm never rides this sweep.
pub(crate) fn constructor_of(ty: TyView<'_>) -> Option<FnView<'_>> {
    let ty_name = ty.name();
    let ast = ty.file.doc.ast();
    for f in ty.file.fns() {
        if f.owner().as_deref() != Some(ty_name) {
            continue;
        }
        if let Some(m) = f.h.downcast::<MethodDeclNode>() {
            if m.get(ast).marker() == Some(rut_core::sym::CONSTRUCTOR_MARKER) {
                return Some(f);
            }
        }
    }
    None
}

/// the interface named `name` — the file scope's answer
/// ([`type_decl`](super::type_decl)) with the computed-head fallback (a
/// value-reached receiver's interface), cut to the interface form
/// (the declaration and the observing receiver may live in
/// different files). The use-both gate reads this answer — a
/// foreign interface resolves only once the document names it.
pub(crate) fn iface_decl<'a>(w: &'a World, file: FileId, name: &str) -> Option<TyView<'a>> {
    super::type_decl(w, file, name)
        .or_else(|| super::type_via_imports(w, file, name))
        .filter(|t| t.form() == TyForm::Interface)
}

// ---- the enclosing-type walk (the self-receiver's resolver) ----

/// the type whose body contains `pos` — a class/struct/interface body,
/// or the target of the enclosing impl (via the method's owner). Over
/// any file window (the queried file alone or the inference pass's
/// slice — one law, two shapes).
pub(crate) fn enclosing_type<'a, I>(files: I, pos: u32) -> Option<TyView<'a>>
where
    I: IntoIterator<Item = &'a File>,
{
    let mut best: Option<(u32, TyView<'a>)> = None;
    for f in files {
        for t in f.types() {
            if t.form() == TyForm::Enum || !contains(t.span(), pos) {
                continue;
            }
            if best
                .map(|(l, _)| t.span().hi - t.span().lo < l)
                .unwrap_or(true)
            {
                best = Some((t.span().hi - t.span().lo, t));
            }
        }
        for fv in f.fns() {
            // the impl's target — the owner string IS the type's name
            // (inherent impls own their methods)
            let Some(target) = fv.owner() else {
                continue;
            };
            if let Some(t) = f.ty(&target) {
                if contains(fv.span(), pos)
                    && best
                        .map(|(l, _)| fv.span().hi - fv.span().lo < l)
                        .unwrap_or(true)
                {
                    best = Some((fv.span().hi - fv.span().lo, t));
                }
            }
        }
    }
    best.map(|(_, t)| t)
}

/// is `pos` inside `sp`? — the span-containment predicate the
/// enclosing-type walk reads
fn contains(sp: Span, pos: u32) -> bool {
    sp.lo <= pos && pos < sp.hi
}

// ---- the member law's verdict types (the shared resolver's shape) ----

/// The member-lookup verdict: found (a resolved declaration target), a
/// final miss (the receiver's type is known — the gate or a genuine
/// miss), or an unknown receiver (the caller may fall through to the
/// bare-name search).
pub(crate) enum MemberHit<'a> {
    Found(MemberTarget<'a>),
    None,
    UnknownReceiver,
}

/// where a member of `recv` actually declares — the shared resolver
/// behind hover AND definition
pub(crate) enum MemberTarget<'a> {
    /// a method/field of the type's own surface (`via` renders a
    /// qualifying member's provenance when it came through a chain)
    Member(TyView<'a>, MemberView<'a>, Option<String>),
    /// an enum member — hover shows the whole enum's block
    EnumMember(TyView<'a>, MemberView<'a>),
    /// an impl-block method (`impl T { fn m(self) ... }`)
    ImplFn(FnView<'a>),
    /// an interface's declared member, observed through the interface
    /// (the receiver's static type IS the interface)
    IfaceMember(TyView<'a>, MemberView<'a>),
}

/// interface names the open document `use`s — the use-both gate's
/// document side: a foreign interface's members resolve only when its
/// name appears here; an interface declared in this document is in
/// scope natively. Derived from the head doc's own use edges.
pub(crate) fn used_ifaces(doc_head: &File) -> HashSet<String> {
    let ast = doc_head.doc.ast();
    let mut out = HashSet::new();
    for &h in doc_head.use_handles() {
        let Some(u) = h.downcast::<UseNode>() else {
            continue;
        };
        let d = u.get(ast);
        out.extend(d.names.iter().map(|n| ast.name(n.ident).to_string()));
    }
    out
}
