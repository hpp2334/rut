# RFC 0007: IR & Bytecode

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0002 (types), RFC 0006 (AST), RFC 0003 (suspend), RFC 0005 (host fns)
- **Implementation side:** the VM that executes this code is RFC 0008;
  coroutine/heap op internals are sketched in RFC 5003/5004.

## Summary

Compilation is AOT, in-process, deterministic:

```
Ast ─► resolve ─► typecheck (bidirectional inference)
    ─► HIR: typed, SSA-ish, generic-free (monomorphization queue)
    ─► optimize: const-fold, inline, CSE/LICM, downcast-chain → BrTable
    ─► LIR: typed register bytecode  ─► verify ─► serialize (versioned)
```

There is no JIT and no deopt (RFC 0001 P7): whatever HIR proves is final.
The unit of compilation is the **module**; linking patches call targets at
load (RFC 0008 §6).

## 1. Resolve

Name resolution over the AST: paths → symbol ids, imports resolved through
the embedder's loader (RFC 0008 §6), `Self` bound, `private`/`export`
visibility checked (RFC 0002 §1.3), dataclass-vs-class distinction applied
(literals only for dataclasses; type-calls only for classes/builtins).

## 2. Typecheck

Bidirectional inference (RFC 0002 §4): expected types flow down, literal
types flow up; generic calls get their instantiation inferred or take
explicit args (`downcast<Point>(o)`). Output: every AST expression node is
decorated with a `TyId` (index into the module's type table) and generic
functions enter the **monomorphization queue** — HIR contains no generic
code.

## 3. HIR (typed, SSA-ish)

- One function at a time → `HirFunc { sig, blocks: Vec<HirBlock> }`;
  blocks end in exactly one terminator (`Ret`, `Jmp`, `Br`, `BrTable`,
  `Await` (RFC 0003 §2.1), `Trap`).
- Values are numbered `v0..` (SSA-ish: phis only where the frontend needs
  them — `when` expressions and short-circuit `&&`/`||`; mutable locals are
  explicit `SlotGet/SlotSet` on frame slots, NOT phi webs. Pragmatism over
  purity).
- **Type lattice per value** (RFC 5002 §4): `exact > interface > opaque`;
  `downcast.is_some()` branches refine back to exact. Optimizations run
  here: const-fold (incl. `type_id<T>()` / `size_of<T>()` — §7), inline
  (single-callee calls, small bodies), CSE/LICM for pure ops (downcast,
  field loads on immutable boxes), downcast chains → `BrTable` on `TypeId`.
- Closures capture by explicit `Capture` lists → a capture-struct class is
  synthesized (RFC 0002 §9); closures are values of `fn(..)` type backed by
  `{ fn_ptr, env: *const capture-struct }` pairs in two consecutive
  registers (RFC 5002 §1's inline-values rule).

## 4. LIR — typed register bytecode

Registers are `u16` indices into a per-frame `Vec<Slot>` (RFC 5002 §1);
every register has a **static type** recorded in the function signature —
the verifier re-checks all of it at load (§9). Inline values (dataclass /
bare class, RFC 0002 §10.2) occupy consecutive registers; `StructCopy` is a
`memcpy` of `size_of` bytes with compiler-emitted ref-field retain/release.

Representative ops (abbreviated; the full table is mechanical and lives
with the VM crate):

```text
mov     rD, rS            ; untyped move (verifier: non-ref type)
movref  rD, rS            ; ref move: retain new, release old
i32add  rD, rA, rB        ; i32sub i32mul i64.. f32add f64.. (typed arith)
i32wrap rD, rA, rB        ; &+= family (RFC 0002 §8); plain `+=` traps
icmp    rD, rA, rB, cond  ; int/float compares → bool
jmp     L | br rC, L1, L2
brtable rIdx, table, n    ; `when` on enums, downcast chains
call    f, args -> rD     ; direct (devirtualized) call
callm   f, rThis, args    ; direct method call
calli   slot, rRecv, args ; interface vtable call (RFC 5002 §2)
callh   hid, args -> rD   ; host/native fn (RFC 0005 §2)
ret     rD
ctor    cid, args -> rD   ; class type-call: run factory -> instance
newrc   cid, rV -> rD     ; Rc(v): mint cell (RFC 5004 §1)
getf    rD, rO, fidx      ; field load (inline value or Rc cell — layout known)
setf    rO, fidx, rV      ; field store (+retain/release where typed)
scopy   rD, rS, size      ; inline value copy (memcpy + ref fields)
is_a    rD, rO, tid       ; type test (RFC 5002 §3)
downc   rD, rO, tid       ; Opaque downcast → Option<T>
typeid  rD, tid           ; const-folded from type table (§7)
arrnew  rD, tid, rLen     ; Array<T>(n) zeroed
arrlen  rD, rO | arrget rD, rO, rI | arrset rO, rI, rV   ; typed by elem tid
strcat  rD, args          ; format-literal concat (RFC 0002 §4.1)
tmpl    rD, parts, args   ; Template construction (RFC 0005 §6)
optsome rD, rV | optnone rD, tid | optis rD, rO ...
await   rD, rF            ; suspend point — state N (RFC 0003 §2.1)
spawn   rD, rF | cancel rT
chsend  rCh, rV | chrecv rD, rCh | chselect ...
panic   code, rMsg
```

### 4.1 suspend lowering

`suspend fn` compiles to a **state machine**: each `await` gets a state
number; the resume table maps state → block; locals live across suspension
points are promoted to frame slots that survive (RFC 5003 §2 sketches
`CoroutineFrame`). `spawn` wraps the machine in a `Task`; `cancel` drops
the frame at its suspension point (RFC 0003 §3).

## 5. Module image

```rust
struct ModuleImage {                 // serialized, versioned, hash-stable
    version: u32, name: String,
    imports: Vec<(specifier, Vec<ImportedName>)>,   // resolved by loader
    types: Vec<RutType>,             // incl. field layouts + vtables
                                       (RFC 5002 §2, repr C — RFC 0005 §4)
    consts: Vec<Const>,              // strings, byte blobs, i64/f64, tids
    funcs: Vec<FuncImage>,           // name, signature (typed regs), code,
}                                    // state tables (suspend), host slots
```

- Deterministic serialization: same AST + same dependency versions →
  byte-identical image (cacheable by content hash; the embedder may ship
  `.rutc` artifacts instead of sources).
- `type_id`s are **module-local indices at rest**; link-time rebase maps
  them into the VM's global type table (RFC 0008 §6). `type_id<T>()`
  constants are re-based with everything else.

## 6. Const pool & const-expressions

RFC 0002 §1.1's const-expression rule is enforced here: module `const` and
`static` initializers are **folded at compile time**; the surviving forms
are literals, enum members, operators over consts, dataclass literals,
`Array<T>(n)`/`bytes(n)` blobs, and the layout builtins (§7). A call to a
user function in a const position is a compile error (OQ-11), not a
deferred-evaluation hack.

## 7. Layout & identity builtins

`type_id<T>()`, `size_of<T>()`, `align_of<T>()` (RFC 0002 §3.2) never
execute at runtime: HIR folds them to constants from the type table.
`type_id<T>()` values are `u32`, comparable, and unique per *instantiated*
type within a VM run (`Array<f32>` ≠ `Array<f64>`, `Point` = `Point`
across modules — identity is assigned at link). `size_of<T>()`/`align_of<T>()`
return the repr-C value size/alignment (RFC 0005 §4): for dataclasses and
classes this is the C-layout field block (what `Array<Point>` strides by,
what `StructCopy` copies, what a host struct mirrors).

## 8. Verification (load time)

The verifier re-checks, per function: register types vs op signature,
def-before-use, jump targets in-range and to block heads, `brtable`
density, factory `Self { .. }` completeness (every uninitialized field
covered — RFC 0002 §5.2), suspend
state tables closed under resume edges, host-slot signatures vs the
registered native fns (RFC 0005 §2). A failed verification is a load error
reporting the module and function — corrupted images never execute.

## 9. Optimization policy

Fixed pipeline, no flags in v1: resolve → infer → monomorphize → fold →
inline (budget: callee < N ops, single call site) → CSE/LICM (pure ops) →
code emit. Everything is measurable against the `examples/` corpus before
anything fancier is added; the no-JIT rule keeps the pipeline honest —
there is no tier-2 to fall back on, so LIR must be right the first time.

## Open questions

- OQ-1: phi-free HIR vs minimal-SSA — the pragmatic SlotGet/SlotSet choice
  may cost CSE precision; measure on the dashboard example before
  committing.
- OQ-2: cross-module inlining after linking (whole-program at embed time) —
  attractive for host-heavy code; needs image format support for re-emission.
- OQ-3: `brtable` on `f64` ranges (non-enum `when` with many int literals) —
  dense switch detection threshold.
- OQ-4: image compatibility across VM versions — `version` field exists;
  policy (refuse, recompile, or migration shims) undecided.
