# RFC 0022: Embedding Model & Native Modules

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** Part D (RFC 0018–0021); RFC 0001 (G8)
- **Supersedes:** RFC 0005 §1–2, §7 (pre-restructure)
- **Part:** E — Host & FFI

## Summary

The host is a Rust program that owns a `Vm` (RFC 0034): it registers
native module **implementations** (surfaces are declared in rut-source
**declaration files**, RFC 0025), loads modules (no load-time execution), and
drives entry points. Every crossing is checked against reified types
(RFC 0015) — the bridge boilerplate tur needed simply does not exist.

**Non-feature:** there is no `box<T>` / loan / `&mut`-in-the-language
(RFC 0004 §2). The only shared-reference mechanism is `Rc<T>`; the only
borrows are **host-side, call-scoped, flag-guarded** (RFC 0023) — anything
wider is unsound and will not be added.

## 1. Embedding model

```rust
let mut vm = Vm::new(HostHooks { .. });            // RFC 0035 §1
vm.register_struct::<Vertex>("Vertex")?;           // RFC 0024 — layout contract
vm.register_module("app:gfx", gfx_module())?;      // bodies, bound against
vm.register_module("plugin:my_map", my_map_module())?;  // declaration files (RFC 0025)
vm.load("widgets")?;                               // verify + link, run nothing
let t = vm.spawn("main", &[])?;                    // suspend entry
vm.run_until_idle()?;                              // host owns the loop
```

`register_module` binds **implementations only** — every native surface
(what exists, its signatures, param bounds) is declared in rut source:
declaration files (RFC 0025). Compiling and checking rut code never requires any
Rust; `vm.load`'s link step proves every referenced native member has a
bound, signature-equal implementation (RFC 0026).

Native code never sees `&Vm` while rut runs (single-threaded); native fns
receive a `VmCtx` that permits re-entrant `vm.call` (RFC 0023 guards make
that safe) and registering wakeups (RFC 0020).

## 2. Native modules & typed functions

Registration binds bodies against the module's declaration file (RFC 0025) — the
string labels below are binding-time keys resolved against the decl's
slot table once, at startup; dispatch is by slot (RFC 0026):

```rust
fn gfx_module() -> NativeModule {
    NativeModule::new("app:gfx")                  // decl: app/gfx.rut
        .fn_("newCanvas", |ctx, w: i32, h: i32| Ok(Canvas::new(ctx, w, h)))
        .fn_("blit",      |ctx, c: Handle<Canvas>, layer: StructRef<Vertex>,
                           n: u32| { .. Ok(Value::Void) })
        .fn_("label",     |ctx, t: Template| Ok(log_localized(ctx, t)))
}
```

- Parameter and return types are declared **once**, as Rust types; the VM
  checks every call against them using the same `TypeId` machinery as
  `is<T>` (RFC 0015 §6). No coercion code, no `as number`, no
  `require_props_object`.
- Native registration supplies implementations for decl-module items:
  **`extern fn`s** (above), **class methods / factories** (RFC 0025, RFC
  0026), and — for the types themselves — the backing of **enum, interface,
  and builtin-impl registry entries** declared in declaration files
  (`std:collection`'s `Equal<T>`/`Hashable` + their builtin impls are
  the canonical case, RFC 0028).
- Failures are `Result<_, Trap>` values — a native fn that errors traps
  cleanly with a message and a rut backtrace (RFC 0034 §2).
- Long-running host work must NOT block the loop: hand back a future
  (RFC 0020) and let `await` integrate it.

## 3. Traps, budgets, interrupts

Native fns run **outside** the op budget (RFC 0034 §4) — the host is
trusted to be fast or hand back a future (§2). Traps raised inside a
native fn propagate as `Err(Trap)` with the native frame attributed in the
backtrace; `vm.call` re-entrancy nests budgets per outer frame. This is
the boundary where bugs become host problems — RFC 0001 P4's rule.
