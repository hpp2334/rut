//! Portable backend — a loop over the same [`Machine`] methods.
//!
//! Used on wasm and any target without verified `become` lowering. The op
//! bodies are identical to the native backend's; only the dispatch differs
//! (a `loop`/`match` instead of tail jumps), which is what the wasm JITs
//! want.
//!
//! Phase 0: mirrors the native backend's three synthetic ops.

use crate::api::Machine;

pub fn run<M: Machine>(m: &mut M, pc: u32) -> Result<M::Out, M::Err> {
    let mut pc = pc;
    loop {
        match m.tag_at(pc) {
            0 => pc = m.inc(pc)?,
            1 => pc = m.dec(pc)?,
            _ => return Ok(m.halt(pc)),
        }
    }
}
