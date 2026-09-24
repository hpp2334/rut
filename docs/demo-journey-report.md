# demo-journey-fixes — the batch report

The batch: the playground's user journey — pick a case, read highlighted
source, edit, run, resume — actually works end to end. Three phases, base
`e88be67` (the no-sidecars close-out), all work on `master`. The three
seeds: (1) the selection erases the selected text, (2) live editing feels
slow (measure first), (3) auto-run on case switch + edit. Audit of
record: `docs/demo-journey-audit.md` (phase 0, driven + measured).

The phases: `cebf14e` (the audit — the journey driven end-to-end over raw
CDP, the census, the latency decomposition, F1–F6 verification recipes),
`0d1a412` (every fix landed), this commit (the record). Zero engine
changes anywhere in the batch — `rut-lsp`/`rut-wasm` untouched, the
delta-analyze lane rejected by measurement rather than built. No new
runtime deps; bench pins untouched.

Scratch (not committed): `/tmp/opencode/batch-demo-journey/p0/` — the
phase-0 drive (25 screenshots, the measurement JSONs, the cpuprofile),
`p1/` — the phase-1 re-drive (26 after-shots, the re-measured probe),
`p2/` — this phase (docs only, no new artifacts).

## 1. The issue table — every census row's disposition

| id | sev | area | found (phase 0) | disposition | receipt |
|---|---|---|---|---|---|
| H-1 | HIGH | editor/selection | selection paints opaque `rgb(43,61,87)` over the overlay — selected text unreadable | **fixed** (F1) | §2.1 |
| H-2 | HIGH | editor/scroll | long sources blow the page out to 27,950 px; the editor never scrolls internally | **fixed** (F2) | §2.2 |
| H-3 | HIGH | a11y/keyboard | the textarea traps Tab AND Shift+Tab; Run/Resume/budget/pane-tabs keyboard-unreachable | **fixed** (F3) | §2.3 |
| M-1 | MEDIUM | perf/typing | +30 ms/keystroke at ~1500 lines; ~27% of active CPU burned by dev-React validators; felt term = the 150 ms debounce | **fixed** (F4) — at the bare-textarea floor | §2.4 |
| D-1 | DESIGN | runs | nothing auto-runs: a case switch leaves `(no output)`, an edit runs nothing | **landed** (F5) per the pinned design | §2.5 |
| L-1 | LOW | perf/build | `dist/` ships development-mode React | **fixed** — rode F4 (the build lane's mode split); menu'd in the census, disclosed here | §2.4 |
| L-2 | LOW | wording | UI strings still said "sidecar" after the storage flip | **fixed** (F6) | §2.6 |
| I-1 | INFO | editor/overlay | overlay windowing for very long docs | **menu'd** — unneeded at the floor; re-open bar recorded | §4 |
| I-2 | INFO | editor/probe | the hidden metrics probe adds 10 phantom chars to the overlay text | **recorded** | §4 |
| I-3 | INFO | engine shape | the one-call `rut_analyze` delta lane | **rejected by measurement** before being built; menu item with numbers | §4 |

The census was 10 issues from 3 seeds — the audit found well beyond what
it was asked (§5).

## 2. The receipts (each: the F-recipe, the numbers, the pair paths)

Screenshot pairs are by path: before in `p0/shots/`, after in
`p1/shots/` (which mirrors the before copies side by side). All paths
relative to `/tmp/opencode/batch-demo-journey/`.

### 2.1 F1 (H-1) — the translucent selection

- **Change**: one CSS rule — `.editor-input::selection { background:
  rgb(111 179 255 / 0.25); color: transparent }`. The overlay's colored
  glyphs and the squiggle underlines show through the band; the caret is
  unaffected. The mirrored-pre and projected-classes directions were
  rejected in the audit with reasons.
- **Before**: `::selection` computed fully opaque `rgb(43, 61, 87)`; the
  textarea's own text is transparent and the overlay never paints a
  selection of its own — any selection rendered as a solid dark block
  with zero visible glyphs.
- **After**: computed alpha 0.25; glyphs legible inside the band; a
  squiggle covered by the selection still shows its underline.
- **Pairs**: `p0/shots/03-selection-programmatic.png` +
  `p0/shots/04-selection-doubleclick.png` →
  `p1/shots/03-selection-programmatic-after.png` +
  `p1/shots/04-selection-doubleclick-after.png`.

### 2.2 F2 (H-2) — the height chain restored

- **Change**: `min-height: 0` on `.editor` (THE fix — the grid item's
  default `min-height: auto` let content height beat its row) and
  defensively on `.editor-gutter` (the flex item). CSS only; the
  existing gutter/overlay JS scroll-sync engages exactly as written.
- **Before (measured)**: `document.scrollHeight` **27,950 px** (viewport
  900); `.editor`/`.editor-body`/textarea/overlay all **27,884 px**;
  textarea `scrollHeight == clientHeight` → `scrollTop` clamps to 0 —
  the editor could never scroll, the WINDOW scrolled the header, case
  list, panes, and status bar away, and editor text overlapped the
  status bar.
- **After (measured, a 1548-line source)**: document scrollHeight
  **900 = the viewport**; the textarea carries ~**27,100 px** of
  internal overflow and is the real scroller; at max scroll
  `textarea.scrollTop == overlay.scrollTop == gutter.scrollTop` EXACTLY,
  with the header/case-list/panes/status-bar in place.
- **Pairs**: `p0/shots/13-longsource-scrolled-bottom.png`,
  `17-scrolled-bottom-verified.png`, `18-scroll-after-edit.png`,
  `19-scroll-clean-bottom.png` → `p1/shots/19-scroll-clean-bottom-after.png`,
  `p1/shots/F2-scroll-bottom-after.png`.
- **The ride-along the re-drive forced**: the overlay's final-row
  separator now mirrors the value's trailing newline (the `<pre>` end
  tag swallows one), so the pre's content is byte-exact with the
  textarea value — that byte-exactness is what makes the scroll
  equality land at max scroll. Caught by F2's own assertion, fixed in
  phase 1, ride-along disclosed.

