# RFC 0032: Typed Bytecode (LIR)

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0031 (HIR), RFC 0015 (slots, inline values),
  RFC 0018 (suspend), RFC 0025–0026 (native calls)
- **Supersedes:** RFC 0007 §4 (pre-restructure)
- **Implementation side:** the VM that executes this code is RFC 0034.
- **Part:** F — Toolchain & artifacts

## Summary

Registers are `u16` indices into a per-frame `Vec<Slot>` (RFC 0015 §5);
every register has a **static type** recorded in the function signature —
the verifier re-checks all of it at load (RFC 0033 §2). Inline values
(dataclass / bare class, RFC 0015 §4) occupy consecutive registers;
`StructCopy` is a `memcpy` of `size_of` bytes with compiler-emitted
ref-field retain/release.

## 1. Ops

Representative ops (abbreviated; the full table is mechanical and lives
with the VM crate):

```text
mov     rD, rS            ; untyped move (verifier: non-ref type)
movref  rD, rS            ; ref move: retain new, release old
i32add  rD, rA, rB        ; i32sub i32mul i64.. f32add f64.. (typed arith)
i32wrap rD, rA, rB        ; &+= family (RFC 0004 §3); plain `+=` traps
icmp    rD, rA, rB, cond  ; int/float compares → bool
jmp     L | br rC, L1, L2
brtable rIdx, table, n    ; `when` on enums, downcast chains
call    f, args -> rD     ; direct call (inherent fns only)
callm   f, rThis, args    ; direct method call
calli   slot, rRecv, args ; interface vtable call (RFC 0015 §6) — the ONLY
                          ; path for interface-declared members: always
                          ; dynamic, even when the receiver's exact class is
                          ; statically known (no devirtualization)
callnat slot, (rRecv,) args -> rD
                          ; host/extern member (RFC 0025/0026): slot indexes
                          ; the module's native table — assigned at .d.rut
                          ; compile, resolved at register, a constant at IR
                          ; time (fold/CSE-safe, same class as calli with a
                          ; known slot). Names never dispatch. The recv form
                          ; covers host methods (construction `new` slots
                          ; included); the body stays
                          ; opaque — never inlined. Also reaches the VM's
                          ; internal natives (§1.1 R2: `str`/`concat`/`tmpl`,
                          ; `Opaque` construction, Vec's named API) — slots fixed at
                          ; boot in the same registry (RFC 0022 §2)
ret     rD
newcell cid -> rD         ; mint a value cell (the `Self { .. }` / dataclass
                           ; literal — RFC 0009/0010; fields follow via setf);
                           ; also the `own(x)` body:
                           ; newcell + payload copy + handle-field retains
                           ; (class construction is a plain `call` of the
                           ; class method; host classes construct via
                           ; callnat on their native `new` slot — RFC 0026)
getf    rD, rO, fidx      ; field load (cell — payload layout known)
setf    rO, fidx, rV      ; field store (+retain/release where typed)
scopy   rD, rS, size      ; payload copy for `own` (memcpy + ref-field
                           ; retain/release)
tidof   rD, rO            ; read an object handle's runtime TypeId → u32
                          ; (Opaque box, dyn I object, slice cell — the
                          ; vtable ty load, RFC 0015 §6); a pure load
unbox   rD, rO, tid       ; extract an Opaque box's repr-C payload as the
                          ; statically known T (RFC 0014); traps on TypeId
                          ; mismatch — the compiler always guards (br on
                          ; tidof == tid first); the trap is the safety
                          ; net, like OOB
arrnew  rD, tid, rLen     ; Vec<T>(n) zeroed
arrget  rD, rO, rI | arrset rO, rI, rV   ; CONCRETE Vec<T> / Array<T, N>
                          ; element access — typed by elem tid, layout
                          ; known (flat inline elements for primitive T,
                           ; handle slots otherwise, RFC 0016 §4),
                          ; bounds trap. Array<T, N> const-index folds its
                          ; bounds check against const N (§1.1 R1); dyn
                          ; Slice<T> get/set/len are `calli` vtable slots
                          ; (§1.1 R2); Vec / Array → dyn Slice widening
                          ; lowers to the view-cell mint op
                          ; (RFC 0016 §4)
optsome rD, rV | optnone rD, tid | optis rD, rO ...
await   rD, rF            ; suspend point — state N (RFC 0018 §3)
spawn   rD, rF | cancel rT                  ; RFC 0019
chsend  rCh, rV | chrecv rD, rCh | chselect ...   ; RFC 0021
panic   code, rMsg
```

