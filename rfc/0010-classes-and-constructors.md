# RFC 0010: Classes — Sealed Value Records, No Inheritance

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclasses — the contrast case)
- **Supersedes:** RFC 0002 §5.2, §5.4 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`demo/src/examples/classes.rut`** — class-method construction
(`Rect.new(..)`), the class-private `Self { .. }` literal, `suspend`
class methods, sealing without constructors, explicit `self` receivers.

## 1. Classes — sealed value records, class-method construction

- **Also a shared cell.** A bare `Circle` is a heap cell handle like every
  non-primitive (RFC 0004 §2, RFC 0016 §1): assignment shares, mutation
  is visible through aliases, `own(c)` is the eager copy (RFC 0011 §1).
  What a class *adds* over a dataclass is **sealing**: module-private fields
  (nothing is exposed without `pub`),
  class-method-gated construction, and `impl Disposal`
  (RFC 0011). Reflection:
  a class is walkable **iff** it has `impl std:reflect.Reflectable for C`
  (or a contract layer requiring it — RFC 0037) — default opaque, a
  *default* not a law; and it can never implement `Deserializable` —
  construction is the class's own job, reflective mint is
  descriptor-backed only. (Impl blocks
  are allowed for dataclasses as well — RFC 0009.) A dataclass auto-implements
  `std:reflect`'s `Reflectable` + `Deserializable` (RFC 0037) — that,
  not the keyword, is why it reflects and round-trips; classes stay
  opt-in by hand. `==` on class values is cell identity for both
  (RFC 0012 §4); field-wise comparison is an opted-in `Hashable.eq`.
- **Classes construct through their own class methods — nothing else
  is constructible.** There is no `constructor` keyword and no
  type-call: `Circle(1, 2, 3)` does not parse as construction, and no
  outside literal for a class exists. A class method named by the
  caller — `Rect.new(w, h)`, `Rect.from(other)`,
  `Version.parse(s)` — is the one construction surface (§2 defines
  class methods). `new` is not special syntax, just the conventional
  primary-constructor name (`from`, `parse`, `open`, `default` are its
  siblings); it is an ordinary identifier (RFC 0002 §4).
- **The `Self { field: expr, .. }` literal is the class-private
  construction** — legal anywhere inside the class body (class methods
  and instance methods alike); the class name spells it inside the body
  too (`Rect { .. }`). The literal must initialize every field without
  an initializer; field initializers run for omitted fields. Private
  fields are settable in the literal — inside the class body only.
  That privacy *is* the seal: outside code can only build a class
  value by calling a class method that chooses to build one.
- **Class methods are just functions** — no `self` receiver (§2),
  ordinary params, ordinary body, a `return`, and any declared return
  type: `fn new(s: str) -> Option<Version>` is a "try" constructor,
  `fn new(path: str) -> Self` an ordinary one (a `Disposal` class's
  constructing method still returns `Self` — the handle is the ownership,
  RFC 0011 §2). Being functions, they validate, default, cache,
  register, or hand out singletons — construction logic has no
  special rules.
- **No parameter properties**: parameters are parameters — and the
  `Self { field: name }` literal
  makes the param→field mapping explicit.
- **`suspend` class methods are allowed** — same function, `await` in
  the body: `await Socket.connect(addr)` returns when the connection
  is up (cold future, RFC 0018 §2). No partially constructed instance
  ever exists across an `await` — the `Self { .. }` literal is an
  ordinary expression, and locals live in the coroutine frame.
- **No implicit default construction.** A class whose fields all have
  initializers still needs an explicit `fn new() -> Self { return
  Self {}; }` if outsiders should build it. A class with no accessible
  constructing class method is **sealed** — constructible only inside
  its own body (the module-private `fn of(..)` + `pub fn parse(..)` pair
  is the standard shape: parsing validates, `of` trusts).

## 2. Methods: explicit `self` — plus accessors

- **The receiver is explicit.** An instance method spells its receiver as
  the first parameter — `fn add(self, x: i32, y: i32)` — and the body
  reads fields through `self`. There is no `this` keyword at all (RFC 0002
  §4). A method without a `self` parameter is a **class method** —
  `fn new(w: f32, h: f32)`, `fn from(x: i32)` — invoked on the class
  itself (`Rect.new(..)`, `Version.from(..)`, and `Self.new(..)` inside
  the body). Class methods are the construction surface (§1).
  Presence or absence of `self` is the whole distinction, stated in the
  signature and greppable — the same explicitness rule as `dyn`
  (RFC 0012 §2); there is no separate "static" method form. Class methods never take
  `self` (there is no receiver yet — or ever, for pure utilities);
  `Disposal.dispose` does (`dispose(mut self)`,
  RFC 0011 §2). Trait methods follow the identical rule (RFC 0012 §2);
  native surfaces declare no methods at all — host fns live behind a
  rut wrapper class's methods (RFC 0025, revised). Trait declarations
  may not contain class methods — trait members are instance
  methods with `self` (RFC 0012 §2).
- No `get`/`set` accessor syntax anywhere — a computed property is just a
  method (`c.count()`), and a settable one takes an argument
  (`c.set_count(n)`). One member kind, one call convention, no hidden code
  behind field-access syntax.
- **Member visibility is `pub`, scoped exactly like declarations**
  (RFC 0003 §2). An unannotated field or method is module-private —
  the same safe default every declaration gets; there is no `private`
  keyword. `pub` exposes a member to importers, `pub(mod)`/`pub(super)`/
  `pub(self)` scope it to the package/parent/module. The construction
  surface is therefore explicit: `pub fn new(..)` builds, unannotated
  members stay the class's own business (within its module). Static
  fields take the same forms. Dataclass members are always public —
  RFC 0009 keeps the all-record contract; trait signatures and impl
  methods carry no visibility of their own (as public as the trait).

## 3. No inheritance

- **No `extends` for classes** — v1 has no inheritance at all: no base-class
  constructors (`super(..)`), no method overriding (`override`), no `super.m()`
  calls, no `protected`.
- Classes are standalone types. Code sharing is composition (hold a helper
  object or dataclass in a field) or free functions; subtyping is only
  class→trait object via `dyn` widening (RFC 0012).
- Layout stays trivial: fields at fixed offsets — identical inside every
  cell payload (primitive fields inline, composite fields as handle
  slots), no prefix layout, no fat pointers, and every object has
  exactly one concrete class forever. This keeps `is` a single descriptor
  check (RFC 0015 §6) and the RC/cycle-collector walk flat (RFC 0017).
- If real code demands it later, inheritance returns as a separate RFC —
  the vtable design (RFC 0015 §6) already reserves room for it (OQ-1).

## Open questions

- OQ-1: class inheritance (`extends`/`super`/`override`) — not in v1
  (§3); reintroduce only if composition proves insufficient, as a separate
  RFC.
