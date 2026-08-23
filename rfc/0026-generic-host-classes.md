# RFC 0026: Generic Host Classes — a User-Defined Map

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0025 (host classes & declaration files), RFC 0012 §2
  (`requires`), RFC 0015 (reification)
- **Supersedes:** RFC 0005 §5.1 (pre-restructure)
- **Part:** E — Host & FFI

## Summary

The two halves of a native module (full listing:
**`examples/host/plugin/my_map.d.rut`** — the declaration;
**`examples/host/my_map.rs`** — the implementation;
**`examples/host/my-map.rut`** — a consumer).

**Declaration — rut source** (embedder-authored, ships with the plugin):

```rut
// plugin/my_map.d.rut — the declaration file for "plugin:my_map"
import { Hashable } from "std:collection";

export host class MyMap<K requires Hashable, V> {     // K bound = admission only
    fn new(cap: i32): Self;                        // native class method —
    fn set(self, k: K, v: V): void;                // the construction surface
    fn get(self, k: K): Option<V>;
    fn size(self): i32;
    fn keys(self): Vec<K>;
}
```

**Implementation — Rust, bodies only** (no surface data declared in Rust
at all):

```rust
pub struct MyMap {                              // NOT generic — see below
    inner: HashMap<IfaceHandle, RutValue>,      // K: fat ref, V: erased
}

fn build_my_map(args: &GenericArgs, types: &TypeRegistry)
    -> Result<ClassTable, Trap>                 // once per instantiation
{
    let v_ty = args.of("V").ty();               // reified V — drives checks
    let k_ty = args.of("K").ty();               // for keys(): Vec<K>

    ClassTable::new::<MyMap>(args)
        .method("new",  |ctx: &mut VmCtx, cap: i32| Ok(MyMap::with_capacity(cap)))
        .method("set",  |ctx, this: &mut MyMap, k: IfaceHandle, v: RutValue| {
            ctx.check_arg(&v, v_ty)?;           // value's TypeId == this
            this.inner.insert(k, v); Ok(())     // instantiation's V
        })
        .method("get",  |ctx, this: &MyMap, k: IfaceHandle|
            Ok(this.inner.get(&k).cloned()))    // -> builtin Option<V>
        .method("size", |ctx, this: &MyMap| Ok(this.inner.len() as i32))
        .method("keys", |ctx, this: &MyMap| { /* Vec<K> — unerased */ .. })
        .build()
}

pub fn my_map_module() -> NativeModule {
    NativeModule::new("plugin:my_map")
        .implement("MyMap", build_my_map)       // binds BY DECL NAME
}
```

## 1. The connection contract — two checkpoints

| checkpoint | when | checks | errors to |
|---|---|---|---|
| **compile/verify** | `rutc check` / `vm.load` verify | instantiation vs the **host decl**: `MyMap<Canvas, ..>` is a rut-line error (`Canvas` does not implement `Hashable`); admission closes over the `requires` graph (RFC 0037; `Hashable` is standalone — RFC 0028). Checking needs **no Rust at all**. | rut author |
| **link** | `vm.load` | every host member **referenced** by rut code has a bound impl, and the ClassTable (reflected member names + Rust shapes under the crossing rule) equals the decl's signatures — a pure data compare, nothing runs. | loader / embedder |

A name bound that no decl declares is an embedder **startup** error
(typo guard). Drift between the halves never reaches a rut runtime.

## 2. What is `V`? Reified instantiation, erased storage

A Rust generic (`build_my_map<V>`) is impossible: Rust monomorphizes at
*Rust* compile time, but the builder runs at *rut* runtime, once per
instantiation — nobody can supply `V`. Instead:

- the rut side **reifies** each instantiation — `GenericArgs` carries
  K's and V's `TypeId`s (RFC 0015); that is what the builder
  receives (`args.of("V").ty()`);
- the Rust side stores **erased** — `RutValue` (owning handle;
  `Value<'v>` in RFC 0023 is its call-scoped borrow), checked per call against
  the reified V. `get` needs no per-call check: values only enter via
  `set`, and the cell carries the instantiation's `TypeId`.

## 3. Crossing rule for decl types → Rust shapes

| decl type | Rust shape |
|---|---|
| param constrained to an interface (`K requires Hashable` — the bound stays bare on the rut side, RFC 0013 §2) | `IfaceHandle` (RFC 0015 §6 fat ref; the rut-facing object type is `dyn Hashable`) — concrete; its `Hash`/`Eq` are implemented **once** by the rut crate, vtable-dispatching into the value's own `hash()`/`eq()` (user impls are rut code; builtin/voucher impls are native trampolines). Content hashing for `string` keys, value hashing for primitive/enum keys — same Rust type, different attached vtable. |
| unconstrained param (`V`) | `RutValue` — erased owning handle; per-call check against the reified `TypeId` |
| concrete types (`i32`, `f32`, `Template`, …) | the Rust type — as in RFC 0022 §2, embedder-pinned at Rust compile time |
| `Self` | the instance handle |

Rust generics survive **only** for concrete signatures (RFC 0022 §2);
generic rut classes never see them. (OQ-1: a `.specialize(v_ty, builder)`
escape hatch for unboxed storage on hot instantiations — the erased builder
is the semantic baseline.)

## 4. Slots, not strings

The `.method("get", ..)` label exists for the register step only:
compiling the declaration file assigns slot ids (`new→0, set→1, get→2, …`);
consumer calls compile to `callnat { slot }` (RFC 0032 — fold/CSE-safe,
no string in IR, no lookup at dispatch); `register_module` resolves each
label against the slot table **once**, before any rut code runs — a typo is
a startup error, never a runtime one. IR still cannot inline into native
bodies (honest FFI limit); if a host wants an optimizable body, it is rut
source.

## 5. Notes

- **Who satisfies an interface constraint**: user classes and dataclasses
  (`implements` — RFC 0009, RFC 0012 §2), builtins via registered impls
  (`Hashable`: content for `string`, value for
  numerics/`enum`), and registered structs via a `register_struct`
  voucher (the Rust mirror is `Hash + Eq` — no rut-side
  methods needed). The builtin generics
  (`Option`/`Result`/`Vec`/`Array<T, N>`), interfaces themselves,
  `Opaque` boxes, and slices satisfy nothing.
- Builtin types flow back natively — a method may return `Option<V>` or
  build a `Vec<K>` host-side (`keys()` yields `Vec<K>`, **not**
  `Vec<dyn Hashable>`: keys are *unerased* at the boundary — rut cannot
  consume interface refs there, RFC 0012 §3); rut cannot tell it wasn't
  written in rut.
- Hashing/`eq` on a user type may be **rut code**, reached through the
  interface vtable — a method call that re-enters the VM (`VmCtx`,
  RFC 0022 §1). Re-entrancy is already guarded: `set` holds `&mut this`, the
  cell's borrow flag is set, and a hash impl that calls `m.set(..)` again
  traps `borrowed by host` (RFC 0023 §2) instead of corrupting the table.
- The **native class method** makes construction an ordinary method
  call (`MyMap<string, i32>.new(32)`), same rule as user classes
  (RFC 0010 §1): construction is a function everywhere, and a host
  class simply supplies the function.
  Helper fns like `newCanvas()` remain the shape for host-computed or
  side-effecting construction.

## Open questions

- OQ-1: `.specialize(v_ty, builder)` — unboxed-storage escape hatch for hot
  instantiations.
