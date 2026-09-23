# the demo journey audit — phase 0 of `demo-journey-fixes`

Docs-only census. Everything here was **driven, measured, and screenshotted
today against a fresh `npm run build` of this tree at `e88be67`** (the
no-sidecars batch's phase-1). The three seeds get diagnoses grounded in
evidence, not guesses; the census is severity-ordered; every fix has a
verification recipe for phase 1.

## 0. method + disclosures

- **The drive**: the journey script was executed end-to-end against
  `http://127.0.0.1:8131/` (a static server over a fresh
  `demo/dist`), 1500×900 viewport.
- **Disclosed fallback (the no-sidecars precedent)**: the desktop
  agent-browser session was not connected (`No desktop browser is
  connected to this session`), so the drive ran over **raw CDP against
  the session's own chrome-for-testing build**
  (`~/.agent-browser/browsers/chrome-153`, `--headless=new`, flat
  session, real `Input.dispatchKeyEvent`/`dispatchMouseEvent` — not JS
  synthesis) from scratch tooling under
  `/tmp/opencode/batch-demo-journey/p0/` (`lib.js` client, `drive1-5`
  stages). No repo files were touched by the tooling.
- **Scratch artifacts** (not committed): 25 screenshots in
  `/tmp/opencode/batch-demo-journey/p0/shots/`, the measurement JSONs
  (`analyze-decomposition.json`, `typing-measurements.json`) and the
  cpuprofile (`typing-burst-big.cpuprofile`) in the same directory.
- **Gates baseline (untouched)**: `npm run smoke` → **273 passed,
  0 failed** (`THE SMOKE IS GREEN`) on this exact tree, before the
  docs commit. `npm run build` green. This commit changes no code.

## 1. the journey log — current reality per step

| step | current reality | evidence | verdict |
|---|---|---|---|
| boot | live banner `live — rut.wasm · the playground slice: core, calc, rt, ink, pouch, nmapset mounted (RFC 0041 §3)`; all **25 cases** (8 inline + 17 classics); overlay already painting LSP tokens (53 `tok-*` spans on boot) | `01-boot.png` | GREEN |
| select inline case | `hello, format` swaps source, 18 lines / 53 token spans, overlay text reconstructs the source, panes reset to `(no output)` | `02-case-inline-hello.png` | GREEN |
| select classic | `quicksort` 44 lines / 182 spans; `matrix multiply` 31 lines / 170 spans | `02-case-classic-quicksort.png`, `02-case-classic-matrix.png` | GREEN |
| select gap-filler | `maps & sets` 75 lines / 257 spans (list must be scrolled; see drive note D-1) | `12-case-maps-belowfold.png` | GREEN |
| read highlighted source | overlay prefix-equals the textarea value on every case (full equality differs only by the hidden 10-char metrics probe — census I-2); legend-name classes verified rendering (keywords/strings/types/functions/enums) | stage-1 probes | GREEN |
| edit → squiggle flow | `41 + 1` → `41 + ;`: **2 squiggle spans** titled `expected an expression, found ';'` render on the offending `;`; the chip retires on edit (stays absent — verdict belongs to the last run); restore clears the squiggles | `05-squiggle.png` | GREEN |
| explicit Run | chip `✓ matches expected @ 10M fuel`, fuel 39 / heap 261 B, output `hi rut! n=42 tab:` / `sour`; AST pane renders the structured tree (Q40 Module → Use/Enum/Fn with spans, expand/collapse); IR pane renders a real 1310-byte LIR dump | `20-clean-run-output.png`, `21-clean-run-ast.png`, `22-clean-run-ir.png` | GREEN |
| failing expectation | `41 + 1` → `41 + 2`, Run: chip `✗ 1 line differ @ 10M fuel` + the line-paired diff (`✗ 1 got: hi rut! n=43… / expected: …n=42…`, `= 2 sour`) | `16-run-fail-diff.png` | GREEN |
| a failing COMPILE | the four diags render loud in the Output pane; chip stays absent; AST/IR panes say `(compile to see …)` honestly | `06-run-hello.png`, `08-run-ir.png` | GREEN |
| Resume flow | fuel demo Run parks: `Trap::OutOfFuel`, output exactly the trap, chip `✓ matches expected @ 10M fuel` (the expected pins the default), Resume enabled → Resume: output accumulates `tick 1000000` + trap, fuel box 20M, fuel used 20,000,002, chip `✗ 2 lines differ @ 20M fuel` **by design** (the expected pins the default budget) | `10-parked-outoffuel.png`, `11-resumed-ticks.png` | GREEN |
| text selection legibility | **BROKEN — census H-1**: any selection renders as a solid opaque block, zero glyphs visible | `03/04-selection-*.png` | RED (seed 1) |
| scroll at long sources | **BROKEN — census H-2**: the editor never scrolls internally; the PAGE scrolls; the header/case-list/panes/status-bar all scroll away; the status bar gets overlapped by editor text | `13/17/18/19-*.png` | RED |
| keyboard-only pass | case buttons are reachable and Enter/Space activate them; focus outlines visible on buttons — but **the textarea traps Tab AND Shift+Tab (both insert two spaces), so Run/Resume/budget/pane-tabs are unreachable by keyboard — census H-3** | stage-3 probes | RED |
| typing latency probe | at real case sizes keystrokes sit at the bare-textarea floor (no measurable app cost, zero longtasks); at ~1500 lines every keystroke costs **+30 ms** above floor (median 60.8 ms vs 29.2 ms control); the overlay/diags catch-up lands ~150 ms after the last key (the debounce). Full decomposition in §2 M-1 (seed 2) | `typing-measurements.json`, `typing-burst-big.cpuprofile` | AMBER (seed 2) |
| auto-run (seed 3) | **absent today**: a case switch leaves `(no output)` + no chip; an edit runs nothing — only ▶ Run executes | stage-1/2 probes | ABSENT (seed 3 designs it) |

Drive notes: **D-1** below-fold case buttons need the list scrolled
(before my `scrollIntoView` a blind coordinate click landed on nothing —
a drive artifact, not an app bug; a human scrolls first). **D-2** the
four compile diags in `06-run-hello.png` came from a driver-side botched
restore (`;;`), kept as the failing-compile evidence.

## 2. the census (severity-ordered)

| id | severity | area | one-line | disposition |
|---|---|---|---|---|
| H-1 | HIGH | editor/selection | selection paints opaque over the overlay — selected text unreadable | **fix, phase 1** (seed 1) |
| H-2 | HIGH | editor/scroll | long sources blow out the page; the editor never scrolls internally | **fix, phase 1** |
| H-3 | HIGH | a11y/keyboard | the textarea traps Tab and Shift+Tab; the whole lower UI is keyboard-unreachable | **fix, phase 1** |
| M-1 | MEDIUM | perf/typing | +30 ms/keystroke at ~1500 lines (React reconciliation of the whole overlay, no memo boundaries; ~27% of active CPU is dev-React validation); felt lag at all sizes = the 150 ms debounce | **fix, phase 1** (seed 2, measured scope in §2) |
| D-1 | DESIGN | runs | no auto-run on case switch or edit (the seed-3 mandate) | **design now, implement phase 1** (§2 D-1) |
| L-1 | LOW | perf/build | `dist/` ships development-mode React (unminified) — feeds M-1's tax | menu (README already records the stance) |
| L-2 | LOW | wording | UI strings still say "sidecar": `Panes.tsx` diff head `(got vs sidecar)`, `StatusBar.tsx` chip titles — the sidecars are retired, expected is inline | fix, phase 1 (wording-only, rides along) |
| I-1 | INFO | editor/overlay | overlay windowing for very long docs (paint visible lines ± buffer) — only if M-1's memo pass doesn't reach the bar | menu, measure-gated |
| I-2 | INFO | editor/probe | the hidden metrics probe appends `0000000000` inside the aria-hidden overlay → `overlay.textContent ≠ value` by 10 chars; harmless (smoke reconstructs spans, not textContent), but future full-text-equality tests must know | record only |
| I-3 | INFO | engine shape | the one-call `rut_analyze` delta lane | **rejected by measurement** for phase 1 — §2 M-1's numbers; stays a menu item |

### H-1 — selection erases the selected text (seed 1) — exact layer diagnosis

Repro: double-click or drag-select anywhere in the editor. The selected
range renders as a **solid dark block with zero visible glyphs**
(`04-selection-doubleclick.png`: lines 1–2 of `matrix multiply` show two
empty bars).

Layer facts, measured in the live page:

- the textarea (`.editor-input`) is the TOP layer; its own text is
  `color: transparent` (the overlay is the visible text);
- its `::selection` computes to **`background: rgb(43, 61, 87)` — fully
  OPAQUE** (`styles.css` `.editor-input::selection { background:
  #2b3d57; color: transparent }`), `color: rgba(0,0,0,0)`;
- the highlighted `<pre>` overlay is `aria-hidden`, `pointer-events:
  none`, sits BELOW the textarea, and never paints a selection of its
  own — so the only thing painted inside the selection band is the
  textarea's opaque background, which covers the overlay's colored
  glyphs beneath.

**Fix pick: semi-transparent selection background on the textarea
only.** `background: rgb(111 179 255 / 0.25)` (the accent at 25%), keep
`color: transparent`. Why this one: the overlay's colored glyphs then
show through the translucent band (the selection reads as a tint, the
squiggle underlines stay visible through it, the caret is unaffected);
it is one CSS declaration; zero JS. Rejected alternatives: **mirrored
::selection on the pre** — the pre never owns a selection (the textarea
does), so a mirror requires JS selection syncing to be meaningful, which
is option three anyway; **projected selection classes into the overlay**
— needs a `selectionchange` listener + per-line re-cut on every selection
change (heavy churn in exactly the hot path M-1 is trying to empty) for
zero visual gain over translucency.

### H-2 — long sources blow out the page (scroll broken) — exact mechanism

Repro: set a ~1500-line source (or pick any long doc). Measured:

- `document.documentElement.scrollHeight` = **27 950 px** (viewport 900);
- `.editor-input` and `.editor-overlay` and `.editor-body` and
  `.editor` ALL compute to **27 884 px tall**; the textarea's
  `scrollHeight == clientHeight` → `scrollTop` clamps to **0** — the
  editor can never scroll internally, so the JS gutter/overlay scroll
  sync (`Editor.onScroll`) never engages;
- the WINDOW scrolls instead: the header, case list, panes, and status
  bar scroll away (`19-scroll-clean-bottom.png` shows raw editor rows
  filling the whole viewport), and the status bar gets overlapped by
  editor text at partial heights (`17-scrolled-bottom-verified.png`).

Root cause chain: the **gutter's in-flow line-number `<pre>`** grows to
the source's full line count (27 884 px), which makes the `.editor` flex
container's content-based height 27 884; `.editor` is a grid item in
`.app-main` with default `min-height: auto`, so that content height beats
its 784 px row and the column grows; `.editor-body` stretches with it;
the two absolute layers' `height: 100%` follow (both measured at
27 884 px). The scroll-sync design (textarea is THE scroller; gutter +
overlay follow via JS) is correct but structurally disarmed.

