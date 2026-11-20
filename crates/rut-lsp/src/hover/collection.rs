//! The mounted-file store — every file in the session's world is a
//! [`MountedFile`]: the document (the kept tree), its provenance stamp,
//! and the decl map (name → node address, built once per file version).
//! The [`DefIndexCollection`] owns them and answers the cross-file
//! questions through typed views over `(Doc, AnyHandle)`: a view's
//! verbatim text is a span×src join on the OWNING file's document, its
//! line number is `doc.line_of`, its `ambient`/vis verdict derives from
//! the node's modifier chain — nothing stored, nothing stale.
//!
//! The doors are questions with one home each: [`DefIndexCollection::type_decl`]
//! carries the mounted-over-embed preference, [`DefIndexCollection::functions`]
//! the 0/1/many honesty rule, [`DefIndexCollection::module_let`] the
//! unique-or-none rule, and [`DefIndexCollection::member`] the member
//! law (receiver resolver, own-surface/enum/impl/interface gates). The
//! body-over-decl preference lives once in [`DefIndexCollection::prefer_bodies`].

use std::collections::HashSet;

use rut_ast::{
    AliasNode, AnyHandle, Ast, BuiltinPrimitiveNode, BuiltinTyNode, ClassNode, DocData,
    EnumMemberNode, EnumNode, ExprRef, FieldDeclNode, FieldNode, FieldRef, FnNode, Handle, IdentId,
    ImplNode, InterfaceNode, ItemRef, Lit, MemberRef, MethodDeclNode, MethodNode, MethodRef,
    ModDeclNode, ModuleLetNode, ParamRef, PathNode, StructNode, SurfaceFnNode, SurfaceStructNode,
    UseNode, Vis,
};

use crate::hover::bindings::{BindKind, Binding};
use crate::hover::types::{ty_head, ty_src, Mount, TyForm};
use crate::semantic::{mode_of, Document};

use super::infer;

/// a member door's receiver — the node-derived spelling: the receiver
/// expression (a method/field access), or the `::` path segment
/// immediately before the member
#[derive(Clone, Copy)]
pub(crate) enum RecvNode {
    Expr(ExprRef),
    Seg(IdentId),
}

// ---- the decl map — name → node address, one pass per file version ----

/// one file's decl inventory: the node ADDRESSES in source order, the
/// name lookups resolving through the interner at query time. Built
/// once per file version (at mount / at sync); views derive every
/// other fact on read.
#[derive(Debug, Default)]
pub(crate) struct DeclMap {
    /// type decls — class/struct/interface/enum/builtin/primitive/
    /// host-struct/alias, in source order
    types: Vec<AnyHandle>,
    /// fns — free fns, surface fns, and impl-block methods (an impl
    /// method's owner derives from its parent impl's target), in
    /// source order. `builtin impl` blocks record nothing: their
    /// methods are compiler-lowered, core's own surfaces answer.
    fns: Vec<AnyHandle>,
    /// module-scope lets
    lets: Vec<AnyHandle>,
    /// use edges (one row per imported name rides the view, not here)
    uses: Vec<AnyHandle>,
    /// `mod NAME;` namespace edges
    mods: Vec<AnyHandle>,
}

/// The decl map builder — the small pass that replaced the row
/// extractor: walk the root items once, record the node addresses.
/// Error slots render no rows (errors are the panel's business).
pub(crate) fn decls_of(doc: &Document) -> DeclMap {
    let ast = doc.ast();
    let mut m = DeclMap::default();
    for item in ast.root().items(ast) {
        match item {
            ItemRef::Class(h) => m.types.push(h.any()),
            ItemRef::Struct(h) => m.types.push(h.any()),
            ItemRef::Interface(h) => m.types.push(h.any()),
            ItemRef::Enum(h) => m.types.push(h.any()),
            ItemRef::BuiltinTy(h) => m.types.push(h.any()),
            ItemRef::BuiltinPrimitive(h) => m.types.push(h.any()),
            ItemRef::SurfaceStruct(h) => m.types.push(h.any()),
            ItemRef::Alias(h) => m.types.push(h.any()),
            ItemRef::Fn(h) => m.fns.push(h.any()),
            ItemRef::SurfaceFn(h) => m.fns.push(h.any()),
            // the inherent impl owns its methods — they ride the fns
            // list at the impl's own position; `BuiltinImpl` records
            // nothing (compiler-lowered)
            ItemRef::Impl(h) => {
                for meth in &h.get(ast).methods {
                    let MethodRef::Method(mh) = meth else {
                        continue; // the recovery slot renders no row
                    };
                    m.fns.push(mh.any());
                }
            }
            ItemRef::ModuleLet(h) => m.lets.push(h.any()),
            ItemRef::Use(h) => m.uses.push(h.any()),
            ItemRef::ModDecl(h) => m.mods.push(h.any()),
            ItemRef::Module(_) | ItemRef::BuiltinImpl(_) | ItemRef::Error(_) => {}
        }
    }
    m
}

// ---- the mounted file + the collection ----

/// One file in the session's world: the document (tree KEPT), the
/// provenance stamp, and the decl map built once per version. A plain
/// record: many construction sites (boot prelude, dep walk, workspace
/// scan, doc sync), no invariant beyond "decls describes doc".
pub(crate) struct MountedFile {
    /// the document — the pipeline owner (text, tokens, tree) the
    /// views join over
    pub(crate) doc: Document,
    /// the per-file stamp — origin/module/mod_path/mode, ONE record
    /// built at the mount site (shape unchanged from the row era)
    pub(crate) mount: Mount,
    /// the decl map — node addresses, built once per file version
    pub(crate) decls: DeclMap,
}

impl MountedFile {
    /// parse fixture/mounted source into a mounted file — the mode
    /// derives from the mount's stamp when it carries one, else the
    /// origin's filename convention (`mode_of`)
    pub(crate) fn from_src(mount: Mount, src: String) -> MountedFile {
        let mode = mount.mode.unwrap_or_else(|| mode_of(&mount.origin));
        MountedFile::from_document(Document::parse(mode, src), mount)
    }

    /// mount an already-parsed document (the sync lane's shape)
    pub(crate) fn from_document(doc: Document, mount: Mount) -> MountedFile {
        let decls = decls_of(&doc);
        MountedFile { doc, mount, decls }
    }

    /// the file's origin (the defs store's replace-by-origin key)
    pub(crate) fn origin(&self) -> &str {
        &self.mount.origin
    }

