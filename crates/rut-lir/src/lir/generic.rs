//! Generic instantiation by structural unification (RFC 0013 SS2):
//! explicit call-site arguments first, then inference from argument
//! types through builtin containers (Vec/Array/Option/Result) and fn
//! types. The completed substitution keys the Inst in the
//! monomorphization queue.

use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// generic params in a type node that are not yet bound
    pub(crate) fn free_generics(
        &self,
        node: NodeHandle<AnyTy>,
        decl_generics: &[IdentId],
        subst: &[(IdentId, TypeId)],
    ) -> Vec<IdentId> {
        let mut out = Vec::new();
        self.collect_free(node, decl_generics, subst, &mut out);
        out
    }

    pub(crate) fn collect_free(
        &self,
        node: NodeHandle<AnyTy>,
        decl_generics: &[IdentId],
        subst: &[(IdentId, TypeId)],
        out: &mut Vec<IdentId>,
    ) {
        match self.ctx.ast.ty(node) {
            TypeKind::TyPath { segs, .. } => {
                for seg in segs {
                    if decl_generics.contains(&seg.name)
                        && !subst.iter().any(|(n, _)| *n == seg.name)
                        && !out.contains(&seg.name)
                    {
                        out.push(seg.name);
                    }
                    for g in &seg.generics {
                        self.collect_free(*g, decl_generics, subst, out);
                    }
                }
            }
            TypeKind::TyFn { params, ret } => {
                for p in params {
                    self.collect_free(*p, decl_generics, subst, out);
                }
                self.collect_free(*ret, decl_generics, subst, out);
            }
            TypeKind::TyArray { elem } => {
                self.collect_free(*elem, decl_generics, subst, out);
            }
            _ => {}
        }
    }

    /// The best-effort EXPECTED type for an argument whose parameter node
    /// still mentions unbound generics (`f: fn(DeriveCtx) -> T` at a
    /// call that has not inferred `T` yet): the free generics bind to
    /// fresh placeholder types for the hint only, so a spelled lambda
    /// (`fn (ctx) -> str { .. }`) takes its parameter annotations from
    /// the BOUND part of the shape — unification then binds the real
    /// values from the completed argument.
    pub(crate) fn hint_with_placeholders(
        &mut self,
        node: NodeHandle<AnyTy>,
        decl_generics: &[IdentId],
        subst: &[(IdentId, TypeId)],
    ) -> TypeId {
        let free = self.free_generics(node, decl_generics, subst);
        if free.is_empty() {
            return self.resolve_type_now(node);
        }
        let mut hinted = subst.to_vec();
        for g in &free {
            let ph = self.ctx.param_placeholder(*g);
            hinted.retain(|(n, _)| n != g);
            hinted.push((*g, ph));
        }
        let saved = std::mem::replace(&mut self.subst, hinted);
        let t = self.resolve_type_now(node);
        self.subst = saved;
        t
    }

    /// structural unification: bind generic params from an argument's type
    pub(crate) fn unify_generic(
        &mut self,
        param_node: NodeHandle<AnyTy>,
        arg_ty: TypeId,
        decl_generics: &[IdentId],
        subst: &mut Vec<(IdentId, TypeId)>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<()> {
        let bind = |name: IdentId, ty: TypeId, subst: &mut Vec<(IdentId, TypeId)>| {
            if let Some(e) = subst.iter_mut().find(|(n, _)| *n == name) {
                e.1 = ty;
            } else {
                subst.push((name, ty));
            }
        };
        match self.ctx.ast.ty(param_node).clone() {
            TypeKind::TyPath { segs, .. } if segs.len() == 1 && segs[0].generics.is_empty() => {
                let name = segs[0].name;
                if decl_generics.contains(&name) {
                    if let Some(&(_, prev)) = subst.iter().find(|(n, _)| *n == name) {
                        if prev != arg_ty {
                            self.ctx.err(sp, format!(
                                "generic parameter `{}` binds to both `{}` and `{}`",
                                self.ctx.name(name),
                                self.ctx.type_name(prev),
                                self.ctx.type_name(arg_ty)
                            ));
                            return Err(());
                        }
                    }
                    bind(name, arg_ty, subst);
                    return Ok(());
                }
                // non-generic: resolve and compare — a registered impl
                // (nominal widening, RFC 0012 §4) counts as a match
                let want = self.ctx.resolve_type(param_node, subst);
                if !self.widens(arg_ty, want) {
                    self.ctx.err(sp, format!(
                        "argument is `{}`, `{}` expected",
                        self.ctx.type_name(arg_ty), self.ctx.type_name(want)
                    ));
                    return Err(());
                }
                Ok(())
            }
            TypeKind::TyArray { elem } => {
                // `[T]` against an array argument — unify element-wise
                // (RFC 0005 §9): `fn f<T>(xs: [T])`
                let arg_kind = self.ctx.types.kind(arg_ty).clone();
                if let TyKind::Array { elem: arg_elem } = arg_kind {
                    self.unify_generic(elem, arg_elem, decl_generics, subst, sp)
                } else {
                    self.ctx.err(sp, format!(
                        "argument is `{}`, an array `[..]` expected",
                        self.ctx.type_name(arg_ty)
                    ));
                    Err(())
                }
            }
            TypeKind::TyPath { segs, .. }
                if segs.len() == 1
                    && !segs[0].generics.is_empty()
                    && self
                        .ctx
                        .find_trait(segs[0].name)
                        .map(|t| !t.generics.is_empty())
                        .unwrap_or(false)
                    && !self.free_generics(param_node, decl_generics, subst).is_empty() =>
            {
                // a generic trait's instantiation against a concrete
                // argument (`fn read_id<T>(a: Readable<T>)` over
                // `Source<str>`): the widening's trait args come from the
                // target's registered parameterized trait impl, re-resolved
                // under the target's own substitution (`Readable<T>` →
                // `Readable<str>`, the phase-2 unification) — then the
                // param's argument slots unify against them element-wise,
                // binding `T := str`. A trait-object argument unifies
                // through its own instantiation's args.
                let tname = segs[0].name;
                let arg_nodes = segs[0].generics.clone();
                let concrete: Vec<TypeId> = match self.ctx.types.kind(arg_ty).clone() {
                    TyKind::TraitObj { trait_id } => {
                        let hit = self
                            .ctx
                            .trait_inst
                            .iter()
                            .find(|(_, &id)| id == trait_id)
                            .map(|(k, _)| k.clone());
                        match hit {
                            Some((n, args)) if n == tname => args,
                            _ => {
                                self.ctx.err(sp, format!(
                                    "argument is `{}`, `{}` expected",
                                    self.ctx.type_name(arg_ty),
                                    self.ctx.name(tname)
                                ));
                                return Err(());
                            }
                        }
                    }
                    _ => match self.ctx.template_trait_args(tname, arg_nodes.len(), arg_ty) {
                        Some(args) => args,
                        None => {
                            self.ctx.err(sp, format!(
                                "no impl of `{}` for `{}` — a parameterized trait impl must cover the widening (RFC 0012 §4)",
                                self.ctx.name(tname),
                                self.ctx.type_name(arg_ty)
                            ));
                            return Err(());
                        }
                    },
                };
                if concrete.len() != arg_nodes.len() {
                    self.ctx.err(sp, format!(
                        "`{}` takes {} type argument(s), {} given",
                        self.ctx.name(tname),
                        concrete.len(),
                        arg_nodes.len()
                    ));
                    return Err(());
                }
                for (node, c) in arg_nodes.iter().zip(concrete.into_iter()) {
                    self.unify_generic(*node, c, decl_generics, subst, sp)?;
                }
                Ok(())
            }
            TypeKind::TyPath { .. } => {
                // builtin containers (`opaque`): resolve and compare — the
                // core names, gated on the use statement like everywhere
                // else (an unused name falls through to `resolve_type`,
                // which reports it as not in scope)
                let want = self.ctx.resolve_type(param_node, subst);
                if !self.widens(arg_ty, want) {
                    self.ctx.err(sp, format!(
                        "argument is `{}`, `{}` expected",
                        self.ctx.type_name(arg_ty), self.ctx.type_name(want)
                    ));
                    Err(())
                } else {
                    Ok(())
                }
            }
            TypeKind::TyFn { params, ret } => {
                // fn(P1, P2) -> R against the arg's fn type
                let arg_kind = self.ctx.types.kind(arg_ty).clone();
                if let TyKind::Fn { params: aps, ret: ar } = arg_kind {
                    if params.len() != aps.len() {
                        self.ctx.err(sp, "fn type arity mismatch");
                        return Err(());
                    }
                    for (p, a) in params.iter().zip(aps.iter()) {
                        self.unify_generic(*p, *a, decl_generics, subst, sp)?;
                    }
                    self.unify_generic(ret, ar, decl_generics, subst, sp)} else {
                    self.ctx.err(sp, format!(
                        "expected an fn type, found `{}`",
                        self.ctx.type_name(arg_ty)
                    ));
                    Err(())
                }
            }
            _ => {
                let want = self.ctx.resolve_type(param_node, subst);
                if want != arg_ty {
                    self.ctx.err(sp, format!(
                        "argument is `{}`, `{}` expected",
                        self.ctx.type_name(arg_ty), self.ctx.type_name(want)
                    ));
                    Err(())
                } else {
                    Ok(())
                }
            }
        }
    }
}
