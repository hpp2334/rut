# rut

**rut** is a small, statically typed, embeddable scripting language with
Rust-flavored syntax, implemented in Rust. It is designed to be the
scripting layer of host applications — typically UI apps — replacing
JavaScript engines with a language whose type system is *not erased*:
types exist at runtime, drive bytecode specialization, and make the host
boundary fully checked.

Syntax feels like TypeScript (simple `enum`, the `class` shape, anonymous
functions), with a Rust-flavored trait surface (`trait` + nominal
satisfaction through `impl I for T` blocks) and Kotlin-style structured
concurrency (`async`/`await` over poll-based futures). The runtime is a
bytecode VM with **no JIT** — deterministic, fuel-metered, and small
enough to embed everywhere a wasm binary fits.

```rut
use pouch::{ Vec };

fn sieve(limit: i32) -> Vec<i32> {
    let mut marks = Vec<u8>.filled(0, limit + 1);
    let primes: Vec<i32> = Vec.new();
    for (let i = 2; i <= limit; i += 1) {
        if (marks[i] == 0) {
            primes.push(i);
            let mut m = i * i;
            while (m <= limit) {
                marks[m] = 1;
                m += i;
            }
        }
    }
    return primes;
}

pub fn main() {
    let log = Logger.new("sieve");
    let primes = sieve(100);
    log.info(f"{primes.len()} primes up to 100, last={primes[primes.len() - 1]}");
}
```

## Where to go next

| If you want to… | Read |
|---|---|
| Install the toolchain and run a program | [Quick Start](quick-start/installation.md) |
| Learn the language hands-on | the [Tutorial](tutorial/values-and-variables.md) |
| Understand *why* the language is shaped this way | [Core Concepts](core-concepts/design-goals.md) |
| Study complete programs | [Examples](examples/index.md) |
| Look up exact rules | the [Reference](reference/lexical-structure.md) |

The book is also served live at **<https://rut.hpp2334.com>**, and the
wasm playground — where every snippet on this site runs in the browser —
lives at **<https://playground.rut.hpp2334.com>**.

## The repo

```
crates/     the implementation: lexer, parser, compiler, VM, std, CLI, wasm
rut/        the in-tree rut packages (core, pouch, ink, json, http, …)
examples/   seven runnable end-to-end projects
demo/       the wasm playground
docs/       this book (built with mdbook)
```

Build everything from source:

```sh
cargo build --release -p rut-cli   # the `rut` binary: run / fmt / pack / dump
```

## License

MIT OR Apache-2.0.
