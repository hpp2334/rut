//! Calls and member resolution: builtin/free-fn/method/static dispatch,
//! generic instantiation by unification (RFC 0013 SS2), the RFC 0012
//! vtable-always rule for trait members, dyn receivers, and field reads.

use crate::check::TcResult;
use rut_core::ops::*;
use rut_core::types::*;
use super::*;

// ============ part 3: calls & members ============

impl<'a, 'b> FnCompiler<'a, 'b> {
    pub(crate) fn compile_call(
        &mut self,
        node: NodeHandle<AnyExpr>,
        callee: NodeHandle<AnyExpr>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let _ = node;
        // fn-typed local call: `f(x)` where f is a param/local of fn type
        if let ExprKind::Path { segs } = self.ctx.ast.expr(callee).clone() {
            if segs.len() == 1 && self.lookup(segs[0].name).is_some() {
                let ft = self.compile_expr(callee, None)?;
                match self.ctx.types.kind(ft).clone() {
                    TyKind::Fn { .. } => {
                        let freg = self.last_reg;
                        return self.finish_call_fn(freg, args, expected, sp);
                    }
                    _ => {
                        self.ctx.err(sp, format!("`{}` is not callable", self.ctx.types.name(ft)));
                        return Err(());
                    }
                }
            }
        }
        if let ExprKind::Path { segs } = self.ctx.ast.expr(callee).clone() {
            return self.compile_path_call(segs, args, expected, sp);
        }
        // fn-typed value call: `f(x)` where f: fn(T) -> U
        let ft = self.compile_expr(callee, None)?;
        match self.ctx.types.kind(ft).clone() {
            TyKind::Fn { .. } => {
                let freg = self.last_reg;
                self.finish_call_fn(freg, args, expected, sp)
            }
            _ => {
                self.ctx.err(sp, format!("`{}` is not callable", self.ctx.types.name(ft)));
                Err(())
            }
        }
    }

