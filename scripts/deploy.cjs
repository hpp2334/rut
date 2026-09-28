#!/usr/bin/env node
/**
 * Deploy to Cloudflare Pages. Two targets, one script:
 *
 *   demo (default)   demo/dist/      → playground.rut.hpp2334.com
 *   book (--book)    docs/book/      → rut.hpp2334.com
 *
 * Each target is a Pages project with its custom domain attached
 * (Dashboard → Workers & Pages → <project> → Custom domains; the first
 * deploy creates the project, the domain attachment is a one-time
 * dashboard step that drops a CNAME into the zone — the hpp2334.com
 * zone is on Cloudflare, so the apex name for the book flattens
 * automatically).
 *
 * Usage:
 *   node scripts/deploy.cjs [--book] [options]
 *
 * Options:
 *   --book            ship the docs book (mdbook) instead of the demo
 *   --no-build        skip the build step (deploy the existing dist/)
 *   --project <name>  Pages project name        (env RUT_PAGES_PROJECT, default rut-playground / rut-book)
 *   --branch <name>   branch to deploy as       (env RUT_PAGES_BRANCH,   default: current git branch, else main)
 *   --dist <dir>      directory to upload       (env RUT_PAGES_DIST,     default demo/dist / docs/book)
 *   --dry-run         build, print the deploy command, upload nothing
 *   -h, --help
 *
 * The branch the Pages project treats as PRODUCTION (and thus what the
 * custom domain serves) defaults to `master`; override with
 * RUT_PAGES_PRODUCTION_BRANCH.
 *
 * Auth (either):
 *   CLOUDFLARE_API_TOKEN (+ CLOUDFLARE_ACCOUNT_ID when the token sees
 *   multiple accounts) — the non-interactive/CI path, or
 *   `npx wrangler login` once — the interactive OAuth path.
 *
 * Deploys from the Pages project's PRODUCTION branch (usually `master`)
 * go live on the custom domain; any other branch lands as a preview
 * deployment at <hash>.<project>.pages.dev.
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
  console.log(`usage: node scripts/deploy.cjs [--book] [--no-build] [--project <name>] [--branch <name>] [--dist <dir>] [--dry-run]`);
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

// which thing ships: the demo playground (default) or the docs book
const book = argv.includes("--book");

const doBuild = !argv.includes("--no-build");
const dryRun = argv.includes("--dry-run");
const defaultProject = book ? "rut-book" : "rut-playground";
const defaultDist = book ? "docs/book" : "demo/dist";
const project = takeValue("--project") ?? process.env.RUT_PAGES_PROJECT ?? defaultProject;
const dist = path.resolve(ROOT, takeValue("--dist") ?? process.env.RUT_PAGES_DIST ?? defaultDist);
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
  (a) => a.startsWith("-") && a !== "--no-build" && a !== "--dry-run" && a !== "--book"
);
if (unknown.length) {
  usage();
  die(`unknown option(s): ${unknown.join(" ")}`);
}

// ---------------------------------------------------------------------------
// 1. build what ships
//    - demo: preflight-wasm runs inside `npm run build` and fails loudly
//      with the exact build:wasm command when artifacts are missing
//    - book: mdbook renders docs/src into docs/book
// ---------------------------------------------------------------------------

if (!doBuild) {
  info(dim("skipping build (--no-build)"));
} else if (book) {
  const v = spawnSync("mdbook", ["--version"], { encoding: "utf8" });
  if (v.error?.code === "ENOENT") {
    die("mdbook not found — install it with `cargo install mdbook --locked`");
  }
  info(dim(`building docs/ — mdbook build docs (${(v.stdout || "").trim()})`));
  if (!dryRun) {
    const r = spawnSync("mdbook", ["build", "docs"], { cwd: ROOT, stdio: "inherit" });
    if (r.status !== 0) die(`book build failed (exit ${r.status ?? "?"})`);
  }
} else {
  info(dim("building demo/ — npm run build"));
  if (!dryRun) {
    const r = spawnSync("npm", ["run", "build"], { cwd: DEMO, stdio: "inherit", shell: process.platform === "win32" });
    if (r.status !== 0) die(`demo build failed (exit ${r.status ?? "?"})`);
  }
}

// ---------------------------------------------------------------------------
// 2. sanity-check what we are about to ship
// ---------------------------------------------------------------------------

if (book) {
  // index.html (the redirect to the intro) plus a real rendered book —
  // a bare index.html alone would mean an empty build slipped through
  const htmls = fs.existsSync(dist)
    ? fs.readdirSync(dist, { recursive: true }).filter((f) => String(f).endsWith(".html"))
    : [];
  if (!fs.existsSync(path.join(dist, "index.html")) || htmls.length < 10) {
    die(
      `${dist} does not look like a rendered book (${htmls.length} html files) — run ` +
        `\`mdbook build docs\` (or deploy with the build step enabled)`
    );
  }
  ok(`book ready: ${dist} (${htmls.length} pages) — ${isProduction ? bold("PRODUCTION") : `preview ${bold(branch)}`}`);
} else {
  const need = ["index.html", "main.js", "rut.wasm", "rut-lsp.wasm"];
  const missing = need.filter((f) => !fs.existsSync(path.join(dist, f)));
  if (missing.length) {
    die(
      `${dist} is missing ${missing.join(", ")} — run ` +
        `\`cd demo && npm run build\` (wasm artifacts come from \`npm run build:wasm\`)`
    );
  }
  ok(`dist ready: ${dist} — ${isProduction ? bold("PRODUCTION") : `preview ${bold(branch)}`}`);
}

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

const domain = book ? "rut.hpp2334.com" : "playground.rut.hpp2334.com";
const what = book ? "docs/book" : "demo/dist";

ok(`deployed ${what} → Cloudflare Pages project ${bold(project)} (branch ${bold(branch)})`);
console.log(`
  ${bold(isProduction ? "production" : "preview")} deployment — see the URL wrangler printed above.

  Custom domain: ${bold(domain)} serves the PRODUCTION
  branch (${bold(productionBranch)}) of this project. If the domain is not
  attached yet (one-time):
    Dashboard → Workers & Pages → ${project} → Custom domains → Set up a
    custom domain → ${domain} (Cloudflare adds the CNAME${book ? ", flattened at the apex" : ""}).

  Re-deploy without rebuilding:  node scripts/deploy.cjs ${book ? "--book " : ""}--no-build
`);
