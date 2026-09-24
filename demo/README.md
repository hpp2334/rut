# rut demo page

The wasm playground for the rut language (RFC 0041 §3): pick a prepared
case or edit your own, run it, and inspect **Output / AST / IR** — every
run under an explicit fuel + heap budget (RFC 0040).

## Quickstart

```
cd demo
npm install
npm run build:wasm  # builds BOTH artifacts (rut.wasm + rut-lsp.wasm) and copies them to public/
npm run dev         # http://localhost:8080 (dev server; react-refresh lives only here)
npm run build       # -> dist/ (static; ships both wasm artifacts alongside the bundle)
npm run smoke       # headless gate: every case really runs + verifies, no browser
```

`dev`/`build` preflight the artifacts (loud, name the exact build
command); `build:wasm` copies them with the same curated loud-fail.

> The static `dist/` build carries no react-refresh machinery — its
> runtime exists only under `rspack serve`. Shipping it in `dist/` used
> to kill the page on load (`ReferenceError: $RefreshReg$ is not
> defined`, an empty `#root`); the phase-4 browser drive caught it and
> the refresh plugin now rides the `dev` lane only (`rspack.config.ts`).
> Serving `dist/` needs any static file server — see "Drive it
> yourself".

## Modes

- **wasm mode (live)**: the page probes `GET /rut.wasm` at boot —
  `npm run build:wasm` drops the artifact at `public/rut.wasm`, and the
  page runs real `compile`/`run` calls through the raw ABI of the
  `rut-wasm` crate (see `src/wasm/rut-api.d.ts` and
  `../crates/rut-wasm/src/lib.rs`). After every run the output is
  diffed against the case's inline expected — the StatusBar chip shows
  ✓/✗ (and a failing verify renders the line-paired diff). The banner
  names the real engine slice mounted into the wasm host.
- **error mode**: there is no preview fallback. A missing/invalid
  artifact boots a full-page panel naming the exact
  `npm run build:wasm` command; the panes never mount. The same law
  covers the highlight artifact: a missing/invalid `rut-lsp.wasm` is
  the page state too (`rut-lsp.wasm missing or invalid`) — highlight
  rides the analyzer or the page says so, never a monochrome shrug
  dressed up as success.

## Auto-run and the editor (the journey batch's visible changes)

Runs happen without the button:

- **Selecting a case runs it immediately** — the panes and the chip
  populate the moment a case opens. Auto-runs use a fixed 1M-fuel auto
  budget and each carries the case's OWN expected, so the chip always
  names the fuel it verified at (`✓ … @ 1M fuel`).
- **Edits auto-run once typing settles** — the same 120 ms debounce
  lane as the highlight, LAST-WINS: a rapid burst lands exactly one
  run. An edit retires the chip at once (the verdict belongs to the
  last run); the settled run posts a fresh verdict, so breaking the
  case's contract while typing shows the ✗ + line-paired diff without
  touching Run.
- **Any auto-run drops a parked frame** — Resume can never point at a
  frame from a foreign source. The park→resume lesson stays an explicit
  ▶ Run sequence; the fuel-demo case parks the moment you select it
  (its expected pins the trap, which matches at any budget, so the
  auto chip stays green).
- **▶ Run itself is unchanged**: it runs at the fuel box's budget
  (default 10M) and its chip names that box.

The editor:

- **Long sources scroll inside the editor** — the textarea is the
  scroller (height-bounded) and the gutter + overlay follow its scroll;
  the page around the editor never scrolls away.
- **Selection is legible** — the band is the accent at 25% alpha, so
  the highlighted glyphs (and any squiggle underneath) show through it.
- **The keyboard model** — unmodified Tab inserts two spaces and keeps
  focus; Shift+Tab walks focus back out of the editor; Escape blurs it.
  Run, Resume, the budget inputs, and the pane tabs are reachable by
  keyboard.

## Drive it yourself (the no-fake acceptance)

The smoke proves the engine slice headlessly; the browser proves the
page. Phase 4's recorded drive (headless Chrome, static server):

```
cd demo && npm run build
(cd dist && python3 -m http.server 8123)   # any static server works
# open http://127.0.0.1:8123/
```

What was verified, state by state (screenshots in the batch report,
`../docs/demo-real-run-report.md`):

1. **boot** — banner reads `live — rut.wasm · …`, all 25 cases listed,
   the overlay already painting LSP tokens;
2. **run-verified** — Run on `hello, format`: real output
   (`hi rut! n=42 tab:` / `sour`), fuel 39 / heap 261 B, chip
   `✓ matches expected @ 10M fuel`;
