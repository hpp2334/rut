# Functions, closures, and generics

Functions are declared with `fn`, values can be functions, and generic
functions monomorphize at compile time — each instantiation gets its
own specialized code. The details live in
[the reference on functions, closures, and generics](../reference/functions-closures-generics.md).

## Declaring functions

Parameter and return types are always written. A function whose return
type is omitted returns nothing:

```rut
fn swap(xs: Vec<i32>, a: i32, b: i32) {
    let t = xs[a];
    xs[a] = xs[b];
    xs[b] = t;
}

fn length(pt: Point) -> f64 {
    return sqrt((pt.x * pt.x + pt.y * pt.y) as f64);
}
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
let add = fn (a: i32, b: i32) -> i32 { return a + b; };

let area_of = fn (r: f32) -> f32 {
    let sq = r * r;
    return sq * 3.14159265f32;
};

add(1, 2);          // call it like any function
```

Function types are written `fn(P..) -> R` and appear anywhere a type
does:

```rut
fn apply(f: fn(i32) -> i32, v: i32) -> i32 {
    return f(v);
}
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
fn first<T>(xs: [T], fallback: T) -> T {
    if (xs.len() == 0) {
        return fallback;
    }
    return xs[0];
}

let head = first([10, 20], -1);      // first<i32> — inferred
let name = first(["a", "b"], "?");   // first<str> — a separate instance
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
fn name<T requires Labeled>(x: T) -> str {
    let w: Labeled = x;     // the bound proves this widening
    return w.label();
}
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
struct Point { x: f32; y: f32 }

fn nudged(pt: Point) -> Point {
    return Point { x: pt.x + 1, y: pt.y };
}
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
