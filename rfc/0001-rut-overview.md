# RFC 0001: rut — Overview & Pillar Decisions

- **Status:** Draft
- **Date:** 2026-08-21
- **Revised:** 2026-08-22 — RFC set restructured into a gradual series of
  small, single-topic RFCs (this file is the index).
- **Author:** hpp2334

## Summary

rut is a small, statically typed, embeddable scripting language with Rust-flavored
syntax, implemented in Rust, designed to be the scripting layer of host
applications (typically UI apps, e.g. tur). It replaces JS engines such as Boa
with a language whose type system is *not* erased: types exist at runtime, drive
bytecode specialization, and make the host boundary fully checked.

Syntax feels like TypeScript (`interface`/`class` implementing interfaces,
simple `enum`, arrow functions); the concurrency model feels like Kotlin
coroutines implemented with Rust's poll-based future semantics; the runtime is
a bytecode VM with **no JIT**.

## Motivation

tur currently embeds Boa to drive its reactive UI from script. The pain points
that motivate rut:

1. **Type erasure.** Host opaques (`Element`, `Source<T>`, `Store`) are empty TS
   interfaces; `T` exists only in `.d.ts` and is "recovered at the call site" by
   convention. Nothing at runtime prevents misuse; every bridge manually
   re-parses and re-coerces arguments (`as number`, `require_props_object`).
2. **Push-based suspend.** tur emulates poll-based cancellation on top of
   promises/generators (`launch` + `sleep` with driver-future drop). The
   emulation is correct but the substrate fights it: promise settlements,
   microtask queues, and downlevelled generators (tslib) add overhead and edge
   cases.
3. **Embedding friction.** Pinned engine revisions, NaN-boxing pointer-layout
   bugs per platform, GC rooting (`Gc<T>`) infecting all host code, no
   deterministic release of host resources.

rut is designed so that these three problems are *structural*, not incidental:
no erasure, native poll-based coroutines, host-owned values with deterministic
lifetimes.

## Why not JavaScript

The pain points above are tur-specific (Boa). The case against shipping JS
*at all* — the language-level rationale — is fourfold, and each item maps to
a structural answer in rut:

1. **History debt compounds.** Two nullish values (`null`/`undefined`),
   direct vs indirect `eval`, `typeof null == "object"`, `==` coercion
   rules, ASI — every one is a permanent spec liability the ecosystem
   re-implements forever. rut starts from a closed, small grammar: no
   nullish values (absence is `Option<T>`, RFC 0005), no `eval` and no
   dynamic code at all (declarations-only modules, RFC 0003), no coercing
   operators (RFC 0007 §1), and legacy JS constructs are reserved words
   whose errors say what to use instead (RFC 0002 §4). Nothing legacy can
   accrete: the grammar is small by charter.
2. **Spec conformance means implementing the world.** A conformant engine
   needs `BigInt`, `Intl`, `Date`'s quirks, microtask semantics — thousands
   of person-hours before any user code runs. rut's VM contract is
   deliberately tiny (primitives, `Option`/`Result`/`Vec`, channels);
   everything else is library: `Map`/`Set` are declaration-file + Rust
   library types (RFC 0028), and logging, IO, and the host's own domain
   live in their own modules (RFC 0022). A rut engine is small because the
   spec is small; an embedder ships only what its domain needs.
3. **Too slow without a JIT.** Interpreter-only JS runs one to two orders
   of magnitude slower — a JS engine's speed *is* its JIT. rut's no-JIT
   pillar (G7) is viable only because the language is statically typed:
   the bytecode is typed (G2), generics monomorphize, `Vec<f32>` is a
   flat `f32` buffer, and calls are direct by default — vtable dispatch
   exists only where an interface is named (RFC 0012 §1). The code shape a
   JS JIT exists to speculate on — polymorphic property loads — does not
   exist in rut.
4. **Too much memory.** JS values are boxes: per-object headers, tagged or
   NaN-boxed pointers, boxed array elements, GC mark/pause overhead. rut's
   memory story is layout: untagged 8-byte slots (RFC 0015 §5), inline
   repr-C values with no headers (RFC 0015 §4), flat unboxed vecs
   (RFC 0016 §4), reference counting with deterministic destructors
   (RFC 0016) and a budgeted cycle collector (RFC 0017). No NaN-boxing —
   the per-platform layout bug class tur hit in Boa — and no collector on
   the hot path.

## Goals

- G1 — Types are held at runtime (reified), not erased like TypeScript.
- G2 — Types make code faster: typed bytecode, typed register slots,
  monomorphized generics, unboxed `array<T>`.
