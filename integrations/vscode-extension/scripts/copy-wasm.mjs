// copy the cargo-built rut-lsp wasm module into bin/ — one artifact,
// every platform (the whole point of the wasm mode)
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

const src = join('..', '..', 'target', 'wasm32-unknown-unknown', 'release', 'rut_lsp_wasm.wasm');
if (!existsSync(src)) {
  console.error(
    `wasm module not found: ${src} — run ` +
      `\`cargo build -p rut-lsp-wasm --target wasm32-unknown-unknown --release\` first`
  );
  process.exit(1);
}
mkdirSync('bin', { recursive: true });
copyFileSync(src, join('bin', 'rut-lsp.wasm'));
console.log(`copied ${src} -> bin/rut-lsp.wasm`);
