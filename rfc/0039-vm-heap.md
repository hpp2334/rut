# RFC 0039: The Self-Managed VM Heap

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0016 (the RC heap), RFC 0017 (heap stats),
  RFC 0034 (VM core), RFC 0022 (embedding)
- **Part:** F — Toolchain & artifacts

## Summary

rut's host code uses std freely — `Vec`, `String`, `Rc` where handy.
The **VM's heap is the exception**: every byte that backs a rut value
comes from the VM's **own heap manager** — accounted, budgeted, pooled,
and freed wholesale at `Vm` drop. No rut allocation is an untracked
`Box`/`Vec` somewhere in Rust land.

Why not just let values be ordinary Rust objects:

1. **Caps** — embedders (a wasm page, a UI process, a plugin host)
   must be able to say "this script gets N bytes" and get
   `Trap::OutOfMemory`, not an OS-level abort (RFC 0040 §1).
2. **Answers** — `heap_usage()` / `heap_stats()` (RFC 0017 §3) can only
   be exact if every allocation is accounted.
3. **Teardown** — `Vm::drop` frees the heap in slab units, after
   `Disposal` runs (RFC 0016 §3); nothing per-object leaks past the VM.
4. **Leak reporting** — the shutdown leak report walks *our* headers,
   which requires our headers everywhere.

## 1. What the VM manages — and what it doesn't

| Managed by the VM heap | The host's business |
|---|---|
| every cell (dataclass, class, enum, `Opaque` — RFC 0016 §5) | module binaries and maps (RFC 0033/0038) |
| buffer blocks (`Vec`/`[T]` data, string `StrBuf`s) | the type table (shared, RFC 0015) |
| coroutine frames & register blocks (RFC 0018 §4) | native-module state (RFC 0022) |
| the ready ring, frame pool, weak boxes (RFC 0017) | host-side handles, caches, futures (RFC 0020) |

Module binaries may be loaded by the host (std loader, files or bundles);
the interpreter reads them zero-copy and never copies them into the
managed heap.

## 2. The heap manager surface

```rust
impl Heap {
    /// The one allocation path for rut values.
    /// Returns None when Limits say no (RFC 0040 §1) — never aborts.
    fn alloc(&mut self, size: u32, align: u8, ty: TypeId)
        -> Option<NonNull<Header>>;

    fn usage_bytes(&self) -> u64;        // feeds vm.heap_usage()
    fn stats(&self) -> PerTypeCounts;    // feeds heap_stats() (RFC 0017 §3)
}
```

`alloc` is the *only* way rut values come into existence — cell mint
(`newcell`), buffer growth, string concat, frame-pool growth all
route through it (check points listed in RFC 0040 §1). Backing store is
ordinary Rust allocation (std) — the manager owns slabs it carves; the
discipline is the point, not the allocator.

## 3. Allocation strategy (v1, shipped)

- **Fixed-size cell slots in the arena**: cells are one fixed-size record
  carved from 1024-slot chunks with a free list (the `Arena`); a dead
  slot is reused by the next mint. Cell reuse is invisible except in
  `heap_usage`.
- **The block store (shipped)**: every *variable-size* cell payload —
  string octets, array element runs — lives in a **block** carved from
  size-classed pages the VM owns, not in a Rust `Vec`. Blocks are 1:1
  with their cell (the cell is the RC unit), need no refcount of their
  own, and never move (RFC 0016 OQ-1), so `&[u8]` into the store stays
  valid for the cell's life. Small blocks reuse through per-class
  freelists (plus a single-slot LIFO cache for the churn pattern);
  in-place geometric growth within a class is what keeps
  append-accumulation loops linear. The block header carries the
  class-rounded capacity, so `free` re-derives the class with no side
  table. Large blocks (> 2 KiB) get dedicated allocations, freed
  wholesale. The release path frees a cell's blocks explicitly (it is
  the only code holding the `&Arena` the store needs) — payloads carry
  no `Drop` glue.
- **Immortal singletons** (interned literals, dataless enum variants —
  RFC 0016 §1/§4) live in a slab freed only at `Vm` drop; they carry the
  rc==0 sentinel.
- **No compaction in v1** (RFC 0016 OQ-1): non-moving keeps host `&mut`
  borrows sound (RFC 0023 §2) and freelists trivial.
- **Cell addressing: the slot holds the stable raw pointer** (`Slot.r`,
  RFC 0015 §5). A 32-bit handle design — `(chunk << 10) | index`,
  resolved through the minting arena — was prototyped against this
  milestone and **reverted on its gate**: chunk bases never move, but
  every ref-slot read pays a decode chain where the pointer design pays
  none, and ref-heavy workloads measured +8-15% (quicksort, fasta)
  against flat pure-arithmetic loops (intloop, u64loop). The handle's
  original consumer — 4-byte slots for the narrowing plan — was
  cancelled, leaving cost with no buyer; integer identity compare and
  stale-handle non-aliasing remain available to a future revision that
  actually needs small slots.

## 4. Deterministic teardown

`Vm::drop` runs remaining `Disposal` impls? No — teardown mirrors
destruction rules: the host drops references, releases run inline
(RFC 0016 §3), and `Vm::drop` frees survivors wholesale (cycles included
— RFC 0017), *after* emitting the leak report. The heap never outlives
the `Vm`, and no rut cell can outlive the heap.

## 5. The contributor rule

**Any allocation path that materializes a rut value must flow through
`Heap::alloc`.** An untracked `Box`/`Vec` holding a value is a bug: the
budget wouldn't bind, `heap_usage` would lie, and the leak report would
miss it. Host-side Rust state (caches, futures, channel buffers at the
FFI) is exempt — it is the host's memory, charged to the host.

## Open questions

- OQ-1: slab sizing & growth policy (fixed vs. doubling) under the
  wasm memory model — tune with the demo page (RFC 0041 §3).
- OQ-2: compaction (ties RFC 0016 OQ-1) — freelist fragmentation vs.
  wholesale teardown simplicity; measure on the GUI corpus.
- OQ-3: should worker transfer buffers (RFC 0021 §4) be charged to the
  child heap at allocation time *and* uncharged at transfer? Proposed:
  yes — transfers move accounting, not just pointers.
