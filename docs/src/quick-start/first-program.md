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

entry fn main() {
    let log = Logger("hello");
    let name = "rut";
    log.info(f"hello, {name}!");
}
```

Every rut program is a **module directory**: a `rut.jsonc` naming the
package and its dependencies, plus the root module — `mod.rut`, the
file the loader mounts. Create it, install the deps, run the directory:

```sh
mkdir hello
cat > hello/rut.jsonc <<'EOF'
// hello/rut.jsonc — the manifest IS the program's door
{
  "name": "hello",
  "deps": {
    // ink — the toolchain's logger package, served by jsDelivr at the
    // std-v10 tag; the pin lives in rut.lock, one install below
    "ink": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/ink.rutbundle" }
  }
}
EOF
cat > hello/mod.rut <<'EOF'
use ink::{ Logger };

entry fn main() {
    let log = Logger("hello");
    log.info(f"hello, rut!");
}
EOF
rut install hello   # fetch ink, pin it, write hello/rut.lock
rut run hello
```

```text
install: ink — downloading https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/ink.rutbundle
install: ink — downloaded (4193 bytes)
install: 1 url deps resolved — 1 downloaded, 0 cache hits (4193 bytes total)
rut.lock written: 1 rows (1 added, 0 changed, 0 dropped)
hello, rut!
```

The run works from ANY directory — nothing about it assumes a clone of
the toolchain's repo. The url row names the package on jsDelivr;
`rut install` fetched it once into the project's cache (`.rut/cache`)
and pinned its sha256 in `hello/rut.lock` — the lock rides beside the
manifest, committed, and every load verifies the fetched bytes against
it (an unpinned url refuses at the door: `run `rut install``). Every
later run is a pure cache hit, `rut install hello --locked` re-verifies
the lock without writing (the CI lane), and `rut fetch hello` re-warms
the cache without running anything.

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
- **`Logger("hello")`** constructs the logger through the class's
  `[constructor]` member — the call form spells the construction
  ([classes](../reference/classes.md)).
- **`f"hello, {name}!"`** is a format literal: `{expr}` interpolates any
  expression, rendered through the value's display contract.

`entry fn main()` is the entry the CLI calls.

Typing the directory by hand is the lesson — the manifest IS the
program's door, and now you have written one. For the next directory,
the CLI scaffolds it in one step: `rut new <dir>` writes exactly those
three files — the manifest with the derived name (and, unlike the
walkthrough above, the whole swappable std set as url deps), the
matching `rut.lock` (the std set's pins), and the same `mod.rut` — and
`rut run <dir>` runs it ([the rut CLI](../reference/cli.md)):

```sh
rut new hello    # hello/rut.jsonc (the std set) + hello/rut.lock + hello/mod.rut
rut run hello
```

## Values, briefly

Extend the file — rut infers, and every type is known at compile time:

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("hello");
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
rut fmt hello/mod.rut         # rewrite in place
rut fmt hello/mod.rut --check # verify only (note: --check goes AFTER the path)
rut fmt .                     # walk a directory tree, every *.rut file
```

`--check` prints `unformatted: <file>` and exits `1` when a file needs
changes, which makes it a clean CI gate. A file that does not parse is
refused loudly rather than half-formatted.

## Inspect

`dump` compiles a single module and prints its structures — useful when
a snippet behaves differently than you expect:

```sh
rut dump hello/mod.rut
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
