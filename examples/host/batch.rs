// batch.rs — the IMPLEMENTATION half of "plugin:batch".
//
// The DECLARATION half is rut source: examples/host/plugin/batch.d.rut
// (`export host class Batch { .. }` + `export host fn submit(b: Batch):
// string` — .d.rut files are the only place `host` may appear,
// RFC 0029). This file declares NO surface — it registers bodies, bound
// by NAME at embedder startup and proven equal to the declaration at
// vm.load link (the same two-checkpoint contract as my_map, RFC 0026
// §1: rut-side checking needs no Rust; drift never reaches a runtime).
//
// What this plugin demonstrates that my_map does not: the ROUND TRIP.
// The host constructs a Batch, rut runs several procedures on it, then
// hands the very instance back to Rust by calling submit — a host fn
// whose parameter type is the host class. One instance, one rc lineage,
// one deterministic Drop (RFC 0016 §3).
//
// (Module loading is M2 — like my_map.rs, this file defines the target
// embedding API: what the callback should FEEL like from Rust.)

use rut::{ClassTable, GenericArgs, NativeModule, Trap, TypeRegistry, VmCtx};
use rut::value::Handle;

/// The host value. NOT generic — every member's signature is concrete,
/// so the crossing rule (RFC 0026 §3) pins Rust types directly: no
/// erased storage, no per-call TypeId check (contrast MyMap's V). The
/// instance lives in a RutOpaque cell (RFC 0016 §5) boxing this struct;
/// its derived Drop drops the Vec — deterministic, at rc 0.
pub struct Batch {
    name: String,
    metrics: Vec<(String, i64)>,
}

/// Instantiation builder — non-generic, so one table serves everyone;
/// the uniform builder shape keeps registration blind to the
/// difference (RFC 0026).
fn build_batch(_args: &GenericArgs, _types: &TypeRegistry) -> Result<ClassTable, Trap> {
    ClassTable::new::<Batch>(_args)
        // binds slot 0 (the declaration's factory -> 0): the native
        // class method `new`. Construction is an ordinary method call
        // (`Batch.new(..)` in the consumer) — RFC 0010 §1.
        .factory(|_ctx: &mut VmCtx, name: String| {
            Ok(Batch { name, metrics: Vec::new() })
        })
        // slot 1: "push" — a BINDING-TIME label, resolved against the
        // declaration's slot table once, at startup (RFC 0026 §4).
        .method("push", |_ctx, this: &mut Batch, metric: String, value: i64| {
            this.metrics.push((metric, value));
            Ok(())
        })
        // slot 2: "len"
        .method("len", |_ctx, this: &Batch| Ok(this.metrics.len() as i32))
        .build()
}

/// The host callback — `submit(b: Batch): string` in the declaration.
///
/// The class-typed parameter crosses as `Handle<Batch>` (RFC 0022 §2):
/// the VM already checked the argument's TypeId against the declared
/// signature at the call site (the same `is` machinery, RFC 0015 §6),
/// so — unlike MyMap's erased V — there is no per-call check to do
/// here; `b` is typed, statically.
///
/// The handle is a CALL-SCOPED borrow (RFC 0023 §2): `.get()` yields
/// `&Batch` for the duration of the call, and the cell's borrow flag is
/// set, so a re-entrant `vm.call` that tried `b.push(..)` again would
/// trap `borrowed by host` instead of racing. Keeping the value past
/// the return takes an owning clone — the rc-inc'd copy, not the
/// borrow:
///
///     let kept: RutValue = ctx.retain(&b);  // "to keep data, the
///     upload_queue.push(kept);              // host copies" — RFC
///                                           // 0023 §2; alive until
///                                           // the host drops it
fn submit_batch(ctx: &mut VmCtx, b: Handle<Batch>) -> Result<String, Trap> {
    let batch: &Batch = b.get();
    // A real embedder drains into its own sink here — fast. Slow
    // delivery must NOT block the loop (RFC 0022 §2): hand back a
    // future and let `await` integrate it (RFC 0020, M3).
    Ok(format!("{}#{}", batch.name, batch.metrics.len()))
}

pub fn batch_module() -> NativeModule {
    NativeModule::new("plugin:batch")
        .implement("Batch", build_batch)   // binds BY DECL NAME
        .fn_("submit", submit_batch)       // ditto — a typo is a
                                           // startup error, never a
}                                          // runtime one (RFC 0026 §4)

// Embedder startup (RFC 0022 §1):
//
//     let mut vm = Vm::new(HostHooks { .. });
//     vm.register_module("plugin:batch", batch_module())?;  // bodies only
//     vm.load("app")?;   // verify vs declaration files; link impl == decl
//
// Then the consumer's `submit(b)` runs submit_batch above: the instance
// the host built comes home as Handle<Batch>, the receipt crosses back
// as the checked return, and — after main returns and b's rc hits 0 —
// the boxed Batch's Rust Drop runs. Deterministic, never "at GC
// someday" (RFC 0016 §3).
