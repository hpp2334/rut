# RFC 0016: The RC Heap & Deterministic Destructors

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0011 (reference semantics/dispose), RFC 0015 (layout, slots)
- **Supersedes:** RFC 0004 §1–3, §6 + RFC 5004 §1–2 (pre-restructure)
- **Part:** C — Memory

## Summary

The rut heap is **reference-counted**. Strong counts reach zero ⇒ the object
is destroyed immediately — deterministic destructors for host resources and
**no collector at all**: strong cycles leak by design (`Weak<T>` is the
answer — RFC 0017). Every VM owns one heap on one thread; nothing is shared
between isolates (RFC 0021), so all counters are plain `Cell`s — no atomics.

## 1. Heap object model

**Everything except primitives is a heap cell.** The numeric types
(`u8..u64`, `i8..i64`, `u/isize`), `f32`/`f64`, `bool`, and `char` are
inline `Slot` values moved by plain `Mov`. Every other value — `string`,
`Vec<T>`, `Array<T, N>`, builtin `Option`/`Result`, user enums,
dataclass and class instances, `Opaque` boxes (RFC 0014), and host
opaques (RFC 0025) — **is a heap cell handle**: assignment, passing, and
returning copy the handle (`MovRef`, rc++), and mutation is visible
through every alias. Reference semantics is the one default regime
(RFC 0004 §2); the eager copy is the `own(x)` builtin (RFC 0011 §1).
Each cell starts with a
header (rc count, type id, flags); the exact layout is §5. Composite
fields and composite elements store **cell handles** (pointer slots);
only primitive-element buffers are flat (§4). Coroutine frames
(RFC 0018 §4) are cells too.

## 2. Refcounting rules

The compiler knows the static type of every register, so it emits ref-aware
ops only where a reference can flow (`Mov` for scalars, `MovRef` for refs —
§5). Consequences:

- **Function boundaries** pass references in registers; call/ret sequences
  do the paired inc/dec. Nothing per-element happens inside `Vec<f32>`
  loops.
