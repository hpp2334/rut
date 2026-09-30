# Typed bytecode

The compiler's output — and the VM's input — is **LIR**: a flat op stream
per function over typed registers. Every register has a static type
recorded in the function's signature, and the load-time verifier re-checks
all of it ([Module binary and verification](module-binary.md)). The op
definitions live in `rut-core/src/ops.rs`; the VM that executes them in
[VM core](vm-core.md).

## Registers and functions

Registers are `u16` indices into a per-frame `Vec<Slot>`; a register file
can never reach `u16::MAX`, which is reserved as the `NOREG` sentinel for
optional single-register operands (a call destination, a native receiver).
One function:

```rust
pub struct FuncCode {
    pub name: IdentId,
    pub params: Vec<TypeId>,   // methods: params[0] is the receiver
    pub ret: TypeId,
    pub is_method: bool,
    pub n_captures: u32,       // closure environment size
    pub regs: Vec<TypeId>,     // the register file's static types
    pub argv: Vec<Reg>,        // operand pool (below)
    pub labels: Vec<Label>,    // branch-table pool (below)
    pub code: Vec<Op>,         // the stream
    pub spans: Vec<(u32, u32)>,// pc -> source byte offset
    pub pos: Vec<(u32, u32)>,  // pc -> (line, col), parallel to spans
    pub host_id: Option<IdentId>, // Some = bodyless host fn (thunk)
}
```

`host_id` is the one name kind carried as an interner id: a host function
is a bodyless `FuncCode` whose `call` dispatches to the embedder's
registered body instead of interpreting ([Embedding](embedding.md)).

## Operand pools

No op owns a heap allocation. The variadic operand lists — call
arguments, record/array/capture element registers, branch-table arms —
live in **per-function pools** (`FuncCode::argv` for register lists,
`FuncCode::labels` for branch targets); each op carries an `(off, argc)`
span into its own function's pool. Consequences:

- `Op` is a fixed **24 bytes** — pinned by a compile-time assert, with a
  never-constructed `Op::Pad` variant holding the reasoning. The natural
  16-byte stride measured +13% on the interpreter gate (op-stream loads
  aliasing register-file stores at power-of-two strides); 24 measures
  clean and stays far narrower than the pointer-carrying layout it
  replaced.
- Pools are append-only and **deduplicated at emission** — repeated
  argument shapes share one entry. The empty list is the span `(0, 0)`
  and is never stored. Optimizer rewrites re-intern on register remap;
  deleted ops orphan their entries, which is dead weight, not corruption.
- `argc` is `u16`: a list can name at most as many registers as the
  register file holds. The emitter rejects literals beyond that bound.
- **Method calls fold the receiver into the pool as `argv[0]`**, so the
  span *is* the callee's parameter list and one uniform copy loop
  transfers a frame — no special-cased slot zero.
- The binary format serializes the pools ahead of the code; the verifier
  bounds-checks every span before execution.

Scalar ops are one opcode per operation — the kind (int vs float) is in
the opcode and the width in a `prim` operand, resolved by the compiler.
The VM never consults the type table for arithmetic and there is no
runtime op selector.

## The op families

Representative ops (exact Rust names; the table below is the whole
machine):

