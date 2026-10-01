# Traits and impl blocks

A trait declares a set of methods; a type joins the trait by writing an
impl block. Satisfaction is **nominal** — the impl block is the only
admission. A type whose members all happen to match a trait's shapes is
still not an instance of it until someone writes the block. The full
dispatch story is in [the reference on traits](../reference/traits.md)
and [traits and dispatch](../core-concepts/traits-and-dispatch.md).

## Declaring a trait

Traits declare method signatures — no bodies, no default
implementations, no fields:

```rut
trait Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}
```

Methods spell their receiver `self` explicitly, like every rut method.
Anything that looks like a property is a method: declare `fn
count(self) -> i32;`, not a field.

## Implementing a trait

`impl Trait for Type { .. }` registers the pair. Every method must
match the trait's signature exactly; extra methods don't belong here
(put those in an inherent block). Trait impl methods carry no `pub` —
they are as visible as the trait. Generic binders are **declared
after `impl`** — `impl<T> Readable<T> for Source<T>` — and the
declared list is the definition site: a bare parameter name in the
head that it does not declare is an error naming the fix.

```rut
use ink::{ Logger };

trait Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

struct Circle { r: f64 }

impl Shape for Circle {
    fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    fn name(self) -> str { return "circle"; }
}

pub fn main() {
    let log = Logger.new("traits");
    let c = Circle { r: 1.0 };
    log.info(f"{c.name()}={c.area()}");
}
```

```text
circle=3.14159
```

Rules worth knowing:

- **One impl per (trait, type) pair, program-wide.** A duplicate is a
  link error.
- **Placement:** an impl may live in a module of the trait's package or
  the type's package — at least one side must be yours. You cannot
  implement two foreign types to each other.
- **An empty impl block is legal** and acts as a marker: `impl
  Serializable for Point {}` says "this type is in" when the trait has
  no required methods.
- **Primitives can implement traits too** (in the trait's own package
  or module) — the standard library uses this to give integer widths a
  common internal interface.

## Inherent impls

`impl Type { .. }` — no trait — is where a type's own methods live:
class methods like `new`, instance methods, helpers. It compiles only
in the module that declares the type. A generic type's inherent impl
declares its binders the same way a trait impl does: `impl<T>
Vec<T> { .. }`.

```rut
use ink::{ Logger };

struct Circle { r: f64 }

impl Circle {
    fn new(r: f64) -> Self {
        return Self { r: r };
    }
}

pub fn main() {
    let log = Logger.new("traits");
    let c = Circle.new(1.0);
    log.info(f"r={c.r}");
}
```

```text
r=1
```

## Using trait values

**Widening is implicit** at any position that expects the trait: an
annotated binding, a field, an argument, a return.

```rut
use ink::{ Logger };

trait Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

struct Circle { r: f64 }

impl Shape for Circle {
    fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    fn name(self) -> str { return "circle"; }
}

pub fn main() {
    let log = Logger.new("traits");
    let c: Shape = Circle { r: 1.0 };   // Circle widens to Shape
    log.info(f"{c.name()}={c.area()}");
}
```

```text
circle=3.14159
```

**Trait-typed parameters accept any implementor** — write the function
once, call it with each concrete type:

```rut
use ink::{ Logger };

trait Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

struct Circle { r: f64 }
struct Square { side: f64 }

impl Shape for Circle {
    fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    fn name(self) -> str { return "circle"; }
}

impl Shape for Square {
    fn area(self) -> f64 { return self.side * self.side; }
    fn name(self) -> str { return "square"; }
}

fn describe(s: Shape) -> str {
    return f"{s.name()}={s.area()}";
}

pub fn main() {
    let log = Logger.new("traits");
    log.info(describe(Circle { r: 1.0 }));
    log.info(describe(Square { side: 3.0 }));
}
```

```text
circle=3.14159
square=9
```

