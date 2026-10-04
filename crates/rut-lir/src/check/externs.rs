//! The used-module/extern registry: `use`d fns, consts, types, ifaces, impls and namespaces, plus the `builtin impl` table — the surface bindings the use-both gate speaks through.

use super::*;

impl<'a> Ctx<'a> {

    /// Bind a used function before body compilation. `is_async` marks
    /// the host future lane (`host async fn`); `host` carries the
    /// row's registration name (`<scope>::<name>`) when the exporter
    /// is a decl/native module — the minted row family's host ids
    /// derive from it.
    pub fn add_extern_fn(
        &mut self,
        name: IdentId,
        func: u32,
        params: Vec<TypeId>,
        ret: TypeId,
        is_async: bool,
        host: Option<String>,
    ) {
        // the engine-minted cx singleton (the async frame's RunContext)
        // is per-unit minted; a foreign surface's cx param re-spells to
        // THIS unit's row, so the crossing check sees one type
        let cx_ty = self.run_context_ty();
        let cx_ty_name = self.types.type_at(cx_ty).name;
        let remap = |t: TypeId| -> TypeId {
            if self.types.type_at(t).name == cx_ty_name {
                cx_ty
            } else {
                t
            }
        };
        let params = params.iter().map(|t| remap(*t)).collect();
        self.extern_fns
            .insert(name, ExternFn { func, params, ret, is_async, host });
    }

    /// Bind a `builtin impl` numeric method of a primitive (core only):
    /// `(method name → receiver prim, lowering id)`.
    /// AMBIENT — primitive receivers resolve their methods without a
    /// `use`; rut-lir expands the call inline.
    pub fn add_builtin_impl(
        &mut self,
        name: IdentId,
        prim: TypeId,
        intrinsic: rut_core::ops::Intrinsic,
    ) {
        self.builtin_impls.entry(name).or_default().push((prim, intrinsic));
    }

    /// The lowering of `name` on a receiver of primitive type `recv`, if
    /// core's `builtin impl` table declares it.
    pub fn builtin_impl(&self, name: IdentId, recv: TypeId) -> Option<rut_core::ops::Intrinsic> {
        self.builtin_impls
            .get(&name)?
            .iter()
            .find(|(p, _)| *p == recv)
            .map(|(_, i)| *i)
    }

    pub fn extern_fn(&self, name: IdentId) -> Option<&ExternFn> {
        self.extern_fns.get(&name)
    }

    /// Bind a used constant (native modules: `calc::PI`).
    pub fn add_extern_const(&mut self, name: IdentId, ty: TypeId, bits: u64) {
        self.extern_consts.insert(name, (ty, bits));
    }

    pub fn extern_const(&self, name: IdentId) -> Option<(TypeId, u64)> {
        self.extern_consts.get(&name).copied()
    }

    /// Bind a used type name to the exporter's scope-qualified id.
    pub fn add_extern_type(&mut self, name: IdentId, ty: TypeId, is_class: bool, newtype: bool) {
        self.extern_types.insert(name, ty);
        if is_class {
            self.extern_classes.insert(ty);
        }
        if newtype {
            self.extern_newtypes.insert(name);
        }
    }

    /// Bind a linked generic: the template row (registered from the
    /// exporter's carried block), its parameter names in order, and the
    /// declaring pkg — the owner every instantiation of it is requested
    /// from. The name still rides `extern_types` (the bare template
    /// spells the owner's own row, e.g. in a `type_id<T>()` probe).
    pub fn add_extern_generic(
        &mut self,
        name: IdentId,
        owner: String,
        params: Vec<IdentId>,
        template: TypeId,
        is_class: bool,
        newtype: bool,
    ) {
        self.decl_owner.insert(name, owner.clone());
        if newtype {
            self.extern_newtypes.insert(name);
        }
        self.extern_generics.insert(
            name,
            ExternGeneric { owner, params, template, is_class, newtype },
        );
    }

