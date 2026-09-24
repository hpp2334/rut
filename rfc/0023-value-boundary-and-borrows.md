# RFC 0023: The `Value` Boundary & Borrow Guards

- **Status:** Draft — **REVISED & IMPLEMENTED (2026-09-18)**: the
  boundary currency is typed Rust; the enum below is the crate-internal
  marshaling format. See "Revision — implemented design" after §1.
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Revised:** 2026-09 — the err-channel amendment (end of this RFC):
  `?T` crosses the boundary nil-flattened (the one `Opt` arm), and the
  two-channel law — returned err = data, panic = drift.
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

## Amendment (Sep 2026, err-channel): `?T` crosses nil-flattened — and the two-channel law

Landed (err-channel phase 3, `docs/err-channel-report.md` §4). Three
records against the text above:

**The one `Opt` arm.** The crossing rule gains exactly one arm:
**`?T` crosses iff `T` crosses** (`crosses_boundary`,
`rut-core/src/types.rs`). This closes the gap between §1's own promise
("optionals cross as the v1.1 tuples they were always spelled as" —
the revision note above) and an implementation that predated it: the
empirical base rejected `entry fn -> (?i64, str)` while the same
signature with a prim first element compiled. The decode is
**nil-flattening** — the null slot becomes `Value::Nil`, a some-slot
decodes its payload (re-entering rut re-boxes through the ordinary
`T → ?T` funnel); no `Value::Opt` variant exists (nil IS absence, the
v1.1 convention). Tuples still cross field-by-field, so the answer
channel **`(?T, err)`** types under the ORIGINAL rule — no
err-specific carve-out, ever: a `?T` whose element does not cross
still rejects (pinned as a compile-error test). RFC 0044 §2.4's "the
host boundary does not admit `?T`" is superseded by this arm.

**The two-channel law (the containment semantics).** A returned err is
**data**: it crosses as the pair's second component and the host reads
and acts on it. A panic is **drift**: it stays on the loud channel
(`Result<Value, Trap>` — `Err` means a trapped turn, a bug), and never
fills an err field. The convention on the pair (RFC 0044's amendment):
empty err + a value = success; empty err + nil = "not found"; non-empty
err = "failed" — the caller's convention, not boundary-enforced.

**The envelopes.** The embedder surface (`vm.call -> Result<Value,
Trap>`) decodes a `-> (?T, err)` entry positionally:
`Value::Tuple([Nil|payload, Str(err)])`. The raw-ABI JSON envelope
(rut-wasm) grows **`"err"` beside `"trap"`** — the two channels stay
distinct by law: `trap` carries panics, `err` carries a returned
failure; a soft-fail run reads `"trap": null, "err": "…"`. RFC 0035's
amendment records the host-loop side (the web pump's decode-report-
keep-draining). Module VERSION rides 8 — a check relaxation, not a
format change (no new op/encoding/surface; old artifacts are
behaviorally bit-identical because the old checker rejected exactly
the shapes the new arm admits).

## Amendment (Sep 2026, nmapset-hostops): the stable-handle law — one crossing per op

Landed (docs/nmapset-hostops-report.md; phases `2f5ea84`/`54fceca`).
Records against the text above — the `nmap_host` surface (§2's
`pub host fn` law) grows the FUSED FAMILY, and the wrapper's impl
control flow moves host-side with it:

**The handle column.** The native table carries, alongside keys and
recorded hashes, one **stable birth handle per key** — assigned at
first insert, **monotonic** (never recycled in v1; a removed key's
handle dies with it), and **relocated with the keys inside the host's
own grow** (the same re-slot walk). The handle is the key's IDENTITY,
not its address: slots move on grow, handles never do.

**The fused family (18 crossings).** `map_h{put,find,remove}_{
i,u,b,s,y,sv}` — the typed lanes' exact key shapes, three answers:
`hput` fuses entry + **internal grow** at the load-factor boundary
and answers the packed **`(handle << 1) | newly`** in `i64` (bit 0 =
the newly flag — `HashSet` shares the lane and reads exactly it; the
handle space is bounded by BIRTHS, not cap, so the i64 carrier has
headroom far past any real table); `hfind`/`hremove` answer the key's
handle, or `-1`; `hremove` names the **dead key's own handle**, so a
sidecar wrapper nils exactly that slot and releases the cell. The sv
lane and the s lane of the same content are ONE key — same slot, same
handle.

**The wrapper law.** A rut-side `[?V]` sidecar indexed by the handle
is append-only (doubling when births outgrow the pre-size): no
sentinel loop, no relocation drain, no per-grow reallocation.
`get -> ?V`'s one-cell aliasing law is untouched by construction —
`sidecar[handle]` IS the cell.

**The legacy list (kept, escape hatches).** The round-1 opaque lanes
(`map_entry`/`map_find`/`map_remove`), `map_needs_grow`, the sentinel
`map_{entry,find,remove}_*` typed lanes, `map_grow` /
`map_take_reloc`, and the val-column pairs `map_val_{set,get}_{u,f}`
stay DECLARED and bound — same table, same slots, same recorded
hashes. The wrapper no longer imports them; `nmap_valcolumn`'s driver
suite keeps them exercised.

**Checksum law.** The handle column is additive state the legacy
paths never read; slot assignment, probe order, and every per-key
answer are untouched — every existing bench pin is immovable through
the takeover (measured, all rows × {rut, node, qjs}).