- **rc overflow**: on increment past `u32::MAX` the object is *immortalized*
  (rc pinned to 0) — a deliberate, logged leak instead of memory unsafety
  (Swift's rule). Debug builds assert the counter discipline.
- **Reference semantics** (RFC 0004 §2): every non-primitive is a handle —
  cells, vecs, arrays, slices, strings,
  and enums retain on assignment. There is no borrow syntax and no
  lifetimes; a handle simply keeps the referent alive, so nothing dangles.
  The `mut`-binding law (RFC 0003 §1) is what gates *writing* through a
  handle, never the sharing. Value divergence — a private copy that
  aliases nothing — is the `own(x)` builtin (RFC 0011 §1).
  Uniqueness matters only for buffer *transfer* across isolates (RFC 0021
  §3), detected via rc==1 at runtime — never via static proofs.

## 3. Deterministic destructors

See **`examples/memory/temp-file.rut`**. A class may implement the
prelude interface `Disposal` (`fn dispose(mut self): void`, RFC 0028);
a Disposal class is just a class.
Ordering guarantees:

1. `Disposal.dispose(mut self)` runs first — dispatched through the
   type's vtable like any interface call — with all fields still valid;
2. fields are then released in declaration order (recursively);
3. host opaques (RFC 0025) run their Rust `Drop` at the same point —
   releasing textures/sockets when the last handle goes away, not "sometime
   later at GC";
4. coroutine frames are objects too: **task cancellation drops locals at the
   suspension point** (RFC 0019) — same machinery, no special case.

Note the difference from boa: no `FinalizationRegistry`, no flush jobs, no
"finalizer may run later or never".

## 4. Vecs, arrays, slices & strings: where flatness survives

- **Primitive-element buffers stay flat.** `Vec<T>` and `Array<T, N>` for
  numeric/bool/char `T` store raw elements inline (a flat `f32` buffer
  behind the header). Only the header is refcounted; element copies in/out
  are plain `Slot` moves, no inc/dec. This is the one place the
  everything-is-a-cell law does not reach: primitive `Slot`s have no
  identity to share.
- **Composite-element buffers store handles.** `Vec<Point>` /
  `Array<Point, N>` hold one cell pointer per element; the scanner sees
  element pointers, RC sees one count for the buffer itself.
  `push`/`pop`/`set` emit the right inc/dec ops. `own(v)` over such a
  buffer clones the buffer but shares the element cells (shallow).
- `Array<T, N>` is a fixed-length cell — the same shape as `RutVec` with
  `len == cap == N` frozen; `N` remains a compile-time constant and part
  of the type's identity (RFC 0005). Fixed-array literals that fold at
  compile time (RFC 0033 §3) become immortal constant-pool cells
  (rc==0 sentinel), like interned strings.
- **`dyn Slice<T>` cells** — one kind: the **view cell**
  (`RutSliceRef`: holds a *cell handle* to the owner Vec or Array cell
  plus `off`/`len`, **never a pointer into the data block**). View cells
  dispatch through the owner cell, so growth (a Vec's buffer may move)
  keeps existing `dyn Slice<T>` values valid; indexing bounds-checks
  len ∩ owner len — an out-of-range index traps, never reads garbage.
  View cells carry the builtin `Slice<T>` slots —
  dyn-slice `x[i]` get/set, `.len()`, `for..of` lower to `calli`
  (RFC 0032 §1.1 R2), and dispatch-through-owner is simply the slot
  target. They are not
  nameable or constructible in script: there is no `as_slice()`; slices
  are born only from implicit widening at the widening site (RFC 0011
  §3). This *refines* the no-interior-pointer rule rather than breaking
  it: the heap walk still sees only typed cell handles.
- Any slot typed with a **`dyn` type stores a cell handle** — `dyn I` and
  `dyn Slice<T>` alike (RFC 0031 §4); unsized payloads always sit behind
  a cell boundary.
- `string` is immutable → interned literals live in the module's constant
  pool (immortal, rc==0 sentinel); runtime-built strings are ordinary
  cells with no interior pointers, and their buffers are shared
  copy-on-write **internally** (Rust-side `Rc<[u8]>`-style cloning at
  concat) — an implementation detail never exposed to the user: there is
  no mutation API on `string`, so COW and always-copy are
  observationally identical (RFC 0004 §2).
- No interior pointers exist anywhere (no `&mut` into the middle of a
  vec in v1), which is exactly what keeps the cycle scanner a
  simple typed walk (RFC 0017 §2) — cycles through ordinary user values
  included (§1).

## 5. Internals: headers, layouts, refcount ops

```rust
#[repr(C)]
struct Header {
    rc:   Cell<u32>,        // 0 = immortal (overflow sentinel, see §2)
    ty:   u32,              // index into the VM's RutType table (RFC 0015)
    flags: Cell<u32>,       // weak-list bit + has-disposal-impl bit
}

#[repr(C)]
struct RutString { h: Header, len: u32, bytes: [u8] }     // UTF-8, immutable; buffers
                                                          // internally COW-shared (§4)
#[repr(C)]
struct RutVec    { h: Header, len: u32, cap: u32, elem: TypeId, data: *mut () } // growable buffer —
                                                              // flat for primitive elem,
                                                              // cell pointers otherwise (§4)
#[repr(C)]
struct RutArray  { h: Header, len: u32, elem: TypeId, data: *mut () } // fixed cell: len == cap == N,
                                                              // const per instantiation (RFC 0005)
struct RutSliceRef   { h: Header, owner: Handle<RutVec>, off: u32, len: u32 } // view cell over a Vec or
                                                              // Array cell — dispatches through
                                                              // the owner, never a pointer into
                                                              // the data block (§4)
#[repr(C)]
struct RutCell   { h: Header, vt: *const VTable, payload: [Slot] } // EVERY user value (dataclass,
                                                              // class) — Header + (vtable, when
                                                              // the type has impls) + the repr-C
                                                              // payload block (RFC 0015 §4);
                                                              // composite fields are cell-handle
                                                              // Slots, primitive fields inline
#[repr(C)]
struct RutEnum   { h: Header, tag: u32, payload: [Slot] }     // builtin Option/Result (RFC 0005)
                                                              // and user enums; dataless variants
                                                              // are immortal singleton cells (§1)
#[repr(C)]
struct RutOpaque { h: Header, boxed: *mut HostBoxed }     // host opaques (RFC 0025) AND
                                                          // user `Opaque` boxes (RFC 0014);
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
            self.heap.destroy(p);      // Disposal impl (§3) + release fields
        }                              // no suspect list — cycles leak (RFC 0017)
    }
}
```

## Open questions

- OQ-1: moving/compacting collector for long-lived UI heaps — defer to v2;
  non-moving keeps host `&mut` borrows into `Vec` buffers trivially sound
  (RFC 0023 borrow guards).
- OQ-2: finalizer-style `dispose()` observer vs destructor-only —
  destructor-only proposed (weak refs, RFC 0017, cover the observer use
  case).
