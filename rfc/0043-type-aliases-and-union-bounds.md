# RFC 0043: Type Aliases & Union Bounds

- **Status:** Draft
- **Date:** 2026-09-19
- **Revised:** 2026-09-20 — §3 grew teeth (native-fastpath phase 1):
  `requires` accepts a **type union** (type names ONLY — a trait member
  in a union spelling is invalid; a trait bound stands alone);
  membership is checked at every instantiation site; a method call on a
  union-bounded value resolves against the WHOLE bound (capability
  resolution at the call site, monomorphized dispatch); the bound
  provenance tracking is specified, with its documented holes
  (closures, shadowing).
- **Author:** hpp2334
- **Depends on:** RFC 0002 (lexical structure — `type` leaves the
  reserved table, `where` leaves the keyword set), RFC 0012 (traits, the
  impl registry), RFC 0013 (generics — §2 rewritten: inline `requires`
  replaces the trailing `where`), RFC 0030 (frontend grammar), RFC 0031
  (compiler — admission at the substitution-completing sites)
- **Part:** B — Language surface

## Summary

Two admission-surface features, one removal:

- `type X = A;` — a **transparent type alias**: `X` resolves to `A`'s
  `TypeId` everywhere (params, fields, returns, bounds). Chains expand;
  cycles are a compile error. Non-generic in v1.
- `type X = A | B;` — a **union alias**. Unions are **bound-only**:
  legal in `requires` bounds; a union in a value position is a compile
  error. There is no runtime union kind and no subtyping — RFC 0001's
  "no union value types" stays true.
- `fn f<T requires A | B>(..)` — inline **admission-only** bounds on fn
  and method generic parameters, enforced at instantiation; a union
  bound additionally carries the whole-bound method-call contract (§3 —
  every member must provide what the body calls). The trailing
  `where` clause is REMOVED — a stray `where` diagnoses with the inline
  replacement.

All three add no IR (RFC 0031: admission is a compile-time gate). This
RFC is the grammar + machinery A0 layer that generic-class bounds build
on (`pub class HashMap<K requires Hashable, V>` — RFC 0028's mapset
realization).

## 1. Type aliases

```
typealias := 'pub'?('(',vis,')')? 'type' Ident '=' Type ';'
```

- `pub`, `pub(mod)`, `pub(super)`, `pub(self)` all parse on `type` —
  recorded on the item like on `struct`/`class` (RFC 0003 §2). The
  `use`-both rule remains the binding access gate: the alias NAME is
  usable in a consumer only when the consumer wrote it in
  `use <pkg>::{ .. }`.
- **Transparency by construction.** The export surface carries a row
  keyed by the TARGET's id: the importer binds the alias name to the
  target's `(scope, local)` and needs zero other changes. A boot target
  (`pub type Meters = i64;`) keeps the shared boot scope — its id needs
  no rebase.
- Chains (`type Km = Meters; type Meters = i64;`) expand through
  resolution; `type A = B; type B = A;` diagnoses
  "recursive type alias" at the re-entered alias. Forward references are
  legal (targets validate after every module name is declared).
- Aliases are **non-generic** in v1: `type X = Vec<T>;` is not
  expressible (a generic alias would be a type constructor, not a
  transparent name).

## 2. Union aliases — bound-only

```
unionalias := typealias   // with Type spelled `A | B`
bound      := Type ('|' Type)*
```

- The target of an alias may be a union: `type Num = i32 | str;`. The
  union exists only in the type GRAMMAR (`TyUnion` AST node); the
  compiler never interns a union runtime kind.
- **Value positions reject unions.** A `TyUnion` (or a union-alias name)
  in a param/field/return/let/is position diagnoses
  "bound-only". `|` continues a completed type as a union ONLY where
  unions are legal — the alias target and `requires` bounds — so
  `x as u32 | y` keeps spelling the binary operator.
- Union aliases are **module-local**: never exported, a cross-module use
  fails as "unknown type" (a bound is a compile-time gate of the
  declaring module; carrying it would need a serialization story v1 does
  not have).

## 3. Inline `requires` — the admission-only bound

```
gparam := Ident ('requires' bound)?     // fns, methods, and classes (§A5)
```