    pub(crate) fn finish_call_fn(
        &mut self,
        freg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let (ptys, ret) = match self.ctx.types.kind(self.regs[freg as usize]).clone() {
            TyKind::Fn { params, ret } => (params, ret),
            _ => {
                self.ctx.err(sp, "not a function value");
                return Err(());
            }
        };
        // trailing captures are not callable args
        let declared = ptys.len();
        if args.len() != declared {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), declared));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.types.name(t), self.ctx.types.name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let dst = if ret == TY_UNIT { None } else { Some(self.new_reg(ret)) };
        self.emit(Op::CallFn { fval: freg, args: aregs, dst }, sp.lo);
        Ok(ret)
    }

    pub(crate) fn compile_path_call(
        &mut self,
        segs: Vec<PathSeg>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        if segs.len() == 2 {
            let base = segs[0].name;
            let member = segs[1].name;
            let base_generics = segs[0].generics.clone();
            let member_generics = segs[1].generics.clone();
            return self.compile_static_call(base, base_generics, member, member_generics, args, expected, sp);
        }
        if segs.len() != 1 {
            self.ctx.err(sp, "unsupported call path (imports are not available in this build, RFC 0035)");
            return Err(());
        }
        let name = segs[0].name;
        let n = self.ctx.name(name).to_string();
        let generics = segs[0].generics.clone();
        match n.as_str() {
            "own" => {
                if args.len() != 1 {
                    self.ctx.err(sp, "own(x) takes one argument (RFC 0011 §1)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                let src = self.last_reg;
                let dst = self.new_reg(t);
                self.emit(Op::Own { dst, src, ty: t }, sp.lo);
                return Ok(t);
            }
            "downcast" => {
                // prelude body: tidof + icmp + br + guarded unbox (RFC 0032 §1.1)
                if args.len() != 1 || generics.len() != 1 {
                    self.ctx.err(sp, "downcast<T>(o) takes one explicit type argument and one value (RFC 0014)");
                    return Err(());
                }
                let want = self.resolve_type_now(generics[0]);
                if matches!(self.ctx.types.kind(want), TyKind::TraitObj { .. }) {
                    self.ctx.err(sp, "downcast needs a CONCRETE type —trait objects have no recovery path (RFC 0014)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_OPAQUE))?;
                if t != TY_OPAQUE {
                    self.ctx.err(sp, "downcast takes an `Opaque` box (RFC 0014)");
                    return Err(());
                }
                let oty = self.ctx.mk_option(want);
                let orecv = self.last_reg;
                let tid_reg = self.new_reg(TY_U32);
                self.emit(Op::TidOf { dst: tid_reg, obj: orecv }, sp.lo);
                let want_reg = self.new_reg(TY_U32);
                self.emit(Op::ConstRaw { dst: want_reg, bits: want as u64 }, sp.lo);
                let eq = self.new_reg(TY_BOOL);
                self.emit(Op::Cmp { op: CmpOp::Eq, ty: TY_U32, dst: eq, a: tid_reg, b: want_reg }, sp.lo);
                let dst = self.new_reg(oty);
                let l_some = self.new_label();
                let l_none = self.new_label();
                let l_end = self.new_label();
                self.br(eq, l_some, l_none);
                self.bind(l_some);
                let un = self.new_reg(want);
                self.emit(Op::Unbox { dst: un, box_: orecv, ty: want }, sp.lo);
                self.emit(Op::OptSome { dst, ty: oty, val: un }, sp.lo);
                self.jmp(l_end);
                self.bind(l_none);
                self.emit(Op::OptNone { dst, ty: oty }, sp.lo);
                self.bind(l_end);
                // the value lives in `dst`; move it out so last_reg holds it
                let out = self.new_reg(oty);
                self.emit(Op::MovRef { dst: out, src: dst }, sp.lo);
                return Ok(oty);
            }
            "panic" => {
                if args.len() != 1 {
                    self.ctx.err(sp, "panic(msg) takes a message (RFC 0034 §2)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_STR))?;
                if t != TY_STR {
                    self.ctx.err(sp, "panic takes a `string`");
                }
                self.emit(Op::Panic { msg: self.last_reg }, sp.lo);
                return Ok(TY_UNIT);
            }
            "assert" => {
                if args.is_empty() || args.len() > 2 {
                    self.ctx.err(sp, "assert(cond, msg?) takes a condition (RFC 0034 §2)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_BOOL))?;
                if t != TY_BOOL {
                    self.ctx.err(sp, "assert takes a `bool`");
                }
                let cond = self.last_reg;
                let msg = if args.len() == 2 {
                    let t = self.compile_expr(args[1], Some(TY_STR))?;
                    if t != TY_STR {
                        self.ctx.err(sp, "assert message must be a `string`");
                    }
                    Some(self.last_reg)
                } else {
                    None
                };
                self.emit(Op::Assert { cond, msg }, sp.lo);
                return Ok(TY_UNIT);
            }
            "print" => {
                if args.len() != 1 {
                    self.ctx.err(sp, "print(s) takes one string");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_STR))?;
                if t != TY_STR {
                    self.ctx.err(sp, format!("print takes a `string`, found `{}`", self.ctx.types.name(t)));
                }
                self.emit(Op::CallNat { nat: Nat::Print, recv: None, args: vec![self.last_reg], dst: None }, sp.lo);
                return Ok(TY_UNIT);
            }
            "type_id" | "size_of" | "align_of" => {
                // compile-time constants —never executed (RFC 0015 §3, 0033 §3)
                if generics.len() != 1 || !args.is_empty() {
                    self.ctx.err(sp, format!("{n}<T>() takes one explicit type argument"));
                    return Err(());
                }
                let t = self.resolve_type_now(generics[0]);
                let v: u64 = match n.as_str() {
                    "type_id" => t as u64,
                    "size_of" => self.ctx.layout_of(t).0 as u64,
                    _ => self.ctx.layout_of(t).1 as u64,
                };
                let dst = self.new_reg(TY_U32);
                self.emit(Op::ConstRaw { dst, bits: v }, sp.lo);
                return Ok(TY_U32);
            }
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" => {
                if args.len() != 1 {
                    self.ctx.err(sp, format!("{n}(x) takes one argument"));
                    return Err(());
                }
                let to = match n.as_str() {
                    "i8" => TY_I8, "i16" => TY_I16, "i32" => TY_I32, "i64" => TY_I64,
                    "u8" => TY_U8, "u16" => TY_U16, "u32" => TY_U32, "u64" => TY_U64,
                    "f32" => TY_F32, _ => TY_F64,
                };
                let from = self.compile_expr(args[0], None)?;
                if !matches!(self.ctx.types.kind(from), TyKind::Prim(_)) {
                    self.ctx.err(sp, format!("{}(..) converts numbers —found `{}`", n, self.ctx.types.name(from)));
                    return Err(());
                }
                let src = self.last_reg;
                let dst = self.new_reg(to);
                self.emit(Op::Conv { dst, src, from, to }, sp.lo);
                return Ok(to);
            }
            "str" => {
                if args.len() != 1 {
                    self.ctx.err(sp, "str(x) takes one argument");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                self.check_formattable(t, self.ctx.ast.span(args[0].id()))?;
                let src = self.last_reg;
                let dst = self.new_reg(TY_STR);
                self.emit(Op::CallNat { nat: Nat::Str, recv: None, args: vec![src], dst: Some(dst) }, sp.lo);
                return Ok(TY_STR);
            }
            _ => {}
        }
        // user free fn (monomorphized instantiation, RFC 0013 §2)
        if self.ctx.find_free_fn(name) {
            return self.compile_free_fn_call(name, generics, args, expected, sp);
        }
        // builtin type-call: Vec<T>(n) — or bare `Vec()` with the element
        // inferred from the expected type (`let primes: Vec<i32> = Vec()`)
        if n == "Vec" {
            if !generics.is_empty() {
                return self.compile_vec_alloc(generics, args, sp);
            }
            let elem = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                Some(TyKind::Vec { elem }) => elem,
                _ => {
                    self.ctx.err(sp, "cannot infer the element type — write `Vec<T>()` or annotate the binding");
                    return Err(());
                }
            };
            let vty = self.ctx.mk_vec(elem);
            let zero = self.new_reg(TY_I32);
            self.emit(Op::ConstRaw { dst: zero, bits: 0 }, sp.lo);
            let dst = self.new_reg(vty);
            self.emit(Op::ArrNew { dst, ty: vty, len: zero }, sp.lo);
            return Ok(vty);
        }
        if self.ctx.find_data(name).is_some() {
            self.ctx.err(sp, format!(
                "construction is a method call, never a type-call —use a class method ({}.new(..)) or a dataclass literal `{} {{ .. }}` (RFC 0010 §1)",
                n, n
            ));
            return Err(());
        }
        self.ctx.err(sp, format!("unknown function `{n}`"));
        Err(())
    }

    pub(crate) fn compile_vec_alloc(&mut self, generics: Vec<NodeHandle<AnyTy>>, args: Vec<NodeHandle<AnyExpr>>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        if generics.len() != 1 {
            self.ctx.err(sp, "Vec<T>(..) takes one type argument");
            return Err(());
        }
        let elem = self.resolve_type_now(generics[0]);
        let vty = self.ctx.mk_vec(elem);
        match args.len() {
            0 => {
                let zero = self.new_reg(TY_I32);
                self.emit(Op::ConstRaw { dst: zero, bits: 0 }, sp.lo);
                let dst = self.new_reg(vty);
                self.emit(Op::ArrNew { dst, ty: vty, len: zero }, sp.lo);
                Ok(vty)
            }
            1 => {
                if !matches!(self.ctx.types.kind(elem), TyKind::Prim(_)) {
                    self.ctx.err(sp, "zeroed allocation needs a primitive element type (RFC 0005)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_I32))?;
                if t != TY_I32 {
                    self.ctx.err(sp, format!("Vec<T>(n) takes an `i32` length, found `{}`", self.ctx.types.name(t)));
                }
                let len = self.last_reg;
                let dst = self.new_reg(vty);
                self.emit(Op::ArrNew { dst, ty: vty, len }, sp.lo);
                Ok(vty)
            }
            _ => {
                self.ctx.err(sp, "Vec<T>() or Vec<T>(n)");
                Err(())
            }
        }
    }

    pub(crate) fn compile_static_call(
        &mut self,
        base: IdentId,
        base_generics: Vec<NodeHandle<AnyTy>>,
        member: IdentId,
        _member_generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let bn = self.ctx.name(base).to_string();
        let mn = self.ctx.name(member).to_string();
        if !base_generics.is_empty() {
            self.ctx.err(sp, format!("generic type paths (`{bn}<..>.{mn}`) are not supported in this build"));
            return Err(());
        }
        match (bn.as_str(), mn.as_str()) {
            ("Option", "some") => {
                if args.len() != 1 {
                    self.ctx.err(sp, "Option.some(v) takes one value");
                    return Err(());
                }
                let elem_hint = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                    Some(TyKind::Option { elem }) => Some(elem),
                    _ => None,
                };
                let t = self.compile_expr(args[0], elem_hint)?;
                let src = self.last_reg;
                let oty = self.ctx.mk_option(t);
                let dst = self.new_reg(oty);
                self.emit(Op::OptSome { dst, ty: oty, val: src }, sp.lo);
                return Ok(oty);
            }
            ("Option", "none") => {
                let elem = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                    Some(TyKind::Option { elem }) => elem,
                    _ => {
                        self.ctx.err(sp, "cannot infer the element type —annotate the binding `let x: Option<T> = Option.none()`");
                        return Err(());
                    }
                };
                let oty = self.ctx.mk_option(elem);
                let dst = self.new_reg(oty);
                self.emit(Op::OptNone { dst, ty: oty }, sp.lo);
                return Ok(oty);
            }
            ("Result", "ok") | ("Result", "err") => {
                let is_ok = mn == "ok";
                if args.len() != 1 {
                    self.ctx.err(sp, format!("Result.{mn}(v) takes one value"));
                    return Err(());
                }
                let (ok_hint, err_hint) = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                    Some(TyKind::Result { ok, err }) => (Some(ok), Some(err)),
                    _ => (None, None),
                };
                let hint = if is_ok { ok_hint } else { err_hint };
                let t = self.compile_expr(args[0], hint)?;
                let src = self.last_reg;
                // the OTHER side's type comes from the expected hint
                // (bidirectional, RFC 0007 §1) — without one it mirrors `t`
                let other = match if is_ok { err_hint } else { ok_hint } {
                    Some(o) => o,
                    None => t,
                };
                let rty = self.ctx.mk_result(if is_ok { t } else { other }, if is_ok { other } else { t });
                let dst = self.new_reg(rty);
                if is_ok {
                    self.emit(Op::ResOk { dst, ty: rty, val: src }, sp.lo);
                } else {
                    self.emit(Op::ResErr { dst, ty: rty, val: src }, sp.lo);
                }
                return Ok(rty);
            }
            ("Opaque", "new") => {
                if args.len() != 1 {
                    self.ctx.err(sp, "Opaque.new(v) takes one value");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                if matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. }) {
                    self.ctx.err(sp, "`Opaque.new` rejects `dyn` values —trait objects are never boxed (RFC 0014)");
                    return Err(());
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_OPAQUE);
                self.emit(Op::Box { dst, val: src, ty: t }, sp.lo);
                return Ok(TY_OPAQUE);
            }
            ("Vec", "from") => {
                if args.len() != 1 {
                    self.ctx.err(sp, "Vec.from(arr) takes one array");
                    return Err(());
                }
                let elem_hint = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                    Some(TyKind::Vec { elem }) => Some(elem),
                    _ => None,
                };
                let hint_ty = match elem_hint {
                    Some(e) => Some(self.ctx.mk_array(e, 0)),
                    None => None,
                };
                let at = self.compile_expr(args[0], hint_ty)?;
                let elem = match self.ctx.types.kind(at).clone() {
                    TyKind::Array { elem, .. } | TyKind::Vec { elem } => elem,
                    _ => {
                        self.ctx.err(sp, format!("Vec.from expects an Array —found `{}`", self.ctx.types.name(at)));
                        return Err(());
                    }
                };
                // widen through the expected element (RFC 0012 Section 2)
                let elem = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                    Some(TyKind::Vec { elem: want }) if self.widens(elem, want) => want,
                    _ => elem,
                };
                let vty = self.ctx.mk_vec(elem);
                let src = self.last_reg;
                let dst = self.new_reg(vty);
                self.emit(Op::CallNat { nat: Nat::VecFrom, recv: None, args: vec![src], dst: Some(dst) }, sp.lo);
                return Ok(vty);
            }
            _ => {}
        }
        // enum helpers: Color.to_int(c) (RFC 0006)
        if mn == "to_int" {
            if let Some(e) = self.ctx.find_enum(base).cloned() {
                let _ = e;
                self.ctx.err(sp, "enum to_int/from_int are not supported in this build (RFC 0006)");
                return Err(());
            }
        }
        // class method call: `Circle.new(..)` (RFC 0010 §1) — resolve the
        // class BY NAME first, then its member: searching for the first
        // class with a same-named method would shadow every later class
        // (two `new`s in one module made the second uncallable)
        if let Some((dname, d)) = self.ctx.datas.iter().find(|(n, _)| *n == base).map(|(n, d)| (*n, d.clone())) {
            if let Some((_, mnode)) = d.methods.iter().find(|(m, _)| *m == member).cloned() {
                return self.compile_direct_method(dname, d, mnode, args, sp);
            }
        }        if let Some(e) = self.ctx.find_enum(base) {
            let _ = e;
            self.ctx.err(sp, format!("enum `{bn}` has no static `{mn}` in this build"));
            return Err(());
        }
        self.ctx.err(sp, format!("unknown name `{bn}.{mn}`"));
        Err(())
    }

    pub(crate) fn compile_free_fn_call(
        &mut self,
        name: IdentId,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let Some(fnode) = self.ctx.fn_nodes.iter().find(|(n, _)| *n == name).map(|(_, n)| *n) else {
            self.ctx.err(sp, format!("unknown function `{}`", self.ctx.name(name)));
            return Err(());
        };
        let fd = self.ctx.ast.fn_decl(fnode).clone();
        let (decl_generics, params, ret) = (fd.generics, fd.params, fd.ret);
        if generics.len() > decl_generics.len() {
            self.ctx.err(sp, format!(
                "`{}` takes {} generic arguments, {} given",
                self.ctx.name(name), decl_generics.len(), generics.len()
            ));
            return Err(());
        }
        // substitution: explicit args first, then inference from arguments
        let mut subst: Vec<(IdentId, TypeId)> = Vec::new();
        for (g, node) in decl_generics.iter().zip(generics.iter()) {
            let t = self.resolve_type_now(*node);
            subst.push((*g, t));
        }
        // compile args under best-effort expected types; unify generics
        let param_nodes: Vec<Option<NodeHandle<AnyTy>>> = params
            .iter()
            .map(|p| match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => Some(*t),
                _ => None,
            })
            .collect();
        if args.len() != params.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), params.len()));
            return Err(());
        }
        let mut arg_tys: Vec<TypeId> = Vec::new();
        let mut aregs: Vec<u16> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let expected = match param_nodes[i] {
                Some(tn) if self.free_generics(tn, &decl_generics, &subst).is_empty() => {
                    Some(self.ctx.resolve_type(tn, &subst))
                }
                _ => None,
            };
            let t = self.compile_expr(*a, expected)?;
            arg_tys.push(t);
            aregs.push(self.last_reg);
        }
        for (i, _) in args.iter().enumerate() {
            if let Some(tn) = param_nodes[i] {
                self.unify_generic(tn, arg_tys[i], &decl_generics, &mut subst, sp)?;
            }
        }
        for g in &decl_generics {
            if !subst.iter().any(|(n, _)| n == g) {
                self.ctx.err(sp, format!(
                    "cannot infer generic parameter `{}` — annotate the call: `{}<..>(..)`",
                    self.ctx.name(*g), self.ctx.name(name)
                ));
                return Err(());
            }
        }
        // final param types under the completed substitution
        let ptys: Vec<TypeId> = params
            .iter()
            .map(|p| match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => self.ctx.resolve_type(*t, &subst),
                _ => TY_I32,
            })
            .collect();
        for (i, a) in args.iter().enumerate() {
            if !self.widens(arg_tys[i], ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.types.name(arg_tys[i]), self.ctx.types.name(ptys[i])
                ));
            }
        }
        let ret_ty = ret.map(|r| self.ctx.resolve_type(r, &subst)).unwrap_or(TY_UNIT);
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Free(name),
            subst,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_UNIT { None } else { Some(self.new_reg(ret_ty)) };
        self.emit(Op::Call { func: fid, args: aregs, dst }, sp.lo);
        Ok(ret_ty)
    }


    /// class-method call through the type name (`Circle.new(..)`)
    pub(crate) fn compile_direct_method(
        &mut self,
        dname: IdentId,
        d: crate::check::DataDecl,
        mnode: NodeHandle<MethodDeclNode>,
        args: Vec<NodeHandle<AnyExpr>>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let md = self.ctx.ast.method_decl(mnode).clone();
        let (params, ret, mname) = (md.params, md.ret, md.name);
        // no-self first param (or no params at all) = class method
        if matches!(
            params.first().map(|p| self.ctx.ast.param(*p)),
            Some(MemberKind::SelfParam(_))
        ) {
            self.ctx.err(sp, "instance methods are called on a value, not the class");
            return Err(());
        }
        let mut ptys = Vec::new();
        // the callee's signature may spell `Self` — resolve its types under
        // the CALLEE's class (the caller's self_ty is irrelevant here)
        let saved_self = self.self_ty;
        self.self_ty = Some(d.ty);
        for p in &params {
            match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => ptys.push(self.resolve_type_now(*t)),
                _ => ptys.push(TY_I32),
            }
        }
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_UNIT);
        self.self_ty = saved_self;
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.types.name(t), self.ctx.types.name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Method { data: dname, name: mname },
            subst: vec![],
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_UNIT { None } else { Some(self.new_reg(ret_ty)) };
        // class method: no receiver —plain Call
        self.emit(Op::Call { func: fid, args: aregs, dst }, sp.lo);
        Ok(ret_ty)
    }

    pub(crate) fn compile_method(
        &mut self,
        recv: NodeHandle<AnyExpr>,
        name: IdentId,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        // static receiver? `Vec.from(..)`, `Option.some(v)`, `Color.to_int(c)`,
        // `Circle.new(..)` arrive as Method over a TYPE-name path — route to
        // the static-call compiler when the head is not shadowed by a local
        if let ExprKind::Path { segs } = self.ctx.ast.expr(recv).clone() {
            if segs.len() == 1 && self.lookup(segs[0].name).is_none() && segs[0].generics.is_empty() {
                let base = segs[0].name;
                let bn = self.ctx.name(base).to_string();
                let is_type = matches!(bn.as_str(), "Vec" | "Option" | "Result" | "Opaque")
                    || self.ctx.find_enum(base).is_some()
                    || self.ctx.find_data(base).is_some();
                if is_type {
                    return self.compile_static_call(base, Vec::new(), name, generics, args, expected, sp);
                }
            }
        }
        let rt = self.compile_expr(recv, None)?;
        let rreg = self.last_reg;
        let mname = self.ctx.name(name).to_string();
        if !generics.is_empty() {
            self.ctx.err(sp, "generic method calls are not supported in this build");
            return Err(());
        }
        // builtin members (RFC 0005 table; Vec's named API —RFC 0032 §1.1 R2)
        match self.ctx.types.kind(rt).clone() {
            TyKind::Option { elem } => match mname.as_str() {
                "is_some" | "is_none" => {
                    if !args.is_empty() {
                        self.ctx.err(sp, format!("{mname}() takes no arguments"));
                        return Err(());
                    }
                    let dst = self.new_reg(TY_BOOL);
                    self.emit(Op::SumIs { dst, v: rreg, want_err: mname == "is_none" }, sp.lo);
                    return Ok(TY_BOOL);
                }
                "unwrap_or" => {
                    if args.len() != 1 {
                        self.ctx.err(sp, "unwrap_or(default) takes one argument");
                        return Err(());
                    }
                    let t = self.compile_expr(args[0], Some(elem))?;
                    if t != elem {
                        self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                            "unwrap_or takes `{}`, found `{}`",
                            self.ctx.types.name(elem), self.ctx.types.name(t)
                        ));
                    }
                    let dflt = self.last_reg;
                    let dst = self.new_reg(elem);
                    self.emit(Op::UnwrapOr { dst, v: rreg, default: dflt }, sp.lo);
                    return Ok(elem);
                }
                "expect" => {
                    if args.len() != 1 {
                        self.ctx.err(sp, "expect(msg) takes a message");
                        return Err(());
                    }
                    let t = self.compile_expr(args[0], Some(TY_STR))?;
                    if t != TY_STR {
                        self.ctx.err(sp, "expect takes a `string`");
                    }
                    let msg = self.last_reg;
                    let dst = self.new_reg(elem);
                    self.emit(Op::Expect { dst, v: rreg, msg }, sp.lo);
                    return Ok(elem);
                }
                _ => {}
            },
            TyKind::Result { ok, .. } => match mname.as_str() {
                "is_ok" | "is_err" => {
                    let dst = self.new_reg(TY_BOOL);
                    self.emit(Op::SumIs { dst, v: rreg, want_err: mname == "is_err" }, sp.lo);
                    return Ok(TY_BOOL);
                }
                "unwrap_or" => {
                    if args.len() != 1 {
                        self.ctx.err(sp, "unwrap_or(default) takes one argument");
                        return Err(());
                    }
                    let t = self.compile_expr(args[0], Some(ok))?;
                    if t != ok {
                        self.ctx.err(sp, "unwrap_or type mismatch");
                    }
                    let dflt = self.last_reg;
                    let dst = self.new_reg(ok);
                    self.emit(Op::UnwrapOr { dst, v: rreg, default: dflt }, sp.lo);
                    return Ok(ok);
                }
                "expect" => {
                    if args.len() != 1 {
                        self.ctx.err(sp, "expect(msg) takes a message");
                        return Err(());
                    }
                    self.compile_expr(args[0], Some(TY_STR))?;
                    let msg = self.last_reg;
                    let dst = self.new_reg(ok);
                    self.emit(Op::Expect { dst, v: rreg, msg }, sp.lo);
                    return Ok(ok);
                }
                _ => {}
            },
            TyKind::Vec { elem } => match mname.as_str() {
                "len" => {
                    let dst = self.new_reg(TY_I32);
                    self.emit(Op::CallNat { nat: Nat::VecLen, recv: Some(rreg), args: vec![], dst: Some(dst) }, sp.lo);
                    return Ok(TY_I32);
                }
                "push" => {
                    if args.len() != 1 {
                        self.ctx.err(sp, "push(v) takes one argument");
                        return Err(());
                    }
                    let t = self.compile_expr(args[0], Some(elem))?;
                    if t != elem {
                        self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                            "push takes `{}`, found `{}`",
                            self.ctx.types.name(elem), self.ctx.types.name(t)
                        ));
                    }
                    self.emit(Op::CallNat { nat: Nat::VecPush, recv: Some(rreg), args: vec![self.last_reg], dst: None }, sp.lo);
                    return Ok(TY_UNIT);
                }
                "pop" => {
                    if !args.is_empty() {
                        self.ctx.err(sp, "pop() takes no arguments");
                        return Err(());
                    }
                    let dst = self.new_reg(elem);
                    self.emit(Op::CallNat { nat: Nat::VecPop, recv: Some(rreg), args: vec![], dst: Some(dst) }, sp.lo);
                    return Ok(elem);
                }
                _ => {}
            },
            TyKind::Array { len, .. } => match mname.as_str() {
                "len" => {
                    // folds to the const N (RFC 0032 §1.1 R1)
                    let dst = self.new_reg(TY_I32);
                    self.emit(Op::ConstRaw { dst, bits: len as u64 }, sp.lo);
                    return Ok(TY_I32);
                }
                _ => {}
            },
            TyKind::Str => match mname.as_str() {
                "len" => {
                    let dst = self.new_reg(TY_I32);
                    self.emit(Op::CallNat { nat: Nat::StrLen, recv: Some(rreg), args: vec![], dst: Some(dst) }, sp.lo);
                    return Ok(TY_I32);
                }
                _ => {}
            },
            TyKind::Opaque => {
                self.ctx.err(sp, "`Opaque` has no methods in this build —recover with `downcast<T>(o)` (RFC 0014)");
                return Err(());
            }
            _ => {}
        }
        // user types: inherent methods first (direct), then trait impls
        // (vtable —ALWAYS, RFC 0012 §1)
        if let TyKind::Data { .. } = self.ctx.types.kind(rt).clone() {
            let found = self
                .ctx
                .datas
                .iter()
                .find(|(_, d)| d.ty == rt)
                .and_then(|(n, d)| {
                    d.methods
                        .iter()
                        .find(|(mn, _)| *mn == name)
                        .map(|(_, mn)| (*n, *mn))
                });
            if let Some((dname, mnode)) = found {
                let d = self.ctx.find_data(dname).cloned().unwrap();
                return self.compile_inherent_call(dname, d, mnode, rreg, args, expected, sp);
            }
            for idx in self.ctx.impls_of(rt) {
                if self.ctx.impls[idx].methods.iter().any(|(n, _)| *n == name) {
                    return self.compile_trait_call(idx, name, rreg, args, expected, sp);
                }
            }
            self.ctx.err(sp, format!("`{}` has no method `{mname}`", self.ctx.types.name(rt)));
            return Err(());
        }
        if let TyKind::TraitObj { trait_id } = self.ctx.types.kind(rt).clone() {
            // dyn receiver: ONLY that trait's methods (RFC 0012 §2)
            let tdesc = self.ctx.trait_by_id(trait_id).clone();
            if let Some(midx) = tdesc.methods.iter().position(|m| m.name == mname) {
                let slot = self.ctx.trait_slot(trait_id, midx as u32).unwrap();
                return self.finish_trait_call(slot, tdesc.methods[midx].params.clone(), tdesc.methods[midx].ret, rreg, args, expected, sp);
            }
            self.ctx.err(sp, format!(
                "`dyn {}` reaches only `{}`'s methods —`{mname}` is not one of them (RFC 0012 §2)",
                tdesc.name, tdesc.name
            ));
            return Err(());
        }
        self.ctx.err(sp, format!("`{}` has no method `{mname}` in this build", self.ctx.types.name(rt)));
        Err(())
    }

    /// mut-binding law (RFC 0003 §1): writing through a handle requires the
    /// head binding to be `let mut`
    /// RFC 0012 §2: implicit widening — exact > `dyn I` when the exact type
    /// has an impl for I. Same-type always widens.
    pub(crate) fn widens(&self, from: TypeId, to: TypeId) -> bool {
        if from == to {
            return true;
        }
        if let TyKind::TraitObj { trait_id } = self.ctx.types.kind(to).clone() {
            return self.ctx.find_impl(trait_id, from).is_some();
        }
        false
    }

    pub(crate) fn check_recv_mut(&mut self, recv: NodeHandle<AnyExpr>, sp: rut_lexer::span::Span, what: &str) -> bool {
        match self.ctx.ast.expr(recv).clone() {
            ExprKind::Path { segs } if segs.len() == 1 => {
                if let Some(l) = self.lookup(segs[0].name) {
                    if !l.is_mut && !l.loop_var {
                        self.ctx.err(sp, format!(
                            "{what} requires a `let mut` binding (the mut-binding law, RFC 0003 §1)"
                        ));
                        return false;
                    }
                }
                true
            }
            ExprKind::Field { recv: inner, .. } | ExprKind::Index { recv: inner, .. } => {
                self.check_recv_mut(inner, sp, what)
            }
            _ => true,
        }
    }

    pub(crate) fn compile_inherent_call(
        &mut self,
        dname: IdentId,
        d: crate::check::DataDecl,
        mnode: NodeHandle<MethodDeclNode>,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let md = self.ctx.ast.method_decl(mnode).clone();
        let mut_self = matches!(
            md.params.first().map(|p| self.ctx.ast.param(*p)),
            Some(MemberKind::SelfParam(SelfParamData { is_mut: true }))
        );
        let (params, ret, mname) = (md.params, md.ret, md.name);
        let _ = mut_self;
        let mut ptys = Vec::new();
        // the callee's signature may spell `Self` — resolve its types under
        // the CALLEE's class (the caller's self_ty is irrelevant here)
        let saved_self = self.self_ty;
        self.self_ty = Some(d.ty);
        for p in params.iter().skip(1) {
            match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => ptys.push(self.resolve_type_now(*t)),
                _ => ptys.push(TY_I32),
            }
        }
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_UNIT);
        self.self_ty = saved_self;
        let _ = d;
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.types.name(t), self.ctx.types.name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Method { data: dname, name: mname },
            subst: vec![],
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_UNIT { None } else { Some(self.new_reg(ret_ty)) };
        self.emit(Op::CallM { func: fid, recv: rreg, args: aregs, dst }, sp.lo);
        Ok(ret_ty)
    }

    pub(crate) fn compile_trait_call(
        &mut self,
        impl_idx: usize,
        mname: IdentId,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let im = self.ctx.impls[impl_idx].clone();
        let tdesc = self.ctx.trait_by_id(im.trait_id).clone();
        let midx = tdesc
            .methods
            .iter()
            .position(|m| m.name == self.ctx.name(mname))
            .unwrap_or(0);
        let slot = self.ctx.trait_slot(im.trait_id, midx as u32).unwrap();
        self.finish_trait_call(slot, tdesc.methods[midx].params.clone(), tdesc.methods[midx].ret, rreg, args, expected, sp)
    }

    pub(crate) fn finish_trait_call(
        &mut self,
        slot: u32,
        param_tys: Vec<TypeId>,
        ret_ty: TypeId,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        if args.len() != param_tys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), param_tys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(param_tys[i]))?;
            if t != param_tys[i] {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.types.name(t), self.ctx.types.name(param_tys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let dst = if ret_ty == TY_UNIT { None } else { Some(self.new_reg(ret_ty)) };
        // trait-declared members NEVER devirtualize (RFC 0012 §1)
        self.emit(Op::CallI { slot, recv: rreg, args: aregs, dst }, sp.lo);
        Ok(ret_ty)
    }

    pub(crate) fn compile_field(&mut self, recv: NodeHandle<AnyExpr>, name: IdentId, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        let rt = self.compile_expr(recv, None)?;
        let rreg = self.last_reg;
        let fname = self.ctx.name(name).to_string();
        // Option/Result payload accessors (RFC 0005)
        match self.ctx.types.kind(rt).clone() {
            TyKind::Option { elem } => {
                if fname == "value" {
                    let dst = self.new_reg(elem);
                    self.emit(Op::Unwrap { dst, v: rreg, want_err: false }, sp.lo);
                    return Ok(elem);
                }
            }
            TyKind::Result { ok, err } => {
                if fname == "value" {
                    let dst = self.new_reg(ok);
                    self.emit(Op::Unwrap { dst, v: rreg, want_err: false }, sp.lo);
                    return Ok(ok);
                }
                if fname == "error" {
                    let dst = self.new_reg(err);
                    self.emit(Op::Unwrap { dst: dst, v: rreg, want_err: true }, sp.lo);
                    return Ok(err);
                }
            }
            _ => {}
        }
        if let TyKind::Data { fields } = self.ctx.types.kind(rt).clone() {
            if let Some(fidx) = fields.iter().position(|f| f.name == fname) {
                let fty = fields[fidx].ty;
                let dst = self.new_reg(fty);
                self.emit(Op::GetF { dst, obj: rreg, field: fidx as u32 }, sp.lo);
                return Ok(fty);
            }
            self.ctx.err(sp, format!("`{}` has no field `{fname}`", self.ctx.types.name(rt)));
            return Err(());
        }
        if matches!(self.ctx.types.kind(rt), TyKind::TraitObj { .. }) {
            self.ctx.err(sp, "trait objects have no fields —`d.x` on `dyn I` is a compile error (RFC 0012 §2)");
            return Err(());
        }
        self.ctx.err(sp, format!("`{}` has no field `{fname}`", self.ctx.types.name(rt)));
        Err(())
    }
}
