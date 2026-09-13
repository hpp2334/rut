//! The backend-neutral contract between the dispatch engine and the VM.
//!
//! `rut-vm` implements [`Machine`]; `native/` (threaded) and `wasm/` (loop)
//! drive it. The trait is generic over the VM's result / error / word types
//! via associated types, so this crate never names `Value`, `Trap`, or
//! `Slot`.
//!
//! **State is threaded as arguments.** A handler receives the code base, the
//! tag base, the register base, the pc, and the fuel counters, so the hot
//! loop touches no `self` memory and reads the current op directly. This is
//! the shape the reference tail-call interpreters use (and what the
//! `dispatch_bench` here measures as ~25% faster than a match loop).
//!
//! Unthreaded ops map to [`T_SLOW`]; `h_slow` syncs the machine state and
//! returns [`ThreadOut::Bail`] so the VM's match interpreter runs that op.

use rut_core::ops::Op;

// ---- op tags ----

pub const T_SLOW: u8 = 0;
pub const T_MOV: u8 = 1;
pub const T_CONSTRAW: u8 = 2;
pub const T_NOT: u8 = 3;
pub const T_ADDI: u8 = 4;
pub const T_SUBI: u8 = 5;
pub const T_WADDI: u8 = 6;
pub const T_WSUBI: u8 = 7;
pub const T_WMULI: u8 = 8;
pub const T_LTI: u8 = 9;
pub const T_ADDF: u8 = 10;
pub const T_SUBF: u8 = 11;
pub const T_MULF: u8 = 12;
pub const T_BR: u8 = 13;
pub const T_JMP: u8 = 14;
pub const T_LOOPHEAD: u8 = 15;
pub const T_ARRGET: u8 = 16;
pub const T_ARRSET: u8 = 17;
pub const T_ARRGETF: u8 = 18;
pub const T_ARRSETF: u8 = 19;
pub const T_GETF: u8 = 20;
pub const T_SETF: u8 = 21;
pub const T_CALL: u8 = 22;
pub const T_CALLM: u8 = 23;
pub const T_CALLI: u8 = 24;
pub const T_CALLFN: u8 = 25;
pub const T_CALLNAT: u8 = 26;
pub const T_RET: u8 = 27;
pub const T_MULI: u8 = 28;
pub const T_DIVI: u8 = 29;
pub const T_MODI: u8 = 30;
pub const T_WDIVI: u8 = 31;
pub const T_WMODI: u8 = 32;
pub const T_ANDI: u8 = 33;
pub const T_ORI: u8 = 34;
pub const T_XORI: u8 = 35;
pub const T_SHLI: u8 = 36;
pub const T_SHRI: u8 = 37;
pub const T_WRAPSHLI: u8 = 38;
pub const T_EQI: u8 = 39;
pub const T_NEI: u8 = 40;
pub const T_GTI: u8 = 41;
pub const T_LEI: u8 = 42;
pub const T_GEI: u8 = 43;
pub const T_NEGI: u8 = 44;
pub const T_DIVF: u8 = 45;
pub const T_MODF: u8 = 46;
pub const T_NEGF: u8 = 47;
pub const T_EQF: u8 = 48;
pub const T_NEF: u8 = 49;
pub const T_LTF: u8 = 50;
pub const T_GTF: u8 = 51;
pub const T_LEF: u8 = 52;
pub const T_GEF: u8 = 53;
pub const NTAGS: usize = 54;

pub fn tag_of(op: &Op) -> u8 {
    match op {
        Op::Mov { .. } => T_MOV,
        Op::ConstRaw { .. } => T_CONSTRAW,
        Op::Not { .. } => T_NOT,
        Op::AddI { .. } => T_ADDI,
        Op::SubI { .. } => T_SUBI,
        Op::WAddI { .. } => T_WADDI,
        Op::WSubI { .. } => T_WSUBI,
        Op::WMulI { .. } => T_WMULI,
        Op::LtI { .. } => T_LTI,
        Op::AddF { .. } => T_ADDF,
        Op::SubF { .. } => T_SUBF,
        Op::MulF { .. } => T_MULF,
        Op::Br { .. } => T_BR,
        Op::Jmp { .. } => T_JMP,
        Op::LoopHead => T_LOOPHEAD,
        Op::ArrGet { .. } => T_ARRGET,
        Op::ArrSet { .. } => T_ARRSET,
        Op::ArrGetF { .. } => T_ARRGETF,
        Op::ArrSetF { .. } => T_ARRSETF,
        Op::GetF { .. } => T_GETF,
        Op::SetF { .. } => T_SETF,
        Op::Call { .. } => T_CALL,
        Op::CallM { .. } => T_CALLM,
        Op::CallI { .. } => T_CALLI,
        Op::CallFn { .. } => T_CALLFN,
        Op::CallNat { .. } => T_CALLNAT,
        Op::Ret { .. } => T_RET,
        Op::MulI { .. } => T_MULI,
        Op::DivI { .. } => T_DIVI,
        Op::ModI { .. } => T_MODI,
        Op::WDivI { .. } => T_WDIVI,
        Op::WModI { .. } => T_WMODI,
        Op::AndI { .. } => T_ANDI,
        Op::OrI { .. } => T_ORI,
        Op::XorI { .. } => T_XORI,
        Op::ShlI { .. } => T_SHLI,
        Op::ShrI { .. } => T_SHRI,
        Op::WrapShlI { .. } => T_WRAPSHLI,
        Op::EqI { .. } => T_EQI,
        Op::NeI { .. } => T_NEI,
        Op::GtI { .. } => T_GTI,
        Op::LeI { .. } => T_LEI,
        Op::GeI { .. } => T_GEI,
        Op::NegI { .. } => T_NEGI,
        Op::DivF { .. } => T_DIVF,
        Op::ModF { .. } => T_MODF,
        Op::NegF { .. } => T_NEGF,
        Op::EqF { .. } => T_EQF,
        Op::NeF { .. } => T_NEF,
        Op::LtF { .. } => T_LTF,
        Op::GtF { .. } => T_GTF,
        Op::LeF { .. } => T_LEF,
        Op::GeF { .. } => T_GEF,
        _ => T_SLOW,
    }
}