**Fix pick: restore the height chain — `min-height: 0` on `.editor`
(the grid item — that is THE fix) and, defensively, on
`.editor-gutter` (the flex item), keeping `overflow: hidden` there —
so the editor column stays at its row height and the textarea becomes
the real scroller again.** No JS changes: the existing scroll sync then
works as written. (The overlay `pre` keeps `overflow: hidden`.)

### H-3 — the editor is a keyboard trap

Measured: Tab in the textarea inserts two spaces and keeps focus (by
design); **Shift+Tab in the textarea ALSO inserts two spaces and keeps
focus** (`inserted: 2`, focus stays `editor-input`) because
`Editor.onKeyDown` matches `e.key === "Tab"` with no modifier check and
`preventDefault`s everything. Consequence, walked live: after Tabbing
from the case list into the editor, 40 consecutive Tab presses never
leave the textarea — **Run, Resume, the fuel/heap inputs, and the
pane tabs are unreachable by keyboard**. (Escape is not handled either.)

**Fix pick: capture only unmodified Tab** — `if (e.key === "Tab" &&
!e.shiftKey && !e.ctrlKey && !e.metaKey)` — so Shift+Tab walks back out,
and add **Escape blurs the editor** as the forward escape hatch. Ride-along
(attempt, keep if trivial): give the pane-tab buttons and status-bar
controls their default focus outlines (buttons already show `outline:
auto`).

