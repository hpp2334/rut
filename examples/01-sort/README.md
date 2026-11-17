# 01-sort — a Rust app that embeds rut

The second **runnable** host example (after
[`00-todolist`](../00-todolist/)): five sorting algorithms in rut,
driven from Rust through one `entry fn` dispatcher.

The split:

| file | role |
|---|---|
| `mod.rut` | **the rut part** — insertion / bubble / selection (loop-shaped) + quicksort / merge sort (recursion, in-place vs. out-of-place), a deterministic `fill`, and the `entry fn` surface the host calls. No `main` — it's a library |
| `src/main.rs` | **the Rust part** — the embedder: compile → verify → drive the run through `vm.call` |
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
  insertion true  fuel   2043  [208, 320, 353, ..., 854, 893]
  bubble    true  fuel   3155  [208, 320, 353, ..., 854, 893]
  selection true  fuel   2830  [208, 320, 353, ..., 854, 893]
  quick     true  fuel   1803  [208, 320, 353, ..., 854, 893]
  merge     true  fuel   5595  [208, 320, 353, ..., 854, 893]
sort(bogus) = "unknown algorithm: bogus"
fuel used: 21367 of Some(5000000)
```

## The design

**One opaque bank; the data never leaves rut.** `Vec<i32>` cannot cross
the host boundary (the [value boundary](../../docs/src/reference/value-boundary.md) —
enforced on `entry fn` signatures at
compile time), so `create()` boxes a `Bank` in an
`opaque` ([opaque](../../docs/src/reference/opaque.md))
and the host holds the handle. Results come back three ways:

- `serialize(c) -> str` — a JSON array, `[1, 2, 3]`: one string
  crosses, so a whole result is one host-side compare
- `get(c, i) -> i32` — element-by-element, `0` past the end
  (`has(c, i) -> bool` is the membership probe)
- `is_sorted(c) -> bool` — the verdict

## What it demonstrates

- **recursion** — quicksort (in-place partitioning) and merge sort
  (out-of-place `push`-built halves merged back through the shared
  handle); the call-frame stack is the VM's, under fuel
- **`when` on strings** — `sort(c, algo)` dispatches on the algorithm
  name via string-literal pattern arms; unknown names come back as an
  ordinary `str` answer (the `"unknown algorithm: …"` message), not a trap
- **mutable by default; `const` is the opt-out** ([modules and visibility](../../docs/src/reference/modules-and-visibility.md)) —
  every algorithm takes
  `xs: ?Vec<i32>` and writes through the shared handle: there is no
  `mut` head to earn, a binding is reassignable and writable-through
  unless spelled `const` (and that opt-out is shallow — reassignment
  only, writes through the binding stay legal)
- **wrapping escapes** ([primitive types](../../docs/src/reference/primitive-types.md)) —
  `fill`'s LCG runs on `&*`/`&+`
  so the u32 math never traps (plain `*` — and `<<` — would)
- **budgets** ([resource limits](../../docs/src/reference/resource-limits.md)) — the run executes under fuel + heap limits,
  and `main.rs` prints fuel per algorithm: quick < insertion <
  selection < bubble, as it should be
- the executable surface — structs, `opaque(v)`/`opaque::downcast<T>`,
  `?T` optionals, `for`/`while`, short-circuit `&&`, f-strings, the
  wrapping methods (`wrapping_mul`/`wrapping_add`)
