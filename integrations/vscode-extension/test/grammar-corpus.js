// Standalone TextMate grammar gate — the extension-side corpus lock.
// Drives syntaxes/rut.tmLanguage.json through vscode-textmate +
// vscode-oniguruma (the same engine VS Code uses) directly over the repo
// corpus: rut/ + examples/ + demo/src/examples/ + benches/workloads/.
//
// Phase-2 scope decision: this gate asserts the GRAMMAR ONLY — it does
// not load rut-lsp.wasm (that binary is rebuilt by the parallel task and
// the through-wasm e2e gate is phase 3; see docs/lsp-survey-extension.md
// §7). No VS Code host needed: plain node, runs in milliseconds.
//
// Asserts the M1-M8 alignment invariants from docs/lsp-survey-extension.md
// against RFC ground truth (crates/rut-parser `RESERVED_KW` /
// `is_primitive_ty`, RFCs 0009 v1.1, 0042, 0043, 0044).
'use strict';

const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const tg = require('vscode-textmate');
const oniguruma = require('vscode-oniguruma');

const EXT = path.join(__dirname, '..');
const REPO = path.join(EXT, '..', '..');
const GRAMMAR_PATH = path.join(EXT, 'syntaxes', 'rut.tmLanguage.json');

// corpus roots, mirroring crates/rut-lsp/tests/corpus.rs
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

async function makeGrammar(grammarPath) {
  const wasmBin = fs.readFileSync(
    path.join(path.dirname(require.resolve('vscode-oniguruma')), 'onig.wasm'));
  await oniguruma.loadWASM(
    wasmBin.buffer.slice(wasmBin.byteOffset, wasmBin.byteOffset + wasmBin.byteLength));
  const registry = new tg.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (sources) => new oniguruma.OnigScanner(sources),
      createOnigString: (s) => new oniguruma.OnigString(s),
    }),
    loadGrammar: async (scopeName) => {
      if (scopeName === 'source.rut') {
        return tg.parseRawGrammar(fs.readFileSync(grammarPath, 'utf8'), grammarPath);
      }
      return null;
    },
  });
  const grammar = await registry.loadGrammar('source.rut');
  assert.ok(grammar, 'source.rut grammar failed to load');
  return grammar;
}

// tokenize a whole document, flattening to {file, line, start, text, scopes}
function tokenizeDoc(grammar, file, source) {
  const out = [];
  let ruleStack = tg.INITIAL;
  const lines = source.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const { tokens, ruleStack: next } = grammar.tokenizeLine(lines[i], ruleStack);
    ruleStack = next;
    for (const t of tokens) {
      out.push({ file, line: i + 1, start: t.startIndex, text: lines[i].slice(t.startIndex, t.endIndex), scopes: t.scopes });
    }
  }
  return out;
}

const hasScope = (tok, prefix) =>
  tok.scopes.some((s) => s === prefix || s.startsWith(prefix + '.'));
const inComment = (tok) => hasScope(tok, 'comment');
const inString = (tok) => hasScope(tok, 'string');
const isCode = (tok) => !inComment(tok) && !inString(tok);

// ---- checks -------------------------------------------------------------
// Each check walks tokens and pushes human-readable violations.

function checkDeadWords(tok, bad) {
  // M1/M2: `dataclass` (lexer hard error, "spell it struct") and `where`
  // (removed, RFC 0043) must NEVER carry a keyword scope.
  if ((tok.text === 'dataclass' || tok.text === 'where') && hasScope(tok, 'keyword')) {
    bad.push(`${tok.file}:${tok.line} dead word '${tok.text}' is keyword-scoped (scopes: ${tok.scopes.join(' ')})`);
  }
  // M3: `string` is not a rut type — must never be primitive/builtin-scoped.
  if (tok.text === 'string' && hasScope(tok, 'storage.type')) {
    bad.push(`${tok.file}:${tok.line} 'string' is storage.type-scoped — not a rut type`);
  }
}

function checkCodeWord(tok, bad, word, wantScope, label) {
  if (tok.text !== word || !isCode(tok)) return;
  if (!hasScope(tok, wantScope)) {
    bad.push(`${tok.file}:${tok.line} '${word}' (${label}) not scoped ${wantScope} (scopes: ${tok.scopes.join(' ')})`);
  }
}

