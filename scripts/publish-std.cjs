#!/usr/bin/env node
/**
 * Publish the std release: gate the committed cut, then tag + push +
 * verify. The third act of the delivery law — pack-std.cjs CUTS, a
 * HUMAN COMMITS (the review point), this script PUBLISHES. No script
 * ever types `git commit`.
 *
 * THE WIRE: the std CDN lane is source zips only. jsDelivr's gh lane
 * serves committed repo files at a ref, so the tag names committed
 * bytes: the cut envelope dist/std-vNN/ (the lib zips) is git-tracked,
 * and crates/rut-cli/template/rut.lock pins those bytes. THE LOCK IS
 * THE CARRIER: the pkg set, the urls and the live tag are read from it
 * — no pkg table, no sha table, no tag constant here (that duplication
 * is exactly how the retired bin lane rotted).
 *
 * TAGS ARE ALWAYS DERIVED (dense, advancing): publish takes `git
 * ls-remote --tags origin`'s max std-v* + 1 — which also proves the
 * remote is reachable (the cut counts local tags; publish counts the
 * REMOTE's). Derivation counts TAGS, not dist dirs. Publish ADVANCES,
 * never rewrites: a tag that exists locally or on origin is refused,
 * and the new tag must be numerically past every remote std-v*. Tags
 * are annotated, matching the v10–v13 practice.
 *
 * Usage:
 *   node scripts/publish-std.cjs            the gates, then the act:
 *                                           `git tag -a` + push, verify
 *                                           the CDN serves the pinned
 *                                           bytes at the new tag, and
 *                                           prove `rut new → install →
 *                                           run` end-to-end.
 *   node scripts/publish-std.cjs --dry-run  the gates + the act,
 *                                           printed, nothing done —
 *                                           no tag, no push, no fetch.
 *   -h, --help
 *
 * No lane flags, no tag flags — any other argument dies (the zip lane
 * is the only lane; the tag is always the derived next one). The e2e
 * is a real publish's proof and always runs on one; --dry-run is the
 * gates-only lane.
 */
"use strict";

const { spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const TEMPLATE = path.join(ROOT, "crates", "rut-cli", "template");
const LOCK = path.join(TEMPLATE, "rut.lock");
const MANIFEST = path.join(TEMPLATE, "rut.jsonc");

// ---------------------------------------------------------------------------
// output helpers (deploy.cjs's loud-fail culture: say what runs, die loudly)
// ---------------------------------------------------------------------------

const color = process.stdout.isTTY && !process.env.NO_COLOR;
const paint = (code, s) => (color ? `\x1b[${code}m${s}\x1b[0m` : s);
const bold = (s) => paint(1, s);
const dim = (s) => paint(2, s);
const green = (s) => paint(32, s);
const red = (s) => paint(31, s);

const info = (s) => console.log(`${bold("publish-std:")} ${s}`);
const die = (s) => {
  info(red(`FAIL — ${s}`));
  process.exit(1);
};

const usage = () => {
  console.log(`usage: node scripts/publish-std.cjs [--dry-run]`);
};

// ---------------------------------------------------------------------------
// args — bare = publish, --dry-run = gates + print. No lane flags, no tag
// flags: anything else dies loudly with its pointer.
// ---------------------------------------------------------------------------

const argv = process.argv.slice(2);
if (argv.includes("-h") || argv.includes("--help")) {
  usage();
  process.exit(0);
}
const dryRun = argv.includes("--dry-run");
const unknown = argv.filter((f) => f !== "--dry-run");
if (unknown.length) {
  die(
    `unknown option(s): ${unknown.join(" ")} — no lane flags, no tag flags: ` +
      "bare = publish, --dry-run = gates + print; the tag is always the derived next one"
  );
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

const sha256 = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");

/** Synchronous sleep — this script is a spawnSync world, no async. */
const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

/** git output (captured), dying loudly on failure. */
function gitOut(args) {
  const r = spawnSync("git", args, { cwd: ROOT, encoding: "utf8" });
  if (r.status !== 0) die(`git ${args.join(" ")} failed — ${String(r.stderr || "").trim()}`);
  return r.stdout;
}

/** Run a command, inheriting stdio (loud), dying on failure. */
function run(cmd, args) {
  info(dim(`${cmd} ${args.join(" ")}`));
  const r = spawnSync(cmd, args, { cwd: ROOT, stdio: "inherit" });
  if (r.status !== 0) die(`${cmd} ${args.join(" ")} exited ${r.status}`);
}

/** JSONC the way this repo's scripts have always read it: full-line // off. */
const stripComments = (text) => text.replace(/^[ \t]*\/\/.*$/gm, "");

/** The lock's rows ({pkg, url, sha256}) — THE CARRIER. No pkg table here. */
function readLockRows() {
  let lock;
  try {
    lock = JSON.parse(stripComments(fs.readFileSync(LOCK, "utf8")));
  } catch (e) {
    die(`crates/rut-cli/template/rut.lock does not parse (${e.message})`);
  }
  const rows = (lock.deps || []).map((r) => ({ pkg: r.pkg, url: r.url, sha256: r.sha256 }));
  if (!rows.length) die("crates/rut-cli/template/rut.lock has no dep rows — nothing to publish");
  for (const r of rows) {
    if (!r.pkg || !r.url || !r.sha256) {
      die("crates/rut-cli/template/rut.lock has a row missing pkg/url/sha256");
    }
  }
  return rows;
}

/** A std zip url: <base>@<tag>/dist/std/<pkg>.zip. */
const URL_RE = /^(.+)@(std-v\d+)\/dist\/std\/([A-Za-z0-9_]+)\.zip$/;

/** Parse every row's url; die unless they all spell ONE base@tag. */
function urlSpace(rows) {
  let base = null;
  let tag = null;
  for (const row of rows) {
    const m = row.url.match(URL_RE);
    if (!m) die(`the lock's \`${row.pkg}\` url is off the std zip envelope — ${row.url}`);
    if (m[3] !== row.pkg) {
      die(`the lock's \`${row.pkg}\` url names \`${m[3]}.zip\` — pkg and url disagree`);
    }
    if (base === null) {
      base = m[1];
      tag = m[2];
    } else if (m[1] !== base || m[2] !== tag) {
      die(`the lock's urls do not all spell one tag — ${row.url} breaks the ${base}@${tag} story`);
    }
  }
  return { base, tag };
}

/** The template manifest's dep rows ({key: {url}}) — read-only. */
function manifestDeps() {
  let m;
  try {
    m = JSON.parse(stripComments(fs.readFileSync(MANIFEST, "utf8")));
  } catch (e) {
    die(`crates/rut-cli/template/rut.jsonc does not parse (${e.message})`);
  }
  return m.deps || {};
}

/** A green gate row (the house check-row shape). */
const gateRow = (name, detail) =>
  console.log(`  ${green("ok")}: ${name}${detail ? ` — ${dim(detail)}` : ""}`);

// ---------------------------------------------------------------------------
// the carrier: pkg set, pins and live tag all come from the lock
// ---------------------------------------------------------------------------

const rows = readLockRows();
const { tag: liveTag } = urlSpace(rows);

// ---------------------------------------------------------------------------
// the tag: ALWAYS derived — the max std-v* tag on ORIGIN, +1 (which also
// proves the remote is reachable). Counts TAGS, not dist dirs.
// ---------------------------------------------------------------------------

info(`the lock pins ${rows.length} source zips at ${bold(liveTag)} — deriving the publish tag from origin`);
const ls = spawnSync("git", ["ls-remote", "--tags", "origin"], { cwd: ROOT, encoding: "utf8" });
if (ls.status !== 0) {
  die(
    `git ls-remote --tags origin failed (exit ${ls.status ?? "?"}) — the publish act derives ` +
      `the next tag from the remote, so it must be reachable: ${String(ls.stderr || "").trim()}`
  );
}
const remoteTags = new Set();
let maxRemote = 0;
for (const line of ls.stdout.split("\n")) {
  const m = line.match(/refs\/tags\/std-v(\d+)(\^\{\})?$/); // ^{} peeled rows share the number; a suffix like std-v15-rc never matches
  if (m) {
    remoteTags.add(`std-v${m[1]}`);
    maxRemote = Math.max(maxRemote, Number(m[1]));
  }
}
if (!remoteTags.size) {
  die("origin carries no std-v* tags — expected the std lane's history (std-v1, std-v2, …)");
}
const next = maxRemote + 1;
const tag = `std-v${next}`;
info(`derived next tag ${bold(tag)} (origin's max std-v${maxRemote} +1)`);

// ---------------------------------------------------------------------------
// phase 0 — gate 0: the cut envelope. dist/<tag>/ must exist, hold the
// zips the lock keys, and be tracked + clean at HEAD (the tag names
// COMMITTED bytes — an uncommitted cut is not a cut). Any miss dies with
// the remedy, byte-for-byte (tests assert the wording).
// ---------------------------------------------------------------------------

info(bold("phase 0 — the cut envelope"));
const envelope = path.join("dist", tag);
const noEnvelope = () =>
  die(
    `no cut envelope at ${envelope}/ — pack it first, review, and commit:\n\n` +
      `  node scripts/pack-std.cjs\n\n` +
      `then re-run: node scripts/publish-std.cjs`
  );

if (!fs.existsSync(path.join(ROOT, envelope)) || !fs.statSync(path.join(ROOT, envelope)).isDirectory()) {
  noEnvelope();
}
for (const row of rows) {
  const zip = path.join(ROOT, envelope, `${row.pkg}.zip`);
  if (!fs.existsSync(zip) || !fs.statSync(zip).isFile()) noEnvelope();
}
const atHead = new Set(
  gitOut(["ls-tree", "-r", "--name-only", "HEAD", "--", envelope])
    .split("\n")
    .map((s) => s.trim())
);
for (const row of rows) {
  if (!atHead.has(`${envelope}/${row.pkg}.zip`)) noEnvelope();
}
if (gitOut(["status", "--porcelain", "--", envelope]).trim() !== "") noEnvelope();
gateRow(`${envelope}/ — ${rows.length} zips present, tracked and clean at HEAD`);

// ---------------------------------------------------------------------------
// phase 1 — offline gates (each dies loudly with its fix hint)
// ---------------------------------------------------------------------------

info(bold("phase 1 — offline gates"));

// 1. the working tree is clean — the act tags COMMITTED bytes.
const status = gitOut(["status", "--porcelain"]);
if (status.trim() !== "") {
  die(
    `the working tree is not clean — the publish act tags committed bytes, so commit or stash first:\n` +
      status
        .trim()
        .split("\n")
        .map((l) => `  ${l}`)
        .join("\n")
  );
}
gateRow("working tree clean");

// 2. HEAD is pushed — jsDelivr serves repo files at the ref, so an
// unpushed HEAD would tag bytes the CDN cannot serve.
const unpushed = gitOut(["log", "origin/master..HEAD"]);
if (unpushed.trim() !== "") {
  die(
    `HEAD is not pushed (origin/master..HEAD holds ${unpushed.trim().split("\n").length} commit(s)) — ` +
      "git push first: jsDelivr serves repo files at the ref, the pushed HEAD is the wire"
  );
}
gateRow("HEAD is pushed (origin/master..HEAD empty)");

// 3. the tag story is coherent: absent locally AND on origin, numerically
// past every remote std-v* (publish advances, never rewrites), and the
// lock — the carrier — already spells it (the cut re-pinned it).
if (spawnSync("git", ["rev-parse", "-q", "--verify", `refs/tags/${tag}`], { cwd: ROOT, encoding: "utf8" }).status === 0) {
  die(
    `tag ${tag} already exists locally — publish advances, never rewrites ` +
      `(git tag -d ${tag} only if it was a local mistake)`
  );
}
if (remoteTags.has(tag)) {
  die(`tag ${tag} already exists on origin — publish advances, never rewrites`);
}
if (!(next > maxRemote)) {
  die(`${tag} is not past origin's max std-v tag (std-v${maxRemote}) — publish advances, never rewrites`);
}
if (liveTag !== tag) {
  die(
    `the lock's urls spell @${liveTag}, not @${tag} — the carrier and the derived tag disagree ` +
      `(origin moved ahead of the cut): re-cut with node scripts/pack-std.cjs, review, commit`
  );
}
gateRow(`${tag} is fresh — absent locally + on origin, past origin's std-v${maxRemote}; the lock spells it`);

// 4. the pins are law: every committed zip's sha256 equals its lock row.
let pinDrift = 0;
const results = [];
for (const row of rows) {
  const bytes = fs.readFileSync(path.join(ROOT, envelope, `${row.pkg}.zip`));
  const sha = sha256(bytes);
  const pinOk = sha === row.sha256;
  results.push({ pkg: row.pkg, url: row.url, bytes: bytes.length, sha, pinOk, cdn: null });
  if (!pinOk) {
    console.log(
      `  ${red("pin")}: ${envelope}/${row.pkg}.zip hashes to ${sha}, the lock pins ${row.sha256}`
    );
    pinDrift++;
  }
}
if (pinDrift) {
  die(
    `${pinDrift} pin row(s) disagree with ${envelope}/ — re-cut (node scripts/pack-std.cjs), ` +
      "re-pin the carriers, review, commit"
  );
}
gateRow(`all ${rows.length} lock pins match the committed bytes`);

// 5. the two templates tell one story: same keys, same urls, both ways.
const deps = manifestDeps();
let templateDrift = 0;
for (const row of rows) {
  const dep = deps[row.pkg];
  if (!dep) {
    console.log(`  ${red("template")}: template/rut.jsonc has no dep \`${row.pkg}\` — the lock pins it`);
    templateDrift++;
  } else if (dep.url !== row.url) {
    console.log(
      `  ${red("template")}: template/rut.jsonc \`${row.pkg}\` url is \`${dep.url}\`, want the lock's \`${row.url}\``
    );
    templateDrift++;
  }
}
for (const key of Object.keys(deps)) {
  if (!rows.some((r) => r.pkg === key)) {
    console.log(`  ${red("template")}: template/rut.jsonc dep \`${key}\` has no lock row`);
    templateDrift++;
  }
}
if (templateDrift) {
  die(
    `${templateDrift} template/lock drift row(s) — re-pin with the cut (node scripts/pack-std.cjs), ` +
      "review, commit"
  );
}
gateRow(`template/rut.jsonc urls == lock urls (${rows.length} rows)`);

// 6. the law tests — the new lane holds the template/lock/snapshot story
// (PATH discovers cargo; the repo runs tests through nextest).
run("cargo", ["nextest", "run", "-p", "rut-cli", "--test", "new_lane"]);
gateRow("law tests green (cargo nextest run -p rut-cli --test new_lane)");

// ---------------------------------------------------------------------------
// phase 2 — the act: an annotated tag, then its push. --dry-run prints
// both and stops here (the gates ran; nothing is tagged or pushed).
// ---------------------------------------------------------------------------

const tagArgs = ["tag", "-a", tag, "-m", `std publish: ${tag} — ${rows.length} source zips`];
const pushArgs = ["push", "origin", tag];

if (dryRun) {
  info(bold("phase 2 — the act (dry-run: printed, not run)"));
  console.log(`  ${dim(`git ${tagArgs.join(" ")}  (dry-run: skipped)`)}`);
  console.log(`  ${dim(`git ${pushArgs.join(" ")}  (dry-run: skipped)`)}`);
  printSummary();
  printCommands();
  info(green("dry-run complete — the gates are green; run it bare to publish"));
  process.exit(0);
}

info(bold("phase 2 — the act"));
run("git", tagArgs);
run("git", pushArgs);

// ---------------------------------------------------------------------------
// phase 3 — post-verify (network; a real publish only): the CDN must
// serve the pinned bytes at the new tag (fresh tags can lag the edge —
// retry x5 with backoff), then the scaffold must run against the lane.
// ---------------------------------------------------------------------------

info(bold("phase 3 — post-verify (CDN + e2e)"));

/** GET bytes via curl (the repo's fetch lane); null on any failure. */
function curlGet(url) {
  const r = spawnSync("curl", ["-fsSL", "--max-time", "60", url], { encoding: "buffer" });
  if (r.error && r.error.code === "ENOENT") die("curl not found — install it and retry");
  return r.status === 0 ? r.stdout : null;
}

// 9. the CDN lane: each zip url at the NEW tag serves the pinned bytes.
for (const res of results) {
  const url = res.url;
  let bytes = null;
  for (let attempt = 1; attempt <= 5 && !bytes; attempt++) {
    if (attempt > 1) {
      const wait = 2 ** (attempt - 1) * 1000; // 2s, 4s, 8s, 16s between tries
      info(dim(`${url} — not up yet, retry ${attempt - 1}/5 in ${wait / 1000}s (fresh-tag propagation)`));
      sleep(wait);
    }
    bytes = curlGet(url);
  }
  if (!bytes) {
    console.log(`  ${red("cdn")}: ${url} — unreachable after 5 tries`);
    continue;
  }
  res.cdn = sha256(bytes) === res.sha;
  if (res.cdn) {
    console.log(`  ${green("ok")}: ${url} (${bytes.length} bytes, sha256 ${res.sha.slice(0, 16)}…)`);
  } else {
    console.log(`  ${red("cdn")}: ${url} serves bytes other than the pin`);
  }
}

// 10. the scaffold's proof: `rut new → install → run` prints the hello.
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "rut-publish-std-e2e-"));
let e2eFail = null;
try {
  const hello = path.join(tmp, "hello");
  const rut = (sub) => {
    const args = ["run", "-q", "-p", "rut-cli", "--", sub, hello];
    info(dim(`cargo ${args.join(" ")}`));
    return spawnSync("cargo", args, {
      cwd: ROOT,
      encoding: "utf8",
      env: { ...process.env, RUT_CACHE_DIR: path.join(tmp, "cache") },
    });
  };
  for (const sub of ["new", "install"]) {
    const r = rut(sub);
    if (r.status !== 0) {
      e2eFail = `\`rut ${sub}\` failed (exit ${r.status ?? "?"})\n${r.stdout}${r.stderr}`;
      break;
    }
  }
  if (!e2eFail) {
    const r = rut("run");
    if (r.status !== 0) {
      e2eFail = `\`rut run\` failed (exit ${r.status ?? "?"})\n${r.stdout}${r.stderr}`;
    } else if (!r.stdout.includes("hello, rut!")) {
      e2eFail = `\`rut run\` did not print \`hello, rut!\` — stdout:\n${r.stdout}${r.stderr}`;
    }
  }
} finally {
  fs.rmSync(tmp, { recursive: true, force: true });
}
if (e2eFail) die(`the e2e scaffold failed — ${e2eFail}`);
gateRow("e2e: rut new → install → run prints `hello, rut!` (temp dir cleaned)");

