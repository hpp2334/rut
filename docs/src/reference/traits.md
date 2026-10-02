# Traits and dispatch

`trait` — methods only, no bodies, no defaults. Satisfaction is
**nominal**: the impl block is the admission, and nothing else. Dispatch
follows two positive rules, fixed at compile time per call site.

## Traits

```rut
trait Shape {
    fn area(self) -> f64;        // bodiless signatures; async legal
    fn scale(v: f64);            // no-self methods legal (engine contracts)
}
```

- **Methods only, no bodies.** No fields, no properties, and **no
  default implementations, ever** — one member kind, one dispatch
  candidate per call. Anything that reads like a property is a method.
- Methods are instance methods and spell the `self` receiver like every
  other method (`fn draw(self, g: Canvas) -> nil;`), except where an
  engine contract spells a receiver-less descriptor method (`Future`).
  `async fn` signatures are legal; an impl's method must match the
  trait's `async` spelling exactly.
- Trait members carry no `pub` — they are as visible as the trait.
- **No object-type keyword anywhere**: a trait name in *type position*
  is the bare name — `d: Drawable`, `Vec<Widget>`, generic arguments
  included.
- **No top trait.** The erased-storage type is the concrete primitive
  `opaque` (see [opaque — erasure and downcast](opaque.md)), reached by
  an explicit call, never by widening.
- **Intersection types (`A & B`) are never supported** — not deferred:
  heterogeneous needs compose a trait that declares both method sets.

## Impl blocks

Type bodies are fields only; methods live in impl blocks.

| | `impl T { .. }` (inherent) | `impl I for T { .. }` (trait) |
|---|---|---|
| Lives in | `.rut`, **T's module only** | `.rut`, any module of **the trait's pkg or the type's pkg** |
| Valid targets | local `struct`/`class`, or a `builtin class` this module declares | any nominal type — **at least one of the pair must be local to this pkg** |
| `pub(..)` | classes only (structs are all-public) | never — as visible as the trait |
| `async` | legal | legal — must match the trait's signature |
| no-`self` methods | legal (constructors) | legal where the trait declares them |
| fields / empty body | never / legal | never / legal (empty = the opt-in marker) |

```rut
use ink::{ Logger };

trait Shape { fn area(self) -> f64; }
trait Serializable {}

struct Point { x: i32; }
struct User { name: str; }

impl Shape for Point {
    fn area(self) -> f64 { return self.x as f64; }
}
impl Serializable for User {}      // empty trait impl = the opt-in marker

entry fn main() {
    let log = Logger.new("t");
    let p = Point { x: 5 };
    log.info(f"{p.area()}");
}
```

```text
5
```

- **Satisfaction is nominal.** A type that declares every member by
  shape is still not an `I` until some module writes
  `impl I for T`. There is no duck typing and no orphan rule beyond
  placement: for every `impl Trait for Type`, **at least one of `Type`
  or `Trait` must be defined in the current pkg** — both foreign is a
  compile error. Builtin types (`[T]`, the primitives, `?T`, `opaque`)
  are in no pkg: only a *local trait* may be implemented for a builtin.
  This placement rule is the **only** cross-module impl restriction —
  a foreign trait crosses freely for a local type, generic or not.
- **One impl per `(trait, type)` pair, program-wide.** A duplicate —
  two modules, or two blocks in one — is a link error.
- **Bodies match the trait exactly**: receiver form (`self`/`mut
  self`), params, return type, `async` spelling. A missing signature is
  an error; so is any extra method in the block (put those in an
  inherent block).
- Traits are implemented for classes, structs, **and primitives**
  (`impl Hashable for i32` registers like any trait impl), while an
  inherent `impl i32 { .. }` diagnoses — a primitive's inherent surface
  belongs to the engine.
- **Generic traits and generic targets**: `trait Wrap<T>` gives each
  type-argument list its own instantiation (`Wrap<i32>` ≠ `Wrap<str>`).
  Generic binders are **declared after `impl`** — `impl<A, B>
  Hashable for Pair<A, B>` — and that list is the definition site: a
  bare parameter name in the head is a use that must resolve against
  it (an undeclared name is the error it always should have been —
  "undeclared type parameter `T` — declare it: `impl<T> ..`"). The
  same law covers inherent impls: `impl<T> Vec<T> { .. }`, not
  `impl Vec<T> { .. }`. Parameterized trait impls are legal:
  `impl<T> Readable<T> for Source<T>` registers a **template** serving
  every concrete instantiation; a hand-written concrete impl shadows
  the template; repeated parameters (`impl<T> W<T, T> for Pair2<T>`)
  are legal. Each trait argument must be a concrete type or a declared
  binder that names one of the target's own parameters.
- **Generic traits cross modules.** A consumer implements a foreign
  generic trait for its own type — `impl<T> Wrap<T> for Box2<T>`
  against a `use`d pkg's `trait Wrap<T>` — and spells the trait in
  type position (`fn describe(w: Wrap<i32>) -> i32`). The trait's
  declaration crosses the used pkg's surface, each type-argument list
  instantiates it where it is used, and dispatch is the ordinary law:
  one concrete origin binds statically, merged origins consult the
  vtable. The orphan rule above is the only gate.
- **The element is a type argument, not an associated type**:
  `impl Iterable<char> for Counter` — there are no associated `type`
  members.
- **`Self` in impl signatures** names the impl's target under the impl's
  substitution: `-> Self` returns, `Self { .. }` constructs.

## Dispatch — the two-rule law

Every method call compiles under exactly one of two rules; the rule is
a property of the call site, fixed at compile time.

