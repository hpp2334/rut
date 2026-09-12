//! Statement compilation: blocks with scoped locals, if/while/for-c/for-of,
//! when statements (pattern-test chains), exhaustiveness, and the
//! expression-depth budget.

use crate::check::{float_suffix_ty, int_suffix_ty, TcResult};
use rut_core::ops::*;
use rut_core::types::*;
use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {
    // ---- statements ----

    pub(crate) fn compile_block(&mut self, h: impl Into<NodeId>) -> TcResult<()> {
        let stmts = match self.ctx.ast.kind(h.into()) {
            Kind::Expr(ExprKind::Block { stmts }) => stmts,
            _ => return Ok(()),
        };
        let stmts = stmts.clone();
        let base = self.locals.len();
        for s in stmts {
            self.compile_stmt(s)?;
        }
        self.locals.truncate(base);
        Ok(())
    }

    pub(crate) fn compile_stmt(&mut self, node: NodeHandle<AnyStmt>) -> TcResult<()> {
        let sp = self.ctx.ast.span(node.id());
        self.span = sp.lo;
        match self.ctx.ast.stmt(node).clone() {
            StmtKind::LetStmt { is_mut, name, ty, init } => {
                let expected = ty.map(|t| self.resolve_type_now(t));
                let t = self.compile_expr(init, expected)?;
                if let Some(e) = expected {
                    if !self.widens(t, e) {
                        self.ctx.err(sp, format!(
                            "let `{}` is `{}` but the initializer is `{}`",
                            self.ctx.name(name), self.ctx.types.name(e), self.ctx.types.name(t)
                        ));
                    }
                }
                let ty = expected.unwrap_or(t);
                // bind the local directly to the initializer's register
                self.locals.push(Local { name, reg: self.last_reg, ty, is_mut, loop_var: false });
                Ok(())
            }
            StmtKind::If { cond, then, els } => {
                self.compile_expr(cond, Some(TY_BOOL))?;
                let cond_reg = self.last_reg;
                let l_then = self.new_label();
                let l_end = self.new_label();
                let has_else = els.is_some();
                let else_label = if has_else { self.new_label() } else { l_end };
                self.br(cond_reg, l_then, else_label);
                self.bind(l_then);
                self.compile_block(then)?;
                self.jmp(l_end);
                if let Some(e) = els {
                    self.bind(else_label);
                    match e {
                        ElseBranch::If(h) => self.compile_stmt(h.into())?,
                        ElseBranch::Block(b) => {
                            self.compile_block(b)?;
                        }
                    }
                }
                self.bind(l_end);
                Ok(())
            }
            StmtKind::While { cond, body } => {
                let l_head = self.new_label();
                let l_body = self.new_label();
                let l_end = self.new_label();
                self.bind(l_head);
                self.emit(Op::LoopHead, sp.lo);
                self.compile_expr(cond, Some(TY_BOOL))?;
                let cond_reg = self.last_reg;
                self.br(cond_reg, l_body, l_end);
                self.bind(l_body);
                self.loops.push((l_head, l_end));
                self.compile_block(body)?;
                self.loops.pop();
                self.jmp(l_head);
                self.bind(l_end);
                Ok(())
            }
            StmtKind::ForOf { var, iter, body } => self.compile_for_of(node.id(), var, iter, body, sp),
            StmtKind::ForC { var, init, cond, update, body } => {
                // induction var is loop-owned (RFC 0008 §1); bind directly
                // to the initializer's register
                let t = self.compile_expr(init, None)?;
                let reg = self.last_reg;
                self.locals.push(Local { name: var, reg, ty: t, is_mut: true, loop_var: true });
                let l_head = self.new_label();
                let l_body = self.new_label();
                let l_end = self.new_label();
                self.bind(l_head);
                self.emit(Op::LoopHead, sp.lo);
                self.compile_expr(cond, Some(TY_BOOL))?;
                let cond_reg = self.last_reg;
                self.br(cond_reg, l_body, l_end);
                self.bind(l_body);
                let l_cont = self.new_label();
                self.loops.push((l_cont, l_end));
                self.compile_block(body)?;
                self.loops.pop();
                self.bind(l_cont);
                self.compile_expr(update, None)?;
                self.jmp(l_head);
                self.bind(l_end);
                Ok(())
            }
            StmtKind::Return { value } => {
                match value {
                    Some(v) => {
                        let t = self.compile_expr(v, Some(self.ret_ty))?;
                        if t != self.ret_ty {
                            self.ctx.err(sp, format!(
                                "return type mismatch: `{}` expected, `{}` returned",
                                self.ctx.types.name(self.ret_ty), self.ctx.types.name(t)
                            ));
                        }
                        self.emit(Op::Ret { val: Some(self.last_reg) }, sp.lo);
                    }
                    None => {
                        if self.ret_ty != TY_UNIT {
                            self.ctx.err(sp, "missing return value");
                        }
                        self.emit(Op::Ret { val: None }, sp.lo);
                    }
                }
                Ok(())
            }
            StmtKind::WhenStmt { scrut, arms } => {
                self.compile_when(scrut, &arms, None, sp)?;
                Ok(())
            }
            StmtKind::Break => {
                let Some((_, brk)) = self.loops.last().copied() else {
                    self.ctx.err(sp, "`break` outside a loop");
                    return Ok(());
                };
                self.jmp(brk);
                Ok(())
            }
            StmtKind::Continue => {
                let Some((cont, _)) = self.loops.last().copied() else {
                    self.ctx.err(sp, "`continue` outside a loop");
                    return Ok(());
                };
                self.jmp(cont);
                Ok(())
            }
            StmtKind::ExprStmt(e) => {
                self.compile_expr(e, None)?;
                Ok(())
            }
        }
    }

    pub(crate) fn compile_for_of(&mut self, node: NodeId, var: IdentId, iter: NodeHandle<AnyExpr>, body: NodeHandle<BlockNode>, sp: rut_lexer::span::Span) -> TcResult<()> {
        let it = self.compile_expr(iter, None)?;
        let iter_reg = self.last_reg;
        let elem_ty = match self.ctx.types.kind(it).clone() {
            TyKind::Vec { elem } => elem,
            TyKind::Array { elem, .. } => elem,
            TyKind::Str => TY_CHAR,
            _ => {
                self.ctx.err(sp, format!(
                    "`for (let .. of ..)` needs a Vec, Array, or string —found `{}`",
                    self.ctx.types.name(it)
                ));
                return Err(());
            }
        };
        let idx = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: idx, bits: 0 }, sp.lo);
        let var_reg = self.new_reg(elem_ty);
        let l_head = self.new_label();
        let l_body = self.new_label();
        let l_end = self.new_label();
        let l_cont = self.new_label();
        self.bind(l_head);
        self.emit(Op::LoopHead, sp.lo);
        // len
        let len_reg = self.new_reg(TY_I32);
        match self.ctx.types.kind(it).clone() {
            TyKind::Vec { .. } => {
                self.emit(Op::CallNat { nat: Nat::VecLen, recv: Some(iter_reg), args: vec![], dst: Some(len_reg) }, sp.lo);
            }
            TyKind::Array { len, .. } => {
                self.emit(Op::ConstRaw { dst: len_reg, bits: len as u64 }, sp.lo);
            }
            TyKind::Str => {
                self.emit(Op::CallNat { nat: Nat::StrLen, recv: Some(iter_reg), args: vec![], dst: Some(len_reg) }, sp.lo);
            }
            _ => {}
        }
        let cond_reg = self.new_reg(TY_BOOL);
        self.emit(Op::Cmp { op: CmpOp::Lt, prim: PrimTy::I32, dst: cond_reg, a: idx, b: len_reg }, sp.lo);
        self.br(cond_reg, l_body, l_end);
        self.bind(l_body);
        // var = iter[idx]
        match self.ctx.types.kind(it).clone() {
            TyKind::Str => self.emit(Op::StrCharAt { dst: var_reg, s: iter_reg, idx }, sp.lo),
            _ => self.emit(Op::ArrGet { dst: var_reg, arr: iter_reg, idx, repr: self.ctx.types.repr_of(elem_ty) }, sp.lo),
        }
        self.locals.push(Local { name: var, reg: var_reg, ty: elem_ty, is_mut: false, loop_var: false });
        self.loops.push((l_cont, l_end));
        self.compile_block(body)?;
        self.loops.pop();
        self.locals.pop();
        self.bind(l_cont);
        let one = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: one, bits: 1 }, sp.lo);
        let next = self.new_reg(TY_I32);
        self.emit(Op::Arith { op: ArithOp::Add, prim: PrimTy::I32, dst: next, a: idx, b: one }, sp.lo);
        self.emit(Op::Mov { dst: idx, src: next }, sp.lo);
        self.jmp(l_head);
        self.bind(l_end);
        let _ = node;
        Ok(())
    }

    // ---- `when` (RFC 0008) ----

    pub(crate) fn compile_when(
        &mut self,
        scrut: NodeHandle<AnyExpr>,
        arms: &[NodeHandle<AnyArm>],
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let st = self.compile_expr(scrut, None)?;
        let scrut_reg = self.last_reg;
        // arm type unification (statement-when: unit)
        let result_ty = if let Some(e) = expected {
            e
        } else {
            // first expression-arm's type
            let mut t = TY_UNIT;
            for a in arms {
                if let ArmKind::WhenArm { body, .. } = self.ctx.ast.arm(*a) {
                    t = self.expr_type_hint(*body);
                    break;
                }
            }
            t
        };
        let result_reg = if result_ty != TY_UNIT {
            Some(self.new_reg(result_ty))
        } else {
            None
        };
        let l_end = self.new_label();
        // exhaustiveness + duplicates (RFC 0008 §2)
        self.check_when_exhaustive(st, arms, sp)?;
        for arm in arms {
            let arm = *arm;
            let ArmKind::WhenArm { pats, body } = self.ctx.ast.arm(arm) else { continue };
            let l_arm = self.new_label();
            let l_next_arm = self.new_label();
            // alternatives chain within the arm: `1, 2, 3 -> ..`
            for (pi, p) in pats.iter().enumerate() {
                let matched = self.compile_pattern_test(*p, st, scrut_reg, sp)?;
                let last = pi == pats.len() - 1;
                if last {
                    self.br(matched, l_arm, l_next_arm);
                } else {
                    let l_next_pat = self.new_label();
                    self.br(matched, l_arm, l_next_pat);
                    self.bind(l_next_pat);
                }
            }
            self.bind(l_arm);
            // unit arms may be statement blocks (`-> { a(); b(); }`,
            // RFC 0008 §1) — blocks aren't value expressions in this
            // build, so compile them as scoped statement blocks
            let body_is_block = matches!(self.ctx.ast.expr(*body), ExprKind::Block { .. });
            let bt = if body_is_block && result_ty == TY_UNIT {
                self.compile_block(*body)?;
                TY_UNIT
            } else {
                self.compile_expr(*body, if result_ty != TY_UNIT { Some(result_ty) } else { None })?
            };
            if result_ty != TY_UNIT {
                if bt != result_ty {
                    self.ctx.err(self.ctx.ast.span(body.id()), format!(
                        "`when` arms must agree: `{}` vs `{}`",
                        self.ctx.types.name(result_ty), self.ctx.types.name(bt)
                    ));
                }
                let rr = result_reg.unwrap();
                if self.ctx.types.is_ref(result_ty) {
                    self.emit(Op::MovRef { dst: rr, src: self.last_reg }, sp.lo);
                } else {
                    self.emit(Op::Mov { dst: rr, src: self.last_reg }, sp.lo);
                }
            }
            self.jmp(l_end);
            self.bind(l_next_arm);
        }
        self.bind(l_end);
        if let Some(rr) = result_reg {
            // the when's value moves into a fresh register so the "value in
            // last_reg" convention holds for the ENCLOSING expression
            let out = self.new_reg(result_ty);
            if self.ctx.types.is_ref(result_ty) {
                self.emit(Op::MovRef { dst: out, src: rr }, sp.lo);
            } else {
                self.emit(Op::Mov { dst: out, src: rr }, sp.lo);
            }
            Ok(result_ty)
        } else {
            Ok(TY_UNIT)
        }
    }

    pub(crate) fn expr_type_hint(&mut self, node: NodeHandle<AnyExpr>) -> TypeId {
        // best-effort type for when-arm unification without full inference:
        // literals only; otherwise first arm decides later via compile
        match self.ctx.ast.expr(node) {
            ExprKind::Lit(Lit::Int(_, s)) => s.map(int_suffix_ty).unwrap_or(TY_I32),
            ExprKind::Lit(Lit::Float(_, s)) => s.map(float_suffix_ty).unwrap_or(TY_F64),
            ExprKind::Lit(Lit::Str(_) | Lit::RawStr(_)) => TY_STR,
            ExprKind::Lit(Lit::Bool(_)) => TY_BOOL,
            ExprKind::Lit(Lit::Char(_)) => TY_CHAR,
            ExprKind::Block { .. } => TY_UNIT,
            ExprKind::Struct { ty, .. } => self.resolve_type_now(*ty),
            _ => TY_UNIT,
        }
    }

    pub(crate) fn check_when_exhaustive(&mut self, scrut_ty: TypeId, arms: &[NodeHandle<AnyArm>], sp: rut_lexer::span::Span) -> TcResult<()> {
        let mut has_else = false;
        let mut seen: Vec<String> = Vec::new();
        for a in arms {
            if let ArmKind::WhenArm { pats, .. } = self.ctx.ast.arm(*a) {
                for p in pats {
                    let key = match self.ctx.ast.pat(*p) {
                        PatKind::PatElse => {
                            has_else = true;
                            "else".to_string()
                        }
                        PatKind::PatLit(l) => format!("{:?}", l),
                        PatKind::PatPath { segs } => segs
                            .iter()
                            .map(|s| self.ctx.name(s.name).to_string())
                            .collect::<Vec<_>>()
                            .join("."),
                        PatKind::PatCtor { segs, .. } => segs
                            .iter()
                            .map(|s| self.ctx.name(s.name).to_string())
                            .collect::<Vec<_>>()
                            .join("."),
                        PatKind::PatWild => "_".to_string(),
                    };
                    if seen.contains(&key) && key != "else" {
                        self.ctx.err(self.ctx.ast.span(p.id()), format!("duplicate pattern `{key}` (RFC 0008 §2)"));
                    }
                    seen.push(key);
                }
            }
        }
        if !has_else {
            if let TyKind::Enum { members } = self.ctx.types.kind(scrut_ty).clone() {
                for (m, _v) in &members {
                    if !seen.iter().any(|s| s.ends_with(&format!(".{m}")) || s == m) {
                        self.ctx.err(
                            sp,
                            format!(
                                "`when` over an enum must be exhaustive —`{m}` is not covered and there is no `else` arm (RFC 0008 §2)"
                            ),
                        );
                        return Err(());
                    }
                }
            } else if scrut_ty != TY_BOOL {
                self.ctx.err(
                    sp,
                    format!(
                        "`when` over `{}` needs an `else` arm —only enums are enumerable (RFC 0008 §2)",
                        self.ctx.types.name(scrut_ty)
                    ),
                );
                return Err(());
            } else {
                // bool: covered iff true and false appear
                if !(seen.contains(&"Bool(true)".to_string()) && seen.contains(&"Bool(false)".to_string())) {
                    self.ctx.err(sp, "`when` over `bool` must cover `true` and `false` (RFC 0008 §2)");
                    return Err(());
                }
            }
        }
        Ok(())
    }

    /// Emit the match test for one pattern; returns the bool reg.
    pub(crate) fn compile_pattern_test(&mut self, p: NodeHandle<AnyPat>, scrut_ty: TypeId, scrut_reg: u16, sp: rut_lexer::span::Span) -> TcResult<u16> {
        match self.ctx.ast.pat(p).clone() {
            PatKind::PatElse | PatKind::PatWild => {
                let r = self.new_reg(TY_BOOL);
                self.emit(Op::ConstRaw { dst: r, bits: 1 }, sp.lo);
                Ok(r)
            }
            PatKind::PatLit(lit_node) => {
                // the pattern holds a literal expression node
                let lit = match self.ctx.ast.expr(lit_node) {
                    ExprKind::Lit(l) => l.clone(),
                    _ => {
                        self.ctx.err(sp, "pattern literal expected");
                        return Err(());
                    }
                };
                let (_lty, lreg) = self.load_lit(lit, Some(scrut_ty), sp)?;
                let r = self.new_reg(TY_BOOL);
                match self.ctx.types.kind(scrut_ty).clone() {
                    TyKind::Prim(p) => {
                        self.emit(Op::Cmp { op: CmpOp::Eq, prim: p, dst: r, a: scrut_reg, b: lreg }, sp.lo);
                    }
                    TyKind::Str => {
                        self.emit(Op::StrCmp { eq: true, dst: r, a: scrut_reg, b: lreg }, sp.lo);
                    }
                    _ => {
                        self.ctx.err(sp, "this literal pattern cannot match the scrutinee type");
                    }
                }
                Ok(r)
            }
            PatKind::PatPath { segs } => {
                // enum member (RFC 0008 §2)
                if segs.len() == 2 {
                    let ename = segs[0].name;
                    let mname = segs[1].name;
                    if let Some(e) = self.ctx.find_enum(ename).cloned() {
                        if e.ty == scrut_ty {
                            let midx = e.members.iter().position(|&m| m == mname);
                            match midx {
                                Some(i) => {
                                    let mreg = self.new_reg(scrut_ty);
                                    self.emit(Op::EnumNew { dst: mreg, ty: scrut_ty, member: i as u32 }, sp.lo);
                                    let r = self.new_reg(TY_BOOL);
                                    self.emit(Op::RefEq { eq: true, dst: r, a: scrut_reg, b: mreg }, sp.lo);
                                    return Ok(r);
                                }
                                None => {
                                    self.ctx.err(sp, format!("`{}` is not a member of {}", self.ctx.name(mname), self.ctx.name(ename)));
                                }
                            }
                        } else {
                            self.ctx.err(sp, format!(
                                "`when` pattern `{}` does not match the scrutinee type `{}`",
                                self.ctx.name(ename), self.ctx.types.name(scrut_ty)
                            ));
                        }
                    }
                }
                if segs.len() == 1 {
                    // bare member of the scrutinee's own enum
                    if let TyKind::Enum { members } = self.ctx.types.kind(scrut_ty).clone() {
                        let mname = segs[0].name;
                        if let Some(i) = members.iter().position(|(m, _)| m == self.ctx.name(mname)) {
                            let mreg = self.new_reg(scrut_ty);
                            self.emit(Op::EnumNew { dst: mreg, ty: scrut_ty, member: i as u32 }, sp.lo);
                            let r = self.new_reg(TY_BOOL);
                            self.emit(Op::RefEq { eq: true, dst: r, a: scrut_reg, b: mreg }, sp.lo);
                            return Ok(r);
                        }
                    }
                    self.ctx.err(sp, "unknown pattern");
                }
                self.ctx.err(sp, "patterns in this build: enum members, literals, and `else`");
                Err(())
            }
            PatKind::PatCtor { .. } => {
                self.ctx.err(sp, "constructor patterns (Ok(x), Some(y)) are not supported in this build (RFC 0008 OQ-2)");
                Err(())
            }
        }
    }

}
