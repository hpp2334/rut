# RFC 0012: Interfaces & Dispatch — the Sole Dynamic Mechanism

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclass implementors), RFC 0010 (classes),
  RFC 0011 (Rc boxing), RFC 0014 (`Any` — read after)
- **Supersedes:** RFC 0002 §5.5, §6, §6.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

See **`examples/basic/interfaces.rut`** — multiple `implements`, a composed
`Widget` interface, `requires`, dataclass implementors, `dyn`-typed
vecs.

## 1. Dispatch: direct by default, vtable at interfaces

The static type decides the opcode:

```text
length(p)   // free fn        ->  call length$Point        (direct, always)
c.area()    // c: Circle      ->  call Circle$area         (direct, always)
b.area()    // b: Rc<Circle>  ->  call Circle$area         (direct — T known)
d.draw(g)   // d: dyn Drawable -> calliface d, slot=3      (vtable load + call)
```

Calls on concrete dataclass/class/Rc types are always direct — the exact
type is statically known and can never change (no inheritance, RFC 0010 §3).
Only values whose static type is an interface object type dispatch through
the vtable. `final` is meaningless in v1 (nothing can override).

## 2. Interfaces

- **`dyn` — the object-type spelling.** An interface name in **type
  position** — parameter/return/local/field types, generic arguments — is
  written `dyn I`: `d: dyn Drawable`, `Vec<dyn Widget>`, `Rc<dyn
  Hashable>`, `Rc<dyn Slice<i32>>` (the builtin slice interface, RFC 0005);
  `dyn` composes wherever a type does. Positions that merely
  **name** an interface stay bare: the `interface` declaration itself,
  `implements` / `requires` lists, and generic bounds (`K: Hashable` —
  admission-only syntax, RFC 0013 §2) — none of them denote an interface
  *value*. Every `dyn` in the source marks a vtable-dispatch use site:
  greppable dynamic dispatch (Rust's rule, adopted verbatim). A bare
  interface name where a type is expected is a compile error with an
  "insert `dyn`" suggestion. Every interface object type is **unsized**
  — `dyn I` and `dyn Slice<T>` alike: the payload lives in a heap cell,
  and the `dyn`-typed slot stores the cell handle (RFC 0031 §4).
- Interfaces declare **plain methods only** — no fields, no properties of
  any kind (there is no `get`/`set` syntax in rut at all, RFC 0010 §2).
  Anything that reads like a property becomes a method: `x.count()` in the
  interface, implemented as a method on the class. Interface methods are
  instance methods and spell the `self` receiver like every other
  (`fn draw(self, g: Canvas): void;` — RFC 0010 §2); a class method (no
  `self`) is not declarable in an interface. Rationale: fields have
  no single offset rule under multiple `implements`, and an interface slot
  is always a code pointer invoked with an explicit `(...)` — one member
  kind, no call-vs-load ambiguity at the vtable boundary. (`d.x` where `d`
  is interface-typed is a compile error.)
- **`Any` — the implicit top interface (RFC 0014).** Every type implements
  `Any`, nobody may declare or list it (`implements Any`, `T: Any` —
  compile errors), and it declares no dispatch methods — only the layout
  accessors `type_id()` / `size()` / `as_bytes()`. Its object type `dyn
  Any` is the erased storage position and the bottom of the interface
  tier: `exact concrete > dyn I > dyn Any` (RFC 0031 §4).
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
  (`Equal<Point>` ≠ `Equal<string>`, RFC 0015 §6). Interface object
  types are the **only** dynamic dispatch in rut: a
  reference plus a vtable lookup per call — every one spelled `dyn I` at
  the use site. No `any`, no dynamic field access, no `this` at all (the
  receiver is the explicit `self` parameter, RFC 0010 §2).
- Heterogeneous collections are interface-typed vecs:
  `Vec<dyn Drawable>` — the replacement for both TS unions and the data-enums
  rut deliberately dropped (RFC 0006).

## 3. Type tests — builtin functions, not keywords

There are **no cast keywords** (`as` is reserved and always errors) and no
`is` operator. The type-directed operations are prelude builtin generics
provided by the host. See **`examples/basic/type-tests.rut`**.

- `is<T>(x): bool` — runtime test, **`T` must be concrete** (an interface
  type argument — any `dyn I` — is a compile error: if you already hold a
  `dyn I`, testing against `I` is trivially true and anything else is a
  `downcast`, which interface values do not have). True when `x`'s exact
  class (or dataclass — RFC 0009) *is* `T`. With no inheritance this is a
  single descriptor lookup — exact `TypeId` compare. Machinery in
  RFC 0015 §6.
- **No `upcast` builtin.** Widening to `dyn I` is implicit on
  assignment/argument passing (`blit_all(g, [c])` passes a `Circle` as
  `dyn Drawable`); the explicit, greppable form is the `dyn` annotation at
  the receiving position (`const d: dyn Drawable = s;`). `upcast` was
  removed when `dyn` landed — the keyword does the marking.
- **Interface values cannot be downcast.** An interface value is used
  through its interface methods — if you need `Circle`-specific behavior
  behind a `dyn Drawable`, put that behavior in the interface. Recovery of an
  erased value exists only through `dyn Any` + `downcast<T>` (RFC 0014):
  erasure is explicit, so nothing dynamic ever flows through interface
  types.
- Numeric and enum conversions follow the same principle: named function
  calls, not operators (RFC 0007 §1). Erasure likewise: **`make_any(v):
  dyn Any`** (RFC 0014), a prelude builtin — rut has no cast syntax at
  all.

## Note

- ~~interface `extends` (interface hierarchies)~~ — **resolved: rejected**;
  `requires` shipped instead (§2) — an implementor-side admission
  constraint: flat vtables, no member inheritance, no
  interface-to-interface widening.
- ~~bare interface names as value types~~ — **resolved: superseded by
  `dyn`**; interface object types are spelled `dyn I` in every type
  position (Rust's rule, §2), while `implements`/`requires`/bounds stay
  bare.
- ~~`upcast<T>` builtin; interface type args to `is<T>`/`downcast<T>``~~ —
  **resolved: removed**; widening to `dyn I` is implicit (the `dyn`
  annotation marks it), and `is<T>`/`downcast<T>` take **concrete `T`
  only** (§3). Erasure is the prelude builtin `make_any(v): dyn Any`
  (RFC 0014) — `as` stays reserved, rut has no cast syntax.
