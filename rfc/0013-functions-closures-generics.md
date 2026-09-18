# RFC 0013: Functions, Closures & Generics

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0012 (traits)
- **Supersedes:** RFC 0002 §9 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`demo/src/examples/closures-generics.rut`** — arrows (single-expr and
block), and a generic `first<T>` monomorphized to two instantiations.

## 1. Closures

- The closure spelling is an **anonymous fn**: `fn (a: i32) -> bool
  { return a > 0; }` — block bodies, no arrow form. Function types are
  first-class: `fn apply(f: fn(i32) -> i32, v: i32) -> i32`; an
  anonymous fn inhabits `fn(P..) -> R` directly.
- Closures capture **by reference** to the enclosing bindings (JS-like),
  which RC keeps safe within a VM (RFC 0016 §2). Closures are not
  transferable across isolates in v1 (RFC 0021 §3).

## 2. Generics

- Generics **monomorphize at compile time**: each instantiation emits its own
  typed opcodes (`arr.get<f32>` vs `arr.get<Handle>`). Instantiations whose
  bodies are descriptor-independent share one generic body at load time
  (RFC 0001 §Execution model); the compiler reports instantiation counts.
  The kind-specialization already covers **unsized trait-typed arguments**: a
  `T`-typed slot instantiated at any trait type (`Vec<Widget>`,
  `MyCow<Slice<T>>`, RFC 0005) becomes a handle slot — trait-typed args are
  ordinary fat-ref arguments, satisfying no bound.
- **Const-generic parameters exist only on the builtin `Array<T, N>`** in
  v1 (RFC 0005); user generics stay type-only — `N` is a constant
  expression, part of the instantiation identity (`Array<i32, 3> ≠
  Array<i32, 4>`, RFC 0015 §3).
- Generic parameters are **unconstrained by default** — no method calls
  on a bare `T` (OQ-1): you cannot call trait methods on a bare type
  parameter. Pass values in, or take an `I`-typed parameter instead of a
  generic. One **admission-only** form ships — it gates which
  instantiations compile, grants no method calls on bare type params,
  and adds no IR: **inline `requires` bounds on fn/method generic
  parameters** (RFC 0043 — the trailing `where` clause is removed):
  `fn f<T requires A | B>(x: T)`. The serde motivating pair: producers
  that return `T` cannot take a `I`-typed parameter instead, so the
  contract rides the call site: `deserialize<T requires
  Deserializable>(v: str): Result<T, JsonError>`. Enforcement is at
  instantiation — every substitution-completing site checks the bound:
  concrete members by `TypeId` equality, trait members via the RFC 0012
  registry, trait objects satisfying nothing, and a failure diagnoses
  naming the bound. A body may widen a `T`-typed *value* to a
  bound-member-typed slot (the bound proves the widening valid) but
  gains no class-method calls on bare `T`. (A user-class bound is the
  deliberate class-frame extension — generic-class bounds
  `pub class HashMap<K requires Hashable, V>` land with the mapset work;
  the old inline bounds on `host class` declarations went with
  `host class` itself — RFC 0025, revised.)

## Open questions

- OQ-1: generic bounds `T requires Trait` with **static dispatch** on
  bare `T` (would unlock it without trait-typed refs) — deferred; the
  admission-only inline bound (RFC 0013 §2, RFC 0043) needs no dispatch
  and adds no IR. The bound proves widening; calls still go through a
  trait-typed slot.
