# Installation

rut is a self-hosting-free, from-source toolchain: one Rust build gives
you the `rut` binary (run / format / pack / dump) plus the language's
in-tree packages, which the engine mounts automatically when you run a
standalone file.

## Prerequisites

- **Rust (nightly)** — the repo pins an exact nightly in
  `rust-toolchain.toml`; `rustup` installs it on the first build. Any
  recent `rustup` works.
- **git** — for the checkout.

- **Node.js 20+** — only if you want to build or drive the wasm
  playground (the rest of the book does not need it).

## Build the toolchain

From a checkout of the repository:

```sh
cargo build --release -p rut-cli
```

This produces the `rut` binary at `target/release/rut`. Put it on your
`PATH` (or call it by path — the pages here use the bare name):

```sh
cargo build --release -p rut-cli
export PATH="$PWD/target/release:$PATH"
rut
# usage: rut — run <file.rut | dir | mod.rutbundle> [--fuel N] | fmt <file.rut | dir> [--check] | pack <dir> [-o out.rutbundle] | dump <file.rut>
```

The binary embeds the whole engine: lexer, parser, compiler, typed
bytecode, and the VM. It also carries the vendored packages from the
repo's `rut/` directory (`core`, `pouch`, `ink`, `json`, …) — a
standalone `hello.rut` that says `use ink::{ Logger };` just works,
no manifest required.

## Smoke-test it

```sh
cat > hello.rut <<'EOF'
use ink::{ Logger };

pub fn main() {
    let log = Logger.new("hello");
    log.info(f"hello, rut!");
}
EOF
rut run hello.rut
```

```text
hello, rut!
```

If that printed, your toolchain is complete. Continue to
[your first rut program](first-program.md).

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
