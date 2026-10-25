// Copy the wasm-pack-built wasm artifacts into public/ — the
// extension's copy-wasm.mjs twin (the lsp-align loud-fail law applied
// to the demo): a missing artifact is a LOUD failure naming the exact
// build command, never a raw ENOENT stack, never a silent stale copy.
// The glue (pkg/*.js) is NOT copied: rspack bundles it straight from
// the pkg dirs into the page bundle.
//
//   (no arg)   copy BOTH artifacts — rut.wasm + rut-lsp.wasm
//   --lsp      copy only rut-lsp.wasm (the build:lsp lane)
import { copyFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const lspOnly = process.argv.includes('--lsp');
const PKG_DIR = join('..', 'crates');

const ARTIFACTS = [
  {
    src: join(PKG_DIR, 'rut-wasm', 'pkg', 'rut_wasm_bg.wasm'),
    dest: join('public', 'rut.wasm'),
    build: 'wasm-pack build ../crates/rut-wasm --target web --release',
  },
  {
    src: join(PKG_DIR, 'rut-lsp-wasm', 'pkg', 'rut_lsp_wasm_bg.wasm'),
    dest: join('public', 'rut-lsp.wasm'),
    build: 'wasm-pack build ../crates/rut-lsp-wasm --target web --release',
  },
];

for (const a of ARTIFACTS) {
  if (lspOnly && a.dest !== join('public', 'rut-lsp.wasm')) continue;
  if (!existsSync(a.src)) {
    console.error(
      `wasm module not found: ${a.src} — run ` +
        `\`npm run build:wasm\` in demo/ (builds both artifacts; ` +
        `this one with: \`${a.build}\`)`
    );
    process.exit(1);
  }
  copyFileSync(a.src, a.dest);
  console.log(`copied ${a.src} -> ${a.dest}`);
}