- G3 — Primitive types: `u8/u16/u32/u64`, `i8/i16/i32/i64`, `f32/f64`, `bool`,
  `char`, `string`, `bytes`, `array<T>`, plus user `interface`/`class`/`enum`.
- G4 — Kotlin-style coroutine suspend built on Rust-style poll semantics
  (cold futures, state machines, cancellation-by-drop). Not promise push.
- G5 — Multi-threading via isolate workers (separate heaps, message passing).
- G6 — Simple memory management: reference counting + cycle collector.
  Deterministic destruction for host resources.
- G7 — No JIT. Ahead-of-time (to bytecode) compilation only. Predictable.
- G8 — Embeddable: host registers native modules with typed functions; host
  owns the event loop and steps the VM; interruptible with budgets.

## Non-goals

- No dynamic typing, no `any`, no gradual typing. Erasure is explicit and
  spelled out: `dyn I` interface objects (RFC 0012) for polymorphism,
  `dyn Any` boxes via `make_any`/`downcast` (RFC 0014) for storage —
  checked, recoverable, never silent.
- No structural ("duck") typing, no object literals — interfaces are
  nominal and declared (RFC 0012).
- No pattern matching beyond `when` literals/enums (RFC 0008), no
  data-carrying enums (RFC 0006), no union or intersection types;
  heterogeneous data goes through interfaces (RFC 0012).
- No JIT, no tiering, no runtime specialization beyond compile-time
  monomorphization and cheap VM-level caches.
- No shared-memory threads in the language. Workers communicate by message
  passing only (RFC 0021).
- No borrow checker, no lifetimes. Memory safety comes from the VM (RC), not
  from type-system proofs. There is no `box<T>`/loan construct: without
  exclusivity proofs loans cannot be sound (RFC 0004, RFC 0023).
- No C interop surface at first; host interop is the Rust embedding API only
  (Part E).

## Document layout — the series

The series is **gradual**: every RFC builds on the ones before it, one topic
each, in reading order. rut example code lives in `examples/`, referenced by
path; the RFCs hold prose and VM-side sketches (implementation sketches are
final sections of the RFC they implement).

**Part A — Orientation**

- **0001 — this file**: overview, pillars, execution model, roadmap, index.

