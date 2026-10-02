# The compiler pipeline

Compilation is AOT, in-process, and deterministic. There is no JIT and no
deoptimization: whatever the checker proves is final, and the emitted
bytecode is the whole story.

```
source ─► lex/parse (rut-lexer, rut-parser)      [The frontend](frontend.md)
      ─► resolve + typecheck (rut-lir/check)
      ─► monomorphize + compile bodies (rut-lir/lir)
      ─► optimize (fold, inline, CSE/LICM, peephole, SROA)
      ─► FuncCode — typed register bytecode       [Typed bytecode](typed-bytecode.md)
      ─► link + flatten (rut-core/link)           [Loading](loading.md)
      ─► encode (rut-core/binary)                 [Module binary](module-binary.md)
```

The unit of compilation is the module. A `.d.rut` declaration surface runs
through resolve/typecheck and stops there — it has no bodies, so it
publishes a signature surface, never code.

## Stages

| stage | crate | in → out | notes |
|---|---|---|---|
| lex + parse | `rut-lexer`, `rut-parser` | source → AST + diags | flat arena, no recursion |
| collect | `rut-lir/check/collect` | AST → types, traits, impls | per-module symbol tables |
| resolve | `rut-lir/check/resolve` | paths → symbols | imports, `Self`, visibility, use-path routing |
| typecheck | `rut-lir/check` (`Ctx`) | expressions → `TyId`s | bidirectional inference, fused with body compilation |
| monomorphize | `rut-lir/check/inst` | generic calls → instantiations | a work queue; HIR contains no generic code |
| compile bodies | `rut-lir/lir` (`FnCompiler`) | instantiations → `FuncCode` | one function at a time |
| optimize | `rut-lir/lir` (`peephole`, `sroa`, …) | `FuncCode` → `FuncCode` | fixed pipeline, no flags |
| link + flatten | `rut-core/link` | modules → one `Program` | type-id rebase, duplicate-impl check |
| encode | `rut-core/binary` | `Program` → bytes | versioned, little-endian, hash-stable |

The middle stages are deliberately **one crate**: the checker's
monomorphization queue drives the body compiler and the body compiler
reports new instantiations back through the checking context. The fusion
keeps instantiation admission exact — every substitution-completing call
site is checked against its inline `requires` bounds as it is compiled.

## Resolve

Name resolution walks the AST and binds every path to a symbol:

- `use` imports resolve against the mounted session — exact,
  single-step: a use path resolves only if a module with that name is
  mounted. Missing modules diagnose against the consumer manifest (see
  [Loading](loading.md) and [Dependency kinds](dependency-kinds.md)).
- `Self` binds inside impls; `pub` visibility is checked per
  [Modules and visibility](modules-and-visibility.md).
- Struct-vs-class is decided here: literals are legal only for structs;
  classes construct through their class methods.
- `is` expressions resolve their right-hand side to a concrete type or a
  trait instantiation id; `is` on an erasure-typed receiver answers by the
  box (see [opaque — erasure and downcast](opaque.md)).
- `host`/`extern` surface references resolve against the declaration
  surfaces of the packages the module imports, with slot ids attached to
  every member reference.

## Typecheck

