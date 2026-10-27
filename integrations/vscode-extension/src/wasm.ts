// The ABI binding lives in the wasm crate: `crates/rut-lsp-wasm/js`
// owns the runtime face (`index.js` — the JSON envelope parsing) and its
// declarations (`index.d.ts`). The extension does not declare LSP
// shapes; it only maps them onto `vscode.*` values (see extension.ts).
//
// This file stays a re-export because it is esbuild's SECOND entry
// (`out/wasm.js`): the through-wasm e2e gate (`test/e2e-wasm.js`) drives
// that bundle, so the gate exercises the exact module the extension
// ships, not a copy.

export * from '../../../crates/rut-lsp-wasm/pkg/index.js';
