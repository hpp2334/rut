#!/usr/bin/env node
/**
 * Pack the std tree (`rut/`) into the committed CDN artifacts
 * (`dist/std/*.rutbundle`) — the delivery law: jsDelivr's gh lane
 * serves REPO FILES at a ref, so the packed bundles are COMMITTED and
 * the publish act is a git tag (`std-vNN`, immutable in practice —
 * publish ADVANCES the number, never rewrites a tag).
 *
 * The bundle set is exhaustive over `rut/` (16 pkgs): lib pkgs pack as
 * compiled v9 roots (their closures ride inside), host pkgs pack as
 * decl v10 roots (single-package — the surface IS the root). Same
 * input directory ⇒ byte-identical bundle (Q4), so "pins fresh" is a
 * PURE EQUALITY gate — CI never touches the network.
 *
 * Usage:
 *   node scripts/pack-std.cjs              pack all 16 → dist/std/
 *   node scripts/pack-std.cjs --pins       rewrite the example
 *                                          manifests' url/sha256 rows
 *                                          (the EXAMPLES map below is
 *                                          the single rewrite map)
 *   node scripts/pack-std.cjs --check      gate mode: repack to a temp
 *                                          dir, byte-compare against
 *                                          the committed artifacts, and
 *                                          assert every example pin
 *                                          row matches sha256(committed
 *                                          bytes) + the url shape.
 *                                          Exit 1 on drift.
 *   node scripts/pack-std.cjs --tag        PRINT the publish commands
 *                                          (git tag + push). Never runs
 *                                          them — publishing is a
 *                                          user-requested act.
 *
 * Options:
 *   --tag-name <t>   the jsDelivr tag the urls spell (env
 *                    RUT_STD_TAG, default std-v6). An ADVANCING
 *                    number: never re-point a published tag (jsDelivr
 *                    caches aggressively; a re-pointed tag lies).
 *   -h, --help
 *
 * No pushing, no network. The `rut fetch` / `rut run` human lane hits
 * jsDelivr once per bundle, then caches; every offline gate is green
 * because the committed artifact IS the seed.
 */
"use strict";

const { spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const RUT = path.join(ROOT, "rut");
const DIST = path.join(ROOT, "dist", "std");
const DEFAULT_TAG = "std-v6";
const URL_BASE = "https://cdn.jsdelivr.net/gh/hpp2334/rut";

// ---------------------------------------------------------------------------
// output helpers (deploy.cjs's loud-fail culture: say what runs, die loudly)
// ---------------------------------------------------------------------------

const color = process.stdout.isTTY && !process.env.NO_COLOR;
const paint = (code, s) => (color ? `\x1b[${code}m${s}\x1b[0m` : s);
const bold = (s) => paint(1, s);
const dim = (s) => paint(2, s);
const green = (s) => paint(32, s);
const red = (s) => paint(31, s);

const info = (s) => console.log(`${bold("pack-std:")} ${s}`);
const die = (s) => {
  info(red(`FAIL — ${s}`));
  process.exit(1);
};

const usage = () => {
  console.log(
    `usage: node scripts/pack-std.cjs [--pins] [--check] [--tag] [--tag-name <t>]`
  );
};

// ---------------------------------------------------------------------------
// args
// ---------------------------------------------------------------------------

const argv = process.argv.slice(2);
if (argv.includes("-h") || argv.includes("--help")) {
  usage();
  process.exit(0);
}
function takeValue(flag) {
  const i = argv.indexOf(flag);
  if (i === -1) return null;
  const v = argv[i + 1];
  if (!v || v.startsWith("--")) die(`${flag} needs a value`);
  argv.splice(i, 2);
  return v;
}
function takeFlag(flag) {
  const i = argv.indexOf(flag);
  if (i === -1) return false;
  argv.splice(i, 1);
  return true;
}
const doPins = takeFlag("--pins");
const doCheck = takeFlag("--check");
const doTag = takeFlag("--tag");
const tag = takeValue("--tag-name") ?? process.env.RUT_STD_TAG ?? DEFAULT_TAG;
if (argv.length) die(`unknown arguments: ${argv.join(" ")}`);
if ([doPins, doCheck, doTag].filter(Boolean).length > 1) {
  die("--pins / --check / --tag are mutually exclusive modes");
}

// ---------------------------------------------------------------------------
// the bundle set — exhaustive over rut/ (the completeness law: every pkg
// a [deps]/[dev-deps] row can spell ships from the CDN). `out` is the
// artifact name (the manifest name — what a deps row spells); dir is the
// tree directory when the two differ.
// ---------------------------------------------------------------------------

const PKGS = [
  // lib pkgs — compiled v7 roots; each closure rides inside
  { dir: "pouch" },
  { dir: "flow" },
  { dir: "nmapset" },
  { dir: "strbuild" },
  { dir: "json" },
  { dir: "ink" },
  { dir: "http" },
  { dir: "futures" },
  // host pkgs — decl v8 roots, single-package
  { dir: "ink_host" },
  { dir: "http_host" },
  { dir: "nmap_host" },
  { dir: "strbuild_host" },
  { dir: "async_host" },
  { dir: "core" },
  { dir: "calc" },
  { dir: "bench-cross" },
];

// The example manifests' url rows — the single rewrite map (--pins).
// `deps` lists the dep KEYS whose rows become url+sha256; keys a
// manifest declares but this list omits stay untouched (path rows).
//
// THE BOUNDARY (the generic-source riding law): a compiled bundle
// whose pkg has an OPEN generic surface rides the source that serves
// consumer-spelled shapes, so the url rows now cover the generic
// owners too (pouch, json — `Vec<Todo>` compiles from the bundle at
// the consumer's link). The examples flip the rows their embedders
// seed offline from dist/std (the seed IS the cache; gates never
// touch the network). See docs/src/reference/bundles.md (the std-CDN
// section).
const EXAMPLES = [
  {
    manifest: "examples/00-todolist/rut.jsonc",
    deps: ["pouch"],
  },
  {
    manifest: "examples/01-sort/rut.jsonc",
    deps: ["pouch"],
  },
  {
    manifest: "examples/02-digest/rut.jsonc",
    deps: ["pouch", "json", "nmapset"],
  },
  {
    manifest: "examples/03-plugin/plugin/rut.jsonc",
    deps: ["pouch"],
  },
  {
    manifest: "examples/05-todolist-web/rut/biz/rut.jsonc",
    deps: ["pouch", "nmapset"],
  },
  {
    manifest: "examples/05-todolist-web/rut/ui/rut.jsonc",
    deps: ["pouch", "nmapset"],
  },
  {
    manifest: "examples/05-todolist-web/tests/store_probe/rut.jsonc",
    deps: ["pouch"],
  },
  {
    manifest: "examples/05-todolist-web/tests/t1_harness/rut.jsonc",
    deps: ["pouch", "nmapset"],
  },
  {
    manifest: "examples/06-github-viewer-cli/rut.jsonc",
    deps: ["http"],
  },
];

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

const sha256 = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");

/** A pkg's manifest name (what a deps row spells — the artifact name). */
function pkgName(dir) {
  const text = fs.readFileSync(path.join(RUT, dir, "rut.jsonc"), "utf8");
  const m = text.match(/"name"\s*:\s*"([A-Za-z0-9_]+)"/);
  if (!m) die(`rut/${dir}/rut.jsonc has no \`name\` row`);
  return m[1];
}

/** Run a command, inheriting stdio (loud), dying on failure. */
function run(cmd, args) {
  info(dim(`${cmd} ${args.join(" ")}`));
  const r = spawnSync(cmd, args, { cwd: ROOT, stdio: "inherit" });
  if (r.status !== 0) die(`${cmd} ${args.join(" ")} exited ${r.status}`);
}

/** The url row for one dep key of one example manifest. */
function urlRow(key) {
  return `${URL_BASE}@${tag}/dist/std/${key}.rutbundle`;
}

/**
 * Rewrite one example manifest's `[deps]` rows: each key in `wants`
 * becomes `key = { url = "…", sha256 = "<hash>" }`, hash from the
 * COMMITTED artifact (the delivery law: pins match what jsDelivr
 * serves). Idempotent — running twice changes nothing.
 */
function pinExample(manifestRel, wants, hashes) {
  const p = path.join(ROOT, manifestRel);
  if (!fs.existsSync(p)) die(`${manifestRel} does not exist — run the phase that lands it first`);
  let text = fs.readFileSync(p, "utf8");
  for (const key of wants) {
    if (!hashes[key]) die(`no artifact for \`${key}\` — pack first`);
    // the row: `"key": { "url": "…", "sha256": "…" }` — path or url flavor,
    // whole-row replace (the key names the row start; the row ends at
    // the object's closing brace)
    const rowRe = new RegExp(`("${key}")\\s*:\\s*\\{[^}]*\\}`);
    if (!rowRe.test(text)) die(`${manifestRel}: no \`deps\` row for \`${key}\``);
    const url = urlRow(key);
    text = text.replace(rowRe, (_m, k) => `${k}: { "url": "${url}", "sha256": "${hashes[key]}" }`);
  }
  fs.writeFileSync(p, text);
  info(`pinned ${dim(manifestRel)} → ${wants.map((k) => `${k}@${tag}`).join(", ")}`);
}

// ---------------------------------------------------------------------------
// modes
// ---------------------------------------------------------------------------

if (doTag) {
  console.log(`
Publish the committed dist/std/ artifacts to jsDelivr (the gh lane
serves repo files at a ref — the artifacts are already committed; this
tag IS the publish). Tags are IMMUTABLE in practice: publish advances
std-vNN, never rewrites:

  git tag ${tag}
  git push origin ${tag}

Then verify one artifact resolves (expect 200):

  curl -fsSI ${urlRow("pouch")}
`);
  process.exit(0);
}

// every mode below needs the artifacts: pack (default, --pins) writes
// them; --check repacks to a temp dir and byte-compares.
function packAll(outDir) {
  fs.mkdirSync(outDir, { recursive: true });
  for (const { dir } of PKGS) {
    const name = pkgName(dir);
    run("cargo", [
      "run", "-q", "-p", "rut-cli", "--",
      "pack", path.join("rut", dir), "-o", path.relative(ROOT, path.join(outDir, `${name}.rutbundle`)),
    ]);
  }
}

if (doCheck) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "rut-pack-std-check-"));
  info(`repacking to ${dim(tmp)} for the byte comparison`);
  packAll(tmp);
  const hashes = {};
  let drift = 0;
  for (const { dir } of PKGS) {
    const name = pkgName(dir);
    const committed = path.join(DIST, `${name}.rutbundle`);
    const fresh = path.join(tmp, `${name}.rutbundle`);
    if (!fs.existsSync(committed)) {
      console.error(`  ${red("missing")}: dist/std/${name}.rutbundle (pack to create it)`);
      drift++;
      continue;
    }
    const a = fs.readFileSync(committed);
    const b = fs.readFileSync(fresh);
    hashes[name] = sha256(a);
    if (!a.equals(b)) {
      console.error(`  ${red("drift")}: dist/std/${name}.rutbundle (repack differs from the committed bytes)`);
      drift++;
    } else {
      console.error(`  ${green("ok")}: dist/std/${name}.rutbundle (${a.length} bytes, sha256 ${hashes[name].slice(0, 16)}…)`);
    }
  }
  fs.rmSync(tmp, { recursive: true, force: true });

  // the example pins: every url row ends /dist/std/<key>.rutbundle and
  // its pin equals sha256(committed bytes)
  for (const { manifest, deps } of EXAMPLES) {
    const p = path.join(ROOT, manifest);
    if (!fs.existsSync(p)) continue; // the phase that lands it owns the row
    const text = fs.readFileSync(p, "utf8");
    for (const key of deps) {
      const rowRe = new RegExp(`"${key}"\\s*:\\s*\\{[^}]*\\}`);
      const row = rowRe.test(text) && text.match(rowRe)[0];
      if (!row) {
        console.error(`  ${red("pin")}: ${manifest} has no \`${key}\` row`);
        drift++;
        continue;
      }
      const url = (row.match(/"url"\s*:\s*"([^"]+)"/) || [])[1];
      const pin = (row.match(/"sha256"\s*:\s*"([0-9a-f]{64})"/) || [])[1];
      const wantUrl = urlRow(key);
      if (url !== wantUrl) {
        console.error(`  ${red("pin")}: ${manifest} \`${key}\` url is \`${url}\`, want \`${wantUrl}\``);
        drift++;
      }
      if (!pin || pin !== hashes[key]) {
        console.error(`  ${red("pin")}: ${manifest} \`${key}\` sha256 does not match dist/std/${key}.rutbundle`);
        drift++;
      }
    }
  }

  if (drift) {
    console.error("");
    die(
      `${drift} drift row(s) — fix with: node scripts/pack-std.cjs && node scripts/pack-std.cjs --pins`
    );
  }
  info(green(`all ${PKGS.length} artifacts byte-fresh, every example pin matches`));
  process.exit(0);
}

// default + --pins: pack the committed artifacts, then (for --pins)
// rewrite the example rows from them
packAll(DIST);
const hashes = {};
for (const { dir } of PKGS) {
  const name = pkgName(dir);
  hashes[name] = sha256(fs.readFileSync(path.join(DIST, `${name}.rutbundle`)));
  console.error(`  ${green("packed")}: dist/std/${name}.rutbundle (sha256 ${hashes[name].slice(0, 16)}…)`);
}
if (doPins) {
  for (const { manifest, deps } of EXAMPLES) pinExample(manifest, deps, hashes);
  info(green(`example pins rewritten for ${tag} — commit dist/ + the manifests together`));
} else {
  info(green(`packed ${PKGS.length} artifacts → dist/std/ (pin the examples with --pins)`));
}
