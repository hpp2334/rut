//! Compiler-lowered numeric methods (RFC 0032 §1.1 R2): the operations
//! `core`'s `builtin impl <int>` blocks declare and the frontend expands
//! inline at the METHOD call — `x.wrapping_add(y)` — instead of calling:
//! `wrapping_*`, `saturating_*`, `checked_*` integer arithmetic. Each
//! lowering is a fixed sequence of typed ops (`WAddI`/`WSubI`/`WMulI`/
//! `WrapShlI`, compares, guarded division, `OptSome`/`OptNone`),
//! monomorphic per primitive width, so the VM needs no new opcodes. The
//! width comes from the RECEIVER's primitive type — the method table is
//! core's surface (`Surface::core().native_impls`), ambient on the
//! primitives (no `use`). `saturating_*`/`checked_*` run on the
//! *wrapping* result plus a branchless overflow test.

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

pub(crate) fn intrinsic_name(i: Intrinsic) -> &'static str {
    // the method spelling (`x.<name>(y)`) — kept for diagnostics/tests
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
    }
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// `x.wrapping_add(y)` — a builtin-impl method call on a primitive
    /// receiver (core's `builtin impl i32 { .. }` table, RFC 0032 §1.1
    /// R2). The receiver is ALREADY compiled (its register reused — a
    /// side-effecting receiver evaluates exactly once); the width comes
    /// from its primitive type.
    pub(crate) fn compile_intrinsic_method(
        &mut self,
        i: Intrinsic,
        recv_ty: TypeId,
        recv_reg: u16,
        rhs: NodeHandle<AnyExpr>,
        sp: Span,
    ) -> TcResult<TypeId> {
        let who = intrinsic_name(i);
        let (ty, prim, a, b) = self.int_method_operands(recv_ty, recv_reg, rhs, sp, who)?;
        match i {
            Intrinsic::WrappingAdd | Intrinsic::WrappingSub | Intrinsic::WrappingMul => {
                let op = Self::arith_of(i);
                let dst = self.new_reg(ty);
                self.emit(wrap_arith(op, prim, dst, a, b), sp.lo);
                Ok(ty)
            }
            Intrinsic::WrappingShl => {
                let dst = self.new_reg(ty);
                self.emit(bitop(BitOp::WrapShl, prim, dst, a, b), sp.lo);
                Ok(ty)
            }
            Intrinsic::SaturatingAdd | Intrinsic::SaturatingSub | Intrinsic::SaturatingMul => {
                self.saturating_arith(i, ty, prim, a, b, sp)
            }
            Intrinsic::CheckedAdd | Intrinsic::CheckedSub | Intrinsic::CheckedMul => {
                self.checked_arith(i, ty, prim, a, b, sp)
            }
        }
    }

    /// `(ty, prim, lhs, rhs)` for a method call: the receiver arrives
    /// pre-compiled; the one argument must land on the same integer type.
    fn int_method_operands(
        &mut self,
        recv_ty: TypeId,
        recv_reg: u16,
        rhs: NodeHandle<AnyExpr>,
        sp: Span,
        who: &str,
    ) -> TcResult<(TypeId, PrimTy, u16, u16)> {
        let prim = match self.ctx.types.kind(recv_ty) {
            TyKind::Prim(p) if p.is_int() => *p,
            _ => {
                self.ctx.err(
                    sp,
                    format!("{who} needs an integer receiver — found `{}`", self.ctx.type_name(recv_ty)),
                );
                return Err(());
            }
        };
        let t2 = self.compile_expr(rhs, Some(recv_ty))?;
        if t2 != recv_ty {
            self.ctx.err(sp, format!(
                "{who} operands must have the same type (`{}` vs `{}`)",
                self.ctx.type_name(recv_ty),
                self.ctx.type_name(t2)
            ));
            return Err(());
        }
        Ok((recv_ty, prim, recv_reg, self.last_reg))
    }

    /// `Math.method(..)` — the `calc` namespace: a host call for the
    /// `f64` primitives (`Math.sqrt(x)`, `Math.abs(x)` — the float
    /// helpers are ordinary host fns).
    /// A namespace member call: an extern fn of the used module
    /// (RFC 0028). The namespace head is passed only for diagnostics —
    /// routing is the caller's bound-namespace check, name-generic.
    pub(crate) fn compile_namespace_member(
        &mut self,
        ns: IdentId,
        member: IdentId,
        args: &[NodeHandle<AnyExpr>],
        _expected: Option<TypeId>,
        sp: Span,
    ) -> TcResult<TypeId> {
        let Some(ef) = self.ctx.extern_fn(member).cloned() else {
            let head = self.ctx.name(ns).to_string();
            let name = self.ctx.name(member).to_string();
            self.ctx.err(sp, format!("`{head}.{name}` is not a namespace member"));
            return Err(());
        };
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

    /// A fresh register holding the raw scalar `v` of an integer primitive.
    fn int_const(&mut self, ty: TypeId, prim: PrimTy, v: i128, sp: u32) -> u16 {
        let bits = if prim.is_unsigned() { v as u64 } else { (v as i64) as u64 };
        let r = self.new_reg(ty);
        self.emit(Op::ConstRaw { dst: r, bits }, sp);
        r
    }

    /// `x >> (bits-1)` — the sign mask (all ones when negative, zero when
    /// not), used branchlessly for saturating clamps.
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

    fn saturating_arith(
        &mut self,
        i: Intrinsic,
        ty: TypeId,
        prim: PrimTy,
        a: u16,
        b: u16,
        sp: Span,
    ) -> TcResult<TypeId> {
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

    fn checked_arith(
        &mut self,
        i: Intrinsic,
        ty: TypeId,
        prim: PrimTy,
        a: u16,
        b: u16,
        sp: Span,
    ) -> TcResult<TypeId> {
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
}
