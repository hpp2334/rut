# Modules and visibility

Module scope contains **declarations only** — every statement lives
inside a function, and **loading a module executes nothing**. Visibility
is private by default, with package-scoped forms for libraries.

## Module structure

Allowed at module scope:

| Declaration | Spelling |
|---|---|
| import | `use pkg::{ A, B };` / `use pkg::A;` |
| binding | `let name: T = expr;` (also under `pub`) |
| enum / struct / class / trait | `enum E { .. }`, `struct S { .. }`, `class C { .. }`, `trait I { .. }` |
| impl block | `impl T { .. }`, `impl I for T { .. }` |
| function | `fn f(..) { .. }`, `async fn f(..) { .. }` |
| entry point | `entry fn f(..) { .. }` |
| alias | `type X = A;` (see [Type aliases and union bounds](type-aliases.md)) |

Anything else — calls, any statement — is a compile error:
*statements are not allowed at module scope — modules contain
declarations only*.

`host fn` / `host struct` and `builtin` declarations are
signature-only native surfaces: `host` rows live in declaration files
(`.d.rut`) and `builtin` rows in the engine's own `core` surface, never
as executable bodies in `.rut`.

### Uses

```rut
use pouch::{ Vec };
use ink::Logger;

pub fn main() {
    let log = Logger.new("uses");
    let v = Vec<i32>.new();
    log.info(f"{v.len()}");
}
```

```text
0
```

The package is **one bare identifier**; the names are one or more
idents. rut is fully statically typed: the compiler resolves every used
name and knows from usage whether it lands in type position (`Vec` in
an annotation) or value position (`Logger.new(..)`), so there is nothing
for the user to annotate. An unreferenced use name is a lint, not an
error.

The engine's builtin names — the primitives, `opaque`, `panic`,
`type_id<T>()`, `str(x)` — are **ambient**: no `use` is
needed for them. The gated names resolve only through
`use core::{ .. }`: the const `NAN`, the `Disposal`/
`DisposalContext` pair, and every builtin trait (`Iterator`,
`Future`, `RunContext`) — the engine's weave never needs the import,
only source that spells a trait name does
([Host fns and declaration files](host-fns.md)).
Package code (`pouch`, `ink`, `nmapset`, ...) mounts only through
`use`.

### Module-level `let`

Initializers must be **load-time expressions**: literals, enum members,
builtin operators over load-time expressions, struct literals whose
fields are load-time expressions, fixed-array literals whose elements
are, `type_id<T>()`, and builtin zero allocations (`[nil; n]` with a
const `n`). Calls to user functions are not load-time expressions.

```rut
let KIND_ADD: i64 = 1;
let ORIGIN = Point { x: 0, y: 0 };
```

There is no mutable module state: program state is constructed in
`main`, or held by the embedder and passed across the boundary. **No
user code runs at load.**

### Entry points

The embedder loads a module and then explicitly calls an entry
function — conventionally `pub fn main`, sync or async. `entry fn`
publishes a function to the *embedder*; `entry` is orthogonal to
visibility and does not combine with `pub`. Consequences of
declarations-only loading: no use side-effect ordering, no load-order
bugs, deterministic and cheap loads.

## Bindings: `let` vs `let mut`

- `let x = e;` binds immutably — no reassignment, no field assignment
  through `x`, and no `mut self` method calls on it. It is a **read-only
  view of the shared object**.
- `let mut x = e;` may be reassigned and grants write access **to the
  object it holds**. Every non-primitive is a shared cell (see
  [By-reference and nullable](by-reference-and-nullable.md)), so that
  write is visible through every other handle: `mut` is *permission*,
  never a copy. Every assignment path must run through a `mut` binding.
- Parameters may declare `mut name` — the callee's permission to mutate
  **the caller's object**: `fn step(mut p: Point)` moves the caller's
  point; `fn area(p: Point)` promises not to.

## Visibility

Modules form a tree per package (files in directories; the package root
is the root module). Every declaration — and every class member —
carries a visibility:

| Form | Meaning |
|---|---|
| `pub fn ..` | **public** — nameable by any rut module (other packages included) |
| `pub(mod) fn ..` | visible everywhere inside this **package's module tree** |
| `pub(super) fn ..` | visible to the **parent module** only |
| `pub(self) fn ..` | module-private — **the default** |

- Unannotated = `pub(self)`: nothing leaks unless it says `pub`. There
  is no `private` keyword — the unannotated default *is* the private
  spelling.
- Applies uniformly: `let`, `enum`, `struct`, `class`, `trait`, `impl`
  (an impl exports with its target type), `fn`, `type` aliases. On a
  `class` declaration it means the *type name* is visible.
- **Class members take the same forms**: an unannotated field or method
  is module-private; `pub` (optionally scoped) exposes it. See
  [Classes and constructors](classes.md).
- Dataclass members are **always public** — no visibility dial (see
  [Structs](structs.md)). Trait method signatures and impl methods are
  as visible as their trait (see [Traits and dispatch](traits.md)).
- Visibility is checked at compile time; it has no runtime
  representation. Only `pub` names enter a module's export table; a
  non-exported declaration is *known* inside its module but *nameable*
  nowhere else — which is how libraries hide implementation surfaces
  behind exported ones.