    // ---- per-file decl views, in source order ----

    pub(crate) fn types(&self) -> Vec<TyView<'_>> {
        self.decls
            .types
            .iter()
            .map(|&h| TyView { file: self, h })
            .collect()
    }

    pub(crate) fn fns(&self) -> Vec<FnView<'_>> {
        self.decls
            .fns
            .iter()
            .map(|&h| FnView { file: self, h })
            .collect()
    }

    pub(crate) fn lets(&self) -> Vec<LetView<'_>> {
        self.decls
            .lets
            .iter()
            .map(|&h| LetView { file: self, h })
            .collect()
    }

    /// use views — one per imported name (a brace list fans out, the
    /// names in order)
    pub(crate) fn uses(&self) -> Vec<UseView<'_>> {
        let mut out = Vec::new();
        for &h in &self.decls.uses {
            let Some(u) = h.downcast::<UseNode>() else {
                continue;
            };
            for i in 0..u.get(self.doc.ast()).names.len() {
                out.push(UseView { file: self, h, i });
            }
        }
        out
    }

    pub(crate) fn mods(&self) -> Vec<ModView<'_>> {
        self.decls
            .mods
            .iter()
            .map(|&h| ModView { file: self, h })
            .collect()
    }

    /// the type decl named `name` — first match in source order
    pub(crate) fn ty(&self, name: &str) -> Option<TyView<'_>> {
        self.types().into_iter().find(|t| t.name() == name)
    }

    /// the names whose decl spelled the import-gated builtin linkage
    /// (`pub builtin` — the core disposal pair): completion offers them
    /// only when the document's `use` names them, the same ambient
    /// split the compiler binds by. DERIVED at read from the modifier
    /// chains — never stored.
    pub(crate) fn pub_gated(&self) -> Vec<String> {
        let ast = self.doc.ast();
        let mut out = Vec::new();
        for &h in &self.decls.fns {
            if let Some(d) = h.downcast::<SurfaceFnNode>() {
                let d = d.get(ast);
                if matches!(d.linkage(), rut_ast::Linkage::Builtin { ambient: false }) {
                    out.push(ast.name(d.name).to_string());
                }
            }
        }
        for &h in &self.decls.types {
            if let Some(d) = h.downcast::<BuiltinTyNode>() {
                if !d.get(ast).ambient() {
                    out.push(ast.name(d.get(ast).name).to_string());
                }
            } else if let Some(d) = h.downcast::<BuiltinPrimitiveNode>() {
                if !d.get(ast).ambient() {
                    out.push(ast.name(d.get(ast).name).to_string());
                }
            }
        }
        out
    }
}

/// the session's mounted world — the files in chain order (the prelude
/// boots first, every mount replaces by origin in place), answering the
/// cross-file questions. Rebuilt on dep-walk mount — never per query.
/// The OPEN documents live in the session's versioned cache
/// (`SessionState.docs`, each a [`MountedFile`]); the query chain is
/// the two combined — the doc first, the world after.
pub(crate) struct DefIndexCollection {
    files: Vec<MountedFile>,
}

impl DefIndexCollection {
    /// the empty world — the boot lane mounts the prelude first
    pub(crate) fn new() -> DefIndexCollection {
        DefIndexCollection { files: Vec::new() }
    }

    /// the defs store's replace-by-origin law — one file per origin,
    /// re-mounting replaces (the world stays idempotent under re-walks)
    pub(crate) fn mount(&mut self, file: MountedFile) {
        match self.files.iter().position(|f| f.origin() == file.origin()) {
            Some(slot) => self.files[slot] = file,
            None => self.files.push(file),
        }
    }

    /// the file mounted at `origin`, if any
    pub(crate) fn file_for(&self, origin: &str) -> Option<&MountedFile> {
        self.files.iter().find(|f| f.origin() == origin)
    }

    /// the query chain — `origin`'s file at the head (the open
    /// document first), the rest of the world after. The chain is an
    /// ordering VIEW over the world's files: the open document's entry
    /// (whose home is the session's versioned cache) rides first, any
    /// stamped twin of the same uri keeps its place behind it (the
    /// definition layer's dedupe law handles the doubled answer).
    pub(crate) fn chain_with<'a>(&'a self, head: &'a MountedFile) -> Chain<'a> {
        let mut files = Vec::with_capacity(1 + self.files.len());
        files.push(head);
        files.extend(self.files.iter());
        Chain::new(files)
    }

    pub(crate) fn files(&self) -> &[MountedFile] {
        &self.files
    }
}

/// the query chain — an ORDERING VIEW over the world's files, the
/// queried document at the head. The head IS the open document (its
/// fresh entry, rebuilt on sync); queries iterate `iter()` exactly
/// where the old code iterated the `&[&DefIndex]` chain. The
/// collection's DOORS hang off the chain (one home per question).
pub(crate) struct Chain<'a> {
    files: Vec<&'a MountedFile>,
    /// the head's position — the open document
    head: usize,
}

impl<'a> Chain<'a> {
    /// the chain over the files, `files[head]` the open document
    pub(crate) fn new(files: Vec<&'a MountedFile>) -> Chain<'a> {
        assert!(!files.is_empty(), "a chain rides at least its head");
        Chain { files, head: 0 }
    }

    /// the open document's own file — the chain head
    pub(crate) fn head(&self) -> &'a MountedFile {
        self.files[self.head]
    }

    /// the whole chain, doc first — the old `idxs.iter()` shape
    pub(crate) fn iter(&self) -> impl Iterator<Item = &'a MountedFile> + Clone + '_ {
        let (head, tail) = self.files.split_at(self.head + 1);
        head.iter().copied().chain(tail.iter().copied())
    }

