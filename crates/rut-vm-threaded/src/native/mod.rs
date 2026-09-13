//! Native backend — `become`-threaded dispatch.
//!
//! Each handler performs one op via the shared [`Machine`] method, then
//! **tail-calls** the next op's handler. `become` guarantees the caller's
//! stack frame is replaced by the callee's, so a long chain never grows the
//! stack and hot state stays in registers (`extern "rust-preserve-none"`).
//!
//! Phase 0: three synthetic handlers (`inc`/`dec`/`halt`) exercising the
//! indirect tail jump `become (*table.add(tag))(…)`.

use crate::api::{Handler, Machine};

extern "rust-preserve-none" fn h_inc<M: Machine>(
    m: *mut M,
    table: *const (),
    pc: u32,
) -> Result<M::Out, M::Err> {
    let next = unsafe { (*m).inc(pc)? };
    let tag = unsafe { (*m).tag_at(next) } as usize;
    let h = unsafe { *(table as *const Handler<M>).add(tag) };
    become h(m, table, next)
}

extern "rust-preserve-none" fn h_dec<M: Machine>(
    m: *mut M,
    table: *const (),
    pc: u32,
) -> Result<M::Out, M::Err> {
    let next = unsafe { (*m).dec(pc)? };
    let tag = unsafe { (*m).tag_at(next) } as usize;
    let h = unsafe { *(table as *const Handler<M>).add(tag) };
    become h(m, table, next)
}

extern "rust-preserve-none" fn h_halt<M: Machine>(
    m: *mut M,
    _table: *const (),
    pc: u32,
) -> Result<M::Out, M::Err> {
    Ok(unsafe { (*m).halt(pc) })
}

/// Run from `pc` until a handler returns (root halt or trap).
///
/// The handler table lives on this frame, which stays alive for the whole
/// chain — handlers tail-call each other but never pop past `run`.
pub fn run<M: Machine>(m: &mut M, pc: u32) -> Result<M::Out, M::Err> {
    let table: [Handler<M>; 3] = [h_inc::<M>, h_dec::<M>, h_halt::<M>];
    let tp = table.as_ptr() as *const ();
    let mp = m as *mut M;
    let tag = unsafe { (*mp).tag_at(pc) } as usize;
    let h = unsafe { *(tp as *const Handler<M>).add(tag) };
    h(mp, tp, pc)
}
