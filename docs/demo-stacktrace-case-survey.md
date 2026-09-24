# demo-stacktrace-case — phase 0: survey

- **Batch:** demo-stacktrace-case (phase 0 of 2, docs-only)
- **Base:** 0d569ed (`phase(2)`: the opaque-is record) — VERSION 11, the
  demo at 25 cases (8 inline + 17 classics), inline expectations,
  auto-run on select @1M fuel + settled-edit last-wins
- **Date:** 2026-09-24
- **Directive:** *"in demo, add a case of stack trace."* The demo
  teaches TODAY, and `capture_stacktrace() -> StackTrace` (err-channel
  phase 2, ed14b9b; VERSION 8 then, now 11) is today's surface with
  ZERO demo visibility — no case, no blurb, no expected row anywhere
  under `demo/` (grep: zero hits, §3.3).
- **Method note:** empirical. Every number below was measured through
  THE LANE THE DEMO RUNS — the fresh `target/wasm32-unknown-unknown/
  release/rut_wasm.wasm` (rebuilt from this tree today, byte-for-byte
  the artifact `npm run build:wasm` ships) driven over the raw ABI by
  scratch node probes, the same shape `crates/rut-wasm/smoke.js` uses.
  Scratch under `/tmp/opencode/batch-demo-stacktrace/p0/`, zero repo
  files touched (§Appendix). The landed wasm smoke was re-run at base
  first and passed byte-exact (`rt:54:13` pins live), so ed14b9b's
  claims were re-provable before anything here was measured.

---

## 1. The census — the StackTrace surface as shipped TODAY (VERSION 11)

### 1.1 The declared surface — `rut/core/core.d.rut:47–64`

```rut
builtin fn capture_stacktrace() -> StackTrace;

builtin class StackTrace {
    fn len(self) -> i32;                 // frame count (innermost first)
    fn name(self, i: i32) -> str;        // frame i's function name
    fn line(self, i: i32) -> i32;        // call-site line (0 when stripped)
    fn col(self, i: i32) -> i32;         // call-site column (0 when stripped)
    fn render(self) -> str;              // the RFC 0036 symbolication string
}
```

Five members, no constructor spelling, no frames field (rejected in
ed14b9b: an array would mint rut-side data per access and fix the
representation into the surface). A pure signature contract (RFC 0025's
`builtin class` row); `StackTrace` takes no user impls, and the trace
cell is shared by handle on assignment/passing (own() shares).

### 1.2 The engine — `crates/rut-vm/src/interp/native/trace.rs`

- **Capture is a RAW frame walk** (`nat_capture_trace`, :13–26): frame 0
  is the function executing the capture (its call site is the CallNat op
  itself), then every saved frame outward with pc backed one op to the
  resume point's call site. O(depth) pushes, one heap cell — no names,
  no source, no symbolication at capture. Opt-in and pay-per-capture:
  the err-propagation path stays zero-cost.
- **Members symbolicate LAZILY, per index**: `name` reads the program
  interner (:60–67); `line`/`col` read the FuncCode.pos table, parallel
  to the spans, one direct read (:72–94); `(0, 0)` when stripped.
- **Out-of-range is LOUD** (`trace_frame`, :40–50): `i < 0 || i >= len`
  traps `TrapKind::IndexOutOfBounds` with
  `"StackTrace index {i} out of range — len is {len}"` — the index is a
  bug, not data; never nil. A non-trace receiver also traps loud
  (wiring drift).
- **render()** (:100–119) is one whole-trace pass, innermost first,
  newline-joined:
  - symbolicated: `at {name} ({prog}:{line}:{col})`
  - stripped (no pos table): `at {name} ({prog} #{func} @ pc {pc})`

### 1.3 render's module name — the link law, measured both lanes

`render` prints `self.prog.name`, and `flatten`/`link` name the program
after `modules.first()` (`crates/rut-core/src/link.rs:67–70`). The
graph's programs vec is **post-order — dependencies precede their
users** (`crates/rut-driver/src/graph.rs:78–79`), so the name is the
deepest linked dep of the entry module:

