#!/usr/bin/env node
/**
 * Deploy the demo playground (demo/dist/) to Cloudflare Pages.
 *
 * Target: playground.rut.hpp2334.com — a custom domain attached to the
 * Pages project (Dashboard → Workers & Pages → <project> → Custom
 * domains; the first deploy creates the project, the domain attachment
 * is a one-time dashboard step that drops a CNAME into the zone).
 *
 * Usage:
 *   node scripts/deploy.cjs [options]
 *
 * Options:
 *   --no-build        skip `npm run build` in demo/ (deploy the existing dist/)
 *   --project <name>  Pages project name        (env RUT_PAGES_PROJECT, default rut-playground)
 *   --branch <name>   branch to deploy as       (env RUT_PAGES_BRANCH,   default: current git branch, else main)
 *   --dist <dir>      directory to upload       (env RUT_PAGES_DIST,     default demo/dist)
 *   --dry-run         build, print the deploy command, upload nothing
 *   -h, --help
 *
 * The branch the Pages project treats as PRODUCTION (and thus what the
 * custom domain serves) defaults to `main`; override with
 * RUT_PAGES_PRODUCTION_BRANCH.
 *
 * Auth (either):
 *   CLOUDFLARE_API_TOKEN (+ CLOUDFLARE_ACCOUNT_ID when the token sees
 *   multiple accounts) — the non-interactive/CI path, or
 *   `npx wrangler login` once — the interactive OAuth path.
 *
 * Deploys from the Pages project's PRODUCTION branch (usually `main`)
 * go live on playground.rut.hpp2334.com; any other branch lands as a
 * preview deployment at <hash>.<project>.pages.dev.
 */
"use strict";

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const DEMO = path.join(ROOT, "demo");

// ---------------------------------------------------------------------------
// output helpers (the repo's loud-fail culture: say what runs, die loudly)
// ---------------------------------------------------------------------------

const color = process.stdout.isTTY && !process.env.NO_COLOR;
const paint = (code, s) => (color ? `\x1b[${code}m${s}\x1b[0m` : s);
const bold = (s) => paint(1, s);
const dim = (s) => paint(2, s);
const green = (s) => paint(32, s);
const red = (s) => paint(31, s);

const info = (s) => console.log(`${bold("deploy:")} ${s}`);
const ok = (s) => info(green(s));
const die = (s) => {
  info(red(`FAIL — ${s}`));
  process.exit(1);
};

const usage = () => {
  console.log(`usage: node scripts/deploy.cjs [--no-build] [--project <name>] [--branch <name>] [--dist <dir>] [--dry-run]`);
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

const doBuild = !argv.includes("--no-build");
const dryRun = argv.includes("--dry-run");
const project = takeValue("--project") ?? process.env.RUT_PAGES_PROJECT ?? "rut-playground";
const dist = path.resolve(ROOT, takeValue("--dist") ?? process.env.RUT_PAGES_DIST ?? "demo/dist");
const branch = takeValue("--branch") ?? process.env.RUT_PAGES_BRANCH ?? gitBranch() ?? "main";
// which branch the Pages project serves the custom domain from
// (this repo deploys from `master`; the project was created with
// --production-branch=master — override here if that ever changes)
const productionBranch = process.env.RUT_PAGES_PRODUCTION_BRANCH ?? "master";
const isProduction = branch === productionBranch;

function gitBranch() {
  const r = spawnSync("git", ["rev-parse", "--abbrev-ref", "HEAD"], { cwd: ROOT, encoding: "utf8" });
  const name = r.status === 0 ? r.stdout.trim() : "";
  return name && name !== "HEAD" ? name : null;
}

const unknown = argv.filter(
  (a) => a.startsWith("-") && a !== "--no-build" && a !== "--dry-run"
);
if (unknown.length) {
  usage();
  die(`unknown option(s): ${unknown.join(" ")}`);
}

// ---------------------------------------------------------------------------
// 1. build the demo (preflight-wasm runs inside `npm run build` and fails
//    loudly with the exact build:wasm command when artifacts are missing)
// ---------------------------------------------------------------------------

if (doBuild) {
  info(dim("building demo/ — npm run build"));
  if (!dryRun) {
    const r = spawnSync("npm", ["run", "build"], { cwd: DEMO, stdio: "inherit", shell: process.platform === "win32" });
    if (r.status !== 0) die(`demo build failed (exit ${r.status ?? "?"})`);
  }
} else {
  info(dim("skipping build (--no-build)"));
}

// ---------------------------------------------------------------------------
// 2. sanity-check what we are about to ship
// ---------------------------------------------------------------------------

const need = ["index.html", "main.js", "rut.wasm", "rut-lsp.wasm"];
const missing = need.filter((f) => !fs.existsSync(path.join(dist, f)));
if (missing.length) {
  die(
    `${dist} is missing ${missing.join(", ")} — run ` +
      `\`cd demo && npm run build\` (wasm artifacts come from \`npm run build:wasm\`)`
  );
}
ok(`dist ready: ${dist} (${isProduction ? bold("PRODUCTION") : `preview ${bold(branch)}`})`);

// ---------------------------------------------------------------------------
// 3. wrangler pages deploy
// ---------------------------------------------------------------------------

// --commit-dirty=true: deploying a dirty worktree is normal here; without
// the flag wrangler prompts interactively and hangs a CI run.
const args = [
  "--yes",
  "wrangler",
  "pages",
  "deploy",
  dist,
  "--project-name",
  project,
  "--branch",
  branch,
  "--commit-dirty=true",
];

info(dim(`npx ${args.join(" ")}${dryRun ? "  (dry-run: skipped)" : ""}`));

if (dryRun) {
  ok("dry-run complete — nothing uploaded");
  process.exit(0);
}

const r = spawnSync("npx", args, {
  cwd: ROOT,
  stdio: "inherit",
  shell: process.platform === "win32",
  env: process.env, // CLOUDFLARE_API_TOKEN / CLOUDFLARE_ACCOUNT_ID pass through
});

if (r.error?.code === "ENOENT") die("npx not found — install Node.js (https://nodejs.org)");
if (r.status !== 0) die(`wrangler deploy failed (exit ${r.status ?? "?"})`);

// ---------------------------------------------------------------------------
// 4. what just shipped, and where to look
// ---------------------------------------------------------------------------

ok(`deployed demo/dist → Cloudflare Pages project ${bold(project)} (branch ${bold(branch)})`);
console.log(`
  ${bold(isProduction ? "production" : "preview")} deployment — see the URL wrangler printed above.

  Custom domain: ${bold("playground.rut.hpp2334.com")} serves the PRODUCTION
  branch (${bold(productionBranch)}) of this project. If the domain is not
  attached yet (one-time):
    Dashboard → Workers & Pages → ${project} → Custom domains → Set up a
    custom domain → playground.rut.hpp2334.com (Cloudflare adds the CNAME).

  Re-deploy without rebuilding:  node scripts/deploy.cjs --no-build
`);
