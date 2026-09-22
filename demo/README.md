# rut demo page

The wasm playground for the rut language (RFC 0041 §3): pick a prepared
case or edit your own, run it, and inspect **Output / AST / IR** — every
run under an explicit fuel + heap budget (RFC 0040).

## Quickstart

```
cd demo
npm install
npm run build:wasm  # cargo build -p rut-wasm (wasm32) + copy to public/rut.wasm
npm run dev         # http://localhost:8080
npm run build       # -> dist/ (ships rut.wasm alongside the bundle)
npm run smoke       # headless gate: every case really runs + verifies, no browser
```

`dev`/`build` preflight the artifact (loud, names the exact build
command); `build:wasm` copies it with the same curated loud-fail.

## Modes

- **wasm mode (live)**: the page probes `GET /rut.wasm` at boot —
  `npm run build:wasm` drops the artifact at `public/rut.wasm`, and the
  page runs real `compile`/`run` calls through the raw ABI of the
  `rut-wasm` crate (see `src/wasm/rut-api.d.ts` and
  `../crates/rut-wasm/src/lib.rs`). After every run the output is
  diffed against the case's sidecar — the StatusBar chip shows
  ✓/✗ (and a failing verify renders the line-paired diff).
- **error mode**: there is no preview fallback. A missing/invalid
  artifact boots a full-page panel naming the exact
  `npm run build:wasm` command; the panes never mount.

## The wasm contract

`src/wasm/rut-api.d.ts` is the source of truth:

```ts
compile(src): { diags, ast, irDump, binary? }
run(binary, { fuel, heapBytes }): { output, trap?, fuelUsed, heapBytes, parked? }
resume(extraFuel): { ... }        // continues the PARKED frame — real
                                  // park/resume (rut_resume), output
                                  // accumulates, never a re-run
dropFrame(): number               // retire a parked frame (case switch)
```

The demo host exposes the logger (+ calc's math) to the guest.
Default budgets: 10M fuel / 4 MiB heap. Case 8 intentionally loops
forever to demonstrate `Trap::OutOfFuel` + the resume button — and its
sidecar pins the DEFAULT budget (zero ticks fit in 10M), so a raised
fuel box or a Resume shows a verify diff BY DESIGN.

## Layout

React + rspack + TypeScript (RFC 0041 §3) — no editor dependency; a
plain textarea component (CodeMirror is a noted upgrade path):

```
src/
  main.tsx               createRoot bootstrap
  App.tsx                layout, run/resume wiring, budget state, boot-error panel
  cases.ts               prepared cases (name, blurb, source, expected[])
  verify.ts              the sidecar flip: real-vs-expected diff (survey D2)
  examples/              the classics — real .rut files + .expected sidecars
    index.ts             metadata + raw imports (asset/source); playground order
  runner.ts              wasm-or-error resolution (no fallback)
  wasm/rut-api.d.ts      the compile/run/resume contract
  smoke/smoke-entry.ts   the smoke bundle's exports (the app's own surface)
  components/
    CaseList.tsx         case selector (groups: cases, classics)
    Editor.tsx           textarea + line-number gutter (tab = 2 spaces)
    Panes.tsx            Output / AST / IR tabs + compile diags + verify diff
    StatusBar.tsx        run/resume, fuel+heap inputs, verify chip, telemetry
scripts/
  preflight-wasm.mjs     dev/build artifact preflight (loud)
  copy-wasm.mjs          build:wasm's curated copy step (loud)
  smoke.mjs              THE SMOKE: headless gate over the shipped artifact
rspack.smoke.config.ts   bundles the app surface for node (dist-smoke/, gitignored)
```