- **the demo lane (wasm): `rt`** — measured, every probe in §Appendix
  (`at c_big (rt:54:13)`, `at main (rt:29:14)`, …). An ink-using case's
  first linked program is the `rt` decl surface ink itself binds
  (`rut-wasm/src/lib.rs:110–164` mounts core+calc as surfaces, rt and
  nmap_host as decls, ink inline, pouch linked, nmapset inline).
- the native core-only lane: `core` (the driver tests' pinned
  `at c_big (core:27:13)`, stack_trace.rs:167).

The case's expected must therefore pin **`rt:…`** — the demo lane's
truth — and the doc says so loudly: a future change to the demo's
mount set moves that name (the smoke re-proves it per run).

### 1.4 THE WASM LANE'S REAL OFFSETS at the demo's mount — +26, measured today

`ink` is an explicitly-inlined module: its TEXT splices above the user
source (`graph.rs:238–298` — the dep leaf, then the loop's closing
`"\n"`, then the unit's own source after one more `"\n"`). `ink.rut`
is **24 lines today**, so the user's line 1 is unit line 27:

```
offset = ink_lines + 2 seam newlines = 24 + 2 = 26      (today)
```

Measured, not assumed:

- **Probe F** (§Appendix): capture in `main` at user line 4 reads
  `line=30` (= 4 + 26), col 14, name `main`, len 1.
- **The landed wasm smoke re-run at base today**: ALL CHECKS PASSED,
  its byte-exact pins `at c_big (rt:54:13)` = user line 28 + 26 still
  hold on this tree.

Per-case dependence (the honest footnote the demo must carry): the
offset is **ink-only +26 for cases that use only ink**. A case adding
`use nmapset::…` would shift further (nmapset is also inline, 576
lines); `pouch` is a LINKED module (no text), core/calc/rt/nmap_host
are surfaces (no text). The stack-trace case uses ink only, so +26 is
its law — and its expected pins positions, which is what makes any
future mount/ink change LOUD in the demo smoke rather than silent.

### 1.5 The inline law, measured in the demo lane — the shape-deciding fact

The checker inlines callees with ≤24 top-level statements; a capture
inside an inlined callee reports the frame it LANDED in (ed14b9b's
inline-law pins, native). This is LIVE in the demo lane:

- **Probe A** — the directive's sketch verbatim (`a` → `b` → `c`, small
  bodies): **`depth: 1`**, `innermost: main at 29:14`, render
  `at main (rt:29:14)`. The whole chain checker-inlines into main; the
  sketch's `[c, b, a, main]` is unreachable with small bodies.
- **Probe D** — the same chain with each body padded past the budget
  (25 `let pN = N;` statements, the driver tests' own `pad(25)`
  recipe): **`depth: 4`**, names exact `[c, b, a, main]`, cols the
  callee idents (14 capture, 12 returns).

So a real-frame demo case MUST pad every callee body past 24
statements. Measured alternatives, both dead or worse:

- **Named fns are not first-class values** (Probe B1): `let g:
  fn() -> StackTrace = c;` fails to compile — ``unknown name `c` ``
  (the diag at the ident). The compact "indirect named chain" is not
  available on today's surface.
- **Anonymous fns keep real frames without padding** (Probe C): three
  lambdas bound to locals, all calls indirect → depth 4 — but the
  names symbolicate as the synthetic **`lambda@93` / `lambda@103` /
  `lambda@114`** (stable engine ids, not source positions; Probe C2
  shows the id surviving a source shift). The ORDER lesson survives,
  the NAMES lesson dies — and the synthetic names would teach a
  side-law the case does not want to carry.

**The call: the padded named chain.** The padding is not noise to hide
— it IS the inline law, taught: the case's header comment says why the
bodies are fat, and the fat pays for the readable `[c, b, a, main]`.
Size is no obstacle: the biggest classic today is `opaque.rut` at 102
lines; the frozen case is 107 (§2.1), and the editor scrolls inside
itself (H-2, landed).

