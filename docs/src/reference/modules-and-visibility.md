# Modules and visibility

Module scope contains **declarations only** — every statement lives
inside a function, and **loading a module executes nothing**. Visibility
is private by default, with package-scoped forms for libraries.

## Module structure

Allowed at module scope:

| Declaration | Spelling |
|---|---|
| import | `use pkg::{ A, B };` / `use pkg::A;` / `use pkg::a::b::{ C };` (see below) |
| child module | `mod name;` / `pub mod name;` — declares a **file module** |
| binding | `let name: T = expr;` (also under `pub`) |
| enum / struct / class / interface | `enum E { .. }`, `struct S { .. }`, `class C { .. }`, `interface I { .. }` |
| impl block | `impl T { .. }` — inherent, the type's module only |
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

### File modules (`mod`)

A `mod NAME;` declaration mounts a **file module**: the sibling
`NAME/mod.rut` directory beside the declaring file. There are no
inline `mod X { .. }` blocks — files are the one mechanism. The
package's root module is `mod.rut` beside its manifest ([Project
structure](project-structure.md)); a child's own declarations may
declare grandchildren (`layout/mod.rut` declaring `pub mod grid;`
mounts `layout/grid/mod.rut`), recursively. Mounting follows
declarations in source order, cycle-guarded by file identity; a
repeated name mounts once. A missing child, a `NAME.rut` file where a
module directory is expected, or a directory without its `mod.rut` is
a loud error naming both spellings. `mod` versus `pub mod` feeds the
same visibility law every declaration follows (below): a `pub mod`
child's `pub` members cross packages; a plain `mod` child is the
package's own.

### Uses

```rut
use pouch::{ Vec };
use ink::Logger;

entry fn main() {
    let log = Logger("uses");
    let v = Vec<i32>.new();
    log.info(f"{v.len()}");
}
```

```text
0
```

The use path is **`pkg::` followed by module segments**: the head is
the mounted package's bare `[a-zA-Z0-9_]+` name, then zero or more
**module segments** walking the package's file modules, then the
leaf — a brace list of one or more names, or a single name:

```text
use tur_kit::layout::grid::Cell;       // one name from a nested module
use tur_kit::layout::{ Column, Span }  // several names from a module
use ink::Logger;                       // the degenerate root path
```

Importing a *module itself* (`use tur_kit::layout;`) is an error —
modules are namespaces, not values; the diagnostic names the fix
(spell a name at the leaf). rut is fully statically typed: the
compiler resolves every used name and knows from usage whether it
lands in type position (`Column` in an annotation) or value position
(`Logger(..)`), so there is nothing for the user to annotate. An
unreferenced use name is a lint, not an error.

The engine's builtin names — the primitives, `opaque`, `panic`,
`type_id<T>()`, `str(x)` — are **ambient**: no `use` is
needed for them. The gated names resolve only through
`use core::{ .. }`: the const `NAN`, the disposal context
`DisposalContext`, the weak reference `Weak<T>`, and the closed async
pair (`Future<T>`, `RunContext`) — the engine's weave never needs the
import, only source that spells one of the names does. The bracket
markers (`[disposal]`/`[iterable]`/`[constructor]`) need no import —
the marker word
is the designation, not a name
([Host fns and declaration files](host-fns.md)).
Package code (`pouch`, `ink`, `nmapset`, ...) mounts only through
`use`.

### Qualified positions

Inside one package, a declaration of another module is nameable where
a path is legal — a type annotation, a call, a field read — by
**dot-qualified position**: `layout.Column`, `store.Source<str>`,
`widget.mk(3)`. The head resolves against the current module's scope,
then the module's ancestors up to the package root; positions never
take a package head — `use` statements are the only cross-package
door. Two spellings, two doors:

| form | spelling | door |
|---|---|---|
| use path | `use pkg::a::b::{ C }` | `::` — cross-package, the import |
| qualified position | `a.b.C` / `a.b.f(..)` | `.` — intra-package, at the use site |

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
function — `entry fn main` for a script, sync or async. `entry fn`
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

Modules form a tree per package (the package root is the root module
`mod.rut`; each declared child is a module file). Every declaration —
and every class member — carries a visibility:

| Form | Meaning |
|---|---|
| `pub fn ..` | **public** — nameable by any rut module (other packages included) |
| `pub(pkg) fn ..` | visible everywhere inside this **package** (its whole module tree) |
| `pub(super) fn ..` | visible to the **parent module's subtree** only |
| *(unannotated)* | module-private — visible in this module and its descendants; **the default** |

- Unannotated = private to the declaring module *and its descendants*
  (a child module sees its ancestors' privates, never a sibling's).
- The paren forms `pub(mod)` / `pub(self)` are repealed — they parsed
  but gated nothing; the parser refuses them with the fix (bare
  private, or `pub(super)`/`pub(pkg)`/`pub`).
- Applies uniformly: `let`, `enum`, `struct`, `class`, `interface`,
  `impl` (an impl exports with its target type), `fn`, `type` aliases,
  and the `mod` edge itself (`pub mod` opens the child's `pub` members
  to other packages; a plain `mod` keeps the child package-internal).
  On a `class` declaration it means the *type name* is visible.
- **Class members take the same forms**: an unannotated field or method
  is module-private; `pub` (optionally scoped) exposes it. See
  [Classes and constructors](classes.md).
- **Struct fields are always public** — the field law has no dial
  (see [Structs](structs.md)). **Impl-block methods take the `pub`
  dial** — `pub fn` exports cross-module, plain `fn` is
  module-private — the same law a class's methods follow, enums
  included (see [Structs](structs.md) and [Enums](enums.md)).
  Interface members take no `pub` at all — they are as visible as
  their interface; satisfaction reads a type's **public** inherent
  members (see [Interfaces and dispatch](interfaces.md)).
- Visibility is checked at compile time; it has no runtime
  representation. Only `pub` names enter a module's export table; a
  non-exported declaration is *known* inside its module but *nameable*
  nowhere else — which is how libraries hide implementation surfaces
  behind exported ones.
