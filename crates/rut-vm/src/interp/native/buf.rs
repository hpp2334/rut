//! The `StrBuf` builder natives (json-perf phase 2): the growable
//! string accumulator — appends mutate the builder's own block in place
//! (geometric growth, the amortized-O(1) accumulator path), `finish`
//! copies the octets out to a fresh immutable `str` cell once. This is
//! the general machinery any tokenizer/encoder's output side wants; the
//! engine never learns what is being built.
use super::*;

impl Vm {
    pub(super) fn call_buf_nat(&mut self, nat: Nat, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        match nat {
            Nat::StrBufNew => {
                // StrBuf(cap) — one empty UTF-8 buffer pre-sized to `cap`
                // octets (the pre-sizing the accumulator shapes could
                // never spell); the block store class-rounds the hint.
                let cap = unsafe { self.reg(args[0]).i };
                if cap < 0 {
                    return Err(Trap::new(TrapKind::Invalid, format!("StrBuf(cap): capacity must be >= 0, got {cap}")));
                }
                let c = self.heap.alloc_str_buf(cap as usize)?;
                self.store_result(dst, c)?;
            }
            Nat::StrBufPush => {
                // b.push(s) — append a str's octets in place. The piece
                // is a different cell from the builder (the checker
                // types the argument `str`), so borrowing its bytes
                // across the append is sound.
                let b = self.reg(recv.unwrap());
                if !matches!(&cell_of(b).data, CellData::StrBuf { .. }) {
                    return Err(Trap::new(TrapKind::Invalid, "push on non-builder"));
                }
                let piece = cell_of(self.reg(args[0]));
                if !matches!(&piece.data, CellData::Str(_) | CellData::StrView { .. }) {
                    return Err(Trap::new(TrapKind::Invalid, "push needs a string"));
                }
                let chars = piece.char_len();
                let bytes = piece.as_bytes();
                self.heap.str_buf_append(b, bytes, chars)?;
            }
            Nat::StrBufPushCode => {
                // b.push_code(c) — append one codepoint. Invalid scalars
                // (surrogates) mint U+FFFD — the `str.from_code` rule.
                let b = self.reg(recv.unwrap());
                if !matches!(&cell_of(b).data, CellData::StrBuf { .. }) {
                    return Err(Trap::new(TrapKind::Invalid, "push_code on non-builder"));
                }
                let cp = unsafe { self.reg(args[0]).i } as u32;
                let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                let mut buf = [0u8; 4];
                let enc = ch.encode_utf8(&mut buf).as_bytes();
                self.heap.str_buf_append(b, enc, 1)?;
            }
            Nat::StrBufLen => {
                // b.len() — the codepoint count (tracked at append, O(1))
                let b = self.reg(recv.unwrap());
                let n = match &cell_of(b).data {
                    CellData::StrBuf { chars, .. } => *chars as i64,
                    _ => return Err(Trap::new(TrapKind::Invalid, "len on non-builder")),
                };
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(n);
                }
            }
            Nat::StrBufFinish => {
                // b.finish() — the ONE materialization: a fresh immutable
                // `str` cell with the builder's octets; the builder keeps
                // its buffer (finish twice answers the same text).
                let b = self.reg(recv.unwrap());
                let bytes = self.heap.str_buf_bytes(b)?;
                let c = self.heap.alloc_str_bytes(bytes)?;
                self.store_result(dst, c)?;
            }
            _ => unreachable!("call_buf_nat: non-builder native"),
        }
        Ok(())
    }
}
