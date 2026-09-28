# The VM heap

Host Rust code may use `std` freely. The VM's heap is the exception:
**every byte that backs a rut value comes from the VM's own heap
manager** — accounted, budgeted, pooled, and freed wholesale at `Vm`
drop. No rut allocation is an untracked `Box`/`Vec` somewhere in Rust
land. The point of the discipline:

1. **Caps** — an embedder says "this script gets N bytes" and gets a
   resumable `OutOfMemory` trap, not an OS abort
   ([resource limits](resource-limits.md)).
2. **Answers** — `vm.heap_usage()` is exact only if every allocation is
   accounted.
3. **Teardown** — `Vm::drop` frees the heap in slab units; nothing
   per-object leaks past the VM.
4. **Leak reporting** — the shutdown leak report
   ([weak references](weak-refs.md)) walks the heap's own headers,
   which requires that every value have one.

## What the VM manages

| Managed by the VM heap | The host's business |
|---|---|
| every cell — structs, classes, enums, `opaque` boxes, weak boxes ([the Rc heap](rc-heap.md)) | module binaries and maps (read zero-copy, never copied into the heap) |
| variable-size payload blocks — `Vec`/`[T]` data, string buffers | the shared type table |
| coroutine frames and register blocks ([async and await](async.md)) | native-module state |
| the ready ring, the frame pool, the timer wheel | host-side handles, caches, futures, channel buffers at the boundary |

Host-side Rust state is the host's memory, charged to the host.

## The manager surface

```rust
impl Heap {
    /// The one allocation path for rut values.
    /// Over budget -> Err(Trap::OutOfMemory); never aborts.
    fn alloc(&mut self, size: u32, align: u8, ty: TypeId)
        -> Result<NonNull<Header>, Trap>;

    fn usage_bytes(&self) -> u64;     // feeds vm.heap_usage()
    fn peak_bytes(&self) -> u64;      // high-water mark: vm.heap_peak()
    fn set_limit(&self, limit: Option<u64>);  // live budget change
}
```

Every route that materializes a rut value goes through it: cell mint,
`Vec` growth, string concatenation, frame-pool growth, `opaque` store
inserts. **The contributor rule**: an allocation path that materializes
a rut value and does not flow through `alloc` is a bug — the budget
would not bind, `heap_usage` would lie, and the leak report would miss
it.

Accounting checks run **before any write**: a failed allocation leaves
the heap byte-identical to its state before the op, and the trap is
resumable ([resource limits](resource-limits.md)).

## Allocation strategy (v1)

- **Fixed-size cell slots in the arena.** Cells are one fixed-size
  record carved from 1024-slot chunks with a free list; a dead slot is
  reused by the next mint. Reuse is invisible except in `heap_usage`.
- **The block store.** Every *variable-size* payload — string octets,
  array element runs — lives in a block carved from size-classed pages
  the VM owns:
  - Size classes 16 B … 2 KiB (multiples of 8); small blocks come from
    32 KiB pages via per-class free lists plus a single-slot LIFO cache
    for the churn pattern (a temp dies, the same size is minted again).
  - In-place growth within a class keeps append-accumulation loops
    linear; the block header carries the class-rounded capacity, so
    free re-derives the class with no side table.
  - Large blocks (> 2 KiB) get dedicated allocations, freed wholesale.
  - Blocks are 1:1 with their cell (the cell is the counted unit) and
    **never move**, so a raw `&[u8]` into the store stays valid for the
    cell's life.
- **Immortal singletons.** Interned string literals and dataless enum
  variants live in a slab freed only at `Vm` drop; they carry the
  `rc == 0` sentinel.
- **The `opaque` store.** Every rut `opaque` value lives in a slab of
  two-kind entries (rut value / host payload) with its own borrow
  guards; entries charge their bytes on insert and refund on death, so
  the budget and the receipts stay honest.
- **No compaction in v1.** Non-moving keeps host borrows into `Vec`
  buffers sound and freelists trivial.
- **Cells are addressed by raw pointer** in the slot word. A 32-bit
  handle design was prototyped and reverted on measurement: ref-heavy
  workloads paid +8–15% for the decode chain per slot read.

## Deterministic teardown

Teardown mirrors the destruction rules: the host drops its references,
releases run inline ([the Rc heap](rc-heap.md)), and `Vm::drop` frees
the survivors wholesale — leaked cycles included — after emitting the
leak report. The heap never outlives the `Vm`, and no rut cell can
outlive the heap.

```rust
let mut vm = Vm::new(prog, limits, hooks, hosts)?;
vm.call::<_, ()>("main", ())?;
println!("used {} bytes (peak {})", vm.heap_usage(), vm.heap_peak());
// dropping `vm` frees every remaining slab
```

Worker heaps are per-VM and charged to the child's own budget
([workers and channels](workers-and-channels.md)).
