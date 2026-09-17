//! `string_encode` / `bytes_decode` (RFC 0004) lowered as a composition of
//! LIR ops — there is no VM native for either. `str` is a `CellData::Str`
//! (a Rust `String`, UTF-8); `bytes` is a `u8` array. Both routines are
//! loops over `StrCharAt` (encode) / `ArrGet` (decode) with the UTF-8 bit
//! arithmetic emitted inline.
//!
//! `bytes_decode` is lossy like `String::from_utf8_lossy`: an invalid lead
//! byte or a missing/invalid continuation byte yields U+FFFD and advances
//! one octet.

use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {
    // ---- immediate/register helpers ----

    fn imm_i32(&mut self, v: i64, sp: u32) -> u16 {
        let d = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: d, bits: v as u64 }, sp);
        d
    }
    fn imm_u32(&mut self, v: u32, sp: u32) -> u16 {
        let d = self.new_reg(TY_U32);
        self.emit(Op::ConstRaw { dst: d, bits: v as u64 }, sp);
        d
    }
    fn u32_cmp(&mut self, op: CmpOp, a: u16, b: u16, sp: u32) -> u16 {
        let d = self.new_reg(TY_BOOL);
        self.emit(cmpop(op, PrimTy::U32, d, a, b), sp);
        d
    }
    fn i32_cmp(&mut self, op: CmpOp, a: u16, b: u16, sp: u32) -> u16 {
        let d = self.new_reg(TY_BOOL);
        self.emit(cmpop(op, PrimTy::I32, d, a, b), sp);
        d
    }
    fn u32_binop(&mut self, op: BitOp, a: u16, b: u16, sp: u32) -> u16 {
        let d = self.new_reg(TY_U32);
        self.emit(bitop(op, PrimTy::U32, d, a, b), sp);
        d
    }
    fn u32_shr(&mut self, a: u16, n: u32, sp: u32) -> u16 {
        let k = self.imm_u32(n, sp);
        self.u32_binop(BitOp::Shr, a, k, sp)
    }
    fn u32_shl(&mut self, a: u16, n: u32, sp: u32) -> u16 {
        let k = self.imm_u32(n, sp);
        self.u32_binop(BitOp::Shl, a, k, sp)
    }
    fn u32_and_imm(&mut self, a: u16, n: u32, sp: u32) -> u16 {
        let k = self.imm_u32(n, sp);
        self.u32_binop(BitOp::And, a, k, sp)
    }
    fn u32_or_imm(&mut self, a: u16, n: u32, sp: u32) -> u16 {
        let k = self.imm_u32(n, sp);
        self.u32_binop(BitOp::Or, a, k, sp)
    }
    fn u32_or(&mut self, a: u16, b: u16, sp: u32) -> u16 {
        self.u32_binop(BitOp::Or, a, b, sp)
    }
    fn mov(&mut self, dst: u16, src: u16, sp: u32) {
        self.emit(Op::Mov { dst, src }, sp);
    }
    fn inc_i32(&mut self, dst: u16, sp: u32) {
        let one = self.imm_i32(1, sp);
        self.emit(arith(ArithOp::Add, PrimTy::I32, dst, dst, one), sp);
    }
    fn add_i32(&mut self, a: u16, n: i64, sp: u32) -> u16 {
        let k = self.imm_i32(n, sp);
        let d = self.new_reg(TY_I32);
        self.emit(arith(ArithOp::Add, PrimTy::I32, d, a, k), sp);
        d
    }
    /// `0x80 <= b < 0xC0` — a UTF-8 continuation octet. One unsigned
    /// compare on the wrapping difference: `(b - 0x80) < 0x40`.
    fn is_cont(&mut self, b: u16, sp: u32) -> u16 {
        let k80 = self.imm_u32(0x80, sp);
        let d = self.new_reg(TY_U32);
        self.emit(wrap_arith(ArithOp::Sub, PrimTy::U32, d, b, k80), sp);
        let k40 = self.imm_u32(0x40, sp);
        self.u32_cmp(CmpOp::Lt, d, k40, sp)
    }
    fn arr_get_u8(&mut self, arr: u16, idx: u16, sp: u32) -> u16 {
        let d = self.new_reg(TY_U8);
        self.emit(Op::ArrGet { dst: d, arr, idx, repr: Repr::Prim(PrimTy::U8) }, sp);
        d
    }
    fn arr_set_u8(&mut self, arr: u16, idx: u16, val: u16, sp: u32) {
        self.emit(Op::ArrSet { arr, idx, val, repr: Repr::Prim(PrimTy::U8) }, sp);
    }
    fn str_char_at(&mut self, s: u16, idx: u16, sp: u32) -> u16 {
        let d = self.new_reg(TY_CHAR);
        self.emit(Op::StrCharAt { dst: d, s, idx }, sp);
        d
    }
    fn to_u32(&mut self, src: u16, from: PrimTy, sp: u32) -> u16 {
        let d = self.new_reg(TY_U32);
        self.emit(Op::Conv { dst: d, src, from, to: PrimTy::U32 }, sp);
        d
    }
    /// Write `val` at `w`, then advance `w` by one.
    fn store_byte(&mut self, out: u16, w: u16, val: u16, sp: u32) {
        self.arr_set_u8(out, w, val, sp);
        self.inc_i32(w, sp);
    }

    // ---- string_encode ----

    /// `string_encode(s) -> bytes` — UTF-8 encode. Pass 1 sums the encoded
    /// length, `ArrNew` the `bytes`, pass 2 writes the octets.
    pub(crate) fn emit_string_encode(&mut self, src: u16, sp: u32) -> u16 {
        let n = self.new_reg(TY_I32);
        { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::StrLen, recv: src, argv_off, argc, dst: n }, sp); }

        // ---- pass 1: total byte length ----
        let total = self.imm_i32(0, sp);
        let i = self.imm_i32(0, sp);
        let l_head = self.new_label();
        let l_body = self.new_label();
        let l_end = self.new_label();
        self.bind(l_head);
        let cond = self.i32_cmp(CmpOp::Lt, i, n, sp);
        self.br(cond, l_body, l_end);
        self.bind(l_body);
        let cp = {
            let ch = self.str_char_at(src, i, sp);
            self.to_u32(ch, PrimTy::Char, sp)
        };
        let len = self.imm_i32(1, sp);
        self.add_if_gt_u32(cp, 0x7F, len, sp);
        self.add_if_gt_u32(cp, 0x7FF, len, sp);
        self.add_if_gt_u32(cp, 0xFFFF, len, sp);
        self.emit(arith(ArithOp::Add, PrimTy::I32, total, total, len), sp);
        self.inc_i32(i, sp);
        self.jmp(l_head);
        self.bind(l_end);

        let out = self.new_reg(TY_BYTES);
        self.emit(Op::ArrNew { dst: out, ty: TY_BYTES, len: total, repr: Repr::Prim(PrimTy::U8) }, sp);

        // ---- pass 2: write ----
        let i2 = self.imm_i32(0, sp);
        let w = self.imm_i32(0, sp);
        let l_head2 = self.new_label();
        let l_body2 = self.new_label();
        let l_end2 = self.new_label();
        self.bind(l_head2);
        let cond2 = self.i32_cmp(CmpOp::Lt, i2, n, sp);
        self.br(cond2, l_body2, l_end2);
        self.bind(l_body2);
        let cp2 = {
            let ch = self.str_char_at(src, i2, sp);
            self.to_u32(ch, PrimTy::Char, sp)
        };
        self.emit_utf8_write(cp2, out, w, sp);
        self.inc_i32(i2, sp);
        self.jmp(l_head2);
        self.bind(l_end2);

        out
    }

    /// `if cp > imm { dst += 1 }` (unsigned `cp`).
    fn add_if_gt_u32(&mut self, cp: u16, imm: u32, dst: u16, sp: u32) {
        let k = self.imm_u32(imm, sp);
        let gt = self.u32_cmp(CmpOp::Gt, cp, k, sp);
        let l_add = self.new_label();
        let l_skip = self.new_label();
        self.br(gt, l_add, l_skip);
        self.bind(l_add);
        self.inc_i32(dst, sp);
        self.bind(l_skip);
    }

    /// The 1–4 UTF-8 octets for scalar `cp`, written at `out[w]`, advancing
    /// `w`.
    fn emit_utf8_write(&mut self, cp: u16, out: u16, w: u16, sp: u32) {
        let l_ascii = self.new_label();
        let l_check2 = self.new_label();
        let l_two = self.new_label();
        let l_check3 = self.new_label();
        let l_three = self.new_label();
        let l_four = self.new_label();
        let l_done = self.new_label();

        let k80 = self.imm_u32(0x80, sp);
        let c1 = self.u32_cmp(CmpOp::Lt, cp, k80, sp);
        self.br(c1, l_ascii, l_check2);

        self.bind(l_ascii);
        self.store_byte(out, w, cp, sp);
        self.jmp(l_done);

        self.bind(l_check2);
        let k800 = self.imm_u32(0x800, sp);
        let c2 = self.u32_cmp(CmpOp::Lt, cp, k800, sp);
        self.br(c2, l_two, l_check3);

        self.bind(l_two);
        let b0 = {
            let s = self.u32_shr(cp, 6, sp);
            self.u32_or_imm(s, 0xC0, sp)
        };
        self.store_byte(out, w, b0, sp);
        let b1 = {
            let m = self.u32_and_imm(cp, 0x3F, sp);
            self.u32_or_imm(m, 0x80, sp)
        };
        self.store_byte(out, w, b1, sp);
        self.jmp(l_done);

        self.bind(l_check3);
        let k10000 = self.imm_u32(0x10000, sp);
        let c3 = self.u32_cmp(CmpOp::Lt, cp, k10000, sp);
        self.br(c3, l_three, l_four);

        self.bind(l_three);
        let b0 = {
            let s = self.u32_shr(cp, 12, sp);
            self.u32_or_imm(s, 0xE0, sp)
        };
        self.store_byte(out, w, b0, sp);
        let b1 = {
            let s = self.u32_shr(cp, 6, sp);
            let m = self.u32_and_imm(s, 0x3F, sp);
            self.u32_or_imm(m, 0x80, sp)
        };
        self.store_byte(out, w, b1, sp);
        let b2 = {
            let m = self.u32_and_imm(cp, 0x3F, sp);
            self.u32_or_imm(m, 0x80, sp)
        };
        self.store_byte(out, w, b2, sp);
        self.jmp(l_done);

        self.bind(l_four);
        let b0 = {
            let s = self.u32_shr(cp, 18, sp);
            self.u32_or_imm(s, 0xF0, sp)
        };
        self.store_byte(out, w, b0, sp);
        let b1 = {
            let s = self.u32_shr(cp, 12, sp);
            let m = self.u32_and_imm(s, 0x3F, sp);
            self.u32_or_imm(m, 0x80, sp)
        };
        self.store_byte(out, w, b1, sp);
        let b2 = {
            let s = self.u32_shr(cp, 6, sp);
            let m = self.u32_and_imm(s, 0x3F, sp);
            self.u32_or_imm(m, 0x80, sp)
        };
        self.store_byte(out, w, b2, sp);
        let b3 = {
            let m = self.u32_and_imm(cp, 0x3F, sp);
            self.u32_or_imm(m, 0x80, sp)
        };
        self.store_byte(out, w, b3, sp);

        self.bind(l_done);
    }

    // ---- bytes_decode ----

    /// `bytes_decode(b) -> str` — lossy UTF-8 decode, one char per code
    /// point (`Nat::Str` + `Nat::Concat`).
    pub(crate) fn emit_bytes_decode(&mut self, src: u16, sp: u32) -> u16 {
        let out = self.new_reg(TY_STR);
        let k = self.konst(ConstVal::Str(String::new()));
        self.emit(Op::Const { dst: out, k: k as u32 }, sp);
        let n = self.new_reg(TY_I32);
        { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::ArrLen, recv: src, argv_off, argc, dst: n }, sp); }
        let i = self.imm_i32(0, sp);

        let l_head = self.new_label();
        let l_body = self.new_label();
        let l_end = self.new_label();
        self.bind(l_head);
        let cond = self.i32_cmp(CmpOp::Lt, i, n, sp);
        self.br(cond, l_body, l_end);
        self.bind(l_body);

        let cp = self.new_reg(TY_U32);
        let adv = self.new_reg(TY_I32);
        self.emit_decode_one(src, i, n, cp, adv, sp);

        let c = self.new_reg(TY_CHAR);
        self.emit(Op::Conv { dst: c, src: cp, from: PrimTy::U32, to: PrimTy::Char }, sp);
        let cs = self.new_reg(TY_STR);
        { let (argv_off, argc) = self.pool_args(&(vec![c])); self.emit(Op::CallNat { nat: Nat::Str, recv: NOREG, argv_off, argc, dst: cs }, sp); }
        { let (argv_off, argc) = self.pool_args(&(vec![out, cs])); self.emit(Op::CallNat { nat: Nat::Concat, recv: NOREG, argv_off, argc, dst: out }, sp); }

        self.emit(arith(ArithOp::Add, PrimTy::I32, i, i, adv), sp);
        self.jmp(l_head);
        self.bind(l_end);
        out
    }

    /// Decode one code point at `src[i]` into `cp` (u32) and `adv` (i32).
    /// Invalid input writes U+FFFD and advances one octet.
    fn emit_decode_one(&mut self, src: u16, i: u16, n: u16, cp: u16, adv: u16, sp: u32) {
        let b0 = self.arr_get_u8(src, i, sp);
        let b0u = self.to_u32(b0, PrimTy::U8, sp);

        let l_ascii = self.new_label();
        let l_bad = self.new_label();
        let l_two = self.new_label();
        let l_three = self.new_label();
        let l_four = self.new_label();
        let l_done = self.new_label();

        let k80 = self.imm_u32(0x80, sp);
        let c1 = self.u32_cmp(CmpOp::Lt, b0u, k80, sp);
        let l_c1 = self.new_label();
        self.br(c1, l_ascii, l_c1);

        self.bind(l_c1);
        let kc0 = self.imm_u32(0xC0, sp);
        let c2 = self.u32_cmp(CmpOp::Lt, b0u, kc0, sp);
        let l_c2 = self.new_label();
        self.br(c2, l_bad, l_c2);

        self.bind(l_c2);
        let ke0 = self.imm_u32(0xE0, sp);
        let c3 = self.u32_cmp(CmpOp::Lt, b0u, ke0, sp);
        let l_c3 = self.new_label();
        self.br(c3, l_two, l_c3);

        self.bind(l_c3);
        let kf0 = self.imm_u32(0xF0, sp);
        let c4 = self.u32_cmp(CmpOp::Lt, b0u, kf0, sp);
        let l_c4 = self.new_label();
        self.br(c4, l_three, l_c4);

        self.bind(l_c4);
        let kf8 = self.imm_u32(0xF8, sp);
        let c5 = self.u32_cmp(CmpOp::Lt, b0u, kf8, sp);
        self.br(c5, l_four, l_bad);

        self.bind(l_ascii);
        self.mov(cp, b0u, sp);
        let one = self.imm_i32(1, sp);
        self.mov(adv, one, sp);
        self.jmp(l_done);

        self.bind(l_two);
        self.decode_multi(src, i, n, b0u, 2, l_bad, l_done, cp, adv, sp);
        self.bind(l_three);
        self.decode_multi(src, i, n, b0u, 3, l_bad, l_done, cp, adv, sp);
        self.bind(l_four);
        self.decode_multi(src, i, n, b0u, 4, l_bad, l_done, cp, adv, sp);

        self.bind(l_bad);
        let repl = self.imm_u32(0xFFFD, sp);
        self.mov(cp, repl, sp);
        let one = self.imm_i32(1, sp);
        self.mov(adv, one, sp);
        self.jmp(l_done);

        self.bind(l_done);
    }

    /// Read the `k-1` continuation octets of a `k`-byte sequence, validate
    /// them, set `cp` and `adv = k`; on failure jump to `l_bad`.
    #[allow(clippy::too_many_arguments)]
    fn decode_multi(
        &mut self,
        src: u16,
        i: u16,
        n: u16,
        b0u: u16,
        k: u32,
        l_bad: u32,
        l_done: u32,
        cp: u16,
        adv: u16,
        sp: u32,
    ) {
        let lead_mask = match k {
            2 => 0x1F,
            3 => 0x0F,
            _ => 0x07,
        };
        let mut acc = self.u32_and_imm(b0u, lead_mask, sp);
        acc = self.u32_shl(acc, 6 * (k - 1), sp);
        for j in 1..k {
            let idx = self.add_i32(i, j as i64, sp);
            let in_bounds = self.i32_cmp(CmpOp::Lt, idx, n, sp);
            let l_read = self.new_label();
            self.br(in_bounds, l_read, l_bad);
            self.bind(l_read);
            let bj = self.arr_get_u8(src, idx, sp);
            let bju = self.to_u32(bj, PrimTy::U8, sp);
            let cont = self.is_cont(bju, sp);
            let l_ok = self.new_label();
            self.br(cont, l_ok, l_bad);
            self.bind(l_ok);
            let m = self.u32_and_imm(bju, 0x3F, sp);
            let shifted = self.u32_shl(m, 6 * (k - 1 - j), sp);
            acc = self.u32_or(acc, shifted, sp);
        }
        self.mov(cp, acc, sp);
        let kk = self.imm_i32(k as i64, sp);
        self.mov(adv, kk, sp);
        self.jmp(l_done);
    }
}