### M-1 — typing latency, measured decomposition (seed 2)

The probe: real per-keystroke CDP key events at ~25 ms cadence,
per-keystroke `dispatch → 2×rAF` frame latency in-page, LongTask
observer, a sampling CPU profile over the burst, plus a **bare-textarea
control page** that pins the driver+browser floor.

**End-to-end (the numbers that name the dominant term):**

| scenario | median keystroke frame | p90 | max | longtasks >50 ms |
|---|---|---|---|---|
| control: bare textarea (the floor) | 29.2–30.3 ms | ~30.8 | 44.7 | 0 |
| app, `hello` (18 lines) | 29.9 ms | 45.8 | 49.8 | 0 |
| app, `quicksort` + AST pane open | 30.6 ms | 45 | 45.8 | 0 |
| app, **~1530-line / 31 KB doc** | **60.8 ms** | 60.3 | 67.4 | 0 |

(The absolute floor is the 2×rAF quantum + driver round-trips; what
matters is the delta: **≈0 ms at real case sizes, ≈+30 ms per keystroke
at ~1500 lines**. The app never crosses the 50 ms longtask threshold —
the slowness is death by a thousand 30 ms frames, not one long stall.)

**Pipeline micro-decomposition** (in-browser, the REAL `rut-lsp.wasm`
through the identical binding; medians of 9; stage functions identical
to `overlay.ts`/`rut-lsp.ts`):

