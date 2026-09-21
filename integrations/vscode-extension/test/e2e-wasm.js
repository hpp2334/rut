// Through-wasm e2e gate — the extension-side twin of
// crates/rut-lsp/tests/corpus.rs (the lsp-align survey §3.6 design).
// Drives the SHIPPED bin/rut-lsp.wasm over the full repo corpus
// (rut/ + examples/ + demo/src/examples/ + benches/workloads/) through
// the extension's OWN ABI binding (src/wasm.ts, bundled to out/wasm.js
// by esbuild.mjs) — the exact load path src/extension.ts uses, minus the
// vscode host. Zero false diagnostics is the alignment lock: the wasm
// module and the native server share crates/rut-lsp queries, so this
// gate cannot pass while the shipped artifact and the language drift.
//
// Plus the hover/completion smoke the survey demanded: a `?T` hover
// (the RFC 0043 alias rendering phase 1 fixed), member hover/completion
// through a `?Circle` let-binding (the ty_head fix), and a bare
// nmapset/nmap_host completion (the std-surface 8/8 fix) — all against
// the shipped binary.
//
// Runs headless in plain node — no `code` needed; the Extension Host
// suite (test:host) stays the loud-skip lane on machines without VS Code.
'use strict';

const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const EXT = path.join(__dirname, '..');
const REPO = path.join(EXT, '..', '..');
const WASM_PATH = path.join(EXT, 'bin', 'rut-lsp.wasm');
const BINDING_PATH = path.join(EXT, 'out', 'wasm.js');
const FIXTURE = path.join(__dirname, 'fixtures', 'symbols.rut');

// corpus roots, mirroring crates/rut-lsp/tests/corpus.rs and
// test/grammar-corpus.js
const CORPUS_ROOTS = [
  'rut',
  'examples',
  path.join('demo', 'src', 'examples'),
  path.join('benches', 'workloads'),
];
const MIN_CORPUS_FILES = 50; // 53 today — guards against a silently empty walk

function* rutFiles(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* rutFiles(p);
    else if (entry.name.endsWith('.rut')) yield p;
  }
}

// [line, character] (0-based, LSP order) of the n-th occurrence of needle
function posOf(src, needle, occurrence = 1) {
  let at = -1;
  for (let i = 0; i < occurrence; i++) at = src.indexOf(needle, at + 1);
  assert.ok(at >= 0, `needle ${JSON.stringify(needle)} not found`);
  const before = src.slice(0, at);
  return [before.split('\n').length - 1, at - (before.lastIndexOf('\n') + 1)];
}

// [line, character] of needle on the first line matching lineRe —
// immune to the same word appearing in comment prose above the code
function posOnLine(src, lineRe, needle) {
  const lines = src.split('\n');
  for (let i = 0; i < lines.length; i++) {
    if (!lineRe.test(lines[i])) continue;
    const ch = lines[i].indexOf(needle);
    assert.ok(ch >= 0, `${JSON.stringify(needle)} not on line matching ${lineRe}: ${lines[i]}`);
    return [i, ch];
  }
  assert.ok(false, `no line matches ${lineRe}`);
}

