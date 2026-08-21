# RFC 0023: The `Value` Boundary & Borrow Guards

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0022 (native modules), RFC 0016 (heap)
- **Supersedes:** RFC 0005 §3 (pre-restructure)
- **Part:** E — Host & FFI

## Summary

The one place tagged values exist: the host boundary. Inside the VM,
registers are untagged slots (RFC 0015 §5); crossing into Rust materializes
a `Value<'v>` — call-scoped, checked, and borrow-guarded.

## 1. The `Value` enum

```rust
pub enum Value<'v> {
    Void, Bool(bool), Char(char),
    I8(i8) /* .. */ I64(i64), U8(u8) /* .. */ U64(u64), F32(f32), F64(f64),
    Str(StrRef<'v>),                       // immutable, may point into heap
    Bytes(Borrow<'v, [u8]>),               // zero-copy, call-scoped
    Array(Borrow<'v, RutArray>),           // typed elem, zero-copy
    Struct(StructRef<'v>),                 // repr C block — RFC 0024
    Rc(Handle), Iface(Handle), Opaque(Handle), Host(Handle),
    Opt(Option<Box<Value<'v>>>), Res(Result<Box<Value<'v>>, Box<Value<'v>>>),
    Template(Tmpl<'v>),                    // RFC 0027
}
```

## 2. Borrow guards

`Borrow<'v, _>` is **call-scoped**: the Rust lifetime prevents storing it
past return; the VM additionally sets a *borrowed* flag on the object
header, and rut-side mutation ops (`buf.set`, `arr.set` …) check it —
so a re-entrant `vm.call` inside a native fn that tries to mutate a
borrowed buffer traps with `borrowed by host` instead of racing. Guards
clear on return. To keep data, the host copies — that is the whole rule.

This is why RFC 0016 OQ-1 (non-moving heap) matters: non-moving keeps
these borrows trivially sound forever.