| source | bytes | lines | envelope open (rut_begin + uri alloc/copy) | src alloc + copy | **rut_analyze** | envelope read | JSON.parse | decodeTokens | buildOverlay | **total** | response JSON |
|---|---|---|---|---|---|---|---|---|---|---|---|
| hello | 343 B | 18 | <0.05 | <0.05 | **0.10** | <0.05 | <0.05 | <0.05 | <0.05 | **0.10 ms** | 1.5 KB / 53 tokens |
| when | 757 B | 26 | <0.05 | <0.05 | **0.10** | <0.05 | <0.05 | <0.05 | <0.05 | **0.10 ms** | 2.6 KB / 66 |
| sieve | 514 B | 19 | <0.05 | <0.05 | **0.10** | <0.05 | <0.05 | <0.05 | <0.05 | **0.10 ms** | 1.4 KB / 91 |
| when ×60 | 45.5 KB | 1560 | <0.05 | 0.10 | **9.30** | <0.05 | 0.40 | <0.05 | 1.40 | **11.2 ms** | 158 KB / 3960 |

**The dominant term, named with numbers:**

- **At real case sizes (< 80 lines): the entire wasm+JS pipeline is
  0.1 ms — the felt lag is the 150 ms debounce itself** (measured
  overlay catch-up: the rebuild lands ~150 ms after the last keystroke;
  that is the design's dead zone, and it is the only "slow" a normal
  case ever shows).
