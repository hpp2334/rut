# Modules and visibility

Module scope contains **declarations only** — every statement lives
inside a function, and **loading a module executes nothing**. Visibility
is private by default, with package-scoped forms for libraries.

## Module structure

Allowed at module scope:

| Declaration | Spelling |
|---|---|
| import | `use pkg::{ A, B };` / `use pkg::A;` / `use pkg::a::b::{ C };` — `pub`-prefixed to re-export (see below) |
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

A `mod NAME;` declaration mounts a **file module**. The child has two
spellings, probed beside the declaring file in order (the whole tree
under the package's `src/` shell — [Project
structure](project-structure.md)):

1. the **directory form** — `NAME/mod.rut` (or `NAME/mod.d.rut` on a
   host surface tree);
2. the **single-file form** — `NAME.rut` in the same directory.

Both spellings present is a loud ambiguity error naming both — the
mount is undecidable until you delete one. There are no inline
`mod X { .. }` blocks anywhere — files are the one mechanism, ONE mod
grammar in declaration and implementation files alike; a stray block
is the ordinary ``expected `;` `` error. The package's
root module is `src/mod.rut` under its manifest ([Project
structure](project-structure.md)); a child's own declarations may
declare grandchildren (`layout/mod.rut` declaring `pub mod grid;`
mounts `layout/grid/mod.rut`), recursively — the whole tree under
`src/`. Mounting follows
declarations in source order, cycle-guarded by file identity; a
repeated name mounts once. A missing child (under both spellings) or
a directory without its `mod.rut` is a loud error naming the
spellings. Logical mod paths are unchanged by the spelling — a
single-file `imp.rut` mounts at module path `imp` exactly as the
`imp/mod.rut` directory would, so qualified positions, uses, and
visibility read identically over either shape. `mod` versus `pub mod`
feeds the same visibility law every declaration follows (below): a
`pub mod` child's `pub` members cross packages; a plain `mod` child is
the package's own. A host surface tree organizes the same way —
`NAME/mod.d.rut` beside the declaring `.d.rut` — and a published
group's mod tree rides the rows envelope over the wire, structure in
the rows, never in the grammar ([Module bundles](bundles.md)).

### Uses

```rut
use pouch::{ Vec };
use ink::Logger;

entry fn main() {
    let log = Logger("uses");
    let v = Vec<i32>::new();
    log.info(f"{v.len()}");
}
```

```text
0
```

The use path is **`pkg::` followed by module segments**: the head is
the mounted package's bare `[a-zA-Z0-9_]+` name — or the consumer's
own package (its name, or one of its `mod` edges; see the
intra-package import below) — then zero or more **module segments**
walking the package's file modules, then the leaf — a brace list of
names, or a single name:

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
(`Logger(..)`), so there is nothing for the user to annotate. A use
line may spell only what the target exports: a name the target doesn't
declare is rejected at the use line itself —
`` `ink` has no `Nope` — unknown import `` — not later, at a call site
that might never be written. An unreferenced use name is a lint, not
an error.

**Every leaf must bind — at the use line.** Validation is total and
it happens where the `use` was written, before any body compiles: a
leaf that binds nothing in its own file is a hard error there. A name
the target declares but does not export — a private row — names the
fix on the exporter's side:

```text
`helper` is not exported by `pkga` — mark it `pub` there
```

(a qualified leaf names the walked module: `` not exported by
`pkga::layout::grid` ``; the intra-package lane keeps its own
visibility spelling, naming both files). A compiled dependency — a
packed `.rutbundle` — carries its declarations as a binary surface
with no source text, so a leaf its surface never carried states the
bare law: `` `leaf` is not part of `pkga`'s exported surface — nothing
binds in this file ``. The rows carry the field dial too — every
carried class field stamps its visibility — so a member access on a
dep class gates from the carried rows, exactly as source does: only a
`pub` field crosses the package line (struct fields stay the
always-public exemption), and anything stricter refuses, naming the
origin package — `` `Box.v` — `v` is private to package `dep` —
declare it `pub` to cross ``.

**A `use` names its leaves — there is no `*`.** The glob leaf is gone
from the grammar, in both of its spellings — bare at the leaf
(`use tur_kit::layout::*;`) and inside a brace list
(`use tur_kit::{ Cell, * };`). A `*` on a use line is a parse error at
the star, naming the fix:

