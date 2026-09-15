# RFC 0012: Traits & Dispatch — the Sole Dynamic Mechanism

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclass implementors), RFC 0010 (classes),
  RFC 0011 (reference semantics), RFC 0014 (`Opaque` — read after)
- **Supersedes:** RFC 0002 §5.5, §6, §6.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

`trait I` interfaces: methods only, `impl Trait for Type` blocks, `dyn I`
object types, `requires` bounds, multiple `impl` blocks per type, composed
interfaces, `is` capability probes.

## 1. Dispatch: direct by default, vtable at traits

Dispatch splits on **where the method is declared**:

```text
length(p)    // free fn                   ->  call length$Point    (direct)
c.area()     // c: Circle, inherent fn    ->  call Circle$area     (direct)
b.area()     // b: Circle (aliased handle ->  call Circle$area     (direct)
               //  — the type is still exact)
d.draw(g)    // draw declared in Drawable ->  calli d, slot=3      (vtable, ALWAYS)
```

Free functions, constructors, and methods declared in a class/dataclass body
itself dispatch directly — the exact type is statically known and can
never change (no inheritance, RFC 0010 §3). Every method **declared in a
interface** — and defined in an `impl` block — dispatches dynamically through
the vtable — unconditionally, even when the receiver's exact class is
statically known: a `Circle` calling its own `Drawable.draw` still emits
`calli`. Trait members are dynamic by charter (one uniform rule, no
devirtualization); static dispatch lives in monomorphized generics and
inherent methods. `final` is meaningless in v1 (nothing can override).

## 2. Traits

