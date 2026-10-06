# Functions, closures, and generics

Function declarations, anonymous closures, and monomorphized generics
with admission-only bounds.

## Functions

```rut
use ink::{ Logger };

fn add(a: i32, b: i32) -> i32 {
    return a + b;
}

entry fn main() {
    let log = Logger("t");
    log.info(f"{add(2, 3)}");
}
```

```text
5
```

- Parameters are `name: Type`; a mutable parameter declares
  `mut name` — the callee's permission to mutate the caller's object
  (see [Modules and visibility](modules-and-visibility.md)).
- A function without a result arrow returns `nil`; `return;` (or
  falling off the end) is its return.
- Methods are functions in impl blocks: the first parameter spelled
  `self` (or `mut self`) makes it an instance method; absence of `self`
  makes it a class method (see
  [Classes and constructors](classes.md)).
- `entry fn` publishes a function to the embedder (see
  [Modules and visibility](modules-and-visibility.md)); `async fn`
  declares a suspending function (see [Async and await](async.md)).
- Function types are first-class: `fn apply(f: fn(i32) -> i32, v: i32) -> i32`.

## Closures

The closure spelling is an **anonymous fn** — block bodies, no arrow
form:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("t");
    let add = fn (a: i32, b: i32) -> i32 { return a + b; };
    let area_of = fn (r: f32) -> f32 {
        let sq = r * r;
        return sq * 3.14159265f32;
    };
    log.info(f"add={add(1, 2)} area={area_of(1)}");
}
```

```text
add=3 area=3.1415927
```

- An anonymous fn inhabits `fn(P..) -> R` directly — it is a value of
  the function type, copyable like a primitive.
- **The capture law**: primitives, `nil`, and `fn` values copy — a
  captured scalar is the closure's own slot, and reassigning it inside
  the closure never moves the original. Ref-headed values (records,
  `Vec`, arrays, `str`/`bytes`, `?T`, enums) cross as handles: writes
  through the captured handle reach the shared cell, and so does a
  whole-value REASSIGNMENT — a captured-and-reassigned binding is
  promoted to a hidden shared-slot cell, so both frames stay linked
  for the binding's whole scope (a stash-then-reassign, a reassign
  inside the closure, and the `for..of` loop variable all follow this
  one law). Refcounting keeps captures alive; a closure is itself a
  shared cell value.

## Generics

```rut
use ink::{ Logger };

fn first<T>(xs: [T], fallback: T) -> T {
    if (xs.len() == 0) { return fallback; }
    return xs[0];
}

entry fn main() {
    let log = Logger("t");
    let head = first([10, 20], -1);      // first<i32>   — monomorphized
    let name = first(["a", "b"], "?");   // first<str>   — separate instance
    log.info(f"{head} {name}");
}
```

```text
10 a
```

- **Generics monomorphize at compile time**: each instantiation emits
  its own typed code. `T` infers from the arguments; explicit type
  arguments may be spelled at call sites, including method calls:
  `self.st.get<T>(self)`.
- Interface-typed arguments are ordinary arguments: a `T` instantiated at an
  interface type becomes a handle slot, satisfying no bound.
- **Generic parameters are unconstrained by default** — you cannot call
  methods on a bare `T`. Pass values in, take an `I`-typed parameter,
  or spell the bound (`T requires I`).
- There are no const-generic user parameters. The builtin surfaces fix
  their shapes (`[T]` is one type; lengths are runtime values).

### Inline bounds — `requires`

```rut
gparam := Ident ('requires' bound)?
bound := Type ('|' Type)*
```

`fn f<T requires A | B>(x: T)` — an **admission-only** bound on fn,
method, and class generic parameters (struct and interface generic
parameters reject `requires`):

- The bound gates which instantiations compile: enforcement is at every
  substitution-completing site (free-fn calls, method instantiation,
  impl-method enqueue). A failing instantiation diagnoses with the
  bound spelled out — "`bool` does not satisfy `T` requires `i32 |
  str`".
- Members may be concrete type names (satisfied by exact type
  identity), aliases (expanded first — see
  [Type aliases and union bounds](type-aliases.md)), a single interface
  (satisfied structurally — the member set checks where `T` is
  chosen), or a **type union** of concrete
  names/aliases. An interface member inside a union spelling is invalid —
  an interface bound stands alone.
- **Interface-typed values satisfy nothing**: a `T` instantiated at an
  interface type fails any bound — only a concrete type with the
  members admits.
- A non-union bound **proves the widening**: the body may widen a
  `T`-typed value into a bound-interface-typed slot
  (`let w: Labeled = x;`), but gains no method calls on bare `T`.
- A **union bound** carries the whole-bound contract: a method call on
  a union-bounded value requires **every** member to provide the method
  — even a member that is never actually instantiated. Dispatch is
  untouched: each instantiation still binds the concrete member's own
  method.
- Bounds may reference the item's other generics:
  `fn hold<T, U requires [T]>(x: U)`.
- Generic **classes** take bounds on their parameters — the bound
  records on the class descriptor and admits every instantiation:
  `pub class HashMap<K requires i8 | .. | bytes, V>` (see
  [Builtin generic types](builtin-generic-types.md)).
