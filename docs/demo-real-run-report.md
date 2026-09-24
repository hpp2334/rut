# demo-real-run — the batch report

The batch: the playground really compiles and runs — no fake anywhere —
and the editor's color rides the analyzer. Five phases, base `6d330e5`
(the todolist-ui close-out), all work on `master`. Spec:
`~/.opencode/plan/demo-real-run.md` (user directive, Sep 22 2026:
"./demo use fake run output today, that is wrong, we should make it
really compiled and run. And then support grammar highlight in it, you
can use lsp to do that."). Survey of record:
`docs/demo-real-run-survey.md` (phase 0, empirical).

The phases: `3aaedde` (survey), `02937e6` (real-run-always + verifier +
REAL resume), `3210668` (concept refresh + nmap mount + grep gate),
`e72246d` (LSP highlight + census), this commit (the browser proof +
close-out). No engine changes anywhere in the batch; `rut-wasm` gained
only additive host glue; bench pins and `expected.json` untouched.

## 1. The mode law: preview is dead, wasm-or-error

Before: `runner.ts` booted to `mode: "preview"` on ANY artifact failure
— 404, network error, instantiate throw — and preview replayed the
`.expected` sidecars as "output", faked a clean compile for any source
(including source that does not parse), and special-cased `fuel-demo`
into a fabricated trap. The fake was not an edge case: `public/rut.wasm`
was gitignored and absent, so every fresh checkout was preview mode —
the dishonesty was the default experience.

Now: `RunnerState.mode` is `"wasm" | "error"`. A boot failure is not a
fallback, it IS the page state: a full-page panel — "rut.wasm missing
or invalid" — with the boot error text and the exact
`npm run build:wasm` command (RFC 0041 §3). The panes never mount; an
error Runner has no working compile/run (loud throw, defense in
depth). `placeholder()` is deleted; the preview replay and the
`fuel-demo` lines.pop() lie died with it. The wasm banner keeps the
honesty law the other way: it names the real engine slice the host
mounts — "live — rut.wasm · the playground slice: core, calc, rt, ink,
pouch, nmapset mounted".

`dev`/`build` preflight both artifacts before the bundler (loud, names
the command); `build:wasm` copies them through the extension's
curated loud-fail (`scripts/copy-wasm.mjs`). The artifacts exist by
construction, ship in `dist/`, and stay gitignored with the exact
build command in the error text.

## 2. The sidecar flip: replay → verify

The `.expected` sidecars were the preview's REPLAY SOURCE — the fake
itself. After the flip they are VERIFIERS: every real run is diffed
against its case's sidecar (`src/verify.ts`, line-paired), the
StatusBar chip shows `✓ matches expected @ <fuel> fuel` (it names the
budget it verified at) or `✗ N lines differ`, and a failing verify
renders the line-paired got-vs-sidecar diff in the Output pane. The
trap line participates (`Trap::OutOfFuel` where pinned). An edit
retires the chip until the next run — an edited source that then
mismatches shows its diff, which is the honest signal. The demo became
a live regression harness: a wrong expectation or a broken build shows
red in the page, not a lie.

Two sidecars legitimately moved in phase 1, both disclosed in its
commit body (the survey's sanctioned moves):

- `values-and-pointers`: the case taught the DEAD pre-RFC-0044 regime
  and did not even compile (`&p`, `&same` diagnose). Rewritten to
  today's sharing surface; all four output lines moved.
- `fuel-demo`: expected re-recorded at the pinned DEFAULT budget —
  zero tick lines + `Trap::OutOfFuel` (the old `tick 1000000/...`
  magnitude was a ~40M-fuel preview-era fabrication; ~10 ops/iteration
  fit zero ticks in 10M). A raised fuel box or a Resume shows a diff
  BY DESIGN.

## 3. The REAL resume

The engine always had parked-frame resume (`Vm::resume`); the wasm
module threw the frame away — `rut_run` is one-shot — so the UI's
"↻ Resume (+10M fuel)" was a re-run pretending to be a resume. The
survey's D5 REAL path landed: `rut-wasm` keeps at most one parked Vm
(static, single-thread wasm) and exports `rut_resume(extra_fuel, heap)`
+ `rut_drop_frame()` (additive; run envelopes gain `"parked":bool`).
The UI's Resume now calls it: the frame continues at the parked pc,
output accumulates, fuelUsed is cumulative, a second OutOfFuel re-parks,
a case switch retires the frame. Proven twice over: four host tests in
`rut-wasm` (resume(0) re-traps with the counter unmoved — a restart
cannot burn 12M ops on a zero budget; resume(+16M) yields
`tick 2000000`, never `tick 1000000` again; cumulative 28M fuelUsed)
and the same proofs in the smoke through the shipped artifact.

## 4. The concept refresh (the audit executed)

Authority: the RFCs as amended + the current std pkg set + the current
grammar — the demo teaches TODAY, comments and blurbs included. The
audit's full verdict tables are §2 of the survey; what changed:

**What died** (each disclosed in phase 2's commit body):

| died | where | replacement taught |
|---|---|---|
| the `dataclasses` identity + `own` divergence | structs (renamed from dataclasses) | RFC 0009 v1.1 `struct`, reference semantics (sharing by default), identity `==` (RFC 0044) |
| `next: *Node` comment + the `Weak`-fix fiction | node-cycle | `?Node` is the pointer shape (prefix-only); the cycle stays live — no collector, no weak refs yet (RFC 0017 future) |
| `*Node` is one word + comma fields | tree | `?Node` one word, `nil` when absent; `;` field style |
| mangled header + fake `Weak(alive)` spelling | weak-cache | the cache/observer shape with today's strong refs; a miss answers `nil` |
| "arrows" in the blurb | closures-generics | anonymous fns (block bodies — RFC 0013 has no arrow form) |
| `[T]`/`Vec` "implement `Iter`" + an unused `bytes(64)` | literals | the engine-woven Iterator; the binding USED (`bin=64` — the sidecar moved, real run) |
| copy-by-value + structural `==` + `&v` | values-and-pointers (inline) | sharing, identity `==`, `?T` (RFC 0044) — §2 above |
| the ~40M-fuel magnitude sidecar | fuel-demo (inline) | the honest trap-at-default story |

**What was added** (four gap-fillers, each with a fresh sidecar
recorded from REAL runs — gated natively by the extended playground
test, re-proved through the wasm artifact by the smoke):

| example | teaches | rfcs |
|---|---|---|
| `maps.rut` | the keyed lane: `HashMap<str, i32>` word/count, `PrimMapI64<str>` id→payload, `HashSet` membership; no hashing trait — keys admitted by the compile-time union bound | 0043 §A5, 0023 §2 |
| `bytes.rut` | encode/decode round trip, assignment SHARES, `bytes.clone()` is the ONE copy, octets vs codepoints | 0044 §3 §4, 0004 |
| `str-views.rut` | O(1) `slice` views (a slice IS a str), `s.code()` / `str.from_code`, slice bounds are codepoint indices | 0042, 0004 |
| `checked-arith.rut` | `wrapping_*` two's-complement, `checked_*` → the (T, bool) tuple | 0032 §1.1, 0004 §3 |

`maps` carried a real prerequisite: `rut-wasm`'s `compile_playground`
now mounts `nmapset` (inline source, like ink) + `nmap_host` (lowered
from its embedded .d.rut, like rt) and `rut_run` installs
`install_std_nmap` — additive crate glue, no engine change, and the
native playground gate mounts the same lane so the sidecar is gated
natively. The wasm banner names the slice.

**The grep gate** (smoke section [5]) commits the audit: every file
under `demo/src`, comments included, whitelist nothing, scanned for
the dead shapes — `dataclass` as keyword, `where` clauses, `Ptr<`,
`Hashable`, the removed `mapset` pkg, postfix `?T`. 50 files, 0 hits.

## 5. The LSP wiring: one `rut_analyze`, the overlay

Highlight rides the analyzer (the user directive), and the demo's
slice of the LSP is ONE call. `src/lsp/rut-lsp.ts` is the demo's typed
twin of the extension's `src/wasm.ts` over the SHIPPED standalone
`rut-lsp.wasm` (~0.7 MB, consumed, never patched): boot once (loud
export-surface validation + the legend read), then per request
`rut_begin` → `rut_alloc` → the `[u32 LE len][JSON]` envelope.
`rut_analyze(uri, src)` stores the doc FULL-SYNC and the one response
carries diagnostics AND the complete semantic-token stream; the app
debounces 150 ms and re-sends the whole doc per change (case selects
and edits both flow through `source` — one effect covers both). No
TextMate layer: one tokenizer, the true one; monochrome text until the
first analyze lands (sub-100 ms) is the honest interim, never a fake.

The editor is the classic zero-dep double-layer (survey D4): a
transparent-text `<textarea>` (caret, selection, Tab = 2 spaces)
exactly over a highlighted `<pre>` overlay — same 12px/1.5 mono
metrics, scroll-synced like the gutter, pointer-transparent so the
caret never loses a click. `src/lsp/overlay.ts` (pure, smoke-driven)
cuts each line at token/diag boundaries: token class → a
`tok-<legend-NAME>` class keyed by `rut_legend()`'s reported names
(14 types, never a hardcoded order), diagnostics → a wavy-underline
squiggle span carrying its message, plus a zero-dep hover tip that
hit-tests the diags through a hidden monospace metrics probe.
Compile-time diags (Run's loud panel) stay the Run-time surface;
squiggles are the while-typing surface.

The binding law is highlight-or-loud: a missing/invalid
`rut-lsp.wasm` boots the same full-page error panel shape as the
runner, naming the command. The smoke gates the highlight through the
SAME binding the editor uses: the 14-type legend, ZERO false
diagnostics across all 25 cases, a census of 33 pinned positions
across 12 cases (keywords, primitives in type position, types, fn
names, strings, name positions — every entry pinned against the
shipped artifact's observed classification), and the overlay law —
the paint-path builder reconstructs every case source EXACTLY.

## 6. The browser drive — the "no fake anywhere" acceptance

Headless Chrome (agent-browser CLI, CDP), static server
(`python3 -m http.server 8123` in `demo/dist/`), the shipped `dist/`
build. Scratch: `/tmp/opencode/batch-demo-real-run/p4/`.

| state | verified in-page | screenshot |
|---|---|---|
| boot | banner `live — rut.wasm · the playground slice: core, calc, rt, ink, pouch, nmapset mounted (RFC 0041 §3)`; all 25 cases (8 inline + 17 classics); the overlay already painting `.tok-keyword` spans before any interaction | `1-boot.png` |
| run-verified | Run on `hello, format`: output pane shows the REAL engine output `hi rut! n=42 tab:` (a real tab) / `sour`; chip `✓ matches expected @ 10M fuel` (class `chip chip-pass`); telemetry `fuel used: 39`, `heap: 261 B`; AST pane a real tree (Module/Use/Fn word_counts…), IR pane real LIR (`unbox/conv/call` at regs) — no placeholders anywhere | `2-run-verified.png` |
| squiggle | editor text `let n = 41 + 1;` → `41 + ;`: the offending `;` renders a `.squiggle` span titled `expected an expression, found \`;`\``; the verify chip retires on the edit (the verdict belongs to the last run) | `3-squiggle.png` |
| maps-case | restore → select the gap-filler `maps & sets` → Run: real nmap-lane output (`rut=3 runs=1 / replace=false rut=9 / … / has x=true has z=false`), chip `✓ matches expected @ 10M fuel`, `fuel used: 638`, `heap: 545 B` | `4-maps-case.png` |

Server killed after the drive; the browser session closed.

**THE DRIVE'S FIND (recorded + fixed here):** the first load of the
shipped `dist/` was a BLANK PAGE — `#root` empty, console clean, and
one uncaught `ReferenceError: $RefreshReg$ is not defined` in
`main.js`. Root cause: the build config shipped the react-refresh
machinery (the dev-server-only plugin + swc `refresh: true`) into the
static bundle — 9 registration CALLS, 0 definitions (the runtime
exists only under `rspack serve`). The smoke could not see it: its
node bundle drives runner/cases/verify/examples with no React mount.
Every prior phase's "build green" was true — and insufficient; the
page had never been loaded from `dist/` in a browser before this
drive. The fix is build wiring only (`rspack.config.ts`): the refresh
plugin and the swc refresh flag now ride the `dev` lane
(`npm_lifecycle_event`), `npm run build` emits a bundle with zero
refresh references, and `npm run dev` keeps hot refresh unchanged.
Re-driven end to end green after the fix (the table above IS the
post-fix drive).

**Practical limits of the drive (honest):**

- The hover tip (the zero-dep mousemove hit-test) was not exercised —
  headless CDP hovering over a squiggle is possible but was not part
  of the scripted pass; the tooltip content is verified via the
  squiggle span's `title` attribute (read back from the live DOM).
- The squiggle is a real DOM span with the wavy underline class, but
  the screenshots are full-page at 100% zoom — the underline is
  visible but small; the DOM assertions above are the precise record.
- The drive exercised one inline case end-to-end (hello, format) plus
  the maps gap-filler; the other 23 cases are covered by the smoke's
  real-run proofs through the same artifact and binding, not by
  individual browser passes.
- Practical browser note: `dist/` needed a plain static server
  (wasm + same-origin fetch); the dev server path (`npm run dev`) is
  the iteration lane and was not separately driven.

## 7. Gates (all green, this commit)

- the drive: green, state by state, 4 screenshots (`p4/1-boot.png`,
  `2-run-verified.png`, `3-squiggle.png`, `4-maps-case.png`);
- the smoke: `npm run smoke` — **272 passed, 0 failed** (runner law +
  25 real runs/resume proofs/diags + grep gate + the highlight
  census/overlay law);
- demo build: `npm run build` green, both artifacts shipped to
  `dist/` (rut.wasm 1,433,110 B; rut-lsp.wasm 719,287 B), zero
  `$RefreshReg$` references in the shipped bundle;
- workspace: `cargo test --workspace` — **647 passed, 0 failed**
  (unchanged from phase 3; no Rust code changed in this phase);
- wasm32: `cargo check --workspace --target wasm32-unknown-unknown`
  exit 0;
- bench sanity (pinned rows, checksums vs `expected.json`): sieve
  `41538` ✓, intloop `628038624` ✓, call `317811` ✓, refvals
  `140052990000` ✓ — pins stand;
- tree clean except this phase's three files (`demo/rspack.config.ts`,
  `demo/README.md`, `docs/demo-real-run-report.md`), staged by
  explicit path; the foreign `batch-plan-impl` stash untouched;
- `tsc --noEmit` — clean (config-only + docs changes; re-verified).

## 8. The MENU (recorded, not fixed)

1. **The classifier token gaps** (phase 3's recorded-for-the-menu
   items, now with the drive's visual verdict):
   - a constructor-style receiver (`X.new`) is shaped `enumMember` —
     `Logger`/`Vec`/`Point` at `.new` sites paint in the enumMember
     color. The drive's verdict: visible but reads fine at 100% — the
     span is on the receiver name, colored like a type-adjacent name;
     harmless, cosmetic. Upstream fix = classify receiver-head
     constructors as `type`-ish, one rule in the wasm face.
   - type parameters (`T`, `U` in `map<T, U>`) get NO token — they
     render monochrome inside otherwise-colored signatures.
   - primitive type-methods in expression position (`bytes.`, `str.`
     before a method name) get no token for the receiver.
   - No false diagnostics in any of these (the smoke's zero-false law
     holds); the gaps are paint-only, and the drive confirms the
     paint reads right overall.
2. **The static build ships development-mode React** (unminified).
   Kept deliberately when the refresh defect was fixed — switching
   `mode` to production changes the shipped surface for every case
   and deserved its own pass (minified bundle, prod React warnings
   gone, bundle size). Menu: a `--profile`-style pair of build lanes.
3. **Playground editing of arbitrary sources**: custom edits verify
   against the case they came from; there is no user-authored
   sidecar. A "pin this output as expected" affordance would make the
   page a full self-serve regression harness.
4. **Delta tokens / a worker**: the LSP face is full-sync
   `rut_analyze` on the main thread; fine at doc sizes, but a
   re-architecture (deltas upstream + worker) is the growth path if
   docs get big.
5. **CodeMirror**: still the noted-not-taken editor upgrade; the
   drive's practical test of the overlay (alignment, scroll-sync,
   squiggles) found no drift, so the zero-dep layer stands.
6. **Host futures**: `select`/`await` parse but are unusable in this
   host (no host-futures bindings in rut-wasm) — taught never,
   honestly. A futures-enabled host would unlock the async lesson.
7. **The fuel-demo verify-by-design diff**: raising fuel or resuming
   shows a red diff BY DESIGN (the sidecar pins the default budget).
   A per-case "expected at budget B" table would let the chip stay
   green across budgets; today the honesty is the feature.

## 9. Follow-up: the no-sidecars batch (phase 1, landed)

This batch's storage law moved under the demo-real-run batch's
descendants: `docs/demo-no-sidecars-survey.md` (phase 0) census'd the
17 sidecars; phase 1 retired them. The verifier this report describes
is untouched — the chip, the line-paired diff, and the smoke's
real+verified law all survive; only where the expected bytes LIVE
moved. The 17 files' contents (md5-receipted in the survey §1.1,
quicksort's load-bearing trailing space included) now ride INLINE in
the case entries (`examples/index.ts` template blocks, `cases.ts`
already carried its 8), with a verbatim copy in
`crates/rut-cli/tests/playground.rs`'s `EXPECTED` table so the native
gate keeps its byte diff; the smoke gained the §7 no-sidecars walk
(`find demo -name '*.expected'` empty, untracked included) and its §5
scan pin re-pinned 40 → 35. The report's own wording above ("sidecar")
is this batch's historical record; the live truth is inline expected.

## 10. Follow-up: the stack-trace case (the demo-stacktrace-case batch, phase 1)

Directive: *"in demo, add a case of stack trace."* Survey of record:
`docs/demo-stacktrace-case-survey.md` (phase 0 — the census, the
frozen 107-line source, the measured expected, the position map). The
case landed inline in `cases.ts` (id `stack-trace`, RFC 0036, between
`closures-generics` and `fuel-demo`): a padded `main → a → b → c`
chain — the padding IS the checker's inline law taught (a callee with
≤ 24 top-level statements inlines and the frames collapse; the
survey's probes measured it live in the demo lane) — the capture deep
in `c` returns the shared trace cell, `Logger` stays `main`'s local,
and the finale traps `IndexOutOfBounds` on `st.name(9)` so the loud
boundary pins as a green row (the fuel-demo precedent). The expected
is the survey's measured wasm-lane bytes; `render()` rides ONE
expected element with embedded newlines — the element-wise verifier
compares it exact, the Output pane renders it as four visual lines.
Positions are part of the contract: any line shift flips the chip BY
DESIGN (survey §2.6, stated on the case's TS comment).

The drive (this commit; agent-browser CLI, headless Chrome over CDP —
the same lane as §6 — against a scratch build served statically, since
the parallel deploy lane works in this tree; `demo/dist` untouched):

| state | verified in-page | screenshot |
|---|---|---|
| boot | banner `live — rut.wasm · the playground slice: core, calc, rt, ink, pouch, nmapset mounted (RFC 0041 §3)`; all 26 cases listed; `stack trace 0036` sitting between `closures & generics` and `fuel demo` | `batch-demo-stacktrace/p1/1-boot.png` |
| run-verified | select `stack trace` → the auto-run fires @ 1M: Output pane reads `depth: 4` / `innermost: c at 63:14` / `outermost: main` / the four `at fn (rt:…)` render rows / `Trap::IndexOutOfBounds` last; chip `✓ matches expected @ 1M fuel` (class `chip chip-pass`); telemetry `fuel used: 0`, `heap: 706 B`; zero console messages at every level | `batch-demo-stacktrace/p1/2-stack-trace-verified.png` |

The quirks, disclosed before anyone chases them:

- **`fuel used: 0` on this case's run is real, not a bug** — a
  non-fuel trap exits the threaded loop without the final fuel sync,
  so the envelope's `fuelUsed` reads 0 on the trap-finale run (the
  same source without the trap measures ~129). Cosmetic; measured
  twice at phase 0 (survey §1.6) and reproduced in this drive.
- **The columns are the callee idents, measured**: `let st = …` sites
  sit at col 14 (the directive sketch's comment said 13 — measured
  values win), `return …;` sites at col 12.
- **The `rt:` name and the +26 lines are the demo mount's splice
  truth** (wasm-lane values; the native lane would print `core:` at
  +0). That disclosure lives in the survey §1.3–1.4 and is re-proven
  by the smoke every run — not captioned on the page, which cannot
  keep a lane claim current.
- **A phase-1 re-measure note**: phase 0's scratch probe3.mjs carries
  a hand-template off-by-one (`67/95/99 + 26`) that prints
  `EXPECTED-MATCH: false`; re-run at phase 1 start, the engine's live
  bytes are exactly the survey's `63/92/120/124` — the survey's frozen
  map is correct, the probe's template was wrong.

Gates: smoke **281 passed / 0 failed** (26 cases real+verified, the
grep gates zero-hit, the file-scan pin still 35), `tsc --noEmit`
clean, scratch `npm run build` exit 0 (zero `$RefreshReg$` in the
shipped bundle), `cargo test --workspace` **770 passed / 0 failed**,
`cargo check --workspace --target wasm32-unknown-unknown` exit 0. The
phase-1 diff: `demo/src/cases.ts` (the case),
`demo/scripts/smoke.mjs` (the corpus pin 25 → 26 + its two
same-arithmetic comments), `demo/README.md` (one word), this section.