/// The next step of a threaded op in the current frame.
pub enum Flow<O> {
    Next(u32),
    Done(O),
    /// the frame changed (call/ret): re-read `thread_state` and continue
    Redispatch,
}

/// Result of a threaded stretch.
pub enum ThreadOut<O> {
    Done(O),
    /// hit an unthreaded op; the machine's pc/fuel were synced to it
    Bail,
}

/// A snapshot of the hot frame state, handed to the threaded handlers.
pub struct ThreadState<W> {
    pub code: *const Op,
    pub tags: *const u8,
    pub regs: *mut W,
    pub pc: u32,
    /// `-1` = unlimited; otherwise remaining ops
    pub fuel: i64,
    pub fuel_used: u64,
}

pub trait Machine {
    type Out;
    type Err;
    type Word;

    /// Read the current frame's hot state.
    fn thread_state(&mut self) -> ThreadState<Self::Word>;
    /// Write pc + fuel back (used by `Bail` / `park`).
    fn sync(&mut self, pc: u32, fuel: i64, fuel_used: u64);
    /// Park on fuel exhaustion and return the trap.
    fn park(&mut self, pc: u32, fuel_used: u64) -> Self::Err;
    /// Write the fuel counters back (used by `Redispatch`); the pc is
    /// already in the VM (set by the call/ret body).
    fn sync_fuel(&mut self, fuel: i64, fuel_used: u64) {
        let _ = (fuel, fuel_used);
        unimplemented!("sync_fuel: not threaded by this Machine")
    }

    fn op_mov(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_mov: not threaded by this Machine")
    }
    fn op_constraw(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_constraw: not threaded by this Machine")
    }
    fn op_not(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_not: not threaded by this Machine")
    }
    fn op_addi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_addi: not threaded by this Machine")
    }
    fn op_subi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_subi: not threaded by this Machine")
    }
    fn op_waddi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_waddi: not threaded by this Machine")
    }
    fn op_wsubi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_wsubi: not threaded by this Machine")
    }
    fn op_wmuli(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_wmuli: not threaded by this Machine")
    }
    fn op_lti(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_lti: not threaded by this Machine")
    }
    fn op_addf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_addf: not threaded by this Machine")
    }
    fn op_subf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_subf: not threaded by this Machine")
    }
    fn op_mulf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_mulf: not threaded by this Machine")
    }
    fn op_br(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_br: not threaded by this Machine")
    }
    fn op_jmp(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_jmp: not threaded by this Machine")
    }
    fn op_loophead(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_loophead: not threaded by this Machine")
    }
    fn op_arr_get(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_arr_get: not threaded by this Machine")
    }
    fn op_arr_set(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_arr_set: not threaded by this Machine")
    }
    fn op_arr_get_f(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_arr_get_f: not threaded by this Machine")
    }
    fn op_arr_set_f(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_arr_set_f: not threaded by this Machine")
    }
    fn op_getf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_getf: not threaded by this Machine")
    }
    fn op_setf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_setf: not threaded by this Machine")
    }
    fn op_call(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_call: not threaded by this Machine")
    }
    fn op_call_m(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_call_m: not threaded by this Machine")
    }
    fn op_call_i(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_call_i: not threaded by this Machine")
    }
    fn op_call_fn(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_call_fn: not threaded by this Machine")
    }
    fn op_call_nat(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_call_nat: not threaded by this Machine")
    }
    fn op_ret(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = (op, regs, pc);
        unimplemented!("op_ret: not threaded by this Machine")
    }
    fn op_muli(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_divi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_modi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_wdivi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_wmodi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_andi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_ori(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_xori(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_shli(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_shri(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_wrapshli(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_eqi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_nei(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_gti(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_lei(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_gei(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_negi(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_divf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_modf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_negf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_eqf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_nef(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_ltf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_gtf(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_lef(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
    fn op_gef(&mut self, op: &Op, regs: *mut Self::Word, pc: u32) -> Result<Flow<Self::Out>, Self::Err> { let _ = (op, regs, pc); unimplemented!() }
}

/// A threaded handler: same ABI, arguments, and return type for every op (a
/// `become` requirement).
#[cfg(rut_threaded)]
pub type Handler<M> = extern "rust-preserve-none" fn(
    *mut M,               // machine
    *const (),            // handler table (type-erased)
    *const Op,            // code base
    *const u8,            // tag base
    *mut <M as Machine>::Word, // register base
    u32,                  // pc
    i64,                  // fuel (-1 = unlimited)
    u64,                  // fuel_used
) -> Result<ThreadOut<<M as Machine>::Out>, <M as Machine>::Err>;

/// The op handler table, built once per `Machine` instantiation. On
/// non-threaded targets it carries no data.
#[cfg(rut_threaded)]
pub struct Table<M: Machine> {
    pub(crate) entries: [Handler<M>; NTAGS],
}

#[cfg(not(rut_threaded))]
pub struct Table<M: Machine>(core::marker::PhantomData<fn() -> M>);