    /// Bind a used core builtin container (`Array`/`Option`/
    /// `Result`/`Opaque`): the type constructor is the
    /// compiler's own; the binding gates the NAME.
    pub fn add_extern_native_type(&mut self, name: IdentId, kind: rut_core::binary::NativeTy) {
        self.extern_native_types.insert(name, kind);
    }

    /// Bind a trait from a used module's surface: the
    /// descriptor joins THIS module's trait table (so slot numbering,
    /// widening and vtables treat it like a declared trait). The name
    /// resolves only when the module used it — the use-both gate's
    /// enforcement point. Method names re-intern from the
    /// surface's interner; parameter/ret ids pass through verbatim —
    /// they are packed with the scopes the surface's type blocks were
    /// registered under (`use_types`).
    ///
    /// `name` is the consumer-side name (already looked up); `None`
    /// registers the descriptor without a name binding (an impl's trait
    /// the module never named — visible to the use-gate diagnostic,
    /// invisible to resolution).
    pub fn add_extern_iface_decl(
        &mut self,
        name: Option<IdentId>,
        desc: &rut_core::binary::SurfaceIface,
        surface_names: &Interner,
    ) -> u32 {
        let tname = self.intern(surface_names.name(desc.name));
        let mut methods = Vec::with_capacity(desc.methods.len());
        for m in &desc.methods {
            methods.push(rut_core::binary::IfaceMethod {
                name: self.intern(surface_names.name(m.name)),
                params: m.params.clone(),
                ret: m.ret,
            });
        }
        let id = self.ifaces.len() as u32;
        self.ifaces.push(IfaceDesc { name: tname, methods });
        if let Some(n) = name {
            self.extern_iface_decls.insert(n, ExternIface { id, generics: desc.generics });
        }
        id
    }

