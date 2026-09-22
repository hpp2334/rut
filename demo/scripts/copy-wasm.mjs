// Copy the cargo-built wasm artifacts into public/ — the extension's
// copy-wasm.mjs twin (the lsp-align loud-fail law applied to the demo):
// a missing artifact is a LOUD failure naming the exact build command,
// never a raw ENOENT stack, never a silent stale copy.
//
//   (no arg)   copy BOTH artifacts — rut.wasm + rut-lsp.wasm
//   --lsp      copy only rut-lsp.wasm (the build:lsp lane)
import { copyFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const lspOnly = process.argv.includes('--lsp');
const TARGET_DIR = join('..', 'target', 'wasm32-unknown-unknown', 'release');

const ARTIFACTS = [
  {
    src: join(TARGET_DIR, 'rut_wasm.wasm'),
    dest: join('public', 'rut.wasm'),
    build: 'cargo build -p rut-wasm --target wasm32-unknown-unknown --release',
  },
  {
    src: join(TARGET_DIR, 'rut_lsp_wasm.wasm'),
    dest: join('public', 'rut-lsp.wasm'),
    build: 'cargo build -p rut-lsp-wasm --target wasm32-unknown-unknown --release',
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
