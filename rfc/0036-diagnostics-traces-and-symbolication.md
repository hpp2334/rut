# RFC 0036: Diagnostics — Stack Traces, Locations & Symbolication

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0034 (traps, frames), RFC 0033 (binaries, symbols),
  RFC 0029 (DeclIr — and what it is *not*), RFC 0028 (std modules),
  RFC 0022–0023 (`VmCtx`, native fns)
- **Part:** F — Toolchain & artifacts

## Summary

Errors are values and bugs are traps (RFC 0001 P4) — either way, the
first question is *where*. Three tiers, three price points:

| tier | cost | when it exists |
|---|---|---|
| `Location` (file:line:col) | compile-time constant, zero runtime | attached to error values at construction |
| `StackTrace` (raw capture) | explicit; a walk of the frame stack | anyhow-style context on `Result` errors |
| `Trap` backtrace | automatic at trap unwind | bugs/overflow/`panic` — the P4 boundary |

And one restoration question: a trace is captured as **raw pcs** against
whatever is loaded — possibly a stripped, published binary (RFC 0029 §6).
Restoration goes through the binary's **SymbolTable** (§4), never through
`.d.ir`: a DeclIr is the declaration *surface* (exports/types/slots) and
an unstable cache (RFC 0029 §4); it has no function symbols and no
pc→span data, and never participates in trace restoration.

## 1. The three tiers

- `std:debug.here() -> Location` — the source position of the call. The
  compiler folds it to a constant from the span table, exactly like
  `type_id<T>()` (RFC 0015 §3, RFC 0033 §3). Free.
- `std:debug.capture_stack_trace() -> StackTrace` — explicit capture (the
  JS `Error.captureStackTrace` role): a host fn that **skips its own
  frame** (the `constructorOpt` behavior, automatic).
- `?` stays zero-cost — no auto-capture on propagation (RFC 0018's
  cheap error path). Opt-in sugar is OQ-3, not v1.

## 2. Capture — `RawTrace` is raw pcs, nothing else

```rust
struct RawTrace { frames: Vec<RawFrame> }          // innermost first
enum RawFrame {
    Rut    { module: ModuleId, func: u32, pc: u32, state: u8 }, // state = suspend resume point
    Native { module: ModuleId, slot: u32 },        // boundary marker
}
```

- Capture walks `vm.frames` (RFC 0034 §1): no formatting, no name
  lookup, no source access. Cheap enough for any error path.
- **Native marker frames**: the VM pushes a one-word `Native { module,
  slot }` marker around every `callnat` (RFC 0032 §1, RFC 0034 §1), so
  traces read `at plugin:my_map.MyMap.set (host)` and re-entrant chains
  are followable. Rust-*internal* frames are not captured — embedder
  bugs are `RUST_BACKTRACE` territory; rut traces cover rut frames plus
  the native boundary.
- `StackTrace` — the value rut code holds — is a host-class handle
  around a `RawTrace`: an rc-managed boxed host value (RFC 0016 §3),
  immutable, safe to store in error dataclasses.
- `Trap` (RFC 0034 §2) carries `trace: RawTrace` captured at unwind;
  rendering is lazy — formatting happens only if someone prints it.

## 3. Symbolication — three restoration paths

1. **In-VM (dev default).** `vm.symbolicate(&raw) -> Vec<TraceEntry>`
   reads names + pc→span tables from the *loaded binaries*
   (`TraceEntry { module, func, span: Option<SourceSpan> }`).
2. **Sidecar map (support builds).** `rutc build --map` serializes the
   binary's `SymbolTable` as `mod.rutc.map` next to the binary, keyed by
   the binary's **content hash** (RFC 0033 §1). A **stable, versioned
   format contract** — the deliberate opposite of `.d.ir` (an unstable
   DeclIr cache; RFC 0029 §4). `rutc trace --map mod.rutc.map < raw.txt`
   restores a trace captured against a fully-stripped `--release`
   binary: the DWARF-separate-debug-file role. A `.rutbundle` carries the
   sidecar inside the zip, and `vm.symbolicate`'s fallback reads it from
   the mount (RFC 0038 §5).
