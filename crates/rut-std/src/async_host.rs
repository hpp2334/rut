//! The async host set's bodies — the standard launcher
//! surface the plan's ruling 8 assigns to the embedder
//! (the async model: `docs/src/reference/async.md`;
//! cancellation/tasks: `docs/src/reference/tasks.md`):
//!
//! ```rut
//! pub host fn __launch(f: opaque);            // enqueue the frame
//! pub host fn __abort(f: opaque) -> bool;     // cancel flag + re-enqueue
//! pub host fn __sleep(ms: u32) -> opaque;     // mint + seal the sleep future
//! pub host fn __sleep_yield(f: opaque, cx: opaque);  // the sleep future's yield
//! pub host fn __select2(a: opaque, b: opaque) -> opaque;      // race mint
//! pub host fn __select2_yield(f: opaque, cx: opaque);         // arm / arbitrate
//! pub host fn __select_all(cohort: opaque) -> opaque;         // cohort race mint
//! pub host fn __select_all_yield(f: opaque, cx: opaque);      // arm / arbitrate
//! pub host fn __completer(ty: u32) -> opaque;                 // manual mint
//! pub host fn __resolve(f: opaque, v: opaque) -> bool;        // settle + wake
//! pub host fn __completer_yield(f: opaque, cx: opaque);       // cancelled retire
//! ```
//!
//! Declared by `rut/async_engine/engine.d.rut` (scope `async_engine`);
//! the `rut/async_host` inline package wraps them in
//! the typed surface (`launch_future`, `LaunchedFutureHandle::abort`,
//! `sleep`). Each embedder mounts the modules AND installs these bodies
//! — a session that mounts neither simply has no launcher, and `await`
//! stays cold-poll inline.
//!
//! Every `opaque` crossing rides the erasure box: the frames
//! are sealed rut values (`RutOpaque` store entries), so the bodies
//! recover the raw frame slot through [`Vm::opaque_rut_value`] — the
//! `Opaque<T>` host-payload machinery does NOT apply (a rut-side box
//! answers `downcast<T>`, never a Rust `T`).

use rut_vm::heap::cell_of;
use rut_vm::interp::{HostPkg, Vm};
use rut_vm::{OpaqueRef, Slot, Trap, TrapKind};

use rut_core::async_frame as af;

/// The sleep mint's seal type: the rut-side `Future<nil>` CLASS
/// spelling (v20's closed builtin class — `mk_future` interns the
/// handle type as `Future<nil>`), so this exact name is what `sleep`'s
/// `opaque.downcast<Future<nil>>` compares the box against (the TidOf
/// law: a Rut entry answers its recorded `val_ty`).
const FUTURE_NIL_OBJ: &str = "Future<nil>";

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

// ---- the structured-competition bodies (select2 / select_all / completer)
//
// The frames are the compiler-minted competition frames
// (`rut-core/async_frame`): the standard engine layout plus the child
// slots, the ANSWER lane carrying the COMPOSITE (`Either2<A, B>`, the
// `(u32, T)` tuple, or the plain answer). Names derive on BOTH sides
// from the same spellings — the compiler from the wrapper's type
// arguments, these bodies from the children's answer fields — so a
// chained composition (a select over select results) reads honestly.

/// A frame's answer-lane type (field 4's declared type — the composite
/// for the competition frames, the fn's return for a woven frame).
fn answer_ty_of(vm: &Vm, frame_ty: u32) -> Result<u32, Trap> {
    match vm.prog.types.kind(frame_ty) {
        rut_core::types::TyKind::Data { fields } => fields
            .get(af::ANSWER_FIELD as usize)
            .map(|f| f.ty)
            .ok_or_else(|| {
                Trap::new(
                    TrapKind::Invalid,
                    "async engine: the frame layout has no answer lane",
                )
            }),
        _ => Err(Trap::new(
            TrapKind::Invalid,
            "async engine: a competition child is not a frame record",
        )),
    }
}

