#!/usr/bin/env node
/**
 * build.mjs — regenerate `register.js` from `src/grammar.js`.
 *
 * `src/grammar.js` is pure ESM (bundler/`import` users); a plain
 * `<script>` tag cannot `import` it, so the script-tag entry is
 * generated: this script splices the single `export default` function
 * (kept import-free on purpose) into a small harness IIFE. Node only,
 * zero dependencies, string splicing — no transpile, no bundler.
 *
 *   cd integrations/rut-highlightjs
 *   node scripts/build.mjs
 *
 * Commit the regenerated `register.js` (dist-style: generated but
 * committed, like the vscode extension commits its packaged artifact).
 */
import { readFileSync, writeFileSync } from 'node:fs';

const MARK = 'export default ';
const grammarPath = new URL('../src/grammar.js', import.meta.url);
const outPath = new URL('../register.js', import.meta.url);

const src = readFileSync(grammarPath, 'utf8');
const at = src.indexOf(MARK);
if (at < 0 || src.indexOf(MARK, at + 1) >= 0) {
  throw new Error('src/grammar.js: expected exactly one `export default` function');
}
// The whole file body with the ESM marker stripped — the helper consts
// (KEYWORDS, modes, …) ride along with the definition function, keeping
// ONE source of truth.
const definition = src.replace(MARK, '').trimStart();

const header = `/*! rut-highlightjs v0.1.0 — register.js
 * GENERATED from src/grammar.js by scripts/build.mjs — do not edit by hand.
 * Regen: cd integrations/rut-highlightjs && node scripts/build.mjs
 *
 * Plain <script> entry for the rut grammar: registers it on a global
 * \`hljs\` (highlight.js) and highlights every \`pre code.language-rut\`
 * block. Without a global \`hljs\` it fails LOUDLY: exposes
 * \`window.rutHljsGrammar\` and warns on the console.
 */
(function (global) {
'use strict';

`;

const harness = `

function boot(hljs) {
  hljs.registerLanguage('rut', rutLanguage);
  // hljs 10.1+ has highlightElement; hljs <= 10.0 and mdbook's bundle
  // call it highlightBlock.
  var highlight = hljs.highlightElement || hljs.highlightBlock;
  var blocks = document.querySelectorAll('pre code.language-rut');
  Array.prototype.forEach.call(blocks, function (el) {
    // mdbook's book.js adds the \`hljs\` class to EVERY code block — even
    // ones it could not highlight — so neither \`:not(.hljs)\` nor the
    // class alone can mean "already done". Skip only blocks our own pass
    // already handled, or that verifiably carry rut spans.
    if (el.getAttribute('data-rut-hljs')) return;
    if (el.querySelector('.hljs-keyword, .hljs-literal, .hljs-type, .hljs-string, .hljs-comment, .hljs-number, .hljs-subst')) return;
    highlight.call(hljs, el);
    el.setAttribute('data-rut-hljs', '1');
  });
}

function start() {
  if (!global.hljs) {
    // No silent no-op: keep the grammar available and say what is missing.
    global.rutHljsGrammar = rutLanguage;
    console.warn(
      "[rut-highlightjs] no global 'hljs' found — loaded the rut grammar " +
      'as window.rutHljsGrammar instead of registering it. Load highlight.js ' +
      'BEFORE this script, or register manually: ' +
      "hljs.registerLanguage('rut', window.rutHljsGrammar)."
    );
    return;
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', function () { boot(global.hljs); });
  } else {
    boot(global.hljs);
  }
}

start();
})(typeof window !== 'undefined' ? window : this);
`;

writeFileSync(outPath, header + definition + harness);
console.log('build.mjs: wrote register.js from src/grammar.js');
