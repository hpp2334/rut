# Structs

`struct` — the open data record: all fields public, constructed only by
literal, sharing its cell like every non-primitive.

## Declaration

```rut
use ink::{ Logger };

struct Point {
    x: f32;
    y: f32;
}

struct Style {
    color: u32 = 0xff00ff;   // field initializer: literals may omit it
    width: f32 = 1;
}

entry fn main() {
    let log = Logger.new("t");
    let s = Style {};
    log.info(f"{s.color} {s.width}");
}
```

```text
16711935 1
```

## Reference semantics

A struct value is a **heap cell handle** (see
[By-reference and nullable](by-reference-and-nullable.md)):
assignment, argument passing, and returning share the cell, and a
mutation through any alias is visible through all of them.

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32; }

entry fn main() {
    let log = Logger.new("t");
    let mut p = Point { x: 1, y: 2 };
    let q = p;                       // SHARE: q and p name one cell (O(1))
    p.x = 4;                         // q.x is 4 now — sharing is the law
    log.info(f"{q.x}");
}
```

```text
4
```

Writing is gated by the `mut`-binding law (see
[Modules and visibility](modules-and-visibility.md)): field stores and
`mut self` methods need a `let mut` binding (or a `mut` parameter).
Parameters declare their intent: `fn nudge(mut pt: Point)` may write
the caller's point; `fn length(pt: Point) -> f64` promises not to, and
returns a fresh record instead:

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32; }

fn nudged(pt: Point) -> Point {     // builds a NEW record
    return Point { x: pt.x + 1, y: pt.y };
}

entry fn main() {
    let log = Logger.new("t");
    let p2 = nudged(Point { x: 1, y: 2 });
    log.info(f"{p2.x} {p2.y}");
}
```

```text
2 2
```

`==` on two struct values is a **cell-identity test** — `q == p` is
true exactly when they name one cell; two separately built literals are
never equal. Field-wise comparison is an interface-shaped contract of
your own (spell a member that compares — see
[Interfaces and dispatch](interfaces.md)).

## Construction — the literal, everywhere

`Name { field: expr, .. }` is the **only** construction. There is no
`new`, no class methods, no type-call. The literal is available
everywhere — function bodies and module-level `let` initializers alike
— and it allocates the cell.

- Fields may be given in any order, by name.
- The literal must initialize **every** field that has no initializer.
- An omitted field with an initializer takes it; `{}` with all-default
  fields is legal.

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32; }
struct Style { color: u32 = 0xff00ff; width: f32 = 1; }
struct Rect { min: Point; max: Point; }

entry fn main() {
    let log = Logger.new("t");
    let q = Point { x: 9, y: 9 };
    let s = Style {};                     // zero-value defaults fill the fields
    let r = Rect { min: Point { x: 0, y: 0 }, max: q };  // max shares q's cell
    log.info(f"{s.color} {r.max.x}");
}
```

```text
16711935 9
```

## All fields public, always

A struct is an open data record: member visibility in a struct body is
a compile error (privacy needs construction control, which is the
class's job — see [Classes and constructors](classes.md)). Structs
also have no `static` members.

## Methods live in impl blocks

A struct body is **fields only** — a `fn` member in the body is a hard
parse error. Methods live in the inherent `impl S { .. }`, in the type's
module only — the one impl form (see
[Interfaces and dispatch](interfaces.md)):

```rut
impl Point {
    fn dist(self, other: Point) -> f32 { .. }   // inherent — the type's module
}

impl Point {
    fn hash(self) -> u64 { .. }                 // a member set, if you want one:
    fn eq(self, other: Point) -> bool { .. }    // satisfaction reads these
}
```

Methods carry real visibility — the class rule: `pub fn` exports
cross-module, plain `fn` is module-private. `Self` spells the struct,
in signatures (`-> Self`) and the literal (`Self { x: 1, y: 2 }`)
alike:

```rut
use ink::{ Logger };

struct Counter { n: i32 }

impl Counter {
    pub fn new() -> Self { return Counter { n: 0 }; }
    fn bump(mut self) -> Self { self.n += 1; return self; }
    fn value(self) -> i32 { return self.n; }
}

entry fn main() {
    let log = Logger.new("t");
    let c = Counter.new().bump().bump();
    log.info(f"{c.value()}");
}
```

```text
2
```

Free functions over data remain the default idiom; methods are for
tight helpers, impl blocks for the members other code observes.

Limits, exhaustively:

- **fields are always public** — no field-visibility dial (privacy
  needs construction control, which is the class's job — see
  [Classes and constructors](classes.md));
- **construction is the literal** — `Point { x: 1, y: 2 }` everywhere
  (open literal vs class-method-gated *is* the struct/class
  distinction).

Everything else class-shaped is allowed — `impl` blocks, `Self`,
`[disposal]`. And the old "no destructor" limit is gone: a shared value
dies exactly when its cell's refcount reaches zero, so cleanup is one
`impl` away — mark a `[disposal]` member on the type and the engine calls
`dispose` at that moment (see
[Rc, dispose, and identity](rc-dispose-identity.md)).

## Interfaces and representation

- Widening a struct to an interface `I` **attaches the itable to the
  same handle** — no allocation, no copy: the interface-typed value aliases
  the record, and mutations through it are visible to every other
  handle.
- Representation: one slot per field inside the cell, in declaration
  order — primitive fields widened into their slot, composite fields as
  cell-handle slots (see [Reified types and layout](reified-types.md)).
  Recursive shapes (`next: ?Node`) are legal because composite fields
  are pointer-sized.
- Structs are for small data (points, rects, colors, configs), but any
  size is allowed.
