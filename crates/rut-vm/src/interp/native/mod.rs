//! `CallNat` natives (RFC 0032 §1.1 R2), split by domain: `str.rs` holds
//! the string/formatting natives, `array.rs` the sequence length.
use super::*;

mod array;
mod str;

impl Vm {
    // ---- natives (RFC 0032 §1.1 R2: named things are natives, not ops) ----

    pub(super) fn call_nat(&mut self, nat: Nat, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        match nat {
            Nat::ArrLen => self.nat_arr_len(recv, dst),
            Nat::Str | Nat::Concat | Nat::StrLen | Nat::StrJoin | Nat::StrSlice => self.call_str_nat(nat, recv, args, dst),
        }
    }

    /// Read operand register `i` (the natives take `&self` for this).
    pub(super) fn reg(&self, i: Reg) -> Slot {
        self.cur_regs[i as usize]
    }
}
