# RFC 0013: Functions, Closures & Generics

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0012 (traits)
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
  ordinary trait-object arguments, satisfying no bound.
- **Const-generic parameters exist only on the builtin `Array<T, N>`** in
  v1 (RFC 0005); user generics stay type-only — `N` is a constant
  expression, part of the instantiation identity (`Array<i32, 3> ≠
  Array<i32, 4>`, RFC 0015 §3).
- Generic parameters are **unconstrained by default** — no `T requires Trait`
  bounds in the parameter list of user generics (OQ-1): you cannot call
  trait methods on a bare `T`. Pass values in, or take a `dyn I`
  parameter instead of a generic. Two **admission-only** forms ship —
  both gate which instantiations compile (closing over `requires`),
  grant no method calls on bare type params, and add no IR:
  1. inline param bounds on **host/extern class declarations**
     (`MyMap<K requires Hashable, V>`, RFC 0025 §1) — always bare (`K requires Hashable`,
     never `K: dyn Hashable`; `T requires Any` is vacuous and rejected, RFC 0014);
  2. a trailing **`where` clause on user generic fns** (RFC 0037 §3) —
     the serde motivating pair: producers that return `T` cannot take a
     `dyn I` parameter instead, so the contract rides the call site:
     `deserialize<T>(v: str): Result<T, JsonError> where T requires
     Deserializable`. A body may widen a `T`-typed *value* to `dyn I`
     (the bound proves the widening valid) but gains no class-method
     calls on bare `T`. (A user-class bound would be pure forwarding
     anyway — deferred with OQ-1.)

## Open questions

- OQ-1: generic bounds `T requires Trait` with **static dispatch** on bare `T`
  (would unlock it without `dyn I` refs) — deferred; the
  admission-only forms (host/extern inline bounds, §2 +
  RFC 0025 §1; user-fn `where` clauses, RFC 0037 §3) need no dispatch
  and add no IR.