/// The checkpoint state slot, null once retired.
fn state_of(vm: &Vm, frame: Slot) -> Slot {
    vm.record_field(frame, af::STATE_FIELD as usize)
        .unwrap_or(Slot::null())
}

/// A retired frame: state null (completed, dropped at its checkpoint,
/// or cancelled-and-retired).
fn retired(vm: &Vm, frame: Slot) -> bool {
    unsafe { state_of(vm, frame).r.is_null() }
}

/// Retire a frame: state null. The driving loop's Done path wakes the
/// awaiter (and takes the queue's own reference accounting from there).
fn retire(vm: &Vm, frame: Slot) {
    vm.set_record_field(frame, af::STATE_FIELD as usize, Slot::null());
}

/// Library law, engine path: cancel a still-live child through the same
/// `cancel` the `__abort` row drives (flag + re-enqueue; the child's
/// own cancelled probe runs its drop path, and a chained child cascades
/// through its own cancelled arm). A retired child answers false —
/// already finished is already finished.
fn cancel_child(vm: &mut Vm, child: Slot) {
    vm.cancel(child);
}

/// The rut-side `T -> ?T` funnel, engine half: a composite answer
/// crossing into the Either2's `?T` fields boxes exactly as a rut
/// struct literal would — a cell-repr payload retains its handle, a
/// primitive copies its bits. The minted box's reference transfers to
/// the caller. A non-nullable field stores raw.
fn box_answer_for_field(vm: &Vm, field_ty: u32, raw: Slot) -> Result<Slot, Trap> {
    let rut_core::types::TyKind::Opt { elem } = vm.prog.types.kind(field_ty) else {
        return Ok(raw);
    };
    let elem = *elem;
    let is_fn = matches!(vm.prog.types.kind(elem), rut_core::types::TyKind::Fn { .. });
    if vm.prog.types.repr_of(elem).is_ref() || is_fn {
        if !unsafe { raw.r.is_null() } {
            vm.retain(raw);
        }
    }
    vm.alloc_opt_value(field_ty, raw)
}

/// Settle a select2 race: mint the composite `Either2<A, B>` with the
/// winner's side set (the `?A`/`?B` field boxed per the funnel), store
/// it in the frame's answer lane, retire. The caller cancels the
/// still-live loser.
fn settle_select2(vm: &Vm, frame: Slot, idx: u32, winner: Slot) -> Result<(), Trap> {
    let sel_ty = cell_of(frame).ty;
    let composite = answer_ty_of(vm, sel_ty)?;
    let field_ty = match vm.prog.types.kind(composite) {
        rut_core::types::TyKind::Data { fields } => {
            let nfields = fields.len();
            let fty = fields.get(idx as usize).map(|f| f.ty);
            (nfields, fty)
        }
        _ => {
            return Err(Trap::new(
                TrapKind::Invalid,
                "async engine: the select answer lane is not a record",
            ))
        }
    };
    let either2 = vm.alloc_zeroed_record(composite, field_ty.0)?;
    let ans = vm
        .record_field(winner, af::ANSWER_FIELD as usize)
        .unwrap_or(Slot::null());
    let boxed = box_answer_for_field(vm, field_ty.1.unwrap_or(rut_core::types::TY_NIL), ans)?;
    vm.set_record_field(either2, idx as usize, boxed);
    // the box's mint reference transfers to the either2 field's own (the
    // store retained); this body holds none
    vm.release(boxed);
    vm.set_record_field(frame, af::ANSWER_FIELD as usize, either2);
    // the either2's mint reference transfers to the answer lane's own
    vm.release(either2);
    retire(vm, frame);
    Ok(())
}

/// Arm one live child: the select frame becomes its awaiter (the ONE
/// shared waker — the first completion wakes the race, the loser's edge
/// is cleared by its own cancel path). A retired child is left alone.
fn arm_child(vm: &Vm, select_frame: Slot, child: Slot) {
    if retired(vm, child) {
        return;
    }
    vm.set_record_field(child, af::AWAITER_FIELD as usize, select_frame);
}

