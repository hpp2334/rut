# RFC 0023: The `Value` Boundary & Borrow Guards

- **Status:** Draft
- **Date:** 2026-08-23
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
    Nil, Bool(bool), Char(char),
    I8(i8) /* .. */ I64(i64), U8(u8) /* .. */ U64(u64), F32(f32), F64(f64),
    Str(StrRef<'v>),                       // immutable, may point into heap
    Bytes(BytesRef<'v>),                   // immutable octets (RFC 0004)
    Vec(Borrow<'v, RutVec>),               // typed elem, zero-copy — internal only
    Cell(Handle), Trait(Handle), Opaque(Handle),
    // user value cells / enums, fat trait-object refs (RFC 0015 §6),
    // and Opaque boxes (RFC 0014) — user-erased values and host
    // payloads (RFC 0023/0026) share the one handle: the cell
    // discriminates, the rut type is `Opaque` either way (revised from
    // the earlier separate `Host(Handle)` — the boundary never needed
    // two currencies for one box)
    Opt(Option<Box<Value<'v>>>), Res(Result<Box<Value<'v>>, Box<Value<'v>>>),
    Template(Tmpl<'v>),                    // RFC 0027
}
```

**What may cross is a compile-time property of the surface.** An `entry
fn`'s parameters and return must be built from: primitives, `str`,
`bytes` (the immutable binary buffer, RFC 0004), `nil`, `Option`/`Result`
over crossable types, and `Opaque` (RFC 0014 — the one cell an embedder
may hold and pass back). Every other cell — dataclasses, classes,
`Vec<T>` of cells, `Vec<u8>` itself, trait-typed values — stays inside the VM;
violating shapes are **compile errors on the `entry fn` declaration**,
not call-time failures. Plain `pub` carries no such restriction: rut
modules exchange cells freely between themselves (RFC 0003 §2).
## 2. Borrow guards

`Borrow<'v, _>` is **call-scoped**: the Rust lifetime prevents storing it
past return; the VM additionally sets a *borrowed* flag on the object
header, and rut-side mutation ops (`buf.set`, `arr.set` …) check it —
so a re-entrant `vm.call` inside a native fn that tries to mutate a
borrowed buffer traps with `borrowed by host` instead of racing. Guards
clear on return. To keep data, the host copies — that is the whole rule.

This is why RFC 0016 OQ-1 (non-moving heap) matters: non-moving keeps
these borrows trivially sound forever.
