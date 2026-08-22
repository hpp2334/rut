# RFC 0017: Weak References & the Cycle Collector

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0016 (the RC heap)
- **Supersedes:** RFC 0004 §4–5, §7 + RFC 5004 §3 (pre-restructure)
- **Part:** C — Memory

## Summary

Reference cycles are the one thing RC cannot free (RFC 0016). rut's answer
is a budgeted **cycle collector** (Bacon–Rajan style trial deletion,
stop-the-VM) plus `Weak<T>` references for caches and observers.

## 1. Weak references

See **`examples/memory/weak-cache.rut`** — `Weak(v): Weak<C>` and
`upgrade(): Option<Rc<C>>`.

- `Weak(v)` allocates a `WeakBox` side object holding a back-pointer that is
  nulled when the referent dies (strong count hits 0). The referent's header
  keeps the weak list; `upgrade()` is a strong-count check + retain.
- Weak refs do not keep objects alive and are **not** destructors: they are
  for caches/observers. Cycles through weak refs are not cycles.
- Host opaques can expose their own `Weak` views (e.g. to detach a bridge
  when the script side is gone).

## 2. Cycle collector (trial deletion)

See **`examples/memory/node-cycle.rut`** for a minimal cycle.

Cycles can only form through **mutable heap slots**: Rc-cell class fields,
builtin `Option`/`Result` payloads holding refs, and `Vec<T: ref>`
elements. Strings, bytes, numeric vecs, dataclasses-without-refs, and bare
class values can never participate (RFC 0016 §4) — the scanner skips them
entirely. Slice view cells are walkable like any other cell: the owner
field is a typed handle (RFC 0016 §4).

Algorithm (Bacon–Rajan style trial deletion, stop-the-VM, budgeted):

1. every `release` that leaves rc==1 pushes the object to a *suspect* list;
2. when the suspect list crosses a threshold (or a byte-budget delta), run a
   collection pass;
3. **trial deletion**: DFS from suspects, decrementing internal counts;
4. **scan**: re-walk from the suspects; objects whose trial count is still 0
   are garbage (nothing outside pointed at them); reachable ones get their
   counts restored;
5. collected cycles are freed — destructors run (RFC 0016 §3), then fields.

- The whole pass walks only composite objects that *can* point at refs —
  never string/bytes/numeric-vec payloads, never registers.
- Budget: passes are capped by a work counter; if a pass exceeds it, the
  remainder is deferred to the next trigger (embedding-safe, like everything
  else — the host can also force `vm.collect_cycles()` between frames,
  RFC 0034 §5).
- Destructors of collected cycle members run in deterministic order
  (discovery order), and run **exactly once**.

## 3. Safety & testing

- rc discipline is asserted in debug builds (retain/release pairing, no
  underflow, no double-free of the same header);
- shutdown leak report: on `Vm::drop`, remaining non-immortal heap objects
  are logged with allocation sites (debug builds) — the same facility that
  makes cycle-collector tests trustworthy;
- fuzz targets: random programs asserting (a) no crash, (b) destructor
  counts match allocations, (c) cycle collection reclaims forced cycles;
- the collector is fully deterministic under the virtual clock, so tests
  reproduce exactly.

## 4. Internals: collector core

```rust
impl CycleCollector {
    /// Phase 3: trial-delete the subgraph reachable from `root`.
    fn trial_delete(&self, heap: &Heap, root: NonNull<Header>) {
        let mut stack = vec![root];
        while let Some(p) = stack.pop() {
            let obj = heap.obj(p);
            if !obj.mark_trial_decrement() { continue; }   // already visited
            for child in obj.ref_children(heap) {           // typed walk via RutType
                heap.trial_dec(child);
                if heap.trial_count(child) == 0 { stack.push(child); }
            }
        }
    }

    /// Phase 4/5: roots reachable from outside restore counts; the rest die.
    fn scan_and_free(&mut self, heap: &mut Heap) {
        let roots = mem::take(&mut self.suspects);
        for root in roots {
            if heap.has_external_ref(root) {        // some non-suspect points in
                self.restore(heap, root);           // undo trial decrements
            } else {
                for p in self.cycle_members(heap, root) {
                    heap.collect(p);                // dtor + release (RFC 0016 §3)
                }
            }
        }
    }
}
```

## Open questions

- OQ-1: should the `suspect` threshold be adaptive (RC-drop-ratio heuristic)
  rather than a fixed count?