function checkTypeAliasLine(line, i, file, toks, bad) {
  // M5: `type X = …;` at decl head — the introducer must be keyword-scoped.
  if (!/^\s*type\s+[A-Za-z_]/.test(line)) return;
  const hit = toks.some((t) => t.text === 'type' && hasScope(t, 'keyword.other.rut'));
  if (!hit) {
    bad.push(`${file}:${i + 1} 'type' alias introducer not keyword-scoped`);
  }
}

// ---- smoke snippets (what the corpus cannot exercise) --------------------
const SNIPPETS = [
  // M1: `dataclass` is a lexer error — the grammar must not endorse it.
  { name: 'M1 dataclass not keyword', src: 'let dataclass = 1;', text: 'dataclass', notScope: 'keyword' },
  // M2: `where` left RESERVED_KW (RFC 0043) — now a legal identifier.
  { name: 'M2 where not keyword', src: 'let where = 1;', text: 'where', notScope: 'keyword' },
  // M1: struct decls color.
  { name: 'M1 struct keyword', src: 'struct P { x: i32 }', text: 'struct', scope: 'keyword.other.rut' },
  // M3: real primitives color; the dead `string` does not.
  { name: 'M3 str primitive', src: 'fn f(s: str, b: bytes) -> str;', text: 'str', scope: 'storage.type.primitive.rut' },
  { name: 'M3 bytes primitive', src: 'fn f(s: str, b: bytes) -> str;', text: 'bytes', scope: 'storage.type.primitive.rut' },
  { name: 'M3 string not a type', src: 'let s: string = "x";', text: 'string', notScope: 'storage.type' },
  // M3: opaque (boot primitive, RFC 0014) colors in type position;
  // call-shaped uses keep one readable scope too (survey §2 M3).
  { name: 'M3 opaque builtin', src: 'fn g(o: opaque) -> opaque;', text: 'opaque', scope: 'storage.type.builtin.rut' },
  // M4: nullable `?` gets a type affordance (prefix; `[?T]` and `?[T]`).
  { name: 'M4 ?T nullable', src: 'fn h(p: ?T) -> ?T;', text: '?', scope: 'storage.type.nullable.rut' },
  { name: 'M4 ?[T] nullable array', src: 'let na: ?[i32] = nil;', text: '?', scope: 'storage.type.nullable.rut' },
  // M5: contextual `type` alias introducer (anchored, not blanket).
  { name: 'M5 type alias introducer', src: 'type Meters = i64;', text: 'type', scope: 'keyword.other.rut' },
  // M5 negative: `type` as an identifier stays plain.
  { name: 'M5 type identifier stays plain', src: 'let type = 3;', text: 'type', notScope: 'keyword' },
  // M6: `..` is operator-classed the day range/slice syntax lands;
  // a single `.` stays punctuation.
  { name: 'M6 dotdot operator', src: 'let r = 0..10;', text: '..', scope: 'keyword.operator.rut' },
  { name: 'M6 single dot punctuation', src: 'let n = v.len;', text: '.', scope: 'punctuation.separator.rut' },
  // M8: builtin decl heads color; prose/comment mentions do not.
  { name: 'M8 builtin keyword', src: 'builtin primitive opaque {', text: 'builtin', scope: 'keyword.other.rut' },
  { name: 'M8 builtin in comment plain', src: '// builtin container note', text: 'builtin', notScope: 'keyword' },
  // aligned-already guards: f-string holes re-enter source.rut; comments win.
  { name: 'f-string hole embedded', src: 'let t = f"{x}";', text: 'x', scope: 'meta.embedded.line.rut' },
  { name: 'comment rule wins', src: '// struct dataclass where string opaque ?', text: 'struct', scope: 'comment.line.double-slash.rut' },
];

