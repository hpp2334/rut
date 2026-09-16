//! Expression compilation: the value-in-last_reg convention, literals with
//! bidirectional inference (RFC 0007 SS1), path/field-chain reads,
//! enum members, and module-let loads.

use crate::check::{float_suffix_ty, int_suffix_ty, numeric_prim, TcResult};
use rut_core::binary::ConstVal;
use rut_core::ops::*;
use rut_core::types::*;
use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {
    // ---- expressions ----

    /// Compile an expression; the value lands in a fresh register (the
    /// last register). `expected` flows down for bidirectional literal
    /// inference (RFC 0007 §1).
    pub(crate) fn compile_expr(&mut self, node: NodeHandle<AnyExpr>, expected: Option<TypeId>) -> TcResult<TypeId> {
        if !self.enter() {
            self.leave();
            return Err(());
        }
        let mut r = self.compile_expr_inner(node, expected);
        self.leave();
        // pointer deref at value positions (RFC 0012 §6): a `*T` where
        // `T` is expected reads its pointee — arguments, returns, lets,
        // assignments. Positions without an expected type (receivers,
        // `==` operands, `let w = p`) keep the pointer: identity, aliasing
        // and field/method deref are pointer-native.
        if let Ok(t) = r {
            if let Some(e) = expected {
                if e != t {
                    if let TyKind::Ptr { elem } = self.ctx.types.kind(t).clone() {
                        if elem == e {
                            let lo = self.ctx.ast.span(node.id()).lo;
                            let src = self.last_reg;
                            let d = self.new_reg(elem);
                            self.emit(Op::GetF { dst: d, obj: src, field: 0, repr: self.ctx.types.repr_of(elem) }, lo);
                            r = Ok(elem);
                        }
                    }
                }
            }
        }
        // copy-by-value boundary (RFC 0009/0016 v1.1): every VALUE-typed
        // expression result is a fresh cell the consumer owns — records and
        // arrays deep-copy here, once, at the expression boundary. Receivers
        // and assignment-target chains bypass this wrapper and alias.
        r
    }

    pub(crate) fn compile_expr_inner(&mut self, node: NodeHandle<AnyExpr>, expected: Option<TypeId>) -> TcResult<TypeId> {
        let sp = self.ctx.ast.span(node.id());
        self.span = sp.lo;
        match self.ctx.ast.expr(node).clone() {
            ExprKind::Lit(lit) => {
                let (ty, reg) = self.load_lit(lit, expected, sp)?;
                let _ = reg;
                Ok(ty)
            }
            ExprKind::Path { segs } => self.compile_path(node, segs, expected, sp),
            ExprKind::Call { callee, args } => self.compile_call(node, callee, args, expected, sp),
            ExprKind::Method { recv, name, generics, args } => {
                self.compile_method(recv, name, generics, args, expected, sp)
            }
            ExprKind::Field { recv, name } => self.compile_field(recv, name, sp),
            ExprKind::Tuple { elems } => {
                // `(a, b, ..)` (RFC 0007): a record with numeric fields.
                // `()` is the unit value.
                if elems.is_empty() {
                    let reg = self.new_reg(TY_UNIT);
                    self.emit(Op::ConstRaw { dst: reg, bits: 0 }, sp.lo);
                    return Ok(TY_UNIT);
                }
                let mut etys = Vec::new();
                let mut vals = Vec::new();
                for e in &elems {
                    let t = self.compile_expr(*e, None)?;
                    etys.push(t);
                    vals.push(self.last_reg);
                }
                let ty = self.ctx.mk_tuple(etys);
                let dst = self.new_reg(ty);
                self.emit(Op::MakeRecord { dst, ty, vals }, sp.lo);
                Ok(ty)
            }
            ExprKind::Index { recv, idx } => {
                let rt = self.compile_expr(recv, None)?;
                let rreg = self.last_reg;
                // `p[i]` auto-derefs (RFC 0005)
                let (rt, rreg) = self.deref_for_use(rt, rreg, sp.lo);
                let it = self.compile_expr(idx, Some(TY_I32))?;
                if it != TY_I32 {
                    self.ctx.err(sp, format!("index must be `i32`, found `{}`", self.ctx.types.name(it)));
                }
                let ireg = self.last_reg;
                if let Some(info) = self.slice_info(rt) {
                    self.emit_slice_get(rreg, ireg, &info, sp.lo)?;
                    return Ok(info.elem);
                }
                self.ctx.err(sp, format!("indexing needs a sequence (Vec, Array, or bytes) —found `{}`", self.ctx.types.name(rt)));
                Err(())
            }
            ExprKind::Unary { op, expr } => {
                use rut_ast::ast::UnOp::*;
                let t = match op {
                    Deref => {
                        // `*p` (RFC 0005): load the pointee — a copy out of
                        // the box (the box's payload slot, repr = the
                        // pointee's own repr)
                        let pt = self.compile_expr(expr, None)?;
                        let src = self.last_reg;
                        let TyKind::Ptr { elem } = self.ctx.types.kind(pt).clone() else {
                            self.ctx.err(sp, "`*` dereferences a pointer —the operand is not `*T`");
                            return Err(());
                        };
                        let dst = self.new_reg(elem);
                        self.emit(
                            Op::GetF { dst, obj: src, field: 0, repr: self.ctx.types.repr_of(elem) },
                            sp.lo,
                        );
                        return Ok(elem);
                    }
                    Not => self.compile_expr(expr, Some(TY_BOOL))?,
                    // `-lit` in a typed position still adapts the literal
                    // (`-2.0` passed to an `f64` param), so forward `expected`
                    _ => self.compile_expr(expr, expected)?,
                };
                // capture the operand's register BEFORE new_reg — it
                // overwrites last_reg with the destination
                let src = self.last_reg;
                let dst = self.new_reg(t);
                match op {
                    Neg => {
                        let prim = if let TyKind::Prim(p) = self.ctx.types.kind(t) { Some(*p) } else { None };
                        let Some(prim) = prim else {
                            self.ctx.err(sp, "negation needs a number");
                            return Err(());
                        };
                        self.emit(negop(prim, dst, src), sp.lo);
                    }
                    Not => {
                        if t != TY_BOOL {
                            self.ctx.err(sp, "`!` needs a bool");
                        }
                        self.emit(Op::Not { dst, a: src }, sp.lo);
                    }
                    BitNot => {
                        self.ctx.err(sp, "`~` is not supported in this build");
                        return Err(());
                    }
                    // `*p` returned early — the pointee load is the value
                    Deref => {}
                }
                Ok(t)
            }
            ExprKind::Binary { op, lhs, rhs } => self.compile_binary(node, op, lhs, rhs, expected, sp),
            ExprKind::Assign { op, target, value } => {
                self.compile_assign(node, op, target, value, sp)?;
                Ok(TY_UNIT)
            }
            ExprKind::Lambda { params, ret, body } => {
                self.compile_lambda(node.id(), params, ret, body, expected, sp)
            }
            ExprKind::Try { .. } => {
                // `?` was Result-syntax: removed with the sums (RFC 0005 §10)
                self.ctx.err(sp, "`?` was removed — errors are `(T, err)` records; test the second element (RFC 0005 §10)");
                Err(())
            }
            ExprKind::Await { .. } | ExprKind::Select { .. } => {
                self.ctx.err(sp, "coroutines are not supported in this build (RFC 0018—020, M3)");
                Err(())
            }
            ExprKind::FStr { parts } => self.compile_fstr(parts, expected, sp),
            ExprKind::Struct { ty, fields } => self.compile_struct(ty, fields, expected, sp),
            ExprKind::ArrayLit { elems } => {
                let arr_ty = self.compile_array_lit(elems, expected, sp)?;
                Ok(arr_ty)
            }
            ExprKind::WhenExpr { scrut, arms } => self.compile_when(scrut, &arms, expected, sp),
            ExprKind::Is { expr, ty } => {
                let rt = self.compile_expr(expr, None)?;
                let recv = self.last_reg;
                let dst = self.new_reg(TY_BOOL);
                // resolve the RHS (naming position, RFC 0012 §3)
                if let TypeKind::TyPath { segs, is_dyn } = self.ctx.ast.ty(ty) {
                    if segs.len() == 1 && segs[0].generics.is_empty() {
                        let n = self.ctx.name(segs[0].name).to_string();
                        if *is_dyn {
                            self.ctx.err(sp, "the `is` RHS must not be `dyn`-prefixed (RFC 0012 §3)");
                        }
                        // trait RHS —capability probe (or fold)
                        if let Some(tid) = self.ctx.trait_id_of(segs[0].name) {
                            match self.ctx.types.kind(rt).clone() {
                                TyKind::TraitObj { trait_id } if trait_id == tid => {
                                    self.emit(Op::ConstRaw { dst, bits: 1 }, sp.lo); // fold
                                }
                                TyKind::TraitObj { .. } => {
                                    self.emit(Op::IsTrait { dst, obj: recv, want: tid }, sp.lo);
                                }
                                TyKind::Opaque => {
                                    self.emit(Op::IsTrait { dst, obj: recv, want: tid }, sp.lo);
                                }
                                exact => {
                                    let _ = exact;
                                    // exact receiver: fold by duck-typed
                                    // satisfaction (RFC 0012 v1.1)
                                    let has = self.ctx.duck_satisfies(rt, tid);
                                    self.emit(Op::ConstRaw { dst, bits: has as u64 }, sp.lo);
                                }
                            }
                            return Ok(TY_BOOL);
                        }
                        // Opaque RHS — import-gated like the type (RFC 0028)
                        if n == "Opaque"
                            && self.ctx.extern_native_types.get(&segs[0].name).copied()
                                == Some(rut_core::binary::NativeTy::Opaque)
                        {
                            match self.ctx.types.kind(rt).clone() {
                                TyKind::Opaque => {
                                    self.emit(Op::ConstRaw { dst, bits: 1 }, sp.lo);
                                }
                                _ => {
                                    self.emit(Op::ConstRaw { dst, bits: 0 }, sp.lo);
                                }
                            }
                            return Ok(TY_BOOL);
                        }
                        // concrete RHS
                        let want = self.resolve_type_now(ty);
                        if !self.ctx.types.is_ref(want) && want != TY_STR {
                            self.ctx.err(sp, format!(
                                "`is` needs a concrete (cell) or trait type —`{}` is by-value",
                                self.ctx.types.name(want)
                            ));
                        }
                        match self.ctx.types.kind(rt).clone() {
                            TyKind::Opaque => {
                                self.emit(Op::IsType { dst, obj: recv, want }, sp.lo);
                            }
                            _ => {
                                // exact receiver: fold (RFC 0012 §3)
                                let same = rt == want;
                                self.emit(Op::ConstRaw { dst, bits: same as u64 }, sp.lo);
                            }
                        }
                        return Ok(TY_BOOL);
                    }
                }
                self.ctx.err(sp, "the `is` right-hand side must be a type name");
                Err(())
            }
            ExprKind::Cast { expr, ty } => {
                // `expr as T` (RFC 0007 §1): the numeric cast — truncating,
                // C/Rust semantics, one `Op::Conv`. The RHS is a naming
                // position restricted to the numeric primitives.
                let from = self.compile_expr(expr, None)?;
                let src = self.last_reg;
                let want = self.resolve_type_now(ty);
                let Some(to) = (if let TyKind::Prim(p) = self.ctx.types.kind(want) {
                    numeric_prim(*p).then_some(*p)
                } else {
                    None
                }) else {
                    self.ctx.err(sp, format!(
                        "`as` converts between numeric primitives —`{}` is not one (RFC 0007 §1)",
                        self.ctx.types.name(want)
                    ));
                    return Err(());
                };
                let TyKind::Prim(from_prim) = self.ctx.types.kind(from).clone() else {
                    self.ctx.err(sp, format!(
                        "`as` converts between numeric primitives —found `{}` (RFC 0007 §1)",
                        self.ctx.types.name(from)
                    ));
                    return Err(());
                };
                let dst = self.new_reg(want);
                self.emit(Op::Conv { dst, src, from: from_prim, to }, sp.lo);
                Ok(want)
            }
            _ => {
                self.ctx.err(sp, "unsupported expression in this build");
                Err(())
            }
        }
    }

    // last register allocated (the value produced by compile_expr)
    // NOTE: maintained by every producing helper via self.new_reg

    pub(crate) fn load_lit(&mut self, lit: Lit, expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<(TypeId, u16)> {
        match lit {
            Lit::Int(v, sfx) => {
                let ty = match sfx {
                    Some(s) => int_suffix_ty(s),
                    None => {
                        // RFC 0007 §1: an unsuffixed literal defaults to
                        // `i32` and adapts bidirectionally — but only while
                        // it FITS that default. Past it the literal must
                        // declare itself: `Vec<u64>.from([..])` no longer
                        // silently absorbs a 20-digit literal, so a dropped
                        // or doubled digit can't masquerade as a value.
                        let over_default = v > i32::MAX as u64;
                        if over_default {
                            let hint = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                                Some(TyKind::Prim(p)) if p.is_float() => {
                                    format!(" —in a float position write `{v}.0`")
                                }
                                _ => String::new(),
                            };
                            self.ctx.err(sp, format!(
                                "integer literal {v} exceeds the `i32` default —add an explicit suffix like `u64`{hint} (RFC 0007 §1)"
                            ));
                        }
                        match expected {
                            // bidirectional inference (RFC 0007 §1): an int
                            // literal adapts to the expected numeric type — int
                            // widths AND float positions (`x: f32 = 1`)
                            Some(e) if matches!(self.ctx.types.kind(e), TyKind::Prim(p) if p.is_int()) => e,
                            Some(e) if matches!(self.ctx.types.kind(e), TyKind::Prim(p) if p.is_float()) => {
                                let f = v as f64;
                                let reg = self.new_reg(e);
                                if e == TY_F32 {
                                    self.emit(Op::ConstRaw { dst: reg, bits: ((f as f32) as f64).to_bits() }, sp.lo);
                                } else {
                                    self.emit(Op::ConstRaw { dst: reg, bits: f.to_bits() }, sp.lo);
                                }
                                return Ok((e, reg));
                            }
                            // past the default without a suffix there is no
                            // honest type left — `u64` is the smallest width
                            // that holds the value and keeps codegen going
                            // for the remaining diagnostics
                            _ if over_default => TY_U64,
                            _ => TY_I32,
                        }
                    }
                };
                // range check (RFC 0007 §1)
                if let TyKind::Prim(p) = self.ctx.types.kind(ty) {
                    let (min, max): (i128, i128) = match p {
                        PrimTy::U8 => (0, u8::MAX as i128),
                        PrimTy::U16 => (0, u16::MAX as i128),
                        PrimTy::U32 => (0, u32::MAX as i128),
                        PrimTy::U64 => (0, u64::MAX as i128),
                        PrimTy::I8 => (i8::MIN as i128, i8::MAX as i128),
                        PrimTy::I16 => (i16::MIN as i128, i16::MAX as i128),
                        PrimTy::I32 => (i32::MIN as i128, i32::MAX as i128),
                        PrimTy::I64 => (i64::MIN as i128, i64::MAX as i128),
                        _ => (i64::MIN as i128, i64::MAX as i128),
                    };
                    if (v as i128) < min || (v as i128) > max {
                        self.ctx.err(sp, format!(
                            "integer literal {v} does not fit `{}` (RFC 0007 §1)",
                            self.ctx.types.name(ty)
                        ));
                    }
                }
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: v }, sp.lo);
                Ok((ty, reg))
            }
            Lit::Float(bits, sfx) => {
                // the same law as the ints above, for the `f32` default: an
                // unsuffixed literal whose magnitude only exists in f64
                // must say so (precision is not the trigger — range is)
                if sfx.is_none() {
                    let x = f64::from_bits(bits);
                    if !x.is_finite() || x.abs() > f32::MAX as f64 {
                        self.ctx.err(sp, format!(
                            "float literal {:e} exceeds the `f32` default —add an explicit `f64` suffix (RFC 0007 §1)",
                            x
                        ));
                    }
                }
                let ty = match sfx {
                    Some(s) => float_suffix_ty(s),
                    None => match expected {
                        Some(e) if matches!(self.ctx.types.kind(e), TyKind::Prim(p) if p.is_float()) => e,
                        _ => TY_F32,
                    },
                };
                // The lexer stores literals as f64 bits. Narrow that payload
                // to the register's width so an `f32` literal actually carries
                // f32 precision in its slot — arithmetic and `str()` re-narrow,
                // but float comparisons read the slot back as f64 (RFC 0004 §3).
                let bits = if ty == TY_F32 {
                    (f64::from_bits(bits) as f32 as f64).to_bits()
                } else {
                    bits
                };
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                Ok((ty, reg))
            }
            Lit::Str(s) | Lit::RawStr(s) => {
                let k = self.konst(ConstVal::Str(s));
                let reg = self.new_reg(TY_STR);
                self.emit(Op::Const { dst: reg, k: k as u32 }, sp.lo);
                Ok((TY_STR, reg))
            }
            Lit::Bool(b) => {
                let reg = self.new_reg(TY_BOOL);
                self.emit(Op::ConstRaw { dst: reg, bits: b as u64 }, sp.lo);
                Ok((TY_BOOL, reg))
            }
            Lit::Nil => {
                // `nil` (RFC 0005): typed by the expected position —
                // `let p: *T = nil`, `p == nil` — or the default `*unit`.
                // The null slot IS zero bits (Slot::null).
                let ty = match expected {
                    Some(e) if matches!(self.ctx.types.kind(e), TyKind::Ptr { .. }) => e,
                    _ => self.ctx.mk_ptr(TY_UNIT),
                };
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: 0 }, sp.lo);
                Ok((ty, reg))
            }
        }
    }

    pub(crate) fn compile_path(&mut self, _node: NodeHandle<AnyExpr>, segs: Vec<PathSeg>, expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // single name: local / module let / enum member of... / builtin value
        if segs.len() == 1 && segs[0].generics.is_empty() {
            let name = segs[0].name;
            let n = self.ctx.name(name).to_string();
            if let Some(l) = self.lookup(name).copied() {
                // inlined `Slice` accessor: `self` is the receiver register
                // itself — no copy (RFC 0005 sequence access)
                if let Some((sid, reg)) = self.inline_self {
                    if sid == name {
                        self.last_reg = reg;
                        return Ok(l.ty);
                    }
                }
                // copy into a fresh register (keeps regs SSA-ish); the
                // wrapper clones value results at the boundary
                let reg = self.new_reg(l.ty);
                if self.ctx.types.is_ref(l.ty) {
                    self.emit(Op::MovRef { dst: reg, src: l.reg }, sp.lo);
                } else {
                    self.emit(Op::Mov { dst: reg, src: l.reg }, sp.lo);
                }
                return Ok(l.ty);
            }
            match n.as_str() {
                "own" | "downcast" | "panic" | "assert" if self.ctx.extern_native_fns.contains(&name) => {
                    self.ctx.err(sp, format!("`{n}` is a function —call it: `{n}(..)`"));
                    return Err(());
                }
                "print" | "type_id" => {
                    self.ctx.err(sp, format!("`{n}` is a function —call it: `{n}(..)`"));
                    return Err(());
                }
                _ => {}
            }
            // module let (load-time, M1: literals only)
            if let Some((_, ty_node, init)) = self.ctx.find_let(name).cloned() {
                let ty = ty_node.map(|t| self.resolve_type_now(t)).unwrap_or(TY_I32);
                let _reg = self.load_const_let(init, ty, expected, sp)?;
                return Ok(ty);
            }
            // imported constant (native modules: `std:math::PI`)
            if let Some((ty, bits)) = self.ctx.extern_const(name) {
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                return Ok(ty);
            }
            let msg = self
                .ctx
                .not_in_core_scope(&n)
                .unwrap_or_else(|| format!("unknown name `{n}`"));
            self.ctx.err(sp, msg);
            return Err(());
        }
        // `<namespace>.CONST` — an imported namespace's constants
        // (`Math.PI`; RFC 0028). Name-generic: routed by the bound
        // namespace head, never by a hardcoded string.
        if segs.len() == 2 && self.ctx.is_extern_namespace(segs[0].name) {
            if let Some((ty, bits)) = self.ctx.extern_const(segs[1].name) {
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                return Ok(ty);
            }
            let ns = self.ctx.name(segs[0].name).to_string();
            let m = self.ctx.name(segs[1].name).to_string();
            self.ctx.err(sp, format!("`{ns}.{m}` is not a namespace constant"));
            return Err(());
        }
        // two segments: local field chain `p.x`, `req.value.reply` — locals
        // shadow type names (a local is a value position)
        if segs.len() >= 2 && segs[0].generics.is_empty() {
            if let Some(l) = self.lookup(segs[0].name).copied() {
                let mut cur = l.reg;
                let mut cur_ty = l.ty;
                for seg in &segs[1..] {
                    if !seg.generics.is_empty() {
                        self.ctx.err(sp, "generic arguments are not valid on a field");
                        return Err(());
                    }
                    // `p.x` auto-deref through a pointer (RFC 0005)
                    if let TyKind::Ptr { elem } = self.ctx.types.kind(cur_ty).clone() {
                        // deref: load the pointee cell, then read the
                        // field from it
                        let dst = self.new_reg(elem);
                        self.emit(Op::GetF { dst, obj: cur, field: 0, repr: Repr::Ref }, sp.lo);
                        cur = dst;
                        cur_ty = elem;
                    }
                    let TyKind::Data { fields } = self.ctx.types.kind(cur_ty).clone() else {
                        self.ctx.err(sp, format!(
                            "`{}` has no field `{}` — `{}` is not a record",
                            self.ctx.types.name(cur_ty), self.ctx.name(seg.name), self.ctx.types.name(cur_ty)
                        ));
                        return Err(());
                    };
                    let Some(fidx) = fields.iter().position(|f| f.name == self.ctx.name(seg.name)) else {
                        self.ctx.err(sp, format!(
                            "`{}` has no field `{}`",
                            self.ctx.types.name(cur_ty), self.ctx.name(seg.name)
                        ));
                        return Err(());
                    };
                    let fty = fields[fidx].ty;
                    let dst = self.new_reg(fty);
                    self.emit(Op::GetF { dst, obj: cur, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                    cur = dst;
                    cur_ty = fty;
                }
                return Ok(cur_ty);
            }
        }
        // two segments: Enum.Member value
        if segs.len() == 2 {
            let base = segs[0].name;
            let member = segs[1].name;
            if !segs[0].generics.is_empty() || !segs[1].generics.is_empty() {
                self.ctx.err(sp, "generic paths are not supported in this build");
                return Err(());
            }
            if let Some(e) = self.ctx.find_enum(base).cloned() {
                if let Some(i) = e.members.iter().position(|&m| m == member) {
                    let reg = self.new_reg(e.ty);
                    self.emit(Op::EnumNew { dst: reg, ty: e.ty, member: i as u32 }, sp.lo);
                    return Ok(e.ty);
                }
                self.ctx.err(sp, format!("`{}` is not a member of enum {}", self.ctx.name(member), self.ctx.name(base)));
                return Err(());
            }
        }
        self.ctx.err(
            sp,
            format!(
                "unknown name `{}` —module paths need imports, which are not available in this build (RFC 0035)",
                segs.iter().map(|s| self.ctx.name(s.name).to_string()).collect::<Vec<_>>().join(".")
            ),
        );
        Err(())
    }

    pub(crate) fn load_const_let(&mut self, init: NodeHandle<AnyExpr>, ty: TypeId, _expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<u16> {
        match self.ctx.ast.expr(init).clone() {
            ExprKind::Lit(Lit::Int(v, sfx)) => {
                // the same default-width law as `load_lit` — module scope
                // gets no free pass (RFC 0007 §1)
                if sfx.is_none() && v > i32::MAX as u64 {
                    self.ctx.err(sp, format!(
                        "integer literal {v} exceeds the `i32` default —add an explicit suffix like `u64` (RFC 0007 §1)"
                    ));
                }
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: v }, sp.lo);
                Ok(reg)
            }
            ExprKind::Lit(Lit::Float(b, _)) => {
                // same f32 narrowing as `load_lit`
                let bits = if ty == TY_F32 {
                    (f64::from_bits(b) as f32 as f64).to_bits()
                } else {
                    b
                };
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                Ok(reg)
            }
            ExprKind::Lit(Lit::Bool(v)) => {
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: v as u64 }, sp.lo);
                Ok(reg)
            }
            ExprKind::Lit(Lit::Str(s)) | ExprKind::Lit(Lit::RawStr(s)) => {
                let k = self.konst(ConstVal::Str(s));
                let reg = self.new_reg(TY_STR);
                self.emit(Op::Const { dst: reg, k: k as u32 }, sp.lo);
                Ok(reg)
            }
            _ => {
                self.ctx.err(sp, "module `let` initializers must be literals in this build (RFC 0003 §1)");
                Err(())
            }
        }
    }
}
