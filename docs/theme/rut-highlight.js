/*! rut-highlightjs v0.1.0 — register.js
 * GENERATED from src/grammar.js by scripts/build.mjs — do not edit by hand.
 * Regen: cd integrations/rut-highlightjs && node scripts/build.mjs
 *
 * Plain <script> entry for the rut grammar: registers it on a global
 * `hljs` (highlight.js) and highlights every `pre code.language-rut`
 * block. Without a global `hljs` it fails LOUDLY: exposes
 * `window.rutHljsGrammar` and warns on the console.
 */
(function (global) {
'use strict';

/**
 * highlight.js language definition for rut — pure data, zero dependencies.
 *
 * Derived from the compiler's own tables (the code is the truth):
 *   - keywords:  `crates/rut-parser/src/lib.rs` `RESERVED_KW`, plus the
 *     contextual words the parser matches by interner text: `entry`,
 *     `host`-lane siblings `builtin`/`primitive` (item.rs), `break`/
 *     `continue` (stmt.rs), `self`/`Self` (expr.rs, item.rs), `super`
 *     (frame.rs), `as` (casts + select arm binds), `type` (the alias
 *     introducer).
 *   - primitive types: `is_primitive_ty` in the same file; `opaque` is
 *     added on top — a real contextual builtin type (the erasure box,
 *     declared `builtin primitive opaque`) that the table doesn't list.
 *   - literals: `crates/rut-lexer/src/lexer.rs` — `"…"` with `\t \n \r
 *     \b \f \\ \" \' \0 \u{…}` escapes, `r"…"` raw (no escapes),
 *     `f"…"` format strings with `{hole}` expressions (`{{`/`}}` are
 *     literal braces, string literals inside a hole are an error),
 *     numbers with `_` separators, `0x`/`0b`/`0o` radices and
 *     `i8..i64`/`u8..u64`/`f32`/`f64` suffixes.
 * If a word is absent from those tables it does not belong here — see
 * README.md for the full derivation notes and cross-check results.
 *
 * The default export is the definition function highlight.js expects:
 *
 *   import rut from 'rut-highlightjs';
 *   hljs.registerLanguage('rut', rut);
 *
 * scripts/build.mjs splices this single `export default` function into
 * the plain-JS `register.js` script-tag entry — keep this file
 * import-free and single-export so that splice stays trivial.
 */

const KEYWORDS = {
  // `$` is an ordinary identifier character in rut (`on_click$`,
  // `$temp`) — the keyword pattern accepts it, like the lexer does.
  $pattern: /[A-Za-z_$][A-Za-z0-9_$]*/,

  keyword:
    // RESERVED_KW — the grammar's one canonical reserved set
    'fn let mut if else while for of return when ' +
    'enum class struct trait impl requires use pub ' +
    'static async await extern is host select ' +
    // contextual words — ordinary identifiers elsewhere, keywords in
    // their slots
    'entry builtin primitive break continue self Self super as type',

  literal: 'true false nil',

  // is_primitive_ty + `opaque`
  type: 'bool str bytes f32 f64 i8 i16 i32 i64 u8 u16 u32 u64 opaque',
};

// `scope` is what highlight.js ≥10.3/11 reads; `className` is the same
// name for highlight.js ≤10.2 — notably mdbook's bundled 10.1.1, which
// ignores `scope` entirely. Setting both keeps one grammar correct on
// every engine; unknown keys are ignored.
function mode(scope, spec) {
  return { scope, className: scope, ...spec };
}

// `\u{1F600}` plus the single-char escapes the lexer accepts
const ESCAPE = mode('', { begin: /\\(?:[tnrbf\\"'0]|u\{[0-9a-fA-F]{1,6}\})/ });

// Member access: `.name(` is a call, any other `.name` a field. The dot
// rides along in the span (compact-grammar convention).
const MEMBER_CALL = mode('title', { begin: /\.[A-Za-z_$][A-Za-z0-9_$]*(?=\s*\()/ });
const MEMBER_FIELD = mode('property', { begin: /\.[A-Za-z_$][A-Za-z0-9_$]*/ });

// Suffix included in the match, so `10u8`, `1.5f32`, `0xFFu64` color as
// one number the way the lexer tokenizes them.
const NUMBER = mode('number', {
  begin: /\b(?:0[xX][0-9a-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|\d[\d_]*(?:\.[\d_]+)?(?:[eE][+-]?\d+)?)(?:[iu](?:8|16|32|64)|f32|f64)?\b/,
});

// `f"…{expr}…"` — the hole is highlighted as `subst`; it recurses for
// nested braces and carries the keywords so hole expressions color.
const FSTRING = mode('string', {
  begin: /f"/,
  end: /"/,
  illegal: /\n/, // rut strings are single-line
  contains: [
    // `{{`/`}}` are literal braces; the same escape set as plain
    // strings — including `\u{…}`, so the `{…}` of an escape is not
    // mistaken for a hole
    mode('', { begin: /\{\{|\}\}|\\(?:[tnrbf\\"'0]|u\{[0-9a-fA-F]{1,6}\})/ }),
    mode('subst', {
      begin: /\{/,
      end: /\}/,
      keywords: KEYWORDS,
      contains: ['self', NUMBER, MEMBER_CALL, MEMBER_FIELD],
    }),
  ],
});

// `r"…"` — the lexer closes at the first `"` (see README: code wins)
const RAW_STRING = mode('string', {
  begin: /r"/,
  end: /"/,
  illegal: /\n/,
});

const STRING = mode('string', {
  begin: /"/,
  end: /"/,
  illegal: /\n/,
  contains: [ESCAPE],
});

/** highlight.js language definition (pure data — no hljs helpers). */
function rutLanguage() {
  return {
    name: 'rut',
    case_insensitive: false,
    keywords: KEYWORDS,
    contains: [
      // `//` line (and `///` doc) comments; `/* … */` — rut block
      // comments do NOT nest
      mode('comment', { begin: /\/\//, end: /(?=\n)/ }),
      mode('comment', { begin: /\/\*/, end: /\*\// }),
      FSTRING,
      RAW_STRING,
      STRING,
      NUMBER,
      MEMBER_CALL,
      MEMBER_FIELD,
    ],
  };
}


function boot(hljs) {
  hljs.registerLanguage('rut', rutLanguage);
  // hljs 10.1+ has highlightElement; hljs <= 10.0 and mdbook's bundle
  // call it highlightBlock.
  var highlight = hljs.highlightElement || hljs.highlightBlock;
  var blocks = document.querySelectorAll('pre code.language-rut');
  Array.prototype.forEach.call(blocks, function (el) {
    // mdbook's book.js adds the `hljs` class to EVERY code block — even
    // ones it could not highlight — so neither `:not(.hljs)` nor the
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
