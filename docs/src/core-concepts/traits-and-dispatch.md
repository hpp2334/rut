# Traits and dispatch

A trait in rut is a **named contract**: a list of method signatures that
a type can implement — by name, in an impl block. Satisfaction is
nominal: the impl block is the admission, and nothing else. A type whose
members happen to match a trait's shapes is *not* an implementor; some
module must say `impl I for T`.

## The shape of a trait

```rut
use ink::{ Logger };

trait Shape {
    fn area(self) -> f64;
    fn scale(v: f64);
}

struct Point { x: f32; y: f32 }

impl Shape for Point {
    fn area(self) -> f64 { return self.x as f64; }
    fn scale(v: f64) { }    // the impl must list every trait method
}

pub fn main() {
    let log = Logger.new("traits");
    let p = Point { x: 3, y: 4 };
    log.info(f"area = {p.area()}");     // a concrete receiver: static
}
```

```text
area = 3
```

- **Methods only, no bodies.** A trait declares signatures — no fields,
  no properties, and no default implementations, ever. One member kind
  means one dispatch candidate per call; a default body would be a
  second candidate with a resolution law of its own.
- **Two impl forms.** `impl T { .. }` defines the type's *inherent*
  methods and may live only in the module that declares `T`.
  `impl I for T { .. }` defines a trait implementation; its methods are
  exactly the trait's, with no `pub` (they are as visible as the trait).
- **One impl per pair, program-wide.** Two modules implementing the same
  trait for the same type is a link error that names both. The impl
  registry — merged when modules link — is the single source of truth
  for "who implements what".
- **Placement is pair-local.** A trait impl may live in the trait's
  package or the type's package — at least one side of every
  `(trait, type)` pair must be yours. Implementing two foreign types'
  pairing is rejected outright; there is no orphan rule beyond that.
- **Any nominal type can be a target** — classes, structs, and even
  primitives (`impl MyTrait for i32` registers like any other impl).
  Traits may be generic (`Wrap<T>`, `Iterator<E>`); each instantiation
  has its own identity and its own method slots, and an impl may be
  parameterized by the target's own type parameters — `impl
  Encode<T> for Store<T>` registers a template that serves every
  instantiation, unless a concrete impl shadows it.

Type bodies are **fields only** — a `fn` inside a `struct` or `class`
body is a parse error. Every method, inherent or trait, lives in an
impl block.

## Trait-typed values

A trait name in type position is the bare name — there is no
object-type keyword:

```rut
use ink::{ Logger };
use pouch::{ Vec };

trait Shape {
    fn area(self) -> f64;
}

struct Circle { r: f64 }
struct UnitSquare { side: f64 }

impl Shape for Circle {
    fn area(self) -> f64 { return 3.14159265358979 * self.r * self.r; }
}

impl Shape for UnitSquare {
    fn area(self) -> f64 { return self.side * self.side; }
}

struct Canvas { log: Logger }

impl Canvas {
    fn render(self, a: f64) { self.log.info(f"blitted a shape of area {a}"); }
}

fn blit(g: Canvas, s: Shape) { g.render(s.area()); }   // a parameter over any Shape

