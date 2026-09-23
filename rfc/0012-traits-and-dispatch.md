# RFC 0012: Traits & Dispatch — Nominal Impls, Two-Rule Dispatch

- **Status:** Draft (v2 — the nominal rewrite; supersedes the v1.1
  duck-typed design in its entirety)
- **Date:** 2026-09-18
- **Author:** hpp2334
- **Depends on:** RFC 0009 (dataclass implementors), RFC 0010 (classes),
  RFC 0011 (reference semantics), RFC 0014 (`Opaque` — read after)
- **Supersedes:** RFC 0002 §5.5, §6, §6.1 (pre-restructure)
- **Part:** B — Language surface

## Summary

`trait I` — methods only, no bodies, no defaults. Satisfaction is
**nominal**: the admission is the impl block, and nothing else. Two impl
forms: **`impl T { .. }`** (inherent methods, the type's module only) and
**`impl I for T { .. }`** (trait impls, any module; duplicate
`(trait, type)` pairs are a link error). Dispatch follows two positive
rules — **static** when the call site names exactly one concrete type,
**vtable** when the receiver's concrete origins are multiple. Trait-typed
values spell the bare trait name (`d: Shape`) — there is no object-type
keyword. `requires` bounds, multiple impl blocks per type, and `is`
capability probes survive the rewrite; structural ("duck") satisfaction
does not.

## 1. Dispatch — the two-rule law

Every method call compiles under one of exactly two rules. There is no
"fallback" form, no best-effort mode, no hybrid: the rule is a property
of the call site, fixed at compile time.

- **Static dispatch — the call site names exactly one concrete type.**
  The receiver's concrete type is known and single; the call binds
  directly to the impl's method (a plain `CallM`), no vtable hop:
  - a **concrete receiver** — `c.area()` on `c: Circle`, including
    aliased handles (the type is still exact, RFC 0010 §3);
  - a **trait-typed local of single concrete origin** — `let d: Shape =
    Point { .. }; d.area()` binds straight to `Point`'s impl;
  - a **trait-typed parameter** — `fn blit_all(g: Canvas, s: Shape)`
    specializes per concrete argument at monomorphization (one clone per
    argument type — finite, terminating), so the body's `s.area()` calls
    are static; a trait parameter *is* an implicit generic bound;
  - a **monomorphized generic** — inside `fn first<T: Clone>(..)`, `T`'s
    members are static per instantiation.
- **Vtable dispatch — the receiver is trait-typed with multiple possible
  concrete origins.** The call consults the value's descriptor (RFC
  0015) and its per-(type × trait) method table (`CallI`, one global
  trait-method slot id per (trait, method), RFC 0015 §6):
  - **heterogeneous container elements** — `for (s of shapes)` over a
    `Vec<Shape>`;
  - **trait-typed field/element loads** — a `Shape`-typed field read out
    of a cell may hold any implementor;
  - **branch-merged origins** — `let s = if (c) { a } else { b };` where
    the arms carry different concretes into one trait-typed binding.

Origins are tracked per binding at compile time (the compiler's origin
counting is conservative by construction: any merge, any indirection
load, any cross-function flow of trait-typed values counts as multiple).
Mis-analysis cannot produce wrong code — an uncertain origin costs a
vtable hop, never a wrong static bind. The same descriptor that answers
vtable calls answers `is` probes (§3); there is one runtime truth per
value.

## 2. Traits

```rut
trait Shape {                           // keyword `trait`
    fn area(self) -> f64;               // bodiless signatures; async legal;
    fn scale(v: f64);                   //   no-self methods legal (engine contracts)
}

impl Shape for Point {                  // ANY module
    fn area(self) -> f64 { self.x as f64 }  // no `pub` — rides the trait's visibility
}

let d: Shape = Point { .. };            // bare trait name in type position
d.area();                               // static (single origin)
let mixed: Vec<Shape> = …;
for (s of mixed) { s.area(); }          // vtable (multiple origins)
```

- **`trait` is the kind word, and builtins spell theirs.** A user
  contract declares `trait Name`; an engine-woven contract declares
  `builtin trait Name` (§7) — the kind is always spelled, never implied
  (RFC 0029 §2). There is no object-type keyword anywhere in rut: a
  trait name in **type position** — parameter/return/local/field types,
  generic arguments — is the bare name (`d: Drawable`, `Vec<Widget>`,
  `Wrap<i32>` for a generic trait instantiation). Positions that merely
  **name** a trait stay bare too: the declaration, impl heads and
  `requires` lists, generic bounds (`K requires Hashable`, RFC 0013 §2).