    /// the scan's per-candidate chain — `f` at the head, the rest of
    /// `base` after (no origin speaks twice; the references scan's law)
    pub(crate) fn reordered(base: &Chain<'a>, f: &'a MountedFile) -> Chain<'a> {
        let mut files: Vec<&'a MountedFile> = Vec::with_capacity(base.files.len());
        files.push(f);
        files.extend(
            base.files
                .iter()
                .copied()
                .filter(|x| x.mount.origin != f.mount.origin),
        );
        Chain { files, head: 0 }
    }

    // ---- the collection's doors ----

    /// the type named `name` — the mounted-over-embed preference
    /// inside: the chain may carry the same ty twice (the dep walk's
    /// real origins and the ambient prelude answering for a name
    /// nothing mounted), and the MOUNTED carrier wins (chain order
    /// keeps the doc freshest among equals); the embed is the
    /// zero-fetch fallback.
    pub(crate) fn type_decl(&self, name: &str) -> Option<TyView<'a>> {
        let mut fallback = None;
        for f in self.iter() {
            if let Some(t) = f.ty(name) {
                if f.mount.src_path.is_none() {
                    return Some(t);
                }
                if fallback.is_none() {
                    fallback = Some(t);
                }
            }
        }
        fallback
    }

    /// the fns named `name` across the chain — 0/1/many, the honesty
    /// rule typed (free fns and impl methods alike; a consumer that
    /// wants free fns only filters on `owner()`)
    pub(crate) fn functions(&self, name: &str) -> Vec<FnView<'a>> {
        self.iter()
            .flat_map(|f| f.fns())
            .filter(|f| f.name() == name)
            .collect()
    }

    /// the module let named `name` — unique-or-none inside (ambiguity
    /// stays a miss, the flat chain's honesty rule)
    pub(crate) fn module_let(&self, name: &str) -> Option<LetView<'a>> {
        let hits: Vec<LetView<'a>> = self
            .iter()
            .flat_map(|f| f.lets())
            .filter(|l| l.name() == name)
            .collect();
        match hits.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }

    /// THE MEMBER DOOR — where a member of the receiver actually
    /// declares. Doors consume NODES, not derived facts: the caller
    /// hands the node the cursor's walk found (a method call, a field
    /// access, or a `::` path's tail segment); the member + receiver
    /// derive INSIDE, the receiver's type resolves INSIDE (self → the
    /// enclosing type, literals → primitives, paths → the bindings —
    /// read internally, never a caller-run pass — then module lets),
    /// and the member law (own surface → enum members → inherent impl
    /// methods → interface signatures) with every gate (the use-both
    /// rule) lives here. One rule for every host — hover, definition,
    /// and completion share it.
    pub(crate) fn member(&self, pos: u32, node: AnyHandle) -> MemberHit<'a> {
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

    /// the (member, receiver) pair a member-position node carries —
    /// `None` when the node is not a member position or the cursor is
    /// not on its member segment (a path's head is a name position)
    fn member_at(&self, pos: u32, node: AnyHandle) -> Option<(Option<IdentId>, RecvNode)> {
        let ast = self.head().doc.ast();
        if let Some(m) = node.downcast::<MethodNode>() {
            let d = m.get(ast);
            // the cursor rides the member name (the token-scan's
            // boundary convention: a name ends its own position) — or
            // the shape is the TYPING EDGE (absent member, `recv.`):
            // a member position with nothing resolved yet
            return match d.member() {
                None => Some((None, RecvNode::Expr(d.recv))),
                Some((name, nsp)) => {
                    let on_member = nsp.lo <= pos && pos <= nsp.hi;
                    on_member.then_some((Some(name), RecvNode::Expr(d.recv)))
                }
            };
        }
        if let Some(f) = node.downcast::<FieldNode>() {
            let d = f.get(ast);
            let on_member = d.name_span.lo <= pos && pos <= d.name_span.hi;
            return if on_member {
                Some((Some(d.name), RecvNode::Expr(d.recv)))
            } else {
                None
            };
        }
        if let Some(p) = node.downcast::<PathNode>() {
            let segs = &p.get(ast).segs;
            if segs.len() >= 2 {
                if let Some(i) = segs
                    .iter()
                    .position(|s| s.name_span.lo <= pos && pos <= s.name_span.hi)
                {
                    if i + 1 == segs.len() {
                        // the tail segment IS the member lane
                        return Some((Some(segs[i].name), RecvNode::Seg(segs[i - 1].name)));
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
                    super::lookup::enclosing_type(self.iter(), pos).map(|t| t.name().to_string())
                }
                ExprRef::Path(ph) => {
                    let segs = &ph.get(ast).segs;
                    if segs.len() != 1 {
                        return None;
                    }
                    self.named_recv(pos, segs[0].name)
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
        if super::infer::is_cap(text) || crate::semantic::is_primitive_ty(text) {
            return Some(text.to_string());
        }
        let b = rut_ast::resolve_binding(self.head().doc.bindings(), pos, id)?;
        self.binding_ty(b)
            .map(|t| crate::hover::types::head_of_ty(&t))
            .or_else(|| super::infer::module_let_ty(self.iter(), text))
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
            if let Some(elem) = super::infer::elem_ty(&ty) {
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
        super::infer::expr_ty(home, &files, e)
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

    /// the interface named `name` — [`type_decl`](Self::type_decl) cut
    /// to the interface form (the declaration and the observing
    /// receiver may live in different files)
    pub(crate) fn iface_decl(&self, name: &str) -> Option<TyView<'a>> {
        self.type_decl(name)
            .filter(|t| t.form() == TyForm::Interface)
    }

    /// body-over-decl — THE single preference: a package that indexes
    /// BOTH its `.d.rut` surface (`mode == Some(Decl)`) and its
    /// `mod.rut` body (`Impl` — or any mount whose builder did not
    /// stamp) answers from the body; the surface hits ride only when
    /// they are all there is. One precedence above the body law: a
    /// MOUNTED dep outranks the ambient prelude — the prelude is the
    /// zero-fetch fast path for the ONE embedded pkg, a fallback, never
    /// the answer when the real thing is mounted (its `src_path` marks
    /// it). The generic payload keeps one law for every shape the
    /// layers collect (spans, views).
    pub(crate) fn prefer_bodies<T: Copy>(
        &self,
        hits: &[(&'a MountedFile, T)],
    ) -> Vec<(&'a MountedFile, T)> {
        DefIndexCollection::prefer_bodies(hits)
    }
}

impl DefIndexCollection {
    /// body-over-decl — the preference's one home (the chain's door
    /// delegates here)
    pub(crate) fn prefer_bodies<'a, T: Copy>(
        hits: &[(&'a MountedFile, T)],
    ) -> Vec<(&'a MountedFile, T)> {
        let mounted: Vec<(&MountedFile, T)> = hits
            .iter()
            .copied()
            .filter(|(f, _)| f.mount.src_path.is_none())
            .collect();
        let pool: &[(&MountedFile, T)] = if mounted.is_empty() { hits } else { &mounted };
        let bodies: Vec<(&MountedFile, T)> = pool
            .iter()
            .copied()
            .filter(|(f, _)| f.mount.mode != Some(crate::semantic::Mode::Decl))
            .collect();
        if bodies.is_empty() {
            pool.to_vec()
        } else {
            bodies
        }
    }
}

// ---- completion's tier doors — the folded contract ----
//
// Enumeration + GATES live here, keyed on the walk's NODE; prefix
// filtering and the wire mapping stay at the call site (presentation).
// The doors read `Doc::bindings()` internally where a receiver needs
// it; no caller runs a pass.

/// one member-tier row — the view the door found it from
#[derive(Clone, Copy)]
pub(crate) enum MemberRow<'a> {
    Field(MemberView<'a>),
    Method(MemberView<'a>),
    ImplFn(FnView<'a>),
}

/// one use-tier export row — `Mod` carries the child's mod path (the
/// detail renders the mounted display from it)
#[derive(Clone)]
pub(crate) enum ExportRow<'a> {
    Mod(ModView<'a>, String),
    Ty(TyView<'a>),
    Fn(FnView<'a>),
    Let(LetView<'a>),
    ReExport(UseView<'a>),
}

/// the use tier's answer: the package's module names (no walk yet),
/// or the walked module's exports (the pkg + resolved path ride along
/// for the rendering)
pub(crate) enum UseExports<'a> {
    Modules(Vec<(String, String)>),
    Exports {
        pkg: String,
        path: String,
        rows: Vec<ExportRow<'a>>,
    },
}

/// the bare tier's answer — the visible decls under the bare-name law
/// (keywords are the call site's vocabulary, not a collection fact)
pub(crate) struct Bare<'a> {
    pub types: Vec<TyView<'a>>,
    pub fns: Vec<FnView<'a>>,
}

impl<'a> Chain<'a> {
    /// completion's MEMBER tier — keyed on the walk's NODE: the
    /// member/receiver derive inside (a method call, a field access,
    /// or a `::` path's tail), the receiver resolves inside (the
    /// resolver family — the bindings read internally, never a
    /// caller-run pass), and the gates live here (the interface lane
    /// with the use-both rule; the enum-member lane; the inherent impl
    /// fns by owner). `None` = not a member position; an unresolved
    /// receiver is an EMPTY tier, never a wrong one. The typing edge's
    /// absent member (`recv.` mid-typing) enumerates the FULL list.
    pub(crate) fn members(&self, pos: u32, node: AnyHandle) -> Option<Vec<MemberRow<'a>>> {
        let (_, recv) = self.member_at(pos, node)?;
        self.members_for(pos, recv)
    }

    /// the member tier keyed on an already-derived receiver (the type
    /// slot's `.`/`::` spelling: the head is the slot's own segments,
    /// node-less data on the ty path). The member node's OWN name never
    /// restricts the list — the typed prefix (the call site's filter)
    /// is the only narrowing.
    pub(crate) fn members_for(&self, pos: u32, recv: RecvNode) -> Option<Vec<MemberRow<'a>>> {
        let ty_name = self.recv_ty_name(pos, recv).or_else(|| match recv {
            RecvNode::Expr(e) => self
                .expr_ty(self.head(), e)
                .map(|t| crate::hover::types::head_of_ty(&t)),
            _ => None,
        });
        let Some(ty_name) = ty_name else {
            return Some(Vec::new());
        };
        // an interface receiver: the declared signatures are the whole
        // member list — satisfaction is structural, so no impl lookup
        // rides along. The use-both gate: a foreign interface's
        // members complete only when this document names the interface
        // in a `use`; an interface declared here is in scope natively
        if let Some(iface) = self.iface_decl(&ty_name) {
            let mut rows: Vec<MemberRow<'a>> = Vec::new();
            if std::ptr::eq(iface.file, self.head())
                || used_ifaces(self.head()).contains(iface.name())
            {
                for m in iface.methods() {
                    rows.push(MemberRow::Method(m));
                }
            }
            return Some(rows);
        }
        let ty = self.type_decl(&ty_name)?;
        let mut rows: Vec<MemberRow<'a>> = Vec::new();
        for m in ty.fields() {
            rows.push(MemberRow::Field(m));
        }
        for m in ty.methods() {
            rows.push(MemberRow::Method(m));
        }
        // inherent impl-block methods — where methods live since type
        // bodies went fields-only; the owner string IS the target
        // type's name
        for i in self.iter() {
            for f in i.fns() {
                if f.owner().as_deref() == Some(ty_name.as_str()) {
                    rows.push(MemberRow::ImplFn(f));
                }
            }
        }
        Some(rows)
    }

    /// completion's USE tier — keyed on the walk's NODE (the use edge
    /// the cursor sits in): the walk derives from the node's own
    /// recorded spans (the segments BEFORE the cursor; a sync that ran
    /// past the incomplete leaf can leave later tokens in the path —
    /// the statement's recovery — and they are not the walk). The pub
    /// gates and the walk live inside. `None` = the cursor is not in a
    /// use statement.
    pub(crate) fn use_exports(&self, node: AnyHandle, pos: u32) -> Option<UseExports<'a>> {
        let uh = node.downcast::<UseNode>()?;
        let ast = self.head().doc.ast();
        let d = uh.get(ast);
        // the walk: the path BEFORE the segment the cursor rides (the
        // riding segment is the typed prefix, never a walk segment —
        // `use po⏐` is the module-NAME tier); nothing ridden → every
        // segment that closed at-or-before the cursor (`use
        // gadgets::layout::⏐` walks both)
        let before: Vec<String> = match d
            .path_spans
            .iter()
            .position(|sp| sp.lo <= pos && pos <= sp.hi)
        {
            Some(k) => d.path[..k]
                .iter()
                .map(|id| ast.name(*id).to_string())
                .collect(),
            None => d
                .path
                .iter()
                .zip(&d.path_spans)
                .filter(|(_, sp)| sp.hi <= pos)
                .map(|(id, _)| ast.name(*id).to_string())
                .collect(),
        };
        let Some(pkg) = before.first() else {
            // `use <cursor>`: the chain's module names — every NAMED
            // index (the names come from the uniform index set, no
            // hardcoded table)
            return Some(UseExports::Modules(
                self.iter()
                    .filter_map(|f| f.mount.module.clone().map(|m| (m, f.mount.origin.clone())))
                    .collect(),
            ));
        };
        // the import is the gate-opener, so the `pub builtin` gate
        // does not bind here (this IS the use that names them)
        let scoped: Vec<&MountedFile> = self
            .iter()
            .filter(|f| crate::query::definition::matches_pkg(f, pkg))
            .collect();
        if scoped.is_empty() {
            return Some(UseExports::Exports {
                pkg: pkg.clone(),
                path: String::new(),
                rows: Vec::new(),
            });
        }
        // resolve the walked module: the flat spelling answers the
        // root — for a mod-carrying package that is the ROOT file only
        // (members live under their module path), for a flat package
        // today's whole chain; deeper segments walk the pub edges (the
        // trailing PARTIAL ident is not a walk segment — `use
        // pkg::layout::Co⏐` walks `layout` only)
        let carrying = scoped.iter().any(|f| f.mount.mod_path.is_some());
        let interior: &[String] = &before[1..];
        let walked = if interior.is_empty() {
            Some(String::new())
        } else {
            crate::mods::walk_use_mods(self, pkg, interior)
        };
        let Some(path) = walked else {
            return Some(UseExports::Exports {
                pkg: pkg.clone(),
                path: String::new(),
                rows: Vec::new(),
            });
        };
        let scope: Vec<&MountedFile> = if carrying {
            scoped
                .into_iter()
                .filter(|f| crate::mods::mod_path_of(f) == path)
                .collect()
        } else {
            scoped
        };
        let mut rows: Vec<ExportRow<'a>> = Vec::new();
        for f in &scope {
            // the walkable children — pub edges only (cross-package law)
            for m in f.mods() {
                if m.vis() != Vis::Pub {
                    continue;
                }
                let child = crate::mods::child_path(&path, m.name());
                rows.push(ExportRow::Mod(m, child));
            }
            for t in f.types() {
                if t.is_pub() {
                    rows.push(ExportRow::Ty(t));
                }
            }
            for fv in f.fns() {
                // the walked path binds through the compiler's
                // qualified-use gate: only `pub` names cross — on
                // walked AND flat spellings alike (one gate, no
                // ungated legacy shape)
                if fv.owner().is_none() && fv.vis() == Vis::Pub {
                    rows.push(ExportRow::Fn(fv));
                }
            }
            for l in f.lets() {
                if l.vis() == Vis::Pub {
                    rows.push(ExportRow::Let(l));
                }
            }
            // a `pub use` re-export: the imported name joined the
            // module's surface — the tier's gate mirrors the fns/lets
            // above (a walked path binds only `pub` edges across
            // packages, flat spellings included)
            for u in f.uses() {
                if u.vis() == Vis::Pub {
                    rows.push(ExportRow::ReExport(u));
                }
            }
        }
        Some(UseExports::Exports {
            pkg: pkg.clone(),
            path,
            rows,
        })
    }

    /// completion's BARE tier — the visible decls under the compiler's
    /// own bare-name law: the open document's decls (the chain head),
    /// the chain's AMBIENT builtin rows (`prelude builtin` — bound in
    /// every unit, no `use` needed), and the names the document
    /// already `use`s (matched `(name, pkg)` against the head's uses —
    /// the same set shape the auto-import tier dedups against). Every
    /// other foreign name rides ONLY the auto-import tier: a plain
    /// offer the compile would reject is a trap. Free fns and types
    /// only; methods are reached through a receiver, not named bare.
    pub(crate) fn bare(&self) -> Bare<'a> {
        let doc = self.head();
        let imported = crate::query::definition::import_map(doc);
        let mut out = Bare {
            types: Vec::new(),
            fns: Vec::new(),
        };
        for i in self.iter() {
            let own = std::ptr::eq(i, doc);
            // the names this document's `use`s bound FROM this file's
            // package — the same (name, pkg) match the definition
            // layer's use-graph walk reads (`matches_pkg`, module name
            // first, the origin/stem derivation on an unnamed file)
            let used_from: HashSet<&str> = imported
                .iter()
                .filter(|(_, pkg)| crate::query::definition::matches_pkg(i, pkg))
                .map(|(name, _)| *name)
                .collect();
            // the bare gate per row: own decls always; foreign rows
            // only when the ambient linkage bound them or the
            // document's `use` named them from this package
            let offered = |name: &str, ambient: bool| own || ambient || used_from.contains(name);
            for t in i.types() {
                if offered(t.name(), t.ambient()) {
                    out.types.push(t);
                }
            }
            for f in i.fns() {
                if f.owner().is_none() && offered(f.name(), f.ambient()) {
                    out.fns.push(f);
                }
            }
        }
        out
    }
}

// ---- the views — facts derived from (Doc, AnyHandle) at read ----

/// the doc lines a decl node carries, as rendered lines — the parse's
/// own graft (the Go-convention rules). Empty when nothing attaches.
fn doc_lines(src: &str, d: &DocData) -> Vec<String> {
    d.lines(src).map(|l| l.trim_start().to_string()).collect()
}

/// verbatim signature: the source up to (not including) the body; a
/// bodiless decl ends at its own `;` (the exact-span law) and the
/// trailing `;` is trimmed off the render
fn sig_src(
    src: &str,
    ast: &Ast,
    span: rut_lexer::span::Span,
    body: Option<Handle<rut_ast::BlockNode>>,
) -> String {
    let hi = match body {
        Some(b) => b.span(ast).lo,
        None => span.hi, // the item's own terminator — the `;`
    };
    src[span.lo as usize..hi as usize]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

/// the verbatim decl slice through the item's own `;`, trimmed — the
/// shared cut of the `mod` edges and the alias head
fn decl_src(src: &str, span: rut_lexer::span::Span) -> String {
    src[span.lo as usize..span.hi as usize]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

/// verbatim head: the source from the item's FIRST token (span law
/// v3.1 — modifiers included, generics and admission bounds as
/// written) up to the body's `{` — the brace pair's left edge, the
/// head slice's right cut
fn head_src(src: &str, span: rut_lexer::span::Span, body: rut_lexer::span::Span) -> String {
    let hi = if body.lo > span.lo { body.lo } else { span.hi };
    src[span.lo as usize..hi as usize]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

/// declared parameter names in order, `self` excluded — the AST's own
/// data; the inlay hints' call-site parameter names price exact-arity
/// matches against this
fn param_names(ast: &Ast, params: &[ParamRef]) -> Vec<String> {
    params
        .iter()
        .filter_map(|p| match p {
            ParamRef::Param(d) => Some(ast.name(d.get(ast).name).to_string()),
            _ => None, // `self` / `mut self`
        })
        .collect()
}

/// a type decl view — every fact derives from the node on read: the
/// head slice joins over the file's own text, the vis/ambient verdicts
/// ride the modifier chain, the members walk the node's child lists.
#[derive(Clone, Copy)]
pub(crate) struct TyView<'a> {
    pub(crate) file: &'a MountedFile,
    pub(crate) h: AnyHandle,
}

/// a member view — a field, a method, or an enum member of
/// [`TyView`]'s body
#[derive(Clone, Copy)]
pub(crate) struct MemberView<'a> {
    pub(crate) file: &'a MountedFile,
    /// the owning type decl
    pub(crate) ty: TyView<'a>,
    pub(crate) h: AnyHandle,
}

/// a fn view — a free fn, a surface fn, or an impl-block method (the
/// owner derives from the parent impl's target)
#[derive(Clone, Copy)]
pub(crate) struct FnView<'a> {
    pub(crate) file: &'a MountedFile,
    pub(crate) h: AnyHandle,
}

/// a module-scope `let` view
#[derive(Clone, Copy)]
pub(crate) struct LetView<'a> {
    pub(crate) file: &'a MountedFile,
    pub(crate) h: AnyHandle,
}

/// one imported name of a use edge (a brace list's names share the
/// node; each is its own row)
#[derive(Clone, Copy)]
pub(crate) struct UseView<'a> {
    pub(crate) file: &'a MountedFile,
    pub(crate) h: AnyHandle,
    /// the name's index within the use node's names
    pub(crate) i: usize,
}

/// a `mod NAME;` namespace edge view
#[derive(Clone, Copy)]
pub(crate) struct ModView<'a> {
    pub(crate) file: &'a MountedFile,
    pub(crate) h: AnyHandle,
}

impl<'a> TyView<'a> {
    fn ast(&self) -> &'a Ast {
        self.file.doc.ast()
    }

    fn text(&self) -> &'a str {
        self.file.doc.src()
    }

    /// the declaring ident's span (the definition target)
    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        Some(self.name_span_raw())
    }

    /// the node's own recorded name span (the degenerate-empty span
    /// included — fields fall back to their own extent)
    fn name_span_raw(&self) -> rut_lexer::span::Span {
        let ast = self.ast();
        macro_rules! ns {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return d.get(ast).name_span;
                }
            };
        }
        ns!(ClassNode);
        ns!(StructNode);
        ns!(InterfaceNode);
        ns!(EnumNode);
        ns!(BuiltinTyNode);
        ns!(BuiltinPrimitiveNode);
        ns!(SurfaceStructNode);
        ns!(AliasNode);
        unreachable!("a type view addresses a type decl node")
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.ast();
        macro_rules! nm {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return ast.name(d.get(ast).name);
                }
            };
        }
        nm!(ClassNode);
        nm!(StructNode);
        nm!(InterfaceNode);
        nm!(EnumNode);
        nm!(BuiltinTyNode);
        nm!(BuiltinPrimitiveNode);
        nm!(SurfaceStructNode);
        nm!(AliasNode);
        unreachable!("a type view addresses a type decl node")
    }

    pub(crate) fn form(&self) -> TyForm {
        macro_rules! f {
            ($t:ty, $v:expr) => {
                if self.h.is::<$t>() {
                    return $v;
                }
            };
        }
        f!(ClassNode, TyForm::Class);
        f!(StructNode, TyForm::Struct);
        f!(InterfaceNode, TyForm::Interface);
        f!(EnumNode, TyForm::Enum);
        f!(BuiltinTyNode, TyForm::Builtin);
        f!(BuiltinPrimitiveNode, TyForm::Primitive);
        f!(SurfaceStructNode, TyForm::HostStruct);
        f!(AliasNode, TyForm::Alias);
        unreachable!("a type view addresses a type decl node")
    }

    /// the decl head as WRITTEN — the verbatim `span × src` slice from
    /// the item's first token cut at the body's `{`; an alias slices
    /// through its own terminator
    pub(crate) fn head(&self) -> String {
        let ast = self.ast();
        let src = self.text();
        macro_rules! hs {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    let d = d.get(ast);
                    return head_src(src, d.span, d.body_span);
                }
            };
        }
        hs!(ClassNode);
        hs!(StructNode);
        hs!(InterfaceNode);
        hs!(EnumNode);
        hs!(BuiltinTyNode);
        hs!(BuiltinPrimitiveNode);
        hs!(SurfaceStructNode);
        // `type X = A;` — the whole decl is the verbatim slice (target
        // as written, terminator trimmed); the head IS the decl text
        if let Some(d) = self.h.downcast::<AliasNode>() {
            return decl_src(src, d.get(ast).span);
        }
        unreachable!("a type view addresses a type decl node")
    }

    /// the declared visibility — the completion/hover tier gate reads
    /// it (`pub(pkg)`/`pub(super)` are not `pub`). The engine surfaces
    /// (`builtin`/`builtin primitive`/`host struct`) render `pub`: the
    /// row-era shape — their gate is the ambient split, never the vis.
    pub(crate) fn vis(&self) -> Vis {
        let ast = self.ast();
        macro_rules! v {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return d.get(ast).vis();
                }
            };
        }
        v!(ClassNode);
        v!(StructNode);
        v!(InterfaceNode);
        v!(EnumNode);
        v!(AliasNode);
        if self.h.is::<BuiltinTyNode>()
            || self.h.is::<BuiltinPrimitiveNode>()
            || self.h.is::<SurfaceStructNode>()
        {
            return Vis::Pub;
        }
        unreachable!("a type view addresses a type decl node")
    }

    /// `pub` on the declaration — the completion tiers' names face
    /// offers pub rows only, while the INDEX itself stays unfiltered
    pub(crate) fn is_pub(&self) -> bool {
        self.vis() == Vis::Pub
    }

    /// the ambient builtin linkage (`prelude builtin`) — the name
    /// binds in every compilation unit with no `use`. DERIVED from the
    /// modifier chain at read.
    pub(crate) fn ambient(&self) -> bool {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<BuiltinTyNode>() {
            return d.get(ast).ambient();
        }
        if let Some(d) = self.h.downcast::<BuiltinPrimitiveNode>() {
            return d.get(ast).ambient();
        }
        false
    }

    pub(crate) fn fields(&self) -> Vec<MemberView<'a>> {
        let ast = self.ast();
        let mut out = Vec::new();
        macro_rules! flds {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    out.extend(fields_of(self, &d.get(ast).fields));
                }
            };
        }
        flds!(ClassNode);
        flds!(StructNode);
        flds!(SurfaceStructNode);
        // an enum's members ride the fields list (the row-era shape:
        // `Color::Red` resolves through the member list)
        if let Some(d) = self.h.downcast::<EnumNode>() {
            for m in &d.get(ast).members {
                let MemberRef::Member(mh) = m else {
                    continue; // the recovery slot renders no row
                };
                out.push(MemberView {
                    file: self.file,
                    ty: *self,
                    h: mh.any(),
                });
            }
        }
        out
    }

    pub(crate) fn methods(&self) -> Vec<MemberView<'a>> {
        let ast = self.ast();
        let mut out = Vec::new();
        macro_rules! mths {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    out.extend(methods_of(self, &d.get(ast).methods));
                }
            };
        }
        mths!(ClassNode);
        mths!(StructNode);
        mths!(InterfaceNode);
        // the engine surfaces carry their member list under `members`
        if let Some(d) = self.h.downcast::<BuiltinTyNode>() {
            out.extend(methods_of(self, &d.get(ast).members));
        } else if let Some(d) = self.h.downcast::<BuiltinPrimitiveNode>() {
            out.extend(methods_of(self, &d.get(ast).members));
        }
        out
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let ast = self.ast();
        macro_rules! dd {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return doc_lines(self.text(), &d.get(ast).doc);
                }
            };
        }
        dd!(ClassNode);
        dd!(StructNode);
        dd!(InterfaceNode);
        dd!(EnumNode);
        dd!(BuiltinTyNode);
        dd!(BuiltinPrimitiveNode);
        dd!(SurfaceStructNode);
        dd!(AliasNode);
        unreachable!("a type view addresses a type decl node")
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.ast().span_of(self.h)
    }

    /// 1-based line of the declaration — the doc's own line geometry
    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}

