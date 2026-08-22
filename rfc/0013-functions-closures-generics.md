# RFC 0013: Functions, Closures & Generics

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0012 (interfaces)
- **Supersedes:** RFC 0002 §9 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`examples/basic/closures-generics.rut`** — arrows (single-expr and
block), and a generic `first<T>` monomorphized to two instantiations.

## 1. Closures

- Closures capture **by reference** to the enclosing bindings (JS-like),
  which RC keeps safe within a VM (RFC 0016 §2). Closures are not
  transferable across isolates in v1 (RFC 0021 §3).

## 2. Generics

- Generics **monomorphize at compile time**: each instantiation emits its own
  typed opcodes (`arr.get<f32>` vs `arr.get<Handle>`). Instantiations whose
  bodies are descriptor-independent share one generic body at load time
  (RFC 0001 §Execution model); the compiler reports instantiation counts.
  The kind-specialization already covers **unsized `dyn` arguments**: a
  `T`-typed slot instantiated at any `dyn` type (`Vec<dyn Widget>`,
  `MyCow<dyn Slice<T>>`, RFC 0005) becomes a handle slot — `dyn` args are
  ordinary interface-object arguments, satisfying no bound.
- **Const-generic parameters exist only on the builtin `Array<T, N>`** in
  v1 (RFC 0005); user generics stay type-only — `N` is a constant
  expression, part of the instantiation identity (`Array<i32, 3> ≠
  Array<i32, 4>`, RFC 0015 §3).
- Generic parameters are unconstrained in v1 — no `T: Iface` bounds on
  user generics (OQ-1): you cannot call interface methods on a bare `T`.
  Pass values in, or take a `dyn I` parameter instead of a
  generic. **The one exception**: generic params of **host/extern class
  declarations** may carry interface bounds (`MyMap<K: Hashable, V>`,
  RFC 0025 §1) — admission-only syntax: it constrains which
  instantiations compile (closing over `requires`), grants no method
  calls on bare `K`, and adds no IR. Bounds stay **bare** — `K: Hashable`,
  never `K: dyn Hashable` (a bound names an interface, it does not form
  an interface value; `T: Any` is rejected as vacuous, RFC 0014). A
  user-class bound would be pure
  forwarding anyway (without dispatch, a body could only pass `K` onward
  to extern positions) — deferred with OQ-1.

## Open questions

- OQ-1: generic bounds `T: Iface` (would unlock static dispatch on
  bare `T` without `dyn I` refs) — still deferred for **user**
  generics; the **admission-only** form shipped scoped to extern decls
  (§2, RFC 0025 §1), which needs no dispatch and adds no IR.
