# Your first rut program

rut feels like TypeScript on the surface and Rust underneath the hood.
This page walks one file through the whole toolchain — running,
formatting, inspecting, and budgeting — so every later page can assume
the workflow.

## Write and run

```rut
use ink::{ Logger };

pub fn main() {
    let log = Logger.new("hello");
    let name = "rut";
    log.info(f"hello, {name}!");
}
```

Save it as `hello.rut` and run it:

```sh
rut run hello.rut
```

```text
hello, rut!
```

Three things to notice:

- **One file is one module.** There is no include form; cross-module
  code is reached through `use` paths (see the
  [modules tutorial](../tutorial/modules.md)).
- **`use ink::{ Logger };`** pulls `Logger` out of the `ink` package.
  When you `rut run` a standalone file, the vendored packages mount
  automatically — no manifest needed.
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
rut fmt hello.rut            # rewrite in place
rut fmt hello.rut --check    # verify only (note: --check goes AFTER the path)
rut fmt .                    # walk a directory tree, every *.rut file
```

`--check` prints `unformatted: <file>` and exits `1` when a file needs
changes, which makes it a clean CI gate. A file that does not parse is
refused loudly rather than half-formatted.

## Inspect

`dump` compiles a single module and prints its structures — useful when
a snippet behaves differently than you expect:

```sh
rut dump hello.rut
```

`dump` runs the lone module through the frontend *without* mounting the
packages, so it is the tool for surface questions ("what does this lower
to?") rather than whole-program runs. The playground's **AST** and
**IR** panes show the same two views live — see
[the playground](playground.md).

## Budget it

Every `rut run` is fuel-metered. Cap it with `--fuel N` (default
10,000,000):

```sh
rut run hello.rut --fuel 1000
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
