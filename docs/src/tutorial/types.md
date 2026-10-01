# Structs, enums, and classes

rut has three user-defined type forms, each with one job:

- **`enum`** — a small set of named constants.
- **`struct`** — an open data record: every field public, literal
  construction everywhere.
- **`class`** — a sealed record: private fields, construction through
  class methods only.

In all three, **the type body is fields only** — every method lives in
an `impl` block. A `fn` written inside a type body is a parse error.
The reference pages are
[structs](../reference/structs.md),
[enums](../reference/enums.md), and
[classes and constructors](../reference/classes.md).

## Enums

An enum is a distinct named type over integer constants. No payloads
and no computed members — where another language would use a
union of literal strings, rut uses an enum:

```rut
use ink::{ Logger };

enum Light { Green, Yellow, Red }

enum Direction { Up = 1, Down, Left, Right }   // 1, 2, 3, 4

pub fn main() {
    let log = Logger.new("enums");
    log.info(f"{Light.Green} {Direction.Left} {Direction.Right}");
}
```

```text
Green Left Right
```

Members are the enum's values and are spelled qualified:
`Light.Green`. Explicit initializers set where the numbering starts;
the rest continue from there.

`when` over an enum must be exhaustive — every member, or an `else`
arm (see [control flow and when](control-flow.md)). Enums render as
their member name in format strings. Enums take `impl` blocks —
non-self methods on the name, `self` methods on a value, and trait
impls (`impl Iterator<E> for Light` makes `for (let v of l)` walk) —
see [enums](../reference/enums.md).

## Structs — open records

Declare with `struct`, construct with a literal — anywhere in a
function body, nested inside other literals. Construction is the
literal; there is no constructor gate:

```rut
use ink::{ Logger };

struct Point {
    x: f32;
    y: f32;
}

struct Rect {
    min: Point;
    max: Point;
}

struct Style {
    color: u32 = 0xff00ff;      // field initializer: literals may omit it
    width: f32 = 1;
}

pub fn main() {
    let log = Logger.new("structs");
    let p = Point { x: 1, y: 2 };                    // every field, by name
    let s = Style {};                                // defaults fill the rest
    let r = Rect { min: Point { x: 0, y: 0 }, max: p };
    log.info(f"r.max.x={r.max.x} width={s.width}");
}
```

```text
r.max.x=1 width=1
```

All fields are public, always — member visibility in a struct is a
compile error. Privacy is what classes are for. Methods live in
`impl` blocks with real visibility — `pub fn` exports cross-module,
plain `fn` stays module-private (see
[structs](../reference/structs.md)).

**Structs share.** A struct value is a handle to a heap cell:
assignment, arguments, and returns all pass the handle, and a write
through any alias is visible through all of them. Writing needs a
`mut` binding or `mut` parameter:

```rut
use ink::{ Logger };

struct Point { x: f32; y: f32 }

pub fn main() {
    let log = Logger.new("sharing");
    let mut p = Point { x: 1, y: 2 };
    let q = p;              // q and p name ONE cell
    p.x = 4;                // q.x is 4 now
    log.info(f"q.x={q.x} same cell: {q == p}");
}
```

```text
q.x=4 same cell: true
```

`==` on struct values is **cell identity** — `q == p` is true (one
cell), `p == Point { x: 4, y: 2 }` is false (a different cell). To
compare field by field, write a function.

Recursive shapes are legal — a record may name itself through a
nullable field, because `?Node` is one word:

```rut
use ink::{ Logger };

struct Node {
    value: i32;
    left: ?Node;
    right: ?Node;
}

fn count(n: ?Node) -> i32 {
    let mut c = 1;
    if (n.left != nil) { c += count(n.left); }
    if (n.right != nil) { c += count(n.right); }
    return c;
}

pub fn main() {
    let log = Logger.new("nodes");
    let n = Node {
        value: 1,
        left: Node { value: 2, left: nil, right: nil },
        right: nil,
    };
    log.info(f"count={count(n)}");
}
```

```text
count=2
```

## Classes — sealed records

