# Installation

rut reaches your project in two ways, ranked by intent:

1. **Depend on the engine crates** from your own Rust project through a
   Cargo git dependency. This is the intended path — your `Cargo.toml`
   names this repository and a commit, and the engine (lexer, parser,
   compiler, typed bytecode, VM) builds as part of *your* build, ready
   to [embed](../reference/embedding.md).
2. **Build the `rut` CLI** from a checkout of this repository — the
   **temporary-run** lane: running module directories and bundles,
   `fmt`, `dump`, `pack`. The fastest way to poke at the language; not
   the way anyone ships.

Both lanes build the same engine, and the crates are not published to
crates.io — the git dependency is the only consumption path.

## Use rut in your project

### Prerequisites

- **Rust (nightly)** — the repo pins an exact nightly in
  `rust-toolchain.toml` (because `rut-vm-threaded` uses incomplete
  features); `rustup` installs it on the first build. Any recent
  `rustup` works.
- **git** — to fetch the dependency.

- **Node.js 20+** — only if you want to build or drive the wasm
  playground (the rest of the book does not need it).

No checkout of this repository is needed — your `Cargo.toml` does
everything.

### Depend on the crates

Name the engine crates as git dependencies:

```toml
[dependencies]
rut-driver = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-core   = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-parser = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-vm     = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
```

Grab the current commit and drop it into every `rev`:

```sh
git ls-remote https://github.com/hpp2334/rut.git HEAD
```

Pin **all** the rut crates to the **same** rev — cargo then resolves
them to a single checkout, one consistent engine. These four are the
minimal embedder set: the driver pipeline, binary decode, the parser's
`Mode`, and verify + the VM itself.

### Pin the nightly

`rut-vm` depends on `rut-vm-threaded`, which uses the incomplete
`#![feature(explicit_tail_calls)]` and
`#![feature(rust_preserve_none_cc)]` — a stable toolchain refuses to
build it. Give your project the same pin the repo uses (check this
repo's `rust-toolchain.toml` for the current value):

```toml
# your project's rust-toolchain.toml
[toolchain]
channel = "nightly-2026-07-15"
```

### Smoke-test the embed

One rut module — compiled, verified, booted, called from Rust — the
[embedding loop](../reference/embedding.md) at minimum size:

```rust
use std::rc::Rc;

use rut_parser::Mode;
use rut_vm::interp::{HostHooks, HostRegistry, Limits, Vm};

const SRC: &str = "entry fn answer() -> i32 { return 6 * 7; }";

fn main() -> Result<(), String> {
    // 1. compile the module (a fresh session mounts core + calc)
    let out = rut_driver::compile_module(SRC, Mode::Impl, "app");
    if !out.diags.is_empty() {
        for d in &out.diags {
            eprintln!("error: {}", d.msg);
        }
        std::process::exit(1);
    }

    // 2. decode and verify the binary
    let prog = rut_core::binary::decode(&out.binary.unwrap())?;
    rut_vm::verify::verify(&prog)?;

    // 3. boot the VM — `Limits::default()` is uncapped
    let limits = Limits::default();
    let mut vm = Vm::new(
        Rc::new(prog),
        &limits,
        HostHooks::default(),
        HostRegistry::new(),
    )
    .map_err(|t| t.msg)?;

    // 4. call an export, get a Rust value back
    let answer: i32 = vm.call::<_, i32>("answer", ()).map_err(|t| t.msg)?;
    println!("answer = {answer}"); // 42
    Ok(())
}
```

`entry fn` is the host-callable surface — what `vm.call` can name.
`cargo run` prints `answer = 42`: a rut value crossed into Rust
unchanged, no std bindings involved.

When the program uses the std packages (`ink`, `pouch`, `json`, …), add
`rut-std` as a dependency the same way and bind its `pkg()` builders —
the [embedding and native modules](../reference/embedding.md) page is
the full contract.

## The rut CLI — temporary runs

For temporary runs the repo builds a standalone binary. Clone
and build it:

```sh
git clone https://github.com/hpp2334/rut.git
cd rut
cargo build --release -p rut-cli
```

This produces the `rut` binary at `target/release/rut`. Put it on your
`PATH` (or call it by path — the pages here use the bare name):

```sh
cargo build --release -p rut-cli
export PATH="$PWD/target/release:$PATH"
rut
# usage: rut — run <dir | mod.rutbundle> [--fuel N] [--symbols <file.rutsym>] | fmt <file.rut | dir> [--check] | pack <dir> [-o out.rutbundle] [--strip] | fetch <dir> | dump <file.rut>
```

The binary embeds the whole engine: lexer, parser, compiler, typed
bytecode, and the VM. The toolchain's standard packages (`core`,
`pouch`, `ink`, `json`, …) ship as compiled `.rutbundle`s on jsDelivr —
a program says `use ink::{ Logger };` and its manifest's `deps` row
mounts the package from the CDN, pinned by sha256
([dependency kinds](../reference/dependency-kinds.md)), since every rut
program is a module directory ([your first rut
program](first-program.md)).

Try it:

```sh
mkdir hello
cat > hello/rut.jsonc <<'EOF'
// hello/rut.jsonc — the manifest IS the program's door
{
  "name": "hello",
  "entry": { "lib": "./main.rut" },
  "deps": {
    // ink — the toolchain's logger package, served by jsDelivr at the
    // std-v5 tag, pinned by sha256
    "ink": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v5/dist/std/ink.rutbundle", "sha256": "200ab1ea41bee7145d20c0665f799302436dcbe82bdab8858c7c74708d1b306a" }
  }
}
EOF
cat > hello/main.rut <<'EOF'
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("hello");
    log.info(f"hello, rut!");
}
EOF
rut run hello
```

```text
hello, rut!
```

The first run fetched `ink` from jsDelivr into `hello/.rut/cache`;
`rut fetch hello` pre-warms that cache without running anything.

If that printed, the CLI is ready. Continue to
[your first rut program](first-program.md) — and keep the lane's role
in mind: the CLI is for **temporary runs** (`run`, `fmt`,
`dump`, `pack`), not a shipping path. Real projects depend on the
crates and embed the engine; [the rut CLI](../reference/cli.md) is the
binary's reference.

## Optional: the wasm playground

The [playground](playground.md) runs the same engine compiled to
WebAssembly. Building it needs one extra target:

```sh
rustup target add wasm32-unknown-unknown
cargo build -p rut-wasm --target wasm32-unknown-unknown --release
```

or, from `demo/`, the one-command lane that also copies the artifact
into place:

```sh
cd demo && npm install && npm run build:wasm
```

## Optional: build this book

The documentation is an [mdbook](https://rust-lang.github.io/mdBook/)
site:

```sh
cargo install mdbook --locked
mdbook serve docs     # live-reload preview at http://localhost:3000
mdbook build docs     # static site in docs/book/
```
