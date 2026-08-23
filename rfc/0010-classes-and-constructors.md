# RFC 0010: Classes & Constructors — Sealed Value Records, No Inheritance

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclasses — the contrast case)
- **Supersedes:** RFC 0002 §5.2, §5.4 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`examples/basic/classes.rut`** — constructor type-calls, the class-private
`Self { .. }` literal, `suspend constructor`, `private constructor` sealing,
explicit `self` receivers, static fields.

## 1. Classes — sealed value records with constructors

- **Also a shared cell.** A bare `Circle` is a heap cell handle like every
  non-primitive (RFC 0004 §2, RFC 0016 §1): assignment shares, mutation
  is visible through aliases, `own(c)` is the eager copy (RFC 0011 §1).
  What a class *adds* over a dataclass is **sealing**: private fields,
  constructor-only construction, `static` fields, and `dispose()`
  (RFC 0011). Reflection:
  a class is walkable **iff** it implements `std:reflect.Reflectable`
  (or a contract layer requiring it — RFC 0037) — default opaque, a
  *default* not a law; and it can never implement `Deserializable` —
  construction is the constructor's job, reflective mint is
  descriptor-backed only. (`implements`
  is no longer class-only — RFC 0009.) A dataclass auto-implements
  `std:reflect`'s `Reflectable` + `Deserializable` (RFC 0037) — that,
  not the keyword, is why it reflects and round-trips; classes stay
  opt-in by hand. `==` on class values is cell identity for both
  (RFC 0012 §4); field-wise comparison is an opted-in `Hashable.eq`.
- **Construction is a type-call**: `Circle(1, 2, 3)` runs the class's
  `constructor`. No `new` keyword exists, and there is no outside literal for
  a class — construction always flows through a constructor.
- **`constructor` is just a function** — class-level by definition (it never
  takes a `self` receiver), ordinary
  params, ordinary body, a `return`. It builds the instance with the
  **class-private `Self { field: expr, .. }` literal** (the class name
  spells it inside the body too). The literal must initialize every field
  without an initializer; field initializers run for omitted fields.
  Private fields are settable in the literal — inside the class body only.
- The return type defaults to `Self` and may be declared otherwise, so
  "try" constructors are just constructors:
  `constructor parse(s: string): Option<Version>` (a `Disposal` class's
  constructor still returns `Self` — the handle is the ownership,
  RFC 0011 §2).
- **`suspend constructor` is allowed** — same function, `await` in the body:
  `Circle(..)` then returns `Future<Circle>` and callers write
  `await Circle(..)` (cold future, RFC 0018 §2). No partially constructed
  instance ever exists across an `await` — the `Self { .. }` literal is an
  ordinary expression, and locals live in the coroutine frame.
- **No constructor declared + every field has an initializer → default no-arg
  constructor** (`Sprite()`). A field without an initializer and no constructor
  makes the class unconstructible outside its own body.
- **`private constructor` seals** the class: the type-call is legal only inside
  the class body. The named-constructor pattern is an opt-in on top — a class
  method (`fn issue(): AuthToken { return AuthToken("..") }` — no `self`,
  §2) validates,
  caches, or registers, and outside code cannot bypass it.

## 2. Methods: explicit `self` — plus static fields & accessors

- **The receiver is explicit.** An instance method spells its receiver as
  the first parameter — `fn add(self, x: i32, y: i32)` — and the body
  reads fields through `self`. There is no `this` keyword at all (RFC 0002
  §4). A method without a `self` parameter is a **class method** —
  `fn from(x: i32)` — invoked on the class itself (`Version.from(..)`).
  **There is no `static fn`**: presence or absence of `self` is the whole
  distinction, stated in the signature and greppable — the same
  explicitness rule as `dyn` (RFC 0012 §2). Constructors never take `self`
  (construction has no receiver yet); `Disposal.dispose` does (`dispose(mut self)`,
  RFC 0011 §2). Interface methods and host/extern class methods follow
  the identical rule (RFC 0012 §2, RFC 0025 §2).
- `static` **fields** live in the class's module-static slot table. Static
  initializers must be load-time expressions (RFC 0003 §1) and are materialized
  at load — constructors and methods are the only places to run logic.
- No `get`/`set` accessor syntax anywhere — a computed property is just a
  method (`c.count()`), and a settable one takes an argument
  (`c.set_count(n)`). One member kind, one call convention, no hidden code
  behind field-access syntax.

## 3. No inheritance

- **No `extends` for classes** — v1 has no inheritance at all: no base-class
  constructors (`super(..)`), no method overriding (`override`), no `super.m()`
  calls, no `protected`.
- Classes are standalone types. Code sharing is composition (hold a helper
  object or dataclass in a field) or free functions; subtyping is only
  class→interface via `implements` (RFC 0012).
- Layout stays trivial: fields at fixed offsets — identical inside every
  cell payload (primitive fields inline, composite fields as handle
  slots), no prefix layout, no fat pointers, and every object has
  exactly one concrete class forever. This keeps `is` a single descriptor
  check (RFC 0015 §6) and the RC/cycle-collector walk flat (RFC 0017).
- If real code demands it later, inheritance returns as a separate RFC —
  the vtable design (RFC 0015 §6) already reserves room for it (OQ-1).

## Open questions

- OQ-1: class inheritance (`extends`/`super`/`override`) — removed from v1
  (§3); reintroduce only if composition proves insufficient, as a separate
  RFC.
- ~~constructor parameter properties~~ — **obsolete twice over**:
  constructors are plain functions (params are params), and the
  `Self { field: name }` literal makes the param→field mapping explicit.