```text
there is no `*` in a `use` — a `use` names its leaves (`use pkg::{ A, B }`)
```

Every name a file binds, it spells: its own declaration, or an explicit
leaf on one of its `use` lines. To bring a module's names into a file,
list them — `use tur_kit::layout::{ Cell, Column, Span };`.

**A `use` binds its own file.** The binding a `use` creates is
visible in the file that wrote it and nowhere else: every file of a
multi-file package spells its own imports, the root's `use` never
feeds a child module's file, and a sibling's never feeds its
sibling's. There is no ambient fallback beyond the engine's own names
(the builtins below); beyond them, a bare dep name resolves in a file
iff that file's own `use` bound it, and a miss at the call site says
exactly which fix applies — the name rides another file's use:

```text
`fetch` is bound by a `use` in the root module — write the `use` in this file
```

(intra-package bindings add where that file imported it from), or the
name is a mounted export no use spells at all:

```text
`dup` is not bound — no `use` statement names it
```

**One name, one leaf, per file.** The name law is NAME-level, not
package-level, and it covers every module-scope binding — not just
use leaves. A file's scope binds each name at most once, by a
declaration of any kind (class, struct, enum, interface, alias, fn,
module `let`, `mod` edge) or by a use leaf; the SECOND spelling of a
name in the same file errors, citing the first:

```text
class Widget { }        // fine
fn Widget() { }         // ERROR: duplicate name `Widget` — the class already declares it in this file
```

```text
use pkga::{ x };
use pkgb::{ x };
// duplicate name `x` — `use pkga::{ x }` (mod.rut) already binds it in this file
```

`use pkga::{ x }; use pkga::{ y }` is fine — the name is the rule,
never the package, the kind, or the list.

**The layer model.** A name resolves through three lexical layers,
inner wins:

1. **local** — the fn body (its params and locals); shadowing is
   legal inside a body and is nobody else's business;
2. **file scope** — the file's own module-scope declarations plus the
   bindings its `use` lines wrote. This is the layer the one-name law
   governs: uniqueness exactly where names meet;
3. **ambient** — the engine's own names (the builtins below). The
   fallback layer: shadowable, never colliding — `fn panic` in your
   file is legal, and a call spells yours.

**Siblings never meet — across kinds.** The law is per FILE (module ↔
file is 1:1): a fn in one sibling beside a class in the other, two
module `let`s of one name, or a `mod NAME;` edge beside a sibling's
decl — all legal, each file resolving its own. Two of the SAME kind
still refuse across siblings — two fns, or two types, of one name
(the two exceptions spelled out below). Cross-module access stays
explicit: qualified paths (`a::Foo`) or use bindings in the importing
file. The miss hints point at the `use` fix:

```text
`Foo` is declared in module `math` (math/mod.rut) — bind it with `use math::{ Foo }` (or qualify: `math::Foo`)
```

**Not in the law.** Everything with its own narrower scope keeps its
own checks, unchanged: enum members, fields, methods, impl-block
methods (type-scoped), and params/locals (fn-scoped).

**Two same-kind refusals across modules.** A package's fn table and
its type table are each keyed by name, so two sibling modules cannot
both declare a fn — or a type — of one name today; the refusal cites
the first spelling's module. Two `fn`s of one name in different
modules: ``duplicate fn `f` — already declared in module `a```. Two
types of one name in different modules: ``duplicate type name `T` —
already declared in module `a```. A fn and a type of one name across
sibling modules is legal — the collision the old precedence rule used
to arbitrate cannot even spell.

**`entry fn` by name.** The host calls entry fns BY NAME, so a
program may spell many differently-named entries (a dispatcher's
surface; the run lane picks with `--entry`, and without the flag
several entries refuse to guess). What may not repeat is a NAME: two
sibling modules both spelling `entry fn main` refuse at the second —
the ambiguity the entry gate owns.

**`pub use` — the re-export.** A use may carry the `pub` prefix, with
the same scoped forms every declaration takes:

```rut
pub use util::{ helper, Widget };    // both names surface through THIS module
pub(pkg) use util::{ scoped };       // package-wide, no further
use util::{ hidden };                // a private import — nothing joins the surface
```

