# Interfaces and impl blocks

An interface declares a set of method signatures; a type satisfies it
by **having the members**. Satisfaction is **structural** — there is
no admission step, no registration, no `impl Interface for Type`
anywhere in the language: `impl T { .. }` (the type's own inherent
impl) is the only impl form, and the boundary check is the whole law.
The one-mechanism-per-job philosophy behind this is in
[interfaces and dispatch](../core-concepts/interfaces-and-dispatch.md);
the full story is in
[the reference on interfaces](../reference/interfaces.md).

## Declaring an interface

Interfaces declare method signatures — no bodies, no default
implementations, no fields:

```rut
pub interface Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}
```

Methods spell their receiver `self` explicitly, like every rut method.
Anything that looks like a property is a method: declare `fn
count(self) -> i32;`, not a field. `Self` and generics are legal in
signatures; interface members carry no `pub` — they are as visible as
the interface — and the interface itself takes no bounds and embeds
nothing: a flat member set, compose by declaring the members.

## Satisfying an interface

Satisfaction is membership. Give the type a `pub` member for every
interface member, on the type's own inherent impl, and it satisfies —
nothing else to write:

```rut
use ink::{ Logger };

pub interface Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

class Circle { r: f64; }

impl Circle {
    pub fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    pub fn name(self) -> str { return "circle"; }
}

entry fn main() {
    let log = Logger("interfaces");
    let c = Circle { r: 1.0 };
    log.info(f"{c.name()}={c.area()}");
}
```

```text
circle=3.14159
```

Rules worth knowing:

- **Signatures must match** — receiver form (`self` vs `mut self` vs
  none), parameter types, return type, and `async` spelling, exactly.
  A member that is almost right is not right: the diagnostic names the
  first mismatching member.
- **Members live on the type's own inherent impl**, in the type's
  module. `impl Circle { pub fn area(self) -> f64 { .. } }` is where
  the member set comes from; there is no second impl form to reach
  for.
- **Only the type's own public members count.** A used package's type
  satisfies through the members its own module spells; you cannot add
  members to a foreign type, and nothing you write can register a
  satisfaction for a type that lacks the members.
- **Shape-equal is satisfied** — including by types whose author never
  thought of your interface. Two types with the same member shapes
  both satisfy, and there is no opt-out. The convention that keeps
  this honest: give near-miss members distinct names or shapes, so
  accidental satisfaction stays a non-event in practice.
- **An interface with no members is satisfied by everything** — the
  empty member set is vacuously true. There is no marker idiom: if a
  declaration needs to mean something, it spells a member.

## Inherent impls

`impl Type { .. }` is where a type's own methods live: class methods
like `new`, instance methods, helpers. It compiles only in the module
that declares the type. A generic type's inherent impl declares its
binders up front: `impl<T> Vec<T> { .. }`.

```rut
use ink::{ Logger };

class Circle {
    r: f64;
}

impl Circle {
    pub fn new(r: f64) -> Self {
        return Self { r: r };
    }
}

entry fn main() {
    let log = Logger("interfaces");
    let c = Circle.new(1.0);
    log.info(f"r={c.r}");
}
```

```text
r=1
```

Every member the satisfaction law reads comes from here — public
members of the type's inherent impl, and nothing else.

## Using interface values

An interface name in type position is the bare name. Four forms, one
law each — checking at the boundary, never a search:

**Interface-typed parameters accept any satisfying type** — write the
function once, call it with each concrete type:

```rut
use ink::{ Logger };

pub interface Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

class Circle { r: f64; }
class Square { side: f64; }

impl Circle {
    pub fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    pub fn name(self) -> str { return "circle"; }
}

impl Square {
    pub fn area(self) -> f64 { return self.side * self.side; }
    pub fn name(self) -> str { return "square"; }
}

fn describe(s: Shape) -> str {
    return f"{s.name()}={s.area()}";
}

entry fn main() {
    let log = Logger("interfaces");
    log.info(describe(Circle { r: 1.0 }));    // Circle satisfies: checked here
    log.info(describe(Square { side: 3.0 })); // and here
}
```

