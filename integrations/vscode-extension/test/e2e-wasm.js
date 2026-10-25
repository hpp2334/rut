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
// Phase 3 (lsp-features) adds the INLAY smokes: type hints on
// unannotated bindings and param-name hints at exact-arity call sites,
// resolved through the REBUILT artifact at known corpus positions —
// each label checked against the source's own ground truth (an
// annotation or a decl-site type line in the same file, or the
// stdlib's true mod.rut decl) — plus the hover-consistency law (the
// hint's type text is what hover shows for the same binding) and the
// annotated-lets-never-restated + range-filter laws on a synthetic doc.
//
// Phase 4 (lsp-features) adds the REFERENCES + SIGNATURE-HELP smokes:
// references round trips (a decl -> the EXACT expected use-position
// set, shadow-aware across scopes, and cross-file through the use
// graph's reverse edges in BOTH directions), and signature help
// (verbatim signature + active parameter correct at first/middle/
// nested positions; an arity mismatch -> no help).
//
// Phase 5 (manifest-deps) adds the DEP ABI: rut_parse_manifest (the
// repo's own path+url fixture manifest through the envelope — names,
// entry modes, the Rust-computed cache path), rut_add_def_named (a
// named dep feeding the `use <name>::` tier), and rut_add_bundle (the
// committed dist/std/pouch.rutbundle — the offline url-dep fixture the
// CLI tests use — landing pouch's surface in hover + go-to-definition).
// These exports ride the RAW ABI (the smoke.js protocol): the extension
// binding learns them in the next lane, the ABI gate ships first.
//
// Runs headless in plain node — no `code` needed; the Extension Host
// suite (test:host) stays the loud-skip lane on machines without VS Code.
'use strict';

