//! Expression compilation: the value-in-last_reg convention, literals with
//! bidirectional inference (RFC 0007 SS1), path/field-chain reads,
//! enum members, and module-let loads.

use crate::check::{float_suffix_ty, int_suffix_ty, TcResult};
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
        let r = self.compile_expr_inner(node, expected);
        self.leave();
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
            ExprKind::Index { recv, idx } => {
                let rt = self.compile_expr(recv, None)?;
                let rreg = self.last_reg;
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
                self.ctx.err(sp, "the `?` operator is not supported in this build (RFC 0005, M2)");
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
                                    // exact receiver: fold by impl presence (RFC 0012 §3)
                                    let has = self
                                        .ctx
                                        .impls
                                        .iter()
                                        .any(|i| i.trait_id == tid && i.target == rt);
                                    self.emit(Op::ConstRaw { dst, bits: has as u64 }, sp.lo);
                                }
                            }
                            return Ok(TY_BOOL);
                        }
                        // Opaque RHS
                        if n == "Opaque" {
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
                    None => match expected {
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
                        _ => TY_I32,
                    },
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
            Lit::Char(c) => {
                let reg = self.new_reg(TY_CHAR);
                self.emit(Op::ConstRaw { dst: reg, bits: c as u64 }, sp.lo);
                Ok((TY_CHAR, reg))
            }
            Lit::Bool(b) => {
                let reg = self.new_reg(TY_BOOL);
                self.emit(Op::ConstRaw { dst: reg, bits: b as u64 }, sp.lo);
                Ok((TY_BOOL, reg))
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
                // copy into a fresh register (keeps regs SSA-ish)
                let reg = self.new_reg(l.ty);
                if self.ctx.types.is_ref(l.ty) {
                    self.emit(Op::MovRef { dst: reg, src: l.reg }, sp.lo);
                } else {
                    self.emit(Op::Mov { dst: reg, src: l.reg }, sp.lo);
                }
                return Ok(l.ty);
            }
            match n.as_str() {
                "own" | "downcast" | "panic" | "assert" | "print" | "type_id" => {
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
            self.ctx.err(sp, format!("unknown name `{n}`"));
            return Err(());
        }
        // `Math.PI` — the `std:math` constants (namespace member)
        let math_const = segs.len() == 2 && self.ctx.name(segs[0].name) == "Math";
        if math_const {
            if let Some((ty, bits)) = self.ctx.extern_const(segs[1].name) {
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                return Ok(ty);
            }
            let m = self.ctx.name(segs[1].name).to_string();
            self.ctx.err(sp, format!("`Math.{m}` is not a math constant"));
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
                    // Option/Result payload accessors mid-chain (RFC 0005)
                    match self.ctx.types.kind(cur_ty).clone() {
                        TyKind::Option { elem } if self.ctx.name(seg.name) == "value" => {
                            let dst = self.new_reg(elem);
                            self.emit(Op::Unwrap { dst, v: cur, want_err: false }, sp.lo);
                            cur = dst;
                            cur_ty = elem;
                            continue;
                        }
                        TyKind::Result { ok, err } => {
                            let sn = self.ctx.name(seg.name);
                            if sn == "value" {
                                let dst = self.new_reg(ok);
                                self.emit(Op::Unwrap { dst, v: cur, want_err: false }, sp.lo);
                                cur = dst;
                                cur_ty = ok;
                                continue;
                            }
                            if sn == "error" {
                                let dst = self.new_reg(err);
                                self.emit(Op::Unwrap { dst, v: cur, want_err: true }, sp.lo);
                                cur = dst;
                                cur_ty = err;
                                continue;
                            }
                        }
                        _ => {}
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
            // Option.none / Option.none() without call parens
            if self.ctx.name(base) == "Option" && self.ctx.name(member) == "none" {                let elem = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                    Some(TyKind::Option { elem }) => elem,
                    _ => {
                        self.ctx.err(sp, "cannot infer the element type of `Option.none` here —annotate the binding");
                        return Err(());
                    }
                };
                let oty = self.ctx.mk_option(elem);
                let reg = self.new_reg(oty);
                self.emit(Op::OptNone { dst: reg, ty: oty }, sp.lo);
                return Ok(oty);
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
            ExprKind::Lit(Lit::Int(v, _)) => {
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
            ExprKind::Lit(Lit::Char(c)) => {
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: c as u64 }, sp.lo);
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
