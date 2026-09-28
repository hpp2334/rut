#!/usr/bin/env node
/**
 * bake-book.mjs — bake static highlight.js spans into the built book.
 *
 * `mdbook build docs` emits `<pre><code class="language-rut">` blocks
 * with NO spans: mdbook (0.5) highlights only in the browser — its
 * bundled highlight.js runs at page load, and `register.js` (wired as
 * `additional-js` in docs/book.toml) teaches it rut there. For static
 * HTML (no-JS readers, view-source, SEO) this script rewrites the rut
 * blocks with `hljs-*` spans after a build.
 *
 * Zero dependencies, one source of truth: it imports the SAME grammar
 * the runtime entry uses (`src/grammar.js`) and interprets the small
 * mode subset that grammar uses — `{ scope, begin, end?, keywords?,
 * contains? }` plus the `'self'` reference, with string-scoped modes
 * ending at end-of-line (rut strings are single-line). If grammar.js
 * grows a construct outside the subset, this engine says so rather
 * than mis-coloring.
 *
 *   mdbook build docs
 *   node scripts/bake-book.mjs [book-dir]   # default: ../../../docs/book
 *
 * Idempotent: blocks that already contain `hljs-` spans are skipped.
 */
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import rutLanguage from '../src/grammar.js';

const bookDir =
  process.argv[2] ?? new URL('../../../docs/book', import.meta.url).pathname;

const grammar = rutLanguage();

// Resolve `'self'` — the f-string hole recurses for nested braces.
// The hole ends up containing itself, so visit each mode once.
const seen = new Set();
(function linkSelf(mode) {
  if (!mode || seen.has(mode)) return;
  seen.add(mode);
  if (!Array.isArray(mode.contains)) return;
  mode.contains = mode.contains.map((c) => (c === 'self' ? mode : c));
  mode.contains.forEach(linkSelf);
})(grammar);

// word -> hljs class; first group in this order wins (`nil` is both a
// primitive type and a literal — it colors as literal, like hljs does).
const GROUPS = ['keyword', 'literal', 'type', 'built_in'];
const KW_MAP = new Map();
for (const g of GROUPS) {
  const v = grammar.keywords[g];
  if (!v) continue;
  for (const w of (typeof v === 'string' ? v : v.join(' ')).split(/\s+/)) {
    if (w && !KW_MAP.has(w)) KW_MAP.set(w, g);
  }
}
const KW_PATTERN = grammar.keywords.$pattern ?? /[A-Za-z_$][A-Za-z0-9_$]*/;

function sticky(re) {
  return new RegExp(re.source, re.flags.includes('y') ? re.flags : re.flags + 'y');
}

function matchAt(re, text, pos) {
  const r = sticky(re);
  r.lastIndex = pos;
  const m = r.exec(text);
  return m && m.index === pos ? m : null;
}

function esc(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Render text[start, limit) with `modes` + keyword scanning with `kw`
 * (null inside literals: no keyword coloring). A mode with `end`
 * recurses into its contains; string-scoped modes stop at end-of-line.
 * Returns [html, posAfter, hitEnd]. Longest match wins at a position
 * (hljs semantics — `{{` beats the `{` hole opener).
 */
function renderLevel(text, start, limit, modes, kw, stop) {
  let out = '';
  let pos = start;
  while (pos < limit) {
    if (stop) {
      if (stop.end) {
        const m = matchAt(stop.end, text, pos);
        if (m) return [out + esc(m[0]), pos + m[0].length, true];
      }
      if (stop.singleLine && text[pos] === '\n') return [out, pos, false];
    }
    let best = null;
    for (const mode of modes ?? []) {
      const m = matchAt(mode.begin, text, pos);
      if (m && (!best || m[0].length > best.m[0].length)) best = { mode, m };
    }
    if (best) {
      let body = esc(best.m[0]);
      pos += best.m[0].length;
      if (best.mode.end) {
        const [inner, after, ] = renderLevel(
          text,
          pos,
          limit,
          best.mode.contains,
          best.mode.keywords ?? null, // literals carry no keywords
          {
            end: best.mode.end,
            singleLine: (best.mode.scope ?? '').startsWith('string'),
          },
        );
        body += inner;
        pos = after;
      }
      out += best.mode.scope
        ? `<span class="hljs-${best.mode.scope}">${body}</span>`
        : body;
      continue;
    }
    if (kw) {
      const m = matchAt(KW_PATTERN, text, pos);
      if (m) {
        const cls = KW_MAP.get(m[0]);
        out += cls ? `<span class="hljs-${cls}">${esc(m[0])}</span>` : esc(m[0]);
        pos += m[0].length;
        continue;
      }
    }
    const ws = matchAt(/\s+/, text, pos);
    if (ws) {
      out += ws[0];
      pos += ws[0].length;
    } else {
      out += esc(text[pos]);
      pos += 1;
    }
  }
  return [out, pos, false];
}

function highlight(src) {
  return renderLevel(src, 0, src.length, grammar.contains, grammar.keywords, null)[0];
}

// ---- the book walker -------------------------------------------------

function unescapeHtml(s) {
  return s
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, '&');
}

const BLOCK = /<pre><code class="language-rut([^"]*)">([\s\S]*?)<\/code><\/pre>/g;

let pages = 0;
let blocks = 0;

function walk(dir) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      walk(p);
    } else if (e.name.endsWith('.html')) {
      pages += 1;
      const html = readFileSync(p, 'utf8');
      let touched = false;
      const next = html.replace(BLOCK, (whole, extra, body) => {
        if (body.includes('hljs-')) return whole; // already baked
        touched = true;
        blocks += 1;
        return `<pre><code class="language-rut${extra}">${highlight(unescapeHtml(body))}</code></pre>`;
      });
      if (touched) writeFileSync(p, next);
    }
  }
}

walk(bookDir);
console.log(`bake-book: ${blocks} rut block(s) across ${pages} page(s) under ${bookDir}`);