- **At long docs: React, not the engine.** The CPU profile over the
  big-doc burst shows the active time dominated by reconciliation of the
  whole overlay subtree per keystroke — `FiberNode` 60.9 ms,
  `reconcileChildrenArray`/`reconcileChildFibers`,
  `jsxDEV`/`ReactElement`, `createWorkInProgress`, `diffProperties` —
  with **~27% of active CPU burned by development-mode React
  validators** (`warnInvalidARIAProps` 17.6 ms, 
  `defineKeyPropWarningGetter` 17.3 ms, `warnOnInvalidKey` 14.9 ms,
  `warnUnknownProperties`, `validateChildKeys`, `warnForMissingKey`).
  **Zero profile samples in `rut_analyze`, `decodeTokens`, or
  `buildOverlay`** — the engine runs once per pause, outside the
  keystroke path. Every keystroke re-renders the App (state change) and
  the Editor re-creates the full ~4 000-span JSX tree (new element
  identities each render → no bailout), plus CaseList (25 buttons),
  Panes (incl. an open AstTree), StatusBar.

**Fix directions, in measured order:**

1. **Memo boundaries** (kills the per-keystroke overlay diff): extract
   the overlay's line rendering into a memoized component keyed on the
   line's span array identity, and memoize per-line span arrays so an
   edit only re-cuts the edited line (`buildOverlay` already
   line-decomposes — reuse it incrementally). Also `memo()` CaseList /
   Panes / StatusBar so a `source` keystroke re-renders the Editor only.
2. **Debounce tuning** (the felt term at all sizes): keep LAST-WINS
   stale-drop (already correct), trim 150 ms → ~100–120 ms and verify
   feel + catch-up in the re-drive; the analyze is 0.1 ms — the budget
   is all human patience.
3. **Overlay windowing** (menu, measure-gated): paint only the visible
   line window ± buffer once H-2 makes scrolling real; only if (1)
   doesn't bring 1500-line keystroke cost to ≤ floor + 10 ms.
4. **Ship production React in dist** (menu, L-1): ~27% of the big-doc
   active CPU is dev-only validation.
5. **The delta-analyze lane: REJECTED for phase 1 by measurement** —
   `rut_analyze` is 0.1 ms at playground sizes and 9.3 ms at 1560 lines,
   once per pause; the envelope (`rut_begin` + allocs + copies) is
   <0.05 ms. There is no measured case where the one-call shape is the
   bottleneck; it stays a menu item with these numbers attached.

### D-1 — auto-run design (seed 3), guards pinned

Current reality: nothing auto-runs (census table). The design, with
each guard's recommendation:

- **Case switch runs IMMEDIATELY** (no debounce): `selectCase` already
  resets the machine state; the auto-run piggybacks on that reset —
  compile + run the fresh source synchronously (it is exactly what ▶ Run
  does today). The chip + panes populate the moment a case opens; the
  fuel demo parks immediately, teaching the trap on selection.
- **Edits auto-run after typing settles** — the SAME debounce lane as
  the highlight (one timer, two consumers): default 150 ms, tuned with
  M-1 (§2) toward 100–120 ms. **LAST-WINS, never queued**: a new edit
  invalidates the pending auto-run timer (the existing stale-drop
  pattern); an in-flight run, if superseded by a newer edit before it
  applies, is dropped by the run-token check that already guards
  `run()` (`if (token !== runToken.current) return`).
