# RFC 0032: Typed Bytecode (LIR)

- **Status:** Draft
- **Date:** 2026-08-22
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
call    f, args -> rD     ; direct (devirtualized) call
callm   f, rThis, args    ; direct method call
calli   slot, rRecv, args ; interface vtable call (RFC 0015 §6)
callnat slot, (rRecv,) args -> rD
                          ; host/extern member (RFC 0025/0026): slot indexes
                          ; the module's native table — assigned at .d.rut
                          ; compile, resolved at register, a constant at IR
                          ; time (fold/CSE-safe, same class as calli with a
                          ; known slot). Names never dispatch. The recv form
                          ; covers host methods & factories; the body stays
                          ; opaque — never inlined
ret     rD
ctor    cid, args -> rD   ; class type-call: run factory -> instance
                          ; (host classes construct via callnat — the native
                          ; factory is a slot, not a cid)
newrc   cid, rV -> rD     ; Rc(v): mint cell (RFC 0011)
getf    rD, rO, fidx      ; field load (inline value or Rc cell — layout known)
setf    rO, fidx, rV      ; field store (+retain/release where typed)
scopy   rD, rS, size      ; inline value copy (memcpy + ref fields)
is_a    rD, rO, tid       ; type test (RFC 0015 §6)
downc   rD, rO, tid       ; Opaque downcast → Option<T> (RFC 0014)
typeid  rD, tid           ; const-folded from type table (RFC 0033 §3)
arrnew  rD, tid, rLen     ; Array<T>(n) zeroed
arrlen  rD, rO | arrget rD, rO, rI | arrset rO, rI, rV   ; typed by elem tid
strcat  rD, args          ; format-literal concat (RFC 0007 §2)
tmpl    rD, parts, args   ; Template construction (RFC 0027)
optsome rD, rV | optnone rD, tid | optis rD, rO ...
await   rD, rF            ; suspend point — state N (RFC 0018 §3)
spawn   rD, rF | cancel rT                  ; RFC 0019
chsend  rCh, rV | chrecv rD, rCh | chselect ...   ; RFC 0021
panic   code, rMsg
```

## 2. Suspend lowering

`suspend fn` compiles to a **state machine**: each `await` gets a state
number; the resume table maps state → block; locals live across suspension
points are promoted to frame slots that survive (RFC 0018 §4 sketches
`CoroutineFrame`). `spawn` wraps the machine in a `Task`; `cancel` drops
the frame at its suspension point (RFC 0019 §2).
