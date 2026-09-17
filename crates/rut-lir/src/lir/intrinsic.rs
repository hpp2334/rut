//! Compiler-lowered intrinsics (RFC 0032 §1.1 R2): the operations
//! `calc` exposes that the frontend expands inline instead of calling —
//! `wrapping_*`, `saturating_*`, `checked_*` integer arithmetic and the
//! `abs`/`min`/`max`/`signum` helpers. Each lowering is a fixed sequence of
//! typed ops (`WAddI`/`WSubI`/`WMulI`/`WrapShlI`, compares, guarded
//! division, `OptSome`/`OptNone`), monomorphic per primitive width, so the
//! VM needs no new opcodes. `saturating_*`/`checked_*` run on the *wrapping*
//! result plus a branchless overflow test.

use super::*;

/// Inclusive bounds of an integer primitive, as exact mathematical values.
fn int_bounds(p: PrimTy) -> (i128, i128) {
    match p {
        PrimTy::U8 => (0, u8::MAX as i128),
        PrimTy::U16 => (0, u16::MAX as i128),
        PrimTy::U32 => (0, u32::MAX as i128),
        PrimTy::U64 => (0, u64::MAX as i128),
        PrimTy::I8 => (i8::MIN as i128, i8::MAX as i128),
        PrimTy::I16 => (i16::MIN as i128, i16::MAX as i128),
        PrimTy::I32 => (i32::MIN as i128, i32::MAX as i128),
        PrimTy::I64 => (i64::MIN as i128, i64::MAX as i128),
        _ => (0, 0),
    }
}

/// Bit width of an integer primitive.
fn int_bits(p: PrimTy) -> i128 {
    match p {
        PrimTy::U8 | PrimTy::I8 => 8,
        PrimTy::U16 | PrimTy::I16 => 16,
        PrimTy::U32 | PrimTy::I32 => 32,
        PrimTy::U64 | PrimTy::I64 => 64,
        _ => 64,
    }
}

