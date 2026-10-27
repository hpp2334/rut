// release.mjs — the one lane from source to a shippable vsix:
// preflight → build:wasm → gates → vsce package → byte-verify.
//
// The ordering law: the gates run against the FRESH bin/rut-lsp.wasm and
// the pack embeds that same file, so the artifact you ship is the
// artifact you gated — a stale wasm cannot pass (the e2e corpus gate
// reads a new syntax through an old wasm as false diagnostics).
//
// No version writes, no git commands: the script packs whatever
// package.json says — a bump is a deliberate human act
// (`npm version X.Y.Z --no-git-tag-version`). Older rut-vscode-*.vsix
// files are reported, never deleted.
//
//   npm run release             # the full lane
//   npm run release -- --dry-run  # build + gates, stop before packing
//
// npm scripts run with cwd = the extension folder (demo/ uses the same).
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const DRY = process.argv.includes('--dry-run');
for (const arg of process.argv.slice(2)) {
  if (arg !== '--dry-run') {
    die(
      `unknown flag: ${arg} — the lane packs the CURRENT package.json version; ` +
        'to bump first: `npm version X.Y.Z --no-git-tag-version`',
    );
  }
}

function die(msg) {
  console.error(`release: ${msg}`);
  process.exit(1);
}
function run(cmd, args, label) {
  const r = spawnSync(cmd, args, { stdio: 'inherit' });
  if (r.status !== 0) die(`${label} failed (exit ${r.status ?? r.error?.code})`);
}
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';

// ---- 1. preflight — every missing piece names its remedy ----------------
if (spawnSync('cargo', ['--version']).status !== 0) {
  die(
    'cargo not found on PATH — the wasm rebuild needs the rust toolchain ' +
      '(rustup.rs); e.g. `source ~/.cargo/env`',
  );
}
for (const dep of ['@vscode/vsce', 'vscode-textmate', 'vscode-oniguruma']) {
  if (!existsSync(join('node_modules', dep))) {
    die(`devDependency ${dep} missing — run \`npm install\` first`);
  }
}

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const { version } = pkg;
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  die(`package.json version is not X.Y.Z: ${version}`);
}
const vsix = `rut-vscode-${version}.vsix`;

// ---- 2. build the shipped artifact --------------------------------------
console.log(`release: building bin/rut-lsp.wasm for ${version}`);
run(npm, ['run', 'build:wasm'], 'build:wasm');

// ---- 3. gates — against the fresh artifact ------------------------------
for (const gate of ['typecheck', 'test:grammar', 'test:e2e', 'test:host']) {
  console.log(`release: gate ${gate}`);
  run(npm, ['run', gate], gate);
}

if (DRY) {
  console.log(
    `release: dry-run complete — build + gates green; ` +
      `would pack ${vsix} (nothing packed, version untouched)`,
  );
  process.exit(0);
}

// ---- 4. pack the current version ----------------------------------------
if (existsSync(vsix)) {
  console.log(`release: NOTE — ${vsix} already exists and will be overwritten`);
}
run(npm, ['run', 'package'], 'package');
if (!existsSync(vsix)) {
  die(`vsce reported success but ${vsix} is missing`);
}

// ---- 5. verify — the wasm inside the vsix IS the gated file -------------
function sha256(buf) {
  return createHash('sha256').update(buf).digest('hex');
}
const inner = spawnSync('unzip', ['-p', vsix, 'extension/bin/rut-lsp.wasm'], {
  encoding: 'buffer',
  maxBuffer: 64 * 1024 * 1024, // the wasm is ~3 MB — the 1 MB default ENOBUFS's
});
if (inner.status !== 0 || !inner.stdout?.length) {
  console.log('release: NOTE — no usable `unzip` on this box: the wasm byte-verify is SKIPPED (loudly)');
} else {
  const shipped = sha256(inner.stdout);
  const gated = sha256(readFileSync(join('bin', 'rut-lsp.wasm')));
  if (shipped !== gated) {
    die(
      `the wasm inside ${vsix} is NOT the gated bin/rut-lsp.wasm ` +
        `(${shipped.slice(0, 12)}… vs ${gated.slice(0, 12)}…)`,
    );
  }
  console.log(`release: byte-verified — the vsix carries the gated wasm (${gated.slice(0, 12)}…)`);
  const grammar = spawnSync('unzip', ['-p', vsix, 'extension/syntaxes/rut.tmLanguage.json'], {
    encoding: 'utf8',
  });
  if (grammar.status === 0 && grammar.stdout) {
    try {
      JSON.parse(grammar.stdout);
    } catch (e) {
      die(`the grammar inside ${vsix} is not valid JSON: ${e.message}`);
    }
  }
}

// ---- 6. report -----------------------------------------------------------
const mb = (statSync(vsix).size / 1024 / 1024).toFixed(2);
console.log(`release: PACKED ${vsix} (${mb} MB) — install: code --install-extension ${vsix}`);
const others = readdirSync('.')
  .filter((f) => /^rut-vscode-.*\.vsix$/.test(f) && f !== vsix)
  .sort();
if (others.length) {
  console.log(`release: older artifacts left in place (report only): ${others.join(', ')}`);
}
