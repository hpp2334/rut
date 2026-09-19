# RFC 0044: Pass-by-Reference & Nullable Types — the Sharing Regime and `?T`

- **Status:** Draft — landed (the byref-nullable batch; see §8)
- **Date:** 2026-09-19
- **Author:** hpp2334
- **Depends on:** RFC 0004 (primitives), RFC 0005 (builtin generics — the
  `?T` type), RFC 0011 (reference semantics), RFC 0016 (the RC heap),
  RFC 0042 (views — §6's spelling moves to `?Vec<T>`)
- **Supersedes:** RFC 0005 §8 (the `*T`/`&v` pointer surface), RFC 0011 §1
  (the `own(x)` copy law), the record/array copy-by-value clauses of
  RFC 0004 §1/§2 and RFC 0016 §1/§4, RFC 0042 §6's `*Vec<T>` spelling
- **Part:** B — Language surface

## Summary

One sharing regime, one nullable type, one copy escape hatch:

- **Bindings share by reference.** Primitives and `fn` values copy
  (immediate slots); **every cell type shares its cell** — `str`,
  `bytes`, struct/class records, `[T]` arrays, enums, trait objects,
  `opaque` boxes, closures' captured cells, and `?T` boxes. `let b = a`
  is an O(1) handle move (`MovRef`: retain new, release old); mutation
  through any alias is visible through all of them, and the `mut`-binding
  law (RFC 0003 §1) gates writing — never the sharing.
- **`?T` is the nullable**: a nil-able cell, spelled prefix-only
  (`?` binds tightest), with `nil` as the null literal and auto-deref at
  every use. It replaces the removed `*T`/`&v` surface outright.
- **`==` is identity for cells**: primitives by value, `str`/`bytes` by
  content, everything else the raw slot compare.
- **`bytes.clone()` is the ONLY copy**. `own(x)`, `make_ptr(v)`, and the
  generic deep-copy engine path are removed; a use site diagnoses with
  the removal and its replacement.

The engine deletes its copy machinery (`CloneVal`, `MoveVal`,
`ArrGetRef`, `ValEq`, the move-elision liveness pass); the module binary
version bumps to 5 to invalidate stale caches. Nothing else in this RFC
is optional: sharing, `?T`, identity `==`, and `bytes.clone()` landed
together and the language's RFCs (0004, 0005, 0007, 0016, 0042) were
amended in place to match.

## 1. The sharing regime

The old law copied: records and arrays were **values** — `let b = a`
memcpy'd the whole payload, every store into a record field or array
slot cloned, and a liveness pass move-elided the copies it could prove
dead. The copy law is gone, machinery and all:

- `let b = a` — any cell type — is one handle move. A rehash's
  `let old = self.keys` is O(1); passing a 1M-element array to a
  function retains; `for (let x of xs)` yields the shared element, not
  a per-element copy (`ArrGetRef` is deleted — the element handle
  comes out as-is).
- Primitive elements in a `[T]` stay flat slots (RFC 0016 §4) — those
  copies were and are plain `Slot` moves, untouched by this RFC.
- A `T` stored into a `?T` slot boxes once (§2); a cell stored into any
  slot retains its handle. There is no deref-on-load/box-on-store
  special case anywhere.
- Copy-by-value is now unobservable except through the two legal
  effects: aliasing (writes visible through every binding) and `==`
  (§3). `bytes.clone()` (§4) is the one way to mint a divergent value.

The `own` spelling is removed with the law (`REMOVED_CORE`: *"`own` was
removed — bindings share by reference now (RFC 0044); `bytes.clone()`
is the one copy escape hatch"*).

## 2. `?T` — the nullable type

`?T` is a nil-able cell: the same one-slot box the old `*T` pointer was,
under a name that says what it means. `nil` is the null literal;
dereferencing it traps `NilDeref` — never a silent read.

### 2.1 Grammar — prefix only, binds tightest

`?` applies to the **type term that follows it**:

| spelling | type | reading |
|---|---|---|
| `?T` | `T \| nil` | the nullable |
| `[?T]` | `[T \| nil]` | array of nullables (each element nil-able) |
| `?[T]` | `[T] \| nil` | nullable array (the whole array nil-able) |
| `??T` | `T \| nil \| nil` | chains — same runtime box, unwrapped transitively |

There is no postfix `T?` and no `*T`: each diagnoses — *"the pointer
spelling `*T` was renamed — write `?T`"* / *"the postfix spelling `T?`
was removed — write `?T`"* — and the compile fails. In expression
position `*x`/`&x` diagnose *"bindings share by reference now — pass
`x` directly"* (binary `*`/`&` operators are untouched).

### 2.2 Coercions — the one-line rule

**`T → ?T` boxes; `?T → T` derefs.**

- The widen is implicit at any expected-`?T` position (let/arg/return/
  field/element store): the `MakeOpt` op mints a fresh one-slot cell.
  The box **aliases** `v`'s cell — a share, never a copy; primitives
  and `nil` copy the bits.
- The narrow is implicit at any expected-`T` position: a field-0 read
  plus nil check. A `nil` payload reaching a `T` use is the `NilDeref`
  trap.
- Unwrap is **transitive**: a `??T` funnels through as many derefs as
  its nesting demands at the use site — no per-level ceremony in user
  code.

Auto-deref covers every value position: `p.x`, `p.m(..)`, `p[i]`,
`for (x of p)`, arithmetic on `p`'s payload, `p == nil` (compares the
null slot). Guard with `p == nil` before use; the trap is the bug-catcher,
not the semantics.

### 2.3 `nil` typing

A context-free `nil` has type `nil` (RFC 0004 §4). In an expected-`?T`
position it types as that `?T` — so `let p: ?Node = nil` and a
`left: nil` field in a literal just work, and absence reads plainly:
a lookup returns `?V`, and `nil` means "not found".

### 2.4 Boundaries

- **The host boundary does not admit `?T`** — the same rule `*T` had
  (RFC 0023). Inside the VM it is an ordinary cell; crossing it is a
  decl-surface error, not a runtime hazard.
- `on_drop<T>(p: ?T, cleanup: fn(?T))` (RFC 0016 §3) — a `?T` binding
  IS the cell reference, so the cleanup signature is the nullable.

## 3. `==` — identity for cells, content for text

| operand type | `==` means | lowering |
|---|---|---|
| numeric/bool primitives | value | `CmpOp` |
| `str` | content (codepoints) | `StrCmp` |
| `bytes` | content (octets) | `ArrayCmp` |
| everything else — records, arrays, enums, closures, trait objects, `opaque`, `?T` | **cell identity** | `RefEq` (the raw slot compare) |

Identity is the only `==` sharing can defend: with aliasing everywhere,
structural equality of two independently-built `[1, 2]` cells is
ambiguous (`[1,2] == [1,2]` is **false** — two cells), and identity is
O(1) with no deep walk. Compare content where content is the contract:
a loop, or mapset's `Hashable.hash_eq` for keys (§5). `?T == ?T` is
slot identity — two `nil`s are equal, a null and a box are not, and
`p == nil` (the elem/nullable mixed form) derefs the nullable side and
compares against the null. `==` on `Option`/`Result` spellings remains
a compile error (RFC 0005 §10); `ValEq` is deleted from the engine.

## 4. `bytes.clone()` — the one copy escape hatch

`b.clone() -> bytes` mints a fresh buffer with `b`'s octets — a one-shot
deep copy, the ONLY copy syntax in the language. It lowers to the
native `BytesClone`; `bytes.from(a)` deep-copies through the retained
`Own` op. There is no generic `clone(x)` and no `own` — every other
type shares on binding, and a divergent value of any other type is
unreachable (build a new one instead).

## 5. Containers under sharing

The tree packages resyntaxed mechanically; their shapes are now:

- **`pouch`'s `Vec<T>`**: `buf: [?T]` — the nullable-handle backing.
  `[nil; cap]` is the generic zero (`nil` is the slot's zero; the
  repeat's nil-fill stays the memset-class op), `push` takes the
  `T → ?T`-boxed handle, loads yield the `?T` (uses auto-deref). The
  repeat `[v; n]` **retains** the handle n times — every slot aliases
  the one cell, by the sharing law.
- **`mapset`**: `keys: [?K]`, `vals: [?V]`, `get(k) -> ?V` — absence is
  `nil` on the `?V`. No key is ever copied: a `?K` handle shares the
  key's cell, and `mark_dead`'s nil-store releases it with the last
  reference. Rehash is O(1) by construction — the old arrays are
  shared, not copied. The HOT-LOOP LAW comments now state the sharing
  law (hoist scalar field reads; array bindings are O(1) shares).
- **`nmapset`** (the host-table wrapper): the wrapper got simpler —
  `?V` IS the absence type, so `get` returns the stored cell directly.

RFC 0042 §6's array windows lower unchanged, boxed as **`?Vec<T>`**
via the same `T → ?T` coercion — the window IS the shared cell, so
writes through it hit the parent.

## 6. `opaque(v)` and `opaque.downcast<T>(o)`

The erasure surface (RFC 0014, as amended) rides on this regime:

- Construction is a **call of the type name**: `opaque(v)` boxes any
  value — the `opaque.new(v)` static was folded into the call. The box
  aliases `v`'s cell (§2.2's widen law, at `T → opaque`).
- Recovery is the member static: `opaque.downcast<T>(o) -> (T, bool)` —
  `false` on a mismatch with `.0` at the type's zero; `x is T` probes
  without recovering. The free `downcast<T>(o)` fn is removed
  (`REMOVED_CORE`), semantics unchanged.

## 7. What it buys, what it costs

Full-suite before/after lives in `benches/README.md`'s performance log
(same-day runs, all checksums identical — the flip is behavior-neutral
by construction). Headlines from the in-process probe: the workloads
the old regime copied hardest moved most — fannkuch −54% exec,
quicksort −42%, binary-trees −17% (record/array churn is now handle
traffic); the map rows moved −2-6% (rehash was already move-elided
under the old law); and the price is visible where primitive elements
cross a growable sequence's `[?T]` backing per store/load — the
`array` workload +21% exec, `sieve` +7%. Flat primitive buffers
(`[i32]`) are untouched; the known lever for the growable case is
deferred (OQ-1).

## 8. Shipped state

- `TyKind::Opt { elem }` (name `"?{}"`) — the old pointer kind code 13
  is REUSED for the nullable; `TypeKind::TyOpt` in the AST. Module
  format **VERSION 5** invalidates v4 caches.
- Ops: `MakeOpt` (wire code 89 — the old `MakePtr`'s, unchanged).
  Deleted: `MakePtr` (renamed), `CloneVal`, `MoveVal`, `ArrGetRef`,
  `ValEq`, the `moveval.rs` liveness pass and its peephole rules, the
  boxed-pointer-array path in slice lowering (`SliceInfo::boxed`).
- `RefEq` for identity, `StrCmp`/`ArrayCmp` for text content, `NilDeref`
  for the null use, `Nat::BytesClone` for §4.
- `types.rs` `is_value` answers `false` uniformly — kept as a predicate
  because the VM's deep-copy paths branch on it.
- Parser: prefix `?` stage only; the `*T`, postfix `T?`, `*x`, `&x`
  arms diagnose and recover as §2.1 quotes.
- `REMOVED_CORE` rows: `own`, `make_ptr`, `downcast` (moved to the
  `opaque` member).
- RFCs amended in place: 0004 §1/§2/§4, 0005 §8/§9/§10, 0007 (nil,
  array literals), 0016 §1/§2/§3/§4, 0042 §1/§6/§8.

## Open questions

- OQ-1: flat primitive elements under a growable sequence — `Vec<i32>`
  pays one box per `push` under the `[?T]` law (the `array`/`sieve`
  regression). A copy-on-write or boxed-only-when-shared element scheme
  is the known lever; deferred — it complicates the release walk for a
  measured, bounded cost.
- OQ-2: `?T` across the host boundary — follow-up if ever needed
  (a non-goal here, §2.4).
- OQ-3: a content-equality request channel (a user-land `equals`
  convention) — deliberately none; `Hashable.hash_eq` covers the map
  need, and everything else is a loop.