The prefix turns the import into a **re-export edge**: the imported
names join the module's public surface as if declared there, each
carrying the use's visibility — `pub` crosses packages,
`pub(pkg)`/`pub(super)` cross per the scoped law, and a private `use`
imports only (nothing joins the surface; consumers never see it).
Consequences:

- **The leaf gate reads the row.** A consumer importing the module may
  spell a re-exported name like any of its `pub` names — `use
  mid::{ helper }` names a name `mid` *did* declare — while the same
  name behind a private `use` stays the unknown import it always was.
- **A dependency's name may ride the edge.** `pub use` of a dep's
  name is a re-export edge, not a private import: the name surfaces
  through the re-exporting module, so a consumer validates its import
  against *that* module — the deeper package stays an implementation
  detail of the exporter. The exporter's compiled surface carries the
  re-export as a **surface row** whose origin is the declaring package:
  a fn, generic fn, type (class/struct), enum, interface, or host-surface
  const re-export binds through it like any of the exporter's own `pub`
  names, and a chain of re-exports collapses to the original declaration.
  A module `let` has no surface row (it is not a surface const), so a
  `pub use` naming one refuses at the re-exporting use line.
- **The editor reads the edge.** Re-exported names complete through
  the module (`use mid::{ ⏐` offers `helper`, the detail naming the
  edge), and the imported names carry the re-export visibility.

**`pub async fn` crosses package surfaces.** A plain `pub async fn`
exports like any `pub fn`. The exporter materializes the sugar law's
export shape beside the woven body — a hidden start wrapper spelling
`fn f(p) -> Future<T>` that mints the frame — and the surface row
carries the declared **answer** type. The consumer's `use` binds the
name, a call answers the closed `Future<answer>`, and `await` drives
it like any future: the wrapper mints the frame in the one unit that
knows its full layout, and the drive reaches the exporter's woven
body through the frame's yield slot exactly as a local call would.
The one gap is the **generic** async fn — the weave refuses generics,
no row materializes, and a use leaf naming one refuses at the use
line: `` `grab` is a generic `pub async fn` — generic async fns do
not cross package surfaces yet ``.

The head may also name the **consumer's own package** — the
intra-package import. The package's own name resolves to its root
module, and a `mod` edge of its own tree resolves by walking from the
file the `use` is written in (the current module first, then the
ancestors — the same walk qualified positions take):

```rut
use imp::{ TodoList, Task };   // beside `mod imp;` — imp's pub names

mod imp;
```

In a package with no child module the own name is no use head: the
compiler refuses the line — name the declaration's module, or declare
the dependency.

The leaves obey the same laws every crossing does. Each must be a
declaration of the walked module, **visible from the importing file**:
a bare, private member of `imp` is not importable — `pub` or `pub(pkg)`
is (import is no visibility bypass). A module itself is not an import —
walk it (`use imp::grid::{ Cell }`, not `use imp::{ grid }`). And the
binding is per file, exactly like a dep use: a module needs its own
`use` line. One head must never answer as two things: a name that is
both a declared dep and an own module edge is a loud ambiguity —
rename one of the two.

The head itself is **declaration-scoped**: a package may `use` exactly
the heads its own manifest names — `deps` ∪ (`dev-deps`
when the package is the program root) ∪ `core`. Mounting stays
transitive (a dep's own `deps` still join the program's closure), but
a head that is mounted without being declared by *this* package is the
resolver's dedicated diagnostic, naming who pulled it in and the row
to add ([dependency kinds](dependency-kinds.md)).