fn fields_of<'a>(ty: &TyView<'a>, fields: &[FieldRef]) -> Vec<MemberView<'a>> {
    fields
        .iter()
        .filter_map(|f| {
            let FieldRef::Field(h) = f else {
                return None; // the recovery slot renders no row
            };
            Some(MemberView {
                file: ty.file,
                ty: *ty,
                h: h.any(),
            })
        })
        .collect()
}

fn methods_of<'a>(ty: &TyView<'a>, methods: &[MethodRef]) -> Vec<MemberView<'a>> {
    methods
        .iter()
        .filter_map(|m| {
            let MethodRef::Method(h) = m else {
                return None; // the recovery slot renders no row
            };
            Some(MemberView {
                file: ty.file,
                ty: *ty,
                h: h.any(),
            })
        })
        .collect()
}

impl<'a> MemberView<'a> {
    fn ast(&self) -> &'a Ast {
        self.file.doc.ast()
    }

    fn text(&self) -> &'a str {
        self.file.doc.src()
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.ast();
        macro_rules! nm {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return ast.name(d.get(ast).name);
                }
            };
        }
        nm!(FieldDeclNode);
        nm!(MethodDeclNode);
        nm!(EnumMemberNode);
        unreachable!("a member view addresses a member node")
    }

    /// verbatim source of the declaration (signature for methods) —
    /// modifiers included: the decl span starts at the member's first
    /// token
    pub(crate) fn src(&self) -> String {
        let ast = self.ast();
        let src = self.text();
        if let Some(d) = self.h.downcast::<FieldDeclNode>() {
            let d = d.get(ast);
            return decl_src(src, d.span);
        }
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            let d = d.get(ast);
            return sig_src(src, ast, d.span, d.body);
        }
        if let Some(d) = self.h.downcast::<EnumMemberNode>() {
            let d = d.get(ast);
            return d.span.text(src).to_string();
        }
        unreachable!("a member view addresses a member node")
    }

    /// the declared type as written — fields: the type; methods: the
    /// return (`None` = unwritten, i.e. `nil`-returning); enum members
    /// carry none
    pub(crate) fn ty(&self) -> Option<String> {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<FieldDeclNode>() {
            return Some(ty_src(ast, d.get(ast).ty));
        }
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            return d.get(ast).ret.map(|r| ty_src(ast, r));
        }
        None
    }

    /// the `static` verdict, derived from the field's modifier nodes —
    /// the `::` completion tier's gate (methods are never static)
    pub(crate) fn is_static(&self) -> bool {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<FieldDeclNode>() {
            return d.get(ast).is_static();
        }
        false
    }

    /// byte span of the name identifier — the field / enum-member
    /// decl-site hover and the definition lookup key on it. The
    /// synthesized newtype's `inner` records no name token (the span
    /// degenerates empty) — the field's own extent stands in.
    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<FieldDeclNode>() {
            let d = d.get(ast);
            return Some(if d.name_span.hi > d.name_span.lo {
                d.name_span
            } else {
                d.span
            });
        }
        macro_rules! ns {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return Some(d.get(ast).name_span);
                }
            };
        }
        ns!(MethodDeclNode);
        ns!(EnumMemberNode);
        unreachable!("a member view addresses a member node")
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let ast = self.ast();
        macro_rules! dd {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return doc_lines(self.text(), &d.get(ast).doc);
                }
            };
        }
        dd!(FieldDeclNode);
        dd!(MethodDeclNode);
        dd!(EnumMemberNode);
        unreachable!("a member view addresses a member node")
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.ast().span_of(self.h)
    }

    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}

