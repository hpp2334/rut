# RFC 0001: rut — Overview & Pillar Decisions

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334

## Summary

rut is a small, statically typed, embeddable scripting language with Rust-flavored
syntax, implemented in Rust, designed to be the scripting layer of host
applications (typically UI apps, e.g. tur). It replaces JS engines such as Boa
with a language whose type system is *not* erased: types exist at runtime, drive
bytecode specialization, and make the host boundary fully checked.

Syntax feels like TypeScript (`interface`/`class` with single `extends`,
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
2. **Push-based async.** tur emulates poll-based cancellation on top of
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

## Goals

- G1 — Types are held at runtime (reified), not erased like TypeScript.
- G2 — Types make code faster: typed bytecode, typed register slots,
  monomorphized generics, unboxed `array<T>`.
- G3 — Primitive types: `u8/u16/u32/u64`, `i8/i16/i32/i64`, `f32/f64`, `bool`,
  `char`, `string`, `bytes`, `array<T>`, plus user `interface`/`class`/`enum`.
- G4 — Kotlin-style coroutine async built on Rust-style poll semantics
  (cold futures, state machines, cancellation-by-drop). Not promise push.
- G5 — Multi-threading via isolate workers (separate heaps, message passing).
- G6 — Simple memory management: reference counting + cycle collector.
  Deterministic destruction for host resources.
- G7 — No JIT. Ahead-of-time (to bytecode) compilation only. Predictable.
- G8 — Embeddable: host registers native modules with typed functions; host
  owns the event loop and steps the VM; interruptible with budgets.

## Non-goals

- No dynamic typing, no `any`/`dyn` values, no gradual typing, no structural
  ("duck") typing, no object literals — interfaces are nominal and declared.
- No pattern matching, no data-carrying enums, no union types; heterogeneous
  data goes through interfaces (RFC 0002 §7).
- No JIT, no tiering, no runtime specialization beyond compile-time
  monomorphization and cheap VM-level caches.
- No shared-memory threads in the language. Workers communicate by message
  passing only (RFC 0003).
- No borrow checker, no lifetimes. Memory safety comes from the VM (RC), not
  from type-system proofs.
- No C interop surface at first; host interop is the Rust embedding API only
  (RFC 0004).

## Pillar decisions

| # | Decision | RFC |
|---|----------|-----|
| P1 | Fully static type system, **no dynamic typing**, inference-first, reified runtime types | 0002 |
| P2 | Isolate workers with typed channels and transferable buffers | 0003 |
| P3 | Reference counting + cycle collector; deterministic destructors | 0004 |
| P4 | `Result<T, E>` + `?` for recoverable errors; traps (panics) catchable only at the host boundary | 0002 §3 |
| P5 | TS-like data model: `interface` (vtable dispatch — the sole dynamic-dispatch mechanism), `class` with single `extends`, simple `enum`; no object literals, no data-enums | 0002 |
| P6 | Cold poll-based futures; `await` is the only suspension; cancellation drops the state machine at its suspension point | 0003 |
| P7 | Register-based typed bytecode VM, no JIT; frontend lowers through an SSA-ish IR for folding/inlining before bytecode emission | 0001 §Execution model |

### Why "no dynamic typing" is workable

Without `any`/`dyn`, JSON-shaped data becomes a small class set behind an
interface (`interface JsonValue` implemented by `JString`/`JNum`/`JArr`/…),
heterogeneous collections are interface-typed arrays, and host APIs that were
`any`-shaped in JS become generics or opaque `extern class` handles. The one
dynamic mechanism kept is **interface dispatch** — a checked vtable call on a
value whose exact class is still known at runtime. What we keep from "types
held at runtime" (G1) is what the *VM and the host* need: every value's type
identity is available for checked casts across the FFI, for distinct runtime
identities of instantiated generics (`Source<i32>` ≠ `Source<string>` as
opaque handle types), for debugging, and for serialization.

## Execution model

```
source ─► lexer/parser ─► AST ─► resolver/typecheck ─► IR (SSA-ish)
      ─► const-fold / inline / monomorphize ─► typed bytecode ─► VM
```

- **Bytecode**: register-based, typed opcodes (`i32.add`, `f32.mul`,
  `arr.get<f32>`, `str.concat`, `call`, `await`…). Function signatures carry
  exact register types; the verifier re-checks them at load time.
- **No JIT**: all optimization happens at compile time. The VM may keep cheap
  inline caches for field access on host opaques, but nothing is ever compiled
  to machine code.
- **Single-threaded VM**: a `Vm` instance runs on one thread; workers are
  separate `Vm`s (RFC 0003). No atomics inside the heap.
- **Host stepping**: the host owns time. `vm.run_until_idle()`,
  `vm.poll(deadline)`, op budgets and interrupt callbacks keep the UI thread
  responsive and enable virtual clocks in tests (the tur test scheduler model).

## Roadmap

- **M0** — Lexer, parser, AST, pretty errors.
- **M1** — Type checker + inference; bytecode + VM for the static core
  (no async, single module).
- **M2** — Rust embedding API: modules, typed native fns, opaque types,
  `Result` mapping, interrupts/budgets (RFC 0004).
- **M3** — Coroutines: `async`/`await` state machines, host-driven executor,
  cancellation (RFC 0003).
- **M4** — Workers, channels, transferables (RFC 0003).
- **M5** — Cycle collector, weak refs, finalization polish (RFC 0004).
- **M6** — Tooling: CLI runner, formatter, LSP, debugger protocol; pilot
  integration in tur behind a feature flag.

## Open questions

1. Default integer type for uncontextualized literals (`i32` vs `i64`), and
   implicit widening ladder (RFC 0002 §3).
2. Integer overflow policy: trap always, or trap in debug / wrap in release
   with explicit wrapping intrinsics (RFC 0002 §4).
3. Standard-library container set (`map<K,V>`, `set<T>`) as builtins vs
   library types (RFC 0002 §6).
4. Module/system semantics: URL-like specifiers (`tur:core` today) vs paths;
   how the host intercepts loads (RFC 0004 §6).
5. Whether workers may share read-only immutable data (e.g. string interning
   table) across isolates (RFC 0003 §6).