```text
circle=3.14159
square=9
```

The check fires where the concrete value crosses into the interface —
each call above verifies the argument's member set once. A type
without the members is refused at that same spot, and the diagnostic
names the type, the interface, and the missing member:

```rut
pub interface Shape {
    fn area(self) -> f64;
}

class Point { x: i32; y: i32; }

fn admit(p: Shape) -> nil { }
fn call_it() { admit(Point { x: 1, y: 2 }); }
```

```text
`Point` does not satisfy `Shape`: no member `area`
```

**Typed lets are satisfaction assertions** — the annotation is the
question, the initializer's member set is the answer:

```rut
use ink::{ Logger };

pub interface Shape {
    fn area(self) -> f64;
}

class Square { side: f64; }

impl Square {
    pub fn area(self) -> f64 { return self.side * self.side; }
}

entry fn main() {
    let log = Logger("interfaces");
    let s: Shape = Square { side: 2.0 };   // asserted and checked
    log.info(f"area={s.area()}");
}
```

```text
area=4
```

**Heterogeneous containers** hold mixed satisfiers behind the
interface name. Inside a single-typed context calls bind directly;
iterating a collection of `Shape` consults each value's itable — the
compiler picks, and mis-guessing costs one hop, never wrong behavior:

```rut
use pouch::{ Vec };
use ink::{ Logger };

pub interface Shape {
    fn area(self) -> f64;
}

class Circle { r: f64; }
class Square { side: f64; }

impl Circle {
    pub fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
}

impl Square {
    pub fn area(self) -> f64 { return self.side * self.side; }
}

entry fn main() {
    let log = Logger("interfaces");
    let mut shapes: Vec<Shape> = Vec.new();   // each push checks
    shapes.push(Circle { r: 1.0 });
    shapes.push(Square { side: 3.0 });
    let mut total: f64 = 0.0;
    for (let s of shapes) {
        total += s.area();       // dispatches per element
    }
    log.info(f"total={total} n={shapes.len()}");
}
```

```text
total=12.14159 n=2
```

## Primitives ride in wrappers

Primitives, arrays, tuples, and `?T` have no inherent members, so they
can never satisfy — and no layer will invent a satisfaction for you.
The route in is a **newtype class**: a one-field wrapper whose
inherent impl carries the members, spelled at every call site:

```rut
use ink::{ Logger };

pub interface Show {
    fn show(self) -> str;
}

class NumI64(i64);

impl NumI64 {
    pub fn show(self) -> str { return str(self.inner); }
}

fn print_it(x: Show) -> str { return x.show(); }

entry fn main() {
    let log = Logger("interfaces");
    log.info(print_it(NumI64(64)));    // spelled manufacture
}
```

```text
64
```

`print_it(NumI64(64))` — ✓ spelled, sound, per-call.
`print_it(64)` — ✗ forever: no auto-insertion exists. Wrappers
compose — a generic wrapper `class Box<T requires Show> { v: T; }`
checks its bound at instantiation, and its own members let `Box`
satisfy in turn. The standard library's json and flow packages are
built exactly this way (see
[the standard library](stdlib.md)).

## Type tests: `is`

`expr is Type` answers with a `bool` and never traps:

- **Concrete RHS** — `p is Circle`: an exact-type test.
- **Interface RHS** — `p is Shape`: a capability probe — true when
  the value's concrete type was satisfaction-boxed somewhere (itable
  fills exist for `Shape`).

