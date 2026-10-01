/*! jsonc-highlight.js — hand-written committed shim (NOT generated).
 *
 * mdbook's bundled highlight.js is 10.1.1 and does NOT ship a `jsonc`
 * grammar (its `json` grammar carries no `jsonc` alias either), so every
 * ` ```jsonc ` block — the rut.jsonc manifest examples — renders as
 * plain uncolored text. This script registers a self-contained
 * pure-data `jsonc` grammar on the global `hljs` and re-highlights the
 * matching blocks after DOMContentLoaded.
 *
 * Like theme/rut-highlight.js, it defines its grammar with a `mode()`
 * helper that sets both `scope` (highlight.js >=10.3/11) and
 * `className` (what mdbook's 10.1.1 reads) to the same name — one
 * grammar, correct on every engine.
 *
 * JSONC specifics vs the bundled `json`: line and block comments are
 * legal anywhere (comments come FIRST in `contains`, so a `//` that
 * is inside a string never wins — string mode is entered only on `"`),
 * and trailing commas are legal (plain text, no highlighting needed).
 * Token classes are hljs-standard (`comment`/`attr`/`string`/`number`/
 * `literal`), so the existing ayu (default) and light themes color them
 * with NO additional-css changes.
 *
 * Without a global `hljs` it fails LOUDLY: exposes
 * `window.jsoncHljsGrammar` and warns on the console.
 */
(function (global) {
'use strict';

// `scope` is what highlight.js >=10.3/11 reads; `className` is the same
// name for highlight.js <=10.2 — notably mdbook's bundled 10.1.1, which
// ignores `scope` entirely. Setting both keeps one grammar correct on
// every engine; unknown keys are ignored.
function mode(scope, spec) {
  return { scope, className: scope, ...spec };
}

// JSON string escapes (RFC 8259): `\" \\ \/ \b \f \n \r \t \uXXXX`.
const ESCAPE = mode('', { begin: /\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4})/ });

// Object keys: a string followed by `:` — matched in one go with a
// lookahead so the same `"` never re-enters the string mode.
const KEY = mode('attr', { begin: /"(?:\\.|[^\\"\n])*"(?=\s*:)/ });

const STRING = mode('string', {
  begin: /"/,
  end: /"/,
  illegal: /\n/, // JSON strings are single-line
  contains: [ESCAPE],
});

const NUMBER = mode('number', {
  begin: /-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/,
});

/** highlight.js language definition for jsonc (pure data — no hljs helpers). */
function jsoncLanguage() {
  return {
    name: 'jsonc',
    // Comments first: at any position hljs tries modes in order, so a
    // `//` that starts outside a string wins before anything else — and
    // a `//` inside a string is safe regardless, because string mode is
    // entered only on `"` and consumes through the closing quote.
    contains: [
      mode('comment', { begin: /\/\//, end: /(?=\n)/ }),
      mode('comment', { begin: /\/\*/, end: /\*\// }),
      KEY,
      STRING,
      NUMBER,
    ],
    // Values only: strings/keys are consumed by their modes, so the
    // literal keywords never fire inside them.
    keywords: { $pattern: /\w+/, literal: 'true false null' },
  };
}


function boot(hljs) {
  hljs.registerLanguage('jsonc', jsoncLanguage);
  // hljs 10.1+ has highlightElement; hljs <= 10.0 and mdbook's bundle
  // call it highlightBlock.
  var highlight = hljs.highlightElement || hljs.highlightBlock;
  var blocks = document.querySelectorAll('pre code.language-jsonc');
  Array.prototype.forEach.call(blocks, function (el) {
    // mdbook's book.js adds the `hljs` class to EVERY code block — even
    // ones it could not highlight — so neither `:not(.hljs)` nor the
    // class alone can mean "already done". Skip only blocks our own pass
    // already handled, or that verifiably carry jsonc spans.
    if (el.getAttribute('data-jsonc-hljs')) return;
    if (el.querySelector('.hljs-attr, .hljs-string, .hljs-comment, .hljs-number, .hljs-literal')) return;
    highlight.call(hljs, el);
    el.setAttribute('data-jsonc-hljs', '1');
  });
}

function start() {
  if (!global.hljs) {
    // No silent no-op: keep the grammar available and say what is missing.
    global.jsoncHljsGrammar = jsoncLanguage;
    console.warn(
      "[jsonc-highlight] no global 'hljs' found — loaded the jsonc grammar " +
      'as window.jsoncHljsGrammar instead of registering it. Load highlight.js ' +
      'BEFORE this script, or register manually: ' +
      "hljs.registerLanguage('jsonc', window.jsoncHljsGrammar)."
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
