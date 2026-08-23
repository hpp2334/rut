# rut demo page

The wasm playground for the rut language (RFC 0041 §3): pick a prepared
case or edit your own, run it, and inspect **Output / AST / IR** — every
run under an explicit fuel + heap budget (RFC 0040).

## Quickstart

```
cd demo
npm install
npm run dev        # http://localhost:8080
npm run build      # -> dist/
```

## Modes

- **wasm mode**: the page probes `GET /rut.wasm` at boot. When a rut
  build is dropped in at `public/rut.wasm` it is fetched, instantiated,
  and used for real `compile`/`run` calls (see `src/wasm/rut-api.d.ts`).
- **static-preview mode**: with no wasm artifact, prepared cases show
  their annotated expected output with a banner; AST/IR panes show a
  placeholder. The page exercises the *spec* today.

## The wasm contract

`src/wasm/rut-api.d.ts` is the source of truth:

```ts
compile(src): { diags, astDump, irDump, binary? }
run(binary, { fuel, heapBytes }): { output, trap?, fuelUsed, heapBytes }
```

The demo host exposes one native to the guest: `print(s: string)`.
Default budgets: 10M fuel / 4 MiB heap. Case 8 intentionally loops
forever to demonstrate `Trap::OutOfFuel` + the resume button.

## Layout

React + rspack + TypeScript (RFC 0041 §3) — no editor dependency; a
plain textarea component (CodeMirror is a noted upgrade path):

```
src/
  main.tsx               createRoot bootstrap
  App.tsx                layout, run/resume wiring, budget state
  cases.ts               prepared cases (name, blurb, source, expected[])
  runner.ts              RutApi resolution: wasm -> stub fallback
  wasm/rut-api.d.ts      the compile/run contract
  components/
    CaseList.tsx         case selector
    Editor.tsx           textarea + line-number gutter (tab = 2 spaces)
    Panes.tsx            Output / AST / IR tabs
    StatusBar.tsx        run/resume, fuel+heap inputs, telemetry
```