const assert = require('node:assert');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const EXT = path.join(__dirname, '..');
const REPO = path.join(EXT, '..', '..');
const WASM_PATH = path.join(EXT, 'bin', 'rut-lsp.wasm');
const BINDING_PATH = path.join(EXT, 'out', 'wasm.js');
const FIXTURE = path.join(__dirname, 'fixtures', 'symbols', 'src', 'mod.rut');

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

  // ---- the capability probe (the stale-install seam): the extract
  // exports an artifact must carry for the source-zip dep lane to
  // mount. A same-version-but-older install without them dead-ended
  // every std ctrl+click in a scaffold (the wasm answered the ambient
  // std embed's repo-relative path, the extension dropped it) — the
  // binding now answers a synthetic error envelope and the walk hints
  // the reinstall. The SHIPPED artifact must have both.
  if (rut.hasSrczip !== true || rut.hasBundleExtract !== true) {
    bad.push(`capability probe: hasSrczip=${rut.hasSrczip} hasBundleExtract=${rut.hasBundleExtract} — stale artifact, rebuild (npm run build:wasm)`);
  }
  smokes++;

  // ---- the semantic-token legend (16 types, the extension's map —
  // comments joined the classes, phase 5 added the namespace class) ----
  const legend = rut.legend();
  if (!(legend.length === 16 && legend.includes('enumMember') && legend.includes('keyword') && legend.includes('comment') && legend.includes('namespace'))) {
    bad.push(`legend drifted: ${JSON.stringify(legend)}`);
  }
  smokes++;

  // ---- the corpus: zero false diagnostics through the SHIPPED wasm ----
  // (the pre-align artifact flagged 15 of 53 files with 1,796 false
  // diagnostics — every ?T site read `expected a type name`)
  // THIS LOCK RUNS FIRST — ahead of every fail-fast (assert) lane that
  // follows — so a drifted pin or a stale artifact can never die the
  // gate early and mask the alignment verdict: a corpus violation lands
  // in `bad` even when a later lane would have thrown first. (A
  // hardcoded std-v8 URL pin once sat ahead of this block and did
  // exactly that masking.)
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
    if (!h || !h.contents.value.includes('fn area(self) -> f64') || !h.contents.value.includes('in `Circle`')) {
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

  // ---- bare completion over the std surface (A2's honest bare scope):
  // a foreign name is a PLAIN bare item only when the doc's `use` names
  // it — every other foreign name rides the AUTO-IMPORT tier (the
  // `use pkg::Name;` insert rides the item). The std surface is the
  // editor's ambient dep lane, so the family pair still completes at a
  // bare position — as import offers, sorted after the locals. ----
  {
    const uri = 'file:///ws/e2e-std.rut';
    const src = 'fn main() -> nil {\n}\n';
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`std probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const bareItems = rut.complete(uri, 0, 0);
    const labels = bareItems.map((i) => i.label);
    // the type-name-law batch: the completion pin offers the family
    // pair — now through the auto-import tier (the alias-row form is
    // repealed, so the `pub type HashMap<K, ..>` rows that used to sit
    // beside the class in the index are gone)
    for (const want of ['HashMap', 'HashSet']) {
      if (!labels.includes(want)) bad.push(`bare completion lacks nmapset's '${want}' (${labels.length} items)`);
    }
    for (const nm of ['HashMap', 'HashSet']) {
      const hit = bareItems.find((i) => i.label === nm);
      if (!hit) continue;
      if (!/^nmapset::.+ — import$/.test(hit.detail || '')) {
        bad.push(`bare completion '${nm}' is not the import offer: '${hit.detail}'`);
      }
      const edits = hit.additionalTextEdits || [];
      if (edits.length !== 1 || !/use nmapset::\{? ?/.test(edits[0].newText || '')) {
        bad.push(`bare completion '${nm}' carries no use-insert edit: ${JSON.stringify(edits)}`);
      }
      if (!hit.sortText || !hit.sortText.startsWith('~')) {
        bad.push(`import offer '${nm}' must sort after the locals: ${JSON.stringify(hit.sortText)}`);
      }
    }
    // nmap-host: the width moved INTO the name (no overloading) — the
    // keyed-lane fn is `map_hput` now (the `_i` spelling is gone with
    // the integer-suffix plan)
    if (!labels.includes('map_hput')) bad.push(`bare completion lacks nmap_host's 'map_hput' (${labels.length} items)`);
    // nmap-hostvals P4: the probing core's lanes left the surface — the
    // round-1 opaque-keyed `map_entry` must not come back
    if (labels.includes('map_entry')) bad.push(`bare completion still offers nmap_host's 'map_entry' (deleted at P4)`);
    // the 9th std pkg (the rut-json batch phase 1): json's base surface —
    // the entries + the traits + the reader/writer — rides every bare
    // completion exactly like nmapset's types do (std_surface 8 -> 9)
    for (const want of ['encodeJson', 'decodeJson', 'decodeJsonBytes', 'JsonSerialize', 'JsonDeserialize', 'JsonReader', 'JsonWriter']) {
      if (!labels.includes(want)) bad.push(`bare completion lacks json's '${want}' (${labels.length} items)`);
    }
    // the ambient core rows stay PLAIN bare offers (the `prelude
    // builtin` spellings bind in every unit — no import edit on them)
    for (const want of ['StackTrace', 'panic']) {
      const hit = bareItems.find((i) => i.label === want);
      if (!hit) bad.push(`bare completion lacks the ambient core row '${want}'`);
      else if ((hit.additionalTextEdits || []).length !== 0) {
        bad.push(`ambient core row '${want}' must complete plain, no import edit`);
      }
    }
    smokes++;
  }

  // ---- plan3 phase 2 (A3): the TYPE TIER at an annotation boundary —
  // `let v: u⏐` offers the primitive table (u8..u64), never the
  // keyword table, fns, or locals; and (A6 pin) the brace form of a
  // use — `use pouch::{ ⏐` — offers the module's exports. ----
  {
    const uri = 'file:///ws/e2e-type-tier.rut';
    const src = 'fn go() -> nil {\n    let v: u\n}\n';
    const items = rut.complete(uri, 1, '    let v: u'.length, src);
    const labels = items.map((i) => i.label);
    for (const want of ['u8', 'u16', 'u32', 'u64']) {
      if (!labels.includes(want)) bad.push(`type tier lacks '${want}' (got ${labels.join(', ')})`);
    }
    for (const absent of ['fn', 'let', 'go']) {
      if (labels.includes(absent)) bad.push(`type tier leaked '${absent}' (got ${labels.join(', ')})`);
    }

    // A6 pin: the brace form rides the names tier (`{` is a trigger
    // char — the extension fires the request on the keystroke)
    const bsrc = 'use pouch::{\nfn main() -> nil {\n}\n';
    const buri = 'file:///ws/e2e-use-brace.rut';
    const bitems = rut.complete(buri, 0, 'use pouch::{'.length, bsrc);
    const blabels = bitems.map((i) => i.label);
    if (!blabels.includes('Vec')) bad.push(`use brace form lacks pouch's 'Vec' (got ${blabels.join(', ')})`);
    smokes++;
  }

  // ---- member completion through a CONSTRUCTOR binding, with the
  // completion lane's doc sync + dot guard (the completion-correctness
  // phase): `let log = Logger("h")` types the binding from the ink
  // surface's class (the ctor-call inference), `log.` offers the impl
  // methods — and NEVER the bare tier's keyword/host-fn dump. The
  // CURRENT text rides the request (rut.complete's src argument →
  // rut_complete_in), so the dot is fresh without the analyze debounce. ----
  {
    const uri = 'file:///ws/e2e-ctor-completion.rut';
    // the member NAMES after each dot keep the doc parsing clean; the
    // request cursor sits right after the dot (prefix empty — the full
    // member list), exactly the keystroke moment in the editor
    const src = [
      'fn go() -> nil {',
      '    let log = Logger("h");',
      '    log.info;',
      '    someUnknown.x;',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`ctor-completion probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const [cline, cch] = posOf(src, 'log.'); // cursor right after the dot
    const items = rut.complete(uri, cline, cch + 4, src);
    const labels = items.map((i) => i.label);
    for (const want of ['new', 'debug', 'info', 'warn', 'error']) {
      if (!labels.includes(want)) bad.push(`ctor binding completion lacks '${want}' (got ${labels.join(', ')})`);
    }
    if (labels.includes('abs')) bad.push(`ctor binding completion leaked the host fn 'abs' (got ${labels.join(', ')})`);
    // the dot guard: an unresolved receiver answers EMPTY, never the
    // keyword table
    const [uline, uch] = posOf(src, 'someUnknown.');
    const guarded = rut.complete(uri, uline, uch + 'someUnknown.'.length, src);
    if (guarded.length !== 0) {
      bad.push(`dot guard failed: 'someUnknown.' completed ${guarded.length} item(s): ${guarded.slice(0, 8).map((i) => i.label).join(', ')}`);
    }
    smokes++;
  }

  // ---- plan3 phase 1 (A7+A1): END-OF-BUFFER completion — the doc ends
  // EXACTLY at the incomplete statement, so the parser ends the
  // enclosing block's span at the EOF token and the cursor sits ON it.
  // Before the fn body's scope extension through the source end, every
  // binding of the block dropped and `list.` answered EMPTY at the most
  // common typing moment. The member tier answers through the real ABI
  // path now, and the locals tier (A1) offers the fn-local bindings
  // ahead of the keyword table. ----
  {
    const uri = 'file:///ws/e2e-eof.rut';
    const src = [
      'class List {',
      '    items: i32;',
      '}',
      'impl List {',
      '    fn count(self) -> i32 { return 0; }',
      '}',
      'fn go() -> nil {',
      '    let list = List.new();',
      '    list.',
    ].join('\n');
    // member completion at EOF: cursor right after the final dot, the
    // CURRENT text riding the request (the editor's exact call)
    const lastLine = src.slice(src.lastIndexOf('\n') + 1);
    const items = rut.complete(uri, src.split('\n').length - 1, lastLine.length, src);
    const labels = items.map((i) => i.label);
    for (const want of ['items', 'count']) {
      if (!labels.includes(want)) bad.push(`EOF member completion lacks '${want}' (got ${labels.join(', ')})`);
    }
    if (labels.includes('fn')) bad.push(`EOF member completion leaked the keyword table (got ${labels.join(', ')})`);

    // the locals shape at EOF: `che⏐` after `let mut checked …` — the
    // binding completes ahead of the keywords, riding LSP kind 6
    // (VARIABLE; the extension maps kind - 1, unchanged)
    const lsrc = 'fn go() -> nil {\n    let mut checked: str = " ";\n    che';
    const luri = 'file:///ws/e2e-eof-locals.rut';
    const litems = rut.complete(luri, 2, lsrc.slice(lsrc.lastIndexOf('\n') + 1).length, lsrc);
    const checked = litems.find((i) => i.label === 'checked');
    if (!checked) bad.push(`EOF locals completion lacks 'checked' (got ${litems.map((i) => i.label).join(', ')})`);
    else if (checked.kind !== 6) bad.push(`EOF local 'checked' kind ${checked.kind} != 6 (VARIABLE)`);
    const checkedAt = litems.findIndex((i) => i.label === 'checked');
    const letAt = litems.findIndex((i) => i.label === 'let');
    if (checkedAt >= 0 && letAt >= 0 && checkedAt > letAt) {
      bad.push(`EOF local 'checked' rides behind the keyword table (${checkedAt} > ${letAt})`);
    }
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
  // declaring ident in the REAL pouch source
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
    const real = fs.readFileSync(path.join(REPO, 'rut', 'pouch', 'src', 'mod.rut'), 'utf8');
    const wantUri = path.join('rut', 'pouch', 'src', 'mod.rut');
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

  // ---- phase 3 (lsp-features): INLAY hints through the SHIPPED
  // artifact. CORPUS ground truth: every hint's label is checked
  // against the source's own decl lines (an annotation elsewhere in the
  // file, the callee's ret, or the stdlib's true pouch decl). ----
  {
    const digestPath = path.join(REPO, 'examples', '02-digest', 'src', 'mod.rut');
    const digestSrc = fs.readFileSync(digestPath, 'utf8');
    const digestUri = `file://${digestPath}`;
    const a = rut.analyze(digestUri, digestSrc);
    if (a.diags.length !== 0) {
      bad.push(`digest inlay probe has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const all = rut.inlayHint(
      digestUri,
      { line: 0, character: 0 },
      { line: digestSrc.split('\n').length, character: 0 }
    );
    if (!all.length) bad.push('digest inlay: no hints at all');
    // a TYPE hint hangs at the END of the binding ident (needle = the
    // ident, so the hint position is needle-end); a PARAMETER hint sits
    // at the arg's first byte (needle = the arg's first token)
    const hintAt = (src, lineRe, needle, wantLabel, wantKind, gt, what) => {
      const [line, ch] = posOnLine(src, lineRe, needle);
      const at = wantKind === 1 ? ch + needle.length : ch;
      const h = all.find((x) => x.position.line === line && x.position.character === at);
      if (!h) {
        bad.push(`${what}: no hint at ${line}:${at} (have ${all.filter((x) => x.position.line === line).map((x) => `${x.position.character}:${x.label}`).join(', ')})`);
        return;
      }
      if (h.label !== wantLabel) {
        bad.push(`${what}: label ${JSON.stringify(h.label)} != ${JSON.stringify(wantLabel)}`);
      }
      if (h.kind !== wantKind) {
        bad.push(`${what}: kind ${h.kind} != ${wantKind}`);
      }
      if (gt && !gt.test(src)) {
        bad.push(`${what}: ground-truth line missing from the corpus file`);
      }
    };
    // `let src = "0123456789abcdef";` → str — GT: the file's own
    // `fn hex_val(c: str)` param annotation
    hintAt(digestSrc, /let src = "0123456789abcdef";/, 'src', ': str', 1,
      /^fn hex_val\(c: str\) -> i32 \{/m, 'digest str let');
    // `let d = hex_digits();` → Vec<str> — GT: the file's ANNOTATED
    // `let d: Vec<str> = Vec::new();` three lines up (the corpus spells
    // the paths grammar's `::` since fe7b5f94)
    hintAt(digestSrc, /let d = hex_digits\(\);/, 'd', ': Vec<str>', 1,
      /^    let d: Vec<str> = Vec::new\(\);/m, 'digest vec let');
    // `let v = hex_val(f"{c}");` → i32 — GT: the callee's `-> i32` ret
    hintAt(digestSrc, /let v = hex_val\(f/, 'v', ': i32', 1,
      /^fn hex_val\(c: str\) -> i32 \{/m, 'digest call ret let');
    // for-c counter → i32 — GT: RFC 0007 §1 (the suffixless literal
    // default; the same law the file's `let mut i = 0;` hints ride)
    // (std-v10 era: the mut-law sweep spells the header `let mut i`)
    hintAt(digestSrc, /for \(let mut i = 0; i < n; i \+= 3\) \{/, 'i', ': i32', 1,
      /^    let mut i = 0;/m, 'digest for-c counter');
    // PARAMETER hint at the free call: `hex_val(f"{c}")` → `c:` —
    // GT: the callee's own param name in its decl line
    hintAt(digestSrc, /let v = hex_val\(f"\{c\}"\);/, 'f', 'c:', 2,
      /^fn hex_val\(c: str\) -> i32 \{/m, 'digest free-call param');
    // PARAMETER hint through the CROSS-FILE method: `d.push(f"{c}")` →
    // `v:` — GT: the TRUE stdlib decl `pub fn push(mut self, v: T)`
    const pouchSrc = fs.readFileSync(path.join(REPO, 'rut', 'pouch', 'src', 'mod.rut'), 'utf8');
    hintAt(digestSrc, /d\.push\(f"\{c\}"\);/, 'f', 'v:', 2, null, 'digest push param');
    if (!/pub fn push\(mut self, v: T\) -> nil/.test(pouchSrc)) {
      bad.push('digest push param: ground-truth push decl missing from pouch src/mod.rut');
    }
    // the hover-consistency law, gated through the artifact: the
    // hint's type text IS what hover shows for the same binding
    {
      const [line, ch] = posOnLine(digestSrc, /let d = hex_digits\(\);/, 'd');
      const h = rut.hover(digestUri, line, ch);
      const hint = all.find((x) => x.position.line === line && x.position.character === ch + 1);
      if (!h || !hint || !h.contents.value.includes(`let d: ${hint.label.slice(2)}`)) {
        bad.push(`digest hover/hint consistency lost: hover=${h && JSON.stringify(h.contents.value.slice(0, 60))} hint=${hint && JSON.stringify(hint.label)}`);
      }
    }
    smokes++;
  }

  // ---- phase 3: for-of corpus ground truth — the element of an
  // ANNOTATED Vec<i32> types the loop variable, in a second corpus
  // file (demo/src/examples/closures-generics/src/mod.rut) ----
  {
    const cgPath = path.join(REPO, 'demo', 'src', 'examples', 'closures-generics', 'src', 'mod.rut');
    const cgSrc = fs.readFileSync(cgPath, 'utf8');
    const cgUri = `file://${cgPath}`;
    const a = rut.analyze(cgUri, cgSrc);
    if (a.diags.length !== 0) {
      bad.push(`closures-generics inlay probe has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const all = rut.inlayHint(cgUri, { line: 0, character: 0 }, { line: 9999, character: 0 });
    const hintAt = (lineRe, needle, wantLabel, what) => {
      const [line, ch] = posOnLine(cgSrc, lineRe, needle);
      const h = all.find((x) => x.position.line === line && x.position.character === ch + needle.length);
      if (!h) {
        bad.push(`${what}: no hint at ${line}:${ch + needle.length}`);
        return;
      }
      if (h.label !== wantLabel || h.kind !== 1) {
        bad.push(`${what}: got ${JSON.stringify(h.label)} kind ${h.kind}, wanted ${JSON.stringify(wantLabel)} kind 1`);
      }
    };
    // `for (let x of xs)` → i32 — GT: the iterable's own annotation
    // `fn sum(xs: Vec<i32>)`: the element of Vec<i32> IS i32
    hintAt(/for \(let x of xs\) \{/, 'x', ': i32', 'for-of elem');
    // `let mut total = 0;` → i32 — the RFC 0007 literal default, GT:
    // the file's annotated `a: i32` closure params
    hintAt(/let mut total = 0;/, 'total', ': i32', 'literal default');
    if (!/fn sum\(xs: Vec<i32>\) -> i32 \{/.test(cgSrc)) {
      bad.push('for-of elem: ground-truth Vec<i32> annotation missing');
    }
    smokes++;
  }

  // ---- phase 3: the synthetic laws — annotated lets are NEVER
  // restated, param hints skip `self`, the range filter narrows, and
  // the tooltip is the hover markdown ----
  {
    const uri = 'file:///ws/e2e-inlay.rut';
    const src = [
      'class Circle {',
      '    r: f64;',
      '}',
      'impl Circle {',
      '    fn grown(self, k: f64) -> Circle { return self; }',
      '}',
      'fn go() -> Circle {',
      '    let c = Circle.new(1.0);',
      '    let d: Circle = Circle.new(1.0);',
      '    return c.grown(2.0);',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`inlay probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const all = rut.inlayHint(uri, { line: 0, character: 0 }, { line: 99, character: 0 });
    const labels = all.map((h) => `${h.position.line}:${h.label}(k${h.kind})`);
    // exactly two: the unannotated `c` and the exact-arity grown arg —
    // the ANNOTATED `d` is restated by nothing
    if (labels.join(', ') !== '7:: Circle(k1), 9:k:(k2)') {
      bad.push(`inlay synthetic hints drifted: [${labels.join(', ')}]`);
    }
    const tip = all[0] && all[0].tooltip && all[0].tooltip.value;
    if (!tip || !tip.includes('let c: Circle') || !tip.includes('type inferred')) {
      bad.push(`inlay tooltip is not the hover markdown: ${JSON.stringify(tip)}`);
    }
    // the range filter: only line 7's hint stays inside [7, 8)
    const narrow = rut.inlayHint(uri, { line: 7, character: 0 }, { line: 8, character: 0 });
    if (!(narrow.length === 1 && narrow[0].label === ': Circle')) {
      bad.push(`inlay range filter leaked: ${JSON.stringify(narrow.map((h) => h.label))}`);
    }
    smokes++;
  }

  // ---- phase 4 (lsp-features): REFERENCES round trips — a decl ->
  // the EXACT expected use-position set. Shadow-aware across scopes,
  // and cross-file through the use graph's reverse edges in BOTH
  // directions (lib-decl query finds the importer's sites; doc-side
  // query finds the same set through the doc's own use graph). ----

  // the byte offset of a reference — locations must carry (line, char)
  const refAt = (src, needle, occurrence = 1) => {
    const [line, ch] = posOf(src, needle, occurrence);
    return line * 100000 + ch;
  };
  const refOffsets = (src, locs, uri) =>
    locs
      .filter((l) => l.uri === uri)
      .map((l) => l.range.start.line * 100000 + l.range.start.character)
      .sort((a, b) => a - b);

  // WITHIN-FILE + SHADOW-AWARE: the outer v answers with its if-cond
  // use + final return; the inner shadow answers with exactly its own
  // return — neither leaks across the scope boundary
  {
    const uri = 'file:///ws/e2e-refs-shadow.rut';
    const src = [
      'fn go() -> i32 {',
      '    let v = 1;',
      '    if (v > 0) {',
      '        let v = 2;',
      '        return v;',
      '    }',
      '    return v;',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`refs-shadow probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    // the OUTER decl
    let [dl, dc] = posOf(src, 'let v = 1;', 1);
    let locs = rut.references(uri, dl, dc + 4, false);
    let want = [refAt(src, 'v > 0'), refAt(src, 'return v;', 2) + 7].sort((x, y) => x - y);
    let got = refOffsets(src, locs, uri);
    if (JSON.stringify(got) !== JSON.stringify(want)) {
      bad.push(`outer v refs drifted: ${JSON.stringify(got)} != ${JSON.stringify(want)}`);
    }
    // the INNER decl: exactly its own return
    ;[dl, dc] = posOf(src, 'let v = 2;', 1);
    locs = rut.references(uri, dl, dc + 4, false);
    if (!(locs.length === 1 && locs[0].range.start.line === 4)) {
      bad.push(`inner v refs leaked: ${JSON.stringify(locs)}`);
    }
    // includeDeclaration appends the declaring ident
    locs = rut.references(uri, dl, dc + 4, true);
    if (!(locs.length === 2 && locs.some((l) => l.range.start.line === 3 && l.range.start.character === dc + 4))) {
      bad.push(`includeDeclaration lost the decl ident: ${JSON.stringify(locs)}`);
    }
    smokes++;
  }

  // CROSS-FILE, both directions: the lib is indexed via rut.add_def
  // (pkg matched by its path segment); the importing doc's use name +
  // usage are the EXACT reference set of the lib's decl — queried from
  // the doc side AND from the lib-decl side
  {
    const libUri = 'file:///ws/gadgets/lib.rut';
    const libSrc = 'class Widget {\n    id: i32;\n}\n';
    const mainUri = 'file:///ws/e2e-refs-cross.rut';
    const mainSrc = 'use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget.new();\n}\n';
    rut.addDef(libUri, libSrc);
    rut.addDef(mainUri, mainSrc); // the extension indexes every workspace file
    const a = rut.analyze(mainUri, mainSrc);
    if (a.diags.length !== 0) {
      bad.push(`refs-cross probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const want = [refAt(mainSrc, 'Widget }'), refAt(mainSrc, 'Widget.new')].sort((x, y) => x - y);
    // from the importing doc's usage site
    let [dl, dc] = posOf(mainSrc, 'Widget.new', 1);
    let locs = rut.references(mainUri, dl, dc, false);
    let got = refOffsets(mainSrc, locs, mainUri);
    if (JSON.stringify(got) !== JSON.stringify(want)) {
      bad.push(`cross-file refs (doc side) drifted: ${JSON.stringify(got)} != ${JSON.stringify(want)}`);
    }
    // from the lib's DECL ident: the same two sites, carrying the doc's uri
    const a2 = rut.analyze(libUri, libSrc);
    if (a2.diags.length !== 0) {
      bad.push(`refs-cross lib doc has ${a2.diags.length} diagnostic(s): ${JSON.stringify(a2.diags[0])}`);
    }
    ;[dl, dc] = posOf(libSrc, 'class Widget', 1);
    locs = rut.references(libUri, dl, dc + 6, false);
    if (!(locs.length === 2 && locs.every((l) => l.uri === mainUri))) {
      bad.push(`cross-file refs (lib side) drifted: ${JSON.stringify(locs)}`);
    }
    smokes++;
  }

  // ---- phase 4: SIGNATURE HELP — the verbatim signature, the active
  // parameter from the comma/paren depth (first/middle/nested), the
  // method receiver through its type head, and a mismatch -> no help ----
  {
    const uri = 'file:///ws/e2e-sighelp.rut';
    const src = [
      'fn inner(a: i32) -> i32 {',
      '    return a;',
      '}',
      'fn outer(b: i32, c: i32) -> i32 {',
      '    return b + c;',
      '}',
      'fn hex_val(c: str, k: i32) -> i32 {',
      '    return k;',
      '}',
      'class Circle {',
      '    r: f64;',
      '}',
      'impl Circle {',
      '    fn grown(self, k: f64) -> Circle { return self; }',
      '}',
      'fn arity(a: i32, b: i32) -> i32 {',
      '    return a;',
      '}',
      'fn main() -> i32 {',
      '    let x = outer(inner(1), 2);',
      '    let h = hex_val("a", x);',
      '    let bad: i32 = arity(1, 2, 3);',
      '    let c = Circle.new(1.0);',
      '    let g = c.grown(2.0);',
      '    return x + h + bad + g.r as i32;',
      '}',
      '',
    ].join('\n');
    const a = rut.analyze(uri, src);
    if (a.diags.length !== 0) {
      bad.push(`sighelp probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`);
    }
    const wantHelp = (pos, wantLabel, wantActive, what) => {
      const h = rut.signatureHelp(uri, pos[0], pos[1]);
      if (!h) {
        bad.push(`${what}: no help (wanted ${JSON.stringify(wantLabel)} slot ${wantActive})`);
        return;
      }
      const sig = h.signatures[0];
      if (sig.label !== wantLabel) {
        bad.push(`${what}: label ${JSON.stringify(sig.label)} != ${JSON.stringify(wantLabel)}`);
      }
      if (h.activeParameter !== wantActive) {
        bad.push(`${what}: activeParameter ${h.activeParameter} != ${wantActive}`);
      }
    };
    // FIRST arg of a nested call: the innermost unclosed paren wins
    wantHelp(posOf(src, 'inner(1)', 1).map((v, i) => (i === 1 ? v + 6 : v)), 'fn inner(a: i32) -> i32', 0, 'nested first arg');
    // MIDDLE arg (after the nested call closed): the outer's second slot
    wantHelp(posOf(src, ', 2);', 1).map((v, i) => (i === 1 ? v + 2 : v)), 'fn outer(b: i32, c: i32) -> i32', 1, 'outer second arg');
    // the outer's FIRST arg (the inner call itself): slot 0
    {
      const [ol, oc] = posOf(src, 'outer(inner', 1);
      wantHelp([ol, oc + 6], 'fn outer(b: i32, c: i32) -> i32', 0, 'outer first arg');
    }
    // free call: first arg then second
    wantHelp(posOf(src, '"a"', 1), 'fn hex_val(c: str, k: i32) -> i32', 0, 'free call first arg');
    wantHelp(posOf(src, ', x);', 1).map((v, i) => (i === 1 ? v + 2 : v)), 'fn hex_val(c: str, k: i32) -> i32', 1, 'free call second arg');
    // METHOD call through a typed receiver: self never surfaces
    {
      const [ml, mc] = posOf(src, 'grown(2.0)', 1);
      const h = rut.signatureHelp(uri, ml, mc + 6);
      if (!h || h.signatures[0].label !== 'fn grown(self, k: f64) -> Circle' || h.activeParameter !== 0) {
        bad.push(`method receiver help drifted: ${JSON.stringify(h)}`);
      } else {
        const labels = h.signatures[0].parameters.map((p) => p.label);
        if (JSON.stringify(labels) !== JSON.stringify(['k: f64'])) {
          bad.push(`method param pieces drifted: ${JSON.stringify(labels)}`);
        }
      }
    }
    // a mismatch shows NO help, never wrong help
    {
      const [bl, bc] = posOf(src, '2, 3);', 1);
      const h = rut.signatureHelp(uri, bl, bc + 5);
      if (h !== null) {
        bad.push(`arity mismatch must show no help: ${JSON.stringify(h)}`);
      }
    }
    smokes++;
  }

  // ---- the dedicated postfix-removal diagnostic (M2's acceptance;
  // the RFC sweep reworded it — the message names the fix, not a
  // proposal number) ----
  {
    const b = rut.analyze('file:///ws/e2e-postfix.rut', 'fn f(p: i32?) -> nil {\n}\n');
    if (!(b.diags.length === 1 && b.diags[0].message.includes('postfix spelling'))) {
      bad.push(`postfix T? must give the ONE dedicated postfix-removal diag, got: ${JSON.stringify(b.diags)}`);
    }
    const g = rut.analyze('file:///ws/e2e-prefix.rut', 'fn g(p: ?i32) -> nil {\n}\n');
    if (g.diags.length !== 0) {
      bad.push(`?i32 must analyze clean (M1), got: ${JSON.stringify(g.diags)}`);
    }
    smokes++;
  }

  // ---- phase 5 (manifest-deps): the DEP ABI — rut_parse_manifest /
  // rut_add_def_named / rut_add_bundle — through the SHIPPED artifact's
  // raw exports. Separate instance: its state.defs is the std surface
  // plus whatever this lane indexes, so the bundle assertions read
  // exactly what the ABI landed (the binding lane above keeps its own
  // instance unpolluted for the corpus walk). ----
  {
    const bytes = fs.readFileSync(WASM_PATH);
    const { instance } = await WebAssembly.instantiate(bytes, {});
    const e = instance.exports;
    const enc = new TextEncoder();
    const dec = new TextDecoder();
    const put = (s) => {
      const b = enc.encode(s);
      const p = e.rut_alloc(b.length);
      assert.notStrictEqual(p, 0, 'dep lane: arena overflow');
      new Uint8Array(e.memory.buffer, p, b.length).set(b);
      return [p, b.length];
    };
    // binary (ptr, len) — the bundle bytes are a zip, not text
    const putBytes = (b) => {
      const p = e.rut_alloc(b.length);
      assert.notStrictEqual(p, 0, 'dep lane: arena overflow');
      new Uint8Array(e.memory.buffer, p, b.length).set(b);
      return [p, b.length];
    };
    const take = (ptr) => {
      assert.notStrictEqual(ptr, 0, 'dep lane: arena overflow');
      const len = new DataView(e.memory.buffer, ptr, 4).getUint32(0, true);
      return JSON.parse(dec.decode(new Uint8Array(e.memory.buffer, ptr + 4, len)));
    };

    // the manifest: the repo's own path+url fixture (03-plugin) — the
    // same bytes rut-lsp's dep_table unit test pins
    const manifestPath = path.join(REPO, 'examples', '03-plugin', 'plugin', 'rut.jsonc');
    const manifest = fs.readFileSync(manifestPath, 'utf8');
    e.rut_begin();
    const table = take(e.rut_parse_manifest(...put(manifest)));
    assert.strictEqual(table.name, 'plugin', `manifest name: ${JSON.stringify(table)}`);
    // the body rides the src/mod.rut convention (the entry keys are
    // repealed — the root module is a file under the `src/` shell,
    // never a manifest key)
    assert.deepStrictEqual(table.entries, [{ path: 'src/mod.rut', mode: 'Impl' }],
      `entry modes: ${JSON.stringify(table.entries)}`);
    const row = (name) => table.deps.find((r) => r.name === name);
    assert.deepStrictEqual(row('server'), {
      name: 'server',
      source: { kind: 'path', dir: '../server' },
      optional: false,
      lib: null,
    }, `path row: ${JSON.stringify(row('server'))}`);
    const pouchRow = row('pouch');
    assert.strictEqual(pouchRow.source.kind, 'url');
    // the expected URL comes from the FIXTURE's own bytes, never a
    // hardcoded tag here — a std-tag sweep edits rut.jsonc and this
    // gate follows it, so the two can never drift again (a pinned
    // std-v8 URL once failed this lane BEFORE the corpus lock ran and
    // masked the alignment verdict entirely)
    const pouchUrl = manifest.match(/"pouch"\s*:\s*\{\s*"url"\s*:\s*"([^"]+)"/)[1];
    assert.ok(pouchUrl, 'the fixture manifest carries a pouch url dep');
    assert.strictEqual(pouchRow.source.url, pouchUrl);
    // the manifest row mirrors the fixture bytes — and since the
    // rut.lock migration (66c8e6a8) a manifest url row is
    // { "url": ... } only, so the parsed sha256 rides as null
    assert.strictEqual(pouchRow.source.sha256, null);
    // the pin itself lives in the companion rut.lock (the pin is law
    // at the mount door): hex64, riding the SAME url, and equal to the
    // sha256 of the COMMITTED bundle bytes the url names (the tag
    // segment of the url IS the dist dir) — all derived from fixture
    // bytes, so a std-tag sweep edits the fixtures and this gate
    // follows; no constant here can drift
    const lock = fs.readFileSync(path.join(path.dirname(manifestPath), 'rut.lock'), 'utf8');
    const lockPouch = lock.match(/\{[^{}]*"pkg"\s*:\s*"pouch"[^{}]*\}/)[0];
    assert.strictEqual(lockPouch.match(/"url"\s*:\s*"([^"]+)"/)[1], pouchUrl,
      'the lock row rides the manifest url');
    const pin = lockPouch.match(/"sha256"\s*:\s*"([0-9a-f]{64})"/)[1];
    assert.ok(pin, 'the lock pins the pouch bundle (hex64)');
    const pouchBundle = fs.readFileSync(path.join(REPO, 'dist', pouchUrl.match(/rut@([^/]+)\//)[1], 'pouch.rutbundle'));
    assert.strictEqual(crypto.createHash('sha256').update(pouchBundle).digest('hex'), pin,
      'the lock pin matches the committed bundle bytes');
    // the cache path is computed RUST-side — sha256 of the URL string
    // (crates/rut-lsp/src/deps.rs cache_path); the binding never
    // hashes, so the gate recomputes the SAME digest over the fixture
    // URL and the row must equal it — derived, never a pinned constant
    assert.strictEqual(pouchRow.source.cache_path,
      `.rut/cache/${crypto.createHash('sha256').update(pouchUrl).digest('hex')}.rutbundle`);
    assert.deepStrictEqual(table.dev_deps, []);
    // the peer row set is gone from the wire (the retired `[peer-deps]`
    // table's client-visible removal) — the envelope spells no
    // `peer_deps` row at all
    assert.strictEqual('peer_deps' in table, false,
      'the envelope carries no peer_deps row');
    smokes++;

    // entry.type indexes in Decl mode (the envelope spells the variant)
    const hostManifest = '{ "name": "host", "type": "host", "entry": { "type": "./surface.d.rut" } }';
    e.rut_begin();
    const host = take(e.rut_parse_manifest(...put(hostManifest)));
    assert.deepStrictEqual(host.entries, [{ path: 'src/surface.d.rut', mode: 'Decl' }],
      `decl entry: ${JSON.stringify(host.entries)}`);
    // a malformed manifest is the ERROR envelope — the host's hint
    // (named `manifestErr`, NOT `bad` — that name is the lane's
    // error collector and a shadow here silences every later push)
    e.rut_begin();
    const manifestErr = take(e.rut_parse_manifest(...put('{"name": six}')));
    assert.ok(manifestErr.error && manifestErr.error.startsWith('line 1: '), `error envelope: ${JSON.stringify(manifestErr)}`);
    smokes++;

    // rut_add_def_named: a dep indexed under its module NAME feeds the
    // `use <name>::` tier — the module tier lists the name, the names
    // tier lists the dep's public surface
    const gUri = 'file:///ws/gadgets/lib.rut';
    const gSrc = 'pub class Widget {\n    id: i32;\n}\npub fn tag() -> i32 {\n    return 1;\n}\n';
    e.rut_begin();
    e.rut_add_def_named(...put(gUri), ...put('gadgets'), ...put(gSrc));
    const depDoc = 'use gadgets::\nfn main() -> nil {\n}\n';
    const depUri = 'file:///ws/e2e-dep.rut';
    e.rut_begin();
    take(e.rut_analyze(...put(depUri), ...put(depDoc)));
    e.rut_begin();
    const namesTier = take(e.rut_complete(...put(depUri), 0, depDoc.indexOf('::') + 2));
    const depLabels = namesTier.map((i) => i.label);
    for (const want of ['Widget', 'tag']) {
      if (!depLabels.includes(want)) bad.push(`named dep 'gadgets' lacks '${want}' in use completion (got ${depLabels.join(', ')})`);
    }
    e.rut_begin();
    const modTier = take(e.rut_complete(...put(depUri), 0, 4)); // right after `use `
    if (!modTier.map((i) => i.label).includes('gadgets')) {
      bad.push(`module tier lacks 'gadgets' (got ${modTier.map((i) => i.label).join(', ')})`);
    }
    smokes++;

    // rut_add_bundle: the COMMITTED pouch bundle (the CLI tests'
    // offline url-dep fixture) — the envelope names module + entry
    // paths, the surface lands in hover and go-to-definition
    const bundleBytes = fs.readFileSync(path.join(REPO, 'dist', 'std', 'pouch.rutbundle'));
    e.rut_begin();
    const bundle = take(e.rut_add_bundle(...putBytes(bundleBytes)));
    assert.strictEqual(bundle.module, 'pouch', `bundle envelope: ${JSON.stringify(bundle)}`);
    // wire v12: the compiled group ships its GENERATED declaration
    // entry as the editing surface (the .d.ts pattern) — the envelope
    // names the entry, never the compiled mod.rutc body
    assert.deepStrictEqual(bundle.files, ['mod.d.rut']);
    e.rut_begin();
    const junk = Buffer.from('definitely not a zip');
    const bundleErr = take(e.rut_add_bundle(...putBytes(junk)));
    assert.ok(bundleErr.error && bundleErr.error.includes('not a zip'),
      `bundle error envelope: ${JSON.stringify(bundleErr)}`);
    // the indexed surface resolves: the MOUNTED bundle outranks the
    // ambient std embed (prefer_bodies — the embed is the zero-fetch
    // fallback, never the answer when the real thing is mounted), so
    // the use name jumps to the bundle's declaration entry — exactly
    // one target — and a member hover answers through the named module
    const pouchDoc = 'use pouch::{ Vec };\nfn main() -> nil {\n    let v: Vec<i32> = Vec.new();\n    v.push(1);\n}\n';
    const pouchUri = 'file:///ws/e2e-bundle.rut';
    e.rut_begin();
    take(e.rut_analyze(...put(pouchUri), ...put(pouchDoc)));
    e.rut_begin();
    const vecLocs = take(e.rut_definition(...put(pouchUri), 0, pouchDoc.indexOf('Vec')));
    const targetUris = vecLocs.map((l) => l.uri);
    assert.deepStrictEqual(targetUris, ['bundle:pouch/mod.d.rut'],
      `bundle go-to-definition targets: ${JSON.stringify(targetUris)}`);
    const pushCh = pouchDoc.split('\n')[3].indexOf('push');
    e.rut_begin();
    const pushHover = take(e.rut_hover(...put(pouchUri), 3, pushCh));
    if (!pushHover || !pushHover.contents.value.includes('push')) {
      bad.push(`bundle member hover missed: ${pushHover && JSON.stringify(pushHover.contents.value.slice(0, 60))}`);
    }
    smokes++;

    // ---- plan2 phase 3 (deps face): EXTRACT-TO-DISK — the extension's
    // url-dep mount lane: rut_extract_bundle unpacks WITHOUT indexing,
    // the host writes the files under .rut/deps/<module>-<pin12>/
    // (content-addressed by the byte pin) and re-indexes them through
    // rut_add_def_mod with real file:// origins — go-to-definition
    // lands in REAL files, never bundle: URIs. The BINDING drives the
    // lane (the exact call src/extension.ts mountUrlDep makes), the
    // files are read back from disk. ----
    assert.strictEqual(typeof e.rut_extract_bundle, 'function',
      'rut_extract_bundle missing from the artifact — rebuild (npm run build:wasm)');
    {
      const extracted = rut.extractBundle(bundleBytes);
      assert.ok(!extracted.error, `extract envelope: ${JSON.stringify(extracted)}`);
      assert.strictEqual(extracted.module, 'pouch', `extracted module: ${JSON.stringify(extracted.module)}`);
      assert.strictEqual(extracted.files.length, 1, `extracted files: ${JSON.stringify(extracted.files.map((f) => f.path))}`);
      const [ef] = extracted.files;
      assert.strictEqual(ef.path, 'mod.d.rut');
      assert.strictEqual(ef.mode, 'Decl');
      assert.strictEqual(ef.mod_path, '', `the root rides mod path '': ${JSON.stringify(ef.mod_path)}`);
      assert.ok(ef.src.includes('pub class Vec'), `the body rides the src: ${ef.src.slice(0, 60)}`);
      // a corrupt archive is the error envelope — never a partial tree
      const badExtract = rut.extractBundle(new TextEncoder().encode('definitely not a zip'));
      assert.ok(badExtract.error && badExtract.error.includes('not a zip'),
        `extract error envelope: ${JSON.stringify(badExtract)}`);

      // ---- the source-zip transport's extract, through the SAME
      // binding mountUrlDep drives (the std set ships as source zips —
      // `src/` shells + source-zip deps): `rut_extract_srczip` unpacks
      // the package's own `src/**` files with their mod paths, and the
      // Logger class the scaffold's `use ink::{ Logger }` jumps to rides
      // the body. The unpack law's ingredients ride beside: the
      // archive's own manifest (the walk lays it beside the files) and
      // the embedded `deps/` groups (the transitive host pkgs a
      // scaffold never spells — ink carries ink_host). (The capability
      // seam above guarantees the export exists; this proves the round
      // trip.)
      const inkZip = fs.readFileSync(path.join(REPO, 'dist', 'std', 'ink.zip'));
      const srczip = rut.extractSrczip(new Uint8Array(inkZip));
      assert.ok(!srczip.error, `srczip extract envelope: ${JSON.stringify(srczip)}`);
      assert.strictEqual(srczip.module, 'ink', `srczip module: ${JSON.stringify(srczip)}`);
      const inkRoot = srczip.files.find((f) => f.path === 'src/mod.rut');
      assert.ok(inkRoot, `srczip files: ${JSON.stringify(srczip.files.map((f) => f.path))}`);
      assert.strictEqual(inkRoot.mode, 'Impl', 'the source body indexes in Impl mode');
      assert.strictEqual(inkRoot.mod_path, '', `the src/ shell root answers mod path '': ${JSON.stringify(inkRoot.mod_path)}`);
      assert.ok(inkRoot.src.includes('pub class Logger'), 'the Logger class rides the srczip body');
      assert.ok(
        !srczip.files.some((f) => f.path.startsWith('deps/')),
        `other pkgs' embedded dep trees stay out of THIS pkg's files: ${JSON.stringify(srczip.files.map((f) => f.path))}`
      );
      // the unpack ingredients: the own manifest verbatim, the embedded
      // group's entries verbatim (paths stay archive-relative — the
      // walk lays them under the tree and the pkg's own path rows
      // resolve INTO them)
      assert.ok(srczip.manifest.includes('"name": "ink"'), 'the own manifest text rides');
      const groupManifest = srczip.embedded.find((e) => e.path === 'deps/ink_host@7e9a5d738fbd9797/rut.jsonc');
      assert.ok(groupManifest, `the embedded group's manifest rides: ${JSON.stringify(srczip.embedded.map((e) => e.path))}`);
      assert.ok(groupManifest.src.includes('"name": "ink_host"'), 'the group names itself');
      const groupSurface = srczip.embedded.find((e) => e.path === 'deps/ink_host@7e9a5d738fbd9797/src/ink_host.d.rut');
      assert.ok(groupSurface && groupSurface.src.includes('pub host fn create_logger'), 'the host surface rides the group');
      // THE HOP-2 LAW, through the shipped artifact: mount the laid
      // tree the way the walk does, then the ink source's `use
      // ink_host::{ … }` lands IN the embedded group — head and leaves,
      // all real file:// origins (the ambient embed's repo-relative
      // answer would drop outside any scaffold)
      const ws = fs.mkdtempSync(path.join(os.tmpdir(), 'rut-srczip-'));
      const depDir = path.join(ws, '.rut', 'deps', 'ink-eb2b568316ef');
      const lay = (rel, src) => {
        const p = path.join(depDir, rel);
        fs.mkdirSync(path.dirname(p), { recursive: true });
        fs.writeFileSync(p, src);
      };
      lay('rut.jsonc', srczip.manifest);
      for (const f of srczip.files) lay(f.path, f.src);
      for (const e of srczip.embedded) lay(e.path, e.src);
      const mounted = new Set();
      const walkManifest = (manifestPath) => {
        const dir = path.dirname(manifestPath);
        const t = rut.parseManifest(fs.readFileSync(manifestPath, 'utf8'));
        assert.ok(!t.error, `the laid manifest parses: ${JSON.stringify(t)}`);
        if (mounted.has(t.name)) return; // first mount wins
        mounted.add(t.name);
        for (const e of t.entries) {
          rut.addDefNamed('file://' + path.join(dir, e.path), t.name, fs.readFileSync(path.join(dir, e.path), 'utf8'));
        }
        for (const p of t.entries.filter((e) => e.mode === 'Impl').map((e) => e.path)) {
          const uri = path.join(dir, p);
          const stamped = rut.addDefMod('file://' + uri, t.name, '', fs.readFileSync(uri, 'utf8'));
          assert.ok(!stamped.error, `the root stamps: ${JSON.stringify(stamped)}`);
        }
        for (const row of t.deps) walkManifest(path.join(dir, row.source.dir, 'rut.jsonc'));
      };
      // THE HOP-1 LAW: the lane's OWN files index directly — the zip's
      // published manifest carries no entry rows, so the mini-walk's
      // entries loop cannot see them; the mod path rides the envelope
      // ('' the root) and the uri is the laid file (the mountUrlDep
      // zip lane writes AND indexes tree.files — dropping either half
      // strands every own-symbol click on the ambient repo-relative
      // answer, which opens nowhere outside the rut repo).
      for (const f of srczip.files) {
        const stamped = rut.addDefMod('file://' + path.join(depDir, f.path), srczip.module, f.mod_path, f.src);
        assert.ok(!stamped.error, `the lane's own file indexes: ${JSON.stringify(stamped)}`);
      }
      walkManifest(path.join(depDir, 'rut.jsonc'));
      assert.deepStrictEqual([...mounted].sort(), ['ink', 'ink_host'], 'the embedded group mounts with its parent');
      const inkUri = 'file://' + path.join(depDir, 'src/mod.rut');
      const inkSrc = fs.readFileSync(path.join(depDir, 'src/mod.rut'), 'utf8');
      rut.analyze(inkUri, inkSrc);
      const useLine = inkSrc.split('\n').findIndex((l) => l.includes('use ink_host::'));
      const target = 'file://' + path.join(depDir, 'deps/ink_host@7e9a5d738fbd9797/src/ink_host.d.rut');
      for (const needle of ['ink_host', 'create_logger', 'logger_log']) {
        const c = inkSrc.split('\n')[useLine].indexOf(needle);
        const hits = rut.definition(inkUri, useLine, c);
        assert.strictEqual(hits.length, 1, `hop-2 ${needle}: ${JSON.stringify(hits)}`);
        assert.strictEqual(hits[0].uri, target, `hop-2 ${needle} lands in the embedded group`);
      }
      smokes++;

      // hop-1, the scaffold's actual gesture: a consumer's `Logger(`
      // click lands IN the laid tree's own src/mod.rut — a real
      // openable file:// — never the ambient `rut/ink/...` answer
      const consumerDoc = 'use ink::{ Logger };\nentry fn main() {\n    let log = Logger("hello");\n}\n';
      const consumerUri = 'file://' + path.join(ws, 'src', 'mod.rut');
      rut.analyze(consumerUri, consumerDoc);
      const cLines = consumerDoc.split('\n');
      const ctorLine = cLines.findIndex((l) => l.includes('Logger("hello")'));
      const ctorCol = cLines[ctorLine].indexOf('Logger(') + 2;
      const hop1 = rut.definition(consumerUri, ctorLine, ctorCol);
      const hop1Own = hop1.filter((l) => l.uri.includes('.rut/deps/ink-'));
      assert.strictEqual(hop1Own.length, 1, `hop-1 Logger targets: ${JSON.stringify(hop1)}`);
      assert.ok(
        !hop1.some((l) => l.uri.startsWith('rut/')),
        `no ambient repo-relative answer beside the mounted tree: ${JSON.stringify(hop1)}`
      );
      assert.strictEqual(
        hop1Own[0].uri,
        'file://' + path.join(depDir, 'src/mod.rut'),
        'hop-1 lands in the laid tree'
      );
      assert.ok(
        fs.readFileSync(hop1Own[0].uri.replace(/^file:\/\//, ''), 'utf8').includes('pub class Logger'),
        'the hop-1 target is the Logger body, read back from disk'
      );
      // the binding's null-coercion law: an unanalyzed doc answers []
      // (the provider never sees null — the TypeError class dies here)
      assert.deepStrictEqual(rut.definition('file:///nowhere/x.rut', 0, 5), [], 'null definition coerces to []');
      assert.deepStrictEqual(
        rut.inlayHint('file:///nowhere/x.rut', { line: 0, character: 0 }, { line: 1, character: 0 }),
        [],
        'null inlay coerces to []'
      );
      smokes++;

      // the mount: <tmp>/.rut/deps/pouch-<pin12>/ — the pin prefix IS
      // the content address (same bytes ⇒ same dir, a bump ⇒ a sibling)
      const pin12 = crypto.createHash('sha256').update(bundleBytes).digest('hex').slice(0, 12);
      const wsRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'rut-e2e-deps-'));
      try {
        const depDir = path.join(wsRoot, '.rut', 'deps', `pouch-${pin12}`);
        for (const xf of extracted.files) {
          const fp = path.join(depDir, xf.path);
          fs.mkdirSync(path.dirname(fp), { recursive: true });
          fs.writeFileSync(fp, xf.src);
          const stamped = rut.addDefMod(`file://${fp}`, extracted.module, xf.mod_path, xf.src);
          assert.ok(!stamped.error, `extracted file indexes: ${JSON.stringify(stamped)}`);
        }
        const pdoc = 'use pouch::{ Vec };\nfn main() -> nil {\n    let v: Vec<i32> = Vec.new();\n}\n';
        const puri = 'file:///ws/e2e-deps.rut';
        const pa = rut.analyze(puri, pdoc);
        assert.strictEqual(pa.diags.length, 0, `extracted-deps probe doc: ${JSON.stringify(pa.diags)}`);

        // the LEAF click: exactly one location under the extracted
        // tree, the declaring `Vec` ident of the body file (read back
        // from DISK — the target is an openable real file)
        const depLocs = rut.definition(puri, ...posOf(pdoc, 'Vec'))
          .filter((l) => l.uri.includes('.rut/deps/pouch-'));
        assert.strictEqual(depLocs.length, 1,
          `extracted-dir Vec targets: ${JSON.stringify(rut.definition(puri, ...posOf(pdoc, 'Vec')))}`);
        const bodySrc = fs.readFileSync(depLocs[0].uri.replace(/^file:\/\//, ''), 'utf8');
        assert.ok(bodySrc.includes('pub class Vec'), 'the extracted target is the body file');
        eqRange(depLocs, { uri: depLocs[0].uri, range: identAt(bodySrc, /^pub class Vec/, 'Vec'), len: 3 },
          'extracted pouch leaf (body preferred)');

        // the PKG-HEAD click: the same extracted root file, top
        const headLocs = rut.definition(puri, ...posOf(pdoc, 'pouch'));
        assert.strictEqual(headLocs.length, 1, `pkg head targets: ${JSON.stringify(headLocs)}`);
        assert.ok(headLocs[0].uri.includes('.rut/deps/pouch-') && headLocs[0].uri.endsWith('/mod.d.rut'),
          `pkg head lands in the extracted tree: ${headLocs[0].uri}`);
        assert.deepStrictEqual(headLocs[0].range,
          { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
          `pkg head opens the file top: ${JSON.stringify(headLocs[0].range)}`);
        smokes++;

        // ---- the path-dep lane: a mod-tree dep's REAL files — the
        // middle segment lands on the `pub mod layout;` edge, the leaf
        // in the child file ----
        const toolsDir = path.join(wsRoot, 'tools2');
        const rootSrc = 'pub mod layout;\npub fn tag() -> i32 { return 1; }\n';
        const childSrc = 'pub struct Column {\n    w: i32;\n}\npub fn mk() -> Column { return Column { w: 1 }; }\nmod grid;\n';
        fs.mkdirSync(path.join(toolsDir, 'src', 'layout'), { recursive: true });
        fs.writeFileSync(path.join(toolsDir, 'src', 'mod.rut'), rootSrc);
        fs.writeFileSync(path.join(toolsDir, 'src', 'layout', 'mod.rut'), childSrc);
        let ts = rut.addDefMod(`file://${path.join(toolsDir, 'src', 'mod.rut')}`, 'tools2', '', rootSrc);
        assert.ok(!ts.error, `tools2 root indexes: ${JSON.stringify(ts)}`);
        ts = rut.addDefMod(`file://${path.join(toolsDir, 'src', 'layout', 'mod.rut')}`, 'tools2', 'layout', childSrc);
        assert.ok(!ts.error, `tools2 child indexes: ${JSON.stringify(ts)}`);

        const mdoc = 'use tools2::layout::{ Column };\nfn main() -> nil {\n    let c: Column = Column { w: 1 };\n}\n';
        const muri = 'file:///ws/e2e-dep-mods.rut';
        const ma = rut.analyze(muri, mdoc);
        assert.strictEqual(ma.diags.length, 0, `dep-mods probe doc: ${JSON.stringify(ma.diags)}`);
        // the MIDDLE segment: the mounting edge in the root file
        const midLocs = rut.definition(muri, 0, 'use tools2::'.length);
        assert.strictEqual(midLocs.length, 1, `middle segment targets: ${JSON.stringify(midLocs)}`);
        assert.ok(midLocs[0].uri.endsWith('/tools2/src/mod.rut'), `middle segment lands in the root: ${midLocs[0].uri}`);
        eqRange(midLocs, { uri: midLocs[0].uri, range: identAt(rootSrc, /^pub mod layout;/, 'layout'), len: 6 },
          'path-dep middle segment');
        // the LEAF: the decl in the child file
        const leafLocs = rut.definition(muri, ...posOf(mdoc, 'Column'));
        assert.strictEqual(leafLocs.length, 1, `leaf targets: ${JSON.stringify(leafLocs)}`);
        assert.ok(leafLocs[0].uri.endsWith('/tools2/src/layout/mod.rut'), `leaf lands in the child: ${leafLocs[0].uri}`);
        eqRange(leafLocs, { uri: leafLocs[0].uri, range: identAt(childSrc, /^pub struct Column/, 'Column'), len: 6 },
          'path-dep leaf');
        // and the usage site rides the same edge into the child
        const usageLocs = rut.definition(muri, ...posOf(mdoc, 'Column { w: 1 }'));
        assert.ok(usageLocs.some((l) => l.uri.endsWith('/tools2/src/layout/mod.rut')),
          `usage into the child: ${JSON.stringify(usageLocs)}`);

        // ---- the editor-agreement lane: the doc IS a package file —
        // the mod-edge click opens the mounted child at its file top,
        // and a file-form child clicks to its REAL file (never a
        // re-derived grid/mod.rut) ----
        const gridSrc = 'pub fn dig() -> i32 { return 1; }\n';
        fs.writeFileSync(path.join(toolsDir, 'src', 'layout', 'grid.rut'), gridSrc);
        ts = rut.addDefMod(`file://${path.join(toolsDir, 'src', 'layout', 'grid.rut')}`, 'tools2', 'layout/grid', gridSrc);
        assert.ok(!ts.error, `tools2 grid.rut indexes: ${JSON.stringify(ts)}`);

        const rootUri = `file://${path.join(toolsDir, 'src', 'mod.rut')}`;
        rut.analyze(rootUri, rootSrc); // the doc's stamp rides its indexed twin
        const edgeLocs = rut.definition(rootUri, 0, 'pub mod '.length);
        assert.strictEqual(edgeLocs.length, 1, `edge click targets: ${JSON.stringify(edgeLocs)}`);
        assert.ok(edgeLocs[0].uri.endsWith('/tools2/src/layout/mod.rut'),
          `edge click lands in the child: ${edgeLocs[0].uri}`);
        assert.deepStrictEqual(edgeLocs[0].range,
          { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
          `edge click opens the file top: ${JSON.stringify(edgeLocs[0].range)}`);

        const layoutUri = `file://${path.join(toolsDir, 'src', 'layout', 'mod.rut')}`;
        rut.analyze(layoutUri, childSrc);
        const gridLocs = rut.definition(layoutUri, ...posOf(childSrc, 'grid'));
        assert.strictEqual(gridLocs.length, 1, `file-form child targets: ${JSON.stringify(gridLocs)}`);
        assert.ok(gridLocs[0].uri.endsWith('/tools2/src/layout/grid.rut'),
          `file-form child lands in grid.rut: ${gridLocs[0].uri}`);
        assert.deepStrictEqual(gridLocs[0].range,
          { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
          `file-form child opens the file top: ${JSON.stringify(gridLocs[0].range)}`);
        smokes++;
      } finally {
        fs.rmSync(wsRoot, { recursive: true, force: true });
      }
    }

    // ---- file modules (the mod walk): rut_add_def_mod stamps the
    // tree — the root's `pub mod layout;` rides back as the mount
    // walk's next hop, `use tools::layout::` completes the walked
    // module's exports (never the private edge), and the use-path
    // segment hover names the mounted file. A fresh package name: the
    // lane's earlier flat `gadgets` index stays the transitional shape
    // it was indexed as. ----
    e.rut_begin();
    const mRoot = 'pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n';
    const rootOut = take(e.rut_add_def_mod(
      ...put('file:///ws/tools/src/mod.rut'), ...put('tools'), ...put(''), ...put(mRoot)
    ));
    assert.deepStrictEqual(rootOut.decls, [
      { name: 'layout', vis: 'pub' },
      { name: 'secret', vis: 'mod' },
    ], `root mod decls: ${JSON.stringify(rootOut.decls)}`);
    e.rut_begin();
    const mChild = 'pub struct Column {\n    w: i32;\n}\npub fn mk() -> Column { return Column { w: 1 }; }\n';
    const childOut = take(e.rut_add_def_mod(
      ...put('file:///ws/tools/src/layout/mod.rut'), ...put('tools'), ...put('layout'), ...put(mChild)
    ));
    assert.deepStrictEqual(childOut.decls, [], `leaf mod decls: ${JSON.stringify(childOut.decls)}`);

    const modDoc = 'use tools::layout::{ Column };\nfn main() -> nil {\n    let c: Column = Column { w: 1 };\n}\n';
    const modUri = 'file:///ws/e2e-mods.rut';
    e.rut_begin();
    take(e.rut_analyze(...put(modUri), ...put(modDoc)));
    // the walked tier: the resolved module's exports
    e.rut_begin();
    const walkTier = take(e.rut_complete(...put(modUri), 0, 'use tools::layout::'.length));
    const walkLabels = walkTier.map((i) => i.label);
    for (const want of ['Column', 'mk']) {
      assert.ok(walkLabels.includes(want), `mod walk lacks '${want}' (got ${walkLabels.join(', ')})`);
    }
    // the root tier: the pub mod child offers, the private edge and
    // the child's members never leak
    e.rut_begin();
    const rootTier = take(e.rut_complete(...put(modUri), 0, 'use tools::'.length));
    const rootLabels = rootTier.map((i) => i.label);
    assert.ok(rootLabels.includes('layout'), `root tier mod edges: ${JSON.stringify(rootLabels)}`);
    assert.ok(!rootLabels.includes('secret'), `a bare \`mod\` edge never crosses: ${JSON.stringify(rootLabels)}`);
    assert.ok(!rootLabels.includes('Column'), `child members stay under their path: ${JSON.stringify(rootLabels)}`);
    // the segment hover resolves the mounted module
    e.rut_begin();
    const segHover = take(e.rut_hover(...put(modUri), 0, 'use tools::layout::'.length - 3));
    assert.ok(segHover && segHover.contents.value.includes('layout/mod.rut'),
      `segment hover missed: ${segHover && JSON.stringify(segHover.contents.value.slice(0, 80))}`);
    smokes++;

    // ---- the completion lane's doc-sync ABI (rut_complete_in): the
    // CURRENT text is stored BEFORE the tiers run, so a stale stored doc
    // (the tail still short the dot — the analyze debounce's window)
    // cannot mask the member tier, and the dot guard holds through the
    // raw entry (an unresolved receiver answers [], never the dump). ----
    assert.strictEqual(typeof e.rut_complete_in, 'function',
      'rut_complete_in missing from the artifact — rebuild (npm run build:wasm)');
    const syncUri = 'file:///ws/e2e-sync.rut';
    const staleDoc = 'fn go() -> nil {\n    let log = Logger("h");\n    log\n}\n';
    const freshDoc = 'fn go() -> nil {\n    let log = Logger("h");\n    log.\n}\n';
    e.rut_begin();
    take(e.rut_analyze(...put(syncUri), ...put(staleDoc)));
    e.rut_begin();
    const synced = take(e.rut_complete_in(...put(syncUri), ...put(freshDoc), 2, 8));
    const syncedLabels = synced.map((i) => i.label);
    for (const want of ['new', 'debug', 'info', 'warn', 'error']) {
      assert.ok(syncedLabels.includes(want),
        `doc-sync completion lacks '${want}' over the stale stored doc (got ${syncedLabels.join(', ')})`);
    }
    assert.ok(!syncedLabels.includes('abs'),
      `doc-sync completion leaked the host fn 'abs' (got ${syncedLabels.join(', ')})`);
    // the dot guard through the raw entry: `probe().` — [] exactly
    const guardDoc = 'fn go() -> nil {\n    let x = probe().\n}\n';
    e.rut_begin();
    const guarded = take(e.rut_complete_in(...put(syncUri), ...put(guardDoc), 1, 20));
    assert.deepStrictEqual(guarded, [], `dot guard through the raw entry: ${JSON.stringify(guarded)}`);
    smokes++;

    // ---- plan3 phase 4 (B1+B2): SEMANTIC DIAGNOSTICS — the check lane.
    // A doc whose package is STAMPED (rut_add_def_mod named it) compiles
    // with its dep closure: the checker's type errors surface with the
    // right file and range — the bool→str assignment mismatch AND the
    // missing record field (the diagnosis pair). The dep rides the chain
    // (mounted via rut_add_def_mod); an UNSTAMPED doc with the same text
    // stays parse-only clean — today's behavior, the corpus lock above
    // depends on it. ----
    {
      const chkDepSrc =
        'pub struct Task {\n    id: u32;\n    title: str;\n    completed: bool;\n}\n';
      e.rut_begin();
      const chkDep = take(
        e.rut_add_def_mod(...put('file:///ws/chk/dep/src/mod.rut'), ...put('chkdep'), ...put(''), ...put(chkDepSrc))
      );
      assert.ok(Array.isArray(chkDep.decls), `chkdep stamps: ${JSON.stringify(chkDep)}`);
      const chkAppSrc = [
        'use chkdep::{ Task };',
        'fn main() -> nil {',
        '    let mut checked: str = " ";',
        '    checked = true;',
        '    let t = Task { title: "x", completed: false };',
        '}',
        '',
      ].join('\n');
      const chkAppUri = 'file:///ws/chk/app/src/mod.rut';
      e.rut_begin();
      const chkApp = take(
        e.rut_add_def_mod(...put(chkAppUri), ...put('chkapp'), ...put(''), ...put(chkAppSrc))
      );
      assert.ok(Array.isArray(chkApp.decls), `chkapp stamps: ${JSON.stringify(chkApp)}`);
      e.rut_begin();
      const chkMerged = take(e.rut_analyze(...put(chkAppUri), ...put(chkAppSrc)));
      const chkDiags = chkMerged.diags || [];
      const mismatch = chkDiags.find((d) => (d.message || '').includes('assignment type mismatch'));
      const missingField = chkDiags.find((d) => (d.message || '').includes('id` is missing'));
      if (!mismatch) {
        bad.push(`check lane: no assignment-mismatch diag (got ${JSON.stringify(chkDiags.map((d) => d.message))})`);
      } else {
        if (mismatch.source !== 'rut') bad.push(`check lane: mismatch source ${mismatch.source} != rut`);
        if (mismatch.range.start.line !== 3) {
          bad.push(`check lane: mismatch diag on line ${mismatch.range.start.line} != 3 (${JSON.stringify(mismatch.range)})`);
        }
      }
      if (!missingField) {
        bad.push(`check lane: no missing-field diag (got ${JSON.stringify(chkDiags.map((d) => d.message))})`);
      } else if (missingField.range.start.line !== 4) {
        bad.push(`check lane: missing-field diag on line ${missingField.range.start.line} != 4`);
      }
      // the parse-only fallback: the SAME text at an unstamped uri
      e.rut_begin();
      const chkPlain = take(e.rut_analyze(...put('file:///ws/chk/plain.rut'), ...put(chkAppSrc)));
      if ((chkPlain.diags || []).length !== 0) {
        bad.push(`unstamped doc must stay parse-only clean: ${JSON.stringify(chkPlain.diags)}`);
      }
      smokes++;
    }
  }

  // ---- plan6 phase 1 (the lock feed): rut_set_lock — the pin ledger
  // the dep walk feeds from the workspace root's rut.lock BEFORE its
  // manifest parses. Drives the REAL pair (examples/03-plugin's
  // committed rut.jsonc + rut.lock) through the BINDING (the exact
  // calls src/extension.ts's walkDeps/mountUrlDep make) on a fresh
  // instance: the unlocked parse answers sha256 null (the honest
  // unpinned refusal), the fed parse stamps the pin, and the whole
  // url-dep chain lands go-to-definition in the extracted .rut/deps
  // tree — the chain the editor lane was missing. ----
  {
    const manifestPath = path.join(REPO, 'examples', '03-plugin', 'plugin', 'rut.jsonc');
    const manifest = fs.readFileSync(manifestPath, 'utf8');
    const lock = fs.readFileSync(path.join(path.dirname(manifestPath), 'rut.lock'), 'utf8');
    const pouchUrl = manifest.match(/"pouch"\s*:\s*\{\s*"url"\s*:\s*"([^"]+)"/)[1];
    assert.ok(pouchUrl, 'the fixture manifest carries a pouch url dep');
    const inst = await RutWasm.load(WASM_PATH);
    // BEFORE any setLock: the url row answers sha256 null — without a
    // lock the unpinned refusal stays honest, never a guessed pin
    const bare = inst.parseManifest(manifest);
    assert.ok(!bare.error, `unlocked manifest parse: ${JSON.stringify(bare)}`);
    assert.strictEqual(bare.deps.find((r) => r.name === 'pouch').source.sha256, null,
      'the unlocked parse must leave the url row unpinned');
    // setLock(<real rut.lock bytes>) answers the ledger size (derived
    // from the fixture's rows, never a constant); the re-parse stamps
    // the pin onto the url row — and the pin IS the committed bundle
    // bytes' sha256 (the mount door's law, from fixture bytes alone)
    const fed = inst.setLock(lock);
    const wantPins = (lock.match(/"pkg"\s*:/g) || []).length;
    assert.deepStrictEqual(fed, { pins: wantPins }, `setLock envelope: ${JSON.stringify(fed)}`);
    const pinned = inst.parseManifest(manifest);
    assert.ok(!pinned.error, `pinned manifest parse: ${JSON.stringify(pinned)}`);
    const pouchRow = pinned.deps.find((r) => r.name === 'pouch');
    assert.strictEqual(pouchRow.source.url, pouchUrl);
    const lockPouch = lock.match(/\{[^{}]*"pkg"\s*:\s*"pouch"[^{}]*\}/)[0];
    const pin = lockPouch.match(/"sha256"\s*:\s*"([0-9a-f]{64})"/)[1];
    assert.strictEqual(pouchRow.source.sha256, pin, 'the lock-sourced pin rides the url row');
    const bundleBytes = fs.readFileSync(
      path.join(REPO, 'dist', pouchUrl.match(/rut@([^/]+)\//)[1], 'pouch.rutbundle'));
    assert.strictEqual(crypto.createHash('sha256').update(bundleBytes).digest('hex'), pin,
      'the lock pin matches the committed bundle bytes');
    // a malformed lock is the error envelope — and the ledger stands
    // EMPTY after it (the re-parse answers unpinned, never guessed
    // pins); the same holds for the miss lane's empty bytes (a folder
    // with no lock — walkDeps hands over '' to clear the ledger)
    const badLock = inst.setLock(
      '{"format": "rutlock", "format_version": 1, "deps": [{"pkg": "p", "url": "https://x/p.rutbundle", "sha256": "nothex"}]}');
    assert.ok(badLock.error && badLock.error.includes('64 hex digits'),
      `malformed lock envelope: ${JSON.stringify(badLock)}`);
    assert.strictEqual(inst.parseManifest(manifest).deps.find((r) => r.name === 'pouch').source.sha256, null,
      'a malformed lock must clear the ledger, not keep it');
    assert.ok(inst.setLock('').error, 'the empty-bytes miss lane answers the error envelope');
    assert.strictEqual(inst.parseManifest(manifest).deps.find((r) => r.name === 'pouch').source.sha256, null,
      'the miss lane must leave the ledger empty');
    // the whole chain: extract the PINNED bundle under
    // .rut/deps/<module>-<pin12>/ — pin12 from the LOCK-SOURCED row —
    // index through addDefMod/addDefNamed (the exact mountUrlDep
    // moves), and the `use` leaf answers the file:// location whose
    // content holds the decl
    const extracted = inst.extractBundle(bundleBytes);
    assert.ok(!extracted.error, `extract envelope: ${JSON.stringify(extracted)}`);
    const wsRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'rut-e2e-lock-'));
    try {
      const depDir = path.join(wsRoot, '.rut', 'deps', `pouch-${pin.slice(0, 12)}`);
      for (const xf of extracted.files) {
        const fp = path.join(depDir, xf.path);
        fs.mkdirSync(path.dirname(fp), { recursive: true });
        fs.writeFileSync(fp, xf.src);
        if (xf.mod_path === null) {
          inst.addDefNamed(`file://${fp}`, extracted.module, xf.src);
          continue;
        }
        const stamped = inst.addDefMod(`file://${fp}`, extracted.module, xf.mod_path, xf.src);
        assert.ok(!stamped.error, `extracted file indexes: ${JSON.stringify(stamped)}`);
      }
      const pdoc = 'use pouch::{ Vec };\nfn main() -> nil {\n    let v = Vec.new();\n}\n';
      const puri = 'file:///ws/e2e-lock-feed.rut';
      const pa = inst.analyze(puri, pdoc);
      assert.strictEqual(pa.diags.length, 0, `lock-feed probe doc: ${JSON.stringify(pa.diags)}`);
      const leaf = inst.definition(puri, ...posOf(pdoc, 'Vec'))
        .filter((l) => l.uri.includes(`.rut/deps/pouch-${pin.slice(0, 12)}`));
      assert.strictEqual(leaf.length, 1,
        `lock-fed leaf targets: ${JSON.stringify(inst.definition(puri, ...posOf(pdoc, 'Vec')))}`);
      const body = fs.readFileSync(leaf[0].uri.replace(/^file:\/\//, ''), 'utf8');
      assert.ok(body.includes('pub class Vec'), 'the lock-fed target holds the decl');
      eqRange(leaf, { uri: leaf[0].uri, range: identAt(body, /^pub class Vec/, 'Vec'), len: 3 },
        'lock-fed extracted leaf');
    } finally {
      fs.rmSync(wsRoot, { recursive: true, force: true });
    }
    smokes++;
  }

  if (bad.length) {
    console.error(`e2e-wasm: ${bad.length} violation(s):\n  ${bad.slice(0, 40).join('\n  ')}${bad.length > 40 ? `\n  … (+${bad.length - 40} more)` : ''}`);
    process.exit(1);
  }
  console.log(`e2e-wasm: PASS — ${files.length} corpus files, 0 false diagnostics, ${symbols} symbols, ` +
    `${smokes} smoke assertions (legend/fixture/?T hover/primitives/member-nullable/std-completion/ctor-completion+dot-guard/postfix-removal/` +
    `field-decl-hover/inferred-ident/field-read-receiver/for-of-receiver/primitive-hover/` +
    `def-within/def-cross/def-std/typeDefinition/` +
    `inlay-corpus-ground-truth/inlay-for-of-corpus/inlay-synthetic-laws/` +
    `refs-shadow/refs-cross/sighelp/` +
    `manifest-dep-table/manifest-decl+error/named-dep-use-tier/bundle-pouch-surface/` +
    `extract-deps-leaf+head/path-dep-segments/` +
    `eof-member+locals/` +
    `complete-in-doc-sync/check-lane-mismatch+missing-field/` +
    `lock-feed-unpinned+pinned+extract-leaf) ` +
    `through bin/rut-lsp.wasm`);
}

main().catch((e) => {
  console.error('e2e-wasm: FAILED —', e);
  process.exit(1);
});
