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
            _ => {}
        }
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
                                self.ctx.types.name(prev),
                                self.ctx.types.name(arg_ty)
                            ));
                            return Err(());
                        }
                    }
                    bind(name, arg_ty, subst);
                    return Ok(());
                }
                // non-generic: resolve and compare
                let want = self.ctx.resolve_type(param_node, subst);
                if want != arg_ty {
                    self.ctx.err(sp, format!(
                        "argument is `{}`, `{}` expected",
                        self.ctx.types.name(arg_ty), self.ctx.types.name(want)
                    ));
                    return Err(());
                }
                Ok(())
            }
            TypeKind::TyPath { segs, .. } => {
                // builtin containers: unify element-wise
                let head = self.ctx.name(segs[0].name);
                let arg_kind = self.ctx.types.kind(arg_ty).clone();
                match (head, arg_kind) {
                    ("Vec", TyKind::Vec { elem }) if segs[0].generics.len() == 1 => {
                        self.unify_generic(segs[0].generics[0], elem, decl_generics, subst, sp)
                    }
                    ("Option", TyKind::Option { elem }) if segs[0].generics.len() == 1 => {
                        self.unify_generic(segs[0].generics[0], elem, decl_generics, subst, sp)
                    }
                    ("Result", TyKind::Result { ok, err })
                        if segs[0].generics.len() == 2 =>
                    {
                        self.unify_generic(segs[0].generics[0], ok, decl_generics, subst, sp)?;
                        self.unify_generic(segs[0].generics[1], err, decl_generics, subst, sp)
                    }
                    ("Array", TyKind::Array { elem }) if segs[0].generics.len() == 1 => {
                        self.unify_generic(segs[0].generics[0], elem, decl_generics, subst, sp)
                    }
                    _ => {
                        let want = self.ctx.resolve_type(param_node, subst);
                        if want != arg_ty {
                            self.ctx.err(sp, format!(
                                "argument is `{}`, `{}` expected",
                                self.ctx.types.name(arg_ty), self.ctx.types.name(want)
                            ));
                            Err(())
                        } else {
                            Ok(())
                        }
                    }
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
                        self.ctx.types.name(arg_ty)
                    ));
                    Err(())
                }
            }
            _ => {
                let want = self.ctx.resolve_type(param_node, subst);
                if want != arg_ty {
                    self.ctx.err(sp, format!(
                        "argument is `{}`, `{}` expected",
                        self.ctx.types.name(arg_ty), self.ctx.types.name(want)
                    ));
                    Err(())
                } else {
                    Ok(())
                }
            }
        }
    }
}
