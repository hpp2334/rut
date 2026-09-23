//! `CallNat` natives (RFC 0032 §1.1 R2), split by domain: `str.rs` holds
//! the string/formatting natives, `array.rs` the sequence length/window,
//! `bytes.rs` the buffer copy, `trace.rs` the StackTrace snapshot
//! (RFC 0036).
use super::*;

mod array;
mod buf;
mod bytes;
mod str;
mod trace;

impl Vm {
    // ---- natives (RFC 0032 §1.1 R2: named things are natives, not ops) ----

    pub(super) fn call_nat(&mut self, nat: Nat, recv: Reg, argv_off: u32, argc: u16, dst: Reg) -> Result<(), Trap> {
        let prog = std::rc::Rc::clone(&self.prog);
        let args = self.cur_argv(&prog, argv_off, argc);
        let (recv, dst) = (reg_opt(recv), reg_opt(dst));
        match nat {
            Nat::ArrLen => self.nat_arr_len(recv, dst),
            Nat::Str | Nat::Concat | Nat::StrLen | Nat::StrJoin | Nat::StrSlice | Nat::StrScan | Nat::StrStartsWith => self.call_str_nat(nat, recv, args, dst),
            Nat::StrBufNew | Nat::StrBufPush | Nat::StrBufPushCode | Nat::StrBufLen | Nat::StrBufFinish => self.call_buf_nat(nat, recv, args, dst),
            Nat::ArrSlice => self.nat_arr_slice(recv, args, dst),
            Nat::BytesClone => self.nat_bytes_clone(recv, dst),
            Nat::CaptureTrace => self.nat_capture_trace(dst),
            Nat::TraceLen => self.nat_trace_len(recv, dst),
            Nat::TraceName => self.nat_trace_name(recv, args, dst),
            Nat::TraceLine => self.nat_trace_line(recv, args, dst),
            Nat::TraceCol => self.nat_trace_col(recv, args, dst),
            Nat::TraceRender => self.nat_trace_render(recv, dst),
        }
    }

    /// Read operand register `i` (the natives take `&self` for this).
    pub(super) fn reg(&self, i: Reg) -> Slot {
        self.cur_regs[i as usize]
    }
}