A class adds two things to a struct: module-private fields and
construction gated through class methods. There is no `constructor`
keyword, no `new` operator, and no outside literal — the only way to
build a class value from outside is to call a class method that chooses
to. (Cleanup hooks are not a class privilege: implement `Disposal` for
either shape, and the engine calls `dispose` when the value's cell
refcount reaches zero — see
[Rc, dispose, and identity](../reference/rc-dispose-identity.md).)

```rut
use ink::{ Logger };

class Counter {
    n: i32 = 0;             // module-private — the class's business
}

impl Counter {
    pub fn new() -> Self {                 // the construction surface
        return Self { };                   // the class-private literal
    }

    pub fn press(mut self) {
        self.n = self.n.wrapping_add(1);
    }

    pub fn count(self) -> i32 { return self.n; }
}

pub fn main() {
    let log = Logger.new("counter");
    let c = Counter.new();
    c.press();
    c.press();
    log.info(f"count={c.count()}");      // 2
}
```

```text
count=2
```

The pieces:

- **A class method is just a function without `self`.** `Rect.new(w, h)`,
  `Version.parse(s)`, `Rect.from_square(s)` — any no-`self` method
  returning `Self` is a constructor. `new` is a convention, not syntax.
- **The `Self { .. }` literal is class-private** — legal anywhere in
  the class's own impl block, never outside. This is the seal.
- **Instance methods spell `self` explicitly** as the first parameter;
  `mut self` marks methods that write. There is no `this`, no static
  methods, no `get`/`set` syntax — a computed property is a method
  (`c.count()`).
- **Construction is validation.** A constructor is an ordinary
  function — it can check arguments and refuse:

```rut
use ink::{ Logger };

class Rect {
    w: f32;
    h: f32;
}

impl Rect {
    pub fn new(w: f32, h: f32) -> Self {
        if (w <= 0 || h <= 0) {
            panic("Rect: negative extents");
        }
        return Self { w: w, h: h };
    }

    pub fn from_square(s: f32) -> Self {
        return Rect.new(s, s);
    }

    pub fn area(self) -> f32 { return self.w * self.h; }
}

pub fn main() {
    let log = Logger.new("rect");
    let r = Rect.new(3, 4);
    let sq = Rect.from_square(2);
    log.info(f"area={r.area()} square={sq.area()}");
}
```

```text
area=12 square=4
```

A *try*-constructor answers the nullable — `nil` is a failed
validation, not a crash:

```rut
impl Version {
    pub fn parse(s: str) -> ?Version {
        // ... split "1.2" — on failure:
        return nil;
    }
}
```

**No inheritance.** There is no `extends`, no `super`, no overriding.
Code sharing is composition (hold a helper in a field) or free
functions; polymorphism is traits — the next chapter.

## Visibility recap

- Struct members: public, always.
- Class members: unannotated is module-private; `pub` exposes to
  importers (also `pub(mod)`, `pub(super)`, `pub(self)`).
- The types themselves follow the same rule: `pub class Vec<T>` is
  importable, a bare `class Helper` stays in its module. See
  [modules and packages](modules.md).

## Struct or class?

Default to a struct: data in, data out, shared like every other value.
Reach for a class when the type has rules that construction must
enforce, holds private state that outsiders must not read, or owns a
resource that needs explicit cleanup.

## Put it together

```rut
use ink::{ Logger };

class Counter {
    n: i32 = 0;
}

impl Counter {
    pub fn new() -> Self {
        return Self { };
    }

    pub fn press(mut self) {
        self.n = self.n.wrapping_add(1);
    }

    pub fn count(self) -> i32 { return self.n; }
}

class Rect {
    w: f32;
    h: f32;
}

impl Rect {
    pub fn new(w: f32, h: f32) -> Self {
        if (w <= 0 || h <= 0) {
            panic("Rect: negative extents");
        }
        return Self { w: w, h: h };
    }

    pub fn from_square(s: f32) -> Self {
        return Rect.new(s, s);
    }

    pub fn area(self) -> f32 { return self.w * self.h; }
}

pub fn main() {
    let log = Logger.new("classes");
    let c = Counter.new();
    c.press();
    c.press();
    let r = Rect.new(3, 4);
    let sq = Rect.from_square(2.0f32);
    log.info(f"count={c.count()} area={r.area()} square={sq.area()}");
}
```

```text
count=2 area=12 square=4
```

Next: [traits and impl blocks](traits.md).