- **Parked/Resume fate on a new auto-run: DROP the parked frame.**
  Recommendation pinned: any auto-run (case switch or edit) first
  `runner.dropFrame()` (the law selectCase already follows: "case
  switches must not inherit machines") and resets `parked=false`,
  `canResume=false` — a Resume button pointing at a frame from a
  different source is a correctness trap. The park→Resume lesson stays
  available as the explicit sequence Run → park → Resume; the trap stat
  honestly resets.
- **Fuel stance: reduced auto budget, full fuel on ▶ Run.**
  Recommendation pinned: auto-runs execute at a reduced fuel budget
  (candidate: 1M) while the fuel box and the ▶ Run button keep the
  user's full budget (default 10M). The chip already names the fuel it
  verified at (`✓ matches expected @ 1M fuel`) so the reduction is
  honest, never silent. Nice property, verified against the case table:
  `fuel-demo`'s inline expected (`Trap::OutOfFuel`) matches at ANY
  budget — auto-runs keep its chip green, and the diff-on-resume story
  is unchanged.
- **The ▶ Run button STAYS** at full budget; auto-runs never touch the
  budget inputs.
- **The chip updates per auto-run** — every auto-run verifies against
  the case's inline expected exactly like an explicit run (same
  `verifyAgainstExpected` path), so an edit that breaks the contract
  shows `✗` while typing settles.

## 3. fix designs + verification recipes (phase 1 input)

Every recipe = exact drive + assertion; screenshots land in the phase-1
scratch as before/after pairs named `<id>-before.png` (already in
`p0/shots/`) and `<id>-after.png`.

**F1 (H-1, seed 1) — translucent selection.**
Change: `.editor-input::selection { background: rgb(111 179 255 / 0.25);
color: transparent }` (one rule).
Drive: load → programmatic `setSelectionRange(4, 24)` + a real
double-click selection → screenshot.
Assert: `getComputedStyle(ta,'::selection').backgroundColor` has
alpha < 1; the after screenshot shows glyphs inside the selection band;
squiggle underlines still visible through a selection that covers one;
caret still visible. Pairs: `04-selection-doubleclick.png` → after.

**F2 (H-2) — the height chain.**
Change: `min-height: 0` on `.editor` and `.editor-gutter` (CSS only).
Drive: load → set the 1548-line source → settle 1.5 s → probe →
`ta.scrollTop = 999999` → probe → screenshot.
Assert: `document.documentElement.scrollHeight` ≤ viewport + 1;
`ta.clientHeight ≈ editor-body height` (≈ 810, not 27 884);
`ta.scrollHeight - ta.clientHeight` > 10 000; after the scrollTop set,
`ta.scrollTop > 1000` AND `overlay.scrollTop === ta.scrollTop` AND
`gutter.scrollTop === ta.scrollTop` (the existing sync engages);
after-screenshot shows gutter + overlay + caret aligned at the bottom
with the status bar in place. Pairs: `19-scroll-clean-bottom.png` → after.

**F3 (H-3) — un-trap the keyboard.**
Change: `Editor.onKeyDown` guards `!e.shiftKey && !e.ctrlKey &&
!e.metaKey`; Escape blurs.
Drive (keyboard-only): body → Tab to a case button → Enter (activates) →
Tab into the editor → type (Tab inserts two spaces, focus stays) →
Shift+Tab (focus returns to the case list) → Tab forward through: editor
→ pane tabs → Run → Resume → fuel → heap.
Assert: the recorded stop sequence contains Run/Resume/inputs/pane tabs
after the editor; Shift+Tab in the textarea does NOT change
`value.length`; Tab in the textarea still inserts two spaces.
(Reuses the stage-3 stop-walker.)

