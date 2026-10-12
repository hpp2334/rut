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
    'enum class struct interface impl requires use pub ' +
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

// Use lines — `use ink::{ Logger };` — mirror the classifier's law
// (`crates/rut-semantic/src/names.rs`, ItemKind::Use): the path
// segments read as their heads (lowercase = a module, capitalized = a
// type), the imported names as the leaf members (capitalized = a type,
// lowercase = a function). Scope mapping, on the grammar's
// single-segment convention (title/property above): capitalized →
// `type`, imported lowercase → `title` (the callable slot, like
// MEMBER_CALL), lowercase segments → `meta` — the palette's qualifier
// slot. The natural hljs scope for a module would be `namespace`, but
// the book's two stylesheets (mdbook's bundled highlight.css and
// theme/rut-book.css) only carry single-segment scopes and neither
// knows `.hljs-namespace`; `meta` is styled by both.
//
// No container mode: one beginning at `use` would swallow the keyword
// into its begin match (plain in the baked book — the book baker emits
// begin text verbatim). Instead each ident carries a lookbehind keying
// it to its use-line slot, so `use` itself stays with the keyword
// machinery and every ident is one whole span. `self`/`super`/`Self`
// are excluded — keyword words keep their keyword color, matching the
// classifier's `owned_by_tokens` carve-out. Text outside use lines is
// untouched: the lookbehind needs a `\buse\s` with no `;` before the
// ident (`;` closes every use item), braces split segments from
// imports, and comments/strings consume their own text first.

// path segments — between `use` and the `{` of an import list. The
// `(?<![A-Za-z0-9_$])` keeps a match from starting mid-word (without
// it the lazy lookbehind gap can end inside `super`, leaving `s` plain
// and `uper` classed); the lookahead hands keyword words to the
// keyword machinery instead.
const USE_SEGMENT = mode('meta', {
  begin: /(?<=\buse\s[^;{}]*?)(?<![A-Za-z0-9_$])(?!(?:self|super)\b)[a-z_$][A-Za-z0-9_$]*/,
});
const USE_SEGMENT_TYPE = mode('type', {
  begin: /(?<=\buse\s[^;{}]*?)(?<![A-Za-z0-9_$])(?!Self\b)[A-Z][A-Za-z0-9_$]*/,
});

// imported names — between the `{` of an import list and its `}`
const USE_IMPORT_TYPE = mode('type', {
  begin: /(?<=\buse\s[^;]*\{[^}]*?)(?<![A-Za-z0-9_$])(?!Self\b)[A-Z][A-Za-z0-9_$]*/,
});
const USE_IMPORT_FUNCTION = mode('title', {
  begin: /(?<=\buse\s[^;]*\{[^}]*?)(?<![A-Za-z0-9_$])(?!(?:self|super)\b)[a-z_$][A-Za-z0-9_$]*/,
});

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
export default function rutLanguage() {
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
      USE_SEGMENT,
      USE_SEGMENT_TYPE,
      USE_IMPORT_TYPE,
      USE_IMPORT_FUNCTION,
    ],
  };
}
