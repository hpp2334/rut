# Modules and packages

A rut file is a module: one namespace, one visibility scope. A
directory with a `rut.jsonc` is a *package* — the unit you depend on and
share. This chapter walks up those two levels. The reference pages are
[modules and visibility](../reference/modules-and-visibility.md),
[project structure and rut.jsonc](../reference/project-structure.md),
and [dependency kinds](../reference/dependency-kinds.md).

## What lives at module scope

Module scope contains **declarations only**: `use`, `let`, `fn`,
`struct`, `class`, `enum`, `interface`, `impl`. Every statement lives
inside a function — and **loading a module executes nothing**. There is
no load-time side-effect ordering to reason about; the host loads your
module and calls one of its `entry fn`s (`entry fn main` for a plain
program).

Module-level `let` initializers must be load-time literals — `42`,
`"app"`, `true`. Arithmetic, record literals, and calls to user
functions are not accepted in this build (there is no mutable module
state; programs build their state in `main`).

```rut
use pouch::{ Vec };
use ink::{ Logger };

struct Point { x: f32; y: f32 }

let version = 1;                       // fine: a literal
let app_name = "app";                  // any literal works

fn main_body() { /* statements live here */ }

entry fn main() {
    let log = Logger("app");
    let origin = Point { x: 0, y: 0 }; // record literals live in function bodies
    log.info(f"{app_name} v{version} origin.x={origin.x}");
}
```

```text
app v1 origin.x=0
```

## Visibility

Every declaration has a visibility, and unannotated means
**module-private** — the safe default. Nothing leaks unless it says
`pub`:

| Form | Meaning |
|---|---|
| `pub fn ..` | public — importable by any module, other packages included |
| `pub(mod) fn ..` | visible everywhere in this package's module tree |
| `pub(super) fn ..` | visible to the parent module only |
| `fn ..` | module-private (`pub(self)`) — the default |

The same forms apply to types, module `let`s, and class members.
Structs are the exception: a struct is an open record — all members
public, always, and its impl methods take no visibility annotation
(they are as public as the type).

## `use` — naming other modules

`use` imports names with Rust-like paths:

```rut
use ink::{ Logger };
use pouch::{ Vec };
use json::{ decodeJsonBytes, JsonDeserialize };
```

Builtin names — the primitives, `str`/`bytes` members, `panic`,
`Vec`-free array grammar, `opaque` — are **ambient**: no
`use` needed. The exceptions are core's import-gated names — the
disposal context `DisposalContext` and the weak reference `Weak` —
they resolve only through `use core::{ .. }`, like any package name.
Package names from your manifest are imported the same way; an unused
name in a `use` is a lint, not an error, while a name the target
doesn't export is rejected at the use line itself —
`` `ink` has no `Nope` — unknown import ``.

Within one module, everything is visible — including declarations
later in the file. Order never matters.

## Packages: `rut.jsonc`

A package is a directory with a manifest. The small but complete case —
one library package and one app:

```text
greet/
├── pkg/
│   ├── rut.jsonc
│   └── mod.rut            # the package's root module
└── app/
    ├── rut.jsonc
    └── mod.rut            # the app's root module
```

```jsonc
// greet/pkg/rut.jsonc
{
  "name": "greet",

  "deps": {
    // the toolchain's pouch package, from jsDelivr — the pin lives in
    // the app's rut.lock (one lock per walk root)
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v9/dist/std/pouch.rutbundle" }
  }
}
```

```rut
// greet/pkg/mod.rut
use pouch::{ Vec };

pub struct Greeting {
    to: str;
    lines: Vec<str>;
}

impl Greeting {
    pub fn new(to: str) -> Self {
        return Self { to: to, lines: Vec::new() };
    }

    pub fn add(mut self, line: str) {
        self.lines.push(line);
    }

    pub fn render(self) -> str {
        let mut out = f"dear {self.to},";
        for (let line of self.lines) {
            out = f"{out} {line}";
        }
        return out;
    }
}

fn shout(msg: str) -> str {   // module-private: no `pub`, never importable
    return f"{msg}!";
}
```