    /// Bind a trait impl registered by a used module's surface
    /// (the impl may live in any module). `methods` pair
    /// each trait method with the exporter's scope-qualified fn id
    /// (slot ABI); `methods_concrete` the concrete-ABI twin.
    /// The carried mirror rows join the instantiation maps: a field's
    /// type (`Mutation<str, Req>`) arrives through a used struct's type
    /// block, never through this unit's own resolution, so the
    /// method-call route's `inst_data` lookup — and a later spelling of
    /// the same instantiation, which must dedup onto the very row the
    /// field carries — needs the maps filled. Rows this unit resolved
    /// itself are already registered; a row whose base is not a used
    /// generic class passes through.
    pub fn register_carried_insts(&mut self) {
        let names: Vec<String> = (0..self.types.types.len())
            .map(|i| {
                let name = self.types.types[i].name;
                self.interner.name(name).to_string()
            })
            .collect();
        for (i, text) in names.iter().enumerate() {
            let Some(ty) = self.types.id_for_pub_checked(i as u32) else { continue };
            if self.inst_data.contains_key(&ty) {
                continue;
            }
            let Some((base_text, args_text)) = text.split_once('<') else {
                continue;
            };
            let Some(args_text) = args_text.strip_suffix('>') else {
                continue;
            };
            let Some((base, g)) = self
                .extern_generics
                .iter()
                .find(|(n, _)| self.name(**n) == base_text)
                .map(|(n, g)| (*n, g.clone()))
            else {
                continue;
            };
            let arg_texts = crate::check::collect::split_top_commas(args_text);
            if arg_texts.len() != g.params.len() {
                continue;
            }
            let mut args = Vec::with_capacity(arg_texts.len());
            let mut ok = true;
            for at in &arg_texts {
                // a no-intern lookup: a miss must not grow the name
                // table (the two compile lanes byte-compare programs)
                let resolved = if let Some(bare) = at.strip_prefix('?') {
                    self.interner
                        .lookup(bare)
                        .and_then(|bid| self.types.dense_id_of_name(bid))
                        .map(|e| self.mk_opt(e))
                } else {
                    self.interner
                        .lookup(at)
                        .and_then(|iid| self.types.dense_id_of_name(iid))
                };
                match resolved {
                    Some(a) => args.push(a),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            // NO type_inst entry: a later spelling of the same
            // instantiation must mint its OWN row (this unit's substitute
            // of the template's fields — ids this unit can resolve). The
            // carried row's internal ids name the EXPORTER's world (its
            // deps, its seed region); deduping onto it would hand the
            // speller a type whose fields resolve only in the exporter.
            // `inst_data` stays: the method/dispatch route reads the
            // carried row where it sits, and the link unifies mirrors of
            // one owner-anchored instantiation at the canonical key.
            self.inst_data.insert(ty, (base, args.clone()));
            if g.is_class {
                self.extern_classes.insert(ty);
            }
            let owner = self.intern(&g.owner);
            self.ledger_types.push(rut_core::binary::InstTy {
                owner,
                decl: base,
                args: args.clone(),
                ty,
            });
            self.request_inst(g.owner.clone(), base, args);
        }
    }

    /// Bind a used enum (the linkable-classes phase): the member paths
    /// (`EncodeErrorKind.Depth`) resolve through this registry; the
    /// descriptor row itself rides the carried type block.
    pub fn add_extern_enum(&mut self, name: IdentId, ty: TypeId, members: Vec<(IdentId, i64)>) {        self.extern_enums.insert(name, (ty, members));
    }

    /// The used enum's (type id, members), if bound.
    pub fn extern_enum(&self, name: IdentId) -> Option<(TypeId, Vec<(IdentId, i64)>)> {
        self.extern_enums.get(&name).cloned()
    }

    /// Is this type row a bare placeholder leaf for one of `params`
    /// (the `#<param>` row the surface build spelled)?
    pub fn template_leaf(&self, t: TypeId, params: &[IdentId]) -> bool {
        let text = self.interner.name(self.types.type_at(t).name).to_string();
        params
            .iter()
            .any(|p| format!("#{}", self.name(*p)) == text)
    }

    /// The element of a `Future<E>` handle: the kind carries it
    /// structurally (v20's closed class).
    pub fn future_elem(&mut self, ty: TypeId) -> Option<TypeId> {
        match self.types.kind(ty) {
            TyKind::Future { elem } => Some(*elem),
            _ => None,
        }
    }

    /// Re-spell a foreign impl method's signature type for THIS call's
    /// concrete target: a `Self`-spelled trait object (the descriptor's
    /// `?Self`, at any structural depth) becomes the concrete type. A
    /// row that mentions no `Self` passes through unchanged.
    pub fn respell_trait_self_deep(
        &mut self,
        t: TypeId,
        iface_id: u32,
        concrete: TypeId,
    ) -> TypeId {
        let kind = self.types.kind(t).clone();
        let rebuilt = match kind {
            TyKind::IfaceObj { iface_id: t } if t == iface_id => Some(concrete),
            TyKind::Opt { elem } => {
                let e = self.respell_trait_self_deep(elem, iface_id, concrete);
                (e != elem).then(|| self.mk_opt(e))
            }
            TyKind::Array { elem } => {
                let e = self.respell_trait_self_deep(elem, iface_id, concrete);
                (e != elem).then(|| self.mk_array(e))
            }
            TyKind::Weak { elem } => {
                let e = self.respell_trait_self_deep(elem, iface_id, concrete);
                (e != elem).then(|| self.mk_weak(e))
            }
            TyKind::Fn { params, ret } => {
                let ps: Vec<TypeId> = params
                    .iter()
                    .map(|&p| self.respell_trait_self_deep(p, iface_id, concrete))
                    .collect();
                let r = self.respell_trait_self_deep(ret, iface_id, concrete);
                (ps != params || r != ret).then(|| self.mk_fn_ty(ps, r))
            }
            TyKind::Data { fields } if !fields.is_empty() => {
                let fs: Vec<FieldInfo> = fields
                    .iter()
                    .map(|f| FieldInfo {
                        name: f.name,
                        ty: self.respell_trait_self_deep(f.ty, iface_id, concrete),
                    })
                    .collect();
                let changed = fs.iter().zip(fields.iter()).any(|(n, o)| n.ty != o.ty);
                changed.then(|| {
                    let name = self.intern(&format!(
                        "({})",
                        fs.iter()
                            .map(|f| self.type_name(f.ty).to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    self.types.intern(RutType { name, kind: TyKind::Data { fields: fs } })
                })
            }
            _ => None,
        };
        rebuilt.unwrap_or(t)
    }

    /// Resolve an element TEXT (a trait-inst name's carrier) to a row:
    /// the carried/boot rows by name; a leading `?` peels to the
    /// element and rebuilds the nullable (the boot answer lanes are
    /// named by their element, so the plain lookup cannot see them).
    pub fn resolve_elem_text(&mut self, inner: &str) -> Option<TypeId> {
        if let Some(bare) = inner.strip_prefix('?') {
            let bid = self.intern(bare);
            let elem = self.types.dense_id_of_name(bid)?;
            return Some(self.mk_opt(elem));
        }
        let id = self.intern(inner);
        self.types.dense_id_of_name(id)
    }

    /// The async futures re-spell at the binding: a linked
    /// pkg's `Future<Response>` ret is ITS unit's instantiation row
    /// (exporter-packed ids); the importing unit re-mints the row over
    /// THIS unit's element. The element rides the kind structurally
    /// (v20's closed class — no name decoding): it resolves through the
    /// binding's own type exports first (the pkg carries the row), then
    /// the carried/boot rows (`?T` peels — see `resolve_elem_text`).
    pub fn respell_future_ret(
        &mut self,
        ret: TypeId,
        surface_exports: &[(IdentId, TypeId)],
    ) -> TypeId {
        let TyKind::Future { elem } = self.types.kind(ret) else {
            return ret;
        };
        let text = self.type_name(*elem).to_string();
        let inner_id = self.intern(&text);
        let elem2 = surface_exports
            .iter()
            .find(|(n, _)| *n == inner_id)
            .map(|(_, ty)| *ty)
            .or_else(|| self.resolve_elem_text(&text));
        match elem2 {
            Some(e) => self.mk_future(e),
            None => ret,
        }
    }

    /// Bind a used class's inherent method surface (the
    /// linkable-classes phase): `methods` ride the surface verbatim,
    /// each fn id scope-qualified with the exporter's scope. A
    /// `[constructor]` byte on a row fills the designation registry —
    /// the consumer's `Name(..)` fallback reads it (the marker crossed
    /// the surface; the exporter's own gate proved the class law).
    pub fn add_extern_inherent(&mut self, target: TypeId, methods: Vec<rut_core::binary::SurfaceMethod>) {
        if methods.is_empty() {
            return;
        }
        let cname = self.types.type_at(target).name;
        for m in &methods {
            if m.marker == rut_core::binary::MARKER_CONSTRUCTOR {
                self.class_ctors.insert(cname, m.name);
            }
        }
        self.extern_inherents.push(ExternInherent { target, methods });
    }

    /// Bind a used module's exported generic fn (the linkable-classes
    /// phase): the placeholder signature the call site checks against,
    /// and the declaring pkg the body requests route to.
    pub fn add_extern_generic_fn(
        &mut self,
        name: IdentId,
        owner: String,
        params: Vec<IdentId>,
        args: Vec<TypeId>,
        ret: TypeId,
    ) {
        self.extern_generic_fns
            .insert(name, ExternGenericFn { owner, params, args, ret });
    }

    pub fn extern_generic_fn(&self, name: IdentId) -> Option<&ExternGenericFn> {
        self.extern_generic_fns.get(&name)
    }

    /// The used class whose inherent surface answers `name` on
    /// receiver `rt`, with the receiver's class substitution: the
    /// row's target matches either the receiver's own id (a plain
    /// class's extern row) or the receiver's instantiation's TEMPLATE
    /// (a mirror instantiation of a used generic — `inst_data` names
    /// the class, `extern_generics` its template). The substitution
    /// pairs the template's parameter names with the receiver's
    /// concrete arguments, in the row's own order.
    pub fn find_extern_method(
        &self,
        rt: TypeId,
        name: IdentId,
    ) -> Option<(usize, usize, Vec<(IdentId, TypeId)>)> {
        for (i, ih) in self.extern_inherents.iter().enumerate() {
            // the receiver names the class: either directly (the plain
            // extern row) or through its instantiation's decl name
            let (dname, cargs) = match self.inst_data.get(&rt) {
                Some((d, args)) => (Some(*d), args.clone()),
                None => (None, vec![]),
            };
            let hit = ih.target == rt
                || dname.and_then(|d| self.extern_generics.get(&d))
                    .map_or(false, |g| g.template == ih.target);
            if !hit {
                continue;
            }
            if let Some(midx) = ih.methods.iter().position(|m| m.name == name) {
                let subst = match dname.and_then(|d| self.extern_generics.get(&d)) {
                    Some(g) => g.params.iter().cloned().zip(cargs.iter().cloned()).collect(),
                    None => vec![],
                };
                return Some((i, midx, subst));
            }
        }
        None
    }

    /// Does the used class surface provide `name` at all (the
    /// capability probe for union bounds and diagnostics)?
    pub fn has_extern_method(&self, rt: TypeId, name: IdentId) -> bool {
        self.find_extern_method(rt, name).is_some()
    }

    /// The id of a used module's exported trait, if the module used the
    /// name (the use-both gate).
    pub fn extern_trait(&self, name: IdentId) -> Option<&ExternIface> {
        self.extern_iface_decls.get(&name)
    }

    /// The carried placeholder descriptor's generic-parameter leaves, in
    /// signature walk order: methods in order, each method's parameters
    /// then return, the DISTINCT `#`-prefixed leaf texts in
    /// first-appearance order. The k-th leaf binds the k-th trait
    /// argument — the crossing template law the dispatch sites read by
    /// (`descriptor_leaf_env`'s per-method walk, lifted to the whole
    /// descriptor so a mint substitutes every signature at once).
    pub fn carried_trait_leaves(&self, base: &IfaceDesc) -> Vec<String> {
        let mut leaves: Vec<String> = Vec::new();
        let mut scan = |text: &str, leaves: &mut Vec<String>| {
            for piece in text.split(|c: char| !c.is_alphanumeric() && c != '#') {
                let piece = piece.trim();
                if piece.len() > 1 && piece.starts_with('#') && !leaves.iter().any(|l| l == piece) {
                    leaves.push(piece.to_string());
                }
            }
        };
        for tm in &base.methods {
            for &p in &tm.params {
                scan(&self.type_name(p).to_string(), &mut leaves);
            }
            scan(&self.type_name(tm.ret).to_string(), &mut leaves);
        }
        leaves
    }

    /// Bind a used core compiler-lowered function (`own`,
    /// `downcast`, `assert`/`panic`, the `str`/`bytes` natives).
    pub fn add_extern_native_fn(&mut self, name: IdentId) {
        self.extern_native_fns.insert(name);
    }

    /// Bind a namespace head: `use calc::{Math}`.
    pub fn add_extern_namespace(&mut self, name: IdentId) {
        self.extern_namespaces.insert(name);
    }

    /// Is `name` a bound namespace head?
    pub fn is_extern_namespace(&self, name: IdentId) -> bool {
        self.extern_namespaces.contains(&name)
    }


}
