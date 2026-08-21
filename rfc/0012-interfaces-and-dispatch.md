# RFC 0012: Interfaces & Dispatch — the Sole Dynamic Mechanism

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclass implementors), RFC 0010 (classes),
  RFC 0011 (Rc boxing)
- **Supersedes:** RFC 0002 §5.5, §6, §6.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`examples/basic/interfaces.rut`** — multiple `implements`, a composed
`Widget` interface, `requires`, dataclass implementors, interface-typed
arrays.

## 1. Dispatch: direct by default, vtable at interfaces

The static type decides the opcode:

```text
length(p)   // free fn        ->  call length$Point        (direct, always)
c.area()    // c: Circle      ->  call Circle$area         (direct, always)
b.area()    // b: Rc<Circle>  ->  call Circle$area         (direct — T known)
d.draw(g)   // d: Drawable    ->  calliface d, slot=3      (vtable load + call)
```

Calls on concrete dataclass/class/Rc types are always direct — the exact
type is statically known and can never change (no inheritance, RFC 0010 §3).
Only values whose static type is an interface dispatch through the vtable.
`final` is meaningless in v1 (nothing can override).

## 2. Interfaces

- Interfaces declare **plain methods only** — no fields, no properties of
  any kind (there is no `get`/`set` syntax in rut at all, RFC 0010 §2).
  Anything that reads like a property becomes a method: `x.count()` in the
  interface, implemented as a method on the class. Rationale: fields have
  no single offset rule under multiple `implements`, and an interface slot
  is always a code pointer invoked with an explicit `(...)` — one member
  kind, no call-vs-load ambiguity at the vtable boundary. (`d.x` where `d`
  is interface-typed is a compile error.)
- **Intersection types (`A & B`) are never supported** — not deferred, not
  planned: heterogeneous needs compose an interface that declares both
  method sets (`interface Widget` in the example). This is a design
  principle, not a v1 limitation.
- `implements` is **nominal and declared** — structural ("duck") conformity
  does not satisfy an interface. This keeps runtime type identity exact
  (RFC 0015) and casts cheap.
- **`requires` — an admission constraint, not subtyping.** An interface
  may require others: `interface Hashable requires Equal<Self> { .. }`.
  To implement `Hashable`, a type's `implements` list must **also** list
  `Equal<Self>` with `Self` bound to the implementor — `dataclass Point
  implements Hashable, Equal<Point>`; `Equal<SomeOtherType>` does not
  satisfy it. Requirements are transitive (`A requires B`, `B requires C`
  ⇒ `A` needs `C` too), cycles in the requires-graph are a link error,
  and registered builtin impls satisfy requirements like any other impl
  (RFC 0026). What `requires` deliberately is **not** (this is why
  interface `extends` was rejected): no member inheritance —
  `Hashable` declares only `hash`, and `eq` is reachable only through
  an `Equal<T>` ref; no subtyping — a `Hashable` ref does not widen to
  an `Equal<T>` ref; **vtables stay flat** — one interface, one vtable,
  the type test stays a single scan (RFC 0015 §6). The requires-graph is
  a compile-time walk over the implements list, never a runtime dispatch.
- Interfaces are implemented **by classes and dataclasses** (RFC 0009).
  An interface type is never a value's exact type; every interface
  value points at an Rc cell whose exact class or dataclass it carries
  (RFC 0015 §6). Generic interfaces exist — `Equal<T>` above is the
  canonical example — and each instantiation has its own vtable slots
  (`Equal<Point>` ≠ `Equal<string>`, RFC 0015 §6).
- Interface-typed values are the **only** dynamic dispatch in rut: a
  reference plus a vtable lookup per call. No `dyn`-typed variables, no
  `any`, no dynamic field access, no dynamic `this`.
- Heterogeneous collections are interface-typed arrays:
  `Array<Drawable>` — the replacement for both TS unions and the data-enums
  rut deliberately dropped (RFC 0006).

## 3. Type tests & upcasts — builtin functions, not keywords

There are **no cast keywords** (`as`, `as?`) and no `is` operator. The two
type-directed operations are prelude builtin generics. See
**`examples/basic/type-tests.rut`**.

- `is<T>(x): bool` — runtime test, true when `x`'s exact class (or
  dataclass — RFC 0009) implements `T` (or `T` is the exact type itself).
  With no inheritance this is a single descriptor lookup — exact
  `TypeId` compare plus one flat `implements` scan. Machinery in
  RFC 0015 §6.
- `upcast<T>(x): T` — explicit widening to an interface.
  **Compile-time checked** (`x`'s class must declare `implements T` —
  otherwise a compile error, never a runtime failure) and **zero runtime
  cost**: an interface value *is* the object ref, so `upcast` erases to a
  plain move (RFC 0015 §6).
- Implicit widening already happens on assignment/argument passing
  (`blit_all(g, [c])` passes a `Circle` as `Drawable`); `upcast` is the
  explicit form for disambiguation and for making the widening greppable.
- **Interface values cannot be downcast.** An interface value is used
  through its interface methods — if you need `Circle`-specific behavior
  behind a `Drawable`, put that behavior in the interface. Recovery of an
  erased value exists only through `Opaque` + `downcast<T>` (RFC 0014):
  erasure is explicit, so nothing dynamic ever flows through interface
  types.
- Numeric and enum conversions follow the same principle: named function
  calls, not operators (RFC 0007 §1, RFC 0006).

## Note

- ~~interface `extends` (interface hierarchies)~~ — **resolved: rejected**;
  `requires` shipped instead (§2) — an implementor-side admission
  constraint: flat vtables, no member inheritance, no
  interface-to-interface widening.
