# RFC 5004: rut — Memory Internals (implementation)

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Implements:** RFC 0004 (memory semantics) — heap layout, RC ops, cycle
  collector core. Language-facing contract + `examples/memory/*` references
  live in RFC 0004. Numbering: 5xxx = implementation RFCs.

## 1. Heap object layout

```rust
#[repr(C)]
struct Header {
    rc:   Cell<u32>,        // 0 = immortal (overflow sentinel, see RFC 0004 §2)
    ty:   u32,              // index into the VM's RutType table (RFC 0002 §10)
    flags: Cell<u32>,       // CC colors + weak-list bit + has-dtor bit
}

#[repr(C)]
struct RutString { h: Header, len: u32, bytes: [u8] }     // UTF-8, immutable
#[repr(C)]
struct RutBytes  { h: Header, len: u32, cap: u32, data: *mut u8 }
#[repr(C)]
struct RutArray  { h: Header, len: u32, cap: u32, elem: TypeId, data: *mut () } // unboxed
#[repr(C)]
struct RutClass  { h: Header, vt: *const VTable, fields: [Slot] } // Rc<T> CELL — Header + vt + the
                                                            // repr-C field block (RFC 0002 §10.2)
#[repr(C)]
struct RutEnum   { h: Header, tag: u32, payload: [Slot] }     // builtin Option/Result (RFC 0002 §3)
#[repr(C)]
struct RutOpaque { h: Header, boxed: *mut HostBoxed }     // host opaques (RFC 0005) AND
                                                          // user Opaque boxes (RFC 0002 §3.1);
                                                          // the header TypeId discriminates
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

Dataclass/bare-class values never appear as heap objects: they are inline
byte sequences spanning slots or a stack-frame region (RFC 0002 §10.2 —
repr C field blocks; `size_of<T>()` is this block's size).

## 2. Refcount ops

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
            self.collect.release(p);   // dtor (RFC 0004 §3) + release fields
        } else if n == 1 && h.ty_desc().can_participate_in_cycles() {
            self.collect.suspect(p);   // RFC 0004 §5: might be a dead cycle
        }
    }
}
```

## 3. Cycle collector core

Bacon–Rajan style trial deletion, budgeted (semantics in RFC 0004 §5):

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
                    heap.collect(p);                // dtor + release (RFC 0004 §3)
                }
            }
        }
    }
}
```
