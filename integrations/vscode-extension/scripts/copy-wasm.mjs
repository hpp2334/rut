// copy the wasm-pack-built rut-lsp module into bin/ — one artifact,
// every platform (the whole point of the wasm mode). The glue
// (`pkg/rut_lsp_wasm.js`) is NOT copied: esbuild bundles it straight
// from the pkg dir into the extension's own bundle.
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

const src = join('..', '..', 'crates', 'rut-lsp-wasm', 'pkg', 'rut_lsp_wasm_bg.wasm');
if (!existsSync(src)) {
  console.error(
    `wasm module not found: ${src} — run ` +
      `\`wasm-pack build ../../crates/rut-lsp-wasm --target web --release\` first ` +
      `(npm run build:wasm)`
  );
  process.exit(1);
}
mkdirSync('bin', { recursive: true });
copyFileSync(src, join('bin', 'rut-lsp.wasm'));
console.log(`copied ${src} -> bin/rut-lsp.wasm`);