```rut
use pouch::{ Vec };
use ink::{ Logger };

pub interface Shape {
    fn area(self) -> f64;
}

class Circle { r: f64; }
class Square { side: f64; }

impl Circle {
    pub fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
}

impl Square {
    pub fn area(self) -> f64 { return self.side * self.side; }
}

entry fn main() {
    let log = Logger("interfaces");
    let mut shapes: Vec<Shape> = Vec.new();
    shapes.push(Circle { r: 1.0 });
    shapes.push(Square { side: 2.0 });
    let sq = Square { side: 2.0 };
    log.info(f"shape: {sq is Shape} circle: {sq is Circle}");
}
```

```text
shape: true circle: false
```

`is` answers the question; it changes nothing — there is no narrowing
and no downcast through an interface. An interface-typed value is used
through its interface's members; if you need the erased-storage
version — a value whose type is *forgotten* until recovered — that is
`opaque`, covered in [errors and optionality](errors.md) and
[the reference on opaque](../reference/opaque.md).

## Bounds connect interfaces to generics

`fn name<T requires Labeled>(x: T)` admits exactly the instantiations
whose concrete type satisfies `Labeled` — the same member-set check,
run where `T` is chosen — see
[functions, closures, and generics](functions.md). The bound is what
makes interface use legal inside the body: `let w: Labeled = x;`.

## Making your type iterable

A type becomes a `for..of` target by marking an inherent member
`[iterable]` — the bracket marker designates the member (the NAME is
free; the element type falls out of the member's own emit parameter).
No import is needed — the marker word IS the designation; the builtin
sequences never mark one:

```rut
use pouch::{ Vec };
use ink::{ Logger };

class CountUp { n: i32; }

impl CountUp {
    [iterable] pub fn iterate(self, emit: fn(i32) -> bool) {
        for (let i = 1; i <= self.n; i += 1) {
            if (!emit(i)) { return; }   // false = stop
        }
    }
}

entry fn main() {
    let log = Logger("iter");
    let ups = CountUp { n: 4 };
    let got: Vec<i32> = Vec.new();   // shared: survives the loop's captures
    for (let v of ups) {
        got.push(v);
    }
    log.info(f"n={got.len()} first={got[0]} last={got[got.len()-1]}");
}
```

```text
n=4 first=1 last=4
```

`for (let v of it)` desugars to `it.iterate(emit)` with a synthetic
closure, and the capture law covers it like every closure: scalars
copy (a rebind inside the loop stays local), ref-headed bindings share
their slot — reassigning one inside the loop moves the original, and
the loop variable is ONE variable reassigned per iteration, exactly
like a handwritten `while` (see
[functions, closures, and generics](functions.md)). Accumulating
through a shared `vec.push(v)` works, and so does plain reassignment.

The builtin sequences (`[T]`, `Vec<T>`, `str`, `bytes`) iterate
without the marker — their loops are fused, never a per-element call.

## Put it together

```rut
use pouch::{ Vec };
use ink::{ Logger };

pub interface Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

class Circle { r: f64; }
class Square { side: f64; }

impl Circle {
    pub fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    pub fn name(self) -> str { return "circle"; }
}

impl Square {
    pub fn area(self) -> f64 { return self.side * self.side; }
    pub fn name(self) -> str { return "square"; }
}

fn describe(s: Shape) -> str {
    return f"{s.name()}={s.area()}";
}

entry fn main() {
    let log = Logger("interfaces");

    let c: Shape = Circle { r: 1.0 };        // satisfaction assertion
    log.info(describe(c));                    // single origin: static call

    let sq = Square { side: 2.0 };
    log.info(f"is shape: {sq is Shape}");

    let mut shapes: Vec<Shape> = Vec.new();   // heterogeneous storage
    shapes.push(Circle { r: 1.0 });
    shapes.push(Square { side: 3.0 });
    let mut total: f64 = 0.0;
    for (let s of shapes) {
        total += s.area();                    // itable per element
    }
    log.info(f"total={total} n={shapes.len()}");
}
```

```text
circle=3.14159
is shape: true
total=12.14159 n=2
```

Next: [errors and optionality](errors.md).