- **Methods only, no bodies.** Traits declare **plain method
  signatures** — no fields, no properties of any kind (there is no
  `get`/`set` syntax in rut at all, RFC 0010 §2), and **no method
  bodies**: no default implementations, ever. One member kind, one
  dispatch candidate per call — a default body would be a second
  candidate and a law of its own. Anything that reads like a property is
  a method: `x.count()` in the trait, defined in the type's impl block.
  Methods are instance methods and spell the `self` receiver like every
  other (`fn draw(self, g: Canvas) -> nil;`), **except** where an engine
  contract declares a no-`self` method (§7): a `Task` resumption takes
  the run context as its parameter, not a receiver. `async fn` signatures
  are legal in traits; a trait impl's method must match the trait's
  `async` spelling exactly (RFC 0018 §2).
- **Satisfaction is nominal — the impl block is the admission.**
  Structural ("duck") conformity does not satisfy a trait: a type that
  declares every member by shape is still not an `I` until some module
  writes `impl I for T`. This keeps runtime type identity exact (RFC
  0015) and casts cheap, and it makes the registry the single source of
  truth for "who implements what". There is no orphan rule beyond
  placement (§4).
- **One impl per `(trait, type)` pair, program-wide.** A duplicate —
  two modules, or two blocks in one — is a **link error** (RFC 0038 §4
  merges the registries; the pair, not the block, is the unit).
- **Generic traits and generic targets.** `trait Wrap<T>` gives each
  type-argument list its own instantiation with its own trait id and
  vtable slots (`Wrap<i32>` ≠ `Wrap<str>`, RFC 0015 §6); `impl
  Hashable for Pair<A, B>` binds the target's generic args as the impl's
  type parameters. No impl-level `where` in v1 — bounds live inline on
  the fn/method's generic parameters (RFC 0013 §2, RFC 0043) and the
  trait's own `requires`.
- **The element is a type argument, not an associated type.** `impl
  Iterator<char> for str`, `impl Index<T> for Vec<T>` — there are no
  associated `type` members. The element type resolves at the use site
  from the impl's argument and stays reified in the type table. Two
  builtin contracts: `Index<T>` (`len`/`get`/`set`) drives `x[i]` and the
  indexed `for..of`; `Iterator<E>` (`__iterate`) drives cursor
  `for..of` (§6). Both are core decls like every prelude name (RFC
  0028) — the builtin `[T]`/`str`/`bytes` index themselves without the
  trait; user types reach the contracts through `use core::{ Index,
  Iterator };`. A type may have more than one element choice; the use
  site selects it.
- **Traits are implemented for classes, dataclasses** (RFC 0009), **and
  primitives**: `impl Hashable for i32` registers like any trait impl —
  any module may write it, and the duplicate rule is the §5 link check
  — while an inherent `impl i32 { .. }` diagnoses: a primitive's
  inherent surface is core's `builtin impl` (RFC 0032 §1.1). A
  primitive widens to `I` through the same nominal gate; its
  single-origin dispatch binds statically, the scalar receiver crossing
  as an ordinary argument.
- **A trait-typed value is never exactly typed**: every trait-typed
  slot is a fat ref over a cell whose exact class or dataclass it
  carries (RFC 0015 §6). A trait type is **unsized** — the payload
  lives in a heap cell and the slot stores the cell handle (RFC 0031
  §4).
- **No top trait.** The erased-storage type is the concrete host class
  `Opaque` (RFC 0014) — reached by the explicit type-call `Opaque(v)`,
  never by widening, and never nameable in an `impl`/`requires` list (it
  is a class, not a trait). The tier is simply `exact concrete > I`
  (RFC 0031 §4); the universal type test is `x is Opaque` (§3).
- **Intersection types (`A & B`) are never supported** — not deferred,
  not planned: heterogeneous needs compose a trait that declares both
  method sets. This is a design principle, not a v1 limitation.

## 3. Type tests — the `is` keyword

`as` is the numeric cast and nothing else — `expr as T`, truncating,
RHS a naming position restricted to the numeric primitives (RFC 0007
§1) — but type tests are the **`is` keyword**: `expr is Type` → `bool`.

- **Grammar:** `expr is Type` at relational precedence,
  non-associative (RFC 0030 §2/§3). The RHS is a **naming position**
  like `impl`/`requires` lists (§2): a bare trait name or instantiation
  (`x is Hashable`, `x is Wrap<i32>`) or a concrete type (`d is Circle`).
  Every type test spells `is` — there is no `is<T>()` builtin.