3. **squiggle** — `let n = 41 + 1;` → `41 + ;`: the offending `;`
   renders a wavy underline titled
   `expected an expression, found \`;\``; the chip retires on edit
   (the verdict belongs to the last run — the journey batch's auto-run
   then re-verdicts it once typing settles);
4. **maps-case** — the gap-filler `maps & sets`: a real run through
   the nmap lane (HashMap/PrimMapI64/HashSet), verified green, fuel
   638 / heap 545 B.

The demo-journey batch later re-drove this whole journey end-to-end
(before/after pairs for every fix): `../docs/demo-journey-report.md`.

## The wasm contract

`src/wasm/rut-api.d.ts` is the source of truth:

```ts
compile(src): { diags, ast, irDump, binary? }
run(binary, { fuel, heapBytes }): { output, trap?, fuelUsed, heapBytes, parked? }
resume(extraFuel): { ... }        // continues the PARKED frame — real
                                  // park/resume (rut_resume), output
                                  // accumulates, never a re-run
dropFrame(): number               // retire a parked frame (case switch,
                                  // any auto-run)
```

The demo host exposes the logger (+ calc's math) to the guest.
Default budgets: 10M fuel / 4 MiB heap. Case 8 intentionally loops
forever to demonstrate `Trap::OutOfFuel` + the resume button — and its
expected pins the DEFAULT budget (zero ticks fit in 10M), so a raised
fuel box or a Resume shows a verify diff BY DESIGN.

## Layout

React + rspack + TypeScript (RFC 0041 §3) — no editor dependency; the
classic double-layer textarea (CodeMirror is a noted upgrade path):

```
src/
  main.tsx               createRoot bootstrap
  App.tsx                layout, run/resume wiring, budget state, boot-error
                         panels (runner + LSP), the debounced re-analyze
  cases.ts               prepared cases (name, blurb, source, expected[])
  verify.ts              the real-run vs expected diff (survey D2's sidecar flip)
  examples/              the classics — real .rut files, expected inline
    index.ts             metadata + raw .rut imports (asset/source) + the
                         inline expected blocks (the retired sidecars' bytes,
                         verbatim); playground order
  runner.ts              wasm-or-error resolution (no fallback)
  lsp/
    rut-lsp.ts           the standalone rut-lsp.wasm binding (survey D3):
                         rut_begin/alloc envelopes, ONE analyze call per change
    overlay.ts           tokens+diags -> overlay spans (pure, smoke-driven)
  wasm/rut-api.d.ts      the compile/run/resume contract
  smoke/smoke-entry.ts   the smoke bundle's exports (the app's own surface)
  components/
    CaseList.tsx         case selector (groups: cases, classics)
    Editor.tsx           transparent textarea OVER a highlighted <pre> overlay
                         (same metrics, scroll-synced; squiggles + hover tip)
    Panes.tsx            Output / AST / IR tabs + compile diags + verify diff
    StatusBar.tsx        run/resume, fuel+heap inputs, verify chip, telemetry
scripts/
  preflight-wasm.mjs     dev/build artifact preflight (loud, both artifacts)
  copy-wasm.mjs          build:wasm's curated copy step (loud)
  smoke.mjs              THE SMOKE: headless gate over the shipped artifacts
rspack.config.ts         the page bundle (refresh = dev lane only — see the
                         callout above)
rspack.smoke.config.ts   bundles the app surface for node (dist-smoke/, gitignored)
```

## Honest limits

- **Custom edits verify against the case they came from.** A free-form
  edit that still matches the case's expected stays green; one that
  doesn't shows the diff. There is no per-document expected authoring —
  that is the playground-editing menu item (see the batch report).
- **The wasm host mounts a slice, not the whole std**: core, calc, rt,
  ink, pouch, nmapset (+ the nmap host bindings). `select`/`await`
  parse but have no host futures in this host — never taught, honestly.
- **dist/ ships production React** (the build lane's mode split —
  `npm run dev` keeps react-refresh + dev React for iteration). The
  development-mode validators that used to burn ~27% of active CPU on
  big-doc keystrokes are gone from the shipped bundle; switching the
  mode was its own measured pass, not a silent change.
- **The highlight is one `rut_analyze` per keystroke-debounce** (120
  ms, full-sync, whole doc). Doc-sized sources make that milliseconds
  (measured 9.3 ms at 1560 lines); there is no delta endpoint upstream,
  and the measured numbers keep it a menu item, not a silent change.
- **Classifier gaps are recorded, not worked around** — see the MENU
  in `../docs/demo-real-run-report.md`.
