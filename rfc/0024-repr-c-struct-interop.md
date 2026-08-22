# RFC 0024: Repr C Struct Interop

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0023 (Value boundary), RFC 0015 §4 (value layout)
- **Supersedes:** RFC 0005 §4 (pre-restructure)
- **Part:** E — Host & FFI

## Summary

Both user value types are **C-layout** (RFC 0015 §4): fields in
declaration order, natural alignment, size padded to alignment; no hidden
members; `private`, `implements`, `dispose`, and `static` add **nothing**
to the layout. An `Rc<C>` cell is `Header + vtable-ptr + that same block`
(RFC 0015 §6) — `StructRef::fields_ptr()` hides the prefix.

The host mirrors the struct in Rust and registers the contract at startup:

```rust
#[repr(C)]
struct Vertex { x: f32, y: f32, color: u32 }

vm.register_struct::<Vertex>("Vertex")?;   // checks rut's Vertex layout:
                                           // size_of, align_of, field offsets
                                           // — mismatch = startup error
```

After registration, `StructRef<'v, Vertex>` in a native fn is literally
`&Vertex` — the host reads/writes fields at native speed (zero copies, no
per-field accessors). rut passes inline values by pointer to the frame /
vec element storage; the borrow flag guards re-entrant mutation
(RFC 0023 §2).

Script-side, the layout is queryable at compile time (RFC 0015 §3):
`size_of<T>()`, `align_of<T>()`, `type_id<T>()` — `Vec<T>` strides,
`StructCopy` sizes, and host struct mirrors all agree on one number
(`size_of<Array<T, N>>() = N × stride`, RFC 0005).

## Open questions

- OQ-1: `StructRef` mutability — v1 hands out `&T` (shared) + explicit
  `&mut T` only when the rut side provably cannot observe (moved values)?
  Proposed: `&T` only; writes go through returned values.
- OQ-2: should `register_struct` also permit *packed* layouts
  (`#[repr(packed)]`) or is repr C the single contract? Proposed: C only.
