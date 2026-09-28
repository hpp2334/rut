# Structs

`struct` — the open data record: all fields public, constructed only by
literal, sharing its cell like every non-primitive.

## Declaration

```rut
struct Point {
    x: f32;
    y: f32;
}

struct Style {
    color: u32 = 0xff00ff;   // field initializer: literals may omit it
    width: f32 = 1;
}
```

The keyword is `struct`. The removed spelling `dataclass` is a reserved
word whose error names the replacement.

## Reference semantics

A struct value is a **heap cell handle** (see
[By-reference and nullable](by-reference-and-nullable.md)):
assignment, argument passing, and returning share the cell, and a
mutation through any alias is visible through all of them.

```rut
let mut p = Point { x: 1, y: 2 };
let q = p;                       // SHARE: q and p name one cell (O(1))
p.x = 4;                         // q.x is 4 now — sharing is the law
```

Writing is gated by the `mut`-binding law (see
[Modules and visibility](modules-and-visibility.md)): field stores and
`mut self` methods need a `let mut` binding (or a `mut` parameter).
Parameters declare their intent: `fn nudge(mut pt: Point)` may write
the caller's point; `fn length(pt: Point) -> f64` promises not to, and
returns a fresh record instead:

```rut
fn nudged(pt: Point) -> Point {     // builds a NEW record
    return Point { x: pt.x + 1, y: pt.y };
}
```

`==` on two struct values is a **cell-identity test** — `q == p` is
true exactly when they name one cell; two separately built literals are
never equal. Field-wise comparison is a trait contract of your own
(declare and implement it — see
[Traits and dispatch](traits.md)).

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
let s = Style {};                     // zero-value defaults fill the fields
let r = Rect { min: Point { x: 0, y: 0 }, max: q };  // max shares q's cell
```

## All fields public, always

A struct is an open data record: member visibility in a struct body is
a compile error (privacy needs construction control, which is the
class's job — see [Classes and constructors](classes.md)). Structs
also have no `static` members.

## Methods live in impl blocks

A struct body is **fields only** — a `fn` member in the body is a hard
parse error. Inherent methods live in `impl S { .. }`, in the type's
module only; trait impls in `impl I for S { .. }` (see
[Traits and dispatch](traits.md)):

```rut
impl Point {
    fn dist(self, other: Point) -> f32 { .. }   // inherent — the type's module
}

impl Hashable for Point {
    fn hash(self) -> u64 { .. }
    fn eq(self, other: Point) -> bool { .. }
}
```

Free functions over data remain the default idiom; methods are for
tight helpers, impl blocks for trait contracts.

Limits, exhaustively:

- **no member visibility** — all fields are public, always;
- **no class methods** — the literal is the only construction (open
  literal vs class-method-gated *is* the struct/class distinction);
- **no destructor** — a value shared everywhere has no single death to
  hook; if you need one, write a class and attach `on_drop` (see
  [Rc, dispose, and identity](rc-dispose-identity.md)).

Everything else class-shaped is allowed, including `impl` blocks.

## Traits and representation

- Widening a struct to a trait `I` **attaches the impl vtable to the
  same handle** — no allocation, no copy: the trait-typed value aliases
  the record, and mutations through it are visible to every other
  handle.
- Representation: one slot per field inside the cell, in declaration
  order — primitive fields widened into their slot, composite fields as
  cell-handle slots (see [Reified types and layout](reified-types.md)).
  Recursive shapes (`next: ?Node`) are legal because composite fields
  are pointer-sized.
- Structs are for small data (points, rects, colors, configs), but any
  size is allowed.
