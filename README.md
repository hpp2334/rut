# rut

**rut** is a small, statically typed, embeddable scripting language with
Rust-flavored syntax, designed as the scripting layer of host
applications. Types are reified — they exist at runtime — the syntax
reads like Rust, and the language runs on a straightforward bytecode VM
with no JIT: deterministic, auditable, and easy to embed.

A taste — the Sieve of Eratosthenes, verbatim from the playground's
prepared cases (`demo/src/examples/sieve.rut`):

```rut
use pouch::{ Vec };
// Sieve of Eratosthenes — flat Vec<u8>/Vec<i32> primitive buffers.

use ink::{ Logger };

fn sieve(limit: i32) -> Vec<i32> {
    let mut marks = Vec<u8>.filled(0, limit + 1);  // zeroed: 0 = candidate, 1 = crossed; `mut` — index writes
    let primes: Vec<i32> = Vec.new();       // empty growable: Vec.new(), not []
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
    let last = primes[primes.len() - 1];
    log.info(f"{primes.len()} primes up to 100, last={last}");
}
```

## Quick start

```sh
cargo build --release -p rut-cli
target/release/rut run hello.rut
```

The CLI ships four subcommands: `run` (execute a loose script or a
module directory), `fmt` (format), `pack` (bundle a module directory
into a `.rutbundle`), and `dump` (inspect the AST / IR / bytecode).

## Documentation

- **The book** — the user manual, source under
  [`docs/src/`](docs/src/README.md), live at <https://rut.hpp2334.com>
- **The playground** — write and run rut in the browser:
  <https://playground.rut.hpp2334.com>

## Repository layout

| path | what |
|---|---|
| [`crates/`](crates/) | the implementation — lexer, parser, compiler, VM, std, CLI, LSP, wasm |
| [`rut/`](rut/) | the in-tree packages (`core`, `pouch`, `nmapset`, `json`, `strbuild`, `http`, …) |
| [`examples/`](examples/) | seven example projects — six runnable end to end, plus a parse-only corpus |
| [`demo/`](demo/) | the wasm playground page |
| [`docs/`](docs/) | the user manual (this book — mdbook source in `docs/src/`) |

## License

Dual-licensed, at your option: MIT or Apache-2.0.