Bidirectional inference: expected types flow down, literal types flow up.
Every expression node is decorated with a type id (an index into the
module's type table). Generic calls get their instantiation inferred or
take it explicitly (`downcast<Point>(o)`).

Laws enforced here:

- **The `==` law** — primitives and `str` compare by value; every other
  cell type compares by identity; nullable `?T` operands and tuple
  operands are a compile error (*pattern-match instead* — destructure the
  pair). A lint flags `==` between two obviously fresh composites.
- **`is` folding** — when the receiver's static type already answers the
  question (a concrete receiver, `d: I is I`), the expression folds to a
  constant and an always-true/false lint fires.
- **Union bounds** — a generic parameter's `requires T1 | T2` bound is
  checked at every site that completes the substitution
  ([Type aliases and union bounds](type-aliases.md)).
- **The crossing rule** — an `entry fn`'s published signature may only use
  types that cross the host boundary: primitives, `str`, `bytes`,
  `opaque`, `?T` over a crossing type (nil-flattened), and tuples of
  crossing types. A violation is a compile error, so a bad surface never
  reaches the embedder at load time ([The host boundary](../core-concepts/host-boundary.md)).
- **The orphan rule** — `impl Trait for Type` is legal only in a package
  that defines the trait or the type. Trait impls for foreign pairs are a
  compile-time rejection, not a link-time surprise
  ([Traits and dispatch](traits.md)).

## Monomorphization

Generic functions never reach a binary. The checker's instantiation queue
compiles one concrete copy per substitution; the queue closes because
instantiating a body can enqueue more. Instantiation names (`[i32]`,
`Vec<f32>`) are synthesized into the shared interner, and `type_id<T>()`
folds to a constant from the type table at this point — it never executes
at runtime.

The same law fixes WHERE an instantiation compiles: where the body
lives. `Vec<i64>` is one type program-wide — the declaring package owns
every instantiation of its generics, and consumers request them: a
consumer's spelling lays out the concrete shape and routes the bodies to
the declaring package's compile, so a linked library's
`make() -> Vec<i64>` and the consumer's own `Vec<i64>` are one row, and
the binary ships one copy of each instantiation's code.

That leaves `inline = true` for packages whose methods live on class
bodies (inherent impls cross no surface yet): the graph compiler splices
their source into every consumer instead of linking them (`ink`, `json`,
`nmapset`, `strbuild`, `async_host`, `http`, `pouch`). See
[Project structure and rut.jsonc](project-structure.md).

## The type lattice

In a no-JIT VM, compile-time type knowledge is the only knowledge. The IR
tracks a per-value lattice:

```
exact concrete  >  I (trait-typed, satisfies I)
```

- **Exact types** compile to direct calls and known layouts.
- **Trait-typed values** (`d: Drawable`) are unsized: the payload lives in
  a heap cell and the slot stores the cell handle. One indirect
  vtable call per multi-origin use; fields are inaccessible; no inlining
  without evidence. The cost is per-call, never per-field — and when a
  call's receiver is statically concrete (a sealed impl set, a monomorphic
  body), the call devirtualizes at body-compile time and the dispatch
  overhead is zero.
- **Erasure sits off the lattice.** The `opaque` primitive is reached only
  through the type-call `opaque(v)`, and `opaque.downcast<T>(o)` is the
  refinement:
  the successful branch re-enters the *exact* lattice position, so
  downstream code optimizes as if nothing was erased. Downcast checks are
  pure dataflow — repeated checks CSE, invariant ones hoist, and a chain
  over one cell folds to a type-id switch. What the compiler must **not**
  assume is the type inside a box at a given program point; speculative
  devirtualization through `opaque` is JIT behavior and does not exist.
- **The slice tier** parallels trait widening:
  `Array<T, N> | Vec<T> (concrete)  >  Slice<T> view (unsized)`.
  `N` is part of the type's identity; slices are never boxed or erased.

The intended program shape: exact types on the hot path, trait-typed
values where polymorphism is real, `opaque` only inside heterogeneous
storage. Lints flag `downcast` in loop bodies and erasure crossing
non-storage function boundaries.

## Optimization

A fixed pipeline with no flags:

1. **Folding** — constant arithmetic, `type_id<T>()`, `Array<T, N>.len()`
   → the constant `N` (with const-index bounds checks folded against it),
   statically-decided `is` probes, dead branches.
2. **Inlining** — single-callee calls and small bodies (a callee op-count
   budget). Cross-module inlining after linking is future work; v1 binaries
   carry no cross-function inlined code.
3. **CSE / LICM** over pure operations — type-id loads, downcast checks,
   field loads on immutable records.
4. **Peephole + SROA** — the rewriters re-intern operand pools on register
   remap, so pool sharing stays consistent (see
   [Typed bytecode](typed-bytecode.md)). SROA declines a record whose
   class implements `Disposal`: the cell's death is observable (dispose
   runs at refcount zero), so its mint is never deleted.
5. **Pattern lowering** — downcast chains become one type-id load plus a
   jump table; `when` on enums lowers to `brtable` over the member value.

The pipeline is honest by construction: there is no tier-2 to fall back
on, so the emitted code must be right the first time. Every gate measures
against the example programs and the benchmark corpus.

## Async lowering

`async fn` compiles to a state machine: each `await` is a checkpoint
state in the hidden frame, resume dispatch is the existing jump-table op,
locals become frame fields, and suspension is a plain return. The op set
grows zero rows for this — the driven half is an ordinary trait-vtable
call through the future's `yield` row. The full protocol lives in
[Async and await](async.md) and [launched futures](tasks.md).

## Determinism

Same source + same dependency versions + same compiler version ⇒
byte-identical output. Serialization is ordered and little-endian
everywhere, and the graph compiles dependencies in a fixed post-order
before flattening. This is what makes compile caches content-addressable,
bundles diffable, and trace restoration by recompilation possible
([Diagnostics, traces, and symbolication](diagnostics.md)).
