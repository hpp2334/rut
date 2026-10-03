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

entry fn main() {
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

entry fn main() {
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

entry fn main() {
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
closure does. The capture law has two halves: primitives (and `nil`
and `fn` values) copy — the closure gets its own slot, and a reassign
inside the closure never moves the original — while everything
ref-headed (a `Vec`, a struct, `str`, `?T`, …) crosses as a handle to
the same cell. That second half includes whole-value reassignment: a
captured binding that is reassigned — either frame — shares its slot
with the closure, so both sides always see the current value:

```rut
use pouch::{ Vec };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("cap");
    let mut xs: Vec<i32> = Vec.new();
    xs.push(1);
    let len = fn() -> i32 { return xs.len(); };
    let mut ys: Vec<i32> = Vec.new();
    ys.push(9); ys.push(9);
    xs = ys;                       // a rebind the closure sees
    log.info(f"len={len()}");      // 2 — the closure shares the slot
}
```

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

entry fn main() {
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
  whose type is just `T`. Pass concrete values in, spell an interface
  bound (`T requires Labeled`), or take an interface-typed parameter
  (next chapter).
- In v1 a *generic class* in a parameter position does not unify —
  `fn sum(xs: Vec<i32>)` is fine, but a function generic over `T`
  taking `Vec<T>` is not yet the shape to reach for. Concrete
  instantiations cover most code.

### `requires` bounds

An inline `requires` bound gates which instantiations compile:

```rut
use ink::{ Logger };

interface Labeled {
    fn label(self) -> str;
}

struct Tag { id: i32 }

impl Tag {
    fn label(self) -> str { return f"tag-{self.id}"; }
}

fn name<T requires Labeled>(x: T) -> str {
    let w: Labeled = x;     // the bound proves this widening
    return w.label();
}

entry fn main() {
    let log = Logger.new("bounds");
    log.info(name(Tag { id: 7 }));
}
```

```text
tag-7
```

The bound is *admission-only*: it checks at each call site that the
concrete type satisfies the named interface (or union of type names), and it
proves a `T`-typed value may be used as that interface type — it does not
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

entry fn main() {
    let log = Logger.new("free-fns");
    let p = nudged(Point { x: 2, y: 5 });
    log.info(f"p.x={p.x} p.y={p.y}");
}
```

```text
p.x=3 p.y=5
```

Reach for methods when something is genuinely *the receiver's*
behavior, and for the members other code observes through interfaces —
everything else is a plain function.

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

entry fn main() {
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
