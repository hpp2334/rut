# Interfaces and dispatch

An interface in rut is an **observed capability**: a list of method
signatures that a type satisfies by *having the members*. Satisfaction
is structural — checked once, at the boundary where a concrete value
is used as the interface. There is no admission step: no registration,
no impl block for an interface, nothing a type can join by saying so.

## The philosophy

Every polymorphism mechanism in rut answers to one table of equations;
they are worth internalizing before any syntax:

- **inherent = contract** — a type's inherent impl is its contract:
  the members it actually has, visible to whoever can name the type.
- **interface = observed capability** — an interface never changes a
  type; it is a member set some code observes the type through.
- **newtype = manufacture** — a wrapper class manufactures a member
  set for a type that cannot carry one (a primitive, a composite);
  the call site spells the manufacture, every time.
- **marker = engine hook** — the bracket markers (`[disposal]`,
  `[iterable]`) are engine contract slots, not polymorphism.
- **closed class = engine value** — `Future<T>` and `RunContext` are
  engine-minted; no user spelling can become one.

And the corollaries that keep the model honest:

- **Scope changes spelling, never meaning.** What a module imports
  changes which names resolve, never what a type can do.
- **One job per mechanism.** The member-set check admits; the itable
  dispatches; the marker designates. No mechanism does two jobs.
- **No mechanism may make a type's capability depend on imports.** A
  type's members are where the type is declared; no `use` anywhere can
  add or remove them.
- **Wrappers are explicit manufacture at the call site.** A primitive
  crosses into polymorphism inside a spelled wrapper construction —
  never auto-inserted.
- **Satisfaction is boundary checking, never resolution search.** The
  compiler checks the member set at param-passing, typed lets/assigns,
  returns, and boxing sites; it never scans the program for "who
  might implement this".

## The shape of an interface

```rut
use ink::{ Logger };

pub interface Area {
    fn area(self) -> f64;
}

class Point {
    x: f64;
    y: f64;
}

impl Point {
    pub fn area(self) -> f64 { return self.x * self.y; }
}

entry fn main() {
    let log = Logger.new("areas");
    let p = Point { x: 3, y: 4 };
    log.info(f"area = {p.area()}");     // a concrete receiver: static
}
```

```text
area = 3
```

- **Signatures only, no bodies.** An interface declares members — no
  fields, no properties, and no default implementations, ever. One
  member kind means one dispatch candidate per call; a default body
  would be a second candidate with a resolution law of its own.
- **`Self` and generics are legal** in signatures; interface generic
  parameters take no bounds. Each type-argument list gives the
  interface its own member set.
- **No `pub` on members, no `requires` on the interface, no
  embedding.** A flat member set; compose by declaring the members.
- **One impl form.** `impl T { .. }` defines the type's inherent
  members and may live only in the module that declares `T`.
  `impl I for T` does not parse — there is no second form.

Type bodies are **fields only** — a `fn` inside a `struct` or `class`
body is a parse error. Every method a type has lives in an impl block.

## Interface-typed values

An interface name in type position is the bare name — there is no
object-type keyword:

```rut
use ink::{ Logger };
use pouch::{ Vec };

pub interface Area {
    fn area(self) -> f64;
}

class Circle {
    r: f64;
}

class UnitSquare {
    side: f64;
}

impl Circle {
    pub fn area(self) -> f64 { return 3.14159265358979 * self.r * self.r; }
}

impl UnitSquare {
    pub fn area(self) -> f64 { return self.side * self.side; }
}

class Canvas {
    log: Logger;
}

impl Canvas {
    pub fn render(self, a: f64) { self.log.info(f"blitted a shape of area {a}"); }
}

fn blit(g: Canvas, s: Area) { g.render(s.area()); }   // a parameter over any Area

entry fn main() {
    let log = Logger.new("shapes");
    let g = Canvas { log: log };
    let mut mixed: Vec<Area> = Vec.new();   // heterogeneous storage
    mixed.push(Circle { r: 1.0 });
    mixed.push(UnitSquare { side: 2.0 });
    for (let s of mixed) { blit(g, s); }
}
```

```text
blitted a shape of area 3.14159265358979
blitted a shape of area 4
```

An interface-typed value is a reference to a real cell that still
carries its exact class. Passing a concrete value at an
interface-typed position is implicit on assignment, argument passing,
and returns, allocates nothing, and is gated by exactly one question:
does the value's type have the members? There is no top interface to
widen to, and intersection types (`A & B`) are ruled out by design —
when you need both capabilities, declare an interface that spells both
member sets.

## Dispatch: the two-rule law

Every method call compiles under one of exactly two rules, fixed at
compile time as a property of the *call site*:

