//! The async host set's bodies (RFC 0018) — the standard launcher
//! surface the plan's ruling 8 assigns to the embedder:
//!
//! ```rut
//! pub host fn __launch(f: any);            // enqueue the frame
//! pub host fn __abort(f: any) -> bool;     // cancel flag + re-enqueue
//! pub host fn __sleep(ms: u32) -> any;     // mint the sleep future
//! pub host fn __sleep_yield(f: any, cx: any);  // the sleep future's yield
//! ```
//!
//! Declared by `rut/async_engine/engine.d.rut` (host scope
//! `async_engine`); the `rut/async_host` inline package wraps them in
//! the typed surface (`launch_future`, `LaunchedFutureHandle::abort`,
//! `sleep`). Each embedder mounts the modules AND installs these bodies
//! — a session that mounts neither simply has no launcher, and `await`
//! stays cold-poll inline.

use rut_vm::interp::{HostRegistry, HostVal, Vm};
use rut_vm::{Slot, Trap, TrapKind, ValSlot};

use rut_core::async_frame as af;

/// The sleep frame type / checkpoint enum, found by their reserved
/// names (the compiler mints them the first time `__sleep` is called).
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
                format!("async engine: `{name}` was not minted — `sleep` was never compiled"),
            )
        })
}

/// Install the async engine's bodies. No sink, no state: everything
/// routes through the VM's driving API (`launch` / `cancel` /
/// `arm_timer`) — the engine owns the queues, the host owns only the
/// crossing.
pub fn install_std_async(hosts: &mut HostRegistry) {
    // launch_future's engine half: the frame slot crosses as its own
    // cell (the any-arg law); the queue takes its own reference
    rut_vm::register!(hosts, "async_engine::__launch", (HostVal,) -> (),
        |vm: &mut Vm, f: HostVal| {
            vm.launch(f.slot);
            Ok(())
        });
    // LaunchedFutureHandle::abort's engine half: `false` when the frame
    // already retired, else the flag + re-enqueue (the probe at its
    // checkpoint runs the drop path)
    rut_vm::register!(hosts, "async_engine::__abort", (HostVal,) -> bool,
        |vm: &mut Vm, f: HostVal| Ok(vm.cancel(f.slot)));
    // sleep's engine-assisted mint: the sleep frame with `ms` in its
    // local field; the slot crosses as the any-answer (raw, no box)
    rut_vm::register!(hosts, "async_engine::__sleep", (u32,) -> Option<ValSlot>,
        |vm: &mut Vm, ms: u32| {
            let ckpt = find_ty(vm, af::SLEEP_CKPT)?;
            let frame_ty = find_ty(vm, af::SLEEP_FRAME)?;
            let frame = vm.alloc_zeroed_record(frame_ty, 5)?;
            let s0 = vm.enum_member_slot(ckpt, 0)?;
            vm.set_record_field(frame, af::STATE_FIELD as usize, s0);
            vm.set_record_field(frame, af::LOCALS_BASE as usize, Slot::int(ms as i64));
            Ok(Some(ValSlot::Ref(frame)))
        });
    // the sleep future's yield, engine-backed: the fresh arm arms the
    // timer and parks at s1; the resumed arm (or a fresh arm already
    // cancelled) retires — sleep's drop path has no locals to run
    rut_vm::register!(hosts, "async_engine::__sleep_yield", (HostVal, HostVal) -> (),
        |vm: &mut Vm, f: HostVal, _cx: HostVal| {
            let ckpt = find_ty(vm, af::SLEEP_CKPT)?;
            let frame = f.slot;
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
