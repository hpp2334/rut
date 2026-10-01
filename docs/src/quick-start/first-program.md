# Your first rut program

rut feels like TypeScript on the surface and Rust underneath the hood.
This page walks one program through the whole toolchain — running,
formatting, inspecting, and budgeting — so every later page can assume
the workflow.

## Write and run

This chapter drives everything through the temporary-run `rut` CLI —
the fastest lane for learning the language ([installation](installation.md)).
A real project embeds the engine instead and calls it from Rust
([embedding and native modules](../reference/embedding.md)); the
language, the types, and the workflow below are the same either way.

```rut
use ink::{ Logger };

pub fn main() {
    let log = Logger.new("hello");
    let name = "rut";
    log.info(f"hello, {name}!");
}
```

Every rut program is a **module directory**: a `rut.jsonc` naming the
package and its dependencies, plus the source file the manifest points
at. Create it and run the directory:

```sh
mkdir hello
cat > hello/rut.jsonc <<'EOF'
// hello/rut.jsonc — the manifest IS the program's door
{
  "name": "hello",
  "entry": { "lib": "./main.rut" },
  "deps": {
    // ink — the toolchain's logger package, served by jsDelivr at the
    // std-v3 tag, pinned by sha256
    "ink": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v3/dist/std/ink.rutbundle", "sha256": "200ab1ea41bee7145d20c0665f799302436dcbe82bdab8858c7c74708d1b306a" }
  }
}
EOF
# save the program above as hello/main.rut, then:
rut run hello
```

```text
hello, rut!
```

The run works from ANY directory — nothing about it assumes a clone of
the toolchain's repo. The url row names the package on jsDelivr; the
CLI fetches it once into the project's cache (`.rut/cache`), every
later run is a pure cache hit, and `rut fetch hello` pre-warms the
cache without running anything.

Three things to notice:

- **One directory with a `rut.jsonc` is one program.** The manifest is
  the runnable unit — `rut run <dir>` (or a packed `.rutbundle`) is the
  only run lane. Inside the package, one file is one module; cross-module
  code is reached through `use` paths (see the
  [modules tutorial](../tutorial/modules.md)).
- **`use ink::{ Logger };`** pulls `Logger` out of the `ink` package —
  and the manifest's `deps` row is what mounts it: a `use` line says
  *which* names the program wants, the manifest says *where* the package
  lives ([project structure](../reference/project-structure.md)).
- **`f"hello, {name}!"`** is a format literal: `{expr}` interpolates any
  expression, rendered through the value's display contract.

`pub fn main()` is the entry the CLI calls.

## Values, briefly

Extend the file — rut infers, and every type is known at compile time:

```rut
use ink::{ Logger };

pub fn main() {
    let log = Logger.new("hello");
    let n = 10;            // i32 — the integer default
    let scale = 1.5;       // f32 — the float default
    let big: u64 = 10;     // u64 via annotation — fits the default
    let flags = [1, 2, 3]; // [i32] — a fixed heap cell
    let mut sum = 0;       // `mut` — this one is written to
    for (let x of flags) {
        sum += x * n;
    }
    log.info(f"sum={sum} scale={scale}");
}
```

```text
sum=60 scale=1.5
```

The full tour — suffixes, casts, strings, `bytes`, structs — is the
[values and variables](../tutorial/values-and-variables.md) tutorial,
and the exact rules live in
[literals and inference](../reference/literals-and-inference.md).

## Format

The formatter is the style — there is no configuration:

```sh
rut fmt hello/main.rut         # rewrite in place
rut fmt hello/main.rut --check # verify only (note: --check goes AFTER the path)
rut fmt .                      # walk a directory tree, every *.rut file
```

`--check` prints `unformatted: <file>` and exits `1` when a file needs
changes, which makes it a clean CI gate. A file that does not parse is
refused loudly rather than half-formatted.

## Inspect

`dump` compiles a single module and prints its structures — useful when
a snippet behaves differently than you expect:

```sh
rut dump hello/main.rut
```

`dump` runs the lone module through the frontend *without* mounting the
packages, so it is the tool for surface questions ("what does this lower
to?") rather than whole-program runs. The playground's **AST** and
**IR** panes show the same two views live — see
[the playground](playground.md).

## Budget it

Every `rut run` is uncapped by default — fuel is opt-in via `--fuel N`:

```sh
rut run hello --fuel 1000
```

A program that exhausts its fuel halts with a diagnostic naming the
budget — the same contract the VM enforces for embedded hosts, along
with heap ceilings and depth limits
([resource limits](../reference/resource-limits.md)).

## Where next

- The [tutorial](../tutorial/values-and-variables.md) teaches the
  language hands-on, one concept per chapter.
- The [examples](../examples/index.md) are seven complete projects you
  can run today.
- [Core concepts](../core-concepts/design-goals.md) explains *why* the
  language is shaped the way it is.
