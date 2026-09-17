//! Calls and member resolution: builtin/free-fn/method/static dispatch,
//! generic instantiation by unification (RFC 0013 SS2), the RFC 0012
//! vtable-always rule for trait members, dyn receivers, and field reads.

use crate::check::TcResult;
use rut_core::ops::*;
use rut_core::types::*;
use super::slice::SliceSource;
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
        let dst = if ret == TY_NIL { None } else { Some(self.new_reg(ret)) };
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
        // std:core prelude functions (RFC 0028): compiler-lowered, visible
        // only when the name was imported from the std:core surface — the
        // prelude is imported, never ambient. A local fn of the same name
        // wins when the import is absent (fallthrough below).
        let core_fn = self.ctx.extern_native_fns.contains(&name);
        match n.as_str() {
            "own" => {
                self.ctx.err(sp, rut_core::binary::removed_core("own").unwrap());
                return Err(());
            }
            "make_ptr" if core_fn => {
                // make_ptr(v) (RFC 0005): box v into a fresh one-slot cell;
                // the result is a nil-able `*T`
                if args.len() != 1 {
                    self.ctx.err(sp, "make_ptr(v) takes one argument (RFC 0005)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                let src = self.last_reg;
                let pty = self.ctx.mk_ptr(t);
                let dst = self.new_reg(pty);
                self.emit(Op::MakePtr { dst, src, ty: pty }, sp.lo);
                return Ok(pty);
            }
            "on_drop" if core_fn => {
                // on_drop(p, cleanup) (RFC 0016 §3): cleanup runs when the
                // cell's refcount reaches zero
                if args.len() != 2 {
                    self.ctx.err(sp, "on_drop(p, cleanup) takes two arguments (RFC 0016 §3)");
                    return Err(());
                }
                let pt = self.compile_expr(args[0], None)?;
                if !matches!(self.ctx.types.kind(pt), TyKind::Ptr { .. }) {
                    self.ctx.err(sp, "on_drop needs a pointer —`*T` from `make_ptr` (RFC 0016 §3)");
                    return Err(());
                }
                let obj = self.last_reg;
                let ct = self.compile_expr(args[1], None)?;
                if !matches!(self.ctx.types.kind(ct), TyKind::Fn { .. }) {
                    self.ctx.err(sp, "on_drop's cleanup must be a function value —`fn(*T)` (RFC 0016 §3)");
                    return Err(());
                }
                let cleanup = self.last_reg;
                self.emit(Op::OnDrop { obj, cleanup }, sp.lo);
                return Ok(TY_NIL);
            }
            "downcast" if core_fn => {
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
                // v1.1: downcast yields a TUPLE `(T, bool)` — the value and
                // a success flag; no Option in the language anymore
                let tty = self.ctx.mk_tuple([want, TY_BOOL].to_vec());
                let orecv = self.last_reg;
                let tid_reg = self.new_reg(TY_U32);
                self.emit(Op::TidOf { dst: tid_reg, obj: orecv }, sp.lo);
                let want_reg = self.new_reg(TY_U32);
                let wk = self.konst(ConstVal::TypeId(want));
                self.emit(Op::Const { dst: want_reg, k: wk as u32 }, sp.lo);
                let eq = self.new_reg(TY_BOOL);
                self.emit(cmpop(CmpOp::Eq, PrimTy::U32, eq, tid_reg, want_reg), sp.lo);
                let dst = self.new_reg(tty);
                let l_some = self.new_label();
                let l_none = self.new_label();
                let l_end = self.new_label();
                self.br(eq, l_some, l_none);
                self.bind(l_some);
                let un = self.new_reg(want);
                self.emit(Op::Unbox { dst: un, box_: orecv, ty: want }, sp.lo);
                self.emit(Op::MakeRecord { dst, ty: tty, vals: vec![un, eq] }, sp.lo);
                self.jmp(l_end);
                self.bind(l_none);
                let zero = self.new_reg(want);
                self.emit(Op::ConstRaw { dst: zero, bits: 0 }, sp.lo);
                let no = self.new_reg(TY_BOOL);
                self.emit(Op::ConstRaw { dst: no, bits: 0 }, sp.lo);
                self.emit(Op::MakeRecord { dst, ty: tty, vals: vec![zero, no] }, sp.lo);
                self.bind(l_end);
                // the value lives in `dst`; move it out so last_reg holds it
                let out = self.new_reg(tty);
                self.emit(Op::MovRef { dst: out, src: dst }, sp.lo);
                return Ok(tty);
            }
            "panic" if core_fn => {
                if args.len() != 1 {
                    self.ctx.err(sp, "panic(msg) takes a message (RFC 0034 §2)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_STR))?;
                if t != TY_STR {
                    self.ctx.err(sp, "panic takes a `str`");
                }
                self.emit(Op::Panic { msg: self.last_reg }, sp.lo);
                return Ok(TY_NIL);
            }
            "assert" if core_fn => {
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
                        self.ctx.err(sp, "assert message must be a `str`");
                    }
                    Some(self.last_reg)
                } else {
                    None
                };
                self.emit(Op::Assert { cond, msg }, sp.lo);
                return Ok(TY_NIL);
            }
            "print" => {
                self.ctx.err(sp, "`print` was removed — import a logger (`import { log } from \"std:log\"`)");
                return Err(());
            }
            "string_len" if core_fn => {
                if args.len() != 1 {
                    self.ctx.err(sp, "string_len(s) takes one `str`");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_STR))?;
                if t != TY_STR {
                    self.ctx.err(sp, format!("string_len takes a `str`, found `{}`", self.ctx.types.name(t)));
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_I32);
                self.emit(Op::CallNat { nat: Nat::StrLen, recv: Some(src), args: vec![], dst: Some(dst) }, sp.lo);
                return Ok(TY_I32);
            }
            "string_join" if core_fn => {
                // join every element of an `Array<str>` in one pass: the
                // native sizes once and allocates once (RFC 0032 §1.1 R2).
                if args.len() != 1 {
                    self.ctx.err(sp, "string_join(parts) takes one `Array<str>`");
                    return Err(());
                }
                let hint = Some(self.ctx.mk_array(TY_STR));
                let at = self.compile_expr(args[0], hint)?;
                match self.ctx.types.kind(at) {
                    TyKind::Array { elem } if *elem == TY_STR => {}
                    _ => {
                        self.ctx.err(sp, format!("string_join expects `Array<str>` —found `{}`", self.ctx.types.name(at)));
                        return Err(());
                    }
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_STR);
                self.emit(Op::CallNat { nat: Nat::StrJoin, recv: None, args: vec![src], dst: Some(dst) }, sp.lo);
                return Ok(TY_STR);
            }
            "type_id" => {
                // compile-time constant —never executed (RFC 0015 §3, 0033 §3)
                if generics.len() != 1 || !args.is_empty() {
                    self.ctx.err(sp, format!("{n}<T>() takes one explicit type argument"));
                    return Err(());
                }
                let t = self.resolve_type_now(generics[0]);
                let dst = self.new_reg(TY_U32);
                // a rebasable const-pool entry: link maps the module-local
                // TypeId into the global table (RFC 0035 §1)
                let k = self.konst(ConstVal::TypeId(t));
                self.emit(Op::Const { dst, k: k as u32 }, sp.lo);
                return Ok(TY_U32);
            }
            "size_of" | "align_of" => {
                // the repr-C layout contract was removed: records are slot
                // arrays, not byte blocks, so there is no value size/alignment
                self.ctx.err(sp, format!(
                    "`{n}` was removed — records are stored as one slot per field, not a repr-C block"
                ));
                return Err(());
            }
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" => {
                // removed when the `as` cast landed (RFC 0007 §1) — the
                // conversion family is spelled `x as T` now. The message
                // mirrors the `size_of` removal above.
                self.ctx.err(sp, format!("`{n}(x)` was removed — use `x as {n}` (RFC 0007 §1)"));
                return Err(());
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
        // imported function: signature from the surface, a direct call to the
        // exporter's scope-qualified id (RFC 0029 surface / RFC 0035 §1)
        if let Some(ef) = self.ctx.extern_fn(name).cloned() {
            if let Some(i) = ef.intrinsic {
                // a compiler-lowered intrinsic (RFC 0032 §1.1 R2): no call
                return self.compile_intrinsic(i, &args, expected, sp);
            }
            if !generics.is_empty() {
                self.ctx.err(sp, format!("`{}` is an imported fn and takes no type arguments", self.ctx.name(name)));
                return Err(());
            }
            if args.len() != ef.params.len() {
                self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ef.params.len()));
                return Err(());
            }
            let mut aregs = Vec::new();
            for (i, a) in args.iter().enumerate() {
                let t = self.compile_expr(*a, Some(ef.params[i]))?;
                if !self.widens(t, ef.params[i]) {
                    self.ctx.err(self.ctx.ast.span(a.id()), format!(
                        "argument {} is `{}`, `{}` expected",
                        i + 1, self.ctx.types.name(t), self.ctx.types.name(ef.params[i])
                    ));
                }
                aregs.push(self.clone_arg(self.last_reg, ef.params[i], sp.lo));
            }
            let dst = if ef.ret == TY_NIL { None } else { Some(self.new_reg(ef.ret)) };
            self.emit(Op::Call { func: ef.func, args: aregs, dst }, sp.lo);
            return Ok(ef.ret);
        }
        // builtin type-call: Array<T>(n) — allocate n slots (runtime length,
        // non-growable, RFC 0005); the storage under `std:collection`'s Vec.
        // Import-gated like the type itself (RFC 0028)
        if n == "Array"
            && self.ctx.find_data(name).is_none()
            && self.ctx.extern_native_types.get(&name).copied()
                == Some(rut_core::binary::NativeTy::Array)
        {
            if generics.len() != 1 {
                self.ctx.err(sp, "Array<T>(n) takes one type argument");
                return Err(());
            }
            let elem = self.resolve_type_now(generics[0]);
            let aty = self.ctx.mk_array(elem);
            let len = match args.len() {
                0 => {
                    let z = self.new_reg(TY_I32);
                    self.emit(Op::ConstRaw { dst: z, bits: 0 }, sp.lo);
                    z
                }
                1 => {
                    let t = self.compile_expr(args[0], Some(TY_I32))?;
                    if t != TY_I32 {
                        self.ctx.err(sp, format!("Array<T>(n) takes an `i32` length, found `{}`", self.ctx.types.name(t)));
                    }
                    self.last_reg
                }
                _ => {
                    self.ctx.err(sp, "Array<T>() or Array<T>(n)");
                    return Err(());
                }
            };
            let dst = self.new_reg(aty);
            self.emit(Op::ArrNew { dst, ty: aty, len, repr: self.ctx.types.repr_of(elem) }, sp.lo);
            return Ok(aty);
        }
        // builtin bytes type-call: `bytes(n)` zeroed (RFC 0004)
        if n == "bytes" {
            return self.compile_bytes_alloc(args, sp);
        }
        if self.ctx.find_data(name).is_some() {
            self.ctx.err(sp, format!(
                "construction is a method call, never a type-call —use a class method ({}.new(..)) or a struct literal `{} {{ .. }}` (RFC 0010 §1)",
                n, n
            ));
            return Err(());
        }
        let msg = self
            .ctx
            .not_in_core_scope(&n)
            .unwrap_or_else(|| format!("unknown function `{n}`"));
        self.ctx.err(sp, msg);
        Err(())
    }

    /// `bytes(n)` — a zeroed immutable buffer of `n` octets (RFC 0004).
    pub(crate) fn compile_bytes_alloc(&mut self, args: Vec<NodeHandle<AnyExpr>>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        let len_reg = match args.len() {
            0 => {
                let z = self.new_reg(TY_I32);
                self.emit(Op::ConstRaw { dst: z, bits: 0 }, sp.lo);
                z
            }
            1 => {
                let t = self.compile_expr(args[0], Some(TY_I32))?;
                if t != TY_I32 {
                    self.ctx.err(sp, format!("bytes(n) takes an `i32` length, found `{}`", self.ctx.types.name(t)));
                }
                self.last_reg
            }
            _ => {
                self.ctx.err(sp, "bytes() or bytes(n)");
                return Err(());
            }
        };
        let dst = self.new_reg(TY_BYTES);
        self.emit(Op::ArrNew { dst, ty: TY_BYTES, len: len_reg, repr: self.ctx.types.repr_of(TY_U8) }, sp.lo);
        Ok(TY_BYTES)
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
        // std:core builtin statics (RFC 0028): `Option.some`, `Result.ok`,
        // `Opaque.new` — the prelude is imported, never ambient, so the
        // arms fire only when the base name was bound from the surface
        let core_ty = self.ctx.extern_native_types.get(&base).copied();
        // Explicit type args on a static head are meaningful only where the
        // member can use them (`Vec<u32>.from(..)` — the element type);
        // everywhere else they stay unsupported rather than silently ignored.
        let is_data = self.ctx.find_data(base).is_some();
        if !base_generics.is_empty() && !is_data {
            self.ctx.err(sp, format!("generic type paths (`{bn}<..>.{mn}`) are not supported in this build"));
            return Err(());
        }
        // an imported namespace's members (`Math.sqrt`; RFC 0028) —
        // routed by the bound head, name-generic
        if self.ctx.is_extern_namespace(base) {
            return self.compile_namespace_member(base, member, &args, expected, sp);
        }
        match (bn.as_str(), mn.as_str()) {
            ("Opaque", "new") if core_ty == Some(rut_core::binary::NativeTy::Opaque) => {
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
            ("bytes", "from") => {
                // bytes.from(a) — copy an Array<u8> into an immutable
                // buffer (RFC 0004)
                if args.len() != 1 {
                    self.ctx.err(sp, "bytes.from(source) takes one `Array<u8>`");
                    return Err(());
                }
                let hint = Some(self.ctx.mk_array(TY_U8));
                let at = self.compile_expr(args[0], hint)?;
                match self.ctx.types.kind(at) {
                    TyKind::Array { elem } if *elem == TY_U8 => {}
                    _ => {
                        self.ctx.err(sp, format!("bytes.from expects `Array<u8>` —found `{}`", self.ctx.types.name(at)));
                        return Err(());
                    }
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_BYTES);
                self.emit(Op::Own { dst, src, ty: TY_BYTES }, sp.lo);
                return Ok(TY_BYTES);
            }
            ("bytes", "zeroed") => {
                // bytes.zeroed(n) — n zeroed octets (RFC 0004)
                if args.len() != 1 {
                    self.ctx.err(sp, "bytes.zeroed(n) takes one `i32`");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_I32))?;
                if t != TY_I32 {
                    self.ctx.err(sp, "bytes.zeroed takes an `i32`");
                }
                let len_reg = self.last_reg;
                let dst = self.new_reg(TY_BYTES);
                self.emit(Op::ArrNew { dst, ty: TY_BYTES, len: len_reg, repr: self.ctx.types.repr_of(TY_U8) }, sp.lo);
                return Ok(TY_BYTES);
            }
            ("str", "from_code") => {
                // str.from_code(n) -> str — the 1-codepoint str for the
                // codepoint `n` (RFC 0004 v1.1: `char` is gone)
                if args.len() != 1 {
                    self.ctx.err(sp, "str.from_code(n) takes one `u32` codepoint");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_U32))?;
                if t != TY_U32 {
                    self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                        "str.from_code takes a `u32`, found `{}`",
                        self.ctx.types.name(t)
                    ));
                }
                let cp = self.last_reg;
                let c = self.new_reg(TY_CHAR);
                self.emit(Op::Conv { dst: c, src: cp, from: PrimTy::U32, to: PrimTy::Char }, sp.lo);
                let dst = self.new_reg(TY_STR);
                self.emit(Op::CallNat { nat: Nat::Str, recv: None, args: vec![c], dst: Some(dst) }, sp.lo);
                return Ok(TY_STR);
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
                // class instantiation args: explicit `Name<..>`, else the
                // enclosing class's own args (a `Self`-ish call in a body)
                let class_args: Vec<TypeId> = if !base_generics.is_empty() {
                    base_generics.iter().map(|g| self.resolve_type_now(*g)).collect()
                } else if !d.generics.is_empty() && self.current_class == Some(dname) {
                    // a `Self`-ish call inside the class body: the enclosing
                    // method's class args
                    d.generics
                        .iter()
                        .map(|g| {
                            self.subst
                                .iter()
                                .find(|(n, _)| n == g)
                                .map(|(_, t)| *t)
                                .unwrap_or(TY_I32)
                        })
                        .collect()
                } else if !d.generics.is_empty() {
                    // infer from the expected type: `let b: Box<i32> = Box.new(..)`
                    match expected.and_then(|e| self.ctx.inst_data.get(&e).cloned()) {
                        Some((ed, eargs)) if ed == dname => eargs,
                        _ => {
                            self.ctx.err(sp, format!(
                                "cannot infer the type arguments for `{bn}` — write `{bn}<..>.{mn}(..)` or annotate the binding"
                            ));
                            return Err(());
                        }
                    }
                } else {
                    vec![]
                };
                if !d.generics.is_empty() && class_args.len() != d.generics.len() {
                    self.ctx.err(sp, format!(
                        "`{bn}<..>` takes {} type argument(s), {} given",
                        d.generics.len(),
                        class_args.len()
                    ));
                    return Err(());
                }
                let self_ty = if d.generics.is_empty() {
                    d.ty
                } else {
                    self.ctx.mk_data_inst(dname, class_args.clone())
                };
                return self.compile_direct_method(dname, class_args, self_ty, mnode, args, sp);
            }
        }        if let Some(e) = self.ctx.find_enum(base) {
            let _ = e;
            self.ctx.err(sp, format!("enum `{bn}` has no static `{mn}` in this build"));
            return Err(());
        }
        let msg = self
            .ctx
            .not_in_core_scope(&bn)
            .unwrap_or_else(|| format!("unknown name `{bn}.{mn}`"));
        self.ctx.err(sp, msg);
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
        let ret_ty = ret.map(|r| self.ctx.resolve_type(r, &subst)).unwrap_or(TY_NIL);
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Free(name),
            subst,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        self.emit(Op::Call { func: fid, args: aregs, dst }, sp.lo);
        Ok(ret_ty)
    }


    /// class-method call through the type name (`Circle.new(..)`)
    pub(crate) fn compile_direct_method(
        &mut self,
        dname: IdentId,
        class_args: Vec<TypeId>,
        self_ty: TypeId,
        mnode: NodeHandle<MethodDeclNode>,
        args: Vec<NodeHandle<AnyExpr>>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let d = self.ctx.find_data(dname).cloned().unwrap();
        let class_subst: Vec<(IdentId, TypeId)> =
            d.generics.iter().cloned().zip(class_args.iter().cloned()).collect();
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
        // the callee's signature may spell `Self`/`T` — resolve under the
        // CALLEE's class instantiation (the caller's context is irrelevant)
        let saved_self = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, class_subst.clone());
        self.self_ty = Some(self_ty);
        for p in &params {
            match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => ptys.push(self.resolve_type_now(*t)),
                _ => ptys.push(TY_I32),
            }
        }
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_NIL);
        self.self_ty = saved_self;
        self.subst = saved_subst;
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
            subst: class_subst,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
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
        // the static-call compiler when the head is not shadowed by a local.
        // The head may carry generic args (`Vec<u32>.from(..)`) — they go
        // along; compile_static_call decides which statics can use them.
        if let ExprKind::Path { segs } = self.ctx.ast.expr(recv).clone() {
            if segs.len() == 1 && self.lookup(segs[0].name).is_none() {
                let base = segs[0].name;
                let bn = self.ctx.name(base).to_string();
                // `Vec` is an ordinary class (std:collection), so it routes
                // here through `find_data`, like any other class; the
                // std:core statics (`Option`, `Result`, `Opaque`) route only
                // when imported (RFC 0028)
                let is_type = matches!(bn.as_str(), "str" | "bytes")
                    || self.ctx.extern_native_types.contains_key(&base)
                    || self.ctx.is_extern_namespace(base)
                    || self.ctx.find_enum(base).is_some()
                    || self.ctx.find_data(base).is_some();
                if is_type {
                    return self.compile_static_call(base, segs[0].generics.clone(), name, generics, args, expected, sp);
                }
            }
        }
        // receiver bypass (RFC 0009/0016 v1.1): a mutating method operates
        // on the ORIGINAL binding — a single-name receiver uses its register
        // raw instead of a boundary clone
        let recv_raw = match self.ctx.ast.expr(recv).clone() {
            ExprKind::Path { segs } if segs.len() == 1 && segs[0].generics.is_empty() => {
                self.lookup(segs[0].name).map(|l| (l.ty, l.reg))
            }
            _ => None,
        };
        let (rt, rreg) = match recv_raw {
            Some((ty, reg)) => self.deref_for_use(ty, reg, sp.lo),
            None => {
                let rt = self.compile_expr(recv, None)?;
                let rreg = self.last_reg;
                // `p.m(..)` auto-derefs (RFC 0005)
                self.deref_for_use(rt, rreg, sp.lo)
            }
        };
        let mname = self.ctx.name(name).to_string();
        // primitives have no method syntax (RFC 0004/0012): `str`/`bytes`
        // operations are free functions (`string_len`, `string_encode`,
        // `bytes_len`, `bytes_decode`, `bytes_from`) — except `s.code()`,
        // the v1.1 codepoint reader that replaced `char` (RFC 0004)
        match self.ctx.types.kind(rt) {
            TyKind::Str => {
                if mname == "code" && args.is_empty() {
                    // s.code() -> u32 — the FIRST codepoint; traps on empty
                    let creg = self.new_reg(TY_CHAR);
                    let zero = self.new_reg(TY_I32);
                    self.emit(Op::ConstRaw { dst: zero, bits: 0 }, sp.lo);
                    self.emit(Op::StrCharAt { dst: creg, s: rreg, idx: zero }, sp.lo);
                    let dst = self.new_reg(TY_U32);
                    self.emit(Op::Conv { dst, src: creg, from: PrimTy::Char, to: PrimTy::U32 }, sp.lo);
                    return Ok(TY_U32);
                }
                if mname == "encode" && args.is_empty() {
                    // s.encode() -> bytes — the UTF-8 octets (RFC 0004)
                    let dst = self.emit_string_encode(rreg, sp.lo);
                    self.last_reg = dst;
                    return Ok(TY_BYTES);
                }
                if mname == "slice" && args.len() == 2 {
                    // s.slice(from, to) — an O(1) view (RFC 0042)
                    let ft = self.compile_expr(args[0], Some(TY_I32))?;
                    if ft != TY_I32 {
                        self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                        return Err(());
                    }
                    let fr = self.last_reg;
                    let tt = self.compile_expr(args[1], Some(TY_I32))?;
                    if tt != TY_I32 {
                        self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                        return Err(());
                    }
                    let tr = self.last_reg;
                    let dst = self.new_reg(TY_STR);
                    self.emit(
                        Op::CallNat { nat: Nat::StrSlice, recv: Some(rreg), args: vec![fr, tr], dst: Some(dst) },
                        sp.lo,
                    );
                    return Ok(TY_STR);
                }
                if mname == "len" && args.is_empty() {
                    if let Some(info) = self.slice_info(rt) {
                        self.emit_slice_len(rreg, &info, sp.lo)?;
                        return Ok(TY_I32);
                    }
                }
                let who = recv_name(&self.ctx, recv);
                self.ctx.err(sp, format!(
                    "`str` has no method `{mname}` — its members are `len`/`slice`/`code`/`encode` (`string_len({who})` is the free-fn spelling)"
                ));
                return Err(());
            }
            TyKind::Bytes => {
                if mname == "decode" && args.is_empty() {
                    // b.decode() -> str — UTF-8, lossy (RFC 0004)
                    let dst = self.emit_bytes_decode(rreg, sp.lo);
                    self.last_reg = dst;
                    return Ok(TY_STR);
                }
                if mname == "len" && args.is_empty() {
                    if let Some(info) = self.slice_info(rt) {
                        self.emit_slice_len(rreg, &info, sp.lo)?;
                        return Ok(TY_I32);
                    }
                }
                let who = recv_name(&self.ctx, recv);
                self.ctx.err(sp, format!(
                    "`bytes` has no method `{mname}` — its members are `len`/`decode` (`bytes_len({who})` is the free-fn spelling)"
                ));
                return Err(());
            }
            _ => {}
        }
        if !generics.is_empty() {
            self.ctx.err(sp, "generic method calls are not supported in this build");
            return Err(());
        }
        // `s.len()` — the Slice surface member shared by every sequence;
        // lowered fused (RFC 0032 §1.1 R1), including the `Vec<T>` class
        if mname == "len" && args.is_empty() {
            if let Some(info) = self.slice_info(rt) {
                self.emit_slice_len(rreg, &info, sp.lo)?;
                return Ok(TY_I32);
            }
        }
        // `v.slice(from, to)` — an O(1) array window (RFC 0042 §6): mints
        // an `ArrView` cell over the backing array and boxes it as
        // `*Vec<T>`/`*Array<T>`. Reads AND writes through the pointer go
        // to the parent (the `*T` aliasing law); the window is
        // fixed-length. str has its own slice (handled above).
        if mname == "slice" && args.len() == 2 {
            if let Some(info) = self.slice_info(rt) {
                match &info.source {
                    SliceSource::DataBuf { buf_field, len_field } => {
                        let arr_ty = self.ctx.mk_array(info.elem);
                        let arr = self.new_reg(arr_ty);
                        self.emit(Op::GetF { dst: arr, obj: rreg, field: *buf_field, repr: Repr::Ref }, sp.lo);
                        let live = self.new_reg(TY_I32);
                        self.emit(Op::GetF { dst: live, obj: rreg, field: *len_field, repr: Repr::Prim(PrimTy::I32) }, sp.lo);
                        let ft = self.compile_expr(args[0], Some(TY_I32))?;
                        if ft != TY_I32 {
                            self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                            return Err(());
                        }
                        let fr = self.last_reg;
                        let tt = self.compile_expr(args[1], Some(TY_I32))?;
                        if tt != TY_I32 {
                            self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                            return Err(());
                        }
                        let tr = self.last_reg;
                        let view = self.new_reg(arr_ty);
                        self.emit(
                            Op::CallNat { nat: Nat::ArrSlice, recv: Some(arr), args: vec![fr, tr, live], dst: Some(view) },
                            sp.lo,
                        );
                        let ptr_ty = self.ctx.mk_ptr(rt);
                        let dst = self.new_reg(ptr_ty);
                        self.emit(Op::MakePtr { dst, src: view, ty: ptr_ty }, sp.lo);
                        return Ok(ptr_ty);
                    }
                    SliceSource::Array => {
                        let ft = self.compile_expr(args[0], Some(TY_I32))?;
                        if ft != TY_I32 {
                            self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                            return Err(());
                        }
                        let fr = self.last_reg;
                        let tt = self.compile_expr(args[1], Some(TY_I32))?;
                        if tt != TY_I32 {
                            self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                            return Err(());
                        }
                        let tr = self.last_reg;
                        let live = self.new_reg(TY_I32);
                        self.emit(Op::CallNat { nat: Nat::ArrLen, recv: Some(rreg), args: vec![], dst: Some(live) }, sp.lo);
                        let view = self.new_reg(rt);
                        self.emit(
                            Op::CallNat { nat: Nat::ArrSlice, recv: Some(rreg), args: vec![fr, tr, live], dst: Some(view) },
                            sp.lo,
                        );
                        let ptr_ty = self.ctx.mk_ptr(rt);
                        let dst = self.new_reg(ptr_ty);
                        self.emit(Op::MakePtr { dst, src: view, ty: ptr_ty }, sp.lo);
                        return Ok(ptr_ty);
                    }
                    _ => {
                        self.ctx.err(sp, "`slice` on this sequence is not supported");
                        return Err(());
                    }
                }
            }
        }
        // builtin members (RFC 0005 table): `Opaque` rejects methods
        match self.ctx.types.kind(rt).clone() {
            TyKind::Opaque => {
                self.ctx.err(sp, "`Opaque` has no methods in this build —recover with `downcast<T>(o)` (RFC 0014)");
                return Err(());
            }
            _ => {}
        }
        // user types: inherent methods first (direct), then trait impls
        // (vtable —ALWAYS, RFC 0012 §1)
        if let TyKind::Data { .. } = self.ctx.types.kind(rt).clone() {
            // the receiver is either an instantiated generic (decl + args in
            // `inst_data`) or a local non-generic record
            let target = match self.ctx.inst_data.get(&rt).cloned() {
                Some((dname, cargs)) => Some((dname, cargs)),
                None => self
                    .ctx
                    .datas
                    .iter()
                    .find(|(_, d)| d.ty == rt)
                    .map(|(n, _)| (*n, vec![])),
            };
            let found = target.and_then(|(dname, cargs)| {
                self.ctx
                    .find_data(dname)
                    .and_then(|d| {
                        d.methods
                            .iter()
                            .find(|(mn, _)| *mn == name)
                            .map(|(_, mnode)| (*mnode, cargs.clone()))
                    })
                    .map(|(mnode, cargs)| (dname, cargs, mnode))
            });
            if let Some((dname, class_args, mnode)) = found {
                return self.compile_inherent_call(dname, class_args, rt, mnode, rreg, args, expected, sp);
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
    pub(crate) fn widens(&mut self, from: TypeId, to: TypeId) -> bool {
        if from == to {
            return true;
        }
        if let TyKind::TraitObj { trait_id } = self.ctx.types.kind(to).clone() {
            // duck-typed satisfaction at the coercion (RFC 0012 v1.1)
            return self.ctx.duck_satisfies(from, trait_id);
        }
        false
    }

    pub(crate) fn check_recv_mut(&mut self, recv: NodeHandle<AnyExpr>, sp: rut_lexer::span::Span, what: &str) -> bool {
        match self.ctx.ast.expr(recv).clone() {
            ExprKind::Path { segs } if segs.len() == 1 => {
                if let Some(l) = self.lookup(segs[0].name) {
                    // a pointer binding is immutable, its POINTEE is not —
                    // element stores through `xs: *Vec<i32>` are legal
                    let head_is_ptr = matches!(
                        self.ctx.types.kind(l.ty).clone(),
                        TyKind::Ptr { .. }
                    );
                    if !l.is_mut && !l.loop_var && !head_is_ptr {
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
        class_args: Vec<TypeId>,
        self_ty: TypeId,
        mnode: NodeHandle<MethodDeclNode>,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let d = self.ctx.find_data(dname).cloned().unwrap();
        let class_subst: Vec<(IdentId, TypeId)> =
            d.generics.iter().cloned().zip(class_args.iter().cloned()).collect();
        let md = self.ctx.ast.method_decl(mnode).clone();
        let mut_self = matches!(
            md.params.first().map(|p| self.ctx.ast.param(*p)),
            Some(MemberKind::SelfParam(SelfParamData { is_mut: true }))
        );
        let (params, ret, mname) = (md.params, md.ret, md.name);
        let _ = mut_self;
        let mut ptys = Vec::new();
        // the callee's signature may spell `Self`/`T` — resolve under the
        // CALLEE's class instantiation (the caller's context is irrelevant)
        let saved_self = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, class_subst.clone());
        self.self_ty = Some(self_ty);
        for p in params.iter().skip(1) {
            match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => ptys.push(self.resolve_type_now(*t)),
                _ => ptys.push(TY_I32),
            }
        }
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_NIL);
        self.self_ty = saved_self;
        self.subst = saved_subst;
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
            // ptys here excludes `self` — args align 1:1
            aregs.push(self.clone_arg(self.last_reg, ptys[i], sp.lo));
        }
        // small instance methods inline at the call site: the class's
        // `push`/`pop`/`freeze` are rut code (RFC 0005), so an interpreted
        // frame per call is the cost of the design; inlining removes it
        if self.try_inline_method(
            dname, &class_subst, self_ty, mnode, mname, mut_self, rreg, &aregs, &ptys, ret_ty, sp,
        ) {
            return Ok(ret_ty);
        }
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Method { data: dname, name: mname },
            subst: class_subst,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        self.emit(Op::CallM { func: fid, recv: rreg, args: aregs, dst }, sp.lo);
        Ok(ret_ty)
    }

    /// Inline a small, non-recursive instance method body at the call site.
    /// Returns `true` when it compiled the body; `false` to emit `CallM`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_inline_method(
        &mut self,
        dname: IdentId,
        class_subst: &[(IdentId, TypeId)],
        self_ty: TypeId,
        mnode: NodeHandle<MethodDeclNode>,
        mname: IdentId,
        mut_self: bool,
        recv: u16,
        aregs: &[u16],
        ptys: &[TypeId],
        ret_ty: TypeId,
        sp: rut_lexer::span::Span,
    ) -> bool {
        const MAX_STMTS: usize = 24;
        const MAX_DEPTH: usize = 4;
        if self.inline_stack.len() >= MAX_DEPTH || self.inline_stack.contains(&(dname, mname)) {
            return false;
        }
        let md = self.ctx.ast.method_decl(mnode).clone();
        if md.is_suspend {
            return false; // `suspend` is diagnosed when the body is compiled
        }
        let Some(body) = md.body else { return false };
        let stmts = match self.ctx.ast.kind(body.id()) {
            Kind::Expr(ExprKind::Block { stmts }) => stmts.clone(),
            _ => return false,
        };
        if stmts.len() > MAX_STMTS {
            return false;
        }
        let saved_self_ty = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, class_subst.to_vec());
        let saved_class = self.current_class;
        let saved_ret = self.ret_ty;
        let saved_inline_ret = self.inline_ret;
        let saved_inline_self = self.inline_self;
        self.self_ty = Some(self_ty);
        self.current_class = Some(dname);
        self.ret_ty = ret_ty;
        let base = self.locals.len();
        let self_id = self.ctx.lookup_name("self");
        if let Some(sid) = self_id {
            self.locals.push(Local { name: sid, reg: recv, ty: self_ty, is_mut: mut_self, loop_var: false });
            self.inline_self = Some((sid, recv));
        }
        let params: Vec<NodeHandle<AnyParam>> = md.params.clone();
        let mut ai = 0usize;
        for p in &params {
            if let MemberKind::Param(ParamData { name, is_mut, .. }) = self.ctx.ast.param(*p) {
                self.locals.push(Local { name: *name, reg: aregs[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false });
                ai += 1;
            }
        }
        let res = self.new_reg(ret_ty);
        let l_end = self.new_label();
        self.inline_ret = Some((res, l_end));
        self.inline_stack.push((dname, mname));
        let ok = self.compile_block(body.id()).is_ok();
        self.inline_stack.pop();
        self.bind(l_end);
        self.locals.truncate(base);
        self.self_ty = saved_self_ty;
        self.subst = saved_subst;
        self.current_class = saved_class;
        self.ret_ty = saved_ret;
        self.inline_ret = saved_inline_ret;
        self.inline_self = saved_inline_self;
        let _ = sp;
        if !ok {
            return false;
        }
        self.last_reg = res;
        true
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
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        // trait-declared members NEVER devirtualize (RFC 0012 §1)
        self.emit(Op::CallI { slot, recv: rreg, args: aregs, dst }, sp.lo);
        Ok(ret_ty)
    }

    pub(crate) fn compile_field(&mut self, recv: NodeHandle<AnyExpr>, name: IdentId, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // `<namespace>.CONST` — an imported namespace's constant (checked
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
        let fname = self.ctx.name(name).to_string();
        // `p.x` auto-derefs (RFC 0005): load the pointee cell first, then
        // the field reads from it
        let (rreg, rt) = match self.ctx.types.kind(rt).clone() {
            TyKind::Ptr { elem } if matches!(self.ctx.types.kind(elem), TyKind::Data { .. }) => {
                let dreg = self.new_reg(elem);
                self.emit(Op::GetF { dst: dreg, obj: rreg, field: 0, repr: Repr::Ref }, sp.lo);
                (dreg, elem)
            }
            _ => (rreg, rt),
        };
        if let TyKind::Data { fields } = self.ctx.types.kind(rt).clone() {
            if let Some(fidx) = fields.iter().position(|f| f.name == fname) {
                let fty = fields[fidx].ty;
                let dst = self.new_reg(fty);
                self.emit(Op::GetF { dst, obj: rreg, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
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

/// The receiver's source name for a diagnostic (`x` for `x.f()`, else
/// `expr`) — used when a primitive method call is rejected.
fn recv_name(ctx: &crate::check::Ctx, recv: NodeHandle<AnyExpr>) -> String {
    match ctx.ast.expr(recv) {
        ExprKind::Path { segs } => {
            segs.iter().map(|s| ctx.name(s.name).to_string()).collect::<Vec<_>>().join(".")
        }
        _ => "expr".to_string(),
    }
}
