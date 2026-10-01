#!/usr/bin/env node
/**
 * The JSONC cutover migration — one deterministic mechanical act over
 * every shipped module manifest:
 *
 *   1. `_comment` keys retire: each `"_comment": "<text>",` line is
 *      REWRITTEN IN PLACE as `//` comment line(s) — the decoded value
 *      text verbatim as prose, one `// ` line per source line, at the
 *      member's own indentation. A first-member `_comment` therefore
 *      becomes the file's leading comment block; a nested one becomes
 *      the comment directly above the key it documented.
 *   2. The wire bump: `"format_version": 7` → 9 (compiled) and
 *      `8` → 10 (decl) — the entry-NAME change (rut.json → rut.jsonc)
 *      is a layout change, so the version moves with it. Any other
 *      value rides untouched.
 *
 * The RENAME itself (rut.json → rut.jsonc) is done by git mv before
 * this script runs, so history keeps the rename edge.
 *
 * Determinism law: the file rewrite is a PURE function of its bytes —
 * run it twice, get identical bytes. `--golden` first asserts the
 * rewrite against embedded before/after pairs and exits.
 *
 * Usage:
 *   node scripts/migrate-manifest-jsonc.cjs --golden   test the rewrite
 *   node scripts/migrate-manifest-jsonc.cjs            migrate all rut.jsonc
 */
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const SKIP = new Set(["target", "node_modules", "dist", "docs", ".git", ".rut"]);

/** Every rut.jsonc in the repo's source trees (the rename has run). */
function findManifests(dir, out) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.isDirectory()) {
      if (SKIP.has(e.name)) continue;
      findManifests(path.join(dir, e.name), out);
    } else if (e.name === "rut.jsonc") {
      out.push(path.join(dir, e.name));
    }
  }
  return out.sort();
}

// ---------------------------------------------------------------------------
// the rewrite — a pure function of the text
// ---------------------------------------------------------------------------

/** Decode one JSON string token (the `_comment` value) to prose. */
function decodeComment(token) {
  try {
    const v = JSON.parse(token);
    if (typeof v !== "string") return null;
    return v;
  } catch {
    return null;
  }
}

/**
 * One line in, zero or more lines out. The `_comment` member line
 * becomes the decoded value's lines as `// ` comments at the same
 * indentation; everything else rides — except the wire bump.
 */
function rewriteLineInner(line) {
  const m = line.match(/^([ \t]*)"_comment"[ \t]*:[ \t]*("(?:[^"\\]|\\.)*")[ \t]*,?([ \t]*)(\r?)$/);
  if (m) {
    const [, indent, token, , cr] = m;
    const value = decodeComment(token);
    if (value === null) return [line]; // not a JSON string value: ride
    const prose = value.split("\n");
    return prose.map((l) => `${indent}//${l === "" ? "" : " " + l}${cr}`);
  }
  // the wire bump: 7 → 9 (compiled), 8 → 10 (decl); anything else
  // rides — the lookahead keeps trailing content (`… 8, "name": …`,
  // inline manifests included) and refuses a longer number (`17`, `80`)
  return [
    line
      .replace(/("format_version"[ \t]*:[ \t]*)7(?![0-9])/, "$19")
      .replace(/("format_version"[ \t]*:[ \t]*)8(?![0-9])/, "$110"),
  ];
}

/** The real entry point: whole-text rewrite preserving the terminator. */
function migrateText(text) {
  const sep = text.includes("\r\n") ? "\r\n" : "\n";
  const lines = text.split(/\r?\n/);
  // a trailing terminator yields a final "" element — split/join round-trips it
  const out = [];
  for (const line of lines) out.push(...rewriteLineInner(line));
  return out.join(sep);
}

// ---------------------------------------------------------------------------
// golden test — the rewrite pinned before it runs
// ---------------------------------------------------------------------------

