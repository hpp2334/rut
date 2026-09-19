//! Calls and member resolution: builtin/free-fn/method/static dispatch,
//! generic instantiation by unification (RFC 0013 SS2), the RFC 0012
//! vtable-always rule for trait members, trait-typed receivers, and field reads.

use crate::check::{ImplHit, TcResult};
use rut_core::ops::*;
use rut_core::sym;
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
                        self.ctx.err(sp, format!("`{}` is not callable", self.ctx.type_name(ft)));
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
                self.ctx.err(sp, format!("`{}` is not callable", self.ctx.type_name(ft)));
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
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let dst = if ret == TY_NIL { None } else { Some(self.new_reg(ret)) };
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::CallFn { fval: freg, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
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
            self.ctx.err(sp, "unsupported call path (module loading is not available in this build, RFC 0035)");
            return Err(());
        }
        let name = segs[0].name;
        let generics = segs[0].generics.clone();
        // core prelude functions (RFC 0028): compiler-lowered, visible
        // only when the name was used from the core surface — the
        // prelude is used, never ambient. A local fn of the same name
        // wins when the use statement is absent (fallthrough below).
        let core_fn = self.ctx.extern_native_fns.contains(&name);
        // removed prelude spellings diagnose themselves — compared as
        // symbols against the interned removal table, they are not
        // well-known symbols
        if let Some(msg) = self.ctx.removed_core(name) {
            self.ctx.err(sp, msg);
            return Err(());
        }
        if name == sym::PRINT {
            self.ctx.err(sp, "`print` was removed — use a logger (`use ink::{log}`)");
            return Err(());
        }
        if matches!(name, sym::SIZE_OF | sym::ALIGN_OF) {
            // the repr-C layout contract was removed: records are slot
            // arrays, not byte blocks, so there is no value size/alignment
            self.ctx.err(sp, format!(
                "`{}` was removed — records are stored as one slot per field, not a repr-C block",
                self.ctx.name(name)
            ));
            return Err(());
        }
        match name {
            sym::ON_DROP if core_fn => {
                // on_drop(p, cleanup) (RFC 0016 §3): cleanup runs when the
                // cell's refcount reaches zero
                if args.len() != 2 {
                    self.ctx.err(sp, "on_drop(p, cleanup) takes two arguments (RFC 0016 §3)");
                    return Err(());
                }
                let pt = self.compile_expr(args[0], None)?;
                if !matches!(self.ctx.types.kind(pt), TyKind::Ptr { .. }) {
                    self.ctx.err(sp, "on_drop needs a pointer —`*T` from `&v` (RFC 0016 §3)");
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
            sym::DOWNCAST => {
                // builtin-surface phase 1: the free `downcast<T>(o)` is no
                // longer a DECLARED prelude fn (the surface spells
                // `opaque.downcast<T>(o)`) — the engine keeps lowering the
                // free spelling as a cheap alias until the phase-2 sweep
                // retires the call sites.
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
                let orecv = self.last_reg;
                return self.emit_opaque_downcast(want, orecv, sp);
            }
            sym::PANIC if core_fn => {
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
            sym::ASSERT if core_fn => {
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
            sym::STRING_JOIN if core_fn => {
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
                        self.ctx.err(sp, format!("string_join expects `Array<str>` —found `{}`", self.ctx.type_name(at)));
                        return Err(());
                    }
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_STR);
                { let (argv_off, argc) = self.pool_args(&(vec![src])); self.emit(Op::CallNat { nat: Nat::StrJoin, recv: NOREG, argv_off, argc, dst: dst }, sp.lo); }
                return Ok(TY_STR);
            }
            sym::TYPE_ID => {
                // compile-time constant —never executed (RFC 0015 §3, 0033 §3)
                if generics.len() != 1 || !args.is_empty() {
                    self.ctx.err(sp, format!("{}<T>() takes one explicit type argument", self.ctx.name(name)));
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
            sym::I8 | sym::I16 | sym::I32 | sym::I64 | sym::U8 | sym::U16 | sym::U32 | sym::U64
            | sym::F32 | sym::F64 => {
                // removed when the `as` cast landed (RFC 0007 §1) — the
                // conversion family is spelled `x as T` now. The message
                // mirrors the `size_of` removal above.
                self.ctx.err(sp, format!("`{}(x)` was removed — use `x as {}` (RFC 0007 §1)", self.ctx.name(name), self.ctx.name(name)));
                return Err(());
            }
            sym::STR => {
                if args.len() != 1 {
                    self.ctx.err(sp, "str(x) takes one argument");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                self.check_formattable(t, self.ctx.ast.span(args[0].id()))?;
                let src = self.last_reg;
                let dst = self.new_reg(TY_STR);
                { let (argv_off, argc) = self.pool_args(&(vec![src])); self.emit(Op::CallNat { nat: Nat::Str, recv: NOREG, argv_off, argc, dst: dst }, sp.lo); }
                return Ok(TY_STR);
            }
            _ => {}
        }
        // user free fn (monomorphized instantiation, RFC 0013 §2)
        if self.ctx.find_free_fn(name) {
            return self.compile_free_fn_call(name, generics, args, expected, sp);
        }
        // used function: signature from the surface, a direct call to the
        // exporter's scope-qualified id (RFC 0029 surface / RFC 0035 §1)
        if let Some(ef) = self.ctx.extern_fn(name).cloned() {
            if !generics.is_empty() {
                self.ctx.err(sp, format!("`{}` is a used fn and takes no type arguments", self.ctx.name(name)));
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
                        i + 1, self.ctx.type_name(t), self.ctx.type_name(ef.params[i])
                    ));
                }
                aregs.push(self.clone_arg(self.last_reg, ef.params[i], sp.lo));
            }
            let dst = if ef.ret == TY_NIL { None } else { Some(self.new_reg(ef.ret)) };
            { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: ef.func, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
            return Ok(ef.ret);
        }
        // builtin bytes type-call: `bytes(n)` zeroed (RFC 0004)
        if name == sym::BYTES {
            return self.compile_bytes_alloc(args, sp);
        }
        if self.ctx.find_data(name).is_some() {
            self.ctx.err(sp, format!(
                "construction is a method call, never a type-call —use a class method ({}.new(..)) or a struct literal `{} {{ .. }}` (RFC 0010 §1)",
                self.ctx.name(name), self.ctx.name(name)
            ));
            return Err(());
        }
        let msg = self
            .ctx
            .not_in_core_scope(name)
            .unwrap_or_else(|| format!("unknown function `{}`", self.ctx.name(name)));
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
                    self.ctx.err(sp, format!("bytes(n) takes an `i32` length, found `{}`", self.ctx.type_name(t)));
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

    /// The `(T, bool)` erasure-box recovery, shared by the free
    /// `downcast<T>(o)` alias and the `opaque.downcast<T>(o)` member
    /// (RFC 0014): tidof + icmp + br + guarded unbox (RFC 0032 §1.1).
    /// The box is in `orecv`; a false `.1` leaves `.0` at the type's
    /// zero value.
    pub(crate) fn emit_opaque_downcast(&mut self, want: TypeId, orecv: u16, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // v1.1: downcast yields a TUPLE `(T, bool)` — the value and
        // a success flag; no Option in the language anymore
        let tty = self.ctx.mk_tuple([want, TY_BOOL].to_vec());
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
        { let (argv_off, argc) = self.pool_args(&(vec![un, eq])); self.emit(Op::MakeRecord { dst: dst, ty: tty, argv_off, argc }, sp.lo); }
        self.jmp(l_end);
        self.bind(l_none);
        let zero = self.new_reg(want);
        self.emit(Op::ConstRaw { dst: zero, bits: 0 }, sp.lo);
        let no = self.new_reg(TY_BOOL);
        self.emit(Op::ConstRaw { dst: no, bits: 0 }, sp.lo);
        { let (argv_off, argc) = self.pool_args(&(vec![zero, no])); self.emit(Op::MakeRecord { dst: dst, ty: tty, argv_off, argc }, sp.lo); }
        self.bind(l_end);
        // the value lives in `dst`; move it out so last_reg holds it
        let out = self.new_reg(tty);
        self.emit(Op::MovRef { dst: out, src: dst }, sp.lo);
        Ok(tty)
    }

    pub(crate) fn compile_static_call(
        &mut self,
        base: IdentId,
        base_generics: Vec<NodeHandle<AnyTy>>,
        member: IdentId,
        member_generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        // core builtin statics (RFC 0028): the erasure primitive's
        // `new`/`downcast` statics and the `bytes`/`str` constructors —
        // AMBIENT now (RFC 0028 revised, builtin-surface): the arms fire
        // whenever core is mounted, no `use` required
        let core_ty = self.ctx.extern_native_types.get(&base).copied();
        // Explicit type args on a static head are meaningful only where the
        // member can use them (`Vec<u32>.from(..)` — the element type);
        // everywhere else they stay unsupported rather than silently ignored.
        let is_data = self.ctx.find_data(base).is_some();
        if !base_generics.is_empty() && !is_data {
            self.ctx.err(sp, format!(
                "generic type paths (`{}<..>.{}`) are not supported in this build",
                self.ctx.name(base), self.ctx.name(member)
            ));
            return Err(());
        }
        // a used namespace's members (`Math.sqrt`; RFC 0028) —
        // routed by the bound head, name-generic
        if self.ctx.is_extern_namespace(base) {
            return self.compile_namespace_member(base, member, &args, expected, sp);
        }
        match (base, member) {
            (sym::OPAQUE, sym::NEW) if core_ty == Some(rut_core::binary::NativeTy::Opaque) => {
                if args.len() != 1 {
                    self.ctx.err(sp, "Opaque.new(v) takes one value");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                if matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. }) {
                    self.ctx.err(sp, "`Opaque.new` rejects trait objects —they are never boxed (RFC 0014)");
                    return Err(());
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_OPAQUE);
                self.emit(Op::Box { dst, val: src, ty: t }, sp.lo);
                return Ok(TY_OPAQUE);
            }
            (sym::BYTES, sym::FROM) => {
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
                        self.ctx.err(sp, format!("bytes.from expects `Array<u8>` —found `{}`", self.ctx.type_name(at)));
                        return Err(());
                    }
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_BYTES);
                self.emit(Op::Own { dst, src, ty: TY_BYTES }, sp.lo);
                return Ok(TY_BYTES);
            }
            (sym::BYTES, sym::ZEROED) => {
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
            (sym::STR, sym::FROM_CODE) => {
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
                        self.ctx.type_name(t)
                    ));
                }
                let cp = self.last_reg;
                let c = self.new_reg(TY_CHAR);
                self.emit(Op::Conv { dst: c, src: cp, from: PrimTy::U32, to: PrimTy::Char }, sp.lo);
                let dst = self.new_reg(TY_STR);
                { let (argv_off, argc) = self.pool_args(&(vec![c])); self.emit(Op::CallNat { nat: Nat::Str, recv: NOREG, argv_off, argc, dst: dst }, sp.lo); }
                return Ok(TY_STR);
            }
            _ => {}
        }
        // the erasure primitive's member statics (RFC 0014, builtin-
        // surface phase 1): `opaque.new(v)` / `opaque.downcast<T>(o)` —
        // the phase-2 rename will make the lowercase spelling the
        // interner's own; until then BOTH base spellings resolve to the
        // one boot type (the driver binds the alias)
        if core_ty == Some(rut_core::binary::NativeTy::Opaque)
            && (base == sym::OPAQUE || self.ctx.name(base) == "opaque")
        {
            match member {
                sym::NEW => {
                    if args.len() != 1 {
                        self.ctx.err(sp, "opaque.new(v) takes one value");
                        return Err(());
                    }
                    let t = self.compile_expr(args[0], None)?;
                    if matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. }) {
                        self.ctx.err(sp, "`opaque.new` rejects trait objects —they are never boxed (RFC 0014)");
                        return Err(());
                    }
                    let src = self.last_reg;
                    let dst = self.new_reg(TY_OPAQUE);
                    self.emit(Op::Box { dst, val: src, ty: t }, sp.lo);
                    return Ok(TY_OPAQUE);
                }
                sym::DOWNCAST => {
                    if args.len() != 1 || member_generics.len() != 1 {
                        self.ctx.err(sp, "opaque.downcast<T>(o) takes one explicit type argument and one value (RFC 0014)");
                        return Err(());
                    }
                    let want = self.resolve_type_now(member_generics[0]);
                    if matches!(self.ctx.types.kind(want), TyKind::TraitObj { .. }) {
                        self.ctx.err(sp, "downcast needs a CONCRETE type —trait objects have no recovery path (RFC 0014)");
                        return Err(());
                    }
                    let t = self.compile_expr(args[0], Some(TY_OPAQUE))?;
                    if t != TY_OPAQUE {
                        self.ctx.err(sp, "downcast takes an `Opaque` box (RFC 0014)");
                        return Err(());
                    }
                    let orecv = self.last_reg;
                    return self.emit_opaque_downcast(want, orecv, sp);
                }
                _ => {
                    self.ctx.err(sp, format!(
                        "`opaque` has no static `{}` — the primitive's members are `new` and `downcast<T>` (RFC 0014)",
                        self.ctx.name(member)
                    ));
                    return Err(());
                }
            }
        }
        // enum helpers: Color.to_int(c) (RFC 0006)
        if self.ctx.name(member) == "to_int" {
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
                                "cannot infer the type arguments for `{b}` — write `{b}<..>.{m}(..)` or annotate the binding",
                                b = self.ctx.name(base), m = self.ctx.name(member)
                            ));
                            return Err(());
                        }
                    }
                } else {
                    vec![]
                };
                if !d.generics.is_empty() && class_args.len() != d.generics.len() {
                    self.ctx.err(sp, format!(
                        "`{}`<..> takes {} type argument(s), {} given",
                        self.ctx.name(base),
                        d.generics.len(),
                        class_args.len()
                    ));
                    return Err(());
                }
                let self_ty = if d.generics.is_empty() {
                    d.ty
                } else {
                    self.ctx.mk_data_inst(dname, class_args.clone(), sp)
                };
                return self.compile_direct_method(dname, class_args, self_ty, mnode, args, sp);
            }
        }        if let Some(e) = self.ctx.find_enum(base) {
            let _ = e;
            self.ctx.err(sp, format!("enum `{}` has no static `{}` in this build", self.ctx.name(base), self.ctx.name(member)));
            return Err(());
        }
        let msg = self
            .ctx
            .not_in_core_scope(base)
            .unwrap_or_else(|| format!("unknown name `{}.{}`", self.ctx.name(base), self.ctx.name(member)));
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
            if let Some(e) = expected {
                self.widen_to_slot(t, e, sp.lo);
            }
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
        // inline bounds gate the completed substitution (RFC 0043,
        // admission-only)
        self.ctx.admit_bounds(&fd.bounds, &subst, sp);
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
                    i + 1, self.ctx.type_name(arg_tys[i]), self.ctx.type_name(ptys[i])
                ));
            }
        }
        // trait-typed parameters specialize per concrete argument (RFC
        // 0012 §5): the Inst carries one origin per trait-obj param
        let mut trait_origins = Vec::new();
        for (i, _) in args.iter().enumerate() {
            if matches!(self.ctx.types.kind(ptys[i]), TyKind::TraitObj { .. })
                && !matches!(self.ctx.types.kind(arg_tys[i]), TyKind::TraitObj { .. })
            {
                trait_origins.push(arg_tys[i]);
            }
        }
        let ret_ty = ret.map(|r| self.ctx.resolve_type(r, &subst)).unwrap_or(TY_NIL);
        // a small, non-recursive body inlines at the call site (P1.3):
        // `mix64`-shaped helpers flatten into the caller, every hash pays
        // no frame. Runs after all checks, so diagnostics stay put.
        if self.try_inline_free_fn(name, fnode, &subst, &aregs, &ptys, ret_ty, sp) {
            return Ok(ret_ty);
        }
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Free(name),
            subst,
            trait_origins,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
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
        // inline bounds gate the completed substitution (RFC 0043)
        self.ctx.admit_bounds(&md.bounds, &class_subst, sp);
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
        let mut arg_tys = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
            arg_tys.push(t);
        }
        // trait-typed parameters specialize per concrete argument (RFC
        // 0012 §5): the Inst carries one origin per trait-obj param
        let mut trait_origins = Vec::new();
        for (i, _) in args.iter().enumerate() {
            if matches!(self.ctx.types.kind(ptys[i]), TyKind::TraitObj { .. })
                && !matches!(self.ctx.types.kind(arg_tys[i]), TyKind::TraitObj { .. })
            {
                trait_origins.push(arg_tys[i]);
            }
        }
        let inst = crate::check::Inst {
            key: crate::check::FnKey::Method { data: dname, name: mname },
            subst: class_subst,
            trait_origins,
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        // class method: no receiver —plain Call
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
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
                // `Vec` is an ordinary class (pouch), so it routes
                // here through `find_data`, like any other class; the
                // core statics (`Opaque`) route only
                // when used (RFC 0028)
                let is_type = matches!(base, sym::STR | sym::BYTES)
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
        // core's builtin-impl numeric methods (RFC 0032 §1.1 R2):
        // `x.wrapping_add(y)` on an integer receiver — ambient on the
        // primitive (no `use`), lowered inline off the receiver's width.
        // The receiver compiled once above; its register is reused, so a
        // side-effecting receiver still evaluates exactly once.
        if let Some(i) = self.ctx.builtin_impl(name, rt) {
            let [rhs, ..] = &args[..] else {
                self.ctx.err(sp, format!(
                    "`{}.{}` takes one argument — `x.{}(y)`",
                    self.ctx.type_name(rt),
                    self.ctx.name(name),
                    self.ctx.name(name)
                ));
                return Err(());
            };
            return self.compile_intrinsic_method(i, rt, rreg, *rhs, sp);
        }
        // primitives have no method syntax (RFC 0004/0012): `str`/`bytes`
        // operations are free functions (`string_len`, `string_encode`,
        // `bytes_len`, `bytes_decode`, `bytes_from`) — except `s.code()`,
        // the v1.1 codepoint reader that replaced `char` (RFC 0004)
        match self.ctx.types.kind(rt) {
            TyKind::Str => {
                if name == sym::CODE && args.is_empty() {
                    // s.code() -> u32 — the FIRST codepoint; traps on empty
                    let creg = self.new_reg(TY_CHAR);
                    let zero = self.new_reg(TY_I32);
                    self.emit(Op::ConstRaw { dst: zero, bits: 0 }, sp.lo);
                    self.emit(Op::StrCharAt { dst: creg, s: rreg, idx: zero }, sp.lo);
                    let dst = self.new_reg(TY_U32);
                    self.emit(Op::Conv { dst, src: creg, from: PrimTy::Char, to: PrimTy::U32 }, sp.lo);
                    return Ok(TY_U32);
                }
                if name == sym::ENCODE && args.is_empty() {
                    // s.encode() -> bytes — the UTF-8 octets (RFC 0004)
                    let dst = self.emit_string_encode(rreg, sp.lo);
                    self.last_reg = dst;
                    return Ok(TY_BYTES);
                }
                if name == sym::SLICE && args.len() == 2 {
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
                    { let (argv_off, argc) = self.pool_args(&(vec![fr, tr])); self.emit(Op::CallNat { nat: Nat::StrSlice, recv: rreg, argv_off, argc, dst: dst }, sp.lo,); }
                    return Ok(TY_STR);
                }
                if name == sym::LEN && args.is_empty() {
                    if let Some(info) = self.slice_info(rt) {
                        self.emit_slice_len(rreg, &info, sp.lo)?;
                        return Ok(TY_I32);
                    }
                }
                // a registered trait impl on the ref target dispatches
                // statically on the bare receiver (RFC 0012 §2/§5) — the
                // concrete and slot ABIs coincide for ref targets (P1.1),
                // so the raw register crosses as-is (`k.hash()` /
                // `k.hash_eq(..)` in a monomorphized map body, P4)
                if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                    return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
                }
                if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                    return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
                }
                let who = recv_name(&self.ctx, recv);
                self.ctx.err(sp, format!(
                    "`str` has no method `{}` — its members are `len`/`slice`/`code`/`encode` (`string_len({who})` is the free-fn spelling)",
                    self.ctx.name(name)
                ));
                return Err(());
            }
            TyKind::Bytes => {
                if name == sym::DECODE && args.is_empty() {
                    // b.decode() -> str — UTF-8, lossy (RFC 0004)
                    let dst = self.emit_bytes_decode(rreg, sp.lo);
                    self.last_reg = dst;
                    return Ok(TY_STR);
                }
                if name == sym::LEN && args.is_empty() {
                    if let Some(info) = self.slice_info(rt) {
                        self.emit_slice_len(rreg, &info, sp.lo)?;
                        return Ok(TY_I32);
                    }
                }
                // a registered trait impl on the ref target dispatches
                // statically on the bare receiver (same law as `str`
                // above — one ABI variant, the raw register crosses)
                if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                    return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
                }
                if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                    return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
                }
                let who = recv_name(&self.ctx, recv);
                self.ctx.err(sp, format!(
                    "`bytes` has no method `{}` — its members are `len`/`decode` (`bytes_len({who})` is the free-fn spelling)",
                    self.ctx.name(name)
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
        if name == sym::LEN && args.is_empty() {
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
        if name == sym::SLICE && args.len() == 2 {
            if let Some(info) = self.slice_info(rt) {
                match &info.source {
                    SliceSource::DataBuf { buf_field, len_field } => {
                        let arr_ty = info.array_ty(self.ctx);
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
                        { let (argv_off, argc) = self.pool_args(&(vec![fr, tr, live])); self.emit(Op::CallNat { nat: Nat::ArrSlice, recv: arr, argv_off, argc, dst: view }, sp.lo,); }
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
                        { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::ArrLen, recv: rreg, argv_off, argc, dst: live }, sp.lo); }
                        let view = self.new_reg(rt);
                        { let (argv_off, argc) = self.pool_args(&(vec![fr, tr, live])); self.emit(Op::CallNat { nat: Nat::ArrSlice, recv: rreg, argv_off, argc, dst: view }, sp.lo,); }
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
        // unless the module registered an inherent impl for it
        // (RFC 0012 §2 — a module-owned `builtin class` takes impls)
        match self.ctx.types.kind(rt).clone() {
            TyKind::Opaque => {
                if let Some((idx, mname)) = self.ctx.impls.iter().enumerate().find_map(|(idx, im)| {
                    if !im.inherent || im.target != TY_OPAQUE {
                        return None;
                    }
                    im.methods.iter().find(|(n, _)| *n == name).map(|(n, _)| (idx, *n))
                }) {
                    return self.compile_native_static_call(idx, mname, vec![], rreg, args, expected, sp);
                }
                self.ctx.err(sp, "`Opaque` has no methods in this build —recover with `downcast<T>(o)` (RFC 0014)");
                return Err(());
            }
            _ => {}
        }
        // user types: inherent methods first (direct), then trait impls
        // — statically bound for this concrete receiver (nominal, RFC
        // 0012 §5: an impl is registered for exactly this (trait, type))
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
            if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                // a bare concrete receiver calls the CONCRETE-ABI variant:
                // the receiver register stays raw and the args cross
                // concretely — no box is minted (P1.2); a tiny body inlines
                // at the site (P1.3)
                return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
            }
            // another module's registration (RFC 0012 §2/§5): static
            // dispatch through the exporter's compiled fn — the gate
            // (the trait's name was used here) is part of the lookup
            if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
            }
            self.no_method_error(rt, name, sp);
            return Err(());
        }
        // primitives take trait impls only (RFC 0012 §2; the inherent
        // surface is core's `builtin impl`, RFC 0032 §1.1): a bare
        // concrete receiver dispatches the impl's CONCRETE-ABI variant —
        // the scalar crosses as an ordinary argument, no slot box (P1.2)
        if matches!(self.ctx.types.kind(rt), TyKind::Prim(_)) {
            if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
            }
            if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
            }
            self.no_method_error(rt, name, sp);
            return Err(());
        }
        // the array type's inherent impl (`impl [T] { .. }`, RFC 0012 §2)
        // — static dispatch through the native shape
        if let TyKind::Array { elem } = self.ctx.types.kind(rt).clone() {
            let hit = self.ctx.impls.iter().enumerate().find_map(|(idx, im)| {
                if !im.inherent {
                    return None;
                }
                match &im.target_data {
                    Some((d, params)) if d == &sym::ARRAY && params.len() == 1 => {
                        im.methods.iter().find(|(n, _)| *n == name).map(|(n, _)| (idx, *n, params[0]))
                    }
                    _ => None,
                }
            });
            if let Some((idx, mname, param)) = hit {
                return self.compile_native_static_call(idx, mname, vec![(param, elem)], rreg, args, expected, sp);
            }
        }
        if let TyKind::TraitObj { trait_id } = self.ctx.types.kind(rt).clone() {
            // trait-typed receiver: ONLY that trait's methods (RFC 0012 §2).
            // Single concrete origin ⇒ static bind; a merged/loaded/unknown
            // origin consults the value's descriptor (vtable)
            let tdesc = self.ctx.trait_by_id(trait_id).clone();
            if let Some(midx) = tdesc.methods.iter().position(|m| m.name == name) {
                if let ExprKind::Path { segs } = self.ctx.ast.expr(recv).clone() {
                    if segs.len() == 1 {
                        let origins = self.origins_of(segs[0].name);
                        if origins.len() == 1 {
                            // the impl may live in any module (RFC 0012 §2):
                            // a local block binds here, another module's
                            // registration binds to its compiled fn
                            match self.ctx.find_impl_ex(trait_id, origins[0]) {
                                Some(ImplHit::Local(idx)) => {
                                    // origin-pinned trait-object receiver: the box
                                    // is already materialized — call the SLOT
                                    // variant (no new boxes, RFC 0012 §5)
                                    return self.compile_trait_static_call(idx, midx, origins[0], rreg, args, expected, sp, true);
                                }
                                Some(ImplHit::Extern(eidx)) => {
                                    return self.compile_extern_trait_static_call(eidx, midx, origins[0], rreg, args, expected, sp, true);
                                }
                                None => {}
                            }
                        }
                    }
                }
                let slot = self.ctx.trait_slot(trait_id, midx as u32).unwrap();
                return self.finish_trait_call(slot, tdesc.methods[midx].params.clone(), tdesc.methods[midx].ret, rreg, args, expected, sp);
            }
            self.ctx.err(sp, format!(
                "`{}` values reach only `{}`'s methods —`{}` is not one of them (RFC 0012 §2)",
                self.ctx.name(tdesc.name), self.ctx.name(tdesc.name), self.ctx.name(name)
            ));
            return Err(());
        }
        self.ctx.err(sp, format!("`{}` has no method `{}` in this build", self.ctx.type_name(rt), self.ctx.name(name)));
        Err(())
    }

    /// The diagnostic for a missing method on a user type: when a
    /// matching impl exists under a trait this call site cannot name,
    /// the message says which `use` unlocks it (RFC 0012 §6, the
    /// use-both gate). Both local impls and other modules'
    /// registrations (RFC 0012 §2) count as "exists".
    fn no_method_error(&mut self, rt: TypeId, name: IdentId, sp: rut_lexer::span::Span) {
        let target_matches = |ctx: &Ctx, im: &crate::check::ImplDecl| {
            im.target == rt
                || matches!(&im.target_data, Some((d, _)) if ctx
                    .inst_data
                    .get(&rt)
                    .map_or(false, |(rd, _)| rd == d))
        };
        let hit = self.ctx.impls.iter().find(|im| {
            !im.inherent
                && im.methods.iter().any(|(n, _)| *n == name)
                && target_matches(self.ctx, im)
        });
        let ext_hit = hit.is_none().then(|| {
            self.ctx
                .extern_impls
                .iter()
                .position(|im| im.target == rt && im.methods.iter().any(|(n, _)| *n == name))
        }).flatten();
        let trait_name = hit.map(|im| im.trait_name).or_else(|| {
            ext_hit.map(|e| self.ctx.extern_impls[e].trait_name)
        });
        if let Some(tname) = trait_name {
            let callable = self.ctx.find_trait(tname).is_some()
                || self.ctx.extern_traits.contains_key(&tname)
                || self.ctx.extern_trait_decls.contains_key(&tname);
            if !callable {
                let tid = hit.map(|im| im.trait_id).unwrap_or_else(|| {
                    self.ctx.extern_impls[ext_hit.unwrap()].trait_id
                });
                let spelled = self.ctx.name(self.ctx.trait_by_id(tid).name);
                self.ctx.err(sp, format!(
                    "`{}` has no method `{}` — use `{}` to call its methods on `{}` (RFC 0012 §6)",
                    self.ctx.type_name(rt),
                    self.ctx.name(name),
                    spelled,
                    self.ctx.type_name(rt)
                ));
                return;
            }
        }
        self.ctx.err(sp, format!("`{}` has no method `{}`", self.ctx.type_name(rt), self.ctx.name(name)));
    }

    /// The registered trait impl on `rt` whose method set contains
    /// `name` — concrete targets by id, generic targets by declaration.
    fn find_trait_impl_method(&self, rt: TypeId, name: IdentId) -> Option<(usize, usize)> {
        for (idx, im) in self.ctx.impls.iter().enumerate() {
            if im.inherent || !im.methods.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let target_matches = im.target == rt
                || matches!(&im.target_data, Some((d, _)) if self
                    .ctx
                    .inst_data
                    .get(&rt)
                    .map_or(false, |(rd, _)| rd == d));
            if !target_matches {
                continue;
            }
            let tdesc = self.ctx.trait_by_id(im.trait_id);
            if let Some(midx) = tdesc.methods.iter().position(|m| m.name == name) {
                return Some((idx, midx));
            }
        }
        None
    }

    /// Static dispatch of a trait method through a registered impl: the
    /// receiver's concrete type names the impl, so the call binds to the
    /// impl method directly (`CallM`) — no vtable hop (RFC 0012 §5).
    /// Arguments type against the trait's declared signature (the impl's
    /// was checked to match at collection).
    ///
    /// The `slot_abi` switch picks the callee variant (P1.1/P1.2): a
    /// trait-object receiver (box already materialized) calls the SLOT
    /// variant — the trait's declared signature crosses, scalars arrive
    /// boxed and the prologue unboxes; a BARE concrete receiver calls the
    /// CONCRETE variant — the receiver stays raw, `Self`-spelled params
    /// cross as the concrete type, no box is minted. At concrete sites a
    /// small body inlines at the call site (P1.3).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_trait_static_call(
        &mut self,
        impl_idx: usize,
        midx: usize,
        concrete: TypeId,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
        slot_abi: bool,
    ) -> TcResult<TypeId> {
        let im = self.ctx.impls[impl_idx].clone();
        let tdesc = self.ctx.trait_by_id(im.trait_id).clone();
        let tm = tdesc.methods[midx].clone();
        // generic target: this instantiation's substitution
        let subst: Vec<(IdentId, TypeId)> = match &im.target_data {
            Some((_, params)) => match self.ctx.inst_data.get(&concrete).cloned() {
                Some((_, cargs)) => params.iter().cloned().zip(cargs.into_iter()).collect(),
                None => vec![],
            },
            None => vec![],
        };
        // inline bounds gate the completed substitution (RFC 0043):
        // the impl method's own bounds under the target substitution
        if let Some((_, mnode)) = im.methods.iter().find(|(n, _)| *n == tm.name) {
            let bounds = self.ctx.ast.method_decl(*mnode).bounds.clone();
            self.ctx.admit_bounds(&bounds, &subst, sp);
        }
        // the impl method's own AST (present for local impls)
        let mnode = im.methods.iter().find(|(n, _)| *n == tm.name).map(|(_, n)| *n);
        // param/ret types for the chosen ABI: the slot ABI crosses the
        // trait's declared signature; the concrete ABI crosses the impl
        // method's own signature resolved under the target substitution
        // (`Self` spells the concrete target)
        let (ptys, ret_ty) = if slot_abi {
            (tm.params.clone(), tm.ret)
        } else {
            match mnode {
                Some(mn) => {
                    let md = self.ctx.ast.method_decl(mn).clone();
                    let mut ps = Vec::new();
                    for p in md.params.iter().skip(1) {
                        match self.ctx.ast.param(*p) {
                            MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                                ps.push(self.ctx.resolve_sig_ty(*t, &subst, Some(concrete)))
                            }
                            _ => ps.push(TY_I32),
                        }
                    }
                    let ret = md.ret.map(|r| self.ctx.resolve_sig_ty(r, &subst, Some(concrete))).unwrap_or(TY_NIL);
                    (ps, ret)
                }
                None => (tm.params.clone(), tm.ret),
            }
        };
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            // a `Self`-typed trait parameter accepts the concrete
            // receiver (nominal widening, RFC 0012 §4); under the
            // concrete ABI the types already match, so the check is
            // exact and NO box is emitted
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            if slot_abi {
                self.widen_to_slot(t, ptys[i], sp.lo);
            }
            aregs.push(self.last_reg);
        }
        // tiny concrete-ABI bodies inline at the site; box sites fall
        // back to the slot-variant CallM (P1.3)
        if !slot_abi {
            if let Some(mn) = mnode {
                // the enclosing class for `Self`-ish statics in the body
                let cname = self
                    .ctx
                    .inst_data
                    .get(&concrete)
                    .map(|(d, _)| *d)
                    .or_else(|| self.ctx.datas.iter().find(|(_, d)| d.ty == concrete).map(|(n, _)| *n));
                if self.try_inline_impl_call(
                    impl_idx, mn, tm.name, concrete, &subst, cname, rreg, &aregs, &ptys, ret_ty, sp,
                ) {
                    return Ok(ret_ty);
                }
            }
        }
        let key = self.ctx.impl_method_key(impl_idx, tm.name, slot_abi);
        let inst = crate::check::Inst {
            key,
            subst,
            trait_origins: vec![],
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

    /// Another module's registration of the `(trait, type)` impl
    /// (RFC 0012 §2/§5): the method is already compiled in the exporter,
    /// so the call binds to its scope-qualified fn id directly (`CallM`)
    /// — link rebases it. The signature comes from the trait's
    /// descriptor as registered from the surface.
    ///
    /// `slot_abi` picks the bound variant (P1.2): the slot id for a
    /// trait-object receiver, the concrete id for a bare receiver
    /// (`Self`-spelled params cross as the concrete target — the trait
    /// descriptor spells them as trait objects, so the concrete ABI maps
    /// this trait's trait-object params onto `concrete`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_extern_trait_static_call(
        &mut self,
        ext_idx: usize,
        midx: usize,
        concrete: TypeId,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
        slot_abi: bool,
    ) -> TcResult<TypeId> {
        let (fid, tm, trait_id) = {
            let im = &self.ctx.extern_impls[ext_idx];
            let tdesc = self.ctx.trait_by_id(im.trait_id);
            let tm = tdesc.methods[midx].clone();
            let list = if slot_abi {
                None
            } else {
                im.methods_concrete.iter().find(|(n, _)| *n == tm.name)
            };
            match list {
                Some(&(_, f)) => (f, tm, im.trait_id),
                // no concrete twin registered (single-ABI exporter):
                // the slot variant IS the concrete one there
                None => match im.methods.iter().find(|(n, _)| *n == tm.name) {
                    Some(&(_, f)) => (f, tm, im.trait_id),
                    None => {
                        let t = self.ctx.name(tdesc.name);
                        self.ctx.err(sp, format!("impl `{t}` is missing `{}`", self.ctx.name(tm.name)));
                        return Err(());
                    }
                },
            }
        };
        // param types for the chosen ABI
        let ptys: Vec<TypeId> = if slot_abi {
            tm.params.clone()
        } else {
            tm.params
                .iter()
                .map(|&p| match self.ctx.types.kind(p) {
                    TyKind::TraitObj { trait_id: t } if *t == trait_id => concrete,
                    _ => p,
                })
                .collect()
        };
        let ret_ty = tm.ret;
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
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            if slot_abi {
                self.widen_to_slot(t, ptys[i], sp.lo);
            }
            aregs.push(self.last_reg);
        }
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

    /// The extern registration of `(trait, type)` whose method set
    /// contains `name` — `(extern impl index, trait method index)`. The
    /// use-both gate lives here: an impl whose trait's name was never
    /// used does not dispatch (RFC 0012 §6); `no_method_error` still
    /// sees it for the "use `I` .." diagnostic.
    fn find_extern_trait_impl_method(&self, rt: TypeId, name: IdentId) -> Option<(usize, usize)> {
        for (eidx, im) in self.ctx.extern_impls.iter().enumerate() {
            if !self.ctx.extern_trait_decls.contains_key(&im.trait_name) {
                continue;
            }
            if im.target != rt || !im.methods.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let tdesc = self.ctx.trait_by_id(im.trait_id);
            if let Some(midx) = tdesc.methods.iter().position(|m| m.name == name) {
                return Some((eidx, midx));
            }
        }
        None
    }

    /// Static dispatch of a native builtin class's inherent method
    /// (`impl Array<T> { .. }`): the native shape supplies the target
    /// substitution; `params` is the `target_data` binding.
    pub(crate) fn compile_native_static_call(
        &mut self,
        impl_idx: usize,
        mname: IdentId,
        subst: Vec<(IdentId, TypeId)>,
        rreg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let im = self.ctx.impls[impl_idx].clone();
        let (_, mnode) = im.methods.iter().find(|(n, _)| *n == mname).cloned().ok_or(())?;
        let md = self.ctx.ast.method_decl(mnode).clone();
        let saved_subst = std::mem::replace(&mut self.subst, subst.clone());
        let saved_self = self.self_ty;
        self.self_ty = None;
        let mut ptys = Vec::new();
        for p in md.params.iter().skip(1) {
            match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => ptys.push(self.resolve_type_now(*t)),
                _ => ptys.push(TY_I32),
            }
        }
        let ret_ty = md.ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_NIL);
        self.subst = saved_subst;
        self.self_ty = saved_self;
        if args.len() != ptys.len() {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ptys.len()));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if t != ptys[i] {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.clone_arg(self.last_reg, ptys[i], sp.lo));
        }
        let inst = crate::check::Inst {
            key: self.ctx.impl_method_key(impl_idx, mname, false),
            subst,
            trait_origins: vec![],
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

    /// mut-binding law (RFC 0003 §1): writing through a handle requires the
    /// head binding to be `let mut`
    /// RFC 0012 §4: implicit widening — exact > trait-typed when an impl
    /// is REGISTERED for the (trait, type) pair. Nominal: no structural
    /// shape is ever consulted. Same-type always widens.
    /// RFC 0043 §A5: a value of the enclosing class's generic parameter
    /// also widens through its recorded `requires` bound — the registry
    /// hit normally answers first (admission proved the impl at the
    /// instantiation), so this only carries a body whose impl is not
    /// (yet) registered.
    pub(crate) fn widens(&mut self, from: TypeId, to: TypeId) -> bool {
        if from == to {
            return true;
        }
        if let TyKind::TraitObj { trait_id } = self.ctx.types.kind(to).clone() {
            if self.ctx.find_impl_ex(trait_id, from).is_some() {
                return true;
            }
            let Some(dname) = self.current_class else {
                return false;
            };
            let Some(d) = self.ctx.find_data(dname).cloned() else {
                return false;
            };
            for (g, bnode) in &d.requires {
                let Some(&conc) = self.subst.iter().find(|(n, _)| n == g).map(|(_, t)| t) else {
                    continue;
                };
                if conc != from {
                    continue;
                }
                let members = self.ctx.resolve_bound_members(*bnode, &self.subst);
                if members
                    .iter()
                    .any(|m| matches!(m, crate::check::BoundMember::Trait(tid) if *tid == trait_id))
                {
                    return true;
                }
            }
            return false;
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
        // inline bounds gate the completed substitution (RFC 0043)
        self.ctx.admit_bounds(&md.bounds, &class_subst, sp);
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
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            self.widen_to_slot(t, ptys[i], sp.lo);
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
            trait_origins: Vec::new(),
        };
        let fid = self.ctx.ensure_inst(inst);
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallM { func: fid, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
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
        if md.is_async {
            return false; // `async` is diagnosed when the body is compiled
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
        // bind `self` (well-known symbol) for the inlined body
        self.locals.push(Local { name: sym::SELF, reg: recv, ty: self_ty, is_mut: mut_self, loop_var: false, origins: Vec::new() });
        self.inline_self = Some((sym::SELF, recv));
        let params: Vec<NodeHandle<AnyParam>> = md.params.clone();
        let mut ai = 0usize;
        for p in &params {
            if let MemberKind::Param(ParamData { name, is_mut, .. }) = self.ctx.ast.param(*p) {
                self.locals.push(Local { name: *name, reg: aregs[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false, origins: Vec::new() });
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

    /// Inline a small, non-recursive free-fn body at the call site.
    /// Returns `true` when it compiled the body; `false` to emit `Call`.
    /// Params bind to the argument registers directly — `Op::Call` copies
    /// slots without deep-copying value args, so the inline keeps the
    /// exact aliasing behavior of the call it replaces. Trait-obj params
    /// bind without origins (vtable dispatch inside the body —
    /// conservative, like [`Self::try_inline_method`]'s params).
    #[allow(clippy::too_many_arguments)]
    fn try_inline_free_fn(
        &mut self,
        name: IdentId,
        fnode: NodeHandle<FnNode>,
        subst: &[(IdentId, TypeId)],
        aregs: &[u16],
        ptys: &[TypeId],
        ret_ty: TypeId,
        sp: rut_lexer::span::Span,
    ) -> bool {
        const MAX_STMTS: usize = 24;
        const MAX_DEPTH: usize = 4;
        // the inline stack keys (owner, method) — free fns have no owner
        if self.inline_stack.len() >= MAX_DEPTH || self.inline_stack.contains(&(name, name)) {
            return false;
        }
        // a trait-obj parameter loses its per-call origin specialization
        // in an inline (RFC 0012 §5: one clone per concrete argument,
        // each binding statically) — that boundary can't splice, so the
        // call stays a call
        if ptys.iter().any(|&t| matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. })) {
            return false;
        }
        let fd = self.ctx.ast.fn_decl(fnode).clone();
        if fd.is_async {
            return false; // `async` is diagnosed when the body is compiled
        }
        let body = fd.body;
        let stmts = match self.ctx.ast.kind(body.id()) {
            Kind::Expr(ExprKind::Block { stmts }) => stmts.clone(),
            _ => return false,
        };
        if stmts.len() > MAX_STMTS {
            return false;
        }
        let saved_self_ty = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, subst.to_vec());
        let saved_class = self.current_class;
        let saved_ret = self.ret_ty;
        let saved_inline_ret = self.inline_ret;
        let saved_inline_self = self.inline_self;
        self.self_ty = None;
        self.current_class = None;
        self.ret_ty = ret_ty;
        let base = self.locals.len();
        let params: Vec<NodeHandle<AnyParam>> = fd.params.clone();
        let mut ai = 0usize;
        for p in &params {
            if let MemberKind::Param(ParamData { name: pname, is_mut, .. }) = self.ctx.ast.param(*p) {
                self.locals.push(Local { name: *pname, reg: aregs[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false, origins: Vec::new() });
                ai += 1;
            }
        }
        let res = self.new_reg(ret_ty);
        let l_end = self.new_label();
        self.inline_ret = Some((res, l_end));
        self.inline_stack.push((name, name));
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

    /// Inline a small, non-recursive trait-impl method body at a
    /// bare-receiver static call site (P1.3). Mirrors
    /// [`Self::try_inline_method`]: `self` (and the `Self`-spelled
    /// params) bind to the concrete registers the call carries — the
    /// receiver register is already bare concrete at these sites.
    /// Returns `true` when it compiled the body; `false` to emit the
    /// concrete-variant `CallM`.
    #[allow(clippy::too_many_arguments)]
    fn try_inline_impl_call(
        &mut self,
        impl_idx: usize,
        mnode: NodeHandle<MethodDeclNode>,
        mname: IdentId,
        self_ty: TypeId,
        subst: &[(IdentId, TypeId)],
        current_class: Option<IdentId>,
        recv: u16,
        aregs: &[u16],
        ptys: &[TypeId],
        ret_ty: TypeId,
        sp: rut_lexer::span::Span,
    ) -> bool {
        const MAX_STMTS: usize = 24;
        const MAX_DEPTH: usize = 4;
        // the inline stack keys (owner, method) — the trait names the
        // impl body (one impl per (trait, type) pair; two impls of one
        // trait share the key and just decline the second inline)
        let trait_name = self.ctx.impls[impl_idx].trait_name;
        if self.inline_stack.len() >= MAX_DEPTH || self.inline_stack.contains(&(trait_name, mname)) {
            return false;
        }
        // a trait-obj parameter loses its per-call origin specialization
        // in an inline (RFC 0012 §5) — that boundary can't splice
        if ptys.iter().any(|&t| matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. })) {
            return false;
        }
        let md = self.ctx.ast.method_decl(mnode).clone();
        if md.is_async {
            return false; // `async` is diagnosed when the body is compiled
        }
        let Some(body) = md.body else { return false };
        let stmts = match self.ctx.ast.kind(body.id()) {
            Kind::Expr(ExprKind::Block { stmts }) => stmts.clone(),
            _ => return false,
        };
        if stmts.len() > MAX_STMTS {
            return false;
        }
        let mut_self = matches!(
            md.params.first().map(|p| self.ctx.ast.param(*p)),
            Some(MemberKind::SelfParam(SelfParamData { is_mut: true }))
        );
        let saved_self_ty = self.self_ty;
        let saved_subst = std::mem::replace(&mut self.subst, subst.to_vec());
        let saved_class = self.current_class;
        let saved_ret = self.ret_ty;
        let saved_inline_ret = self.inline_ret;
        let saved_inline_self = self.inline_self;
        self.self_ty = Some(self_ty);
        self.current_class = current_class;
        self.ret_ty = ret_ty;
        let base = self.locals.len();
        // bind `self` (well-known symbol) for the inlined body — reads
        // alias the receiver register (no copy, RFC 0005 accessor rule)
        self.locals.push(Local { name: sym::SELF, reg: recv, ty: self_ty, is_mut: mut_self, loop_var: false, origins: Vec::new() });
        self.inline_self = Some((sym::SELF, recv));
        let params: Vec<NodeHandle<AnyParam>> = md.params.clone();
        let mut ai = 0usize;
        for p in &params {
            if let MemberKind::Param(ParamData { name: pname, is_mut, .. }) = self.ctx.ast.param(*p) {
                self.locals.push(Local { name: *pname, reg: aregs[ai], ty: ptys[ai], is_mut: *is_mut, loop_var: false, origins: Vec::new() });
                ai += 1;
            }
        }
        let res = self.new_reg(ret_ty);
        let l_end = self.new_label();
        self.inline_ret = Some((res, l_end));
        self.inline_stack.push((trait_name, mname));
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
            if !self.widens(t, param_tys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(param_tys[i])
                ));
            }
            self.widen_to_slot(t, param_tys[i], sp.lo);
            aregs.push(self.last_reg);
        }
        let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
        // the vtable form is final — origins multiple, the call consults
        // the descriptor (RFC 0012 §1, the two-rule dispatch law)
        { let (argv_off, argc) = self.pool_recv_args(rreg, &(aregs)); self.emit(Op::CallI { slot: slot, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret_ty)
    }

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
            TyKind::Ptr { elem } if matches!(self.ctx.types.kind(elem), TyKind::Data { .. }) => {
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