### 2.3 F3 (H-3) — the keyboard trap dies

- **Change**: `Editor.onKeyDown` captures ONLY unmodified Tab
  (`!e.shiftKey && !e.ctrlKey && !e.metaKey`); Escape blurs the editor
  as the forward hatch.
- **Before (measured)**: the handler matched `e.key === 'Tab'` with no
  modifier check — Shift+Tab inserted two spaces too; 40 consecutive
  Tab presses never left the textarea; Run, Resume, the fuel/heap
  inputs, and the pane tabs were unreachable by keyboard.
- **After (recorded walk, keyboard-only)**: editor → heap → fuel →
  Resume → Run → Resume → fuel → heap → Run → IR → AST → Output.
  Shift+Tab leaves the textarea WITHOUT touching the value; unmodified
  Tab still inserts two spaces (by design); Enter/Space activate case
  buttons (already true at phase 0).
- **Pairs**: phase-0 stage-3 probes (the audit §H-3 measurements) →
  `p1/shots/F3-keyboard-after.png`.

### 2.4 F4 (M-1 + L-1) — latency: the fixes in the audit's measured order

- **Change**: memoized `OverlayLine` rows keyed on span-array IDENTITY,
  fed by a NEW incremental `buildOverlayCached` in `overlay.ts` (line
  text AND both mark sets unchanged keeps the previous array — a
  keystroke re-cuts and re-renders only the edited line); `memo()` on
  CaseList/Panes/StatusBar with the panes' AST source folded into
  `PaneData.astSrc` (the tree and its literal slices ship as ONE
  version — a keystroke re-renders the editor only); the debounce
  150 → 120 ms; **production React in `dist/`** via the build lane's
  mode split (L-1: menu'd in the census, landed as this ride-along).
- **End-to-end keystroke frame latency** (median / p90 / max, longtasks):

  | scenario | before (phase 0) | after (phase 1) |
  |---|---|---|
  | control: bare textarea (the floor) | 29.2–30.3 / ~30.8 / 44.7 | 30.5 / 44.6 / 45.8 |
  | app, `hello` (18 lines) | 29.9 / 45.8 / 49.8 | 30.2 / 45.1 / 45.5 |
  | app, `quicksort` + AST pane | 30.6 / 45 / 45.8 | not re-run (at floor before, unchanged shape) |
  | app, ~1530-line / 31 KB doc | **60.8** / 60.3 / 67.4 | **30.0** / 45.1 / 45.6 |

  Zero longtasks in every run, before and after. The big-doc tax
  (+30 ms/keystroke — death by a thousand 30 ms frames, never one long
  stall) is **gone**: median AT the control floor. Overlay catch-up
  after the last key: ~150 ms before → **94.2 ms (hello) / 117.4 ms
  (big)** after (the 120 ms debounce plus one lane turn).
- **The profile receipt**: phase 0's big-doc burst profile showed the
  active time dominated by whole-overlay reconciliation (`FiberNode`
  60.9 ms, `reconcileChildrenArray`/`jsxDEV`/`diffProperties`) with
  **~27% of active CPU burned by development-mode React validators**
  (`warnInvalidARIAProps` 17.6 ms, `defineKeyPropWarningGetter` 17.3 ms,
  `warnOnInvalidKey` 14.9 ms …). The phase-1 profile over the same
  burst: **`jsxDEV` and the dev-validator family measure 0 hits**;
  during the keystroke window the overlay mutates only AFTER the settle
  (the probe's `ovMutDuring`: 1–2, all post-settle) — the per-keystroke
  reconcile storm is gone.
- **The engine was never the cost** (why nothing upstream changed): the
  pipeline micro-decomposition stands as measured — envelope <0.05 ms,
  `rut_analyze` 0.10 ms at 18–26 lines / 9.30 ms at 1560, `JSON.parse`
  0.40, `buildOverlay` 1.40, total 11.2 ms at 1560 lines, once per
  pause, off the keystroke path. Zero profile samples in the engine
  then and now.
- **Pairs**: `p0/typing-measurements.json` + `p0/typing-burst-big.cpuprofile`
  → `p1/typing-measurements.json` + `p1/typing-burst-big.cpuprofile`.

### 2.5 F5 (D-1) — auto-run, the pinned guards, and what the drive caught

- **The landed contract** (the audit's §2-D-1 design, verbatim):
  - **Case switch runs immediately** — panes + chip populate the moment
    a case opens, no click. Auto-runs execute at the fixed
    `AUTO_FUEL = 1M` budget while the fuel box keeps the user's budget;
    the chip names the fuel it verified at, so the reduction is honest,
    never silent.
  - **Edits auto-run on the same 120 ms debounce lane** as the highlight,
    LAST-WINS: a rapid keystroke burst lands EXACTLY ONE run (the
    skip-ref also keeps a case's own immediate run from being
    duplicated and boot inert). An edit retires the chip at once (the
    verdict belongs to the last run); the settled auto-run posts a
    fresh verdict — break the case's contract while typing and the ✗ +
    line-paired diff appear without touching Run.
  - **Any auto-run drops a parked frame** (`dropFrame()` +
    `parked=false`): a Resume can never point at a frame from a foreign
    source. The park→Resume lesson stays available as the explicit
    ▶ Run → park → Resume sequence.
  - **▶ Run is unchanged**: full fuel-box budget (default 10M), its chip
    names the box.
- **The drive caught a real bug before it shipped**: the naive
  implementation verified each auto-run against a closure over the
  PREVIOUS case's expected — the F5 drive's case-switch step showed the
  chip verifying the fresh case against the old contract. Fix: `applyRun`
  takes the run's OWN expected as a parameter; every auto-run carries
  the case it belongs to.
- **Assertions, shot by shot** (`p1/shots/`):
  `F5-case-switch-after.png` (hello populates + ✓ with no click),
  `F5-parked-1M-after.png` (fuel-demo parks on SELECTION, chip ✓ @ 1M —
  its expected pins the trap, which matches at any budget — Resume
  enabled), `11-resumed-ticks-after.png` (Resume accumulates → the
  by-design diff), `F5-edit-diff-after.png` (`n=41+2` → ✗ + diff, no
  Run), `F5-edit-unparks-after.png` (an edit while parked drops the
  frame, Resume disabled), `F5-rapid-one-run-after.png` (a 10-char
  burst → exactly one run), `F5-run-box-budget-after.png` (Run at the
  box budget, chip names the box).

### 2.6 F6 (L-2) — the wording rides the inline truth

- **Change**: the retired-storage strings now say so — the diff head
  `(got vs expected)` (was `got vs sidecar`), `<no expected line>`, the
  StatusBar chip titles, `verify.ts`'s prose.
- **Assertion**: grep of the built bundle for `sidecar` → zero.
- **Pairs**: n/a (string identity, grep-receipted).

## 3. The journey checklist — final state

The audit's §1 journey table, re-driven end-to-end on the fixed build
(same raw-CDP method, zero page errors):

| step | phase-0 verdict | final verdict | evidence |
|---|---|---|---|
| boot | GREEN | GREEN (unchanged) | `p1/shots/01-boot-after.png` |
| select inline case | GREEN | GREEN + auto-runs (`F5-case-switch-after.png`) | |
| select classic | GREEN | GREEN (unchanged) | `p1/shots/02-case-classic-*-after.png` |
| select gap-filler | GREEN | GREEN (unchanged) | `p1/shots/12-case-maps-belowfold-after.png` |
| read highlighted source | GREEN | GREEN; the I-2 probe remains the only textContent delta | |
| edit → squiggle flow | GREEN | GREEN + the settled auto-run re-verdicts the chip | `p1/shots/05-squiggle-after.png`, `F5-edit-diff-after.png` |
| explicit Run | GREEN | GREEN + still the full-box budget | `p1/shots/20-clean-run-output-after.png`, `F5-run-box-budget-after.png` |
| failing expectation | GREEN | GREEN (unchanged) | `p1/shots/16-run-fail-diff-after.png` |
| a failing COMPILE | GREEN | GREEN (unchanged) | `p1/shots/06-run-hello-after.png`, `08-run-ir-after.png` |
| Resume flow | GREEN | GREEN + parks on selection at the auto budget | `p1/shots/10-parked-outoffuel-after.png`, `F5-parked-1M-after.png`, `11-resumed-ticks-after.png` |
| selection legibility | **RED (H-1)** | **GREEN** | §2.1 pairs |
| scroll at long sources | **RED (H-2)** | **GREEN** | §2.2 pairs |
| keyboard-only pass | **RED (H-3)** | **GREEN** | §2.3 pairs |
| typing latency | **AMBER (M-1)** | **GREEN** — at the bare-textarea floor | §2.4 pairs |
| auto-run | **ABSENT (D-1)** | **GREEN** — the full pinned contract | §2.5 shots |

## 4. The menu (recorded, not worked around)

1. **I-1 — overlay windowing: unneeded at the floor.** The audit's bar
   was "only if the memo pass doesn't bring 1500-line keystroke cost to
   ≤ floor + 10 ms". The memo pass landed at floor − 0.5 ms (30.0 vs
   30.5) — the bar is not met, so windowing was NOT built (the phase-1
   scope forbade landing it without its own measurement).
   **Re-open bar**: if a future change puts big-doc keystroke latency
   above control + 10 ms again, painting only the visible line window
   ± buffer is the next lever — and H-2 already made scrolling (hence
   the visible window) real, so the design is ready when the bar trips.
2. **I-2 — the phantom chars.** The hidden metrics probe (the hover
   tip's monospace ruler) appends `0000000000` inside the aria-hidden
   overlay, so `overlay.textContent ≠ textarea.value` by exactly 10
   chars. Harmless today (the smoke's law reconstructs spans, not
   textContent), recorded for any future full-text-equality test.
3. **I-3 — the delta-analyze lane: rejected by measurement BEFORE being
   built.** The phase-0 decomposition showed `rut_analyze` at 0.10 ms
   (case sizes) / 9.30 ms (1560 lines), once per pause, envelope
   <0.05 ms — there is no measured case where the one-call shape is the
   bottleneck. The lane stays a menu item with these numbers attached,
   per the binding's "gaps recorded, not worked around" law.
4. **Standing menu items, untouched**: CodeMirror (the noted-not-taken
   editor upgrade — the zero-dep double-layer passed this batch's drive
   with no drift), per-document expected authoring, host futures — see
   the MENU in `docs/demo-real-run-report.md`.

## 5. The honest story

- **The audit found beyond the seeds.** Seed 1 was only the selection —
  the drive's census surfaced H-2 (a 27,950 px page: the scroll-sync
  was WRITTEN and structurally disarmed by one missing `min-height: 0`)
  and H-3 (a full keyboard trap: Shift+Tab typed spaces, and the entire
  lower UI was unreachable without a mouse). Seed 2's "slow typing"
  decomposed into something the engine was never guilty of: a React
  reconciliation storm with a 27% development-mode-validator tax on
  top — and felt lag at normal sizes that was 100% the debounce.
- **The drive caught D-1's stale closure.** The auto-run design was
  pinned in phase 0, and phase 1's first implementation still shipped a
  bug the F5 recipe was built to catch: each auto-run verified against
  the previous case's expected. The recipe's case-switch assertion
  failed, the closure was replaced by an explicit per-run expected, and
  the shot records the fixed behavior.
- **The re-drive forced a ride-along.** F2's max-scroll equality failed
  until the overlay's final-row separator mirrored the textarea's
  trailing newline — a byte-exactness fix the journey produced, not the
  plan.
- **Measurement killed work before it was written.** The delta-analyze
  lane (I-3) was the "obvious" engine-side fix; the decomposition
  showed the engine costs 0.1 ms where the felt problem lives, so it
  was rejected before implementation and stays on the menu with its
  numbers.
- **Method disclosures carried from phase 0**: the raw-CDP drive (the
  desktop browser was not connected; chrome-for-testing headless with
  real input events — the no-sidecars batch's disclosed fallback), the
  boot-error panels left undriven rather than break artifacts, two
  driver-side retires redone and kept honest, and the scratch `ws`
  install outside the repo.

## 6. Gates (this commit)

- docs-only: `docs/demo-journey-report.md` + `demo/README.md`, staged by
  explicit path; zero code changes; the foreign tracked `SKILL.md` mod,
  the untracked monitor files, and the stash untouched;
- the demo's gates green untouched: `npm run smoke` — **273 passed,
  0 failed**; `tsc --noEmit` clean; `npm run build` green;
- workspace: `cargo test --workspace` exit 0;
- wasm32: `cargo check --workspace --target wasm32-unknown-unknown`
  exit 0;
- tree clean apart from this phase's two files; single commit, pushed.
