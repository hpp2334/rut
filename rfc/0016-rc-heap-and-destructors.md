# RFC 0016: The RC Heap & Deterministic Destructors

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0011 (Rc/dispose), RFC 0015 (layout, slots)
- **Supersedes:** RFC 0004 §1–3, §6 + RFC 5004 §1–2 (pre-restructure)
- **Part:** C — Memory

## Summary

The rut heap is **reference-counted**. Strong counts reach zero ⇒ the object
is destroyed immediately — deterministic destructors for host resources, no
pause-the-world collector in the common case (the budgeted cycle collector
is RFC 0017). Every VM owns one heap on one thread; nothing is shared
between isolates (RFC 0021), so all counters are plain `Cell`s — no atomics.

## 1. Heap object model

Only these are heap objects: **Rc cells** (RFC 0011 — `Header +
vtable + fields`), **arrays**, **strings**, **bytes**, builtin
`Option`/`Result` *of references* (payload of a ref type is a pointer;
payload of a value type is inline), coroutine frames (RFC 0018 §4), and
host opaques (RFC 0025). Dataclasses and bare class values are **never heap
objects** — they are inline byte sequences with no header and no refcount
(RFC 0015 §4).

Each heap object starts with a header (rc count, type id, flags); the exact
layout is §5.

## 2. Refcounting rules

The compiler knows the static type of every register, so it emits ref-aware
ops only where a reference can flow (`Mov` for scalars, `MovRef` for refs —
§5). Consequences:

- **Function boundaries** pass references in registers; call/ret sequences
  do the paired inc/dec. Nothing per-element happens inside `Array<f32>`
  loops.
- **rc overflow**: on increment past `u32::MAX` the object is *immortalized*
  (rc pinned to 0) — a deliberate, logged leak instead of memory unsafety
  (Swift's rule). Debug builds assert the counter discipline.
- **Reference semantics** (RFC 0004 §2): Rc cells, arrays, strings and bytes
  are handles — passing them retains. There is no borrow syntax and no
  lifetimes; a handle simply keeps the referent alive, so nothing dangles.
  Uniqueness matters only for buffer *transfer* across isolates (RFC 0021
  §3), detected via rc==1 at runtime — never via static proofs.

## 3. Deterministic destructors

See **`examples/memory/temp-file.rut`**. A class may declare a reserved
`dispose(): void`; such classes must live behind `Rc` (bare use is a
compile error — RFC 0011 §2). Ordering guarantees:

1. the user `dispose()` method runs first, with all fields still valid;
2. fields are then released in declaration order (recursively);
3. host opaques (RFC 0025) run their Rust `Drop` at the same point —
   releasing textures/sockets when the last handle goes away, not "sometime
   later at GC";
4. coroutine frames are objects too: **task cancellation drops locals at the
   suspension point** (RFC 0019) — same machinery, no special case.

Note the difference from boa: no `FinalizationRegistry`, no flush jobs, no
"finalizer may run later or never".

## 4. Arrays & strings: why they stay cheap

- `Array<T>` for numeric/bool/char `T` stores raw elements inline
  (`Array<f32>` is literally `Vec<f32>` behind a header). Only the header is
  refcounted; element copies in/out are plain `Slot` moves, no inc/dec.
  `Array<Point>` (dataclass elements) is likewise inline and uncounted.
- `Array<T: ref>` stores pointers; the scanner sees element pointers, RC
  sees one count for the array. `push`/`pop`/`set` emit the right inc/dec
  ops.
- `string` is immutable → interned literals live in the module's constant
  pool (immortal, rc==0 sentinel); runtime-built strings are ordinary
  objects with no interior pointers. `bytes` likewise.
- No interior pointers exist anywhere (no `&mut` into the middle of an
  array/bytes in v1), which is exactly what keeps the cycle scanner a
  simple typed walk (RFC 0017 §2).

## 5. Internals: headers, layouts, refcount ops

```rust
#[repr(C)]
struct Header {
    rc:   Cell<u32>,        // 0 = immortal (overflow sentinel, see §2)
    ty:   u32,              // index into the VM's RutType table (RFC 0015)
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
                                                            // repr-C field block (RFC 0015 §4)
#[repr(C)]
struct RutEnum   { h: Header, tag: u32, payload: [Slot] }     // builtin Option/Result (RFC 0005)
#[repr(C)]
struct RutOpaque { h: Header, boxed: *mut HostBoxed }     // host opaques (RFC 0025) AND
                                                          // user Opaque boxes (RFC 0014);
                                                          // the header TypeId discriminates
```

Refcount ops — the compiler emits ref-aware ops only where a reference can
flow:

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
            self.collect.release(p);   // dtor (§3) + release fields
        } else if n == 1 && h.ty_desc().can_participate_in_cycles() {
            self.collect.suspect(p);   // RFC 0017 §2: might be a dead cycle
        }
    }
}
```

## Open questions

- OQ-1: moving/compacting collector for long-lived UI heaps — defer to v2;
  non-moving keeps host `&mut` borrows into `bytes`/`Array` trivially sound
  (RFC 0023 borrow guards).
- OQ-2: finalizer-style `dispose()` observer vs destructor-only —
  destructor-only proposed (weak refs, RFC 0017, cover the observer use
  case).
