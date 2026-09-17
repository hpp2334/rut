//! Statement compilation: blocks with scoped locals, if/while/for-c/for-of,
//! when statements (pattern-test chains), exhaustiveness, and the
//! expression-depth budget.

use crate::check::{float_suffix_ty, int_suffix_ty, TcResult};
use rut_core::ops::*;
use rut_core::sym;
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
            StmtKind::LetStmt { is_mut, name, destructure, ty, init } => {
                let expected = ty.map(|t| self.resolve_type_now(t));
                let t = self.compile_expr(init, expected)?;
                if let Some(e) = expected {
                    if !self.widens(t, e) {
                        self.ctx.err(sp, format!(
                            "let `{}` is `{}` but the initializer is `{}`",
                            self.ctx.name(name), self.ctx.type_name(e), self.ctx.type_name(t)
                        ));
                    }
                }
                let ty = expected.unwrap_or(t);
                match destructure {
                    None => {
                        // copy-by-value binding (RFC 0009/0016 v1.1): a
                        // value-typed initializer that is not a fresh
                        // construction deep-copies into the binding's own
                        // cell — the two never alias
                        let reg = if self.ctx.types.is_value(ty) && !self.last_reg_is_fresh_value() {
                            let src = self.last_reg;
                            let r = self.new_reg(ty);
                            self.emit(Op::CloneVal { dst: r, src, ty }, sp.lo);
                            r
                        } else {
                            self.last_reg
                        };
                        self.locals.push(Local { name, reg, ty, is_mut, loop_var: false });
                    }
                    Some(names) => {
                        // `let (a, b) = ..` (RFC 0007): each binding takes
                        // the matching tuple field
                        let TyKind::Data { fields } = self.ctx.types.kind(ty).clone() else {
                            self.ctx.err(sp, format!("destructuring needs a tuple —got `{}`", self.ctx.type_name(ty)));
                            return Err(());
                        };
                        if fields.len() != names.len() {
                            self.ctx.err(sp, format!(
                                "the pattern binds {} names but the tuple has {} fields",
                                names.len(), fields.len()
                            ));
                            return Err(());
                        }
                        let tuple_reg = self.last_reg;
                        for (i, n) in names.iter().enumerate() {
                            let fty = fields[i].ty;
                            let mut reg = self.new_reg(fty);
                            self.emit(
                                Op::GetF {
                                    dst: reg,
                                    obj: tuple_reg,
                                    field: i as u32,
                                    repr: self.ctx.types.repr_of(fty),
                                },
                                sp.lo,
                            );
                            if self.ctx.types.is_value(fty) {
                                let c = self.new_reg(fty);
                                self.emit(Op::CloneVal { dst: c, src: reg, ty: fty }, sp.lo);
                                reg = c;
                            }
                            if self.ctx.types.is_value(fty) {
                                let c = self.new_reg(fty);
                                self.emit(Op::CloneVal { dst: c, src: reg, ty: fty }, sp.lo);
                                reg = c;
                            }
                            self.locals.push(Local {
                                name: *n,
                                reg,
                                ty: fty,
                                is_mut,
                                loop_var: false,
                            });
                        }
                    }
                }
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
                // inlined method body: `return` targets the inliner's result
                if let Some((dst, l_end)) = self.inline_ret {
                    if let Some(v) = value {
                        let t = self.compile_expr(v, Some(self.ret_ty))?;
                        if t != self.ret_ty {
                            self.ctx.err(sp, format!(
                                "return type mismatch: `{}` expected, `{}` returned",
                                self.ctx.type_name(self.ret_ty), self.ctx.type_name(t)
                            ));
                        }
                        let src = self.last_reg;
                        if self.ctx.types.is_ref(self.ret_ty) {
                            self.emit(Op::MovRef { dst, src }, sp.lo);
                        } else {
                            self.emit(Op::Mov { dst, src }, sp.lo);
                        }
                    }
                    self.jmp(l_end);
                    return Ok(());
                }
                match value {
                    Some(v) => {
                        let t = self.compile_expr(v, Some(self.ret_ty))?;
                        if t != self.ret_ty {
                            self.ctx.err(sp, format!(
                                "return type mismatch: `{}` expected, `{}` returned",
                                self.ctx.type_name(self.ret_ty), self.ctx.type_name(t)
                            ));
                        }
                        self.emit(Op::Ret { val: Some(self.last_reg) }, sp.lo);
                    }
                    None => {
                        if self.ret_ty != TY_NIL {
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
                // inside a desugared `for..of` emit closure (RFC 0012 §6),
                // stopping the iteration IS returning `false`
                if self.emit_closure {
                    let f = self.new_reg(TY_BOOL);
                    self.emit(Op::ConstRaw { dst: f, bits: 0 }, sp.lo);
                    self.emit(Op::Ret { val: Some(f) }, sp.lo);
                    return Ok(());
                }
                let Some((_, brk)) = self.loops.last().copied() else {
                    self.ctx.err(sp, "`break` outside a loop");
                    return Ok(());
                };
                self.jmp(brk);
                Ok(())
            }
            StmtKind::Continue => {
                // skipping to the next element IS returning `true`
                if self.emit_closure {
                    let t = self.new_reg(TY_BOOL);
                    self.emit(Op::ConstRaw { dst: t, bits: 1 }, sp.lo);
                    self.emit(Op::Ret { val: Some(t) }, sp.lo);
                    return Ok(());
                }
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
        // `for (v of p)` auto-derefs a pointer (RFC 0005)
        let (it, iter_reg) = self.deref_for_use(it, iter_reg, sp.lo);
        // v1.1: str iteration yields str elements (not char)
        let it = if it == rut_core::types::TY_CHAR { rut_core::types::TY_STR } else { it };
        // the builtin sequences (Vec, Array, str, bytes) keep their fused
        // loops; a user type iterates through the `__iterate` protocol
        // (RFC 0012 §6)
        let info = match self.slice_info(it) {
            Some(info) => info,
            None => {
                if let Some((dname, env, elem_ty)) = self.iterate_method(it) {
                    return self.compile_for_of_iterate(var, iter_reg, dname, env, elem_ty, body, sp);
                }
                self.ctx.err(sp, format!(
                    "`for (let .. of ..)` needs a sequence — `{}` is not one and declares no `__iterate` (RFC 0012 §6)",
                    self.ctx.type_name(it)
                ));
                return Err(());
            }
        };
        let elem_ty = info.elem;
        let idx = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: idx, bits: 0 }, sp.lo);
        // read a fixed source's length once, before the back-edge (`str`/
        // `bytes` are immutable, `Array` is fixed-size); an `impl Iter`
        // accessor stays inside the loop
        let hoisted_len = if info.fixed_len() {
            Some(self.emit_slice_len(iter_reg, &info, sp.lo)?)
        } else {
            None
        };
        let l_head = self.new_label();
        let l_body = self.new_label();
        let l_end = self.new_label();
        let l_cont = self.new_label();
        self.bind(l_head);
        self.emit(Op::LoopHead, sp.lo);
        let len_reg = match hoisted_len {
            Some(r) => r,
            None => self.emit_slice_len(iter_reg, &info, sp.lo)?,
        };
        let cond_reg = self.new_reg(TY_BOOL);
        self.emit(cmpop(CmpOp::Lt, PrimTy::I32, cond_reg, idx, len_reg), sp.lo);
        self.br(cond_reg, l_body, l_end);
        self.bind(l_body);
        // var = iter[idx]; the dst register is the loop variable (one reg
        // reused every iteration, overwritten/released by the element op).
        // Vec/`[T]` yield `*T` — a fresh element box per iteration
        // (RFC 0012 §6); `str`/`bytes` keep value yields.
        let (var_ty, var_reg) = match info.source {
            super::slice::SliceSource::Str | super::slice::SliceSource::Bytes => {
                let r = self.emit_slice_get(iter_reg, idx, &info, sp.lo)?;
                (elem_ty, r)
            }
            _ => {
                let r = self.emit_slice_get_ref(iter_reg, idx, &info, sp.lo)?;
                (self.ctx.mk_ptr(elem_ty), r)
            }
        };
        self.locals.push(Local { name: var, reg: var_reg, ty: var_ty, is_mut: false, loop_var: false });
        self.loops.push((l_cont, l_end));
        self.compile_block(body)?;
        self.loops.pop();
        self.locals.pop();
        self.bind(l_cont);
        let one = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: one, bits: 1 }, sp.lo);
        let next = self.new_reg(TY_I32);
        self.emit(arith(ArithOp::Add, PrimTy::I32, next, idx, one), sp.lo);
        self.emit(Op::Mov { dst: idx, src: next }, sp.lo);
        self.jmp(l_head);
        self.bind(l_end);
        let _ = node;
        Ok(())
    }


    /// The `__iterate` contract on `ty` (RFC 0012 §6): `(class, subst, E)`
    /// when the type declares `fn __iterate(self, emit: fn(E) -> bool)` —
    /// duck-typed, like every interface satisfaction.
    fn iterate_method(&mut self, ty: TypeId) -> Option<(IdentId, Vec<(IdentId, TypeId)>, TypeId)> {
        if matches!(self.ctx.types.kind(ty), TyKind::TraitObj { .. }) {
            // interface objects dispatch through their vtable — a concrete
            // iterable is needed at the call site in this build
            return None;
        }
        let (dname, args): (IdentId, Vec<TypeId>) = match self.ctx.inst_data.get(&ty) {
            Some((d, a)) => (*d, a.clone()),
            None => match self.ctx.datas.iter().find(|(_, d)| d.ty == ty) {
                Some((n, d)) if d.generics.is_empty() => (*n, vec![]),
                _ => return None,
            },
        };
        let Some((_, d)) = self.ctx.datas.iter().find(|(n, _)| n == &dname) else {
            return None;
        };
        let d = d.clone();
        let (_, mnode) = d.methods.iter().find(|(n, _)| *n == sym::ITERATE)?;
        let env: Vec<(IdentId, TypeId)> =
            d.generics.iter().cloned().zip(args.iter().cloned()).collect();
        let md = self.ctx.ast.method_decl(*mnode).clone();
        // the signature after `self`: exactly one `emit: fn(E) -> bool`
        let mut ptys = Vec::new();
        for p in md.params.iter().skip(1) {
            if let MemberKind::Param(ParamData { ty: Some(t), .. }) = self.ctx.ast.param(*p) {
                ptys.push(self.ctx.resolve_type(*t, &env));
            }
        }
        match ptys.first().map(|t| self.ctx.types.kind(*t).clone()) {
            Some(TyKind::Fn { params, ret }) if params.len() == 1 && ret == TY_BOOL => {
                Some((dname, env, params[0]))
            }
            _ => None,
        }
    }

    /// `for (v of xs)` over a user iterable (RFC 0012 §6) — desugars to
    /// `xs.__iterate(emit)` where `emit` is a synthetic closure carrying
    /// the loop body: `break` returns `false`, `continue` and the fall-through
    /// return `true`. The loop variable is the closure's parameter, so it is
    /// a fresh binding per iteration by construction.
    fn compile_for_of_iterate(
        &mut self,
        var: IdentId,
        rreg: u16,
        dname: IdentId,
        env: Vec<(IdentId, TypeId)>,
        elem_ty: TypeId,
        body: NodeHandle<BlockNode>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<()> {
        // captures: the body's enclosing locals, copied by value (the
        // closure law) — accumulate through a `*T` capture
        let mut referenced = Vec::new();
        self.scan_names(body.id(), &mut referenced);
        let mut caps: Vec<(IdentId, TypeId, u16)> = Vec::new();
        for n in referenced {
            if n == var {
                continue;
            }
            if let Some(l) = self.lookup(n).copied() {
                let cap_reg = self.new_reg(l.ty);
                if self.ctx.types.is_ref(l.ty) {
                    self.emit(Op::MovRef { dst: cap_reg, src: l.reg }, sp.lo);
                } else {
                    self.emit(Op::Mov { dst: cap_reg, src: l.reg }, sp.lo);
                }
                caps.push((n, l.ty, cap_reg));
            }
        }
        self.ctx
            .for_of_sigs
            .insert(body.id().0, (elem_ty, caps.iter().map(|(n, t, _)| (*n, *t)).collect::<Vec<_>>()));
        let emit = crate::check::Inst {
            key: crate::check::FnKey::ForOfEmit { body: body.id(), var },
            subst: vec![],
        };
        let fid = self.ctx.ensure_inst(emit);
        let fty = self.ctx.mk_fn_ty(vec![elem_ty], TY_BOOL);
        let clo = self.new_reg(fty);
        { let (argv_off, argc) = self.pool_args(&(caps.iter().map(|(_, _, r)| *r).collect::<Vec<_>>())); self.emit(Op::MakeClosure { dst: clo, func: fid, argv_off, argc }, sp.lo,); }
        // `xs.__iterate(emit)` — the ordinary method machinery
        let iterate = self.ctx.lookup_name("__iterate").expect("`__iterate` interned");
        let mfid = self
            .ctx
            .ensure_inst(crate::check::Inst {
                key: crate::check::FnKey::Method { data: dname, name: iterate },
                subst: env,
            });
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(vec![clo])); self.emit(Op::CallM { func: mfid, argv_off, argc, dst: NOREG }, sp.lo); }
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
        // arm type unification (statement-when: nil)
        let result_ty = if let Some(e) = expected {
            e
        } else {
            // first expression-arm's type
            let mut t = TY_NIL;
            for a in arms {
                if let ArmKind::WhenArm { body, .. } = self.ctx.ast.arm(*a) {
                    t = self.expr_type_hint(*body);
                    break;
                }
            }
            t
        };
        let result_reg = if result_ty != TY_NIL {
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
            // nil arms may be statement blocks (`-> { a(); b(); }`,
            // RFC 0008 §1) — blocks aren't value expressions in this
            // build, so compile them as scoped statement blocks
            let body_is_block = matches!(self.ctx.ast.expr(*body), ExprKind::Block { .. });
            let bt = if body_is_block && result_ty == TY_NIL {
                self.compile_block(*body)?;
                TY_NIL
            } else {
                self.compile_expr(*body, if result_ty != TY_NIL { Some(result_ty) } else { None })?
            };
            if result_ty != TY_NIL {
                if bt != result_ty {
                    self.ctx.err(self.ctx.ast.span(body.id()), format!(
                        "`when` arms must agree: `{}` vs `{}`",
                        self.ctx.type_name(result_ty), self.ctx.type_name(bt)
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
            Ok(TY_NIL)
        }
    }

    pub(crate) fn expr_type_hint(&mut self, node: NodeHandle<AnyExpr>) -> TypeId {
        // best-effort type for when-arm unification without full inference:
        // literals only; otherwise first arm decides later via compile
        match self.ctx.ast.expr(node) {
            ExprKind::Lit(Lit::Int(_, s)) => s.map(int_suffix_ty).unwrap_or(TY_I32),
            ExprKind::Lit(Lit::Float(_, s)) => s.map(float_suffix_ty).unwrap_or(TY_F32),
            ExprKind::Lit(Lit::Str(_) | Lit::RawStr(_)) => TY_STR,
            ExprKind::Lit(Lit::Bool(_)) => TY_BOOL,
            ExprKind::Block { .. } => TY_NIL,
            ExprKind::Struct { ty, .. } => self.resolve_type_now(*ty),
            _ => TY_NIL,
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
                    let mtext = self.ctx.name(*m);
                    if !seen.iter().any(|s| s.ends_with(&format!(".{mtext}")) || s == mtext) {
                        self.ctx.err(
                            sp,
                            format!(
                                "`when` over an enum must be exhaustive —`{mtext}` is not covered and there is no `else` arm (RFC 0008 §2)"
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
                        self.ctx.type_name(scrut_ty)
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
                        self.emit(cmpop(CmpOp::Eq, p, r, scrut_reg, lreg), sp.lo);
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
                                self.ctx.name(ename), self.ctx.type_name(scrut_ty)
                            ));
                        }
                    }
                }
                if segs.len() == 1 {
                    // bare member of the scrutinee's own enum
                    if let TyKind::Enum { members } = self.ctx.types.kind(scrut_ty).clone() {
                        let mname = segs[0].name;
                        if let Some(i) = members.iter().position(|(m, _)| *m == mname) {
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
