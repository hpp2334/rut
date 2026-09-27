//! The async host set's bodies (RFC 0018) — the standard launcher
//! surface the plan's ruling 8 assigns to the embedder:
//!
//! ```rut
//! pub host fn __launch(f: opaque);            // enqueue the frame
//! pub host fn __abort(f: opaque) -> bool;     // cancel flag + re-enqueue
//! pub host fn __sleep(ms: u32) -> opaque;     // mint + seal the sleep future
//! pub host fn __sleep_yield(f: opaque, cx: opaque);  // the sleep future's yield
//! ```
//!
//! Declared by `rut/async_engine/engine.d.rut` (host scope
//! `async_engine`); the `rut/async_host` inline package wraps them in
//! the typed surface (`launch_future`, `LaunchedFutureHandle::abort`,
//! `sleep`). Each embedder mounts the modules AND installs these bodies
//! — a session that mounts neither simply has no launcher, and `await`
//! stays cold-poll inline.
//!
//! Every `opaque` crossing rides the RFC 0014 erasure box: the frames
//! are sealed rut values (`RutOpaque` store entries), so the bodies
//! recover the raw frame slot through [`Vm::opaque_rut_value`] — the
//! `Opaque<T>` host-payload machinery does NOT apply (a rut-side box
//! answers `downcast<T>`, never a Rust `T`).

use rut_vm::interp::{HostRegistry, Vm};
use rut_vm::{OpaqueRef, Slot, Trap, TrapKind};

use rut_core::async_frame as af;

/// The sleep mint's seal type: the rut-side `Future<nil>` OBJECT
/// spelling — `mk_trait_inst` spells the instantiation `Future<nil>`
/// and `mk_trait_obj` interns the object type under `[trait] <that>`,
/// so this exact name is what `sleep`'s `opaque.downcast<Future<nil>>`
/// compares the box against (the TidOf law: a Rut entry answers its
/// recorded `val_ty`).
const FUTURE_NIL_OBJ: &str = "[trait] Future<nil>";

/// The sleep frame type / checkpoint enum / seal type, found by their
/// reserved names (the compiler mints the frame pair the first time
/// `__sleep` is called, the object type when the `sleep` surface
/// compiles).
fn find_ty(vm: &Vm, name: &str) -> Result<u32, Trap> {
    vm.prog
        .types
        .types
        .iter()
        .position(|t| vm.prog.interner.name(t.name) == name)
        .map(|i| i as u32)
        .ok_or_else(|| {
            Trap::new(
                TrapKind::Invalid,
                format!("async engine: `{name}` was not minted — the sleep surface was never compiled"),
            )
        })
}

/// Unbox a sealed rut value (an `opaque` param): the held slot. A
/// non-box or a host-payload box is a caller bug — the rows' rut-side
/// sealers (`opaque(f)`, the compiler's wrapper) always mint Rut
/// entries — so the trap names the crossing instead of decoding blind.
fn unsealed(vm: &Vm, b: &OpaqueRef, what: &str) -> Result<Slot, Trap> {
    vm.opaque_rut_value(Slot { r: b.ptr() })
        .map(|(slot, _ty)| slot)
        .ok_or_else(|| {
            Trap::new(
                TrapKind::Invalid,
                format!("async engine: `{what}` received a box that holds no rut value"),
            )
        })
}

