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
name in a `use` is a lint, not an error.

Within one module, everything is visible — including declarations
later in the file. Order never matters.

## Packages: `rut.jsonc`

A package is a directory with a manifest. The small but complete case —
one library package and one app:

```text
greet/
├── pkg/
│   ├── rut.jsonc
│   └── greet.rut
└── app/
    ├── rut.jsonc
    └── main.rut
```

```jsonc
// greet/pkg/rut.jsonc
{
  "name": "greet",
  "entry": { "lib": "./greet.rut" },

  "deps": {
    // the toolchain's pouch package, from jsDelivr — pinned by sha256
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v8/dist/std/pouch.rutbundle", "sha256": "128ffcf7daa2c43ed3ebad00a6a91ff7c4c494f2442c0a80d6f2c0daa455cc34" }
  }
}
```

```rut
// greet/pkg/greet.rut
use pouch::{ Vec };

pub struct Greeting {
    to: str;
    lines: Vec<str>;
}

impl Greeting {
    pub fn new(to: str) -> Self {
        return Self { to: to, lines: Vec.new() };
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

The app names its dependencies in `deps`, by url or path — each
package pulls its own dependencies along (`ink` brings the host
surface `ink_host`; you never spell it):

```jsonc
// greet/app/rut.jsonc
{
  "name": "app",
  "entry": { "lib": "./main.rut" },

  "deps": {
    // your own package: a sibling directory
    "greet": { "path": "../pkg" },
    // the toolchain's ink package, from jsDelivr — pinned by sha256
    "ink": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v8/dist/std/ink.rutbundle", "sha256": "04d069cdf71922e7cd41224ee813b78f568c66b878d909e92b026be2530e57c0" }
  }
}
```

```rut
// greet/app/main.rut
use greet::{ Greeting };
use ink::{ Logger };

entry fn main() {
    let log = Logger("app");
    let mut g = Greeting.new("rut");
    g.add("hello");
    g.add("from a package");
    log.info(g.render());
}
```

Run the app from its directory:

```sh
rut run .
```

```text
dear rut, hello from a package
```

Resolution walks `deps` recursively (a dep's own `deps` mount with
it), with a cycle guard and first-mount-wins.

## One package, several files

A package's body can be split across files. The manifest splices them,
base first, in listed order — into **one module**: one namespace, one
visibility scope. A name private to one file is visible to every other
file of the same package:

```jsonc
{
  "name": "app",
  "entry": { "lib": "./biz.rut", "libs": ["./domain.rut", "./world.rut", "./app.rut"] }
}
```

This is assembly, not an include form — cross-package references still
go through `use` paths.

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
