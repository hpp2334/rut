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

    pub(crate) fn compile_fstr(&mut self, parts: Vec<FPartAst>, _expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
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
                    self.check_formattable(t, self.ctx.ast.span(e.id()))?;
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

    pub(crate) fn compile_struct(&mut self, ty: NodeHandle<AnyTy>, fields: Vec<(IdentId, NodeHandle<AnyExpr>)>, _expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
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
        // (RFC 0009); collect each field value in a register, then mint the
        // whole record with one MakeRecord (no NewCell/SetF/MovRef sequence)
        let mut val_regs: Vec<Option<u16>> = vec![None; d.fields.len()];
        let mut set: Vec<bool> = vec![false; d.fields.len()];
        for (fname, v) in &fields {
            let Some(fidx) = d.fields.iter().position(|(n, _, _, _)| n == fname) else {
                self.ctx.err(self.ctx.ast.span(v.id()), format!(
                    "`{}` has no field `{}`", self.ctx.name(dname), self.ctx.name(*fname)
                ));
                return Err(());
            };
            let fty = d.fields[fidx].1;
            let t = self.compile_expr(*v, Some(fty))?;
            if t != fty {
                self.ctx.err(self.ctx.ast.span(v.id()), format!(
                    "field `{}` is `{}`, found `{}`",
                    self.ctx.name(*fname), self.ctx.types.name(fty), self.ctx.types.name(t)
                ));
            }
            val_regs[fidx] = Some(self.last_reg);
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
                    self.ctx.err(self.ctx.ast.span(init.id()), "field initializer type mismatch");
                }
                val_regs[fidx] = Some(self.last_reg);
            }
        }
        let vals: Vec<u16> = val_regs
            .into_iter()
            .map(|r| r.expect("checked: every field is initialized"))
            .collect();
        let dst = self.new_reg(sty);
        self.emit(Op::MakeRecord { dst, ty: sty, vals }, sp.lo);
        Ok(sty)
    }

    pub(crate) fn compile_array_lit(&mut self, elems: Vec<NodeHandle<AnyExpr>>, expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // `[e1, .., en] : Array<T>` (RFC 0007 §1); T from expected or the
        // first element; uncontextualized int elements default to i32
        let elem_hint = match expected.map(|e| self.ctx.types.kind(e).clone()) {
            Some(TyKind::Array { elem }) | Some(TyKind::Vec { elem }) => Some(elem),
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
                    self.ctx.err(self.ctx.ast.span(e.id()), "array literal elements must agree on one type");
                }
            }
            eregs.push(self.last_reg);
        }
        let elem = ety.unwrap_or(TY_I32);
        let aty = self.ctx.mk_array(elem);
        let dst = self.new_reg(aty);
        self.emit(Op::ArrLit { dst, ty: aty, elems: eregs }, sp.lo);
        Ok(aty)
    }

    // ---- closures (RFC 0013 §1 —v1 captures by value) ----

    pub(crate) fn compile_lambda(
        &mut self,
        lambda_node: NodeId,
        params: Vec<NodeHandle<AnyParam>>,
        ret: Option<NodeHandle<AnyTy>>,
        body: NodeHandle<AnyExpr>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let (eptys, eret) = match expected.map(|e| self.ctx.types.kind(e).clone()) {
            Some(TyKind::Fn { params, ret }) => (params, ret),
            _ => (Vec::new(), TY_UNIT),
        };
        // param types: annotations first, then the expected fn type
        let mut param_tys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let ty = match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => self.resolve_type_now(*t),
                MemberKind::Param(ParamData { ty: None, .. }) => {
                    if let Some(&t) = eptys.get(i) {
                        t
                    } else {
                        self.ctx.err(self.ctx.ast.span(p.id()), "lambda parameter needs a type annotation (or an expected fn type)");
                        TY_I32
                    }
                }
                _ => {
                    self.ctx.err(self.ctx.ast.span(p.id()), "lambdas take no `self`");
                    TY_I32
                }
            };
            param_tys.push(ty);
        }
        let ptys = param_tys;
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(eret);
        // capture scan: free names that resolve to enclosing locals
        let mut referenced = Vec::new();
        self.scan_names(body.id(), &mut referenced);
        let lambda_param_names: Vec<IdentId> = params
            .iter()
            .filter_map(|p| match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { name, .. }) => Some(*name),
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
                let node_id = self.ctx.ast.span(body.id()).lo; // not unique per node —use body NodeId instead
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

    /// collect every single-segment path name under `node` (capture scan).
    /// Generic walk over the arena by `NodeId` — it must cross statement and
    /// expression categories freely; type subtrees carry no value names.
    pub(crate) fn scan_names(&mut self, node: NodeId, out: &mut Vec<IdentId>) {
        if out.len() > 4096 {
            return;
        }
        let kids = |n: NodeId, out: &mut Vec<IdentId>, s: &mut Self| s.scan_names(n, out);
        match self.ctx.ast.kind(node).clone() {
            // items / members / patterns / types carry no value names
            Kind::Item(_) | Kind::Member(_) | Kind::Pat(_) | Kind::Type(_) => {}
            Kind::Stmt(StmtKind::LetStmt { init, .. }) => kids(init.id(), out, self),
            Kind::Stmt(StmtKind::If { cond, then, els }) => {
                kids(cond.id(), out, self);
                kids(then.id(), out, self);
                if let Some(e) = els {
                    match e {
                        ElseBranch::If(h) => kids(h.id(), out, self),
                        ElseBranch::Block(h) => kids(h.id(), out, self),
                    }
                }
            }
            Kind::Stmt(StmtKind::While { cond, body }) => {
                kids(cond.id(), out, self);
                kids(body.id(), out, self);
            }
            Kind::Stmt(StmtKind::ForOf { iter, body, .. }) => {
                kids(iter.id(), out, self);
                kids(body.id(), out, self);
            }
            Kind::Stmt(StmtKind::ForC { init, cond, update, body, .. }) => {
                kids(init.id(), out, self);
                kids(cond.id(), out, self);
                kids(update.id(), out, self);
                kids(body.id(), out, self);
            }
            Kind::Stmt(StmtKind::Return { value }) => {
                if let Some(v) = value {
                    kids(v.id(), out, self);
                }
            }
            Kind::Stmt(StmtKind::WhenStmt { scrut, arms }) => {
                kids(scrut.id(), out, self);
                for a in arms {
                    kids(a.id(), out, self);
                }
            }
            Kind::Stmt(StmtKind::ExprStmt(e)) => kids(e.id(), out, self),
            Kind::Stmt(StmtKind::Break | StmtKind::Continue) => {}
            Kind::Arm(ArmKind::WhenArm { pats, body }) => {
                for p in pats {
                    kids(p.id(), out, self);
                }
                kids(body.id(), out, self);
            }
            Kind::Arm(ArmKind::SelectArm { fut, body, .. }) => {
                kids(fut.id(), out, self);
                kids(body.id(), out, self);
            }
            Kind::Expr(ExprKind::Block { stmts }) => {
                for s in stmts {
                    kids(s.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::Path { segs }) => {
                if segs.len() == 1 && out.len() < 4096 && !out.contains(&segs[0].name) {
                    out.push(segs[0].name);
                }
            }
            Kind::Expr(ExprKind::Lit(_)) => {}
            Kind::Expr(ExprKind::Call { callee, args }) => {
                kids(callee.id(), out, self);
                for a in args {
                    kids(a.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::Method { recv, args, .. }) => {
                kids(recv.id(), out, self);
                for a in args {
                    kids(a.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::Field { recv, .. }) => kids(recv.id(), out, self),
            Kind::Expr(ExprKind::Index { recv, idx }) => {
                kids(recv.id(), out, self);
                kids(idx.id(), out, self);
            }
            Kind::Expr(ExprKind::Unary { expr, .. }) => kids(expr.id(), out, self),
            Kind::Expr(ExprKind::Binary { lhs, rhs, .. }) => {
                kids(lhs.id(), out, self);
                kids(rhs.id(), out, self);
            }
            Kind::Expr(ExprKind::Assign { target, value, .. }) => {
                kids(target.id(), out, self);
                kids(value.id(), out, self);
            }
            Kind::Expr(ExprKind::Lambda { body: b, .. }) => kids(b.id(), out, self),
            Kind::Expr(ExprKind::Try { expr }) | Kind::Expr(ExprKind::Await { expr }) => kids(expr.id(), out, self),
            Kind::Expr(ExprKind::Select { arms }) => {
                for a in arms {
                    kids(a.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::Is { expr, .. }) => kids(expr.id(), out, self),
            Kind::Expr(ExprKind::FStr { parts }) => {
                for p in parts {
                    if let FPartAst::Hole(e) = p {
                        kids(e.id(), out, self);
                    }
                }
            }
            Kind::Expr(ExprKind::Struct { fields, .. }) => {
                for (_, v) in fields {
                    kids(v.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::ArrayLit { elems }) => {
                for e in elems {
                    kids(e.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::WhenExpr { scrut, arms }) => {
                kids(scrut.id(), out, self);
                for a in arms {
                    kids(a.id(), out, self);
                }
            }
        }
    }
}
