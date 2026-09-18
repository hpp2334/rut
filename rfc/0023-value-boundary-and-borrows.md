# RFC 0023: The `Value` Boundary & Borrow Guards

- **Status:** Draft — **REVISED & IMPLEMENTED (2026-09-18)**: the
  boundary currency is typed Rust; the enum below is the crate-internal
  marshaling format. See "Revision — implemented design" after §1.
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

## Revision — implemented design (2026-09-18)

The boundary shipped **typed-Rust-first**, superseding the lifetime-
carrying enum above in API terms (`Value`/`Slot` are `pub(crate)`
marshaling formats inside `rut-vm`; nothing outside the crate names
them):

- **Marshaling traits** (`rut-vm/src/interp/boundary.rs`): `Arg<'a>` /
  `Ret` — each Rust type declares the rut `TypeId` it binds against
  (`i32` ⇒ `TY_I32`, `OpaqueBox<T>` ⇒ `TY_OPAQUE`, …) and converts
  checked, with kind-mismatch traps naming both sides.
- **Zero-copy borrows** (§2's law, made visible in the signature):
  `&'a str` / `&'a [u8]` host-fn params read the block store directly;
  the handler's bound is HRTB over `'a`, so a borrowed param cannot
  outlive its call — smuggling is a type error, not a runtime check.
  Owned `String` / `Vec<u8>` params are the explicit "I keep this data"
  copy.
- **No `Opt`/`Res` variants** — the enum never grew them; optionals
  cross as the v1.1 tuples (`(T, err)`, RFC 0007 §7) they were always
  spelled as at the surface. Option/Result cross fine, but the
  host-side spelling is the tuple.
- **Host fns are typed callables**: the registered closure/fn's Rust
  parameter types ARE the `.d.rut` row (RFC 0025 revised); a param type
  with no `Arg` impl is a compile error at the register site.
- **The dispatch entry** is one table row — a fn pointer, a state word,
  the declared return (RFC 0026 revised §ABI) — and host traps travel
  an explicit channel (`vm.trap` + one check per call) instead of a
  `Result` in the ABI.

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
