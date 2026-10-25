// bundle out/extension.js + out/wasm.js — mirrors the repo's
// minimal-tooling ethos. The second entry is the same src/wasm.ts ABI
// binding the extension ships, emitted standalone so test/e2e-wasm.js
// can drive the shipped bin/rut-lsp.wasm through it (the through-wasm
// e2e gate exercises the REAL binding, not a copy).
//
// The binding imports the wasm-bindgen glue from crates/rut-lsp-wasm/pkg
// (gitignored build output) — a missing glue is the loud missing-build
// failure naming the exact command, never a raw resolver error.
import { existsSync } from 'node:fs';
import * as esbuild from 'esbuild';

const GLUE = '../../crates/rut-lsp-wasm/pkg/rut_lsp_wasm.js';
if (!existsSync(GLUE)) {
  console.error(
    `wasm-bindgen glue not found: ${GLUE} — run ` +
      `\`npm run build:wasm\` first (wasm-pack build crates/rut-lsp-wasm --target web --release)`
  );
  process.exit(1);
}

const ctx = await esbuild.context({
  entryPoints: ['src/extension.ts', 'src/wasm.ts'],
  bundle: true,
  outdir: 'out',
  external: ['vscode'],
  format: 'cjs',
  platform: 'node',
  target: 'node18',
  sourcemap: true,
  // the glue's `new URL('..._bg.wasm', import.meta.url)` fallback never
  // runs (the binding always passes the artifact bytes to init), but
  // esbuild cannot lower import.meta into CJS output — define it away
  define: { 'import.meta.url': 'undefined' },
});

if (process.argv.includes('--watch')) {
  await ctx.watch();
} else {
  await ctx.rebuild();
  await ctx.dispose();
}