impl<'a> FnView<'a> {
    fn ast(&self) -> &'a Ast {
        self.file.doc.ast()
    }

    fn text(&self) -> &'a str {
        self.file.doc.src()
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.ast();
        macro_rules! nm {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return ast.name(d.get(ast).name);
                }
            };
        }
        nm!(FnNode);
        nm!(SurfaceFnNode);
        nm!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// byte span of the name identifier — the definition target
    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        let ast = self.ast();
        macro_rules! ns {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return Some(d.get(ast).name_span);
                }
            };
        }
        ns!(FnNode);
        ns!(SurfaceFnNode);
        ns!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// verbatim signature source
    pub(crate) fn src(&self) -> String {
        let ast = self.ast();
        let src = self.text();
        if let Some(d) = self.h.downcast::<FnNode>() {
            let d = d.get(ast);
            return sig_src(src, ast, d.span, Some(d.body));
        }
        if let Some(d) = self.h.downcast::<SurfaceFnNode>() {
            let d = d.get(ast);
            return sig_src(src, ast, d.span, None);
        }
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            let d = d.get(ast);
            return sig_src(src, ast, d.span, d.body);
        }
        unreachable!("a fn view addresses a fn node")
    }

    /// the declared return as written (`None` = unwritten); inferred-
    /// type hovers read it for call initializers. Surface rows never
    /// record one (the v1 shape — inference does not ride the host
    /// boundary).
    pub(crate) fn ret(&self) -> Option<String> {
        let ast = self.ast();
        if self.h.is::<SurfaceFnNode>() {
            return None;
        }
        let r = if let Some(d) = self.h.downcast::<FnNode>() {
            d.get(ast).ret
        } else if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            d.get(ast).ret
        } else {
            unreachable!("a fn view addresses a fn node")
        };
        r.map(|r| ty_src(ast, r))
    }

    /// declared parameter names in order, `self` excluded
    pub(crate) fn params(&self) -> Vec<String> {
        let ast = self.ast();
        macro_rules! ps {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return param_names(ast, &d.get(ast).params);
                }
            };
        }
        ps!(FnNode);
        ps!(SurfaceFnNode);
        ps!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let ast = self.ast();
        macro_rules! dd {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return doc_lines(self.text(), &d.get(ast).doc);
                }
            };
        }
        dd!(FnNode);
        dd!(SurfaceFnNode);
        dd!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// the declared visibility — the position-path completion's tier
    /// gate reads it (impl methods carry their member vis, `pub` when
    /// unannotated — inherent impls own their methods)
    pub(crate) fn vis(&self) -> Vis {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<FnNode>() {
            return d.get(ast).vis();
        }
        if let Some(d) = self.h.downcast::<SurfaceFnNode>() {
            return d.get(ast).vis();
        }
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            return d.get(ast).vis().unwrap_or(Vis::Pub);
        }
        unreachable!("a fn view addresses a fn node")
    }

    /// the ambient builtin linkage (`prelude builtin fn`) — DERIVED
    /// from the modifier chain at read
    pub(crate) fn ambient(&self) -> bool {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<SurfaceFnNode>() {
            return matches!(
                d.get(ast).linkage(),
                rut_ast::Linkage::Builtin { ambient: true }
            );
        }
        false
    }

    /// the owning inherent impl's target type (`Circle` — inherent
    /// impls own their methods, so an impl method renders like the
    /// type's own surface), `None` for a free fn. DERIVED from the
    /// tree: the parent of an impl method IS its impl block.
    pub(crate) fn owner(&self) -> Option<String> {
        let ast = self.ast();
        self.h.downcast::<MethodDeclNode>()?;
        let parent = ast.parent(self.h)?;
        let imp = parent.downcast::<ImplNode>()?;
        Some(ty_head(ast, imp.get(ast).target))
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.ast().span_of(self.h)
    }

    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}

