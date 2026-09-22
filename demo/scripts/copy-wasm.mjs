// Copy the cargo-built rut.wasm into public/ — the extension's
// copy-wasm.mjs twin (the lsp-align loud-fail law applied to the demo):
// a missing artifact is a LOUD failure naming the exact build command,
// never a raw ENOENT stack, never a silent stale copy.
import { copyFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const src = join('..', 'target', 'wasm32-unknown-unknown', 'release', 'rut_wasm.wasm');
if (!existsSync(src)) {
  console.error(
    `wasm module not found: ${src} — run ` +
      `\`npm run build:wasm\` in demo/ (builds it with: ` +
      `\`cargo build -p rut-wasm --target wasm32-unknown-unknown --release\`)`
  );
  process.exit(1);
}
copyFileSync(src, join('public', 'rut.wasm'));
console.log(`copied ${src} -> public/rut.wasm`);