**Part B — Language surface** (the script author's view)

- 0002 — lexical structure: source model, identifiers (`$`), naming rules
- 0003 — modules & visibility: declarations-only scope, `export` forms
- 0004 — primitive types & integer semantics: the by-value regime
- 0005 — builtin generic types: `Option`, `Result`, `Vec` (+ `Array<T, N>`
  fixed arrays, `dyn Slice<T>` slices)
- 0006 — enums: simple named-int sets
- 0007 — literals & inference: suffixes, conversions, plain/raw/format strings
- 0008 — control flow & `when`: exhaustive pattern expressions
- 0009 — dataclasses: open value records (methods + `implements`)
- 0010 — classes & factories: sealed value records, no inheritance
- 0011 — `Rc<T>`, dispose & identity: explicit references
- 0012 — interfaces & dispatch: the sole dynamic mechanism; `dyn I` object
  types; type tests
- 0013 — functions, closures & generics
- 0014 — `dyn Any`: explicit erasure with checked recovery
- 0015 — reified types & layout: `RutType`, repr C, slots & vtables

**Part C — Memory**

- 0016 — the RC heap & deterministic destructors
- 0017 — weak references & the cycle collector

**Part D — Concurrency**

- 0018 — `suspend` & `await`: poll-based coroutines
- 0019 — tasks: `spawn`, `cancel`, `select`
- 0020 — the host-futures bridge
- 0021 — workers & channels: isolate concurrency

**Part E — Host & FFI**

- 0022 — embedding model & native modules
- 0023 — the `Value` boundary & borrow guards
- 0024 — repr C struct interop
- 0025 — host classes & declaration files
- 0026 — generic host classes: a user-defined map
- 0027 — templates: `f"..."` across the boundary
- 0028 — the standard library: `rt:*`, `std:*`, `std:log`, `std:debug`

**Part F — Toolchain & artifacts**

- 0029 — declaration files & the DeclIr: `.d.rut`, `.d.ir`, publishing
- 0030 — frontend: lexer, parser, AST, diagnostics
- 0031 — compiler: resolve, typecheck, HIR
- 0032 — typed bytecode (LIR)
- 0033 — module image & verification
- 0034 — VM core: interpreter loop, traps, budgets
- 0035 — loading, host hooks & the embedding loop
- 0036 — diagnostics: stack traces, locations & symbolication

## Pillar decisions

| # | Decision | RFC |
|---|----------|-----|
| P1 | Fully static type system, **no dynamic typing**, inference-first, reified runtime types | 0004–0015 |
| P2 | Isolate workers with typed channels and transferable buffers | 0021 |
| P3 | Reference counting + cycle collector; deterministic destructors | 0016–0017 |
| P4 | `Result<T, E>` + `?` for recoverable errors; traps (panics) catchable only at the host boundary | 0005, 0033 |
| P5 | TS-like data model: `dataclass`/`class` (both **value types**; `Rc<T>` for explicit references; `factory` type-calls — no `new` keyword, `suspend factory` allowed), `interface` (object type `dyn I` — vtable dispatch, the sole dynamic-dispatch mechanism; implemented by class **and** dataclass; `requires` admission constraints), simple `enum`; no object literals, no data-enums, no intersections | 0006, 0009–0012 |
| P6 | Cold poll-based futures; `await` is the only suspension; cancellation drops the state machine at its suspension point | 0018–0020 |
| P7 | Register-based typed bytecode VM, no JIT; frontend lowers through an SSA-ish IR for folding/inlining before bytecode emission | 0029–0033 |
| P8 | Both value types (`dataclass` **and** `class`) are **repr C**; layout & identity builtins `type_id<T>()` / `size_of<T>()` / `align_of<T>()`; `box<T>` **rejected** — no borrow checker exists to make loans sound | 0015, 0024 |

### Why "no dynamic typing" is workable

Without `any`, JSON-shaped data becomes a small class set behind an
interface (`interface JsonValue` implemented by `JString`/`JNum`/`JArr`/…,
held as `dyn JsonValue` refs),
heterogeneous collections are `Vec<dyn I>` vecs, and host APIs that were
`any`-shaped in JS become generics or opaque `host class` handles. The one
dynamic mechanism kept is **interface dispatch** — a checked vtable call on a
value whose exact class is still known at runtime. What we keep from "types
held at runtime" (G1) is what the *VM and the host* need: every value's type
identity is available for checked type tests (`is<T>` in script, argument
checks at the FFI), for distinct runtime identities of instantiated generics
(`Source<i32>` ≠ `Source<string>` as opaque handle types), for debugging, and
for serialization.

## Execution model

```
source ─► lexer/parser ─► AST ─► resolver/typecheck ─► IR (SSA-ish)
      ─► const-fold / inline / monomorphize ─► typed bytecode ─► VM
```

- **Bytecode**: register-based, typed opcodes (`i32.add`, `f32.mul`,
  `arr.get<f32>`, `str.concat`, `call`, `await`…). Function signatures carry
  exact register types; the verifier re-checks them at load time.
- **Type stability is the optimization currency**: with no JIT there is no
  speculation or deopt — what the type checker proves is all the IR gets.
  The IR keeps a per-value type lattice (exact > `dyn I` > `dyn Any`) and
  the two lattice-lowering features (`dyn I` refs, `dyn Any` boxes) are explicit,
  boundary-local, and carry their own optimization rules (RFC 0031 §4).
- **No JIT**: all optimization happens at compile time. The VM may keep cheap
  inline caches for field access on host opaques, but nothing is ever compiled
  to machine code.
- **Single-threaded VM**: a `Vm` instance runs on one thread; workers are
  separate `Vm`s (RFC 0021). No atomics inside the heap.
- **Host stepping**: the host owns time. `vm.run_until_idle()`,
  `vm.poll(deadline)`, op budgets and interrupt callbacks keep the UI thread
  responsive and enable virtual clocks in tests (the tur test scheduler model).

## Roadmap

- **M0** — Lexer, parser, AST, pretty errors (RFC 0002, 0030).
- **M1** — Type checker + inference; bytecode + VM for the static core
  (no suspend, single module) (RFC 0029, 0031–0034).
- **M2** — Rust embedding API: modules, typed native fns, opaque types,
  `Result` mapping, interrupts/budgets, repr C struct registration
  (RFC 0022–0028).
- **M3** — Coroutines: `suspend`/`await` state machines, host-driven executor,
  cancellation (RFC 0018–0020).
- **M4** — Workers, channels, transferables (RFC 0021).
- **M5** — Cycle collector, weak refs, finalization polish (RFC 0017).
- **M6** — Tooling: CLI runner, formatter, LSP, debugger protocol; pilot
  integration in tur behind a feature flag.

## Open questions

Tracked in the owning RFCs:

1. Default integer type for uncontextualized literals (`i32` vs `i64`) and
   overflow policy — RFC 0004.
2. Module/system semantics: URL-like specifiers vs paths; how the host
   intercepts loads — RFC 0003.
3. Whether workers may share read-only immutable data (e.g. string interning
   table) across isolates — RFC 0021.