**Heterogeneous containers** hold mixed implementors behind the trait
name. Inside a single-typed context calls bind directly; iterating a
collection of `Shape` dispatches through the value's vtable — the
compiler picks, and mis-guessing costs one hop, never wrong behavior:

```rut
use pouch::{ Vec };
use ink::{ Logger };

trait Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

struct Circle { r: f64 }
struct Square { side: f64 }

impl Shape for Circle {
    fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    fn name(self) -> str { return "circle"; }
}

impl Shape for Square {
    fn area(self) -> f64 { return self.side * self.side; }
    fn name(self) -> str { return "square"; }
}

pub fn main() {
    let log = Logger.new("traits");
    let mut shapes: Vec<Shape> = Vec.new();
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

## Type tests: `is`

`expr is Type` answers with a `bool` and never traps:

- **Concrete RHS** — `p is Circle`: an exact-type test.
- **Trait RHS** — `p is Shape`: a capability probe ("does this value's
  type have an impl of `Shape`?").

```rut
use ink::{ Logger };

trait Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

struct Circle { r: f64 }
struct Square { side: f64 }

impl Shape for Circle {
    fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    fn name(self) -> str { return "circle"; }
}

impl Shape for Square {
    fn area(self) -> f64 { return self.side * self.side; }
    fn name(self) -> str { return "square"; }
}

pub fn main() {
    let log = Logger.new("traits");
    let sq = Square { side: 2.0 };
    log.info(f"shape: {sq is Shape} circle: {sq is Circle}");
}
```

```text
shape: true circle: false
```

`is` answers the question; it changes nothing — there is no narrowing
and no downcast through a trait. A trait-typed value is used through
its trait's methods; if you need the erased-storage version — a value
whose type is *forgotten* until recovered — that is `opaque`, covered
in [errors and optionality](errors.md) and
[the reference on opaque](../reference/opaque.md).

## Bounds connect traits to generics

`fn name<T requires Labeled>(x: T)` admits exactly the instantiations
whose concrete type has an impl of `Labeled` — see
[functions, closures, and generics](functions.md). The bound is what
makes widening legal inside the body: `let w: Labeled = x;`.

## Making your type iterable

A type becomes a `for..of` target by implementing the builtin
`Iterable<E>` contract with its single resumption member (`Iterable`
is core's import-gated `pub builtin` trait — an `impl` names it, so
bring it in with `use core::{ Iterable }`; the builtin sequences
themselves never need it):

```rut
use core::{ Iterable };
use pouch::{ Vec };
use ink::{ Logger };

struct CountUp { n: i32 }

impl Iterable<i32> for CountUp {
    fn iterate(self, emit: fn(i32) -> bool) {
        for (let i = 1; i <= self.n; i += 1) {
            if (!emit(i)) { return; }   // false = stop
        }
    }
}

pub fn main() {
    let log = Logger.new("iter");
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
without the trait — their loops are fused, never a per-element call.

## Put it together

```rut
use pouch::{ Vec };
use ink::{ Logger };

trait Shape {
    fn area(self) -> f64;
    fn name(self) -> str;
}

struct Circle { r: f64 }
struct Square { side: f64 }

impl Shape for Circle {
    fn area(self) -> f64 { return 3.14159 * self.r * self.r; }
    fn name(self) -> str { return "circle"; }
}

impl Shape for Square {
    fn area(self) -> f64 { return self.side * self.side; }
    fn name(self) -> str { return "square"; }
}

fn describe(s: Shape) -> str {
    return f"{s.name()}={s.area()}";
}

pub fn main() {
    let log = Logger.new("traits");

    let c: Shape = Circle { r: 1.0 };
    log.info(describe(c));

    let sq = Square { side: 2.0 };
    log.info(f"is shape: {sq is Shape}");

    let mut shapes: Vec<Shape> = Vec.new();
    shapes.push(Circle { r: 1.0 });
    shapes.push(Square { side: 3.0 });
    let mut total: f64 = 0.0;
    for (let s of shapes) {
        total += s.area();
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
