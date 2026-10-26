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
minimal embedder set: the run chain, binary decode, the parser's
`Mode`, and verify + the VM itself. Add `rut-native` the same way when
the program walks in from a directory — it is the filesystem/wire half
([embedding and native modules](../reference/embedding.md)).

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
    // 1. compile the module — the core prelude auto-rides
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
    let mut vm = Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hooks(HostHooks::default())
        .hosts(HostRegistry::new())
        .build()
        .map_err(|e| e.msg)?;

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

For temporary runs the repo builds a standalone binary — three ways to
get it on `PATH`, pick one.

### `cargo install` from git — no checkout

```sh
rustup toolchain install nightly-2026-07-15
cargo +nightly-2026-07-15 install --git https://github.com/hpp2334/rut.git --locked rut-cli
```

The pinned nightly is not optional — the engine builds on incomplete
features (see [Pin the nightly](#pin-the-nightly) above) and a stable
toolchain refuses to; the pin is `nightly-2026-07-15` (check this
repo's `rust-toolchain.toml` for the current value). `--locked` builds
against the repo's committed `Cargo.lock`. The binary lands at
`~/.cargo/bin/rut`, normally already on `PATH`.

### `cargo install` from a checkout

```sh
git clone https://github.com/hpp2334/rut.git
cd rut
cargo install --path crates/rut-cli --locked
```

Standing inside the clone, rustup resolves the repo's
`rust-toolchain.toml` pin for you — same binary, same destination.

### Plain build

```sh
git clone https://github.com/hpp2334/rut.git
cd rut
cargo build --release -p rut-cli
export PATH="$PWD/target/release:$PATH"
```

All three lanes end in the same `rut` binary — call it by path or put
it on your `PATH` (the pages here use the bare name). `rut --help`
prints the surface (and `rut <command> --help` each command's flags):

```sh
rut --help
```

```text
the rut toolchain's temporary-run lane: run, scaffold, install, format, pack, fetch, inspect

Usage: rut <COMMAND>

Commands:
  run      compile and run a package directory or a packed archive (`.rutbundle`)
  new      scaffold a hello-world module directory (the std set as deps)
  install  resolve the url deps and pin them in rut.lock
  fmt      the source formatter — canonical house layout, rewrite in place
  pack     pack a package directory into a deterministic `.rutbundle` archive
  fetch    pre-warm the bundle cache for offline runs
  dump     the AST + IR readbacks for one file
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

The binary embeds the whole engine: lexer, parser, compiler, typed
bytecode, and the VM. The toolchain's standard packages (`core`,
`pouch`, `ink`, `json`, …) ship as source zips on jsDelivr —
source compiles with YOUR toolchain, so a pinned zip never goes stale
the way compiled IR can. A program says `use ink::{ Logger };` and its
manifest's `deps` row
mounts the package from the CDN, pinned by the project's `rut.lock`
([dependency kinds](../reference/dependency-kinds.md)), since every rut
program is a module directory ([your first rut
program](first-program.md)).

Try it:

```sh
mkdir -p hello/src
cat > hello/rut.jsonc <<'EOF'
// hello/rut.jsonc — the manifest IS the program's door
{
  "name": "hello",
  "deps": {
    // ink — the toolchain's logger package, served by jsDelivr at the
    // std-v15 tag; the pin lives in rut.lock, one install below
    "ink": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v15/dist/std/ink.zip" }
  }
}
EOF
cat > hello/src/mod.rut <<'EOF'
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
install: ink — downloading https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v15/dist/std/ink.zip
install: ink — downloaded (2883 bytes)
install: 1 url deps resolved — 1 downloaded, 0 cache hits (2883 bytes total)
rut.lock written: 1 rows (1 added, 0 changed, 0 dropped)
hello, rut!
```

`rut install` fetched `ink` from jsDelivr into `hello/.rut/cache` and
pinned its sha256 in `hello/rut.lock` — the lock every later load
verifies against, committed alongside the manifest (same manifest +
same `rut.lock` ⇒ identical resolved world). `rut install --locked`
re-verifies the pair without writing (the CI lane); `rut fetch hello`
re-warms the cache without running anything.

The hand-typed walkthrough is the lesson — the manifest is what you
are learning. When it is routine, `rut new hello` is the one-step
equivalent: it scaffolds the directory in three files (`rut.jsonc`
plus the matching `rut.lock` plus the hello `src/mod.rut`), with the
whole swappable std set as url deps and the std pins already in the lock
([the rut CLI](../reference/cli.md)).

If that printed, the CLI is ready. Continue to
[your first rut program](first-program.md) — and keep the lane's role
in mind: the CLI is for **temporary runs** (`run`, `fmt`,
`dump`, `pack`), not a shipping path. Real projects depend on the
crates and embed the engine; [the rut CLI](../reference/cli.md) is the
binary's reference.

## Optional: the wasm playground

The [playground](playground.md) runs the same engine compiled to
WebAssembly. Building it needs [wasm-pack](https://rustwasm.github.io/wasm-pack/)
and one extra target:

```sh
rustup target add wasm32-unknown-unknown
wasm-pack build crates/rut-wasm --target web --release
wasm-pack build crates/rut-lsp-wasm --target web --release
```

or, from `demo/`, the one-command lane that also copies the artifacts
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
