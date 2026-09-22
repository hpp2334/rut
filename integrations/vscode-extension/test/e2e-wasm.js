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
// Phase 2 (lsp-features) adds the DEFINITION round trips: request
// position → response span == the declaring ident EXACTLY, through all
// three resolution layers — within-file (binding pass + decl layer),
// cross-file (the use graph into a rut.add_def'ed module), and stdlib
// (the embedded surface's true rut/... source path) — plus a
// typeDefinition smoke. A feature without a gate through the shipped
// artifact does not land.
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

  // ---- phase 1 (lsp-features): field DECL hover — hovering `item`
  // inside `struct Slot { item: ?Circle; }` answers with the field's
  // decl and its owning struct (the decl layer's token-recovered span) ----
  {
    const [line, ch] = posOnLine(fixtureSrc, /^\s*item: \?Circle;/, 'item');
    const h = rut.hover(fixtureUri, line, ch);
    if (!h || !h.contents.value.includes('item: ?Circle') || !h.contents.value.includes('in `Slot`')) {
      bad.push(`field decl hover missed: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`);
    }
    smokes++;
  }

  // ---- phase 1: inferred-type IDENT hover — `let c = Circle.new(1.0)`
  // shows its inferred type at the use site (the display-side ruling) ----
  {
    const uri = 'file:///ws/e2e-inferred-ident.rut';
    const src = [
      'class Circle {',
      '    r: f64;',
      '}',
      'fn go() -> f64 {',
      '    let c = Circle.new(1.0);',
      '    return c.r;',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`inferred-ident probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const [line, ch] = posOnLine(src, /return c\.r;/, 'c');
    const h = rut.hover(uri, line, ch);
    if (!h || !h.contents.value.includes('let c: Circle') || !h.contents.value.includes('type inferred')) {
      bad.push(`inferred ident hover missed: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`);
    }
    smokes++;
  }

  // ---- phase 1: receiver inference through a FIELD READ — the
  // binding pass types `inner` from `wrap.c`, so the ident hover, the
  // member hover AND member completion all resolve through it ----
  {
    const uri = 'file:///ws/e2e-field-read-recv.rut';
    const src = [
      'class Circle {',
      '    r: f64;',
      '}',
      'struct Wrap {',
      '    c: Circle;',
      '}',
      'impl Circle {',
      '    fn grown(self, k: f64) -> Circle { return self; }',
      '}',
      'fn area(wrap: Wrap) -> f64 {',
      '    let inner = wrap.c;',
      '    let big = inner.grown(2.0);',
      '    return big.r;',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`field-read probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    // the ident hover: the field-read initializer types the binding
    const [iline, ich] = posOnLine(src, /let big = inner\.grown/, 'inner');
    const ih = rut.hover(uri, iline, ich);
    if (!ih || !ih.contents.value.includes('let inner: Circle')) {
      bad.push(`field-read ident hover missed: ${ih && JSON.stringify(ih.contents.value.slice(0, 80))}`);
    }
    // the member hover THROUGH the field-read receiver
    const mh = rut.hover(uri, iline, ich + 7); // the `grown` after `inner.`
    if (!mh || !mh.contents.value.includes('fn grown(self, k: f64) -> Circle')) {
      bad.push(`field-read receiver member hover missed: ${mh && JSON.stringify(mh.contents.value.slice(0, 80))}`);
    }
    // the member hover through the CHAINED-CALL receiver (`big.r`)
    const [bline, bch] = posOnLine(src, /return big\.r;/, 'big.r');
    const bh = rut.hover(uri, bline, bch + 4); // the `r` after `big.`
    if (!bh || !bh.contents.value.includes('r: f64') || !bh.contents.value.includes('in `Circle`')) {
      bad.push(`chained receiver field hover missed: ${bh && JSON.stringify(bh.contents.value.slice(0, 80))}`);
    }
    // member completion through the field-read receiver
    const items = rut.complete(uri, iline, ich + 6); // right after `inner.`
    const labels = items.map((i) => i.label);
    for (const want of ['r', 'grown']) {
      if (!labels.includes(want)) bad.push(`field-read completion lacks '${want}' (got ${labels.join(', ')})`);
    }
    smokes++;
  }

  // ---- phase 1: for-of loop-variable receiver — `[Point]` types the
  // iteration binding, its member hover resolves ----
  {
    const uri = 'file:///ws/e2e-for-of-recv.rut';
    const src = [
      'struct Point {',
      '    x: f64;',
      '    y: f64;',
      '}',
      'fn sum(points: [Point]) -> f64 {',
      '    let total = 0.0;',
      '    for (let p of points) {',
      '        total += p.x;',
      '    }',
      '    return total;',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`for-of probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const [pline, pch] = posOnLine(src, /total \+= p\.x;/, 'p');
    const ph = rut.hover(uri, pline, pch);
    if (!ph || !ph.contents.value.includes('p: Point') || !ph.contents.value.includes('loop variable')) {
      bad.push(`for-of ident hover missed: ${ph && JSON.stringify(ph.contents.value.slice(0, 80))}`);
    }
    const mh = rut.hover(uri, pline, pch + 2); // the `x` after `p.`
    if (!mh || !mh.contents.value.includes('x: f64') || !mh.contents.value.includes('in `Point`')) {
      bad.push(`for-of receiver member hover missed: ${mh && JSON.stringify(mh.contents.value.slice(0, 80))}`);
    }
    smokes++;
  }

  // ---- phase 1 (M7): primitive type-token hover — `i32`/`bool` have
  // no surface decl; the static blurb (width, range) is the answer ----
  {
    const uri = 'file:///ws/e2e-prim-hover.rut';
    const src = 'fn pick(x: i32, ok: bool) -> i32 {\n    return x;\n}\n';
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`primitive probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const [line, ch] = posOnLine(src, /fn pick\(x: i32, ok: bool\)/, 'i32');
    const h = rut.hover(uri, line, ch);
    if (!h || !h.contents.value.includes('signed 32-bit') || !h.contents.value.includes('-2147483648 ..= 2147483647')) {
      bad.push(`i32 primitive hover missed: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`);
    }
    const [bline, bch] = posOnLine(src, /fn pick\(x: i32, ok: bool\)/, 'bool');
    const bh = rut.hover(uri, bline, bch);
    if (!bh || !bh.contents.value.includes('true` / `false')) {
      bad.push(`bool primitive hover missed: ${bh && JSON.stringify(bh.contents.value.slice(0, 80))}`);
    }
    smokes++;
  }

  // ---- phase 2 (lsp-features): definition round trips — request pos →
  // response span == the declaring ident EXACTLY. Three resolution
  // layers gated through the SHIPPED artifact: within-file (the binding
  // pass + decl layer), cross-file (the use graph), stdlib (the
  // embedded surface's true rut/... source path) ----

  // expected [line, char] of the ident at `at` inside `needle`'s line
  const identAt = (src, lineRe, needle) => {
    const [line, ch] = posOnLine(src, lineRe, needle);
    return [line, ch, needle.length];
  };
  // the response must carry a location with `want.uri` whose span is
  // the declaring ident EXACTLY — same start, same length
  const eqRange = (locs, want, what) => {
    const loc = locs.find((l) => l.uri === want.uri);
    if (!loc) {
      bad.push(`${what}: no location for ${want.uri} (got ${JSON.stringify(locs)})`);
      return;
    }
    const g = loc.range;
    const [wl, wc] = want.range;
    const wlen = want.len;
    if (g.start.line !== wl || g.start.character !== wc ||
        g.end.line !== wl || g.end.character !== wc + wlen) {
      bad.push(`${what}: span ${JSON.stringify(g)} != declaring ident [${wl},${wc}..+${wlen}]`);
    }
  };

  // WITHIN-FILE: the binding pass (local ident → its declaring let),
  // the decl layer (field read → the field's decl ident), and an impl
  // method call → the impl fn's name ident
  {
    const uri = 'file:///ws/e2e-def-within.rut';
    const src = [
      'class Circle {',
      '    r: f64;',
      '}',
      'struct Wrap {',
      '    c: Circle;',
      '}',
      'impl Circle {',
      '    fn area(self) -> f64 { return 3.14; }',
      '}',
      'fn go(wrap: Wrap) -> f64 {',
      '    let inner = wrap.c;',
      '    return inner.area() + inner.r;',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`def-within probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    // `inner` at its use → the declaring ident in `let inner`
    let [dl, dc] = identAt(src, /return inner\.area\(\)/, 'inner');
    let want = { uri, range: identAt(src, /let inner = wrap\.c;/, 'inner'), len: 5 };
    let locs = rut.definition(uri, dl, dc);
    eqRange(locs, want, 'within-file let binding');
    // field read `wrap.c` → the field's decl ident `c` in Wrap
    want = { uri, range: identAt(src, /^\s{4}c: Circle;/, 'c'), len: 1 };
    ;[dl, dc] = identAt(src, /let inner = wrap\.c;/, 'c');
    locs = rut.definition(uri, dl, dc);
    eqRange(locs, want, 'within-file field read');
    // method call `inner.area()` → the impl fn's name ident
    want = { uri, range: identAt(src, /fn area\(self\)/, 'area'), len: 4 };
    ;[dl, dc] = identAt(src, /return inner\.area\(\)/, 'area');
    locs = rut.definition(uri, dl, dc);
    eqRange(locs, want, 'within-file impl method');
    smokes++;
  }

  // CROSS-FILE: a `use gadgets::{ Widget }` doc jumps into the
  // exporting module (indexed via rut.add_def, pkg matched by its path
  // segment) — both the use-statement name and the usage sites
  {
    const libUri = 'file:///ws/gadgets/lib.rut';
    const libSrc = [
      'class Widget {',
      '    id: i32;',
      '}',
      '',
    ].join('\n');
    rut.addDef(libUri, libSrc);
    const uri = 'file:///ws/e2e-def-cross.rut';
    const src = [
      'use gadgets::{ Widget };',
      'fn main() -> nil {',
      '    let w = Widget.new();',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`def-cross probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const want = {
      uri: libUri,
      range: identAt(libSrc, /^class Widget \{/, 'Widget'),
      len: 6,
    };
    // the use-statement name — the survey's use-graph edge
    let locs = rut.definition(uri, ...identAt(src, /^use gadgets::\{ Widget \};/, 'Widget'));
    eqRange(locs, want, 'cross-file use name');
    // and a usage site through the same edge
    locs = rut.definition(uri, ...identAt(src, /let w = Widget\.new\(\);/, 'Widget'));
    eqRange(locs, want, 'cross-file usage');
    smokes++;
  }

  // STDLIB: `use pouch::{ Vec }` jumps into the EMBEDDED surface — the
  // response carries the true rut/... path and the span of the
  // declaring ident in the REAL pouch.rut source
  {
    const uri = 'file:///ws/e2e-def-std.rut';
    const src = [
      'use pouch::{ Vec };',
      'fn main() -> nil {',
      '    let v = Vec.new();',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`def-std probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const real = fs.readFileSync(path.join(REPO, 'rut', 'pouch', 'pouch.rut'), 'utf8');
    const wantUri = path.join('rut', 'pouch', 'pouch.rut');
    const want = { uri: wantUri, range: identAt(real, /^pub class Vec/, 'Vec'), len: 3 };
    let locs = rut.definition(uri, ...identAt(src, /^use pouch::\{ Vec \};/, 'Vec'));
    if (!locs.length || locs[0].uri !== wantUri) {
      bad.push(`std jump lost the true source path (got ${JSON.stringify(locs.map((l) => l.uri))})`);
    } else {
      eqRange(locs, want, 'std use name');
    }
    locs = rut.definition(uri, ...identAt(src, /let v = Vec\.new\(\);/, 'Vec'));
    if (!locs.length || locs[0].uri !== wantUri) {
      bad.push(`std usage jump lost the true source path (got ${JSON.stringify(locs.map((l) => l.uri))})`);
    } else {
      eqRange(locs, want, 'std usage');
    }
    smokes++;
  }

  // TYPE DEFINITION: an expression → its type's declaration (the cheap
  // shape the survey priced — binding types via the phase-1 inference)
  {
    const uri = 'file:///ws/e2e-typedef.rut';
    const src = [
      'class Circle {',
      '    r: f64;',
      '}',
      'fn go() -> f64 {',
      '    let c = Circle.new(1.0);',
      '    return c.r;',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`typedef probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const want = { uri, range: identAt(src, /^class Circle \{/, 'Circle'), len: 6 };
    const locs = rut.typeDefinition(uri, ...identAt(src, /return c\.r;/, 'c'));
    eqRange(locs, want, 'typeDefinition of a binding');
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
    `${smokes} smoke assertions (legend/fixture/?T hover/primitives/member-nullable/std-completion/RFC 0044/` +
    `field-decl-hover/inferred-ident/field-read-receiver/for-of-receiver/primitive-hover/` +
    `def-within/def-cross/def-std/typeDefinition) ` +
    `through bin/rut-lsp.wasm`);
}

main().catch((e) => {
  console.error('e2e-wasm: FAILED —', e);
  process.exit(1);
});
