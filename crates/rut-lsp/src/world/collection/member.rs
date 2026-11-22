//! The member law's resolver family — where a member of a receiver
//! actually declares. One rule for every host: hover, definition, and
//! completion share [`Chain::member`] and the doors beneath it (the
//! receiver resolver, the bindings read internally, the own-surface →
//! enum → inherent-impl → interface gates with the use-both rule).
//! [`MemberHit`] is the verdict shape every consumer matches on.

use std::collections::HashSet;

use rut_ast::{AnyHandle, ExprRef, FieldNode, IdentId, Lit, MethodNode, PathNode, UseNode, Vis};

use super::{Chain, FnView, MemberView, MountedFile, TyView};
use crate::semantic::Span;
use crate::world::bindings::{BindKind, Binding};
use crate::world::mods::{self, DocMods};
use crate::world::types::TyForm;

/// a member door's receiver — the node-derived spelling: the receiver
/// expression (a method/field access), or the `::` path segment
/// immediately before the member
#[derive(Clone, Copy)]
pub(crate) enum RecvNode {
    Expr(ExprRef),
    Seg(IdentId),
}

impl<'a> Chain<'a> {
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
    /// member-vis dial against `doc` — the site context, the
    /// established [`DocMods`] carrier) lives here. One rule for every
    /// host — hover, definition, and completion share it. The
    /// signature-help lane rides [`Chain::impl_method_fn`] UNGATED: a
    /// call site spelling a refused member is already a panel error.
    pub(crate) fn member(&self, pos: u32, node: AnyHandle, doc: &DocMods) -> MemberHit<'a> {
        match self.member_hit(pos, node) {
            // the member-vis gate: a resolved target the site cannot
            // see is a MISS — hover/definition agree with the compile
            MemberHit::Found(target) if !self.target_visible(&target, doc) => MemberHit::None,
            hit => hit,
        }
    }

    /// the ungated resolve — the member law's walk (the gated
    /// [`Chain::member`] wraps this)
    fn member_hit(&self, pos: u32, node: AnyHandle) -> MemberHit<'a> {
        let ast = self.head().doc.ast();
        // the member + the receiver, from the node itself — the
        // cursor's own segment decides for a path (the head segment is
        // a name position, never a member lane)
        let derived = self.member_at(pos, node);
        let Some((member, recv)) = derived else {
            return MemberHit::UnknownReceiver;
        };
        let Some(member) = member else {
            // the incomplete shape (`recv.` at the typing edge): the
            // member is being typed — nothing to resolve yet
            return MemberHit::UnknownReceiver;
        };
        let member_text = ast.name(member);
        let Some(ty_name) = self.recv_ty_name(pos, recv) else {
            return MemberHit::UnknownReceiver;
        };
        // an interface receiver: the declared signatures ARE the members —
        // satisfaction is structural (a type qualifies by having the
        // members), so no impl lookup rides this path. The use-both gate:
        // a foreign interface's members resolve only when this document
        // names the interface in a `use`; an interface declared here is
        // in scope natively
        if let Some(iface) = self.iface_decl(&ty_name) {
            if !std::ptr::eq(iface.file, self.head())
                && !used_ifaces(self.head()).contains(iface.name())
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
        let Some(ty) = self.type_decl(&ty_name) else {
            return MemberHit::UnknownReceiver;
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
        // type's name. The mounted-over-embed law rides (`type_decl`'s):
        // the chain may carry the same owner twice — the dep walk's
        // real origins and the ambient surface answering for a name
        // nothing mounted — and the mount wins
        let mut fallback: Option<FnView<'a>> = None;
        for f in self.iter() {
            for f in f.fns() {
                if f.name() == member_text && f.owner().as_deref() == Some(ty_name.as_str()) {
                    if f.file.mount.src_path.is_none() {
                        return MemberHit::Found(MemberTarget::ImplFn(f));
                    }
                    if fallback.is_none() {
                        fallback = Some(f);
                    }
                }
            }
        }
        match fallback {
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
    fn target_visible(&self, target: &MemberTarget<'a>, doc: &DocMods) -> bool {
        match target {
            MemberTarget::EnumMember(..) | MemberTarget::IfaceMember(..) => true,
            MemberTarget::Member(ty, m, _) => match ty.form() {
                TyForm::Class => self.member_dial_allows(doc, m.file, m.vis()),
                _ => true,
            },
            MemberTarget::ImplFn(f) => self.member_dial_allows(doc, f.file, f.vis()),
        }
    }

    /// the member dial — may the OPEN document see a member declared
    /// in `decl_file` with visibility `vis`? The site context rides
    /// the established [`DocMods`] carrier (the chain head's own mount
    /// is unstamped; `state.rs doc_mods_of` reads the stamped twin):
    /// cross-package — the decl file names a package the site is not
    /// in, and a `None` site pkg is foreign to every named pkg — only
    /// `pub` passes, the compiler's carried-row strictness; same
    /// package (the unnamed files included) the `vis_crosses`
    /// predicate verbatim, already the position-path tier's gate.
    pub(crate) fn member_dial_allows(
        &self,
        doc: &DocMods,
        decl_file: &MountedFile,
        vis: Vis,
    ) -> bool {
        if let Some(p) = decl_file.mount.module.as_deref() {
            if doc.pkg.as_deref() != Some(p) {
                return vis == Vis::Pub;
            }
        }
        mods::vis_allows(
            doc.mod_path.as_deref().unwrap_or(""),
            mods::mod_path_of(decl_file),
            vis,
        )
    }

    /// the (member, receiver) pair a member-position node carries —
    /// `None` when the node is not a member position or the cursor is
    /// not on its member segment (a path's head is a name position)
    pub(super) fn member_at(
        &self,
        pos: u32,
        node: AnyHandle,
    ) -> Option<(Option<IdentId>, RecvNode)> {
        let ast = self.head().doc.ast();
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
    pub(crate) fn recv_ty_name(&self, pos: u32, recv: RecvNode) -> Option<String> {
        let ast = self.head().doc.ast();
        match recv {
            RecvNode::Expr(e) => match e {
                ExprRef::Self_(h) if !h.get(ast).is_ty => {
                    // the enclosing type lives in the HOME file — the
                    // only file whose spans index the cursor's
                    // offsets (a foreign chain file's byte spans are
                    // its own text's coordinates and would steal the
                    // answer whenever they overlap)
                    enclosing_type(std::iter::once(self.head()), pos).map(|t| t.name().to_string())
                }
                ExprRef::Path(ph) => {
                    let segs = &ph.get(ast).segs;
                    if segs.len() != 1 {
                        return None;
                    }
                    self.named_recv(pos, segs[0].name.ident)
                }
                ExprRef::Lit(h) => match h.get(ast).lit {
                    // every string-literal form is a `str` receiver
                    Lit::Str(_) | Lit::RawStr(_) => Some("str".to_string()),
                    _ => None,
                },
                ExprRef::FStr(_) => Some("str".to_string()),
                _ => None,
            },
            RecvNode::Seg(id) => self.named_recv(pos, id),
        }
    }

    /// a plain name receiver — capitalized and primitive spellings
    /// name themselves; anything else is the bindings' resolve (the
    /// declared-or-inferred head) then the module lets
    pub(crate) fn named_recv(&self, pos: u32, id: IdentId) -> Option<String> {
        let ast = self.head().doc.ast();
        let text = ast.name(id);
        if crate::world::infer::is_cap(text) || crate::semantic::is_primitive_ty(text) {
            return Some(text.to_string());
        }
        let b = rut_ast::resolve_binding(self.head().doc.bindings(), pos, id)?;
        self.binding_ty(b)
            .map(|t| crate::world::types::head_of_ty(&t))
            .or_else(|| crate::world::infer::module_let_ty(self.iter(), text))
    }

    /// a binding row's type — declared-or-inferred, the chain as the
    /// inference window (the hover/tooltip text for a local). A loop
    /// variable's iterated ELEMENT is the answer (`[Point]` iterates
    /// `Point`), the survey's for-of rule
    pub(crate) fn binding_ty(&self, b: &Binding) -> Option<String> {
        let head = self.head();
        if let Some(sp) = b.ty_span {
            return Some(head.doc.text(sp).to_string());
        }
        // a destructured row takes its TUPLE element positionally
        if let (Some(i), Some(ExprRef::Tuple(th))) = (b.destructure_index, b.init) {
            let ast = head.doc.ast();
            if let Some(&elem) = th.get(ast).elems.get(i) {
                return self.expr_ty(head, elem);
            }
        }
        let ty = b.init.and_then(|init| self.expr_ty(head, init))?;
        if matches!(b.kind, BindKind::IterVar) {
            if let Some(elem) = crate::world::infer::elem_ty(&ty) {
                return Some(elem);
            }
        }
        Some(ty)
    }

    /// the resolver family's expression door — the display-side type
    /// of `e`, the chain as the file window (the receiver resolver
    /// generalized: literals → primitives, paths → the bindings (read
    /// internally) and module lets, calls → the callee's written ret,
    /// member chains → the member lanes)
    pub(crate) fn expr_ty(&self, home: &MountedFile, e: ExprRef) -> Option<String> {
        let files: Vec<&MountedFile> = self.iter().collect();
        crate::world::infer::expr_ty(home, &files, e)
    }

    /// the inherent impl-block fn `impl ty_name { fn member(..) }` —
    /// the callee door's method half (the old `impl_method_fn` rule's
    /// one home). Own-surface methods (interface bodies,
    /// builtin/primitive surfaces) have no recorded params: a hit there
    /// is FINAL (the known-receiver-is-final law) and yields `None`,
    /// never a wrong signature from a same-named impl fn in another
    /// file.
    pub(crate) fn impl_method_fn(&self, ty_name: &str, member: &str) -> Option<FnView<'a>> {
        for i in self.iter() {
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

    /// the interface named `name` — [`type_decl`](Chain::type_decl)
    /// cut to the interface form (the declaration and the observing
    /// receiver may live in different files)
    pub(crate) fn iface_decl(&self, name: &str) -> Option<TyView<'a>> {
        self.type_decl(name)
            .filter(|t| t.form() == TyForm::Interface)
    }
}

// ---- the enclosing-type walk (the self-receiver's resolver) ----

/// the type whose body contains `pos` — a class/struct/interface body,
/// or the target of the enclosing impl (via the method's owner). Over
/// any file window (the chain's iteration or the inference pass's
/// slice — one law, two shapes).
pub(crate) fn enclosing_type<'a, I>(files: I, pos: u32) -> Option<TyView<'a>>
where
    I: IntoIterator<Item = &'a MountedFile>,
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
pub(crate) fn used_ifaces(doc_head: &MountedFile) -> HashSet<String> {
    let ast = doc_head.doc.ast();
    let mut out = HashSet::new();
    for &h in &doc_head.decls.uses {
        let Some(u) = h.downcast::<UseNode>() else {
            continue;
        };
        let d = u.get(ast);
        out.extend(d.names.iter().map(|n| ast.name(n.ident).to_string()));
    }
    out
}