impl<'a> LetView<'a> {
    fn ast(&self) -> &'a Ast {
        self.file.doc.ast()
    }

    pub(crate) fn name(&self) -> &'a str {
        self.ast().name(
            self.h
                .downcast::<ModuleLetNode>()
                .expect("a let view")
                .get(self.ast())
                .name,
        )
    }

    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        Some(
            self.h
                .downcast::<ModuleLetNode>()
                .expect("a let view")
                .get(self.ast())
                .name_span,
        )
    }

    /// the declared annotation as written, else the initializer-derived
    /// type (the same display-side heuristic the binding pass uses,
    /// priced against THIS file's own decls)
    pub(crate) fn ty(&self) -> Option<String> {
        let ast = self.ast();
        let d = self
            .h
            .downcast::<ModuleLetNode>()
            .expect("a let view")
            .get(ast);
        match d.ty {
            Some(t) => Some(ty_src(ast, t)),
            None => infer::expr_ty(self.file, &[self.file], d.init),
        }
    }

    /// verbatim decl slice, through the initializer (the decl's own
    /// `;` — the exact-span law), trimmed of trailing space only
    pub(crate) fn src(&self) -> String {
        let span = self.span();
        let src = self.file.doc.src();
        src[span.lo as usize..span.hi as usize]
            .trim_end()
            .to_string()
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let d = self
            .h
            .downcast::<ModuleLetNode>()
            .expect("a let view")
            .get(self.ast());
        doc_lines(self.file.doc.src(), &d.doc)
    }

    /// the declared visibility — the position-path completion's tier
    /// gate reads it (`pub(pkg)`/`pub(super)` are not `pub`)
    pub(crate) fn vis(&self) -> Vis {
        self.h
            .downcast::<ModuleLetNode>()
            .expect("a let view")
            .get(self.ast())
            .vis()
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.ast().span_of(self.h)
    }

    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}