The app names its dependencies in `deps`, by url or path. One table
carries two laws: **mounting** is transitive — each package pulls its
own dependencies along (`ink` brings the host surface `ink_host`; you
never spell it) — while **nameability** is declaration-scoped: the app
may `use` exactly the heads its own manifest names. `ink_host` mounts
here, but only `ink` declared it, so only `ink`'s sources may name it;
an app-level `use ink_host::` is refused, naming the fix (add the row
to *your* `rut.jsonc` `deps`):

```jsonc
// greet/app/rut.jsonc
{
  "name": "app",

  "deps": {
    // your own package: a sibling directory
    "greet": { "path": "../pkg" },
    // the toolchain's ink package, from jsDelivr — the pin lives in
    // this app's rut.lock
    "ink": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v9/dist/std/ink.rutbundle" }
  }
}
```

```rut
// greet/app/mod.rut
use greet::{ Greeting };
use ink::{ Logger };

entry fn main() {
    let log = Logger("app");
    let mut g = Greeting::new("rut");
    g.add("hello");
    g.add("from a package");
    log.info(g.render());
}
```

Run the app from its directory — `rut install` fetches the url deps
and writes the app's `rut.lock` (the pin ledger every load verifies
against; it pins the whole closure, including the url row `../pkg`
declares), then `rut run .`:

```sh
rut install .
rut run .
```

```text
install: 2 url deps resolved — 2 downloaded, 0 cache hits (10594 bytes total)
rut.lock written: 2 rows (2 added, 0 changed, 0 dropped)
dear rut, hello from a package
```

Resolution walks `deps` recursively (a dep's own `deps` mount with
it), with a cycle guard and first-mount-wins. The walk decides what is
**mounted**; what a source may **name** is its own manifest's
business: `deps` ∪ `peer-deps` ∪ `core` (plus `dev-deps` at the
program root). A head that is mounted but undeclared — riding along on
someone else's row — refuses with a dedicated diagnostic naming the
mount edge and the row to add
([dependency kinds](../reference/dependency-kinds.md)).

## One package, several files

A package's body can be split across files — **file modules**. The
root module is `mod.rut` beside the manifest, and a `mod NAME;`
declaration in it mounts the sibling `NAME/mod.rut` as a child module:

```text
app/
├── rut.jsonc        # { "name": "app" } — no entry keys
├── mod.rut          # the root module: `mod domain; pub mod world;`
├── domain/mod.rut   # app's `domain` module
├── world/mod.rut    # app's `world` module
└── app/mod.rut      # the shell
```

Each file is its own module: cross-file references are qualified
(`domain::Todo`, `world::boot(..)`), and visibility is per module —
private to the declaring module, `pub(pkg)` to share inside the
package, `pub` to cross packages. Cross-package references still go
through `use` paths (`use app::world::{ World }`).

## Dependency kinds

Beyond `deps`, a manifest can declare two other relations:

- **`peer-deps`** — a package this one *integrates with* but never
  pulls: the consumer supplies it, or (with `optional = true`) the
  integration mounts only if the peer is already in the program's
  closure. The standard library's `json` package uses this to attach
  its `Vec`/map serialization impls only for programs that carry the
  container packages.
- **`dev-deps`** — mounted only when building/testing the package
  itself, never for a consumer.

```jsonc
{
  "peer-deps": {
    "pouch": { "path": "../pouch", "optional": true, "lib": "./group-pouch.rut" }
  },

  "dev-deps": {
    "pouch": { "path": "../pouch" }
  }
}
```

## `entry fn` — the host-facing surface

`pub` publishes a name to other *rut* modules. `entry` publishes a
function to the *embedding host*, with its signature checked against
the boundary's crossing rules at compile time:

```rut
entry fn hex_enc(data: bytes) -> str { .. }
```

An embedded application drives these entries; `rut run` executes the
program's entry designation — exactly one `entry fn` runs it, several
take `--entry <name>`. See
[the host boundary](../core-concepts/host-boundary.md).

## Tooling

```sh
rut run <dir | mod.rutbundle> [--entry <fn>] [--fuel N]      # compile + run
rut fmt <file.rut | dir> [--check]                    # format in place
rut pack <dir> [-o out.rutbundle]                     # a self-contained bundle
rut dump <file.rut>                                   # dump module info
```

## Put it together

The `greet` project above is complete as shown — two directories, two
manifests, two sources. Copy the trees into files and `rut run .` from
`greet/app` to see:

```text
dear rut, hello from a package
```

Next: [async: futures and await](async.md).
