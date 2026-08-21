# RFC 0010: Classes & Factories — Sealed Value Records, No Inheritance

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclasses — the contrast case)
- **Supersedes:** RFC 0002 §5.2, §5.4 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`examples/basic/classes.rut`** — factory type-calls, the class-private
`Self { .. }` literal, `suspend factory`, `private factory` sealing, statics.

## 1. Classes — sealed value records with factories

- **Also a value type.** A bare `Circle` copies on assignment/passing/return
  exactly like a dataclass — inline, no header, no refcount. What a class
  *adds* over a dataclass is **sealing**: private fields, factory-only
  construction, `static` members, and `dispose()` (RFC 0011). (`implements`
  is no longer class-only — RFC 0009.)
- **Construction is a type-call**: `Circle(1, 2, 3)` runs the class's
  `factory`. No `new` keyword exists, and there is no outside literal for
  a class — construction always flows through a factory.
- **`factory` is just a function** — implicitly static (no `this`), ordinary
  params, ordinary body, a `return`. It builds the instance with the
  **class-private `Self { field: expr, .. }` literal** (the class name
  spells it inside the body too). The literal must initialize every field
  without an initializer; field initializers run for omitted fields.
  Private fields are settable in the literal — inside the class body only.
- The return type defaults to `Self` and may be declared otherwise, so
  "try" constructors are just factories:
  `factory parse(s: string): Option<Version>`; a dispose class's factory
  may hand out `Rc<Self>` directly (RFC 0011).
- **`suspend factory` is allowed** — same function, `await` in the body:
  `Circle(..)` then returns `Future<Circle>` and callers write
  `await Circle(..)` (cold future, RFC 0018 §2). No partially constructed
  instance ever exists across an `await` — the `Self { .. }` literal is an
  ordinary expression, and locals live in the coroutine frame.
- **No factory declared + every field has an initializer → default no-arg
  factory** (`Sprite()`). A field without an initializer and no factory
  makes the class unconstructible outside its own body.
- **`private factory` seals** the class: the type-call is legal only inside
  the class body. The named-factory pattern is an opt-in on top —
  `static fn issue(): AuthToken { return AuthToken("..") }` validates,
  caches, or registers, and outside code cannot bypass it.

## 2. Statics & accessors

- `static` members live in the class's module-static slot table. Static
  initializers must be const-expressions (RFC 0003 §1) and are materialized
  at load — factories and methods are the only places to run logic.
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
- Layout stays trivial: fields at fixed offsets — inline in bare values and
  Rc cells alike, no prefix layout, no fat pointers, and every object has
  exactly one concrete class forever. This keeps `is<T>` a single descriptor
  check (RFC 0015 §6) and the RC/cycle-collector walk flat (RFC 0017).
- If real code demands it later, inheritance returns as a separate RFC —
  the vtable design (RFC 0015 §6) already reserves room for it (OQ-1).

## Open questions

- OQ-1: class inheritance (`extends`/`super`/`override`) — removed from v1
  (§3); reintroduce only if composition proves insufficient, as a separate
  RFC.
- ~~constructor parameter properties~~ — **obsolete twice over**:
  factories are plain functions (params are params), and the
  `Self { field: name }` literal makes the param→field mapping explicit.
