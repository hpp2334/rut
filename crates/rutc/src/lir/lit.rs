//! Composite literals: f-strings (the concat desugaring, RFC 0007 SS2),
//! struct literals (every field initialized, RFC 0009), array literals,
//! and closures (by-value captures; RFC 0013 SS1 with the v1 capture
//! scan).

use crate::check::TcResult;
use rut_core::binary::ConstVal;
use rut_core::ops::*;
use rut_core::types::*;
use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {
    // ---- f-strings: the concat desugaring (RFC 0007 §2) ----

    pub(crate) fn compile_fstr(&mut self, parts: Vec<FPartAst>, _expected: Option<TypeId>, sp: crate::span::Span) -> TcResult<TypeId> {
        let mut part_regs: Vec<u16> = Vec::new();
        for p in &parts {
            match p {
                FPartAst::Lit(s) => {
                    let k = self.konst(ConstVal::Str(s.clone()));
                    let reg = self.new_reg(TY_STR);
                    self.emit(Op::Const { dst: reg, k: k as u32 }, sp.lo);
                    part_regs.push(reg);
                }
                FPartAst::Hole(e) => {
                    let t = self.compile_expr(*e, None)?;
                    self.check_formattable(t, self.ctx.ast.node(*e).span)?;
                    let src = self.last_reg;
                    let sreg = self.new_reg(TY_STR);
                    self.emit(Op::CallNat { nat: Nat::Str, recv: None, args: vec![src], dst: Some(sreg) }, sp.lo);
                    part_regs.push(sreg);
                }
            }
        }
        let dst = self.new_reg(TY_STR);
        self.emit(Op::CallNat { nat: Nat::Concat, recv: None, args: part_regs, dst: Some(dst) }, sp.lo);
        Ok(TY_STR)
    }

    // ---- literals: struct / array ----

    pub(crate) fn compile_struct(&mut self, ty: NodeId, fields: Vec<(IdentId, NodeId)>, _expected: Option<TypeId>, sp: crate::span::Span) -> TcResult<TypeId> {
        // `Self` binds inside class bodies (RFC 0010 §1)
        let sty = self.resolve_type_now(ty);
        let dname = self.ctx.datas.iter().find(|(_, d)| d.ty == sty).map(|(n, _)| *n);
        let Some(dname) = dname else {
            self.ctx.err(sp, format!("`{}` is not a dataclass/class of this module", self.ctx.types.name(sty)));
            return Err(());
        };
        let d = self.ctx.find_data(dname).cloned().unwrap();
        // classes have no outside literal (RFC 0010 §1); the Self {} literal
        // is legal only inside the class body
        let inside_body = self.current_class == Some(dname);
        if d.kind == crate::check::DataKind::Class && !inside_body {
            self.ctx.err(sp, format!(
                "classes have no instance literal —construct through a class method (`{}.new(..)`, RFC 0010 §1)",
                self.ctx.name(dname)
            ));
            return Err(());
        }
        // every field initialized (any order, by name) or has an initializer
        // (RFC 0009)
        let mut set: Vec<bool> = vec![false; d.fields.len()];
        let cell = self.new_reg(sty);
        self.emit(Op::NewCell { dst: cell, ty: sty }, sp.lo);
        for (fname, v) in &fields {
            let Some(fidx) = d.fields.iter().position(|(n, _, _, _)| n == fname) else {
                self.ctx.err(self.ctx.ast.node(*v).span, format!(
                    "`{}` has no field `{}`", self.ctx.name(dname), self.ctx.name(*fname)
                ));
                return Err(());
            };
            let fty = d.fields[fidx].1;
            let t = self.compile_expr(*v, Some(fty))?;
            if t != fty {
                self.ctx.err(self.ctx.ast.node(*v).span, format!(
                    "field `{}` is `{}`, found `{}`",
                    self.ctx.name(*fname), self.ctx.types.name(fty), self.ctx.types.name(t)
                ));
            }
            self.emit(Op::SetF { obj: cell, field: fidx as u32, val: self.last_reg }, sp.lo);
            set[fidx] = true;
        }
        for (fidx, (fname, fty, init, _)) in d.fields.iter().enumerate() {
            if !set[fidx] {
                let Some(init) = init else {
                    self.ctx.err(sp, format!(
                        "literal must initialize every field —`{}` is missing (RFC 0009)",
                        self.ctx.name(*fname)
                    ));
                    return Err(());
                };
                let t = self.compile_expr(*init, Some(*fty))?;
                if t != *fty {
                    self.ctx.err(self.ctx.ast.node(*init).span, "field initializer type mismatch");
                }
                self.emit(Op::SetF { obj: cell, field: fidx as u32, val: self.last_reg }, sp.lo);
            }
        }
        // the value lives in `cell` (allocated before the SetFs); move it out
        // so last_reg holds it
        let out = self.new_reg(sty);
        self.emit(Op::MovRef { dst: out, src: cell }, sp.lo);
        Ok(sty)
    }

    pub(crate) fn compile_array_lit(&mut self, elems: Vec<NodeId>, expected: Option<TypeId>, sp: crate::span::Span) -> TcResult<TypeId> {
        // `[e1, .., en] : Array<T, n>` (RFC 0007 §1); T from expected or the
        // first element; uncontextualized int elements default to i32
        let elem_hint = match expected.map(|e| self.ctx.types.kind(e).clone()) {
            Some(TyKind::Array { elem, .. }) | Some(TyKind::Vec { elem }) => Some(elem),
            _ => None,
        };
        let mut eregs = Vec::new();
        let mut ety = elem_hint;
        for (i, e) in elems.iter().enumerate() {
            let t = self.compile_expr(*e, if i == 0 { ety } else { ety })?;
            if i == 0 {
                // keep the expected element when the first element widens to it
                ety = match ety {
                    Some(u) if self.widens(t, u) => Some(u),
                    _ => Some(t),
                };
            } else if let Some(u) = ety {
                // RFC 0012 §2: heterogeneous elements unify through `dyn`
                if self.widens(u, t) {
                    ety = Some(t);
                } else if !self.widens(t, u) {
                    self.ctx.err(self.ctx.ast.node(*e).span, "array literal elements must agree on one type");
                }
            }
            eregs.push(self.last_reg);
        }
        let elem = ety.unwrap_or(TY_I32);
        let aty = self.ctx.mk_array(elem, elems.len() as u32);
        let dst = self.new_reg(aty);
        self.emit(Op::ArrLit { dst, ty: aty, elems: eregs }, sp.lo);
        Ok(aty)
    }

    // ---- closures (RFC 0013 §1 —v1 captures by value) ----

    pub(crate) fn compile_lambda(
        &mut self,
        lambda_node: NodeId,
        params: Vec<NodeId>,
        ret: Option<NodeId>,
        body: NodeId,
        expected: Option<TypeId>,
        sp: crate::span::Span,
    ) -> TcResult<TypeId> {
        let (eptys, eret) = match expected.map(|e| self.ctx.types.kind(e).clone()) {
            Some(TyKind::Fn { params, ret }) => (params, ret),
            _ => (Vec::new(), TY_VOID),
        };
        // param types: annotations first, then the expected fn type
        let mut param_tys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let ty = match &self.ctx.ast.node(*p).kind {
                NodeKind::Param { ty: Some(t), .. } => self.resolve_type_now(*t),
                NodeKind::Param { ty: None, .. } => {
                    if let Some(&t) = eptys.get(i) {
                        t
                    } else {
                        self.ctx.err(self.ctx.ast.node(*p).span, "lambda parameter needs a type annotation (or an expected fn type)");
                        TY_I32
                    }
                }
                _ => {
                    self.ctx.err(self.ctx.ast.node(*p).span, "lambdas take no `self`");
                    TY_I32
                }
            };
            param_tys.push(ty);
        }
        let ptys = param_tys;
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(eret);
        // capture scan: free names that resolve to enclosing locals
        let mut referenced = Vec::new();
        self.scan_names(body, &mut referenced);
        let lambda_param_names: Vec<IdentId> = params
            .iter()
            .filter_map(|p| match &self.ctx.ast.node(*p).kind {
                NodeKind::Param { name, .. } => Some(*name),
                _ => None,
            })
            .collect();
        let mut caps: Vec<(IdentId, TypeId, u16)> = Vec::new();
        for n in referenced {
            if lambda_param_names.contains(&n) {
                continue;
            }
            if let Some(l) = self.lookup(n).copied() {
                // copy the CURRENT value into a capture register (by value)
                let cap_reg = self.new_reg(l.ty);
                if self.ctx.types.is_ref(l.ty) {
                    self.emit(Op::MovRef { dst: cap_reg, src: l.reg }, sp.lo);
                } else {
                    self.emit(Op::Mov { dst: cap_reg, src: l.reg }, sp.lo);
                }
                caps.push((n, l.ty, cap_reg));
            }
        }
        // register the synthetic fn: params = declared ++ captures
        let node_id = self.ctx.ast.node(body).span.lo; // not unique per node —use body NodeId instead
        let _ = node_id;
        // the Lambda node id: find it by body —the caller passes parts; the
        // lambda node is the PARENT of body. Store captures keyed by the
        // body node; FnKey::Lambda uses the body node id (unique).
        // record the resolved signature for the body compilation
        self.ctx.lambda_sigs.insert(lambda_node, (ptys.clone(), ret_ty));
        self.ctx.lambda_info.insert(lambda_node, caps.iter().map(|(n, t, _)| (*n, *t)).collect());
        let inst = crate::check::Inst { key: crate::check::FnKey::Lambda(lambda_node), subst: vec![] };
        let fid = self.ctx.ensure_inst(inst);
        let fty = self.ctx.mk_fn_ty(ptys.clone(), ret_ty);
        let dst = self.new_reg(fty);
        self.emit(
            Op::MakeClosure { dst, func: fid, captures: caps.iter().map(|(_, _, r)| *r).collect() },
            sp.lo,
        );
        Ok(fty)
    }

    /// collect every single-segment path name under `node` (capture scan)
    pub(crate) fn scan_names(&mut self, node: NodeId, out: &mut Vec<IdentId>) {
        if out.len() > 4096 {
            return;
        }
        let kind = self.ctx.ast.node(node).kind.clone();
        let kids = |n: NodeId, out: &mut Vec<IdentId>, s: &mut Self| s.scan_names(n, out);
        match kind {
            NodeKind::Path { segs } => {
                if segs.len() == 1 && out.len() < 4096 && !out.contains(&segs[0].name) {
                    out.push(segs[0].name);
                }
            }
            NodeKind::Lit(_)
            | NodeKind::Break
            | NodeKind::Continue
            | NodeKind::PatWild
            | NodeKind::PatElse
            | NodeKind::PatLit(_)
            | NodeKind::PatPath { .. }
            | NodeKind::PatCtor { .. }
            | NodeKind::TyPath { .. }
            | NodeKind::TyFn { .. }
            | NodeKind::TyConst(_)
            | NodeKind::Import { .. }
            | NodeKind::ModuleLet { .. }
            | NodeKind::Enum { .. }
            | NodeKind::Dataclass { .. }
            | NodeKind::Class { .. }
            | NodeKind::Trait { .. }
            | NodeKind::Impl { .. }
            | NodeKind::Fn { .. }
            | NodeKind::SurfaceFn { .. }
            | NodeKind::SurfaceClass { .. }
            | NodeKind::Module { .. }
            | NodeKind::FieldDecl { .. }
            | NodeKind::MethodDecl { .. }
            | NodeKind::Param { .. }
            | NodeKind::SelfParam { .. } => {}
            NodeKind::Block { stmts } => {
                for s in stmts {
                    kids(s, out, self);
                }
            }
            NodeKind::LetStmt { init, .. } => kids(init, out, self),
            NodeKind::If { cond, then, els } => {
                kids(cond, out, self);
                kids(then, out, self);
                if let Some(e) = els {
                    kids(e, out, self);
                }
            }
            NodeKind::While { cond, body } => {
                kids(cond, out, self);
                kids(body, out, self);
            }
            NodeKind::ForOf { iter, body, .. } => {
                kids(iter, out, self);
                kids(body, out, self);
            }
            NodeKind::ForC { init, cond, update, body, .. } => {
                kids(init, out, self);
                kids(cond, out, self);
                kids(update, out, self);
                kids(body, out, self);
            }
            NodeKind::Return { value } => {
                if let Some(v) = value {
                    kids(v, out, self);
                }
            }
            NodeKind::WhenStmt { scrut, arms } | NodeKind::WhenExpr { scrut, arms } => {
                kids(scrut, out, self);
                for a in arms {
                    kids(a, out, self);
                }
            }
            NodeKind::ExprStmt(e) => kids(e, out, self),
            NodeKind::WhenArm { pats, body } => {
                for p in pats {
                    kids(p, out, self);
                }
                kids(body, out, self);
            }
            NodeKind::SelectArm { fut, body, .. } => {
                kids(fut, out, self);
                kids(body, out, self);
            }
            NodeKind::Call { callee, args } => {
                kids(callee, out, self);
                for a in args {
                    kids(a, out, self);
                }
            }
            NodeKind::Method { recv, args, generics, .. } => {
                kids(recv, out, self);
                for a in args {
                    kids(a, out, self);
                }
                for g in generics {
                    kids(g, out, self);
                }
            }
            NodeKind::Field { recv, .. } => kids(recv, out, self),
            NodeKind::Index { recv, idx } => {
                kids(recv, out, self);
                kids(idx, out, self);
            }
            NodeKind::Unary { expr, .. } => kids(expr, out, self),
            NodeKind::Binary { lhs, rhs, .. } => {
                kids(lhs, out, self);
                kids(rhs, out, self);
            }
            NodeKind::Assign { target, value, .. } => {
                kids(target, out, self);
                kids(value, out, self);
            }
            NodeKind::Lambda { body: b, .. } => kids(b, out, self),
            NodeKind::Try { expr } => kids(expr, out, self),
            NodeKind::Await { expr } => kids(expr, out, self),
            NodeKind::Select { arms } => {
                for a in arms {
                    kids(a, out, self);
                }
            }
            NodeKind::Is { expr, ty } => {
                kids(expr, out, self);
                kids(ty, out, self);
            }
            NodeKind::FStr { parts } => {
                for p in parts {
                    if let FPartAst::Hole(e) = p {
                        kids(e, out, self);
                    }
                }
            }
            NodeKind::Struct { fields, .. } => {
                for (_, v) in fields {
                    kids(v, out, self);
                }
            }
            NodeKind::ArrayLit { elems } => {
                for e in elems {
                    kids(e, out, self);
                }
            }
        }
    }
}