### 1.6 The loud boundary, measured — plus one envelope quirk to disclose

- `st.name(9)` at any depth traps `IndexOutOfBounds` **before the line
  prints** (the f-string argument evaluates first); output streamed
  before the trap is intact (Probes E1/E2/E3; the frozen case's run).
  Negative indices trap the same way (`line(-1)` via a binding, E3).
- The demo panes show only the trap NAME — `Trap::IndexOutOfBounds` as
  the Output pane's last line (`verify.ts` renderedLines, Panes.tsx:33).
  The engine's detailed message (`StackTrace index 9 out of range —
  len is 4`) is NOT surfaced by the page; the case's comment carries it.
- **Envelope quirk, measured twice**: a non-fuel trap exits the
  threaded loop without the final fuel sync
  (`interp/threaded.rs` keeps `fuel_used` in ThreadState and syncs on
  park/return), so the run envelope's `fuelUsed` reads **0** for the
  trap-finale run while the same source without the trap reads 129.
  The status bar will show `fuel 0` on this case — cosmetic, real, not
  a phase-1 bug. Recorded here so nobody "fixes" it mid-batch.

### 1.7 The cost caption's provenance

"~30 ns a capture" is ed14b9b's measured micro-estimate (scratch
`/tmp/opencode/batch-err-channel/p2/capture-cost.md`, native lane,
min-of-5 release runs): ~30 ns at depth 1, ~45 at 10, ~50 at 100 —
one capture ≈ 30 ns base + ≈0.2 ns marginal per frame, trivial next to
the calls themselves (at depth 100 the padded chain costs ~380 ms; the
capture adds ~1.3%). The case's caption repeats the figure WITHOUT a
lane claim (it was measured natively; the wasm32 walk is the same
frame walk — the smoke proved behavioral identity, the ns figure is a
host-lane number and the comment does not pretend otherwise).

---

## 2. The case, finalized

### 2.1 The source — frozen bytes (107 lines, ASCII throughout)

Verbatim as phase 1 will paste it into `demo/src/cases.ts` (array of
lines joined `"\n"`, the house shape). Line numbers are LOAD-BEARING
(§2.6): the expected pins unit-relative positions through the +26
offset.

```rut
use ink::{Logger};

// RFC 0036: capture_stacktrace() is OPT-IN and cheap (a raw frame
// walk, ~30 ns at depth 1): no names, no source at capture; the
// members symbolicate lazily, per index, against the loaded program.
//
// THE INLINE LAW: the checker inlines callees with <= 24 statements,
// so each body below pads past that budget to stay a REAL frame --
// a small callee's capture reports the frame it landed in, not the
// source's shape.
fn c() -> StackTrace {
    let p0 = 0;
    ... p1..p23 exactly alike ...
    let p24 = 24;
    let st = capture_stacktrace();   // frame 0: the capture site
    return st;
}
fn b() -> StackTrace {
    ... the same 25 pad statements ...
    return c();                     // frame 1's call site
}
fn a() -> StackTrace {
    ... the same 25 pad statements ...
    return b();                     // frame 2's call site
}
pub fn main() {
    let log = Logger.new("case");
    let st = a();                   // frame 3's call site
    log.info(f"depth: {st.len()}");
    log.info(f"innermost: {st.name(0)} at {st.line(0)}:{st.col(0)}");
    log.info(f"outermost: {st.name(st.len() - 1)}");
    log.info(st.render());
    // the boundary is LOUD: index 9 is a bug, not data -- this traps
    // IndexOutOfBounds BEFORE the line can print (the run's last
    // line is the trap, and the expected below pins it).
    log.info(f"never prints: {st.name(9)}");
}
```

(The elided pad blocks are `    let p{i} = {i};` for i in 0..=24 — the
exact listing with 1-based line numbers is probe 3's output in
`/tmp/opencode/batch-demo-stacktrace/p0/`; phase 1 copies the case from
there or regenerates it and re-measures. The load-bearing positions:
capture at user line 37, `return c()` at 66, `return b()` at 94,
`let st = a();` at 98.)

Design notes, each a decision:

- **Nesting**: `main → a → b → c`, capture deep in `c` — the sketch's
  shape; depth 4 with `main` as frame 3, the OUTERMOST frame, so
  `st.name(st.len() - 1)` teaches index arithmetic against `len`.
- **Capture returns the trace cell** (shared by handle) and MAIN does
  the logging — the Logger stays `main`'s local (house shape) and the
  trace crosses a return by reference, which is itself the surface's
  value law (the prim/ref pins).
- **The trap finale is deliberate**: `st.name(9)` traps before the
  line can print, `Trap::IndexOutOfBounds` becomes the run's last
  line, and the expected PINS it — the loud boundary is a green row,
  not an error state (fuel-demo's precedent: expected carries
  `Trap::OutOfFuel`). The comment carries the engine's full message
  text, which the panes do not surface (§1.6).
- **The padding is the inline law, taught** — the header comment says
  exactly why the bodies are fat (§1.5's call).

### 2.2 The expected — measured from real runs, wasm-lane values, disclosed

Run through the demo lane at BOTH budgets (1M auto, 10M default):
identical output, `parked: false`, heap 706 B. The `expected: string[]`
for cases.ts — **the render row is ONE element with embedded newlines**
(§2.4):

```ts
expected: [
  "depth: 4",
  "innermost: c at 63:14",
  "outermost: main",
  "at c (rt:63:14)\nat b (rt:92:12)\nat a (rt:120:12)\nat main (rt:124:14)",
  "Trap::IndexOutOfBounds",
],
```

The position map (user line → unit line, offset +26, cols measured):

| frame | site                | user line | unit line | col |
|-------|---------------------|-----------|-----------|-----|
| 0 `c` | the capture         | 37        | 63        | 14  |
| 1 `b` | `return c();`       | 66        | 92        | 12  |
| 2 `a` | `return b();`       | 94        | 120       | 12  |
| 3 `main` | `let st = a();`  | 98        | 124       | 14  |

Cols are the callee idents (1-based): 14 for `let st = …` sites, 12
for `return …;` sites. These are **wasm-lane values and say so**: the
`rt:` module name and the +26 lines are the demo mount's splice truth
(§1.3–§1.4), not the native lane's (`core:` + 0). Disclosed in the
case itself is impossible without lying by omission — the disclosure
lives HERE and in the smoke (which re-proves the bytes every run), not
as a caption the page cannot keep current.

### 2.3 Placement — inline case in cases.ts, before fuel-demo

- **Inline-string, not a classic file.** The classics are the repo's
  gate files (`crates/rut-cli/tests/playground.rs` compiles each and
  byte-diffs the SAME expected natively) — a line-number-bearing case
  would pin TWO lanes' DIFFERENT offsets (`rt:+26` wasm vs `core:+0`
  native) in one table: unlandable. The inline world is exactly for
  demo-only teaching with demo-lane values. No `examples/*.rut`, no
  `playground.rs` row, no sidecar-shaped anything (§3.3).
- **Order**: between `closures-generics` and `fuel-demo` — the
  language-surface tours close with the diagnostics case, the trap
  case stays the finale. CaseList order follows the array.
- **Identity**: `id: "stack-trace"`, `name: "stack trace"`,
  `rfcs: "0036"`,
  `blurb: "the opt-in stack snapshot — raw frames innermost-first, lazy symbolication, the loud out-of-range trap"`.

### 2.4 The verify wrinkle — one expected row carries the render's four lines

The verifier diffs output ARRAY ELEMENTS (`verify.ts`:
`renderedLines(res)` vs `expected`, element-wise `===`); `render()`
rides ONE `log.info` as ONE string with embedded `\n`; the Output pane
joins elements with `"\n"` inside a `<pre>` (Panes.tsx:62), so the
element RENDERS as four visual lines and COMPARES as one. The expected
above is therefore byte-exact under the existing verifier — no
verify.ts change, no splitting helper. The line-paired diff (on a
mismatch) prints the multi-line row raw — acceptable; rows are
strings, and this row only mismatches when positions moved (§2.6).

### 2.5 Title and caption wording — the teaching beats

- List title: **stack trace**; the blurb carries the three-word law
  (opt-in / lazy / loud).
- The in-source caption (lines 3–10) carries BOTH required beats:
  the law caption — "OPT-IN and cheap (a raw frame walk, ~30 ns at
  depth 1) … members symbolicate lazily" — and THE INLINE LAW caption,
  which is what makes 75 of the 107 lines honest instead of noisy.
- The trap comment (lines 103–105) is the loud-boundary beat, with the
  engine's message text quoted for the panes-blind reader.

### 2.6 The byte-stability law — the case's one sharp edge

The expected pins unit-relative LINE:COL. Any edit that shifts a line
(a comment, a pad line, a reflow) flips the chip to ✗ on the next
auto-run — **by design**: the expected is the contract, and for THIS
case the contract includes positions, which is the lesson itself (the
trace is the pipeline's truth, positions included). Precedent:
fuel-demo's expected flips on a budget change the same way. Phase 1
must land the source byte-frozen and the survey's §2.1/§2.2 together;
future editors get a loud diff, not a silent rot.

---

## 3. The interactions

### 3.1 Auto-run on select @1M — fits, measured

- The case compiles clean and runs deterministically; the whole
  chain + capture + render + trap costs **~129 fuel and 706 B heap**
  (the no-trap sibling's measured counters; the trap run reports the
  §1.6 quirk's `fuelUsed: 0`) — four orders of magnitude under the
  1M auto budget and the 4 MiB heap. Selecting the case auto-runs it
  green: `parked: false`, the trap is not fuel-shaped, the chip
  carries the case's OWN expected (D-1's pass-in law) and shows ✓.
- Settled edits auto-run last-wins on the 120 ms lane — a position-
  shifting edit flips the chip per §2.6, exactly like fuel-demo's
  budget flips: the verdict belongs to the last run.

### 3.2 The smoke gains the case — the exact pins that move in phase 1

The headless smoke drives `[...CASES, ...EXAMPLES]` real+verified
(`smoke.mjs` §2), so a ninth inline case is IN the gate by
construction. Phase 1's complete gate-side diff:

1. `demo/scripts/smoke.mjs:141` — `all.length === 25` → `26`, label
   `(8 inline + 17 classics)` → `(9 inline + 17 classics)`;
2. `demo/scripts/smoke.mjs:10` — the header comment's count, same
   arithmetic;
3. `demo/README.md:95` — "all 25 cases listed" → 26 (one word; the
   sentence reads as current state, so it is trued, not archived).

Everything else STAYS at zero:

- **the grep gates (§5)**: the case text trips none of the six
  DEAD_SHAPES (no `dataclass`/`where`/`Ptr<`/`Hashable`/`mapset`, no
  postfix-`?` shape — the source contains no `?` at all);
  zero-hit by construction, proven by the smoke run.
- **the file-scan pin**: `files.length >= 35` under demo/src — an
  inline case adds no file; the pin's arithmetic is untouched.
- **§7 no-sidecars**: no sidecar is created; the raw fs walk stays
  empty.

### 3.3 What does NOT move

- `crates/rut-cli/tests/playground.rs` + the EXPECTED Rust table — the
  classics' native twin; inline cases are not file stems, lookup
  untouched.
- `crates/rut-wasm/smoke.js` — the artifact-lane check that already
  pins the StackTrace surface byte-exact; it re-ran green at base for
  this survey and needs no new pin (the demo case + demo smoke now
  carry the demo-lane law).
- The LSP/highlight lane — tokens are ident-driven
  (`capture_stacktrace` highlights as any call, `StackTrace` as any
  type through the legend); no per-case work, §6's overlay pins are a
  fixed list and unaffected.
- Engine, VERSION, benches — untouched (docs-only phase 0; no phase-1
  engine work either: the surface is shipped and pinned).

### 3.4 Phase 1's verification recipe (the drive)

1. `npm run smoke` — 26/26 cases real+verified (the new pins green).
2. Browser drive (the disclosed raw-CDP precedent): select `stack
   trace` → the case auto-runs at 1M, the chip shows ✓, the Output
   pane shows the four-visual-line render block and the final
   `Trap::IndexOutOfBounds`, fuel 0 (§1.6's quirk, expect it);
   screenshot pair for the batch report.
3. Edit one comment line, let the 120 ms settle → the chip flips ✗
   with the line-paired diff naming row 2 (the position row) — the
   §2.6 law, shown live.

---

## 4. The phase order — confirmed

Two phases, as dispatched: **phase 0 (this document)** freezes the
census, the case bytes, the measured expected, and the pin list;
**phase 1** lands the case (cases.ts) + the three gate pins (§3.2) +
the drive receipt, runs the full gates, and records the report. The
case is one unit — source and expected move together or not at all
(§2.6). No engine work exists in either phase: the surface is shipped
(VERSION 11 carries it since v8) and the phase-1 diff is demo-only.

---

## 5. Deviations

- **The sketch's `st.line(0)` at col 13 became 63:14** — the sketch's
  `let st =` shape puts the callee ident at col 14; measured values
  win over the sketch's comment.
- **The case pads its callees** (75 of 107 lines are `let pN = N;`) —
  forced by the inline law (§1.5), disclosed and taught in-source
  rather than hidden; the compact alternatives were measured and
  rejected with reasons.
- **The trap finale was added** beyond the sketch — the directive's
  "out-of-range traps LOUD" beat, landed as a pinned green row instead
  of a second snippet (one case, one run, both beats; the fuel-demo
  precedent).
- **`npm run build` (dist/) was not re-run for this phase** — the
  parallel deploy lane owns `demo/package.json`/`scripts/`/
  `.wrangler/` in this tree; the demo gates here are the smoke (which
  drives `public/rut.wasm` through its own dist-smoke bundle) + tsc,
  both green, and phase 0 changes no build input. The full build was
  green on this tree in the demo-journey close-out and nothing since
  touched a build input.
- **The ~30 ns figure is cited, not re-measured** — its probe and
  numbers are recorded in the err-channel batch's scratch
  (`capture-cost.md`); re-running a native micro-bench to re-confirm a
  caption's provenance buys nothing this phase. The wasm32 walk's
  behavioral identity IS re-proven (the landed smoke, re-run today).

---

## Appendix — the probes (scratch, zero repo files touched)

All under `/tmp/opencode/batch-demo-stacktrace/p0/`, driven against
`target/wasm32-unknown-unknown/release/rut_wasm.wasm` (rebuilt from
this tree today; `demo/dist/rut.wasm` matches it — both shipped
artifacts agree):

- `probe.mjs` — A: the sketch shape direct (collapses, depth 1);
  B: indirect named chain (compile fail, see probe2); C: the anon
  chain (depth 4, `lambda@NN`); D: the padded named chain (depth 4,
  exact names, fuel 129); E: the trap (see probe2); F: offset
  calibration (capture at user line 4 → unit line 30; **+26**).
- `probe2.mjs` — B1: named fn → fn-typed let = ``unknown name `c` ``;
  B2: anon fn → fn-typed let works; E1/E2/E3: the trap fires in
  f-string, plain call, and via a negative binding (fuel 0 each);
  C2: `lambda@93` survives a source shift (stable id, not a line).
- `probe3.mjs` — THE FROZEN CASE: the 107-line listing (1-based),
  the run at 1M and 10M (identical output, parked false, heap 706),
  the byte-exact expected receipt (§2.2's table is its output).
- `cargo-test.log` — the workspace gate's output at base (§Gates).

Gates re-run at base for this survey: the smoke **273 passed /
0 failed** (this tree, today), `tsc --noEmit` clean,
`cargo test --workspace` green (log in scratch). The foreign
workspace state — the `demo/package.json` mod, untracked `.wrangler/`
and `scripts/` — untouched; this phase stages exactly one path,
`docs/demo-stacktrace-case-survey.md`.
