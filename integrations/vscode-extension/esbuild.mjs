// bundle out/extension.js + out/wasm.js — mirrors the repo's
// minimal-tooling ethos. The second entry is the same src/wasm.ts ABI
// binding the extension ships, emitted standalone so test/e2e-wasm.js
// can drive the shipped bin/rut-lsp.wasm through it (the through-wasm
// e2e gate exercises the REAL binding, not a copy).
import * as esbuild from 'esbuild';

const ctx = await esbuild.context({
  entryPoints: ['src/extension.ts', 'src/wasm.ts'],
  bundle: true,
  outdir: 'out',
  external: ['vscode'],
  format: 'cjs',
  platform: 'node',
  target: 'node18',
  sourcemap: true,
});

if (process.argv.includes('--watch')) {
  await ctx.watch();
} else {
  await ctx.rebuild();
  await ctx.dispose();
}
