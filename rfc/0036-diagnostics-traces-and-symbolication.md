# RFC 0036: Diagnostics — Stack Traces, Locations & Symbolication

- **Status:** Draft — **REVISED (Sep 2026, err-channel)**: the capture
  is a **`builtin class`** (`capture_stacktrace() -> StackTrace`, the
  engine itself), superseding the host-fn + rut-wrapper design below
  (§2/§5/§6 describe the superseded shape; the amendment at the end of
  this RFC is the law). See also RFC 0025's amendment (the
  builtin-class row's second instance) and RFC 0023's amendment (a
  trace never crosses the host boundary).
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

- `debug.here() -> Location` — the source position of the call. The
  compiler folds it to a constant from the span table, exactly like
  `type_id<T>()` (RFC 0015 §3, RFC 0033 §3). Free.
- `debug.capture_stack_trace() -> Opaque` — explicit capture (the
  JS `Error.captureStackTrace` role): a host fn that **skips its own
  frame** (the `constructorOpt` behavior, automatic).
- `?` stays zero-cost — no auto-capture on propagation (RFC 0018's
  cheap error path). Opt-in sugar is OQ-3, not v1.

## 2. Capture — `RawTrace` is raw pcs, nothing else

```rust
struct RawTrace { frames: Vec<RawFrame> }          // innermost first
enum RawFrame {
    Rut    { module: ModuleId, func: u32, pc: u32, state: u8 }, // state = async resume point
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
- `StackTrace` — the value rut code holds — is a **rut wrapper class**
  (`debug` source, RFC 0025 revised) over an `Opaque` handle
  around a `RawTrace`: the handle is an rc-managed boxed host value
  (RFC 0016 §3), immutable, safe to store in error dataclasses.
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
    name: Option<str>,          // None when names stripped
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
use debug::{ here, capture_stack_trace, Location, StackTrace };

dataclass LoadError {
    msg: str,
    at:    Location,               // free — folded at compile time
    trace: Option<StackTrace>,     // paid only when asked for
}
```

- `trace.render() -> str` — native method; renders via the loaded
  binaries' tables, degrades gracefully when stripped.
- `Location` is a plain dataclass `{ file: str, line: i32, col: i32 }`
  — printable, no host state; crosses the `Value` boundary
  like any dataclass (RFC 0023). `==` on it is cell identity, like every
  composite (RFC 0012 §8) — compare fields if equality matters.

## 6. The `debug` package shape

Same declaration-file machinery as `plugin:my_map` (RFC 0025/0026/0029):

```rut
// std/debug.d.rut (excerpt)
pub host dataclass Location { file: str, line: i32, col: i32 }
pub host fn here() -> Location;                  // folded at compile time (RFC 0033 §3)
pub host fn str(v: Opaque) -> str;        // developer rendering (RFC 0007 §2)
pub host fn type_name(v: Opaque) -> str;  // debug type name
pub host fn capture_stack_trace() -> Opaque;      // every capture is a
pub host fn stack_trace_render(t: Opaque) -> str; // unique snapshot —
pub host fn stack_trace_depth(t: Opaque) -> i32;  // identity `==`, like
```                                              // every composite (RFC 0012 §4)

No `host class` (RFC 0025, revised): the trace is an `Opaque` handle
and `debug`'s rut source wraps it — `pub class StackTrace { h:
Opaque; .. }` whose `render`/`depth` forward to the host fns; `Location`
is a `host dataclass` (flat crossing data the host constructs).

Rust bodies live in the toolchain's std crate (RFC 0028, `debug`);
`VmCtx` gains `capture_trace()` (native API, RFC 0022 §3), and the
embedder surface gains `vm.symbolicate(&raw)` for hosts that log traces
themselves (RFC 0035 §3) — tur, for instance, forwards them to devtools.

## Amendment (Sep 2026, err-channel): `capture_stacktrace() -> StackTrace` — a `builtin class`, raw capture, lazy symbolication

Landed (err-channel phase 2, `docs/err-channel-report.md` §1–2). The
user ruling supersedes §2's wrapper paragraph, §5's sketch, and §6's
package shape: **the engine itself implements the trace** — the
`debug` host fns, the `Opaque` handle, and the rut-side wrapper class
are all gone from the design. The declared surface, in the toolchain's
decl file (`core.d.rut`) only:

```rut
builtin fn capture_stacktrace() -> StackTrace;
builtin class StackTrace {
    fn len(self) -> i32;         // frame count (innermost first)
    fn name(self, i: i32) -> str; // frame i's function name
    fn line(self, i: i32) -> i32; // call-site line (0 when stripped)
    fn col(self, i: i32) -> i32;  // call-site column (0 when stripped)
    fn render(self) -> str;       // the symbolication string (§3)
}
```

That is RFC 0025's `builtin class` row exactly — the engine,
compiler-lowered; nothing to bind; the decl is a pure signature
contract (its amendment records this as the row's second instance).
Not a `builtin primitive`: primitives are value types
(`str`/`bytes`/`opaque`); a trace is a stateful engine snapshot with
methods.

**The consequences of `builtin class` over the wrapper:** no
Opaque-box indirection; no `debug` host bodies to register (wasm32
gets it for free — there are no host fns to bind); member dispatch
through the engine's member contract. Capture lowers to `CallNat
CaptureTrace`; the trace lives in a `CellData::Trace` cell — immutable
snapshot, `own()` shares it by handle; it takes no user impls (the
contract is closed) and never crosses the host boundary
(`crosses_boundary` is false for it).

**Capture is §2's `RawTrace` walk, RAW — minus native markers in v1:**
the active frame innermost, then every saved frame outward, as
`(func, call-site pc)` pairs — 8 B/frame, no name lookup, no source
access, no symbolication at capture. Saved frame pcs are resume
points, so symbolication backs one op to the call site. Target-independent
by construction (the same walk runs on wasm32 — pinned byte-exact).
`Trap`'s own unwind capture is unchanged; this surface is the opt-in
rut-side snapshot.

**Members symbolicate lazily, per index, on access** — §3's three
restoration paths apply unchanged, through the loaded binaries' tables
(§4). Decided AGAINST a `frames` field: a frame array would mint
rut-side data per access, fix the representation into the surface, and
pay for symbolication eagerly. Stripped builds degrade to `0`
line/col. **The render format** (one whole-trace pass, innermost
first): `at c_big (core:27:13)` symbolicated; `at c_big (core #3 @ pc
25)` stripped. Out-of-range `i` traps `IndexOutOfBounds` (the index is
a bug, not data — the loud-boundary convention).

**Cost law:** capture is depth-proportional, pay-per-capture (~30 ns
base + ~0.2 ns/frame measured); members are pay-per-access (`len()`
O(1), one binary search per `name`/`line`/`col`); `render()` is the
only whole-trace pass. The `?`/err propagation path stays zero-cost —
§1's law stands. Opt-in at raise sites: a rich err carries
`trace: ?StackTrace` only where the producer decides the cost is worth
it (nil the default).

**Naming, reconciled:** `capture_stack_trace` (this RFC's old spelling,
and RFC 0028's) is superseded by the ruling's **`capture_stacktrace`**
— the declared surface above is the one law; the old spelling
diagnoses at any stale decl.

**Version:** the surface addition bumped the module format **VERSION 7
→ 8** (the declared-surface precedent; the func table also serializes
its `pos` table beside the spans, so stale v7 artifacts are rejected).

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