/// Install the async engine's bodies. No sink, no state: everything
/// routes through the VM's driving API (`launch` / `cancel` /
/// `arm_timer`) — the engine owns the queues, the host owns only the
/// crossing.
pub fn install_std_async(hosts: &mut HostRegistry) {
    // launch_future's engine half: unbox the sealed frame (the box the
    // rut launcher minted with `opaque(f)`); the queue takes its own
    // reference
    rut_vm::register!(hosts, "async_engine::__launch", (OpaqueRef,) -> (),
        |vm: &mut Vm, f: OpaqueRef| {
            let frame = unsealed(vm, &f, "__launch")?;
            vm.launch(frame);
            Ok(())
        });
    // LaunchedFutureHandle::abort's engine half: `false` when the frame
    // already retired, else the flag + re-enqueue (the probe at its
    // checkpoint runs the drop path)
    rut_vm::register!(hosts, "async_engine::__abort", (OpaqueRef,) -> bool,
        |vm: &mut Vm, f: OpaqueRef| {
            let frame = unsealed(vm, &f, "__abort")?;
            Ok(vm.cancel(frame))
        });
    // sleep's engine-assisted mint: the sleep frame with `ms` in its
    // local field, SEALED under the `Future<nil>` object spelling — the
    // entry owns the mint reference, the owning handle hands the box's
    // own reference to the return path (`seal_opaque`)
    rut_vm::register!(hosts, "async_engine::__sleep", (u32,) -> OpaqueRef,
        |vm: &mut Vm, ms: u32| {
            let ckpt = find_ty(vm, af::SLEEP_CKPT)?;
            let frame_ty = find_ty(vm, af::SLEEP_FRAME)?;
            let fut_obj = find_ty(vm, FUTURE_NIL_OBJ)?;
            // the minted frame type's own field count (state, cancelled,
            // awaiter, pending, answer, ms) — the ANSWER lane joined the
            // engine layout, so the count reads the type, never a literal
            let nfields = match vm.prog.types.kind(frame_ty) {
                rut_core::types::TyKind::Data { fields } => fields.len(),
                _ => return Err(rut_vm::Trap::new(
                    rut_vm::TrapKind::Invalid,
                    "async engine: the sleep frame type is not a record",
                )),
            };
            let frame = vm.alloc_zeroed_record(frame_ty, nfields)?;
            let s0 = vm.enum_member_slot(ckpt, 0)?;
            vm.set_record_field(frame, af::STATE_FIELD as usize, s0);
            vm.set_record_field(frame, af::LOCALS_BASE as usize, Slot::int(ms as i64));
            Ok(vm.seal_opaque(frame, fut_obj)?)
        });
    // the sleep future's yield, engine-backed: the fresh arm arms the
    // timer and parks at s1; the resumed arm (or a fresh arm already
    // cancelled) retires — sleep's drop path has no locals to run. The
    // wire is sealed (the compiler's yield wrapper boxes the raw
    // frame/cx pair the shared ABI passes); the cx is the engine-minted
    // record and the body needs none of it.
    rut_vm::register!(hosts, "async_engine::__sleep_yield", (OpaqueRef, OpaqueRef) -> (),
        |vm: &mut Vm, f: OpaqueRef, _cx: OpaqueRef| {
            let ckpt = find_ty(vm, af::SLEEP_CKPT)?;
            let frame = unsealed(vm, &f, "__sleep_yield")?;
            let cancelled = vm
                .record_field(frame, af::CANCELLED_FIELD as usize)
                .map(|s| s.as_bool())
                .unwrap_or(false);
            let state = vm
                .record_field(frame, af::STATE_FIELD as usize)
                .unwrap_or(Slot::null());
            let fresh = !unsafe { state.r.is_null() }
                && vm
                    .enum_member_slot(ckpt, 0)
                    .ok()
                    .map(|s0| Slot::same_ref(state, s0))
                    .unwrap_or(false);
            if fresh && !cancelled {
                let ms = vm
                    .record_field(frame, af::LOCALS_BASE as usize)
                    .map(|s| unsafe { s.i } as u64)
                    .unwrap_or(0);
                let s1 = vm.enum_member_slot(ckpt, 1)?;
                vm.arm_timer(vm.now_ms() + ms, frame);
                vm.set_record_field(frame, af::STATE_FIELD as usize, s1);
            } else {
                // resumed at s1 (timer fired), cancelled at the checkpoint,
                // or already retired: done either way
                vm.set_record_field(frame, af::STATE_FIELD as usize, Slot::null());
            }
            Ok(())
        });
}
