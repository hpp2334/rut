# Design goals

rut is a small, statically typed, embeddable scripting language with
Rust-flavored syntax, implemented in Rust. It is designed to be the
scripting layer of host applications — UI apps in particular — in the
role a JavaScript engine usually plays: a safe, hot-reloadable language
that drives the host's object graph.

The difference is structural, not cosmetic. In an embedded JS engine the
host's types are erased (`Element`, `Store` are empty interfaces; the
real types exist only in `.d.ts` files and get "recovered" by
convention), cancellation is emulated on top of promises, and every
value crossing the bridge is re-parsed and re-coerced by hand. rut is
built so those three problems cannot arise: **types are kept at
runtime**, **coroutines are natively poll-based**, and **host values
have deterministic lifetimes**.

## The pillars

**Fully static, reified types.** There is no dynamic typing and no
gradual typing. Every value's exact type is known to the compiler and
carried at runtime — type tests, checked erasure, and host-boundary
checks all read the same runtime truth. See [reified types and
layouts](reified-types.md).

**Types make code faster.** The bytecode is typed, generics
monomorphize, and primitive buffers stay flat: a fixed array of `f32` is
a packed `f32` buffer, not an array of boxed numbers. There is no JIT
and none is needed — the code shape a JS JIT exists to speculate on
(polymorphic property loads) does not exist in rut.

**Everything is a value — with two honest regimes.** Primitives move by
value; every composite (`str`, `bytes`, records, arrays, interface objects,
erasure boxes) is a reference-counted cell whose handle copies in O(1).
Sharing is the default and visible; there is no hidden copying. See
[everything is a value](everything-is-a-value.md).

**Poll-based coroutines.** `async`/`await` compiles to cold state
machines with Rust-style poll semantics: a future that nobody drives
costs nothing, and cancellation drops the state machine at its
suspension point. No promises, no microtask queue. See [the async
model](async-model.md).

**Reference counting; no collector.** Objects die the moment their last
reference goes away, so destructors are deterministic — host resources
(threads, sockets, textures) release at a knowable point, not "sometime
at GC". Strong cycles leak by design; weak references are the first-class
answer. See [memory](memory.md).

**No JIT.** All optimization happens ahead of time: fold, inline,
monomorphize, emit typed bytecode. What you run is what you compiled —
predictable startup, predictable per-op cost, no tiering.

**Embeddable by contract.** The host registers native functions whose
Rust signatures *are* the declared surface; every crossing is type-checked
against reified types, and the host owns the event loop, the fuel
budget, and the clock. See [the host boundary](host-boundary.md) and
[the bytecode VM](the-vm.md).

## Why not JavaScript

The case against shipping a JS engine — not just this or that
implementation — is fourfold, and each item has a structural answer:

1. **History debt compounds.** Two nullish values, coercing `==`,
   `typeof null`, implicit semicolons: each is a permanent liability the
   ecosystem re-implements forever. rut starts from a closed, small
   grammar — absence is `nil` on a nullable type, errors are values, and
   the handful of reserved words each name what to write instead.
   Nothing legacy can accrete.

2. **Conformance means implementing the world.** A compliant engine
   needs `BigInt`, `Intl`, `Date`'s quirks, microtask ordering —
   thousands of person-hours before user code runs. rut's built-in
   contract is deliberately tiny: primitives, sequences, channels,
   string building. Everything else is library — maps and sets, JSON,
   logging, and the host's own domain are ordinary packages, several of
   them written *in rut* and swappable.

3. **Too slow without a JIT.** Interpreter-only JS runs one to two
   orders of magnitude slower, because a JS engine's speed *is* its JIT.
   rut's no-JIT stance is viable only because the language is statically
   typed: calls bind statically whenever the call site names one
   concrete type, and dynamic dispatch is a checked vtable hop that
   happens only where the program actually erases types.

4. **Too much memory.** JS values are boxes — per-object headers, tagged
   pointers, boxed array elements, collector overhead. rut's answer is
   layout: untagged 8-byte slots, record payloads as slot arrays, flat
   primitive buffers, and reference counting with no collector pass on
   the hot path.

## What "no dynamic typing" leaves you

Erasure is never silent — it is spelled and checked:

- **Interface-typed values** (`s: Drawable`) for polymorphism. Dispatch is
  dynamic only where a value may be one of several concrete types, and an
  interface-typed value still carries its exact class at runtime.
- **The `opaque` box** (`opaque(v)`, `opaque.downcast<T>(o)`) for
  storage. Erasure mints a checked box; recovery checks the runtime type
  and yields `nil` on a mismatch — never a silent wrong-type read.

JSON-shaped data, heterogeneous collections, and `any`-shaped host APIs
all route through these two doors, which is what keeps "every value has
a runtime type" true without giving up dynamism where it earns its keep.

## The execution model

```text
source ──► lexer/parser ──► typecheck ──► IR ──► typed bytecode ──► VM
                                   (fold, inline, monomorphize)
```

- **One thread per VM.** A `Vm` instance runs on one thread with one
  heap and no atomics; workers are separate VMs communicating by typed
  message passing.
- **The host owns time.** The VM exposes stepping verbs — drain the
  ready queue, expire timers, poll with a deadline — plus op budgets and
  interrupt callbacks. A UI host drives script between frames and stays
  responsive; a test host virtualizes the clock and gets fully
  deterministic schedules.
- **Traps stop at the boundary.** A panic in script (overflow,
  nil deref) unwinds as a `Trap` value to the host call —
  rut code never catches one. Bugs become host-visible errors with a
  script backtrace, not corrupted state.

## What this buys you

If you are **writing scripts**: assignments never deep-copy; cleanup
runs when the last reference dies; a miss is `nil`, a failure is a
returned `err` string, and a crash is a loud trap with a real stack
trace. If you are **embedding**: you declare your API once in a rut
declaration file, bind it with typed Rust closures, and the compiler
plus the load-time verifier reject every mismatched shape before any
script runs — there is no bridge boilerplate to maintain.

The rest of this section walks each pillar in depth; the tutorial shows
the day-to-day syntax, and the reference pins down exact rules.