- `fn f<T requires A | B>(x: T) -> i32` — the bound hangs off the
  generic parameter list. Members may be concrete type names (satisfied
  by `TypeId` equality at instantiation), aliases (expanded before the
  trait-vs-concrete detection), or a **type union** of those.
  **Type names only in a union** (2026-09 amendment): a trait member
  inside a union spelling parses but diagnoses at admission —
  "`Hashable` is a trait — a union bound takes type names only; a trait
  bound must stand alone (RFC 0043)" — and is dropped from the member
  list. A SINGLE-trait bound (`K requires Hashable`) stays legal,
  satisfied via the RFC 0012 impl registry; inside a union it would
  quietly turn capability resolution ("every named member provides the
  method", below) into an any-impl fact.
- **Enforcement at every substitution-completing site**: free-fn calls
  (inference or explicit args complete the substitution), direct and
  instance method instantiation, and impl-method enqueue. A failing
  instantiation diagnoses with the bound spelled out —
  "`bool` does not satisfy `T` requires `i32 | str` — no matching type
  or impl is registered (RFC 0043)" — never a silently widened or
  mis-compiled body.
- **Trait objects satisfy nothing** (RFC 0013 §2): a `T` instantiated at
  a trait-object type fails any bound — only a concrete type with a
  registered impl admits.
- **Bounds may reference the item's other generics**
  (`fn hold<T, U requires [T]>(x: U)`) — members resolve under the
  call-site substitution.
- **Admission-only — except the union's whole-bound contract.** A
  non-union bound (one concrete type, one trait) grants NO method calls
  on bare `T` (OQ-1 stays deferred). What it proves is the WIDENING: a
  body may widen a `T`-typed value into a bound-member-typed slot
  (`let w: Labeled = x;`) — nominal satisfaction through the same
  registry the bound checked.
- **Capability resolution at the call site** (2026-09 amendment): a
  method call on a value whose declared type is spelled from a
  union-bounded generic is checked against the WHOLE bound — EVERY
  member must provide the method through its own impls — diagnosing
  "`i32` does not provide `enc` — `K` requires `i32 | str` and a call
  on `K` needs every member of the union to provide it (RFC 0043)" at
  the call site, even when the offending member is never actually
  instantiated. Dispatch is untouched: each instantiation still
  monomorphizes, binding the concrete member's own impl — the gate
  buys compile-time whole-bound checking, not dynamic dispatch.
- **Provenance — how a value is known to carry a union-bounded type.**
  Per-frame and name-based: a generic PARAMETER, a `let` whose
  annotation spells the generic, a COPY of either (`let b = a;`), or a
  direct `self.f` field whose declared field type spells the generic.
  Provenance rides inlining (the spliced body keeps the gate) and is
  cleared when the name rebinds to a non-union shape. **Documented
  holes** (conservative — the gate under-fires, never over-fires): a
  closure body compiles with fresh, empty provenance, so a captured
  union-bounded value's method calls inside a lambda are NOT gated;
  a destructuring bind (`let (a, b) = ..`) carries none; and a
  shadowing rebind (`let g = g + 1;`) resets the name's provenance to
  the new shape, un-gating later calls on it. Closing these is
  deferred — admission at the instantiation sites still holds every
  value the bound admits.
- **`where` is removed.** The trailing clause and its grammar rule are
  gone; `where` leaves the keyword set (RFC 0002 §4) and becomes an
  ordinary identifier. A stray `where` in the old position diagnoses
  "`where` clauses are removed — write the bound inline:
  `fn f<T requires B>(..)` (RFC 0043)" and the parse recovers into the
  body.
- Misplaced bounds are a targeted error: struct/trait/surface generic
  parameters reject `requires` ("inline bounds bind fn/method/class
  generics only"). Generic-CLASS parameters TAKE them (the mapset plan
  §A5) — the bounds record on the class descriptor and admit every
  instantiation through the same helper (`Foo` does not satisfy `K`
  requires `Hashable` — no impl `Hashable` for `Foo` is registered).
  A class bound may be a TYPE UNION:
  `pub class HashMap<K requires i8 | … | bytes, V>` (the nmapset
  realization, native-fastpath phase 3) turns key admission into a
  compile error at every instantiation site; a single-trait class
  bound (`K requires Hashable`, mapset) stays legal. Inside the
  class's method bodies the union contract applies through the class's
  own `requires` — a method call on `self.k` gates on every member.
  A non-union bound is admission-only and grants no method calls on
  the bare parameter (OQ-1 stays deferred).

## 4. Semantics the engine sees

- Aliases and bounds add **no IR** (RFC 0031 §1): an alias resolves to
  its target's `TypeId` before any table work; a bound is checked
  compile-time only — membership once per substitution-completing
  site, the union's whole-bound method contract at the call sites of
  the generic body — and never consulted at runtime. Dispatch,
  vtables, and layouts are exactly what they would have been had the
  source spelled the target everywhere.
- The alias surface row is the one binary-surface addition: `SurfaceType`
  grows an explicit scope for boot-targeted aliases (the in-memory
  surface is not serialized; RFC 0038 bundles carry sources, so nothing
  on the wire changes).

## 5. Non-goals

- No generic aliases (`type Vec2<T> = Vec<T>`) — v1 aliases name one
  type.
- No union value types, no runtime union kind, no `is`/`when` support
  over unions (RFC 0006: heterogeneous data goes through traits or
  enums).
- No static dispatch on bare `T` (OQ-1, unchanged — see RFC 0013 §2).
- No intersection bounds (`T requires A + B`) — a union admits ANY
  member; conjunctions would need a second mechanism and a second
  diagnostic story.