function checkSnippets(grammar, bad) {
  const isWordChar = (c) => /[A-Za-z0-9_]/.test(c);
  const occurrences = (src, word) => {
    const at = [];
    const wordStart = isWordChar(word[0]);
    const wordEnd = isWordChar(word[word.length - 1]);
    for (let i = src.indexOf(word); i !== -1; i = src.indexOf(word, i + word.length)) {
      const before = i > 0 ? src[i - 1] : '';
      const after = i + word.length < src.length ? src[i + word.length] : '';
      // word-boundary guard only where the searched text has word chars
      // ('str' must not hit inside 'struct'; '?'/'. '..' need no guard)
      if (wordStart && isWordChar(before)) continue;
      if (wordEnd && isWordChar(after)) continue;
      at.push(i);
    }
    return at;
  };
  for (const s of SNIPPETS) {
    const toks = tokenizeDoc(grammar, `<${s.name}>`, s.src);
    const at = occurrences(s.src, s.text);
    if (!at.length) {
      bad.push(`smoke '${s.name}': '${s.text}' not found whole-word in ${JSON.stringify(s.src)}`);
      continue;
    }
    for (const pos of at) {
      const end = pos + s.text.length;
      // TextMate emits tokens only where a rule matched; unmatched runs are
      // gaps — i.e. plain text with no scope of their own. So: a positive
      // scope claim needs an intersecting token carrying it; a negative
      // claim fails only if some intersecting token carries the scope.
      const covering = toks.filter((t) => t.start < end && t.start + t.text.length > pos);
      if (s.scope && !covering.some((t) => hasScope(t, s.scope))) {
        const got = covering.map((t) => t.scopes.join(' ')).join(' | ') || '<gap>';
        bad.push(`smoke '${s.name}': '${s.text}' lacks ${s.scope} (covering: ${got})`);
      }
      if (s.notScope && covering.some((t) => hasScope(t, s.notScope))) {
        const got = covering.map((t) => t.scopes.join(' ')).join(' | ');
        bad.push(`smoke '${s.name}': '${s.text}' must not carry ${s.notScope} (covering: ${got})`);
      }
    }
  }
}

// ---- main ----------------------------------------------------------------
async function main() {
  const grammar = await makeGrammar(GRAMMAR_PATH);
  const bad = [];

  const files = [...rutFiles(path.join(REPO, 'rut'))];
  for (const root of CORPUS_ROOTS.slice(1)) files.push(...rutFiles(path.join(REPO, root)));
  files.sort();
  assert.ok(files.length >= MIN_CORPUS_FILES,
    `corpus walk found only ${files.length} .rut files (expected >= ${MIN_CORPUS_FILES})`);

  let tokens = 0;
  for (const file of files) {
    const rel = path.relative(REPO, file);
    const toks = tokenizeDoc(grammar, rel, fs.readFileSync(file, 'utf8'));
    tokens += toks.length;
    const lines = fs.readFileSync(file, 'utf8').split(/\r?\n/);
    for (const tok of toks) {
      checkDeadWords(tok, bad);
      checkCodeWord(tok, bad, 'struct', 'keyword.other.rut', 'M1 decl');
      checkCodeWord(tok, bad, 'str', 'storage.type.primitive.rut', 'M3 primitive');
      checkCodeWord(tok, bad, 'bytes', 'storage.type.primitive.rut', 'M3 primitive');
      checkCodeWord(tok, bad, 'opaque', 'storage.type.builtin.rut', 'M3 boot primitive');
      checkCodeWord(tok, bad, 'builtin', 'keyword.other.rut', 'M8 decl head');
    }
    // M4: every code-position `?` is the nullable prefix (RFC 0044 — prefix
    // only; corpus has no other `?` shape).
    for (const tok of toks) {
      if (tok.text.startsWith('?') && isCode(tok) && !hasScope(tok, 'storage.type.nullable.rut')) {
        bad.push(`${tok.file}:${tok.line} '${tok.text}' in code position is not nullable-scoped (scopes: ${tok.scopes.join(' ')})`);
      }
    }
    // M5 per-line introducer check
    for (let i = 0; i < lines.length; i++) {
      const lineToks = toks.filter((t) => t.line === i + 1);
      checkTypeAliasLine(lines[i], i, rel, lineToks, bad);
    }
  }

  checkSnippets(grammar, bad);

  if (bad.length) {
    console.error(`grammar-corpus: ${bad.length} violation(s):\n  ${bad.slice(0, 40).join('\n  ')}${bad.length > 40 ? `\n  … (+${bad.length - 40} more)` : ''}`);
    process.exit(1);
  }
  console.log(`grammar-corpus: PASS — ${files.length} corpus files, ${tokens} tokens, ${SNIPPETS.length} smoke assertions, 0 violations`);
}

main().catch((e) => {
  console.error('grammar-corpus: FAILED —', e);
  process.exit(1);
});