- **Static** — the call site names exactly one concrete type; the call
  binds directly to the impl's method, no vtable hop:
  - a concrete receiver — `c.area()` on `c: Circle`;
  - a trait-typed local of **single concrete origin** —
    `let d: Shape = Point { .. }; d.area()` binds straight to `Point`'s
    impl;
  - a trait-typed **parameter** — the callee specializes per concrete
    argument type (one clone per argument type — finite, terminating),
    so a trait parameter *is* an implicit generic bound;
  - a monomorphized generic — inside `fn first<T>(..)`, `T`'s members
    are static per instantiation.
- **Vtable** — the receiver is trait-typed with multiple possible
  concrete origins; the call consults the value's descriptor and its
  per-(type × trait) method table:
  - heterogeneous container elements — `for (s of shapes)` over a
    `Vec<Shape>`;
  - trait-typed field/element loads;
  - branch-merged origins — `let s = if (c) { a } else { b };` where
    the arms carry different concretes into one trait-typed binding.

Origin counting is conservative: any merge, indirection load, or
cross-function flow counts as multiple. Mis-analysis cannot produce
wrong code — an uncertain origin costs a vtable hop, never a wrong
static bind.

**The use-both gate**: `x.trait_method()` requires **both** the type
and the trait to be named at the call site's module — the type by
declaration or `use`, the trait by `use`. A call that matches a
registered impl whose trait no `use` names is an error: "use `I` to
call its methods on `T`".

**Widening is nominal and implicit**: a value of `T` widens to `I`
exactly where the registry holds a visible `impl I for T` — on
assignment, argument passing, and returns. The explicit, greppable
form is the trait annotation at the receiving position
(`let d: Drawable = s;`). A trait-typed
value **cannot be downcast**: use it through the trait, or erase
explicitly through `opaque`.

## Type tests — `is`

`expr is Type` → `bool`, at relational precedence, non-associative.
The right-hand side is a naming position: a concrete type or a bare
trait name/instantiation.

- **Concrete RHS — exact-type test**: true when the value's exact class
  or struct *is* `T`. With no inheritance this is a single descriptor
  lookup.
- **Trait RHS — capability probe**: true when the value's exact type
  has a registered impl for that trait (`k is Hashable`).
- **No flow sensitivity**: `if (x is Hashable) { .. }` grants nothing —
  no narrowing, no widening. The keyword answers; it does not admit.
- **Static folds**: when the receiver's static type already answers,
  the result is a compile-time constant, with an always-true/false lint.
- `is` is total: never traps, yields only `bool`. The same descriptor
  answers the vtable and the probe — one runtime truth per value.

## The iteration protocol

A type is iterable when it registers `impl Iterable<E> for T`:

```rut
use core::{ Iterable };
use ink::{ Logger };

class CountUp {
    n: i32;
}

impl CountUp {
    pub fn new(n: i32) -> Self { return Self { n: n }; }
}

impl Iterable<i32> for CountUp {
    fn iterate(self, emit: fn(i32) -> bool) {
        for (let i = 1; i <= self.n; i += 1) {
            if (!emit(i)) { return; }
        }
    }
}

entry fn main() {
    let log = Logger.new("t");
    for (let v of CountUp.new(3)) {
        log.info(f"tick {v}");
    }
}
```

```text
tick 1
tick 2
tick 3
```

An enum value iterates the same way: `impl Iterable<E> for Color`
makes `for (let v of c)` walk whatever the impl's `iterate` emits —
the desugar is the trait, the target's kind is irrelevant (see
[Enums](enums.md)).

`for (v of it) { body }` desugars to `it.iterate(emit)` with a
synthetic closure: the body runs, then `emit` returns `true`; `break`
returns `false` (stopping the iteration); `continue` returns `true`
immediately; a `return` inside the body stops the iteration (not the
enclosing function). The loop variable is the closure's parameter — a
fresh binding per iteration; captured enclosing locals are copied by
value at the desugar, so accumulate through a shared cell or a method.
The builtin sequences (`[T]`, `Vec<T>`, `str`, `bytes`) keep their
fused index loops and never reach the protocol.

## Engine contracts

`builtin trait` names are compiler-backed but **engine-named, not
engine-closed** — users implement them through the ordinary nominal
path:

```rut
pub builtin trait Iterable<E> {
    fn iterate(self, emit: fn(E) -> bool);
}
pub builtin trait Future<T> { fn yield(cx: RunContext); }
pub builtin trait RunContext {
    fn checkpoint(self) -> u32;
    fn next_checkpoint(mut self, v: u32) -> nil;
    fn cancelled(self) -> bool;
}
pub builtin trait Disposal {
    fn dispose(mut self, cx: DisposalContext);
}
```

`impl Future<nil> for CustomFuture` registers in the same registry as
any other impl. Every builtin trait is the import-gated `pub builtin`
spelling: the ENGINE weaves on the native-trait symbols — async frames,
the minted cx, and the fused `for..of` loops never consult user scope —
but source that SPELLS a trait name resolves it only through
`use core::{ .. }` (`Iterable` for an `impl Iterable<E> for T` or a
trait-typed parameter; `Future` for a user impl, a launcher's
`f: Future<T>`, or `downcast<Future<..>>`; `RunContext` for a yield
signature or an `async fn` head; the `Disposal` pair through
`use core::{ Disposal, DisposalContext }` — see
[Host fns and declaration files](host-fns.md)). See
[Async and await](async.md) and [the Rc heap](rc-heap.md).

## Equality

`==` is a builtin operator with no vtable dispatch: primitives by
value, `str`/`bytes` by content, everything else by cell identity —
see [Rc, dispose, and identity](rc-dispose-identity.md). Field-wise
comparison is a `Hashable`-style contract implemented per type; it is
not connected to `==`.
