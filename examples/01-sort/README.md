# 01-sort — a Rust app that embeds rut

The second **runnable** host example (after
[`00-todolist`](../00-todolist/)): five sorting algorithms in rut,
driven from Rust through one `entry fn` dispatcher.

The split:

| file | role |
|---|---|
| `sort.rut` | **the rut part** — insertion / bubble / selection (loop-shaped) + quicksort / merge sort (recursion, in-place vs. out-of-place), a deterministic `fill`, and the `entry fn` surface the host calls. No `main` — it's a library |
| `src/main.rs` | **the Rust part** — the embedder: compile → verify → drive the session through `vm.call` |
| `tests/session.rs` | asserts every algorithm, edge cases, cross-algorithm agreement, the JSON round trip, and trap cleanliness; runs under `cargo test --workspace` |

## Run it

```sh
cargo run -p sort                # from the repo root
```

Output (abridged):

```
pushed: [5, 2, 9, 2]
after insertion: [2, 2, 5, 9]
fill(16, seed=42), each algorithm:
  insertion true  fuel   2335  [208, 320, 353, ..., 854, 893]
  bubble    true  fuel   3895  [208, 320, 353, ..., 854, 893]
  selection true  fuel   3353  [208, 320, 353, ..., 854, 893]
  quick     true  fuel   2241  [208, 320, 353, ..., 854, 893]
  merge     true  fuel   4358  [208, 320, 353, ..., 854, 893]
sort(bogus) = Res(Err(Str("unknown algorithm: bogus")))
fuel used: 21142 of Some(5000000)
```

## The design

**One opaque bank; the data never leaves rut.** `Vec<i32>` cannot cross
the host boundary (RFC 0023 §2 — enforced on `entry fn` signatures at
compile time), so `create()` boxes a `Bank` in an `Opaque` (RFC 0014)
and the host holds the handle. Results come back three ways:

- `serialize(c) -> str` — a JSON array, `[1, 2, 3]`: one string
  crosses, so a whole result is one host-side compare
- `get(c, i) -> Option<i32>` — element-by-element, `Option.none()`
  past the end
- `is_sorted(c) -> bool` — the verdict

## What it demonstrates

- **recursion** — quicksort (in-place partitioning) and merge sort
  (out-of-place `push`-built halves merged back through the shared
  handle); the call-frame stack is the VM's, under fuel
- **`when` on strings** — `sort(c, algo)` dispatches on the algorithm
  name via string-literal pattern arms; unknown names are an ordinary
  `Result.err` value, not a trap
- **the mut-binding law** (RFC 0003 §1) — every algorithm takes
  `mut xs: Vec<i32>`: writing through a handle requires a `mut` head
  binding, while reading (or `push`) doesn't
- **wrapping escapes** (RFC 0004 §3) — `fill`'s LCG runs on `&*`/`&+`
  so the u32 math never traps (plain `*` — and `<<` — would)
- **budgets** (RFC 0040) — the session runs under fuel + heap limits,
  and `main.rs` prints fuel per algorithm: quick < insertion <
  selection < bubble, as it should be
- the executable M1 surface — dataclasses, `Opaque.new`/`downcast`,
  `Option`/`Result`, `for`/`while`, short-circuit `&&`, f-strings
