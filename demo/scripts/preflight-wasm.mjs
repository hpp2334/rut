// Preflight for dev/build (RFC 0041 §3): the demo only shows REAL runs,
// so the artifact must exist BEFORE the bundler starts. A missing
// public/rut.wasm is a loud failure naming the exact build command —
// the gen-dir/lsp-align loud-fail law applied to the demo.
import { existsSync } from 'node:fs';
import { join } from 'node:path';

const artifact = join('public', 'rut.wasm');
if (!existsSync(artifact)) {
  console.error(
    `missing artifact: ${artifact} — the playground has no preview ` +
      `fallback. Run \`npm run build:wasm\` in demo/ first ` +
      `(builds crates/rut-wasm and copies it here).`
  );
  process.exit(1);
}
console.log(`preflight ok: ${artifact} present`);
