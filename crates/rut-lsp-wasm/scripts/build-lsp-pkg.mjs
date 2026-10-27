// build-lsp-pkg.mjs — finish the wasm-pack package the hosts consume.
//
// `wasm-pack build crates/rut-lsp-wasm --target web --release` writes the
// generated glue + module into `pkg/` (gitignored build output). That
// glue speaks JSON strings; the typed face — the class that parses the
// envelopes, `decodeTokens`, and the `.d.ts` that names every shape —
// lives in this crate's `js/` source. This step installs it INTO the
// package, so a consumer imports ONE package path
// (`crates/rut-lsp-wasm/pkg/index.js`) and never reaches into the
// crate's source tree:
//
//   pkg/index.js      the JS face (imports the glue beside it)
//   pkg/index.d.ts    the declarations
//   pkg/package.json  main/module/types point at the face
//
// Run it after every wasm-pack build of this crate — the npm lanes
// (`build:wasm`, `build:lsp`) and CI do.
import { copyFileSync, existsSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const CRATE = dirname(dirname(fileURLToPath(import.meta.url)));
const PKG = join(CRATE, 'pkg');
const SRC = join(CRATE, 'js');

function die(msg) {
  console.error(`build-lsp-pkg: ${msg}`);
  process.exit(1);
}

for (const f of ['rut_lsp_wasm.js', 'rut_lsp_wasm_bg.wasm']) {
  if (!existsSync(join(PKG, f))) {
    die(
      `pkg/${f} missing — run \`wasm-pack build crates/rut-lsp-wasm ` +
        '--target web --release` first'
    );
  }
}

// the face's own import of the glue is `../pkg/rut_lsp_wasm.js` while it
// sits in `js/`; inside the package it is the sibling `./rut_lsp_wasm.js`.
// The rewrite is asserted: a refactor that re-spells the import must fail
// here rather than ship a package with a dangling path.
const GLUE_FROM_SRC = '../pkg/rut_lsp_wasm.js';
const GLUE_IN_PKG = './rut_lsp_wasm.js';
const src = readFileSync(join(SRC, 'index.js'), 'utf8');
if (!src.includes(GLUE_FROM_SRC)) {
  die(`js/index.js no longer imports "${GLUE_FROM_SRC}" — update this rewrite`);
}
writeFileSync(join(PKG, 'index.js'), src.replaceAll(GLUE_FROM_SRC, GLUE_IN_PKG));
copyFileSync(join(SRC, 'index.d.ts'), join(PKG, 'index.d.ts'));

// point the package at the face (the glue stays reachable as a file, for
// the smoke scripts and the artifact copies, which read paths, not
// specifiers)
const manifestPath = join(PKG, 'package.json');
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
manifest.main = 'index.js';
manifest.module = 'index.js';
manifest.types = 'index.d.ts';
manifest.files = [...new Set([...(manifest.files ?? []), 'index.js', 'index.d.ts'])];
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

console.log(`build-lsp-pkg: pkg/ carries the JS face + declarations (${manifest.name}@${manifest.version})`);