The engine's builtin names — the primitives, `opaque`, `panic`,
`type_id<T>()`, `str(x)` — are **ambient**: no `use` is
needed for them. The gated names resolve only through
`use core::{ .. }`: the surface const `NAN`, the disposal context
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
**qualified position**: `layout::Column`, `store::Source<str>`,
`widget::mk(3)`. The head resolves against the current module's scope,
then the module's ancestors up to the package root; positions never
take a *dep* package head — `use` statements are the only cross-package
door. `use` is also the intra-package import — a use head may be this
package's own name, or one of its `mod` edges (see
[Uses](#uses)) — so a cross-file name binds bare once imported;
positions stay spelling-free: `imp::grid::Cell` never needed an
import, and still spells. `::` is THE path separator — every
name-resolution path (use paths, qualified positions, enum members,
statics/associated fns, dep module paths) spells its segments with
it; `.` is value access only (fields, instance methods, tuple indices
— see [Lexical structure](lexical-structure.md)). One separator, two
doors:

| form | spelling | door |
|---|---|---|
| use path | `use pkg::a::b::{ C }` | `::` — cross-package **and own-package module**, the import |
| qualified position | `a::b::C` / `a::b::f(..)` | `::` — intra-package, at the use site |

### Module-level `let`

Initializers must be **load-time literals** — this build accepts an
`Int`, `Float`, `Str`, or `Bool` literal and nothing else. An operator,
an enum member, a record literal, `[nil; n]`, `type_id<T>()`, or any
call is refused:

```text
error: module `let` initializers must be load-time expressions — literals only in this build
```

```rut
let KIND_ADD: i64 = 1;                 // fine: an Int literal
let NAME: str = "app";                 // fine: a Str literal
```

There is no mutable module state: program state is constructed in
`main`, or held by the embedder and passed across the boundary. **No
user code runs at load.**

### Entry points

The embedder loads a module and then explicitly calls an entry
function — `entry fn main` for a script; an entry fn is sync. `entry
fn` publishes a function to the *embedder*; `entry` is orthogonal to
visibility and does not combine with `pub`. Consequences of
declarations-only loading: no use side-effect ordering, no load-order
bugs, deterministic and cheap loads.

## Bindings: `let` and `const`

Mutability is the **default**. A `let` binding may be reassigned
(`x = e`), written through (`p.x = 3`, `xs[0] = 7`), and passed where a
writer is expected — including every method call: parameters are
mutable too, so `fn step(p: Point)` may write the caller's point, and
there is no `mut` parameter spelling to grant it.

- `let x = e;` — a **mutable binding**, the ordinary shape.
- `const x = e;` — the **opt-out**: a *const binding* refuses the
  binding's own reassignment, diagnosing
  ``cannot reassign `x` — a `const` binding is not reassignable``.

**The const opt-out is shallow — reassignment only.** It gates the one
operation that rebinds the name: `x = ...` on a const binding errors.
It is *not* deep immutability — writes through the binding stay legal,
so `p.x = 3` on a `const p` compiles and mutates the shared object,
exactly as on a `let`. Nothing in rut freezes a record; if you need a
value no one can write through, build a fresh one (a literal, or a
copy of the fields you need) rather than expecting `const` to guard it.

Sharing is not gated at all: every non-primitive is a shared cell (see
[By-reference and nullable](by-reference-and-nullable.md)), writes are
visible through every handle, and a const binding shares like any
other — `const` is *about the name*, never a copy or a fence.

> **Two meanings of "const".** This page's `const` is the *const
> binding* — the statement head `const x = e;` in rut source. The
> **surface const** is a different animal: the load-time constants a
> dependency's `.d.rut` interface (or a host package) exports, bound
> bare by a `use` — spelled `let` in the interface, never `const`
> (see [Host fns and declaration files](host-fns.md#the-surface-grammar)).

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
- The paren forms `pub(mod)` / `pub(self)` are gone — a paren scope
  other than `super`/`pkg` is the ordinary `pub(..)` error.
- Applies uniformly: `let`, `enum`, `struct`, `class`, `interface`,
  `impl` (an impl exports with its target type), `fn`, `type` aliases,
  the `mod` edge itself (`pub mod` opens the child's `pub` members
  to other packages; a plain `mod` keeps the child package-internal),
  and the `use` edge (`pub use` re-exports its imported names — see
  [Uses](#uses)).
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
  behind exported ones. Hiding stops at the checker's door: every
  non-generic fn body whose parameters are not interface-typed is
  type-checked even when nothing calls it. The two exclusions are by
  design — such a body compiles only where something calls it: a
  generic fn or method is monomorphized at its call sites (the call
  supplies the substitution), and a fn with an interface-typed
  parameter is specialized per call (the call supplies the
  parameter's concrete origin), so a never-called body of either is
  never checked. Methods have no interface-typed-parameter exclusion —
  a non-generic method body is checked dead like any other.