impl<'a> UseView<'a> {
    fn d(&self) -> rut_ast::Handle<UseNode> {
        self.h.downcast::<UseNode>().expect("a use view")
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.file.doc.ast();
        let d = self.d().get(ast);
        ast.name(d.names[self.i])
    }

    pub(crate) fn vis(&self) -> Vis {
        self.d().get(self.file.doc.ast()).vis()
    }

    pub(crate) fn pkg(&self) -> &'a str {
        let ast = self.file.doc.ast();
        let d = self.d().get(ast);
        ast.name(d.path[0])
    }

    pub(crate) fn path(&self) -> Vec<String> {
        let ast = self.file.doc.ast();
        self.d()
            .get(ast)
            .path
            .iter()
            .map(|id| ast.name(*id).to_string())
            .collect()
    }
}

impl<'a> ModView<'a> {
    fn d(&self) -> rut_ast::Handle<ModDeclNode> {
        self.h.downcast::<ModDeclNode>().expect("a mod view")
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.file.doc.ast();
        ast.name(self.d().get(ast).name)
    }

    /// byte span of the name identifier — the decl-site hover /
    /// definition target
    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        Some(self.d().get(self.file.doc.ast()).name_span)
    }

    /// the edge's declared visibility (`Vis::Self_` = bare `mod`)
    pub(crate) fn vis(&self) -> Vis {
        self.d().get(self.file.doc.ast()).vis()
    }

    /// the edge as spelled — the verbatim `span × src` slice (span law
    /// v3.1 starts the item at its first modifier), trailing `;`
    /// trimmed: the render never re-synthesizes `pub mod NAME;`
    pub(crate) fn src(&self) -> String {
        decl_src(self.file.doc.src(), self.span())
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let d = self.d().get(self.file.doc.ast());
        doc_lines(self.file.doc.src(), &d.doc)
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.file.doc.ast().span_of(self.h)
    }

    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
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
        out.extend(d.names.iter().map(|n| ast.name(*n).to_string()));
    }
    out
}
