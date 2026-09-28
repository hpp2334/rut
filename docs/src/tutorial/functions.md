# Functions, closures, and generics

Functions are declared with `fn`, values can be functions, and generic
functions monomorphize at compile time — each instantiation gets its
own specialized code. The details live in
[the reference on functions, closures, and generics](../reference/functions-closures-generics.md).

## Declaring functions

Parameter and return types are always written. A function whose return
type is omitted returns nothing:

```rut
use pouch::{ Vec };
use calc::{ Math };
use ink::{ Logger };

struct Point { x: f32; y: f32 }

fn swap(mut xs: Vec<i32>, a: i32, b: i32) {
    let t = xs[a];
    xs[a] = xs[b];
    xs[b] = t;
}

fn length(pt: Point) -> f64 {
    return Math.sqrt((pt.x * pt.x + pt.y * pt.y) as f64);
}

pub fn main() {
    let log = Logger.new("fns");
    let xs = Vec<i32>.from([1, 2, 3]);
    swap(xs, 0, 2);
    log.info(f"first={xs[0]} length={length(Point { x: 3, y: 4 })}");
}
```

```text
first=3 length=5
```

A `mut` parameter (`fn step(mut p: Point)`) is the callee's permission
to write through `p` — and because non-primitive values share, that
means the caller's value. A plain parameter promises not to write
through it.

Recursion works as expected — functions are visible in their own
bodies, and mutual recursion within a module needs no forward
declaration.

## Anonymous functions

The closure spelling is an anonymous `fn` with a block body. There is
no arrow shorthand — every closure states its parameter and return
types, which is what lets it inhabit a first-class `fn` type:

```rut
use ink::{ Logger };

pub fn main() {
    let log = Logger.new("closures");
    let add = fn (a: i32, b: i32) -> i32 { return a + b; };

    let area_of = fn (r: f32) -> f32 {
        let sq = r * r;
        return sq * 3.14159265f32;
    };

    add(1, 2);          // call it like any function
    log.info(f"add={add(1, 2)} area={area_of(1)}");
}
```

```text
add=3 area=3.1415927
```

Function types are written `fn(P..) -> R` and appear anywhere a type
does:

```rut
use ink::{ Logger };

fn apply(f: fn(i32) -> i32, v: i32) -> i32 {
    return f(v);
}

pub fn main() {
    let log = Logger.new("fns");
    let double = fn (x: i32) -> i32 { return x * 2; };
    log.info(f"apply={apply(double, 21)}");
}
```

```text
apply=42
```

Closures capture the enclosing bindings — a closure body sees and can
use the locals around it, including after the enclosing function would
have returned, because captured cells stay alive as long as the
closure does. Since non-primitives share, a capture of a `Vec` or a
struct is a handle to the same cell: writes through the closure are
visible to everyone else holding it.

## Generics

A generic function declares type parameters in angle brackets. Each
concrete instantiation is compiled separately (monomorphized), so
generics cost nothing at runtime:

```rut
use ink::{ Logger };

fn first<T>(xs: [T], fallback: T) -> T {
    if (xs.len() == 0) {
        return fallback;
    }
    return xs[0];
}

pub fn main() {
    let log = Logger.new("generics");
    let head = first([10, 20], -1);      // first<i32> — inferred
    let name = first(["a", "b"], "?");   // first<str> — a separate instance
    log.info(f"head={head} name={name}");
}
```

```text
head=10 name=a
```

`T` is inferred from the arguments; you rarely spell `first<i32>(..)`
explicitly. Two rules to know:

- A bare `T` has **no methods** — you cannot call anything on a value
  whose type is just `T`. Pass concrete values in, or take a
  trait-typed parameter instead (next chapter).
- In v1 a *generic class* in a parameter position does not unify —
  `fn sum(xs: Vec<i32>)` is fine, but a function generic over `T`
  taking `Vec<T>` is not yet the shape to reach for. Concrete
  instantiations cover most code.

### `requires` bounds

An inline `requires` bound gates which instantiations compile:

```rut
use ink::{ Logger };

trait Labeled {
    fn label(self) -> str;
}

struct Tag { id: i32 }

impl Labeled for Tag {
    fn label(self) -> str { return f"tag-{self.id}"; }
}

fn name<T requires Labeled>(x: T) -> str {
    let w: Labeled = x;     // the bound proves this widening
    return w.label();
}

pub fn main() {
    let log = Logger.new("bounds");
    log.info(name(Tag { id: 7 }));
}
```

```text
tag-7
```

The bound is *admission-only*: it checks at each call site that the
concrete type satisfies the named trait (or union of traits), and it
proves a `T`-typed value may be used as that trait type — it does not
put methods on `T` itself. Union bounds admit any member:

```rut
fn kind<T requires Ridge | Trench>(x: T) -> str { .. }
```

This is exactly how typed JSON entry points are spelled —
`decodeJson<T requires JsonDeserialize>(s: str)` — see
[errors and optionality](errors.md). Bounds and unions are covered fully in
[the reference on type aliases and union bounds](../reference/type-aliases.md).

## Free functions are the default idiom

rut's records carry no methods (bodies are fields only; methods live in
`impl` blocks — see [structs, enums, and classes](types.md)), so the
everyday shape is small data plus free functions over it:

```rut
use ink::{ Logger };

struct Point { x: f32; y: f32 }

fn nudged(pt: Point) -> Point {
    return Point { x: pt.x + 1, y: pt.y };
}

pub fn main() {
    let log = Logger.new("free-fns");
    let p = nudged(Point { x: 2, y: 5 });
    log.info(f"p.x={p.x} p.y={p.y}");
}
```

```text
p.x=3 p.y=5
```

Reach for methods when something is genuinely *the receiver's*
behavior, and for trait impls — everything else is a plain function.

## Put it together

```rut
use pouch::{ Vec };
use ink::{ Logger };

fn first<T>(xs: [T], fallback: T) -> T {
    if (xs.len() == 0) {
        return fallback;
    }
    return xs[0];
}

fn sum(xs: Vec<i32>) -> i32 {
    let mut total = 0;
    for (let x of xs) {
        total += x;
    }
    return total;
}

pub fn main() {
    let log = Logger.new("closures");
    let add = fn (a: i32, b: i32) -> i32 { return a + b; };
    let area_of = fn (r: f32) -> f32 {
        let sq = r * r;
        return sq * 3.14159265f32;
    };
    log.info(f"add={add(1, 2)} area={area_of(1)} sum={sum(Vec<i32>.from([1, 2, 3]))}");

    let head = first([10, 20], -1);       // first<i32> — monomorphized
    let name = first(["a", "b"], "?");    // first<str> — separate instance
    log.info(f"head={head} name={name}");
}
```

```text
add=3 area=3.1415927 sum=6
head=10 name=a
```

Next: [structs, enums, and classes](types.md).
