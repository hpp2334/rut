//! The backend-neutral contract between the dispatch engine and the VM.
//!
//! `rut-vm` implements [`Machine`]; `native/` (threaded) and `wasm/` (loop)
//! drive it. The trait is generic over the VM's result / error types via
//! associated types, so this crate never names `Value` or `Trap`.
//!
//! Dispatch is **tag-based**: [`tag_of`] maps an [`Op`] to a small dense tag.
//! Only ops the engine knows how to thread get their own tag; everything
//! else maps to [`T_SLOW`]. The native handlers tail-jump between threaded
//! ops and return [`ThreadOut::Bail`] on a slow op, at which point the VM's
//! match interpreter runs that one op and re-enters threading. The wasm
//! backend runs the same op methods in a loop and bails the same way.
//!
//! Scalar op **behavior lives only in the `Machine::op_*` methods**, so both
//! backends share it verbatim.

use rut_core::ops::Op;

// ---- op tags (dense; `T_SLOW` catches everything unthreaded) ----

/// Any op the engine does not thread: bail to the VM's match interpreter.
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
/// Number of tag slots (the handler table is indexed by tag).
pub const NTAGS: usize = 16;

/// Map an op to its dispatch tag. Ops not listed here are unthreaded and
/// take the `T_SLOW` bail path.
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
    /// continue at this pc (same frame)
    Next(u32),
    /// the run finished (root `ret`)
    Done(O),
}

/// Result of a threaded stretch.
pub enum ThreadOut<O> {
    /// the run finished
    Done(O),
    /// hit an unthreaded op; `Machine::pc()` is left at that op
    Bail,
}

pub trait Machine {
    /// The run's result (`Value`).
    type Out;
    /// The trap type (`Trap`).
    type Err;

    /// The current frame's pc.
    fn pc(&self) -> u32;
    /// Set the current frame's pc (used by `Bail`).
    fn set_pc(&mut self, pc: u32);
    /// The op at `pc` in the current frame.
    fn op_at(&self, pc: u32) -> &Op;
    /// The dispatch tag at `pc`. The default classifies the op each time;
    /// implementors with a precomputed tag stream override this.
    fn op_tag(&self, pc: u32) -> u8 {
        tag_of(self.op_at(pc))
    }
    /// Account for one op and enforce fuel (RFC 0040); parks at `pc` on
    /// exhaustion.
    fn tick(&mut self, pc: u32) -> Result<(), Self::Err>;

    fn op_mov(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_mov: not threaded by this Machine")
    }
    fn op_constraw(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_constraw: not threaded by this Machine")
    }
    fn op_not(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_not: not threaded by this Machine")
    }
    fn op_addi(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_addi: not threaded by this Machine")
    }
    fn op_subi(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_subi: not threaded by this Machine")
    }
    fn op_waddi(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_waddi: not threaded by this Machine")
    }
    fn op_wsubi(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_wsubi: not threaded by this Machine")
    }
    fn op_wmuli(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_wmuli: not threaded by this Machine")
    }
    fn op_lti(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_lti: not threaded by this Machine")
    }
    fn op_addf(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_addf: not threaded by this Machine")
    }
    fn op_subf(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_subf: not threaded by this Machine")
    }
    fn op_mulf(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_mulf: not threaded by this Machine")
    }
    fn op_br(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_br: not threaded by this Machine")
    }
    fn op_jmp(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_jmp: not threaded by this Machine")
    }
    fn op_loophead(&mut self, pc: u32) -> Result<Flow<Self::Out>, Self::Err> {
        let _ = pc;
        unimplemented!("op_loophead: not threaded by this Machine")
    }
}

/// A threaded handler: same ABI, arguments, and return type for every op
/// (a `become` requirement). `table` is the op handler table type-erased to
/// `*const ()` so this alias is not recursive.
#[cfg(rut_threaded)]
pub type Handler<M> = extern "rust-preserve-none" fn(
    *mut M,
    *const (),
    u32,
) -> Result<ThreadOut<<M as Machine>::Out>, <M as Machine>::Err>;

/// The op handler table, built once per `Machine` instantiation and reused
/// across every `run` (so a bail never rebuilds it). On non-threaded targets
/// it carries no data — the loop backend dispatches directly.
#[cfg(rut_threaded)]
pub struct Table<M: Machine> {
    pub(crate) entries: [Handler<M>; NTAGS],
}

#[cfg(not(rut_threaded))]
pub struct Table<M: Machine>(core::marker::PhantomData<fn() -> M>);