3. **Recompile-by-determinism (no artifact at all).** Deterministic
   serialization (RFC 0033 §1) means same source + pinned compiler
   version ⇒ byte-identical binary ⇒ identical pcs ⇒ symbolicate against
   a locally rebuilt binary. Support tooling can rely on this instead of
   shipping maps.

Stripped traces still *work* — they degrade to
`pkg:mod #[3] @ pc 41` — and are restorable via path 2 or 3.

## 4. The `SymbolTable` — binary-side

The name "symbol table" belongs here (the ELF `.symtab` role), never to
`.d.ir` (RFC 0029's naming note):

```rust
struct SymbolTable {               // in ModuleBinary (RFC 0033 §1);
    funcs: Vec<FuncSym>,           //  == the .rutc.map sidecar payload
}
struct FuncSym {
    name: Option<string>,          // None when names stripped
    spans: SpanTable,              // sorted [(pc_start, pc_end) -> (byte_off, line, col)]
}
```

- Flat interval table + binary search. No VLQ/base64 — rut owns both
  ends of the format (standard SourceMap JSON export is OQ-4, for web
  tooling only).
- Strip levels (aligned with RFC 0033 §1 / RFC 0029 §6):
  default keeps names + spans; `--strip-native-names` drops name
  strings; `--release` drops spans too.

## 5. Error handling integration (rut side)

```rut
import { here, capture_stack_trace, Location, StackTrace } from "std:debug";

dataclass LoadError {
    msg:   string,
    at:    Location,               // free — folded at compile time
    trace: Option<StackTrace>,     // paid only when asked for
}
```

- `trace.render() -> string` — native method; renders via the loaded
  binaries' tables, degrades gracefully when stripped.
- `Location` is a plain dataclass `{ file: string, line: i32, col: i32 }`
  — printable, no host state; crosses the `Value` boundary
  like any dataclass (RFC 0023). `==` on it is cell identity, like every
  composite (RFC 0012 §4) — compare fields if equality matters.

## 6. `std:debug` module shape

Same declaration-file machinery as `plugin:my_map` (RFC 0025/0026/0029):

```rut
// std/debug.d.rut (excerpt)
pub dataclass Location { file: string, line: i32, col: i32 }
pub host fn here() -> Location;                  // folded at compile time (RFC 0033 §3)
pub host fn str(v: Opaque) -> string;        // developer rendering (RFC 0007 §2)
pub host fn type_name(v: Opaque) -> string;  // debug type name
pub host fn capture_stack_trace() -> StackTrace;
pub host class StackTrace {
                                             // every capture is a unique
                                             // snapshot — identity `==`,
                                             // like every composite
                                             // (RFC 0012 §4)
    fn render() -> string;
    fn depth() -> i32;
}
```

Rust bodies live in the rut crate's std module (RFC 0028, `std:debug`);
`VmCtx` gains `capture_trace()` (native API, RFC 0022 §3), and the
embedder surface gains `vm.symbolicate(&raw)` for hosts that log traces
themselves (RFC 0035 §3) — tur, for instance, forwards them to devtools.

## Open questions

- OQ-1: skip/filter arguments for `capture_stack_trace` (v1: skip-self
  only)?
- OQ-2: `TaskHandle.trace()` — trace a *parked* coroutine (stuck-await
  debugging); needs a per-task frame retention policy.
- OQ-3: opt-in `?`-sugar that attaches `here()` automatically (`?at`)?
- OQ-4: standard SourceMap JSON export (web tooling interop only).
- OQ-5: inlining vs spans — v1 binaries carry no cross-fn inlined code
  (RFC 0031 OQ-2); if whole-program inlining lands, spans need inline
  frame chains (the DWARF `DW_AT_inlined_subroutine` analog).
- OQ-6: serialized `RawTrace` schema versioning for crash reports.