1. **Static** — the call site names exactly one concrete type. The
   call binds directly to that type's member, no table involved:
   - a concrete receiver (`c.area()` on `c: Circle`);
   - an interface-typed local whose single concrete origin the
     compiler tracked (`let s: Area = Point { .. }; s.area()`);
   - an interface-typed *parameter* — the function specializes per
     concrete argument at monomorphization, so an interface parameter
     is effectively an implicit generic bound;
   - inside monomorphized generics.
2. **Itable** — the receiver is interface-typed with multiple possible
   concrete origins; the call consults the value's itable — the
   per-(concrete type × interface) method table:
   - elements of a heterogeneous container (`Vec<Area>`);
   - interface-typed field and element loads (the cell may hold any
     satisfier);
   - branch-merged bindings (`if` arms carrying different concretes
     into one interface-typed variable).

The itable rows are synthesized at the boxing sites — the same
boundary checks that proved satisfaction fill the rows. The origin
analysis is conservative: any merge, indirection, or cross-function
flow counts as multiple. Mis-analysis cannot produce wrong code — an
uncertain origin costs one itable hop, never a wrong static bind. And
the same descriptor that answers itable calls answers `is` probes: one
runtime truth per value.

## Type tests

`expr is Type` yields a `bool` and is total — it never traps. Two
probes, one keyword:

- **Concrete RHS**: is the value's exact type `T`? (No inheritance — a
  single id compare.)
- **Interface RHS**: does the value's concrete type have itable fills
  for the interface — i.e. was it satisfaction-boxed somewhere? A
  capability probe, usable on any value (`k is Encodable`).

`is` has no flow-sensitive effects: a true probe does not narrow `x`.
When the static type already answers — on an exact receiver the member
set decides at compile time — the probe folds to a constant.
Interface-typed values cannot be downcast to a concrete type at all —
if you need `Circle`-specific behavior behind an `Area`, put that
member on the interface. The one recovery path for erased values is
the explicit `opaque` box (see the reference on
[`opaque`](../reference/opaque.md)).

## Visibility: the use-both gate

Calling an interface's member requires **both** names in scope: the
type (by declaration or `use`) *and* the interface (`use`). Inherent
members need only the type. A call through an interface no `use`
brings in is an error — "use `I` to call its methods on `T`" — so a
foreign interface can never silently change what a call site means.
The gate is diagnostic-level: it governs which calls a module may
spell, never what a type can do (scope changes spelling, never
meaning).

## The iteration protocol

A type is iterable when an inherent impl marks a member `[iterable]`
(the bracket marker — the contract's shape is `fn <free>(self,
emit: fn(E) -> bool)`, the element falling out of the marked member's
own signature):

```rut
use ink::{ Logger };

class CountUp {
    n: i32;
}

impl CountUp {
    [iterable] pub fn iterate(self, emit: fn(i32) -> bool) {
        for (let i = 1; i <= self.n; i += 1) {
            if (!emit(i)) { return; }
        }
    }
}

entry fn main() {
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

`for (let v of it) { body }` desugars to `it.<member>(emit)` with a
synthetic closure: the body runs, then `emit` returns `true`; `break`
returns `false`. The loop variable is ONE variable reassigned per
iteration — the same law the fused index loops follow (the capture
law treats the two forms identically; see
[functions, closures, and generics](../reference/functions-closures-generics.md)).
The builtin sequences (`[T]`, `str`, `bytes`, and the standard
growable `Vec`) keep fused index loops instead; they never pay a
per-element call.

## Engine contracts are not interfaces

The engine's own contracts stand alone — no polymorphism machinery,
no member-set checks: the bracket markers `[disposal]` and
`[iterable]` designate inherent impl members (one per contract per
class, the signature checked against the contract, dispatched through
a designated slot — never an itable lookup), and the async protocol
lives on two CLOSED `builtin class` rows, `Future<T>` and
`RunContext`. The closure IS the wall: futures and cx records are
engine-minted only, so a user type cannot BE one — the member-set law
cannot see engine identity, and nominal closure is the only spelling
of "engine-minted only". See
[the async model](async-model.md),
[interfaces](../reference/interfaces.md),
and [the Rc heap](../reference/rc-heap.md).

## Equality is not an interface

`==` is builtin and cannot be opted into or out of: primitives compare
by value, `str`/`bytes` by content, everything else by cell identity.
Field-wise comparison is a loop you write, or the map packages' own key
rules. See [everything is a value](everything-is-a-value.md).

## What this buys you

- **Structural satisfaction keeps the surface small**: one mechanism
  (the member set), one checking moment (the boundary), no registry to
  reason about — a type's capability is readable off its own impl.
- **The two-rule law** makes dispatch predictable: you can tell, per
  call site, whether you are paying a hop.
- **The boundary check** keeps costs local: satisfaction is proved
  where the value crosses, the itable row is filled right there, and
  nothing anywhere else in the program can change either.