fn intrinsic_name(i: Intrinsic) -> &'static str {
    use Intrinsic::*;
    match i {
        WrappingAdd => "wrapping_add",
        WrappingSub => "wrapping_sub",
        WrappingMul => "wrapping_mul",
        WrappingShl => "wrapping_shl",
        SaturatingAdd => "saturating_add",
        SaturatingSub => "saturating_sub",
        SaturatingMul => "saturating_mul",
        CheckedAdd => "checked_add",
        CheckedSub => "checked_sub",
        CheckedMul => "checked_mul",
        Abs => "abs",
        Min => "min",
        Max => "max",
        Signum => "signum",
    }
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    pub(crate) fn compile_intrinsic(
        &mut self,
        i: Intrinsic,
        args: &[NodeHandle<AnyExpr>],
        _expected: Option<TypeId>,
        sp: Span,
    ) -> TcResult<TypeId> {
        use Intrinsic::*;
        match i {
            WrappingAdd | WrappingSub | WrappingMul => self.wrapping_arith(i, args, sp),
            WrappingShl => self.wrapping_shl(args, sp),
            SaturatingAdd | SaturatingSub | SaturatingMul => self.saturating_arith(i, args, sp),
            CheckedAdd | CheckedSub | CheckedMul => self.checked_arith(i, args, sp),
            Abs => self.intrinsic_abs(args, sp),
            Min | Max => self.intrinsic_minmax(i, args, sp),
            Signum => self.intrinsic_signum(args, sp),
        }
    }

    /// `Math.method(..)` — the `calc` namespace: a host call for the
    /// `f64` primitives, or an inline lowering for the integer intrinsics
    /// (RFC 0032 §1.1 R2).
    /// A namespace member call (`Math.sqrt(x)`, `Math.wrapping_add(a, b)`):
    /// an extern fn or intrinsic of the used module (RFC 0028). The
    /// namespace head is passed only for diagnostics — routing is the
    /// caller's bound-namespace check, name-generic.
    pub(crate) fn compile_namespace_member(
        &mut self,
        ns: IdentId,
        member: IdentId,
        args: &[NodeHandle<AnyExpr>],
        expected: Option<TypeId>,
        sp: Span,
    ) -> TcResult<TypeId> {
        let Some(ef) = self.ctx.extern_fn(member).cloned() else {
            let head = self.ctx.name(ns).to_string();
            let name = self.ctx.name(member).to_string();
            self.ctx.err(sp, format!("`{head}.{name}` is not a namespace member"));
            return Err(());
        };
        if let Some(i) = ef.intrinsic {
            return self.compile_intrinsic(i, args, expected, sp);
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
            aregs.push(self.last_reg);
        }
        let dst = if ef.ret == TY_NIL { None } else { Some(self.new_reg(ef.ret)) };
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: ef.func, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ef.ret)
    }

    // ---- shared helpers ----

    /// The primitive of a *numeric* (int or float) argument type.
    fn num_prim(ty: TypeId, sp: Span, who: &str, ctx: &mut Ctx) -> TcResult<PrimTy> {
        match ctx.types.kind(ty) {
            TyKind::Prim(p) if p.is_int() || p.is_float() => Ok(*p),
            _ => {
                ctx.err(sp, format!("{who} needs numbers — found `{}`", ctx.type_name(ty)));
                Err(())
            }
        }
    }

    /// Compile `name(a, b)` with both operands the same numeric type.
    fn num_binop_args(
        &mut self,
        args: &[NodeHandle<AnyExpr>],
        sp: Span,
        who: &str,
    ) -> TcResult<(TypeId, PrimTy, u16, u16)> {
        if args.len() != 2 {
            self.ctx.err(sp, format!("{who}(a, b) takes two arguments"));
            return Err(());
        }
        let ty = self.compile_expr(args[0], None)?;
        let prim = Self::num_prim(ty, sp, who, self.ctx)?;
        let a = self.last_reg;
        let t2 = self.compile_expr(args[1], Some(ty))?;
        if t2 != ty {
            self.ctx.err(sp, format!(
                "{who} operands must have the same type (`{}` vs `{}`)",
                self.ctx.type_name(ty), self.ctx.type_name(t2)
            ));
            return Err(());
        }
        let b = self.last_reg;
        Ok((ty, prim, a, b))
    }

    /// Compile `name(a, b)` with both operands the same *integer* type.
    fn int_binop_args(
        &mut self,
        args: &[NodeHandle<AnyExpr>],
        sp: Span,
        who: &str,
    ) -> TcResult<(TypeId, PrimTy, u16, u16)> {
        let (ty, prim, a, b) = self.num_binop_args(args, sp, who)?;
        if !prim.is_int() {
            self.ctx.err(sp, format!("{who} needs integers — found `{}`", self.ctx.type_name(ty)));
            return Err(());
        }
        Ok((ty, prim, a, b))
    }

    /// A fresh register holding the raw scalar `v` of an integer primitive.
    fn int_const(&mut self, ty: TypeId, prim: PrimTy, v: i128, sp: u32) -> u16 {
        let bits = if prim.is_unsigned() { v as u64 } else { (v as i64) as u64 };
        let r = self.new_reg(ty);
        self.emit(Op::ConstRaw { dst: r, bits }, sp);
        r
    }

    /// A fresh register holding the float `v` (rounded to the register width).
    fn float_const(&mut self, ty: TypeId, v: f64, sp: u32) -> u16 {
        let bits = if ty == TY_F32 { ((v as f32) as f64).to_bits() } else { v.to_bits() };
        let r = self.new_reg(ty);
        self.emit(Op::ConstRaw { dst: r, bits }, sp);
        r
    }

    /// `x >> (bits-1)` — the sign mask (all ones when negative, zero when
    /// not), used branchlessly for `abs` and saturating clamps.
    fn sign_mask(&mut self, ty: TypeId, prim: PrimTy, x: u16, sp: u32) -> u16 {
        let sh = self.int_const(ty, prim, int_bits(prim) - 1, sp);
        let r = self.new_reg(ty);
        self.emit(bitop(BitOp::Shr, prim, r, x, sh), sp);
        r
    }

    fn cmp(&mut self, op: CmpOp, prim: PrimTy, a: u16, b: u16, sp: u32) -> u16 {
        let r = self.new_reg(TY_BOOL);
        self.emit(cmpop(op, prim, r, a, b), sp);
        r
    }

    // ---- wrapping ----

    fn wrapping_arith(&mut self, i: Intrinsic, args: &[NodeHandle<AnyExpr>], sp: Span) -> TcResult<TypeId> {
        let who = intrinsic_name(i);
        let (ty, prim, a, b) = self.int_binop_args(args, sp, who)?;
        let op = match i {
            Intrinsic::WrappingAdd => ArithOp::Add,
            Intrinsic::WrappingSub => ArithOp::Sub,
            _ => ArithOp::Mul,
        };
        let dst = self.new_reg(ty);
        self.emit(wrap_arith(op, prim, dst, a, b), sp.lo);
        Ok(ty)
    }

    fn wrapping_shl(&mut self, args: &[NodeHandle<AnyExpr>], sp: Span) -> TcResult<TypeId> {
        let who = "wrapping_shl";
        let (ty, prim, a, b) = self.int_binop_args(args, sp, who)?;
        let dst = self.new_reg(ty);
        self.emit(bitop(BitOp::WrapShl, prim, dst, a, b), sp.lo);
        Ok(ty)
    }

    // ---- overflow test on the wrapping result ----

    /// `(r, ovf)`: `r = wrap(a op b)`; `ovf` is a bool register that is true
    /// exactly when the mathematical result escapes the primitive's range.
    fn wrap_and_overflow(
        &mut self,
        op: ArithOp,
        prim: PrimTy,
        ty: TypeId,
        a: u16,
        b: u16,
        sp: u32,
    ) -> (u16, u16) {
        let r = self.new_reg(ty);
        self.emit(wrap_arith(op, prim, r, a, b), sp);
        let ovf = match op {
            ArithOp::Add if prim.is_unsigned() => self.cmp(CmpOp::Lt, prim, r, a, sp),
            ArithOp::Add => {
                // overflow iff the addends share a sign with each other but
                // not with the result: ((a ^ r) & (b ^ r)) < 0
                let x1 = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: x1, a, b: r }, sp);
                let x2 = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: x2, a: b, b: r }, sp);
                let x3 = self.new_reg(ty);
                self.emit(Op::AndI { prim, dst: x3, a: x1, b: x2 }, sp);
                let zero = self.int_const(ty, prim, 0, sp);
                self.cmp(CmpOp::Lt, prim, x3, zero, sp)
            }
            ArithOp::Sub if prim.is_unsigned() => self.cmp(CmpOp::Gt, prim, r, a, sp),
            ArithOp::Sub => {
                // ((a ^ b) & (a ^ r)) < 0
                let x1 = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: x1, a, b }, sp);
                let x2 = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: x2, a, b: r }, sp);
                let x3 = self.new_reg(ty);
                self.emit(Op::AndI { prim, dst: x3, a: x1, b: x2 }, sp);
                let zero = self.int_const(ty, prim, 0, sp);
                self.cmp(CmpOp::Lt, prim, x3, zero, sp)
            }
            ArithOp::Mul => self.mul_overflow(prim, ty, a, b, r, sp),
            _ => unreachable!("wrap_and_overflow: bad arith op"),
        };
        (r, ovf)
    }

    /// Multiplication overflow, guarded so the division test never divides
    /// by zero (and never evaluates `MIN / -1`).
    fn mul_overflow(&mut self, prim: PrimTy, ty: TypeId, a: u16, b: u16, r: u16, sp: u32) -> u16 {
        let zero = self.int_const(ty, prim, 0, sp);
        let ovf = self.new_reg(TY_BOOL);
        let l_zero = self.new_label();
        let l_nonzero = self.new_label();
        let l_end = self.new_label();

        let a_zero = self.cmp(CmpOp::Eq, prim, a, zero, sp);
        self.br(a_zero, l_zero, l_nonzero);
        self.bind(l_zero);
        self.emit(Op::ConstRaw { dst: ovf, bits: 0 }, sp);
        self.jmp(l_end);

        self.bind(l_nonzero);
        if prim.is_unsigned() {
            let q = self.new_reg(ty);
            self.emit(arith(ArithOp::Div, prim, q, r, a), sp);
            let neq = self.cmp(CmpOp::Ne, prim, q, b, sp);
            self.emit(Op::Mov { dst: ovf, src: neq }, sp);
        } else {
            // signed: `a == -1` needs its own case (`r / a` would overflow
            // on `MIN / -1`); there overflow is exactly `b == MIN`
            let minus1 = self.int_const(ty, prim, -1, sp);
            let a_m1 = self.cmp(CmpOp::Eq, prim, a, minus1, sp);
            let l_m1 = self.new_label();
            let l_calc = self.new_label();
            self.br(a_m1, l_m1, l_calc);
            self.bind(l_m1);
            let (imin, _) = int_bounds(prim);
            let minv = self.int_const(ty, prim, imin, sp);
            let is_min = self.cmp(CmpOp::Eq, prim, b, minv, sp);
            self.emit(Op::Mov { dst: ovf, src: is_min }, sp);
            self.jmp(l_end);

            self.bind(l_calc);
            let q = self.new_reg(ty);
            self.emit(arith(ArithOp::Div, prim, q, r, a), sp);
            let neq = self.cmp(CmpOp::Ne, prim, q, b, sp);
            self.emit(Op::Mov { dst: ovf, src: neq }, sp);
        }
        self.bind(l_end);
        ovf
    }

    /// The saturated value on overflow: `MAX`/`MIN` chosen branchlessly.
    fn saturating_clamp(&mut self, op: ArithOp, prim: PrimTy, ty: TypeId, a: u16, b: u16, sp: u32) -> u16 {
        let (imin, imax) = int_bounds(prim);
        match op {
            ArithOp::Add if prim.is_unsigned() => self.int_const(ty, prim, imax, sp),
            ArithOp::Add => {
                // positive overflow iff `a > 0`; MAX when positive else MIN
                let maxr = self.int_const(ty, prim, imax, sp);
                let mask = self.sign_mask(ty, prim, a, sp);
                let r = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: r, a: maxr, b: mask }, sp);
                r
            }
            ArithOp::Sub if prim.is_unsigned() => self.int_const(ty, prim, 0, sp),
            ArithOp::Sub => {
                // `a - b` underflows toward MIN when `b > 0`, toward MAX when
                // `b < 0`; clamp = MIN ^ (b >> w-1)
                let minr = self.int_const(ty, prim, imin, sp);
                let mask = self.sign_mask(ty, prim, b, sp);
                let r = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: r, a: minr, b: mask }, sp);
                r
            }
            ArithOp::Mul if prim.is_unsigned() => self.int_const(ty, prim, imax, sp),
            ArithOp::Mul => {
                // the product's sign comes from the operands: same sign ->
                // MAX, opposite -> MIN; s = 0 when same, ~0 when opposite
                let maxr = self.int_const(ty, prim, imax, sp);
                let ma = self.sign_mask(ty, prim, a, sp);
                let mb = self.sign_mask(ty, prim, b, sp);
                let s = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: s, a: ma, b: mb }, sp);
                let r = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: r, a: maxr, b: s }, sp);
                r
            }
            _ => unreachable!("saturating_clamp: bad arith op"),
        }
    }

    // ---- saturating / checked ----

    fn arith_of(i: Intrinsic) -> ArithOp {
        use Intrinsic::*;
        match i {
            WrappingAdd | SaturatingAdd | CheckedAdd => ArithOp::Add,
            WrappingSub | SaturatingSub | CheckedSub => ArithOp::Sub,
            _ => ArithOp::Mul,
        }
    }

    fn saturating_arith(&mut self, i: Intrinsic, args: &[NodeHandle<AnyExpr>], sp: Span) -> TcResult<TypeId> {
        let who = intrinsic_name(i);
        let (ty, prim, a, b) = self.int_binop_args(args, sp, who)?;
        let op = Self::arith_of(i);
        let (r, ovf) = self.wrap_and_overflow(op, prim, ty, a, b, sp.lo);
        let clamp = self.saturating_clamp(op, prim, ty, a, b, sp.lo);
        let dst = self.new_reg(ty);
        let l_sat = self.new_label();
        let l_ok = self.new_label();
        let l_end = self.new_label();
        self.br(ovf, l_sat, l_ok);
        self.bind(l_sat);
        self.emit(Op::Mov { dst, src: clamp }, sp.lo);
        self.jmp(l_end);
        self.bind(l_ok);
        self.emit(Op::Mov { dst, src: r }, sp.lo);
        self.bind(l_end);
        self.last_reg = dst;
        Ok(ty)
    }

    fn checked_arith(&mut self, i: Intrinsic, args: &[NodeHandle<AnyExpr>], sp: Span) -> TcResult<TypeId> {
        let who = intrinsic_name(i);
        let (ty, prim, a, b) = self.int_binop_args(args, sp, who)?;
        let op = Self::arith_of(i);
        let (r, ovf) = self.wrap_and_overflow(op, prim, ty, a, b, sp.lo);
        // v1.1: the tuple convention — `(wrapped, ok)`, false on overflow
        let no = self.new_reg(TY_BOOL);
        self.emit(Op::ConstRaw { dst: no, bits: 0 }, sp.lo);
        let ok = self.cmp(CmpOp::Eq, PrimTy::Bool, ovf, no, sp.lo);
        let tty = self.ctx.mk_tuple(vec![ty, TY_BOOL]);
        let dst = self.new_reg(tty);
        { let (argv_off, argc) = self.pool_args(&(vec![r, ok])); self.emit(Op::MakeRecord { dst: dst, ty: tty, argv_off, argc }, sp.lo); }
        self.last_reg = dst;
        Ok(tty)
    }

    // ---- abs / min / max / signum (int and float) ----

    fn intrinsic_abs(&mut self, args: &[NodeHandle<AnyExpr>], sp: Span) -> TcResult<TypeId> {
        if args.len() != 1 {
            self.ctx.err(sp, "abs(x) takes one argument");
            return Err(());
        }
        let ty = self.compile_expr(args[0], None)?;
        let prim = Self::num_prim(ty, sp, "abs", self.ctx)?;
        let a = self.last_reg;
        if prim.is_int() {
            if prim.is_unsigned() {
                // identity — an unsigned value is already non-negative
                let dst = self.new_reg(ty);
                self.emit(Op::Mov { dst, src: a }, sp.lo);
                Ok(ty)
            } else {
                // (a ^ mask) - mask, wrapping: MIN stays MIN (RFC 0004 §3)
                let mask = self.sign_mask(ty, prim, a, sp.lo);
                let t = self.new_reg(ty);
                self.emit(Op::XorI { prim, dst: t, a, b: mask }, sp.lo);
                let dst = self.new_reg(ty);
                self.emit(wrap_arith(ArithOp::Sub, prim, dst, t, mask), sp.lo);
                Ok(ty)
            }
        } else {
            // r = a < 0 ? -a : a  (NaN stays NaN, ±0 stay themselves)
            let zero = self.float_const(ty, 0.0, sp.lo);
            let neg = self.cmp(CmpOp::Lt, prim, a, zero, sp.lo);
            let dst = self.new_reg(ty);
            self.emit(Op::Mov { dst, src: a }, sp.lo);
            let l_neg = self.new_label();
            let l_end = self.new_label();
            self.br(neg, l_neg, l_end);
            self.bind(l_neg);
            let n = self.new_reg(ty);
            self.emit(Op::NegF { prim, dst: n, a }, sp.lo);
            self.emit(Op::Mov { dst, src: n }, sp.lo);
            self.bind(l_end);
            self.last_reg = dst;
            Ok(ty)
        }
    }

    fn intrinsic_minmax(&mut self, i: Intrinsic, args: &[NodeHandle<AnyExpr>], sp: Span) -> TcResult<TypeId> {
        let who = intrinsic_name(i);
        let (ty, prim, a, b) = self.num_binop_args(args, sp, who)?;
        // default the result to `b`, then take `a` when the comparison holds
        // (min: a < b; max: a > b). For floats this is the `a < b ? a : b`
        // rule; `NaN` comparisons are false, so `b` is returned.
        let cmp = if i == Intrinsic::Min { CmpOp::Lt } else { CmpOp::Gt };
        let cond = self.cmp(cmp, prim, a, b, sp.lo);
        let dst = self.new_reg(ty);
        self.emit(Op::Mov { dst, src: b }, sp.lo);
        let l_take = self.new_label();
        let l_end = self.new_label();
        self.br(cond, l_take, l_end);
        self.bind(l_take);
        self.emit(Op::Mov { dst, src: a }, sp.lo);
        self.bind(l_end);
        self.last_reg = dst;
        Ok(ty)
    }

    fn intrinsic_signum(&mut self, args: &[NodeHandle<AnyExpr>], sp: Span) -> TcResult<TypeId> {
        if args.len() != 1 {
            self.ctx.err(sp, "signum(x) takes one argument");
            return Err(());
        }
        let ty = self.compile_expr(args[0], None)?;
        let prim = Self::num_prim(ty, sp, "signum", self.ctx)?;
        let a = self.last_reg;

        // `0` is the default result; `1`/`-1` overwrite it. Float `signum`
        // defaults to `a` so NaN and ±0 pass through (JS `Math.sign`).
        let dst = self.new_reg(ty);
        let default = if prim.is_float() { a } else { self.int_const(ty, prim, 0, sp.lo) };
        self.emit(Op::Mov { dst, src: default }, sp.lo);

        let zero = if prim.is_float() {
            self.float_const(ty, 0.0, sp.lo)
        } else {
            self.int_const(ty, prim, 0, sp.lo)
        };
        let one = if prim.is_float() {
            self.float_const(ty, 1.0, sp.lo)
        } else {
            self.int_const(ty, prim, 1, sp.lo)
        };
        let minus_one = if prim.is_float() {
            self.float_const(ty, -1.0, sp.lo)
        } else {
            self.int_const(ty, prim, -1, sp.lo)
        };

        let pos = self.cmp(CmpOp::Gt, prim, a, zero, sp.lo);
        let l_pos = self.new_label();
        let l_check_neg = self.new_label();
        let l_neg = self.new_label();
        let l_end = self.new_label();
        self.br(pos, l_pos, l_check_neg);
        self.bind(l_pos);
        self.emit(Op::Mov { dst, src: one }, sp.lo);
        self.jmp(l_end);
        self.bind(l_check_neg);
        if prim.is_unsigned() {
            self.jmp(l_end);
        } else {
            let neg = self.cmp(CmpOp::Lt, prim, a, zero, sp.lo);
            self.br(neg, l_neg, l_end);
            self.bind(l_neg);
            self.emit(Op::Mov { dst, src: minus_one }, sp.lo);
        }
        self.bind(l_end);
        self.last_reg = dst;
        Ok(ty)
    }
}