- **Two probes, one keyword.** Concrete RHS — exact-type test: true when
  `x`'s exact class (or dataclass — RFC 0009) *is* `T`; with no
  inheritance this is a single descriptor lookup — exact `TypeId`
  compare (RFC 0015 §6). Trait RHS — **capability probe**: true when the
  value's exact type has a registered impl for that trait — the
  descriptor/registry scan of RFC 0015 §6, user-reachable (`k is
  Hashable`, `x is Drawable`). `is` is total: never traps, never
  recovers, yields only `bool`. The same descriptor answers the vtable
  (§1) and the probe — one runtime truth per value.
- **`x is Opaque` is legal** — a plain concrete test ("is this value an
  `Opaque` handle?"), folding to `true` on `Opaque`-typed receivers with
  the usual lint. On an `Opaque` receiver, `o is T` / `o is I` **see
  through the box**: they test the boxed value's type (RFC 0014).
- **No flow sensitivity:** `if (x is Hashable) { .. }` grants nothing —
  no narrowing, no widening of `x` to `I` (a bound proves widening, RFC
  0037 §3 rule 5). The keyword answers; it does not admit.
- **Static folds:** when the receiver's static type already answers
  (concrete `x`, a monomorphized `T`, `d: I is I`) the result is a
  compile-time constant — folded, with an always-true/false lint
  (assertion use is legitimate).
- **No `upcast` builtin.** Widening to `I` is implicit on
  assignment/argument passing (`blit_all(g, [c])` passes a `Circle` as
  `Drawable`) — nominal admission is the only gate it consults (§4);
  the explicit, greppable form is the trait annotation at the receiving
  position (`let d: Drawable = s;`). There is no `upcast` builtin — the
  annotation does the marking.
- **Trait-typed values cannot be downcast.** A trait-typed value is used
  through its trait's methods — if you need `Circle`-specific behavior
  behind a `Drawable`, put that behavior in the trait. Recovery of an
  erased value exists only through `Opaque` + `downcast<T>` (RFC 0014):
  erasure is explicit, so nothing dynamic ever flows through trait
  types. The capability probe is not a recovery path: it answers whether
  dispatch is possible, never hands back a narrower ref.
- Numeric conversions are the one cast: `expr as T` (RFC 0007 §1) — its
  RHS is a naming position, so types stay out of operand position
  otherwise. Enum and erasure conversions remain named type-calls:
  **`Opaque(v): Opaque`** (RFC 0014), the erasure builtin's class method
  — there is no `as` for anything but the numeric primitives.

## 4. Impl blocks — inherent and trait forms

Methods live in impl blocks. Type bodies are **fields only** (RFC
0009/0010): a `fn` member in a `struct`/`class` body is a hard parse
error — no compatibility mode, the migration is total.

| | `impl T { … }` | `impl I for T { … }` |
|---|---|---|
| Lives in | `.rut`, **T's module only** | `.rut`, **any module** |
| Valid targets | local `struct`/`class`, or a `builtin class` this module declares (RFC 0029 §2) | any nominal type — foreign trait for foreign type is legal |
| `pub(..)` | ✅ classes only (RFC 0009; structs are all-public) | ❌ — trait impl methods are as visible as the trait |
| `async` | ✅ | ✅ — must match the trait's signature |
| no-`self` methods | ✅ (constructors, `on_finish`) | ✅ (declared by the trait, §7) |
| fields / empty body | ❌ / legal | ❌ / legal (empty = the opt-in marker, RFC 0037) |

```rut
impl Point {                            // inherent — Point's module only
    fn new(x: i32, y: i32) -> Point { .. }      // class methods (no self)
    pub async fn save(mut self) -> nil { .. }   // pub: classes only (RFC 0009)
}
```

- **Inherent placement.** `impl T { .. }` compiles only in the module
  that declares `T` — anywhere else is a compile error. The target may
  be a local type **or a `builtin class` this module's `.d.rut` declares**
  (the `LaunchedTask<T>` pattern, RFC 0028): the module owns the type,
  so it owns the methods.
- **Trait-impl placement is free.** `impl I for T` may live in any
  module — a module may adapt a foreign trait to a foreign type. The
  registry merges at link time and the duplicate-`(trait, type)` rule
  (§2) is the only constraint.
- **Bodies match the trait exactly.** Each method in a trait impl block
  matches the trait's signature — receiver form (`self`/`mut self`),
  params, return type, `async` spelling; a missing signature is a
  compile error, and so is any extra method inside the block (put those
  in an inherent block). Trait impl methods carry no `pub` — they are as
  visible as the trait. An **empty** trait impl block is legal and is
  the opt-in marker: `impl Serializable for User {}` (RFC 0037).
- **Widening is nominal.** A value of `T` widens to `I` exactly when the
  registry holds an `impl I for T` visible to the call site (§6) — and
  never otherwise. `S` with the right member shapes but no impl does not
  widen, and `s is I` folds false.
- **`Self` in impl signatures.** Inside an impl block, `Self` names the
  impl's target under the impl's substitution — `-> Self` returns,
  `Self { .. }` constructs (RFC 0010 §1).

## 5. Cross-module registry — link-time merge

- **Module surfaces export impl registrations**: trait decls (name,
  generics, method signatures), inherent-method signatures on
  `type_exports`, and `(trait, target, [method → fn ref])` triples.
- **The link merges the registries** (RFC 0038 §4): global trait ids and
  slot layout, cross-scope vtable fill, and the duplicate-pair check —
  two modules registering `(I, T)` is a link error naming both.
- **Trait-typed parameters specialize at link time** — one clone of the
  callee per concrete argument type (finite, terminating), which is what
  keeps §1's static rule honest across module boundaries.
- **`requires` closure is compile-time for local impls, link-time
  cross-module**: to register `impl Serializable for T`, the registering
  module must also hold (or the link must find) `impl Reflectable for T`
  — transitively closed (RFC 0037).

## 6. The use-both gate; the iteration protocol

**The gate.** `x.trait_method()` requires **both** the type and the
trait to be named at the call site's module: the type by declaration or
`use`, the trait by `use`. Inherent methods follow the type alone. The
impl block is the "hint" that names where methods come from — a call
that matches a registered impl whose trait no `use` names is an error:
"use `I` to call its methods on `T`". (The engine's prelude is used,
never ambient — RFC 0028: nothing is in scope until a module writes
`use core::{ .. };`.)

**The iteration protocol.** A type is iterable when it registers
`impl Iterator<E> for T` (nominal — §4):

```rut
use core::{ Iterator, make_ptr };

impl Iterator<i32> for CountUp {
    fn __iterate(self, emit: fn(i32) -> bool) {
        for (let i = 1; i <= self.n; i += 1) {
            if (!emit(i)) { return; }
        }
    }
}
```

`for (v of it) { body }` desugars to `it.__iterate(emit)` with a
synthetic closure: the body runs, then `emit` returns `true`; `break`
returns `false` (stopping the iteration); `continue` returns `true`
immediately. Consequences:

- **The loop variable is the closure's parameter** — a fresh binding per
  iteration by construction (each `emit` call is a fresh frame).
- **Captures follow the closure law** — enclosing locals are copied by
  value at the desugar; accumulate through a shared cell (`*T`) or a
  method (`Vec.push`).
- `return` inside the body returns from the closure — stopping the
  iteration, not the enclosing function.

The builtin sequences (`[T]`, `Vec<T>`, `str`, `bytes`) keep their
fused index loops — never a per-element call (RFC 0032 §1.1 R2); they
index themselves without the trait. The loop variable's type is `*T`,
`*T`, `str`, and `u8` respectively: for the value sequences each
iteration boxes the element into a fresh one-slot cell — ref-typed
elements alias the stored slot, so writes through the loop variable
mutate the sequence itself, and the fresh box keeps the per-iteration
binding law. Scalars deref automatically: a `*T` reads as `T` at
value-expected positions (arguments, returns, lets, assignments, format
holes), in arithmetic and ordinal operands, and in `==`/`!=` against the
pointee type. `*T == *T` stays identity, and pointer-vs-pointer is
untouched everywhere — `p == nil` compares pointers.

## 7. Engine contracts — `Task` and the run contexts

The engine names builtin traits; it does not close them. A `builtin
trait` (RFC 0029 §2) is compiler-backed — the engine auto-implements it
for desugared frames — but **users implement it through the ordinary
nominal path**: `impl Task<T> for CustomTask<T>` registers in the same
registry as any other impl. Builtin traits are engine-*named*, not
engine-*closed*.

The async plan freezes the final member set; the shape it freezes to:

```rut
// core — engine-woven async surface (illustrative; no async module exists)
pub builtin trait Task<T> { fn yield(cx: TaskRunContext); }
pub builtin trait TaskRunContext {
    fn checkpoint(self) -> u32;
    fn next_checkpoint(mut self, v: u32) -> nil;   // mut receiver: it writes
    fn cancelled(self) -> bool;
}
pub builtin fn launch_task<T>(t: Task<T>) -> LaunchedTask<T>;
pub builtin class LaunchedTask<T> { }
impl LaunchedTask<T> { fn on_finish(v: T) -> Self { .. } }
```

All members are **methods** — one member kind, no call-vs-load ambiguity
at the vtable boundary; non-`self` parameters are legal (§2) and are how
engine contracts spell frame entry points. Cancellation is the
**context probe**: the desugared frame checks `cx.cancelled()` after
each resumption and runs its drop path (RFC 0018). `async fn(cx:
TaskRunContext, …)` takes the context as its explicit first parameter.
`launch_task` lives in **core** (there is no async module — RFC 0028);
users may write their own launchers over the same `Task` surface —
`examples/04-custom-async` is that user launcher, and doubles as the
user-impl-of-a-builtin-trait test.

## 8. Equality — `==` is builtin: value for primitives, identity for cells

`a == b` is a builtin operator with no vtable dispatch, no opting in,
no element-wise story. The law is one sentence: **primitives compare by
value; `str` compares by content; everything else compares by cell
identity.** `a != b` is its negation (`!(a == b)`).

- Primitives: `icmp`/`fcmp` value comparison; floats follow IEEE 754
  (`NaN != NaN`, `-0.0 == 0.0`).
- `str`: content comparison (immutable; interned literals make identity
  accidentally work sometimes — content is the law, not the accident).
- `bytes`: content comparison (RFC 0004) — the engine lowers it to the
  generic content op `ArrayCmp` because `bytes` is a `u8` array; plain
  `[T]` stays identity (below).

  Structs are values (RFC 0009 §7), so they join the value side —
  `s == t` compares field by field (`ValEq`), recursing through the same
  law per field. Pointers stay on the identity side: `*T == *T` is the
  cell-and-offset test, never a deep comparison.
- **Everything else — class, dataclass, `Vec`, `[T]`, enums,
  `Opaque`, `I` — is a handle test**: `a == b` is true exactly when both
  point at the same cell (RFC 0016 §1). Since every non-primitive is
  shared, this is aliasing made observable: two structurally identical
  literals are never equal, and a mutation does not change identity.
  Enum dataless variants are immortal singleton cells (RFC 0016 §1), so
  `Flavor.Sweet == Flavor.Sweet` is `true` — the one place identity
  quietly behaves as value.
- **`==` on `Option<T>` / `Result<T, E>` is a compile error** (RFC 0005)
  — identity on freshly built sum cells is almost never the intent;
  compare with `when`, `.is_some()`, or the payload (`.value == d`).
- An **identity-compare lint** flags `==` between two obviously fresh
  composites (`Vec.from([..]) == Vec.from([..])`, `Point{..} ==
  Point{..}`): "always false — compare fields, or `impl Hashable`"
  (assertion of distinctness is legitimate and suppressible).
- **Field-wise comparison is `Hashable.eq`** (RFC 0028): the trait
  declares `hash` + `eq` together — the value-keyed contract `Map`/`Set`
  keys ride (RFC 0026). It is not connected to `==`; a type may be
  `Hashable` (maps) while `==` stays identity.
- `when` literal patterns are unaffected: arms match compile-time
  values, never runtime `==`.

---

## Amendment (Sep 2026): peer-gated impl groups — trait-local, gated before placement

The dep kinds (RFC 0045) add *peer groups*: impl-only `.rut` files a
package appends to its own source only when an optional peer is
present in the consumer's closure (e.g. json's `impl JsonSerialize for
Vec<T>`, mounted only when the consumer carries pouch). Their
placement law, recorded here so the two RFCs agree:

- **The peer-gated impl stays trait-local.** A group's impls are
  trait impls in the declaring pkg — legal by §2's placement rule
  ("trait impls are legal in any module"): the trait is the
  declaring pkg's own, and the declaring pkg is where the impl lives.
  No orphan question exists; the group is the trait's home module
  speaking about the peer's type, which is exactly the shape §2
  blesses.
- **The gate precedes the orphan check.** The peer gate runs at LOAD
  (RFC 0045 §3, one post-closure pass); placement and pair-uniqueness
  run at COMPILE/LINK. The checker never sees a half-mounted world:
  peer absent → the group text was never assembled → there is no
  impl anywhere to place; peer present → the target type resolves
  (the group's `use` of the peer binds it — mounted by definition).
- **§5's duplicate-pair link error is the consumer-side guard**: a
  consumer who hand-writes the same `(trait, type)` pair in a world
  where the group mounted gets the link-time duplicate — loud and
  correct, since the group already provides the impl. Documented
  behavior, not a gap.
- Groups are impl-only by law (RFC 0045 §3): they declare no new
  public names, which is what keeps every peer-related miss on a
  dedicated, peer-aware diagnostic path instead of a bare
  unresolved-name.
