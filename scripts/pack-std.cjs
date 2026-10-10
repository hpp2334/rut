#!/usr/bin/env node
/**
 * Pack the std tree (`rut/`) into the committed CDN artifacts
 * (`dist/std/*.rutbundle`) — the delivery law: jsDelivr's gh lane
 * serves REPO FILES at a ref, so the packed bundles are COMMITTED and
 * the publish act is a git tag (`std-vNN`, immutable in practice —
 * publish ADVANCES the number, never rewrites a tag).
 *
 * The bundle set is exhaustive over `rut/` (16 pkgs): lib pkgs pack as
 * compiled roots (their closures ride inside), host pkgs pack as decl
 * roots (single-package — the surface IS the root); `format_version`
 * is 10, always — the manifest's `type` routes the root kind. Same
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
 *                    RUT_STD_TAG, default std-v7). An ADVANCING
 *                    number: never re-point a published tag (jsDelivr
 *                    caches aggressively; a re-pointed tag lies).
 *   -h, --help
 *
 * No pushing, no network. The `rut fetch` / `rut run` human lane hits
 * jsDelivr once per bundle, then caches; every offline gate is green
 * because the committed artifact IS the seed.
 *
 * THE PIN BOUNDARY (the file-modules phase): dist/std is the WORKING
 * envelope — repacked whenever the pack shape moves (the mod.rut
 * rename did), byte-checked by --check's freshness half. The example
 * manifests' CDN rows are FROZEN at the std-v8 bytes (no tag
 * re-points, no std-v9): those bytes are committed at `dist/std-v8/` —
 * the offline gates' wire — and --check's pin half asserts every row
 * against THEM, not against dist/std. The reader's compat lane loads
 * the old envelope; that is a loading law, not a transition.
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
const PINNED = path.join(ROOT, "dist", "std-v8");
const DEFAULT_TAG = "std-v8";
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
const doCheck = takeFlag("--check");
const doTag = takeFlag("--tag");
const tag = takeValue("--tag-name") ?? process.env.RUT_STD_TAG ?? DEFAULT_TAG;
if (argv.length) die(`unknown arguments: ${argv.join(" ")}`);
if (takeFlag("--pins")) {
  die(
    "--pins is retired: the example manifests' std-v8 rows are FROZEN (no tag re-points, no std-v9) — " +
      "repack updates dist/std; the pins' bytes live at dist/std-v8"
  );
}
if ([doCheck, doTag].filter(Boolean).length > 1) {
  die("--check / --tag are mutually exclusive modes");
}

// ---------------------------------------------------------------------------
// the bundle set — exhaustive over rut/ (the completeness law: every pkg
// a [deps]/[dev-deps] row can spell ships from the CDN). `out` is the
// artifact name (the manifest name — what a deps row spells); dir is the
// tree directory when the two differ.
// ---------------------------------------------------------------------------

const PKGS = [
  // lib pkgs — compiled roots; each closure rides inside
  { dir: "pouch" },
  { dir: "flow" },
  { dir: "nmapset" },
  { dir: "strbuild" },
  { dir: "json" },
  { dir: "ink" },
  { dir: "http" },
  { dir: "futures" },
  // host pkgs — decl roots, single-package
  { dir: "ink_host" },
  { dir: "http_host" },
  { dir: "nmap_host" },
  { dir: "strbuild_host" },
  { dir: "async_host" },
  { dir: "core" },
  { dir: "calc" },
  { dir: "bench-cross" },
];

// The example manifests' url rows — the frozen pins (--check verifies
// them; nothing rewrites them). `deps` lists the dep KEYS whose rows
// are pinned; the PINNED snapshot (`dist/std-v8/`) must carry an
// artifact per distinct key — the bytes the rows' sha256 commits to.
//
// THE BOUNDARY (the file-modules phase): the rows are FROZEN. The
// pack shape moved (mod.rut), dist/std repacked — the pins did NOT
// move: they name the std-v8 bytes, which load through the reader's
// compat lane (a loading law, not a transition). --pins is gone: no
// tool rewrites these rows anymore.
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
  // its pin equals sha256 of the FROZEN std-v8 bytes (dist/std-v8/) —
  // the bytes the row commits to. dist/std (the working envelope) is
  // NOT the pins' reference: the rename moved the pack shape, the pins
  // did not move, and the reader's compat lane loads the old envelope.
  const pinHashes = {};
  for (const { manifest, deps } of EXAMPLES) {
    for (const key of deps) {
      if (pinHashes[key]) continue;
      const frozen = path.join(PINNED, `${key}.rutbundle`);
      if (!fs.existsSync(frozen)) {
        console.error(`  ${red("pin")}: dist/std-v8/${key}.rutbundle is missing — the frozen std-v8 bytes must be committed`);
        drift++;
        continue;
      }
      pinHashes[key] = sha256(fs.readFileSync(frozen));
    }
  }
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
      if (!pin || pin !== pinHashes[key]) {
        console.error(`  ${red("pin")}: ${manifest} \`${key}\` sha256 does not match dist/std-v8/${key}.rutbundle (the frozen std-v8 bytes)`);
        drift++;
      }
    }
  }

  if (drift) {
    console.error("");
    die(`${drift} drift row(s) — fix with: node scripts/pack-std.cjs`);
  }
  info(green(`all ${PKGS.length} artifacts byte-fresh, every example pin matches the frozen std-v8 bytes`));
  process.exit(0);
}

// default: pack the committed working-envelope artifacts (dist/std).
// The pins never move (see --pins above): dist/std is the freshness
// gate's half; dist/std-v8 is the pins' half.
packAll(DIST);
const hashes = {};
for (const { dir } of PKGS) {
  const name = pkgName(dir);
  hashes[name] = sha256(fs.readFileSync(path.join(DIST, `${name}.rutbundle`)));
  console.error(`  ${green("packed")}: dist/std/${name}.rutbundle (sha256 ${hashes[name].slice(0, 16)}…)`);
}
info(green(`packed ${PKGS.length} artifacts → dist/std/ (the std-v8 pins stay frozen; their bytes live at dist/std-v8/)`));
