// batch.rs — the IMPLEMENTATION half of "plugin:batch".
//
// The DECLARATION half is rut source: examples/host/plugin/batch.d.rut
// (`pub host fn batch_new(..) -> Opaque;` and friends — .d.rut files are
// the only place `host` may appear, RFC 0029). This file declares NO
// surface — it registers bodies, bound by NAME at embedder startup and
// proven equal to the declaration at vm.load link (the two-checkpoint
// contract, RFC 0026 §1: rut-side checking needs no Rust; drift never
// reaches a runtime).
//
// There is no `host class` and no ClassTable anymore: every member of
// the module is a host fn, native state crosses as `Opaque` (the box
// holds this struct, RFC 0016 §5), and the rut consumer wraps the fns
// in a `class Batch` of its own (examples/host/batch.rut — the `Logger`
// pattern, RFC 0028). What this plugin demonstrates: the ROUND TRIP —
// the host constructs a Batch, rut runs procedures on the handle, then
// hands the very box back to Rust through submit's `Opaque` parameter.
// One instance, one rc lineage, one deterministic Drop (RFC 0016 §3).
//
// (Module loading is M2 — this file defines the target embedding API:
// what a fn-only plugin should FEEL like from Rust.)

use rut::{NativeModule, OpaqueBox, Trap, VmCtx};

/// The host value. Every host fn's signature is concrete (RFC 0023 §1),
/// so the crossing rule pins Rust types directly — no erased storage, no
/// per-call TypeId check. The instance lives in an Opaque cell (RFC
/// 0016 §5) boxing this struct; its derived Drop drops the Vec —
/// deterministic, at rc 0.
pub struct Batch {
    name: String,
    metrics: Vec<(String, i64)>,
}

/// `pub host fn batch_new(name: str) -> Opaque;` — construction mints the
/// box; rut never sees the layout, only the handle.
fn batch_new(_ctx: &mut VmCtx, name: String) -> Result<OpaqueBox<Batch>, Trap> {
    Ok(OpaqueBox::new(Batch { name, metrics: Vec::new() }))
}

/// `pub host fn batch_push(b: Opaque, metric: str, value: i64) -> unit;`
///
/// The `Opaque` parameter arrives as the box's CALL-SCOPED borrow
/// (RFC 0023 §2): `b.get::<Batch>()` yields `&mut Batch` for the
/// duration of the call, the cell's borrow flag is set, and a
/// re-entrant `vm.call` that tried to touch the same box again traps
/// `borrowed by host` instead of racing. Guards clear on return; to keep
/// data, the host copies — that is the whole rule.
fn batch_push(ctx: &mut VmCtx, b: &OpaqueBox, metric: String, value: i64) -> Result<(), Trap> {
    let batch: &mut Batch = b.get_mut(ctx)?;   // TypeId-checked unbox
    batch.metrics.push((metric, value));
    Ok(())
}

/// `pub host fn batch_len(b: Opaque) -> i32;`
fn batch_len(ctx: &mut VmCtx, b: &OpaqueBox) -> Result<i32, Trap> {
    Ok(b.get::<Batch>(ctx)?.metrics.len() as i32)
}

/// The host callback — `pub host fn submit(b: Opaque) -> str;`.
///
/// The receipt crosses back out as an ordinary checked return value. A
/// real embedder drains into its own sink here — fast. Slow delivery
/// must NOT block the loop (RFC 0022 §2): hand back a future and let
/// `await` integrate it (RFC 0020, M3). Keeping the batch past the
/// return takes an owning clone — the rc-inc'd copy, not the borrow:
///
///     let kept: RutValue = ctx.retain(&b);  // "to keep data, the
///     upload_queue.push(kept);              // host copies" — RFC
///                                           // 0023 §2; alive until
///                                           // the host drops it
fn submit_batch(ctx: &mut VmCtx, b: &OpaqueBox) -> Result<String, Trap> {
    let batch: &Batch = b.get(ctx)?;
    Ok(format!("{}#{}", batch.name, batch.metrics.len()))
}

pub fn batch_module() -> NativeModule {
    NativeModule::new("plugin:batch")
        .fn_("batch_new",  batch_new)    // binds BY DECL NAME — a typo is
        .fn_("batch_push", batch_push)   // a startup error, never a
        .fn_("batch_len",  batch_len)    // runtime one (RFC 0026 §4)
        .fn_("submit",     submit_batch)
}

// Embedder startup (RFC 0022 §1):
//
//     let mut vm = Vm::new(HostHooks { .. });
//     vm.register_module("plugin:batch", batch_module())?;  // bodies only
//     vm.load("app")?;   // verify vs declaration files; link impl == decl
//
// Then the consumer's `submit(b.h)` runs submit_batch above: the box the
// host built comes home as a borrow, the receipt crosses back as the
// checked return, and — after main returns and the box's rc hits 0 —
// the boxed Batch's Rust Drop runs. Deterministic, never "at GC
// someday" (RFC 0016 §3).
