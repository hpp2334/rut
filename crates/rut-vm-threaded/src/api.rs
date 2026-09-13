//! The backend-neutral contract between the dispatch engine and the VM.
//!
//! `rut-vm` implements [`Machine`]; the engine in `native/` or `wasm/`
//! drives it. The trait is generic over the VM's word / result / error
//! types via associated types, so this crate never names `Slot`, `Value`,
//! or `Trap`.
//!
//! Phase 0 is a synthetic tag machine used to validate the mechanism
//! (`become` + `rust-preserve-none`, flat stack, indirect tail jumps)
//! before the real per-op surface is generated. Tags are intentionally
//! tiny: `0` = inc, `1` = dec, anything else = halt.

pub trait Machine {
    /// The result of a completed run (`Value`).
    type Out;
    /// The trap type (`Trap`).
    type Err;

    /// The op tag at `pc` in the current code stream.
    fn tag_at(&self, pc: u32) -> u8;

    /// `inc`: mutate state, return the next `pc`.
    fn inc(&mut self, pc: u32) -> Result<u32, Self::Err>;
    /// `dec`: mutate state, return the next `pc`.
    fn dec(&mut self, pc: u32) -> Result<u32, Self::Err>;
    /// `halt`: finish the run.
    fn halt(&mut self, pc: u32) -> Self::Out;
}

/// A threaded handler: same ABI, arguments, and return type for every op
/// (a `become` requirement). `table` is an opaque pointer to the op handler
/// table (`*const [Handler<M>]`, type-erased to `*const ()` so the alias is
/// not recursive), so every handler can tail-jump to the next op's handler.
#[cfg(rut_threaded)]
pub type Handler<M> = extern "rust-preserve-none" fn(
    *mut M,
    *const (),
    u32,
) -> Result<<M as Machine>::Out, <M as Machine>::Err>;
