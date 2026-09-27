//! Field reads and the union provenance/capability checks that guard them.

use crate::check::TcResult;
use crate::lir::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    pub(crate) fn compile_field(&mut self, recv: NodeHandle<AnyExpr>, name: IdentId, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // `<namespace>.CONST` — a used namespace's constant (checked
        // before the receiver is compiled, since the head is not a value;
        // RFC 0028). Name-generic: routed by the bound head.
        if let ExprKind::Path { segs } = self.ctx.ast.expr(recv).clone() {
            if segs.len() == 1 && self.ctx.is_extern_namespace(segs[0].name) {
                if let Some((ty, bits)) = self.ctx.extern_const(name) {
                    let reg = self.new_reg(ty);
                    self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                    return Ok(ty);
                }
                let ns = self.ctx.name(segs[0].name).to_string();
                let fname = self.ctx.name(name).to_string();
                self.ctx.err(sp, format!("`{ns}.{fname}` is not a namespace constant"));
                return Err(());
            }
        }
        let rt = self.compile_expr(recv, None)?;
        let rreg = self.last_reg;
        // `p.x` auto-derefs (RFC 0005): load the pointee cell first, then
        // the field reads from it
        let (rreg, rt) = match self.ctx.types.kind(rt).clone() {
            TyKind::Opt { elem } if matches!(self.ctx.types.kind(elem), TyKind::Data { .. }) => {
                let dreg = self.new_reg(elem);
                self.emit(Op::GetF { dst: dreg, obj: rreg, field: 0, repr: Repr::Ref }, sp.lo);
                (dreg, elem)
            }
            _ => (rreg, rt),
        };
        if let TyKind::Data { fields } = self.ctx.types.kind(rt).clone() {
            if let Some(fidx) = fields.iter().position(|f| f.name == name) {
                let fty = fields[fidx].ty;
                let dst = self.new_reg(fty);
                self.emit(Op::GetF { dst, obj: rreg, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                return Ok(fty);
            }
            self.ctx.err(sp, format!("`{}` has no field `{}`", self.ctx.type_name(rt), self.ctx.name(name)));
            return Err(());
        }
        if matches!(self.ctx.types.kind(rt), TyKind::TraitObj { .. }) {
            self.ctx.err(sp, "trait objects have no fields —`d.x` on a trait-typed value is a compile error (RFC 0012 §2)");
            return Err(());
        }
        self.ctx.err(sp, format!("`{}` has no field `{}`", self.ctx.type_name(rt), self.ctx.name(name)));
        Err(())
    }

    /// The (generic, bound node) when a receiver expression's static type
    /// is spelled from a union-bounded generic of this frame (RFC 0043
    /// §3): a bare path bound from one (param, annotated let, a copy of
    /// either), or a direct data field whose declared type node spells
    /// the generic (`self.k` in a class body). Pure AST + declared-shape
    /// question — the receiver does not need compiling twice.
    pub(crate) fn union_provenance(&mut self, recv: NodeHandle<AnyExpr>) -> Option<(IdentId, NodeHandle<AnyTy>)> {
        match self.ctx.ast.expr(recv).clone() {
            ExprKind::Path { segs } if segs.len() == 1 => {
                let g = *self.union_syms.get(&segs[0].name)?;
                self.union_bounds.get(&g).map(|&b| (g, b))
            }
            ExprKind::Field { recv: base, name } => {
                let ExprKind::Path { segs } = self.ctx.ast.expr(base) else {
                    return None;
                };
                if segs.len() != 1 || !segs[0].generics.is_empty() {
                    return None;
                }
                let base_ty = self.lookup(segs[0].name)?.ty;
                let field_ty_node = self.declared_field_ty_node(base_ty, name)?;
                match self.ctx.ast.ty(field_ty_node) {
                    TypeKind::TyPath { segs, .. }
                        if segs.len() == 1 && segs[0].generics.is_empty() =>
                    {
                        let g = segs[0].name;
                        self.union_bounds.get(&g).map(|&b| (g, b))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The declared type NODE of field `name` on a data value of `ty` —
    /// the instantiation's substitute types are concrete, but the
    /// capability gate spells the GENERIC through the original decl.
    fn declared_field_ty_node(&self, base_ty: TypeId, name: IdentId) -> Option<NodeHandle<AnyTy>> {
        let decl_node: NodeHandle<AnyItem> = match self.ctx.inst_data.get(&base_ty).cloned() {
            Some((dname, _)) => self.ctx.find_data(dname)?.node,
            None => self
                .ctx
                .datas
                .iter()
                .find(|(_, d)| self.ctx.types.dense(d.ty) == self.ctx.types.dense(base_ty))?
                .1
                .node,
        };
        let fields = match self.ctx.ast.item(decl_node) {
            ItemKind::Class { fields, .. } | ItemKind::Dataclass { fields, .. } => fields.clone(),
            _ => return None,
        };
        fields.iter().find_map(|&f| {
            let fd = self.ctx.ast.field_decl(f);
            (fd.name == name).then_some(fd.ty)
        })
    }

    /// Record (or clear) the union provenance of a freshly bound local:
    /// the declared type NODE spells a union-bounded generic of this
    /// frame — otherwise the name binds without one.
    pub(crate) fn note_union_binding(&mut self, name: IdentId, ty: Option<NodeHandle<AnyTy>>) {
        match ty {
            Some(tn) => match self.ctx.ast.ty(tn) {
                TypeKind::TyPath { segs, .. }
                    if segs.len() == 1 && segs[0].generics.is_empty()
                        && self.union_bounds.contains_key(&segs[0].name) =>
                {
                    self.union_syms.insert(name, segs[0].name);
                }
                _ => {
                    self.union_syms.remove(&name);
                }
            },
            None => {
                self.union_syms.remove(&name);
            }
        }
    }

    /// Register the union-spelled bounds of a decl being spliced inline
    /// (its own bounds plus its class's `requires`). The standalone
    /// instantiation compiles the same body, so the capability gate
    /// applies inside inlines too — a small body is not exempt from the
    /// union law.
    pub(crate) fn arm_union_bounds(&mut self, bounds: &[(IdentId, NodeHandle<AnyTy>)], class_name: Option<IdentId>) {
        for (g, b) in bounds {
            if self.ctx.bound_is_union(*b) {
                self.union_bounds.insert(*g, *b);
            }
        }
        if let Some(cn) = class_name {
            if let Some(d) = self.ctx.find_data(cn) {
                let rs = d.requires.clone();
                for (g, b) in rs {
                    if self.ctx.bound_is_union(b) {
                        self.union_bounds.insert(g, b);
                    }
                }
            }
        }
    }

    /// The capability gate itself (RFC 0043 §3, native-fastpath phase 1):
    /// a call `recv.name(..)` where the receiver is spelled from a
    /// union-bounded generic requires EVERY member of the bound to
    /// provide `name`. The union admits all its members at once, so a
    /// body written against `K` must typecheck for each — diagnosing now
    /// (naming the offending member and the bound) where per-instantiation
    /// implicit checking would only surface it if that member were ever
    /// actually instantiated. Dispatch is unaffected: the concrete member
    /// still binds its own impl below.
    pub(crate) fn check_union_capability(&mut self, recv: NodeHandle<AnyExpr>, name: IdentId, sp: rut_lexer::span::Span) {
        let Some((g, bnode)) = self.union_provenance(recv) else { return };
        let members = self.ctx.resolve_bound_members(bnode, &self.subst);
        for m in members {
            let crate::check::BoundMember::Concrete(c) = m else { continue };
            if !self.ctx.member_has_method(c, name) {
                let ty = self.ctx.type_name(c).to_string();
                let union = crate::check::bound_ty_str(self.ctx, bnode);
                self.ctx.err(sp, format!(
                    "`{ty}` does not provide `{}` — `{}` requires `{union}` and a call on `{}` needs every member of the union to provide it (RFC 0043)",
                    self.ctx.name(name), self.ctx.name(g), self.ctx.name(g)
                ));
                return;
            }
        }
    }
}