/// The mint side common to select2: derive the composite name from the
/// two children's answer lanes, find the per-composite frame the
/// compiler minted, and seal the new race frame under the
/// `Future<Either2<A, B>>` object spelling the wrapper downcasts.
fn mint_select2(vm: &Vm, a: Slot, b: Slot) -> Result<OpaqueRef, Trap> {
    // the children's type comes off the CELLS (the runtime truth — the
    // box's recorded type is the wrapper's `Future<T>` object spelling)
    let a_ans = vm.prog.type_name(answer_ty_of(vm, cell_of(a).ty)?);
    let b_ans = vm.prog.type_name(answer_ty_of(vm, cell_of(b).ty)?);
    let composite = format!("Either2<{a_ans}, {b_ans}>");
    let frame_ty = find_ty(vm, &format!("{}{}>", af::SELECT2_FRAME_PREFIX, composite))?;
    let fut_obj = find_ty(vm, &format!("Future<{composite}>"))?;
    let ckpt = find_ty(vm, af::SELECT_CKPT)?;
    let nfields = match vm.prog.types.kind(frame_ty) {
        rut_core::types::TyKind::Data { fields } => fields.len(),
        _ => return Err(Trap::new(TrapKind::Invalid, "async engine: the select frame type is not a record")),
    };
    let frame = vm.alloc_zeroed_record(frame_ty, nfields)?;
    let s0 = vm.enum_member_slot(ckpt, 0)?;
    vm.set_record_field(frame, af::STATE_FIELD as usize, s0);
    vm.set_record_field(frame, af::CHILD_A_FIELD as usize, a);
    vm.set_record_field(frame, af::CHILD_B_FIELD as usize, b);
    Ok(vm.seal_opaque(frame, fut_obj)?)
}

