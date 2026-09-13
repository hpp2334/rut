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
pub const NTAGS: usize = 16;

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
        _ => T_SLOW,
    }
}

/// The next step of a threaded op in the current frame.
pub enum Flow<O> {
    Next(u32),
    Done(O),
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
