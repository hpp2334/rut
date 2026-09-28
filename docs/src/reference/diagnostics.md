# Diagnostics, traces, and symbolication

Errors are values and bugs are traps — either way, the first question is
*where*. The story has two halves: compile-time diagnostics (one model
from the first illegal byte to the last type error) and runtime traces
(raw captures, lazy symbolication).

## Compile-time diagnostics

One model serves every frontend and checking stage
([The frontend](frontend.md)):

```rust
struct Diag { span: Span, msg: String,
              labels: Vec<(Span, String)>, notes: Vec<String> }
```

Byte-offset spans; one renderer (`render_diags`) produces the caret form
— a primary line, secondary labels, notes — sorted by span so output is
stable across files and phases. The parser recovers at statement
boundaries and keeps going, so a file yields many diagnostics, not one.
Compilation stops before IR if any diagnostic exists; there is no
warnings-only lane.

A compile trap worth knowing: name-level misses that involve
[Dependency kinds](dependency-kinds.md) peers have dedicated diagnostics
(a missing optional peer's integration is never a bare
*unresolved name*).

## Runtime traces

### Capture — raw pcs, nothing else

`capture_stacktrace() -> StackTrace` (a builtin: the engine itself
implements it, compiler-lowered) walks the frame stack and stores raw
frames:

- the active frame innermost, then every saved frame outward, as
  `(func, call-site pc)` pairs — about 8 bytes per frame;
- **no name lookup, no source access, no symbolication at capture** —
  capture is cheap enough for any error path;
- saved-frame pcs are resume points, so symbolication backs one op to the
  call site;
- target-independent by construction — the same walk runs on wasm.

The result lives in a dedicated immutable trace cell: a snapshot, shared
by handle, safe to store in error tuples. It takes no user impls and
**never crosses the host boundary** — a trace is a rut-side diagnostic
object ([The host boundary](../core-concepts/host-boundary.md)).

```rut
builtin class StackTrace {
    fn len(self) -> i32;          // frame count (innermost first)
    fn name(self, i: i32) -> str; // frame i's function name
    fn line(self, i: i32) -> i32; // call-site line (0 when stripped)
    fn col(self, i: i32) -> i32;  // call-site column (0 when stripped)
    fn render(self) -> str;       // the symbolication string
}
```

Out-of-range `i` traps `IndexOutOfBounds` — the index is a bug, not data.

### The tiers

| tier | cost | when it exists |
|---|---|---|
| `StackTrace` capture | explicit; a depth-proportional frame walk (~30 ns base + ~0.2 ns/frame) | opt-in at raise sites — a rich err carries `trace: ?StackTrace` only where the producer decides the cost is worth it |
| `Trap` | automatic at trap unwind | bugs, overflow, `panic` — the loud channel carries `kind` + message |
| `?` / err propagation | zero cost — no auto-capture on propagation | always |

Opt-in is the design: the cheap error path stays cheap, and capture is
pay-per-capture.

## Symbolication — lazy, per index

Members symbolicate **on access**, against the loaded program:

- `name(i)` — one interner lookup for frame *i*'s function name.
- `line(i)` / `col(i)` — one direct read of the function's position table
  (pc → `(line, col)`, parallel to the pc → byte-offset span table;
  [Module binary and verification](module-binary.md)).
- `render()` — the only whole-trace pass, innermost first:

```text
at c_big (core:27:13)          # names + positions available
at c_big (core #3 @ pc 25)     # stripped: pc-only degradation
```

There is no eager symbolication and no frames array — a frame array would
mint rut-side data per access and fix the representation into the
surface.

### Restoration paths

A trace is captured against whatever is loaded — possibly a stripped,
published artifact. Three ways back to names:

1. **In-VM (the default).** The loaded program carries its interner and
   position tables; every member call symbolicates from them.
2. **Recompile by determinism.** Serialization is deterministic — same
   source + same compiler version ⇒ byte-identical binary ⇒ identical
   pcs — so tooling can symbolicate a captured trace against a locally
   rebuilt binary ([The compiler pipeline](compiler.md)).
3. **Stripped, no artifact.** Traces still *work*: they degrade to the
   pc-only render form above, and stay restorable via path 2 later.

## Trap messages

A trap renders as its kind plus message, e.g. `IndexOutOfBounds: index 9,
len 3`, `OutOfFuel: fuel exhausted — the frame is parked; add fuel and
resume()`. Traps do not carry captures — the trace surface is the opt-in
rut-side snapshot — but the host that receives `Err(Trap)` gets the kind,
the message, and (via `vm.fuel_used`, `heap_usage()`) the budget context
([VM core](vm-core.md)).

## Where the surfaces live

- `assert(cond, msg)`, `panic(msg)`, `on_drop(p, cleanup)` — the core
  builtins that raise or clean up
  ([The Rc heap and destructors](rc-heap.md)).
- `capture_stacktrace()` and the `StackTrace` class — ambient builtin
  names, no `use` required ([core and the swappable
  packages](stdlib.md)).
- The web runner surfaces the same data in its envelope: `"trap"` beside
  `"err"` ([Loading and the embed loop](loading.md)).