| family | ops | notes |
|---|---|---|
| moves | `Mov`, `MovRef` | ref move retains new, releases old |
| constants | `Const` (pool), `ConstRaw` (folded scalar bits) | |
| int arithmetic | `AddI` `SubI` `MulI` `DivI` `ModI` (+`prim`) | `+ - * / %` trap on overflow / divide-by-zero |
| wrap arithmetic | `WAddI` `WSubI` `WMulI`, `WrapShlI` | the `wrapping_*`/`wrapping_shl` builtin methods, lowered inline |
| float arithmetic | `AddF` `SubF` `MulF` `DivF` `ModF` `NegF` | |
| int bitwise | `AndI` `OrI` `XorI` `ShlI` `ShrI` | shifts mask the count; overflow traps |
| compares | `EqI`…`GeI` (+`prim`), `EqF`…`GeF` | bools and codepoints ride the int slots |
| equality on refs | `StrCmp` (content), `ArrayCmp` (content), `RefEq` (cell identity) | the `==` law's three arms |
| control | `Jmp`, `Br`, `BrTable` | `BrTable` arms in the `labels` pool |
| calls | `Call`, `CallM`, `CallI`, `CallFn`, `CallNat`, `Ret` | see below |
| records | `NewCell`, `MakeRecord`, `GetF`, `SetF` | `MakeRecord` allocates + initializes every field in one op; field operands bake the field's `Repr` |
| ownership | `Own` (payload copy; `bytes.clone()`'s lowering) | opcode 90 (`OnDrop`) is retired — cell-death code is the `Disposal` trait, dispatched by the release path, not an op ([the Rc heap](rc-heap.md)) |
| nullables | `MakeOpt` | `T -> ?T`: box into a one-slot cell (shares, never copies) |
| weak refs | `WeakNew`, `WeakUpgrade` | [Weak references](weak-refs.md) |
| arrays | `ArrNew` (zeroed), `ArrLit` (fixed), `ArrGet`/`ArrSet`, `ArrGetF`/`ArrSetF` (fused field+index) | bounds trap; element repr baked in |
| enums | `EnumNew` | immortal singleton cell per member |
| type machine | `TidOf`, `IsType`, `IsTrait`, `Unbox`, `Box` | the two readbacks + their guards |
| closures | `MakeClosure` | `{ func, captures }`, captures in the pool |
| panics | `Panic`, `Assert` | |
| conversion | `Conv` | `i32(x)` etc.; narrowing traps when the value does not fit |
| strings | `StrCodeAt` | one codepoint read as `u32`, bounds trap |
| budgets | `LoopHead` | fuel-check back-edge marker at loop heads |

Call ops:

| op | meaning |
|---|---|
| `Call { func, argv, dst }` | direct call — free functions, class construction, closure entries, and host thunks |
| `CallM { func, argv, dst }` | direct method call; receiver is `argv[0]` |
| `CallI { slot, argv, dst }` | trait vtable call — the only path for trait-declared members: always dynamic, even when the receiver's exact class is statically known |
| `CallFn { fval, argv, dst }` | call through an `fn`-typed value (closures: two consecutive registers, `{ fn_ptr, env }`) |
| `CallNat { nat, recv, argv, dst }` | internal native, `recv == NOREG` for free functions |

## Internal natives (`CallNat`)

Things rut spells with a **name** are natives, never ops. The fixed,
compiled-in table:

| native | spelling |
|---|---|
| `Str` | per-type formatting — the `f"..."` desugaring |
| `Concat` | `s.concat(parts...)` |
| `StrLen` / `ArrLen` | `s.len()` (codepoints) / array and bytes length |
| `StrJoin` | join an array of strings in one pass |
| `StrSlice` / `ArrSlice` | `slice(from, to)` — O(1) views ([String slicing and views](string-views.md)) |
| `BytesClone` | `bytes.clone()` — the one copy escape hatch |
| `CaptureTrace`, `TraceLen/Name/Line/Col/Render` | the stack-trace surface ([Diagnostics](diagnostics.md)) |
| `StrScan`, `StrStartsWith` | fused host-side scan/classify and prefix test |
| `StrFromCode` | `str.from_code(n)` — one codepoint to `str` |

Everything else rut spells with a name is ordinary rut code (`Vec`'s
methods) or a bodyless host function. There is no `strcat` op and no
template op.

## What is not an op

An op exists for exactly one of three reasons:

1. **Nothing static.** What the type table answers is a constant, never an
   op: `type_id<T>()` folds at compile time; `Array<T, N>.len()` *is* the
   constant `N` (`N` is part of the type's identity). Hence no `typeid`
   and no `arrlen` op.
2. **Nothing polymorphic, nothing named.** A trait-typed receiver gets
   exactly one op (`CallI`) through its vtable. Slice-view indexing and
   `len` are ordinary vtable calls through the view's builtin impl. The
   erasure primitive's names lower to primitives — the type-call
   `opaque(v)` to the
   `Box` op, `opaque.downcast<T>(o)` to prelude code (below) — and both
   are ambient: no `use` gates them ([opaque — erasure and
   downcast](opaque.md)).
3. **Concrete memory + control + the type machine's two readbacks.** Ops
   touch memory only through compile-time-known layouts (inline blocks,
   cells with known headers), plus control flow and calls — and the two
   readbacks reification needs: `TidOf` (what is it?) and `Unbox` (the
   payload, guarded).

### `is` and `downcast` lowering

- `x is T` with concrete `T`: one `TidOf` + an integer compare. On an
  erasure box, `is` answers **by the box** — it misses for every payload
  type; `downcast` is the only see-through.
- `x is I` with trait `I`: one `IsTrait` descriptor scan — pure in
  `(recv, want)`, so repeated probes CSE and invariant ones hoist.
- `opaque.downcast<T>(o)` (yielding `?T`): `TidOf`; branch on the
  compare against `T`'s id; `Unbox` on the hit arm, a nil box on the
  miss. The check is visible dataflow — the compiler CSEs repeated checks,
  hoists invariant ones, and folds a downcast chain over one cell into a
  single `TidOf` + `BrTable`. The compiler always guards `Unbox` with the
  branch; an unguarded one still verifies but traps on mismatch — the
  safety net, like a bounds check.

Statically-answered `is` expressions never emit an op — the checker folded
them ([The compiler pipeline](compiler.md)).

## Async lowering

`async fn` compiles to a state machine with **zero extra opcodes**:

- Each `await` is a checkpoint state — one enum-member-style singleton per
  suspension point, stored in the hidden frame's state field.
- Resume dispatch is the existing `BrTable` over that state.
- Locals live across suspension as cell-backed frame fields
  (`GetF`/`SetF`).
- The driven half is an ordinary `CallI` through the future's `yield`
  vtable row; suspension is a plain `Ret`.

The wire format is untouched by the async plan — bundles and binaries from
the same compiler version stay compatible. The driving loop that wakes
these frames is [VM core](vm-core.md)'s scheduler; the surface semantics
are [Async and await](async.md).

## Budget hooks in the stream

The only budget artifacts in the op stream are `LoopHead` markers: loop
back-edges are natural checkpoints where fuel is accounted
([Resource limits and fuel](resource-limits.md)). Everything else — fuel
countdown, heap checks at allocation, interrupt slices — lives in the
interpreter loop, not in the code.
