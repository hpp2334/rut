# RFC 0004: Memory — Refcounting, Destructors, Cycle Collection

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0001 (pillar P3), RFC 0002 (types), RFC 0003 (workers)

## Summary

The rut heap is **reference-counted**. Strong counts reach zero ⇒ the object
is destroyed immediately — deterministic destructors for host resources, no
pause-the-world collector in the common case. A budgeted **cycle collector**
(trial deletion) runs occasionally to reclaim reference cycles; weak refs are
supported. Every VM owns one heap on one thread; nothing is shared between
isolates (RFC 0003), so all counters are plain `Cell`s — no atomics.

## 1. Heap object model

```rust
#[repr(C)]
struct Header {
    rc:   Cell<u32>,        // 0 = immortal (overflow sentinel, see §3)
    ty:   u32,              // index into the VM's RutType table (RFC 0002 §9)
    flags: Cell<u32>,       // CC colors + weak-list bit + has-dtor bit
}

#[repr(C)]
struct RutString { h: Header, len: u32, bytes: [u8] }     // UTF-8, immutable
#[repr(C)]
struct RutBytes  { h: Header, len: u32, cap: u32, data: *mut u8 }
#[repr(C)]
struct RutArray  { h: Header, len: u32, cap: u32, elem: TypeId, data: *mut () } // unboxed
#[repr(C)]
struct RutClass  { h: Header, vt: *const VTable, fields: [Slot] } // class instance (RFC 0002 §10.2)
#[repr(C)]
struct RutEnum   { h: Header, tag: u32, payload: [Slot] }     // builtin Option/Result only (RFC 0002 §3)
#[repr(C)]
struct RutOpaque { h: Header, boxed: *mut HostBoxed }     // host type (RFC 0005 §5)
```

Interpreter registers hold **untagged 8-byte slots** — the bytecode is typed
(RFC 0002 §10), so tags are *not* needed for correctness in hot paths:

```rust
#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,        // every int width sign/zero-extends into here
    pub f: f64,        // f32 payloads widened
    pub b: bool,
    pub c: char,       // Unicode scalar
    pub r: Option<NonNull<Header>>,   // heap reference
}
```

The `Value` enum (tagged) exists **only at the FFI boundary** (RFC 0005 §3),
where the host needs to match on a type it may not statically know.

## 2. Refcounting rules

The compiler knows the static type of every register, so it emits ref-aware
ops only where a reference can flow:

```rust
// raw move — verifier guarantees non-ref type (ints, floats, ...)
Op::Mov { dst, src } => regs[dst] = regs[src],

// ref move — inc new, dec old; both are exact, no tag checks
Op::MovRef { dst, src } => {
    let p = regs[src].r;
    self.heap.retain(p);
    self.heap.release(regs[dst].r);
    regs[dst] = Slot { r: p };
}
```

```rust
impl Heap {
    pub(crate) fn retain(&self, p: Option<NonNull<Header>>) { /* rc += 1, */ }

    pub(crate) unsafe fn release(&self, p: Option<NonNull<Header>>) {
        let Some(p) = p else { return };
        let h = unsafe { p.as_ref() };
        let n = h.rc.get().checked_sub(1).expect("rc underflow (bug)");
        h.rc.set(n);
        if n == 0 {
            self.collect.release(p);   // dtor (§4) + release fields
        } else if n == 1 && h.ty_desc().can_participate_in_cycles() {
            self.collect.suspect(p);   // §5: might now be unreachable cycle
        }
    }
}
```

- **Function boundaries** pass references in registers; call/ret sequences do
  the paired inc/dec. Nothing per-element happens inside `array<f32>` loops.
