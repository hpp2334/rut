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
 *   --book            ship the docs book (mdbook) instead of the demo.
 *                     The book lane also builds the ▶ Run buttons' wasm
 *                     artifact: `rustup target add wasm32-unknown-unknown`
 *                     (preflight) + `cargo build -p rut-wasm --target
 *                     wasm32-unknown-unknown --release`, staged to
 *                     docs/wasm/rut.wasm (gitignored build output; the
 *                     build is skipped only when --no-build is passed AND
 *                     the staged artifact already exists), then copied
 *                     post-build into the rendered book — mdbook 0.5
 *                     dropped `additional-resources`, so the lane does
 *                     that copy itself. The post-build step also bakes
 *                     static highlight spans (bake-book.mjs).
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
  // ---- the book lane: artifact, then the rendered book ---------------
  // (a) wasm preflight — the run buttons' artifact target. Die loudly
  //     naming the exact command (the repo's no-silent-fallback law).
  const haveTarget = spawnSync("rustup", ["target", "list", "--installed"], {
    encoding: "utf8",
  });
  const installed =
    haveTarget.status === 0 ? haveTarget.stdout.split(/\s+/) : [];
  if (!installed.includes("wasm32-unknown-unknown")) {
    info(dim("adding the wasm32 target — rustup target add wasm32-unknown-unknown"));
    if (!dryRun) {
      const add = spawnSync(
        "rustup",
        ["target", "add", "wasm32-unknown-unknown"],
        { stdio: "inherit" }
      );
      if (add.status !== 0)
        die(
          "`rustup target add wasm32-unknown-unknown` failed (exit " +
            (add.status ?? "?") + ") — install rustup and retry"
        );
    }
  }

  // (b) build crates/rut-wasm and stage the artifact. docs/wasm/ is
  //     gitignored build output, so --no-build only skips this when the
  //     staged artifact is already there; a missing artifact is built
  //     even under --no-build (the sanity check below refuses a book
  //     whose run buttons would 404).
  const artifact = path.join(ROOT, "docs", "wasm", "rut.wasm");
  const skipWasmBuild = !doBuild && fs.existsSync(artifact);
  if (skipWasmBuild) {
    info(dim("skipping the wasm build (--no-build) — docs/wasm/rut.wasm present"));
  } else {
    if (!doBuild)
      info(
        dim("docs/wasm/rut.wasm missing — building it despite --no-build (the artifact is not committed)")
      );
    info(
      dim(
        "building the run-button artifact — cargo build -p rut-wasm --target wasm32-unknown-unknown --release"
      )
    );
    if (!dryRun) {
      const built = spawnSync(
        "cargo",
        ["build", "-p", "rut-wasm", "--target", "wasm32-unknown-unknown", "--release"],
        { cwd: ROOT, stdio: "inherit" }
      );
      if (built.status !== 0)
        die(
          "`cargo build -p rut-wasm --target wasm32-unknown-unknown --release` failed " +
            "(exit " + (built.status ?? "?") + ") — run it directly for the full error"
        );
      const from = path.join(
        ROOT,
        "target",
        "wasm32-unknown-unknown",
        "release",
        "rut_wasm.wasm"
      );
      if (!fs.existsSync(from))
        die(
          "cargo produced no target/wasm32-unknown-unknown/release/rut_wasm.wasm — " +
            "check crates/rut-wasm's crate-type"
        );
      fs.mkdirSync(path.join(ROOT, "docs", "wasm"), { recursive: true });
      fs.copyFileSync(from, artifact);
    }
  }

  // (c) render the book
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
// 1.5 book post-build: bake spans + stage the wasm artifact into the book
//     (mdbook 0.5 dropped output.html.additional-resources — verified, the
//     build refuses the key — so the lane does both copies itself; both
//     are idempotent and also self-heal a --no-build deploy)
// ---------------------------------------------------------------------------

if (book && !dryRun) {
  if (!fs.existsSync(dist)) {
    die(`${dist} does not exist — run \`mdbook build docs\` (or deploy with the build step enabled)`);
  }
  const bake = path.join(
    ROOT,
    "integrations",
    "rut-highlightjs",
    "scripts",
    "bake-book.mjs"
  );
  if (fs.existsSync(bake)) {
    info(dim("baking static highlight spans — node integrations/rut-highlightjs/scripts/bake-book.mjs"));
    const bk = spawnSync("node", [bake, dist], { cwd: ROOT, stdio: "inherit" });
    if (bk.status !== 0)
      die(`bake-book failed (exit ${bk.status ?? "?"}) — run \`node ${path.relative(ROOT, bake)} ${dist}\` directly`);
  }
  const artifact = path.join(ROOT, "docs", "wasm", "rut.wasm");
  if (!fs.existsSync(artifact)) {
    die(
      "docs/wasm/rut.wasm is missing — run " +
        "`cargo build -p rut-wasm --target wasm32-unknown-unknown --release` and " +
        "copy target/wasm32-unknown-unknown/release/rut_wasm.wasm to docs/wasm/rut.wasm"
    );
  }
  fs.mkdirSync(path.join(dist, "wasm"), { recursive: true });
  fs.copyFileSync(artifact, path.join(dist, "wasm", "rut.wasm"));
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
  // the run buttons' artifact ships with the book — a missing one would
  // 404 on every ▶ Run click (the buttons show the loud build panel,
  // but a deploy that knowingly ships it is a broken deploy)
  if (!fs.existsSync(path.join(dist, "wasm", "rut.wasm"))) {
    die(
      `${dist} is missing wasm/rut.wasm (the ▶ Run buttons would 404) — build the artifact ` +
        `with \`cargo build -p rut-wasm --target wasm32-unknown-unknown --release\`, copy ` +
        `target/wasm32-unknown-unknown/release/rut_wasm.wasm to docs/wasm/rut.wasm, then ` +
        `\`mdbook build docs\` (or deploy with the build step enabled)`
    );
  }
  ok(`book ready: ${dist} (${htmls.length} pages, wasm/rut.wasm) — ${isProduction ? bold("PRODUCTION") : `preview ${bold(branch)}`}`);
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