pub fn main() {
    let log = Logger.new("shapes");
    let g = Canvas { log: log };
    let mut mixed: Vec<Shape> = Vec.new();   // heterogeneous storage
    mixed.push(Circle { r: 1.0 });
    mixed.push(UnitSquare { side: 2.0 });
    for (let s of mixed) { blit(g, s); }
}
```

```text
blitted a shape of area 3.14159265358979
blitted a shape of area 4
```

A trait-typed value is a reference to a real cell that still carries its
exact class. Widening a concrete value to a trait it implements is
implicit on assignment and argument passing, allocates nothing, and is
gated by exactly one question: does the registry hold an `impl I for T`?
There is no top type to widen to, and intersection types (`A & B`) are
ruled out by design — when you need both contracts, declare a trait that
spells both.

## Dispatch: the two-rule law

Every method call compiles under one of exactly two rules, fixed at
compile time as a property of the *call site*:

1. **Static** — the call site names exactly one concrete type. The call
   binds directly to that type's method, no table involved:
   - a concrete receiver (`c.area()` on `c: Circle`);
   - a trait-typed local whose single concrete origin the compiler
     tracked (`let d: Shape = Point { .. }; d.area()`);
   - a trait-typed *parameter* — the function specializes per concrete
     argument at monomorphization, so a trait parameter is effectively
     an implicit generic bound;
   - inside monomorphized generics.
2. **Vtable** — the receiver is trait-typed with multiple possible
   concrete origins:
   - elements of a heterogeneous container (`Vec<Shape>`);
   - trait-typed field and element loads (the cell may hold any
     implementor);
   - branch-merged bindings (`if` arms carrying different concretes
     into one trait-typed variable).

The origin analysis is conservative: any merge, indirection, or
cross-function flow counts as multiple. Mis-analysis cannot produce
wrong code — an uncertain origin costs one vtable hop, never a wrong
static bind. And the same descriptor that answers vtable calls answers
`is` probes: one runtime truth per value.

## Type tests

`expr is Type` yields a `bool` and is total — it never traps. Two
probes, one keyword:

- **Concrete RHS**: is the value's exact type `T`? (No inheritance — a
  single id compare.)
- **Trait RHS**: does the value's exact type have a registered impl?
  A capability probe, usable on any value (`k is Hashable`).

`is` has no flow-sensitive effects: a true probe does not narrow `x`.
When the static type already answers, the probe folds at compile time.
Trait-typed values cannot be downcast to a concrete type at all — if
you need `Circle`-specific behavior behind a `Drawable`, put that
behavior in the trait. The one recovery path for erased values is the
explicit `opaque` box (see the reference on
[`opaque`](../reference/opaque.md)).

## Visibility: the use-both gate

Calling a trait method requires **both** names in scope: the type (by
declaration or `use`) *and* the trait (`use`). Inherent methods need
only the type. A call that matches a registered impl whose trait no
`use` brings in is an error — "use `I` to call its methods on `T`" —
so an added trait method can never silently change what someone else's
call site means.

## The iteration protocol

A type is iterable when it implements the builtin `Iterator<E>` trait:

```rut
use ink::{ Logger };

class CountUp {
    n: i32;
}

impl Iterator<i32> for CountUp {
    fn __iterate(self, emit: fn(i32) -> bool) {
        for (let i = 1; i <= self.n; i += 1) {
            if (!emit(i)) { return; }
        }
    }
}

pub fn main() {
    let log = Logger.new("iter");
    for (let v of CountUp { n: 3 }) {
        log.info(f"tick {v}");
    }
    log.info("done");
}
```

```text
tick 1
tick 2
tick 3
done
```

`for (let v of it) { body }` desugars to `it.__iterate(emit)` with a
synthetic closure: the body runs, then `emit` returns `true`; `break`
returns `false`. The loop variable is the closure's parameter — a fresh
binding per iteration by construction. The builtin sequences (`[T]`,
`str`, `bytes`, and the standard growable `Vec`) keep fused index loops
instead; they never pay a per-element call.

## Engine contracts are traits too

The async machinery is spelled as builtin traits — `Future<T>` and
`RunContext` — which the engine *names* but does not close: a
hand-written type can `impl Future<nil> for MyFuture` through the same
registry as any other impl and be driven by the same loop. See
[the async model](async-model.md) and the worked example in
[04 — Custom async](../examples/04-custom-async.md).

## Equality is not a trait

`==` is builtin and cannot be opted into or out of: primitives compare
by value, `str`/`bytes` by content, everything else by cell identity.
Field-wise comparison is a loop you write, or the map packages' own key
rules. See [everything is a value](everything-is-a-value.md).

## What this buys you

- **Nominal satisfaction** keeps the registry exact: capability probes,
  widening, and vtable fills all read one table, so casts stay cheap and
  runtime identities stay meaningful.
- **The two-rule law** makes dispatch predictable: you can tell, per
  call site, whether you are paying a hop.
- **Placement rules** keep coherence decidable across packages without
  a global uniqueness proof — one of the pair is always local.