- **rc overflow**: on increment past `u32::MAX` the object is *immortalized*
  (rc pinned to 0) — a deliberate, logged leak instead of memory unsafety
  (Swift's rule). Debug builds assert the counter discipline.
- **Reference semantics** (RFC 0002 §2): class instances, arrays, strings and
  bytes are handles — passing them retains. There is no borrow syntax and no
  lifetimes; a handle simply keeps the referent alive, so nothing dangles.
  Uniqueness matters only for buffer *transfer* across isolates (RFC 0003
  §5.2), detected via rc==1 at runtime — never via static proofs.

## 3. Deterministic destructors

```rut
class TempFile {
    constructor(public path: string) { }

    dispose(): void {                    // reserved method = destructor
        fs.remove(this.path);
    }
}

function scratch(): void {
    const f = new TempFile("tmp.dat");
    use(f);
}   // rc hits 0 HERE: `dispose()` runs, then fields are released in order
```

Ordering guarantees:

1. the user `dispose()` method runs first, with all fields still valid;
2. fields are then released in declaration order (recursively);
3. host opaques (RFC 0005 §5) run their Rust `Drop` at the same point —
   releasing textures/sockets when the last handle goes away, not "sometime
   later at GC";
4. coroutine frames are objects too: **task cancellation drops locals at the
   suspension point** (RFC 0003 §3) — same machinery, no special case.

Note the difference from boa: no `FinalizationRegistry`, no flush jobs, no
"finalizer may run later or never".

## 4. Weak references

```rut
const cache = new Cache();
const w = weak(cache);                   // Weak<Cache>
// ... later, cache may or may not be gone:
const hit = w.upgrade();                 // Option<Cache> — upgrade keeps alive
if (hit.isSome()) {
    hit.value.reuse();
}
```

- `weak(&x)` allocates a `WeakBox` side object holding a back-pointer that is
  nulled when the referent dies (strong count hits 0). The referent's header
  keeps the weak list; `upgrade()` is a strong-count check + retain.
- Weak refs do not keep objects alive and are **not** destructors: they are
  for caches/observers. Cycles through weak refs are not cycles.
- Host opaques can expose their own `Weak` views (e.g. to detach a bridge
  when the script side is gone).

## 5. Cycle collector (trial deletion)

Reference cycles are the one thing RC cannot free:

```rut
class Node {
    next: Option<Node> = Option.none();
}

const a = new Node();
const b = new Node();
a.next = Option.some(b);   // rc(b) = 2 ...
b.next = Option.some(a);   // ... and rc(a) = 2: neither ever reaches 0
```

Cycles can only form through **mutable heap slots**: class fields,
builtin `Option`/`Result` payloads, and `array<T: ref>` elements. Strings,
bytes, and numeric arrays can never participate (§6) — the scanner skips them
entirely.

Algorithm (Bacon–Rajan style trial deletion, stop-the-VM, budgeted):

1. every `release` that leaves rc==1 pushes the object to a *suspect* list;
2. when the suspect list crosses a threshold (or a byte-budget delta), run a
   collection pass;
3. **trial deletion**: DFS from suspects, decrementing internal counts;
4. **scan**: re-walk from the suspects; objects whose trial count is still 0
   are garbage (nothing outside pointed at them); reachable ones get their
   counts restored;
5. collected cycles are freed — destructors run (§3), then fields.

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
                    heap.collect(p);                // dtor + release, §3
                }
            }
        }
    }
}
```

- The whole pass walks only composite objects that *can* point at refs —
  never string/bytes/numeric-array payloads, never registers.
- Budget: passes are capped by a work counter; if a pass exceeds it, the
  remainder is deferred to the next trigger (embedding-safe, like everything
  else — the host can also force `vm.collect_cycles()` between frames).
- Destructors of collected cycle members run in deterministic order
  (discovery order), and run **exactly once**.

## 6. Arrays & strings: why they stay cheap

- `array<T>` for numeric/bool/char `T` stores raw elements inline
  (`array<f32>` is literally `Vec<f32>` behind a header). Only the header is
  refcounted; element copies in/out are plain `Slot` moves (`Mov`), no inc/dec.
- `array<T: ref>` stores pointers; the scanner sees element pointers, RC sees
  one count for the array. `push`/`pop`/`set` emit the right inc/dec ops.
- `string` is immutable → interned literals live in the module's constant
  pool (immortal, rc==0 sentinel); runtime-built strings are ordinary objects
  with no interior pointers. `bytes` likewise.
- No interior pointers exist anywhere (no `&mut` into the middle of an
  array/bytes in v1 — see RFC 0002 OQ-5), which is exactly what keeps the
  cycle scanner a simple typed walk.

## 7. Safety & testing

- rc discipline is asserted in debug builds (retain/release pairing, no
  underflow, no double-free of the same header);
- shutdown leak report: on `Vm::drop`, remaining non-immortal heap objects
  are logged with allocation sites (debug builds) — the same facility that
  makes cycle-collector tests trustworthy;
- fuzz targets: random programs asserting (a) no crash, (b) destructor
  counts match allocations, (c) cycle collection reclaims forced cycles;
- the collector is fully deterministic under the virtual clock, so tests
  reproduce exactly.

## Open questions

- OQ-1: moving/compacting collector for long-lived UI heaps — defer to v2;
  non-moving keeps host `&mut` borrows into `bytes`/`array` trivially sound
  (RFC 0005 §3 borrow guards).
- OQ-2: should `suspect` threshold be adaptive (RC-drop-ratio heuristic)
  rather than a fixed count?
- OQ-3: finalizer-style `dispose()` observer vs destructor-only —
  destructor-only proposed (weak refs cover the observer use case).
- OQ-4: per-worker heap quotas (`vm.set_heap_budget`) so a runaway worker
  traps instead of OOM-ing the host — likely yes, cheap to add.
