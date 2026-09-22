// Preflight for dev/build (RFC 0041 §3): the demo only shows REAL runs
// and REAL analyzer highlight, so BOTH artifacts must exist BEFORE the
// bundler starts. A missing public/rut.wasm or public/rut-lsp.wasm is a
// loud failure naming the exact build command — the gen-dir/lsp-align
// loud-fail law applied to the demo.
import { existsSync } from 'node:fs';
import { join } from 'node:path';

const ARTIFACTS = [
  join('public', 'rut.wasm'),
  join('public', 'rut-lsp.wasm'),
];

for (const artifact of ARTIFACTS) {
  if (!existsSync(artifact)) {
    console.error(
      `missing artifact: ${artifact} — the playground has no fake ` +
        `fallback. Run \`npm run build:wasm\` in demo/ first ` +
        `(builds crates/rut-wasm AND crates/rut-lsp-wasm and copies ` +
        `both here).`
    );
    process.exit(1);
  }
  console.log(`preflight ok: ${artifact} present`);
}