## 1.1 What is not an op

An op exists for exactly one of three things:

- **R1 — nothing static.** What the type table answers is a const-pool
  constant, never an op. `type_id<T>()` / `size_of<T>()` /
  `align_of<T>()` fold at compile time (RFC 0015 §3, RFC 0033 §3);
  `Array<T, N>.len()` is the const `N` — `N` is part of the type's
  identity (RFC 0005). Hence no `typeid`, no `arrlen`.
- **R2 — nothing polymorphic, nothing named.** A `dyn` receiver gets
  exactly one op: `calli` through its vtable. `dyn Slice<T>`'s `x[i]`
  get/set, `.len()`, `for..of` are the builtin `Slice<T>` impl's slots
  (RFC 0005; registered like any builtin impl, RFC 0022 §2) — ordinary
  vtable calls; the backing cell's dispatch-through-owner (RFC 0016 §4)
  is simply its slot target. And things rut spells with a **name** are
  internal natives, never ops: `str`/`concat` (the `f""` desugaring —
  RFC 0007 §2), Template construction (RFC 0027), `Opaque.new(v)`
  (RFC 0014), and concrete `Vec<T>`'s `len`/`push`/`pop` — native
  modules the VM boots with, in the same registry host modules use
  (RFC 0022 §2), reached by `callnat`. Hence no `strcat`, no `tmpl`.
- **R3 — concrete memory + control + the type machine's two readbacks.**
  Ops touch memory only through compile-time-known layouts (inline
  blocks, cells with known headers: `getf`/`setf`/`scopy`,
  `arrget`/`arrset`/`arrnew`), plus control flow and calls — and the
  two readbacks reification needs: `tidof` (what is it?) and `unbox`
  (the payload, guarded).

The named type operations lower to R3 primitives:

- `x is T` — the keyword (RFC 0012 §3). Concrete `T`: `tidof` + `icmp`
  — the exact "load the object's vtable `TypeId`, compare" of
  RFC 0015 §6. Interface `T`: the capability probe lowers to the one
  descriptor-scan op, `Op::IsIface { recv, want: TypeId const }` —
  `tidof`, then the descriptor's flat `implements` scan
  (RFC 0015 §6); pure in `(recv, want)`, so repeated probes CSE and
  invariant ones hoist like `tidof` itself. Statically-answered
  receivers never emit an op — typecheck folded them (RFC 0031 §2).
- `downcast<T>(a): Option<T>` (RFC 0014) is a **generic prelude
  function**: `tidof`; `br` on `icmp == TID_T`; `optsome(unbox)` on one
  arm, `optnone` on the other. The check is visible dataflow: `tidof`
  is a pure load, so repeated checks CSE and invariant ones hoist
  (LICM), and a chain of downcasts over one cell folds to one `tidof`
  + `brtable` (RFC 0031 §3) — RFC 0014's IR contract falls out of the
  primitives instead of being blocked by an opaque `downc` op. The
  symmetry is deliberate: `Opaque.new(v)` (erasure) is an internal-native
  call and `downcast` (recovery) is prelude code — neither is magic.

## 2. Suspend lowering

`suspend fn` compiles to a **state machine**: each `await` gets a state
number; the resume table maps state → block; locals live across suspension
points are promoted to frame slots that survive (RFC 0018 §4 sketches
`CoroutineFrame`). `spawn` wraps the machine in a `Task`; `cancel` drops
the frame at its suspension point (RFC 0019 §2).