- **`dyn` — the object-type spelling.** A interface name in **type
  position** — parameter/return/local/field types, generic arguments — is
  written `I`: `d: Drawable`, `Vec<Widget>`,
  `Vec<Hashable>`, and `Wrap<i32>` for a generic interface
  instantiation (RFC 0012 §2.1);
  `dyn` composes wherever a type does. Positions that merely
  **name** a interface stay bare: the `interface` declaration itself,
  `impl` heads and `requires` lists, and generic bounds (`K requires
  Hashable` —
  admission-only syntax, RFC 0013 §2) — none of them denote a interface
  *object*. Every `dyn` in the source marks a vtable-dispatch use site:
  greppable dynamic dispatch (Rust's rule, adopted verbatim). A bare
  interface name where a type is expected is a compile error with an
  "insert `dyn`" suggestion. Every interface object type is **unsized**
  — `I` and `Iter` alike: the payload lives in a heap cell,
  and the `dyn`-typed slot stores the cell handle (RFC 0031 §4).
- Traits declare **plain methods only** — no fields, no properties of
  any kind (there is no `get`/`set` syntax in rut at all, RFC 0010 §2).
  Anything that reads like a property becomes a method: `x.count()` in the
  interface, defined in the type's impl block. Trait methods are
  instance methods and spell the `self` receiver like every other
  (`fn draw(self, g: Canvas) -> unit;` — RFC 0010 §2); a class method (no
  `self`) is not declarable in a interface. Rationale: fields have
  no single offset rule under multiple impls, and a interface slot
  is always a code pointer invoked with an explicit `(...)` — one member
  kind, no call-vs-load ambiguity at the vtable boundary. (`d.x` where `d`
  is interface-typed is a compile error.)
- **No top interface.** The erased-storage type is the concrete host
  class `Opaque` (RFC 0014) — reached by the explicit type-call
  `Opaque(v)`, never by widening, and never nameable in an
  `impl`/`requires` list (it is a class, not a interface). The
  interface tier is simply `exact concrete > I` (RFC 0031 §4);
  the universal type test is `x is Opaque` (§3).
- **Intersection types (`A & B`) are never supported** — not deferred, not
  planned: heterogeneous needs compose a interface that declares both
  method sets (`interface Widget` in the example). This is a design
  principle, not a v1 limitation.
- **`impl Trait for Type` — nominal and declared.** Structural ("duck")
  conformity does not satisfy a interface: the admission is the impl block
  itself. This keeps runtime type identity exact (RFC 0015) and casts
  cheap.
  - **Placement:** an impl block lives in the **module that declares
    `Type`** — anywhere else is a compile error ("impl for a foreign
    type"). Builtin types (`Vec`, `Option`, `str`, …) are not
    declarable as impl targets either: their interface admissions are
    registered natively (RFC 0026 registry). One impl per (interface, type)
    pair per program — a duplicate (two modules, or two blocks in one) is
    a link error.
  - **Bodies:** each method in an impl block matches a interface `methsig`
    exactly — receiver form (`self`/`mut self`), params, and return type;
    a missing signature is a compile error, and so is any extra or
    inherent method inside the block (put those in the class/dataclass
    body). Member visibility, `suspend`, and constructors are not
    declarable in impl blocks. An **empty** impl block is legal and is
    the opt-in marker: `impl Serializable for User {}` (RFC 0037).
  - **Generic targets:** `impl Hashable for Pair<A, B>` — the generic
    args of the target bind as the impl's type parameters; no impl-level
    `where` in v1 (bounds come from the interface's own `requires`, RFC 0013 §2).
- **`requires` — an admission constraint, not subtyping.** A interface
  may require others: `interface Serializable requires Reflectable`
  (RFC 0037). To implement `Serializable`, the module must **also**
  contain `impl Reflectable for T` (transitively closed at link);
  requiring a generic instantiation binds `Self` to the implementor.
  Requirements are transitive (`A
  requires B`, `B requires C` ⇒ `A` needs `C` too), cycles in the
  requires-graph are a link error,
  and registered builtin impls satisfy requirements like any other impl
  (RFC 0026). std:reflect's auto-impls (dataclass/enum) and registry
  impls (`Option`/`Result`/`Vec`/`Array<T, N>`'s `Reflectable`/
  `Deserializable`) enter the graph the
  same way — auto-fills satisfy the `requires` edges of contract layers
  built on top (`impl Serializable for User {}` costs zero
  methods, RFC 0037). What `requires` deliberately is **not** (this is why
  interface `extends` was rejected): no member inheritance —
  `Hashable` declares both `hash` and `eq` itself (RFC 0028), and
  nothing else is reachable through it; no subtyping — a `Serializable`
  ref does not widen to a `Reflectable` ref; **vtables stay flat** — one
  interface, one vtable,
  the type test stays a single scan (RFC 0015 §6). The requires-graph is
  a compile-time walk over the module's impl blocks, never a runtime dispatch.
- Traits are implemented **for classes and dataclasses** (RFC 0009)
  via impl blocks. A interface type is never a value's exact type; every interface
  object is a fat ref over a cell whose exact class or dataclass it
  carries (RFC 0015 §6). **Generic interfaces** are supported — `interface Wrap<T>`
  — and each type-argument list is its own instantiation with its own
  interface id and vtable slots (`Wrap<i32>` ≠ `Wrap<str>`, RFC 0015 §6).
  A generic instantiation is spelled in type position as `Wrap<i32>`.
  Interface object
  types are the **only** dynamic dispatch in rut: a
  reference plus a vtable lookup per call — every one spelled `I` at
  the use site (there is no `dyn`; the engine knows what is an
  interface). No `any`, no dynamic field access, no `this` at all (the
  receiver is the explicit `self` parameter, RFC 0010 §2).
- **The element is a type argument, not an associated type.** An interface
  may be generic (`Index<T>`, `Iterator<T>`) and each impl names the
  element: `impl Iterator<char> for str`, `impl Index<T> for Vec<T>`.
  There are no associated `type` members. The element type is resolved at
  the use site from the impl's argument and stays reified in the type
  table. Two builtin contracts: `Index<T>` (`len`/`get`/`set`) drives
  `x[i]` and the indexed `for..of`; `Iterator<T>` (`next`, no `len`) drives
  cursor `for..of`. Both are std:core imports like every prelude name
  (RFC 0028) — the builtin `Array`/`str`/`bytes` index themselves without
  the trait; user types reach the contracts through
  `import { Index, Iterator } from "std:core"`. A type may have more
  than one element choice; the use
  site selects it. `Vec` (rut code) declares `impl Index<T> for Vec<T>` and
  ships `VecIter<T>` for `v.iter()`.
- Heterogeneous collections are interface-typed vecs:
  `Vec<Drawable>` — the replacement for both TS unions and the data-enums
  rut deliberately dropped (RFC 0006).

## 3. Type tests — the `is` keyword

`as` is now the numeric cast and nothing else — `expr as T`,
truncating, RHS a naming position restricted to the numeric primitives
(RFC 0007 §1) — but type tests are the **`is` keyword**: `expr is Type`
→ `bool`.

- **Grammar:** `expr is Type` at relational precedence,
  non-associative (RFC 0030 §2/§3). The RHS is a **naming position**
  like `impl`/`requires` lists (§2): a bare interface name or
  instantiation (`x is Hashable`, `x is Wrap<i32>`) or a concrete type
  (`d is Circle`) — never `dyn`-prefixed. Every type test spells `is` —
  there is no `is<T>()` builtin.
- **Two probes, one keyword.** Concrete RHS — exact-type test:
  true when `x`'s exact class (or dataclass — RFC 0009) *is* `T`;
  with no inheritance this is a single descriptor lookup — exact
  `TypeId` compare (RFC 0015 §6). Trait RHS — **capability
  probe**: true when the value's exact type has an impl for that interface
  — the `is_a` descriptor/registry scan of RFC 0015 §6,
  user-reachable (`k is Hashable`, `x is Drawable`). `is` is
  total: never traps, never recovers, yields only `bool`.
- **`x is Opaque` is legal** — a plain concrete test ("is this value an
  `Opaque` handle?"), folding to `true` on `Opaque`-typed receivers
  with the usual lint. On an `Opaque` receiver, `o is T` / `o is I`
  **see through the box**: they test the boxed value's type (RFC 0014).
- **No flow sensitivity:** `if (x is Hashable) { .. }` grants nothing
  — no narrowing, no widening of `x` to `I` (a bound proves
  widening, RFC 0037 §3 rule 5). The keyword answers; it does not
  admit.
- **Static folds:** when the receiver's static type already answers
  (concrete `x`, a monomorphized `T`, `d: I is I`) the result is a
  compile-time constant — folded, with an always-true/false lint
  (assertion use is legitimate).
- **No `upcast` builtin.** Widening to `I` is implicit on
  assignment/argument passing (`blit_all(g, [c])` passes a `Circle` as
  `Drawable`); the explicit, greppable form is the `dyn` annotation
  at the receiving position (`let d: Drawable = s;`); there is no
  `upcast` builtin — the keyword does the marking.
- **Trait objects cannot be downcast.** A interface object is used
  through its interface methods — if you need `Circle`-specific behavior
  behind a `Drawable`, put that behavior in the interface. Recovery
  of an erased value exists only through `Opaque` + `downcast<T>`
  (RFC 0014): erasure is explicit, so nothing dynamic ever flows through
  interface types. The capability probe is not a recovery path: it
  answers whether dispatch is possible, never hands back a narrower ref.
- Numeric conversions are the one cast: `expr as T` (RFC 0007 §1) —
  its RHS is a naming position, so types stay out of operand position
  otherwise. Enum and erasure conversions remain named type-calls:
  **`Opaque(v): Opaque`** (RFC 0014), the erasure builtin's class
  method — there is
  no `as` for anything but the numeric primitives.

## 4. Equality — `==` is builtin: value for primitives, identity for cells

`a == b` is a
builtin operator with no vtable dispatch, no opting in, no
element-wise story. The law is one sentence: **primitives compare by
value; `str` compares by content; everything else compares by cell
identity.** `a != b` is its negation (`!(a == b)`).

- Primitives: `icmp`/`fcmp` value comparison; floats follow IEEE 754
  (`NaN != NaN`, `-0.0 == 0.0`).
- `str`: content comparison (immutable; interned literals make
  identity accidentally work sometimes — content is the law, not the
  accident).
- `bytes`: content comparison (RFC 0004) — the engine lowers it to the
  generic content op `ArrayCmp` because `bytes` is a `u8` array; plain
  `Array<T>` stays identity (below).

  **v1.1:** structs are values (RFC 0009 §7), so they join the value
  side — `s == t` compares field by field (`ValEq`), recursing through
  the same law per field. Pointers stay on the identity side: `*T ==
  *T` is the cell-and-offset test, never a deep comparison. The old
  `own(x) == x` aliasing example is gone with `own` itself
  (RFC 0005 §10).
- **Everything else — class, dataclass, `Vec`, `Array`, enums,
  `Opaque`, `I` — is a handle test**: `a == b` is true exactly when
  both point at the same cell (RFC 0016 §1). Since every non-primitive
  is shared, this is aliasing made observable: `own(x) == x` is always
  `false`, two structurally identical literals are never equal, and a
  mutation does not change identity. Enum dataless variants are
  immortal singleton cells (RFC 0016 §1), so `Flavor.Sweet ==
  Flavor.Sweet` is `true` — the one place identity quietly behaves as
  value.
- **`==` on `Option<T>` / `Result<T, E>` is a compile error** (RFC 0005)
  — identity on freshly built sum cells is almost never the intent;
  compare with `when`, `.is_some()`, or the payload (`.value == d`).
- An **identity-compare lint** flags `==` between two obviously
  fresh composites (`Vec.from([..]) == Vec.from([..])`,
  `Point{..} == Point{..}`): "always false — compare fields, or
  `impl Hashable`" (assertion of distinctness is legitimate and
  suppressible).
- **Field-wise comparison is `Hashable.eq`** (RFC 0028): the interface
  declares `hash` + `eq` together — the value-keyed contract
  `Map`/`Set` keys ride (RFC 0026). It is not connected to `==`; a type
  may be `Hashable` (maps) while `==` stays identity.
- `when` literal patterns are unaffected: arms match compile-time values,
  never runtime `==`.

## 5. v1.1 — duck-typed interfaces

`impl Trait for Type` heads are removed. **Satisfaction is structural
and evaluated at the use site**: a type satisfies an interface when the
type itself declares a member for every interface method — same name,
same arity, same resolved signature under the instantiation. There is
nothing to opt into and no orphan rule; a value of the type flows into
an interface-typed slot wherever the shape matches, and a mismatch names
the missing member.

- **Vtables synthesize per (type × interface).** After the module
  compiles, every concrete data type (and every generic instantiation)
  that structurally satisfies an interface gets its slots filled from
  its own methods, which compile at that point with their transitive
  calls. Dead satisfactions cost a slot fill, nothing more.
- **Interface members are not overridable hooks** — the type's method
  IS the implementation, used by both direct and vtable dispatch.
- The engine contracts are gone from the prelude: `Index` (indexing is
  builtin over `[T]`/`Vec`/`str`/`bytes`; a `Vec` is recognized by its
  `buf`/`len` record shape), `Iterator` (§6, the `__iterate` protocol),
  and `Disposal` (`on_drop`, RFC 0016 v1.1). Library contracts without
  engine knowledge (`Hashable`) stay plain `interface` in their module.

## 6. The iteration protocol — `__iterate`

A type is **iterable** when it declares the protocol member:

```rut
interface Iterator<E> {
    fn __iterate(self, emit: fn(E) -> bool);
}
```

`Iterator<E>` lives in the `std:core` prelude (engine-woven — the `for`
desugar lowers to it; satisfaction is the ordinary duck law, so declaring
the member is all a type does). `for (v of it) { body }` desugars to
`it.__iterate(emit)` with a synthetic closure: the body runs, then
`emit` returns `true`; `break` returns `false` (stopping the iteration);
`continue` returns `true` immediately. Consequences:

- **The loop variable is the closure's parameter** — a fresh binding per
  iteration by construction (each `emit` call is a fresh frame).
- **Captures follow the closure law** — enclosing locals are copied by
  value at the desugar; accumulate through a shared cell (`*T`) or a
  method (`Vec.push`).
- `return` inside the body returns from the closure — stopping the
  iteration, not the enclosing function.

The builtin sequences (`Array<T>`, `Vec<T>`, `str`, `bytes`) keep their
fused index loops — never a per-element call (RFC 0032 §1.1 R2) — with
element types `T`, `T`, `str`, and `u8` respectively. Element yields as
`*T` (a pointer into the sequence's own slot) are the remaining
refinement; in this build the fused loops yield element values, which
the fresh-binding law already treats as per-iteration values.