/// Build the async engine's pkg. No sink, no state: everything
/// routes through the VM's driving API (`launch` / `cancel` /
/// `arm_timer`) — the engine owns the queues, the host owns only the
/// crossing.
pub fn pkg() -> HostPkg {
    let mut pkg = HostPkg::new("async_engine");
    // launch_future's engine half: unbox the sealed frame (the box the
    // rut launcher minted with `opaque(f)`); the queue takes its own
    // reference
    rut_vm::pkg_fn!(pkg, "__launch", (OpaqueRef,) -> (),
        |vm: &mut Vm, f: OpaqueRef| {
            let frame = unsealed(vm, &f, "__launch")?;
            vm.launch(frame);
            Ok(())
        });
    // LaunchedFutureHandle::abort's engine half: `false` when the frame
    // already retired, else the flag + re-enqueue (the probe at its
    // checkpoint runs the drop path)
    rut_vm::pkg_fn!(pkg, "__abort", (OpaqueRef,) -> bool,
        |vm: &mut Vm, f: OpaqueRef| {
            let frame = unsealed(vm, &f, "__abort")?;
            Ok(vm.cancel(frame))
        });
    // sleep's engine-assisted mint: the sleep frame with `ms` in its
    // local field, SEALED under the `Future<nil>` object spelling — the
    // entry owns the mint reference, the owning handle hands the box's
    // own reference to the return path (`seal_opaque`)
    rut_vm::pkg_fn!(pkg, "__sleep", (u32,) -> OpaqueRef,
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
    rut_vm::pkg_fn!(pkg, "__sleep_yield", (OpaqueRef, OpaqueRef) -> (),
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
    // ---- the structured-competition rows ----
    //
    // `__select2`: unbox both children, mint the race frame sealed under
    // the composite `Future<Either2<A, B>>` class spelling. Nothing runs here —
    // the yield's fresh arm arms the cohort and launches it.
    rut_vm::pkg_fn!(pkg, "__select2", (OpaqueRef, OpaqueRef) -> OpaqueRef,
        |vm: &mut Vm, a: OpaqueRef, b: OpaqueRef| {
            let a_slot = unsealed(vm, &a, "__select2")?;
            let b_slot = unsealed(vm, &b, "__select2")?;
            mint_select2(vm, a_slot, b_slot)
        });
    // `__select2_yield`: the race frame's `Future::yield`. Fresh: arm the
    // shared waker on both live children, launch them, park at s1. A
    // child already retired answers immediately. Resumed: the first
    // retired child (index order breaks simultaneous ties) settles the
    // answer; the still-live loser is cancelled — the library's law,
    // driven through the `__abort` row's own cancel path. Cancelled
    // race: cascade through the children and retire so the awaiting
    // frame takes its drop path.
    rut_vm::pkg_fn!(pkg, "__select2_yield", (OpaqueRef, OpaqueRef) -> (),
        |vm: &mut Vm, f: OpaqueRef, _cx: OpaqueRef| {
            let ckpt = find_ty(vm, af::SELECT_CKPT)?;
            let frame = unsealed(vm, &f, "__select2_yield")?;
            let cancelled = vm
                .record_field(frame, af::CANCELLED_FIELD as usize)
                .map(|s| s.as_bool())
                .unwrap_or(false);
            let state = state_of(vm, frame);
            let fresh = !unsafe { state.r.is_null() }
                && vm
                    .enum_member_slot(ckpt, 0)
                    .ok()
                    .map(|s0| Slot::same_ref(state, s0))
                    .unwrap_or(false);
            let a = vm
                .record_field(frame, af::CHILD_A_FIELD as usize)
                .unwrap_or(Slot::null());
            let b = vm
                .record_field(frame, af::CHILD_B_FIELD as usize)
                .unwrap_or(Slot::null());
            if fresh && !cancelled {
                let a_done = retired(vm, a);
                let b_done = retired(vm, b);
                if a_done || b_done {
                    let (idx, winner, loser) =
                        if a_done { (0u32, a, b) } else { (1u32, b, a) };
                    settle_select2(vm, frame, idx, winner)?;
                    cancel_child(vm, loser);
                    return Ok(());
                }
                arm_child(vm, frame, a);
                arm_child(vm, frame, b);
                vm.launch(a);
                vm.launch(b);
                let s1 = vm.enum_member_slot(ckpt, 1)?;
                vm.set_record_field(frame, af::STATE_FIELD as usize, s1);
                return Ok(());
            }
            if cancelled {
                // the race itself was aborted: cascade through the
                // children, then retire — the awaiter's cancelled probe
                // takes the drop path at its own checkpoint
                cancel_child(vm, a);
                cancel_child(vm, b);
                retire(vm, frame);
                return Ok(());
            }
            // resumed: one child completed and woke us
            let a_done = retired(vm, a);
            let b_done = retired(vm, b);
            if a_done {
                settle_select2(vm, frame, 0, a)?;
                cancel_child(vm, b);
            } else if b_done {
                settle_select2(vm, frame, 1, b)?;
                cancel_child(vm, a);
            } else {
                // woken by neither child (defensive): re-arm the live
                // edges and stay parked
                arm_child(vm, frame, a);
                arm_child(vm, frame, b);
            }
            Ok(())
        });
    // `__select_all`: mint the cohort race over the sealed `[Future<T>]`
    // array. The element answer spelling comes off the first member's
    // answer lane (the typed surface guarantees uniformity); an empty
    // cohort is a contract breach — a race with no member has no winner.
    rut_vm::pkg_fn!(pkg, "__select_all", (OpaqueRef,) -> OpaqueRef,
        |vm: &mut Vm, cohort: OpaqueRef| {
            let (cohort_slot, _cohort_ty) = vm
                .opaque_rut_value(Slot { r: cohort.ptr() })
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "async engine: `__select_all` received a box that holds no rut value"))?;
            let members = cell_of(cohort_slot)
                .seq_items_copy()
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "async engine: `__select_all` takes a `[Future<T>]` cohort"))?;
            let first = *members.first().ok_or_else(|| {
                Trap::new(
                    TrapKind::Invalid,
                    "async engine: `select_all` over an empty cohort — a race with no member has no winner",
                )
            })?;
            let t_ans = vm.prog.type_name(answer_ty_of(vm, cell_of(first).ty)?);
            let frame_ty = find_ty(vm, &format!("{}{}>", af::SELECT_ALL_FRAME_PREFIX, t_ans))?;
            let fut_obj = find_ty(vm, &format!("Future<(u32, {t_ans})>"))?;
            let ckpt = find_ty(vm, af::SELECT_ALL_CKPT)?;
            let nfields = match vm.prog.types.kind(frame_ty) {
                rut_core::types::TyKind::Data { fields } => fields.len(),
                _ => return Err(Trap::new(TrapKind::Invalid, "async engine: the select-all frame type is not a record")),
            };
            let frame = vm.alloc_zeroed_record(frame_ty, nfields)?;
            let s0 = vm.enum_member_slot(ckpt, 0)?;
            vm.set_record_field(frame, af::STATE_FIELD as usize, s0);
            vm.set_record_field(frame, af::CHILD_A_FIELD as usize, cohort_slot);
            Ok(vm.seal_opaque(frame, fut_obj)?)
        });
    // `__select_all_yield`: the cohort race's `Future::yield`. Same
    // shape as select2's over N members: fresh arms + launches every
    // live member; the first retirement (index order breaks ties)
    // settles `(i, v)` into the tuple and every OTHER still-live member
    // is cancelled; the cancelled race cascades through the whole
    // cohort.
    rut_vm::pkg_fn!(pkg, "__select_all_yield", (OpaqueRef, OpaqueRef) -> (),
        |vm: &mut Vm, f: OpaqueRef, _cx: OpaqueRef| {
            let ckpt = find_ty(vm, af::SELECT_ALL_CKPT)?;
            let frame = unsealed(vm, &f, "__select_all_yield")?;
            let cancelled = vm
                .record_field(frame, af::CANCELLED_FIELD as usize)
                .map(|s| s.as_bool())
                .unwrap_or(false);
            let state = state_of(vm, frame);
            let fresh = !unsafe { state.r.is_null() }
                && vm
                    .enum_member_slot(ckpt, 0)
                    .ok()
                    .map(|s0| Slot::same_ref(state, s0))
                    .unwrap_or(false);
            let cohort = vm
                .record_field(frame, af::CHILD_A_FIELD as usize)
                .unwrap_or(Slot::null());
            let members = if unsafe { cohort.r.is_null() } {
                Vec::new()
            } else {
                cell_of(cohort).seq_items_copy().unwrap_or_default()
            };
            if fresh && !cancelled {
                if let Some(idx) = members.iter().position(|m| retired(vm, *m)) {
                    settle_select_all(vm, frame, idx as u32, members[idx])?;
                    for (i, m) in members.iter().enumerate() {
                        if i != idx {
                            cancel_child(vm, *m);
                        }
                    }
                    return Ok(());
                }
                for m in &members {
                    arm_child(vm, frame, *m);
                    vm.launch(*m);
                }
                let s1 = vm.enum_member_slot(ckpt, 1)?;
                vm.set_record_field(frame, af::STATE_FIELD as usize, s1);
                return Ok(());
            }
            if cancelled {
                for m in &members {
                    cancel_child(vm, *m);
                }
                retire(vm, frame);
                return Ok(());
            }
            if let Some(idx) = members.iter().position(|m| retired(vm, *m)) {
                settle_select_all(vm, frame, idx as u32, members[idx])?;
                for (i, m) in members.iter().enumerate() {
                    if i != idx {
                        cancel_child(vm, *m);
                    }
                }
            } else {
                for m in &members {
                    arm_child(vm, frame, *m);
                }
            }
            Ok(())
        });
    // `__completer`: the manual mint — a cold `#frame@completer<T>`
    // sealed under the `Future<T>` object spelling. `ty` is the linked
    // answer id (`type_id<T>()` rebases through link), the same text
    // the compiler's mint derived from the wrapper's type argument.
    rut_vm::pkg_fn!(pkg, "__completer", (u32,) -> OpaqueRef,
        |vm: &mut Vm, ty: u32| {
            let t_ans = vm.prog.type_name(ty).to_string();
            let frame_ty = find_ty(vm, &format!("{}{}>", af::COMPLETER_FRAME_PREFIX, t_ans))?;
            let fut_obj = find_ty(vm, &format!("Future<{t_ans}>"))?;
            let ckpt = find_ty(vm, af::COMPLETER_CKPT)?;
            let nfields = match vm.prog.types.kind(frame_ty) {
                rut_core::types::TyKind::Data { fields } => fields.len(),
                _ => return Err(Trap::new(TrapKind::Invalid, "async engine: the completer frame type is not a record")),
            };
            let frame = vm.alloc_zeroed_record(frame_ty, nfields)?;
            let s0 = vm.enum_member_slot(ckpt, 0)?;
            vm.set_record_field(frame, af::STATE_FIELD as usize, s0);
            Ok(vm.seal_opaque(frame, fut_obj)?)
        });
    // `__resolve`: the resolution right's engine half — store the
    // answer, retire, wake the awaiter. `__abort`-style stability:
    // false when the frame already retired (resolved, cancelled, or
    // abandoned); a cancelled-but-not-yet-retired frame stays on its
    // own retire path (the queued drive takes it).
    rut_vm::pkg_fn!(pkg, "__resolve", (OpaqueRef, OpaqueRef) -> bool,
        |vm: &mut Vm, f: OpaqueRef, v: OpaqueRef| {
            let frame = unsealed(vm, &f, "__resolve")?;
            let (val, _val_ty) = vm
                .opaque_rut_value(Slot { r: v.ptr() })
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "async engine: `__resolve` received a box that holds no rut value"))?;
            if retired(vm, frame) {
                return Ok(false);
            }
            vm.set_record_field(frame, af::ANSWER_FIELD as usize, val);
            retire(vm, frame);
            let awaiter = vm
                .record_field(frame, af::AWAITER_FIELD as usize)
                .unwrap_or(Slot::null());
            if unsafe { !awaiter.r.is_null() } {
                vm.set_record_field(frame, af::AWAITER_FIELD as usize, Slot::null());
                vm.launch(awaiter);
            }
            Ok(true)
        });
    // `__completer_yield`: the manual frame's `Future::yield` — a no-op
    // while live (the future progresses when the resolution right
    // fires, never by being driven); the cancelled arm retires so the
    // awaiter's probe takes the drop path.
    rut_vm::pkg_fn!(pkg, "__completer_yield", (OpaqueRef, OpaqueRef) -> (),
        |vm: &mut Vm, f: OpaqueRef, _cx: OpaqueRef| {
            let frame = unsealed(vm, &f, "__completer_yield")?;
            let cancelled = vm
                .record_field(frame, af::CANCELLED_FIELD as usize)
                .map(|s| s.as_bool())
                .unwrap_or(false);
            if cancelled {
                retire(vm, frame);
            }
            Ok(())
        });
    pkg.build()
}

/// Settle a select-all race: mint the `(u32, T)` tuple with the
/// winner's index and answer, store it in the frame's answer lane,
/// retire. The caller cancels the other still-live members.
fn settle_select_all(vm: &Vm, frame: Slot, idx: u32, winner: Slot) -> Result<(), Trap> {
    let sel_ty = cell_of(frame).ty;
    let composite = answer_ty_of(vm, sel_ty)?;
    let nfields = match vm.prog.types.kind(composite) {
        rut_core::types::TyKind::Data { fields } => fields.len(),
        _ => {
            return Err(Trap::new(
                TrapKind::Invalid,
                "async engine: the select-all answer lane is not a record",
            ))
        }
    };
    let tuple = vm.alloc_zeroed_record(composite, nfields)?;
    vm.set_record_field(tuple, 0, Slot::int(idx as i64));
    let ans = vm
        .record_field(winner, af::ANSWER_FIELD as usize)
        .unwrap_or(Slot::null());
    vm.set_record_field(tuple, 1, ans);
    vm.set_record_field(frame, af::ANSWER_FIELD as usize, tuple);
    // the mint reference transfers to the answer lane's own (the store
    // retained); this body holds none
    vm.release(tuple);
    retire(vm, frame);
    Ok(())
}