const GOLDEN = [
  {
    name: "first-member comment becomes the leading comment block",
    before: `{
  "_comment": "pouch — the growable sequence package.",
  "name": "pouch",
  "entry": {
    "lib": "./pouch.rut"
  }
}
`,
    after: `{
  // pouch — the growable sequence package.
  "name": "pouch",
  "entry": {
    "lib": "./pouch.rut"
  }
}
`,
  },
  {
    name: "a multi-line value becomes one // per line, empties bare",
    before: `{
  "_comment": "the header.\\n\\nSecond paragraph, \\"quoted\\" and a\\\\slash.",
  "name": "x",
  "entry": { "lib": "./x.rut" }
}
`,
    after: `{
  // the header.
  //
  // Second paragraph, "quoted" and a\\slash.
  "name": "x",
  "entry": { "lib": "./x.rut" }
}
`,
  },
  {
    name: "a nested _comment documents the key below it",
    before: `{
  "name": "x",
  "deps": {
    "_comment": "pouch rides the CDN, pinned.",
    "pouch": { "path": "../pouch" }
  }
}
`,
    after: `{
  "name": "x",
  "deps": {
    // pouch rides the CDN, pinned.
    "pouch": { "path": "../pouch" }
  }
}
`,
  },
  {
    name: "the wire bump: 7 → 9 and 8 → 10, other values ride",
    before: `{
  "format": "rutbundle",
  "format_version": 7,
  "name": "pouch",
  "entry": { "lib": "./pouch.rut" }
}
`,
    after: `{
  "format": "rutbundle",
  "format_version": 9,
  "name": "pouch",
  "entry": { "lib": "./pouch.rut" }
}
`,
  },
  {
    name: "decl roots bump 8 → 10",
    before: `{ "format": "rutbundle", "format_version": 8, "name": "core", "type": "host" }\n`,
    after: `{ "format": "rutbundle", "format_version": 10, "name": "core", "type": "host" }\n`,
  },
  {
    name: "a stale version (4) rides untouched",
    before: `{ "format": "rutbundle", "format_version": 4, "name": "t" }\n`,
    after: `{ "format": "rutbundle", "format_version": 4, "name": "t" }\n`,
  },
  {
    name: "no _comment and no bump: byte identity",
    before: `{
  "name": "plain",
  "entry": { "lib": "./main.rut" }
}
`,
    after: `{
  "name": "plain",
  "entry": { "lib": "./main.rut" }
}
`,
  },
  {
    name: "CRLF file keeps its terminators",
    before: `{\r\n  "_comment": "windows line endings.",\r\n  "format_version": 7,\r\n  "name": "x"\r\n}\r\n`,
    after: `{\r\n  // windows line endings.\r\n  "format_version": 9,\r\n  "name": "x"\r\n}\r\n`,
  },
  {
    name: "the comment key may lack a trailing comma (last member)",
    before: `{
  "name": "x",
  "_comment": "the tail."
}
`,
    after: `{
  "name": "x",
  // the tail.
}
`,
  },
];

function runGolden() {
  let bad = 0;
  for (const g of GOLDEN) {
    const got = migrateText(g.before);
    if (got !== g.after) {
      bad++;
      console.error(`  FAIL: ${g.name}`);
      console.error("    want: " + JSON.stringify(g.after));
      console.error("    got:  " + JSON.stringify(got));
    } else {
      console.log(`  ok: ${g.name}`);
    }
  }
  // determinism: the rewrite of a rewritten file is itself
  for (const g of GOLDEN) {
    if (migrateText(g.after) !== g.after) {
      bad++;
      console.error(`  FAIL: not idempotent — ${g.name}`);
    }
  }
  if (bad) {
    console.error(`golden: ${bad} failing case(s)`);
    process.exit(1);
  }
  console.log("golden: all cases pass, the rewrite is idempotent");
}

// ---------------------------------------------------------------------------

if (process.argv.includes("--golden")) {
  runGolden();
  process.exit(0);
}

const files = findManifests(ROOT, []);
if (!files.length) {
  console.error("migrate: no rut.jsonc found — run the `git mv rut.json rut.jsonc` sweep first");
  process.exit(1);
}
let comments = 0;
let bumped = 0;
for (const f of files) {
  const before = fs.readFileSync(f, "utf8");
  const after = migrateText(before);
  comments += (before.match(/"_comment"/g) || []).length;
  bumped += (/^([ \t]*"format_version"[ \t]*:[ \t]*)(7|8)/m.test(before) ? 1 : 0);
  if (after !== before) fs.writeFileSync(f, after);
}
console.log(
  `migrate: ${files.length} manifest(s) rewritten — ` +
    `${comments} _comment key(s) → // comments, ${bumped} wire bump(s) (7→9 / 8→10)`
);