// ---------------------------------------------------------------------------
// exit: the summary table + the commands the act ran; non-zero on a miss
// ---------------------------------------------------------------------------

function printSummary() {
  console.log("");
  info(bold(`summary — ${rows.length} source zips at ${tag}`));
  const w = Math.max(...results.map((r) => r.pkg.length));
  for (const r of results) {
    const pin = r.pinOk ? green("ok") : red("FAIL");
    const cdn = dryRun ? dim("(dry-run)") : r.cdn === true ? green("ok") : red("FAIL");
    console.log(`  ${r.pkg.padEnd(w)}  ${String(r.bytes).padStart(6)} bytes  ${r.sha.slice(0, 16)}…  pin ${pin}  cdn ${cdn}`);
  }
}

function printCommands() {
  console.log("");
  info(`commands ${dryRun ? "(dry-run — none run)" : "run"}:`);
  console.log(`  git ${tagArgs.join(" ")}`);
  console.log(`  git ${pushArgs.join(" ")}`);
  console.log(`  cargo run -q -p rut-cli -- new|install|run <tmp>/hello`);
}

if (results.some((r) => r.cdn !== true)) {
  printSummary();
  printCommands();
  die(
    "the CDN did not serve the pinned bytes — the tag IS pushed, and a fresh tag can lag " +
      "jsDelivr's edge for a minute: re-check the urls above, and do NOT re-tag (publish " +
      "advances, never rewrites)"
  );
}
printSummary();
printCommands();
info(green(`published ${tag} — ${rows.length} source zips verified on the CDN, the scaffold runs against them`));
