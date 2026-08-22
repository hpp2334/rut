# RFC 0031: Compiler — Resolve, Typecheck, HIR

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0030 (AST), RFC 0003 (visibility), RFC 0029
  (DeclIr), Part B (type rules), RFC 0001 (M1)
- **Supersedes:** RFC 0007 §1–3, §9 + RFC 5002 §4 (pre-restructure)
- **Part:** F — Toolchain & artifacts

## Summary

Compilation is AOT, in-process, deterministic:

```
Ast ─► resolve ─► typecheck (bidirectional inference)
    ─► HIR: typed, SSA-ish, generic-free (monomorphization queue)
    ─► optimize: const-fold, inline, CSE/LICM, downcast-chain → BrTable
    ─► LIR: typed register bytecode (RFC 0032)
```

There is no JIT and no deopt (RFC 0001 P7): whatever HIR proves is final.
The unit of compilation is the **module**; linking patches call targets at
load (RFC 0035 §1). A `.d.rut` compiles through resolve/typecheck too —
stopping before HIR, emitting a DeclIr (RFC 0029 §3).

## 1. Resolve

Name resolution over the AST: paths → symbol ids, imports resolved through
the surface pipeline (RFC 0029 §5 — source, `.d.ir`, or `.d.rut`; never
bodies), `Self` bound, `private`/`export` visibility checked (RFC 0003 §2),
dataclass-vs-class distinction applied (literals only for dataclasses;
type-calls only for classes/builtins, RFC 0009/0010), and surface
declarations (`host`/`extern`) resolved against their DeclIrs with
slot ids attached to every member reference. The resolver also enforces
**engine admission** (RFC 0037 §3): `std:reflect`'s structural symbols
(`reflect<T>`, `type_of`, `TypeInfo`, `FieldInfo`, `SumVariant`)
resolve only in modules declaring ≥1 `implements ReflectEngine` — the
diagnostic names the interface and the fix.

## 2. Typecheck

Bidirectional inference (RFC 0007 §1): expected types flow down, literal
types flow up; generic calls get their instantiation inferred or take
explicit args (`downcast<Point>(o)`). Output: every AST expression node is
decorated with a `TyId` (index into the module's type table) and generic
functions enter the **monomorphization queue** — HIR contains no generic
code (RFC 0013 §2). Instantiation admission for surface-generic types
(`MyMap<Canvas, ..>`) closes over the `requires` graph here (RFC 0025).

## 3. HIR (typed, SSA-ish)

- One function at a time → `HirFunc { sig, blocks: Vec<HirBlock> }`;
  blocks end in exactly one terminator (`Ret`, `Jmp`, `Br`, `BrTable`,
  `Await` (RFC 0018 §3), `Trap`).
- Values are numbered `v0..` (SSA-ish: phis only where the frontend needs
  them — `when` expressions and short-circuit `&&`/`||`; mutable locals are
  explicit `SlotGet/SlotSet` on frame slots, NOT phi webs. Pragmatism over
  purity).
- Optimizations run here: const-fold (incl. `type_id<T>()` /
  `size_of<T>()` — RFC 0015 §3, RFC 0033 §3), inline (single-callee calls,
  small bodies), CSE/LICM for pure ops (downcast, field loads on immutable
  boxes), downcast chains → `BrTable` on `TypeId`.
- Closures capture by explicit `Capture` lists → a capture-struct class is
  synthesized (RFC 0013 §1); closures are values of `fn(..)` type backed by
  `{ fn_ptr, env: *const capture-struct }` pairs in two consecutive
  registers (RFC 0015 §5's inline-values rule).
- Suspension lowering (state splitting at each `await`) is specified with
  the bytecode in RFC 0032 §3.

## 4. Type stability & optimization rules

In a **no-JIT** VM, compile-time type knowledge is the only knowledge:
there is no speculation and no deopt to recover what the type checker
doesn't prove. The IR therefore tracks a per-SSA-value **type lattice**:

```
exact concrete  >  dyn I (satisfies I)  >  dyn Any (erased)
```

(`dyn Any` is the interface tier's bottom — RFC 0012 §2, RFC 0014.)

Every `dyn` type is **unsized** — `dyn I` and `dyn Slice<T>` alike: the
payload lives in a heap cell and a `dyn`-typed slot stores the cell
handle. There is no `dyn`-specific sizedness error; every type position
admits every `dyn` type (RFC 0012 §2). The slice tier has its own boxing
edge, parallel to interface widening:

```
Array<T, N> (exact, sized value)  >  dyn Slice<T> (unsized object)
```

`N` is a constant expression, part of the type's identity
(*"array length must be a constant expression"* otherwise — RFC 0005);
`dyn Slice<T>` is **not** related to `dyn Any` (slices are not `Any`
objects; `make_any` rejects them, RFC 0014). Fixed arrays and slices
satisfy no interface bound — the same family as `Vec`/`Option`/`Result`
(RFC 0026 §4).

Rules that bound the cost of the two lattice-lowering features:

1. **Interface values** (`dyn I`, RFC 0012): one indirect call per use;
   fields inaccessible; callee unknown (no inlining without evidence). Cost is
   per-call, never per-field — the vtable makes it a single load+jump.
2. **`dyn Any` + `downcast`** (RFC 0014): erasure is a hole in the
   lattice, but a *scoped* one:
   - the `downcast` check is the refinement — the `is_some()` branch
     re-enters the **exact** lattice position (strictly more information
     than a `dyn I` value carries), so downstream code optimizes as if
     nothing was erased;
   - boxes are immutable ⇒ downcast is pure ⇒ **CSE** repeated checks,
     **LICM** invariant ones, cache results forever;
   - a chain of downcasts over the same cell **folds to a `TypeId` switch**
     — the compiler emits one jump table, not N checked boxes.
3. **What the compiler must NOT assume**: the type inside a box at a given
   program point. No speculative devirtualization through `dyn Any` (that
   is JIT behavior; rut has no deopt to fall back on).
4. **Guardrails are language-level**: primitives in columnar user stores
   never box (typed vecs); hot shared state uses direct `Rc<State<T>>`
   cells; lints flag `downcast` in loop bodies and `dyn Any` crossing
   non-storage function boundaries. The intended shape of a rut program:
   exact types on the hot path, `dyn I` where polymorphism is real,
   `dyn Any` only inside heterogeneous storage.

## 5. Optimization policy

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
  attractive for host-heavy code; needs image format support for re-emission
  (RFC 0033 OQ-2).