**F4 (M-1 + seed 2) — memo boundaries + debounce tune.**
Change: memoized overlay line rows keyed on span-array identity;
incremental per-line re-cut in the highlight effect; `memo()` on
CaseList/Panes/StatusBar; debounce 150 → 120 ms (re-tune from the
re-drive's catch-up numbers).
Drive: the §2 M-1 probe verbatim (control + hello + big-doc bursts +
cpuprofile) on the phase-1 build.
Assert: hello median ≈ floor; big-doc median ≤ floor + 10 ms; zero
longtasks; profile shows NO `jsxDEV` overlay-subtree reconciliation per
keystroke (only the edited line's row re-renders); overlay catch-up
measured and recorded (target: feels live; the number is the record,
not a gate); **the smoke's overlay-spans-reconstruct-EXACTLY law stays
green for all 25 cases** (the smoke drives the same builder).
Pairs: `typing-measurements.json` (before) → after.

**F5 (D-1, seed 3) — auto-run.**
Change: App wires the pinned §2-D-1 design (immediate run on case
switch; debounced LAST-WINS auto-run on edit with dropFrame + parked
reset; auto fuel = 1M, Run keeps the box budget; chip per auto-run).
Drive: select `hello, format` → panes populate + chip ✓ without clicking
Run; select `fuel demo` → parks instantly, chip ✓ @ 1M, Resume enabled;
Resume → accumulates, chip ✗ by design; edit `n=41+1` → `n=41+2` →
without touching Run the chip flips ✗ with the diff; rapid-type 10 chars
→ exactly ONE auto-run lands after the settle (count via the run token /
a pane mutation counter), in-flight superseded; edit while parked →
parked state resets, Resume disabled.
Assert: each step's chip text + fuel-used + parked/resume-disabled
state; the Run button remains and still runs at the box budget
(10M) with its own chip line.

**F6 (L-2) — wording.** `"got vs sidecar"` → `"got vs expected"`,
StatusBar titles likewise. Assert: grep the built bundle for
`sidecar` → zero UI strings (README/docs prose excepted).

## 4. the phase plan

- **Phase 1 (one batch, clustered):** F1 + F2 + F3 (the three REDs,
  all CSS/handler-sized) + F4 (memo boundaries + debounce tune) + F5
  (auto-run per the pinned design) + F6 (wording ride-along). Gates to
  keep green: `npm run smoke` (273 — the overlay law + the real-run
  law + the no-sidecars gate), `npm run build`, `tsc`. The re-drive:
  this exact journey script re-run, all rows GREEN, before/after
  screenshot pairs per fix, the M-1 probe re-measured and recorded.
- **Phase 2 (close-out):** the batch report — the journey table's
  before/after, the measured latency deltas, the menu (production
  React in dist; overlay windowing if the memo pass misses the bar;
  the delta-analyze lane rejected by §2 M-1's numbers, recorded upstream
  per the binding's "gaps recorded, not worked around" law).
- **Explicitly NOT in this batch:** engine changes (rut-lsp/rut-wasm
  stay untouched — §2 M-1's numbers kill the justification), new runtime
  deps (no CodeMirror; the noted upgrade path stays noted).

## 5. deviations

1. **agent-browser → raw-CDP fallback** (disclosed in §0): the desktop
   browser session was not connected; the drive used the session's own
   chrome-for-testing over a flat CDP session with real input events.
   Everything the journey needs (screenshots, key/mouse events,
   evaluation, CPU profiling) was available over CDP; nothing was
   skipped for it.
2. **The boot-error panels were not driven** (runner-error and
   lsp-error): exercising them requires deleting/breaking the wasm
   artifacts, i.e. touching files this phase must not touch. The
   panels' code paths are unchanged since the phase that drove them
   (demo-real-run-report); recorded here rather than re-driven
   destructively.
3. **Two driver-side retires, kept honest:** stage 2's first restore
   botched the source (`;;`) — kept as the failing-compile evidence
   (D-2) and redone cleanly (16-run-fail-diff); the first
   below-fold/Enter attempts used coordinate clicks / `rawKeyDown`
   Enter which land differently than real input — redone with
   `scrollIntoView` + `keyDown` + `'\r'` and the corrected results are the
   ones recorded (Enter/Space DO activate case buttons; H-3's trap is
   independent of that fix).
4. **The scratch tooling got a `ws` npm install under
   `/tmp/opencode/batch-demo-journey/p0/`** — the CDP client's only
   dependency; the repo's dependency tree is untouched (no-new-deps law
   applies to the repo, and the demo's `package.json` is unmodified).
