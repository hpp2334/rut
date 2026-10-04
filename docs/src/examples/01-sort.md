# 01 — Sort

The second runnable host example, in the shape of
[00 — Todolist](00-todolist.md): five sorting algorithms in rut —
insertion, bubble, and selection (loop-shaped), plus quicksort and
merge sort (recursion, in-place vs. out-of-place) — driven from Rust
through **one** `entry fn` dispatcher. Where 00 is about the boundary,
this one is about the language: recursion, `when` on strings, the
mut-binding law, wrapping arithmetic, and visible fuel budgets.

## Run it

```sh
cargo run -p sort          # from the repo root
cargo test -p sort         # every algorithm, edge cases, cross-agreement
```

The run sorts a hand-picked input, then fills the bank with a
deterministic pseudo-random vector and runs the full sweep, printing
fuel per algorithm:

```text
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

The fuel column is the demo's quiet joke: quick < insertion <
selection < bubble, exactly as the textbooks promise — and the VM
counts it for you.

## Code tour

### One opaque bank; the data never leaves rut

`Vec<i32>` cannot cross the host boundary, so `create()` boxes a
`Bank` in an `opaque` and the host holds the handle — the
[00 — Todolist](00-todolist.md) pattern again. Results come back
three ways, all crossing-shaped. `serialize` below is verbatim, plus
a `main` that boxes a small bank the way `create()` does, so the
block runs on its own:

```rut
use ink::{ Logger };
use pouch::{ Vec };

struct Bank {
    data: ?Vec<i32>;
}

entry fn serialize(c: opaque) -> str {
    let b = opaque.downcast<?Bank>(c);
    let xs = b.data;
    let mut out = "[";
    for (let i = 0; i < xs.len(); i += 1) {
        if (i > 0) {
            out = f"{out}, ";
        }
        out = f"{out}{xs[i]}";
    }
    return f"{out}]";
}

entry fn main() {
    let log = Logger("sort");
    let bank: ?Bank = Bank { data: Vec<i32>.from([5, 2, 9, 2]) };
    let c = opaque(bank);
    log.info(f"serialized: {serialize(c)}");
}
```

```text
serialized: [5, 2, 9, 2]
```

`serialize` is the workhorse: a whole result crosses as **one JSON
array string**, so the host asserts an entire sort in one compare.
`get(c, i)` / `has(c, i)` read element-by-element, and
`is_sorted(c)` returns the verdict.

### The dispatcher: `when` on strings, errors as values

Every algorithm sits behind a single string-keyed entry. Unknown names
are an ordinary value, not a trap — the empty string means success:

```rut
entry fn sort(c: opaque, algo: str) -> str {
    let b = opaque.downcast<?Bank>(c);
    let mut unknown = false;
    when (algo) {
        "insertion" -> { insertion_sort(b.data); },
        "bubble"    -> { bubble_sort(b.data); },
        "selection" -> { selection_sort(b.data); },
        "quick"     -> { quicksort(b.data, 0, b.data.len() - 1); },
        "merge"     -> { merge_sort(b.data); },
        else        -> { unknown = true; },
    }
    if (unknown) {
        return f"unknown algorithm: {algo}";
    }
    return "";
}
```

`when` with string-literal arms is the general dispatch idiom — see
[Control flow and when](../reference/control-flow.md).

### Deterministic input in one call

`fill` replaces the bank with `n` pseudo-random values from a
linear-congruential generator, so sizing a benchmark input is one
host round-trip, not one per element. The u32 math must **wrap**, and
rut makes that explicit — plain `*` traps on overflow. The entry is
verbatim (the `Bank` inlined so the block compiles alone), with a
`main` that fills and prints what one call produces:

```rut
use ink::{ Logger };
use pouch::{ Vec };

struct Bank { data: ?Vec<i32>; }   // inlined from above — the block runs alone

entry fn fill(c: opaque, n: u32, seed: u32) {
    let mut b = opaque.downcast<?Bank>(c);
    let fresh: Vec<i32> = Vec.new();
    let mut x = seed | 1;         // the LCG needs an odd state
    for (let i = 0; i < n as i32; i += 1) {
        x = (x.wrapping_mul(1664525)).wrapping_add(1013904223);
        fresh.push((x % 1000) as i32);
    }
    b.data = fresh;
}

entry fn main() {
    let log = Logger("sort");
    let bank: ?Bank = Bank { data: nil };
    let c = opaque(bank);
    fill(c, 8, 42);
    let b = opaque.downcast<?Bank>(c);
    let xs = b.data;
    let mut out = "";
    for (let x of xs) {
        out = f"{out} {x}";
    }
    log.info(f"fill(8, seed=42):{out}");
}
```

```text
fill(8, seed=42): 798 893 208 375 746 353 452 555
```

The wrapping family (`wrapping_mul`, `wrapping_add`, `wrapping_shl`)
is the honest spelling of two's-complement arithmetic — the
[playground corpus](playground-corpus.md) has a whole `checked-arith`
case on the contrast.

### Recursion under fuel

Quicksort partitions in place; merge sort builds out-of-place halves
and merges back through the shared handle. Both are plain recursive
fns — the call-frame stack is the VM's, counted by the same fuel
budget ([the bytecode VM](../core-concepts/the-vm.md)):

```rut
fn quicksort(mut xs: ?Vec<i32>, lo: i32, hi: i32) {
    if (lo >= hi) {
        return;
    }
    let p = partition(xs, lo, hi);
    quicksort(xs, lo, p - 1);
    quicksort(xs, p + 1, hi);
}
```

Note the `mut xs` parameter heads: vecs are handles, so an in-place
swap inside a call is shared with the bank — but **writing** through
the handle requires the `mut` binding head, while mere reads (and
`push`) do not. That asymmetry is the mut-binding law, and every
algorithm in this file declares it
([Modules and visibility](../reference/modules-and-visibility.md)).

### The embedder's sweep

`src/main.rs` drives the sweep and prints the fuel deltas, which is
how the table above gets its numbers:

```rust
println!("fill(16, seed=42), each algorithm:");
for algo in ["insertion", "bubble", "selection", "quick", "merge"] {
    vm.call::<_, ()>("fill", (c.clone(), 16u32, 42u32)).unwrap();
    let before = vm.fuel_used;
    vm.call::<_, String>("sort", (c.clone(), algo)).unwrap();
    let sorted: bool = vm.call("is_sorted", (c.clone(),)).unwrap();
    println!("  {algo:<9} {sorted}  fuel {:>6}  {}", vm.fuel_used - before, ser(&mut vm));
}
```

## Takeaways

- Same host pattern as [00 — Todolist](00-todolist.md), one entry
  richer: a single string-keyed dispatcher keeps the `entry fn`
  surface small.
- `Vec<T>` never crosses; a serialized JSON array makes a whole
  result one host-side compare.
- Errors are values: an unknown algorithm comes back as a string, not
  a trap.
- `mut` on a binding is a capability law, not a suggestion — and the
  wrapping arithmetic family is explicit about overflow.
- Fuel budgets make algorithmic differences visible from the host,
  for free.

Deep-dives: [the bytecode VM](../core-concepts/the-vm.md) explains
what a unit of fuel counts; [02 — Digest](02-digest.md) takes the
same session shape to the byte level.