async function main() {
  if (!fs.existsSync(WASM_PATH)) {
    console.error(
      `e2e-wasm: bin/rut-lsp.wasm not found — this gate drives the SHIPPED ` +
        `artifact; build it first:\n  cd integrations/vscode-extension && npm run build:wasm`);
    process.exit(1);
  }
  if (!fs.existsSync(BINDING_PATH)) {
    console.error(`e2e-wasm: ${path.relative(REPO, BINDING_PATH)} not found — run \`npm run compile\` first`);
    process.exit(1);
  }
  // the extension's own binding (bundled src/wasm.ts), not a copy
  const { RutWasm } = require(BINDING_PATH);
  const rut = await RutWasm.load(WASM_PATH);
  const bad = [];
  let smokes = 0;

  // ---- the semantic-token legend (14 types, the extension's map) ----
  const legend = rut.legend();
  if (!(legend.length === 14 && legend.includes('enumMember') && legend.includes('keyword'))) {
    bad.push(`legend drifted: ${JSON.stringify(legend)}`);
  }
  smokes++;

  // ---- the fixture: a current-grammar sample, analyzed clean ----
  const fixtureSrc = fs.readFileSync(FIXTURE, 'utf8');
  const fixtureUri = `file://${FIXTURE}`;
  const fx = rut.analyze(fixtureUri, fixtureSrc);
  if (fx.diags.length !== 0) {
    bad.push(`fixture has ${fx.diags.length} diagnostic(s): ${JSON.stringify(fx.diags[0])}`);
  }
  if (!(fx.tokens.data && fx.tokens.data.length >= 100)) {
    bad.push(`fixture token stream thin: ${fx.tokens.data && fx.tokens.data.length}`);
  }
  const fxNames = fx.symbols.map((s) => s.name);
  for (const want of ['Color', 'Point', 'Circle', 'Drawable', 'main', 'Slot', 'tag', 'kind', 'widen']) {
    if (!fxNames.includes(want)) bad.push(`fixture symbol ${want} missing (got ${fxNames.join(', ')})`);
  }
  smokes++;

  // ---- ?T hover: the RFC 0043 alias renders its nullable target ----
  // (M4's ty_src arm — the pre-align builds printed `type Maybe = ;`)
  {
    const [line, ch] = posOnLine(fixtureSrc, /^\s*type Maybe = /, 'Maybe');
    const h = rut.hover(fixtureUri, line, ch);
    if (!h || !h.contents.value.includes('type Maybe = ?i32;')) {
      bad.push(`alias hover lost the ?T target: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`);
    }
    smokes++;
  }
  // the RFC 0004 primitives hover their current `primitive` form
  // (the stale e41915d binary said `builtin str`)
  {
    const [line, ch] = posOnLine(fixtureSrc, /^\s*fn tag\(s: str/, 'str');
    const h = rut.hover(fixtureUri, line, ch);
    if (!h || !h.contents.value.includes('primitive str {')) {
      bad.push(`str hover is not the primitive form: ${h && JSON.stringify(h.contents.value.slice(0, 60))}`);
    }
    smokes++;
  }

  // ---- member hover/completion through a `?Circle` let-binding ----
  // (M4's ty_head arm — mirrors crates/rut-lsp/src/hover/tests.rs's
  // nullable_let_binding_resolves_members through the shipped wasm;
  // methods live in impl blocks — RFC 0012 — so the doc analyzes clean)
  {
    const uri = 'file:///ws/e2e-nullable.rut';
    const src = [
      'class Circle {',
      '    r: f64;',
      '}',
      'impl Circle {',
      '    fn area(self) -> f64 { return 3.14; }',
      '}',
      'fn go() -> f64 {',
      '    let c: ?Circle = nil;',
      '    return c.area();',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`nullable probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const [hline, hch] = posOf(src, 'area', 2); // the call site, not the decl
    const h = rut.hover(uri, hline, hch);
    if (!h || !h.contents.value.includes('fn area(self) -> f64') || !h.contents.value.includes('in `impl Circle`')) {
      bad.push(`?Circle member hover missed: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`);
    }
    const [cline, cch] = posOf(src, 'c.');
    const items = rut.complete(uri, cline, cch + 2);
    const labels = items.map((i) => i.label);
    for (const want of ['area', 'r']) {
      if (!labels.includes(want)) bad.push(`?Circle completion lacks '${want}' (got ${labels.join(', ')})`);
    }
    smokes++;
  }

  // ---- bare nmapset/nmap_host completion (std surface, M3's acceptance:
  // what a user project with no workspace index gets) ----
  {
    const uri = 'file:///ws/e2e-std.rut';
    const src = 'fn main() -> nil {\n}\n';
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`std probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const labels = rut.complete(uri, 0, 0).map((i) => i.label);
    for (const want of ['HashMap', 'HashSet', 'PrimMapI64']) {
      if (!labels.includes(want)) bad.push(`bare completion lacks nmapset's '${want}' (${labels.length} items)`);
    }
    if (!labels.includes('map_entry')) bad.push(`bare completion lacks nmap_host's 'map_entry' (${labels.length} items)`);
    smokes++;
  }

  // ---- the RFC 0044 dedicated diagnostic (M2's acceptance) ----
  {
    const b = rut.analyze('file:///ws/e2e-postfix.rut', 'fn f(p: i32?) -> nil {\n}\n');
    if (!(b.diags.length === 1 && b.diags[0].message.includes('RFC 0044'))) {
      bad.push(`postfix T? must give the ONE dedicated RFC 0044 diag, got: ${JSON.stringify(b.diags)}`);
    }
    const g = rut.analyze('file:///ws/e2e-prefix.rut', 'fn g(p: ?i32) -> nil {\n}\n');
    if (g.diags.length !== 0) {
      bad.push(`?i32 must analyze clean (M1), got: ${JSON.stringify(g.diags)}`);
    }
    smokes++;
  }

  // ---- the corpus: zero false diagnostics through the SHIPPED wasm ----
  // (the pre-align artifact flagged 15 of 53 files with 1,796 false
  // diagnostics — every ?T site read `expected a type name`)
  const files = [...rutFiles(path.join(REPO, 'rut'))];
  for (const root of CORPUS_ROOTS.slice(1)) files.push(...rutFiles(path.join(REPO, root)));
  files.sort();
  assert.ok(files.length >= MIN_CORPUS_FILES,
    `corpus walk found only ${files.length} .rut files (expected >= ${MIN_CORPUS_FILES})`);

  let symbols = 0;
  for (const file of files) {
    const src = fs.readFileSync(file, 'utf8');
    // the absolute path AS the uri — the .d.rut suffix survives, so the
    // module's mode_of picks Decl for the decl surfaces (core.d.rut &
    // co.); a mode mistake here would light them up as diagnostics.
    const uri = `file://${file}`;
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`${path.relative(REPO, file)}: ${a.diags.length} false diagnostic(s), first: ${JSON.stringify(a.diags[0].message)}`);
      continue;
    }
    if (!(a.tokens.data && a.tokens.data.length > 0)) {
      bad.push(`${path.relative(REPO, file)}: empty token stream`);
    }
    if (!path.basename(file).endsWith('.d.rut')) {
      // Impl-mode files yield document symbols (corpus_classifies' law)
      if (!a.symbols.length) {
        bad.push(`${path.relative(REPO, file)}: no document symbols`);
      }
      symbols += a.symbols.length;
    }
  }

  if (bad.length) {
    console.error(`e2e-wasm: ${bad.length} violation(s):\n  ${bad.slice(0, 40).join('\n  ')}${bad.length > 40 ? `\n  … (+${bad.length - 40} more)` : ''}`);
    process.exit(1);
  }
  console.log(`e2e-wasm: PASS — ${files.length} corpus files, 0 false diagnostics, ${symbols} symbols, ` +
    `${smokes} smoke assertions (legend/fixture/?T hover/primitives/member-nullable/std-completion/RFC 0044) ` +
    `through bin/rut-lsp.wasm`);
}

main().catch((e) => {
  console.error('e2e-wasm: FAILED —', e);
  process.exit(1);
});
