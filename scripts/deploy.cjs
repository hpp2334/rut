#!/usr/bin/env node
/**
 * Deploy to Cloudflare Pages. Two targets, one script:
 *
 *   demo (default)   demo/dist/       → playground.rut.hpp2334.com
 *   book (--book)    docs/dist-book/  → rut.hpp2334.com
 *
 *   The book target is a TWO-EDITION merge: the English book at the
 *   site root and the zh-CN book under /zh/ (one Pages project, one
 *   upload dir). docs/book/ and docs/book-zh/ are intermediate build
 *   output; docs/dist-book/ is the assembled upload dir.
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
 *                     Builds BOTH editions loud-fail: `mdbook build
 *                     docs` (en) and the zh edition via docs/book.zh.toml
 *                     (never skipped — the untranslated catalog falls
 *                     back to English and THAT FALLBACK IS THE DESIGN).
 *                     mdbook 0.5 has no -c/--config, so the zh edition
 *                     builds through a thin shim dir mirroring docs/
 *                     (book.toml → ../book.zh.toml + src/theme/po
 *                     symlinks; see docs/book.zh.toml). The lane also
 *                     builds the ▶ Run buttons' wasm artifact:
 *                     `rustup target add wasm32-unknown-unknown`
 *                     (preflight) + `cargo build -p rut-wasm --target
 *                     wasm32-unknown-unknown --release`, staged to
 *                     docs/wasm/rut.wasm (gitignored build output; the
 *                     build is skipped only when --no-build is passed AND
 *                     the staged artifact already exists), then copied
 *                     post-build into BOTH rendered editions — mdbook 0.5
 *                     dropped `additional-resources`, so the lane does
 *                     that copy itself (the buttons fetch wasm/rut.wasm
 *                     relative to the page, so each edition root needs
 *                     its own). The post-build step also bakes static
 *                     highlight spans (bake-book.mjs) per edition.
 *   --no-build        skip the build step (deploy the existing dist/).
 *                     Book lane: reuses existing docs/book + docs/book-zh
 *                     (still bakes/merges); missing edition → loud death
 *                     with the build commands.
 *   --project <name>  Pages project name        (env RUT_PAGES_PROJECT, default rut-playground / rut-book)
 *   --branch <name>   branch to deploy as       (env RUT_PAGES_BRANCH,   default: current git branch, else main)
 *   --dist <dir>      directory to upload       (env RUT_PAGES_DIST,     default demo/dist / docs/dist-book)
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
// the book lane: intermediate edition dirs + the merged upload dir
const DOCS = path.join(ROOT, "docs");
const BOOK_ROOTS = {
  en: path.join(DOCS, "book"),
  zh: path.join(DOCS, "book-zh"),
};
const BOOK_MERGED = path.join(DOCS, "dist-book");

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
const defaultDist = book ? "docs/dist-book" : "demo/dist";
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

/**
 * Build the zh-CN edition into docs/book-zh.
 *
 * mdbook 0.5 has no `-c/--config` flag, so the edition config
 * (docs/book.zh.toml) is built through a throwaway shim dir whose
 * layout mirrors docs/: book.toml → ../book.zh.toml plus src/theme/po
 * symlinks. That keeps every path inside book.zh.toml written exactly
 * like docs/book.toml's. The shim is removed even when the build
 * fails. See the header of docs/book.zh.toml.
 */
function buildZhEdition(mdbookVersion) {
  const shim = path.join(DOCS, ".book-zh");
  fs.rmSync(shim, { recursive: true, force: true });
  fs.mkdirSync(shim);
  const link = (from, to) => fs.symlinkSync(from, path.join(shim, to));
  try {
    link("../book.zh.toml", "book.toml");
    link("../src", "src");
    link("../theme", "theme");
    link("../po", "po");
    info(
      dim(
        `building docs/ (zh) — mdbook build docs/.book-zh -d docs/book-zh (${mdbookVersion.trim()})`
      )
    );
    const r = spawnSync(
      "mdbook",
      ["build", path.relative(ROOT, shim), "-d", path.relative(ROOT, BOOK_ROOTS.zh)],
      { cwd: ROOT, stdio: "inherit" }
    );
    if (r.status !== 0)
      die(
        `zh edition build failed (exit ${r.status ?? "?"}) — try it directly: ` +
          `sh -c 'set -e; rm -rf docs/.book-zh; mkdir docs/.book-zh; cd docs/.book-zh; ` +
          `ln -s ../book.zh.toml book.toml; ln -s ../src src; ln -s ../theme theme; ln -s ../po po; ` +
          `cd ../..; mdbook build docs/.book-zh -d docs/book-zh; rm -rf docs/.book-zh'`
      );
  } finally {
    fs.rmSync(shim, { recursive: true, force: true });
  }
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
//    - book: mdbook renders docs/src into docs/book (en) and — via the
//      docs/book.zh.toml shim — docs/book-zh (zh), then the lane merges
//      both into docs/dist-book
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

  // (b) build crates/rut-wasm (wasm-pack, no-modules: the book's
  //     classic-script page has no module loader — the glue assigns the
  //     `wasm_bindgen` global) and stage BOTH files. docs/wasm/ is
  //     gitignored build output, so --no-build only skips this when the
  //     staged pair is already there; a missing artifact is built even
  //     under --no-build (the sanity check below refuses a book whose
  //     run buttons would 404).
  const artifact = path.join(ROOT, "docs", "wasm", "rut.wasm");
  const glue = path.join(ROOT, "docs", "wasm", "rut-wasm.js");
  const skipWasmBuild = !doBuild && fs.existsSync(artifact) && fs.existsSync(glue);
  if (skipWasmBuild) {
    info(
      dim("skipping the wasm build (--no-build) — docs/wasm/rut.wasm + rut-wasm.js present")
    );
  } else {
    if (!doBuild)
      info(
        dim("docs/wasm/ missing the artifact pair — building it despite --no-build (the artifacts are not committed)")
      );
    const havePack = spawnSync("wasm-pack", ["--version"], { encoding: "utf8" });
    if (havePack.error?.code === "ENOENT") {
      die(
        "wasm-pack not found — install it with `cargo install wasm-pack --locked` " +
          "(or the installer at rustwasm.github.io/wasm-pack/installer)"
      );
    }
    info(
      dim(
        "building the run-button artifact — wasm-pack build crates/rut-wasm --target no-modules --out-dir pkg-book --release"
      )
    );
    if (!dryRun) {
      const built = spawnSync(
        "wasm-pack",
        [
          "build",
          "crates/rut-wasm",
          "--target",
          "no-modules",
          "--out-dir",
          "pkg-book",
          "--release",
        ],
        { cwd: ROOT, stdio: "inherit" }
      );
      if (built.status !== 0)
        die(
          "`wasm-pack build crates/rut-wasm --target no-modules --out-dir pkg-book --release` failed " +
            "(exit " + (built.status ?? "?") + ") — run it directly for the full error"
        );
      const fromWasm = path.join(ROOT, "crates", "rut-wasm", "pkg-book", "rut_wasm_bg.wasm");
      const fromGlue = path.join(ROOT, "crates", "rut-wasm", "pkg-book", "rut_wasm.js");
      if (!fs.existsSync(fromWasm))
        die(
          "wasm-pack produced no crates/rut-wasm/pkg-book/rut_wasm_bg.wasm — " +
            "check crates/rut-wasm's crate-type"
        );
      if (!fs.existsSync(fromGlue))
        die(
          "wasm-pack produced no crates/rut-wasm/pkg-book/rut_wasm.js — " +
            "check crates/rut-wasm's crate-type"
        );
      fs.mkdirSync(path.join(ROOT, "docs", "wasm"), { recursive: true });
      fs.copyFileSync(fromWasm, artifact);
      fs.copyFileSync(fromGlue, glue);
    }
  }

  // (c) render BOTH editions. The zh build is NEVER skipped — with the
  //     seeded untranslated catalog every entry falls back to the
  //     English source text and that fallback is the design.
  const v = spawnSync("mdbook", ["--version"], { encoding: "utf8" });
  if (v.error?.code === "ENOENT") {
    die("mdbook not found — install it with `cargo install mdbook --locked`");
  }
  info(dim(`building docs/ (en) — mdbook build docs (${(v.stdout || "").trim()})`));
  // ALWAYS built — under --dry-run too: the lane's promise is that the
  // assembled docs/dist-book/ holds REAL renders (the zh edition
  // especially: a stale gitignored docs/book-zh/ would silently pass
  // the sanity gates against pages that no longer exist in the source)
  const r = spawnSync("mdbook", ["build", "docs"], { cwd: ROOT, stdio: "inherit" });
  if (r.status !== 0) die(`book build failed (exit ${r.status ?? "?"})`);
  buildZhEdition(v.stdout || "");
} else {
  info(dim("building demo/ — npm run build"));
  if (!dryRun) {
    const r = spawnSync("npm", ["run", "build"], { cwd: DEMO, stdio: "inherit", shell: process.platform === "win32" });
    if (r.status !== 0) die(`demo build failed (exit ${r.status ?? "?"})`);
  }
}

// ---------------------------------------------------------------------------
// 1.5 book post-build: bake spans + stage the wasm artifact into BOTH
//     editions, then assemble the merged upload dir (en at the site
//     root, zh under /zh/). mdbook 0.5 dropped
//     output.html.additional-resources — verified, the build refuses
//     the key — so the lane does the copies itself; every step is
//     idempotent and also self-heals a --no-build deploy. The run
//     buttons fetch wasm/rut.wasm relative to the page, so EACH
//     edition root carries the artifact.
// ---------------------------------------------------------------------------

if (book) {
  const artifact = path.join(ROOT, "docs", "wasm", "rut.wasm");
  const glue = path.join(ROOT, "docs", "wasm", "rut-wasm.js");
  const bake = path.join(
    ROOT,
    "integrations",
    "rut-highlightjs",
    "scripts",
    "bake-book.mjs"
  );
  const notRendered = (root) => !fs.existsSync(path.join(root, "index.html"));

  // (a) bake + wasm into the intermediate editions (skipped under
  //     --dry-run, like the pre-merge lane; the assemble step below
  //     still stages the artifact into the merged dir)
  if (!dryRun) {
    for (const root of Object.values(BOOK_ROOTS)) {
      if (notRendered(root)) {
        die(
          `${root} does not exist or is not a rendered book — build both editions ` +
            `\`node scripts/deploy.cjs --book\` (or deploy with the build step enabled)`
        );
      }
    }
    for (const [lang, root] of Object.entries(BOOK_ROOTS)) {
      if (fs.existsSync(bake)) {
        info(dim(`baking static highlight spans (${lang}) — node integrations/rut-highlightjs/scripts/bake-book.mjs ${path.relative(ROOT, root)}`));
        const bk = spawnSync("node", [bake, root], { cwd: ROOT, stdio: "inherit" });
        if (bk.status !== 0)
          die(`bake-book failed (exit ${bk.status ?? "?"}) — run \`node ${path.relative(ROOT, bake)} ${root}\` directly`);
      }
      if (!fs.existsSync(artifact) || !fs.existsSync(glue)) {
        die(
          "docs/wasm/ is missing the run-button pair (rut.wasm + rut-wasm.js) — run " +
            "`wasm-pack build crates/rut-wasm --target no-modules --out-dir pkg-book --release` and " +
            "copy crates/rut-wasm/pkg-book/{rut_wasm_bg.wasm,rut_wasm.js} to docs/wasm/ as rut.wasm/rut-wasm.js"
        );
      }
      fs.mkdirSync(path.join(root, "wasm"), { recursive: true });
      fs.copyFileSync(artifact, path.join(root, "wasm", "rut.wasm"));
      fs.copyFileSync(glue, path.join(root, "wasm", "rut-wasm.js"));
    }
  }

  // (b) assemble the merged upload dir: en book → root, zh book → zh/.
  //     Runs under --dry-run too: staging is local, only the upload is
  //     dry — that is what makes `--book --dry-run` a real gate.
  for (const root of Object.values(BOOK_ROOTS)) {
    if (notRendered(root)) {
      die(
        `${root} does not exist or is not a rendered book${doBuild ? "" : " (--no-build)"}` +
          `${dryRun ? " (this dry-run skipped the build step)" : ""} — ` +
          `build both editions first: \`node scripts/deploy.cjs --book\` (the zh edition ` +
          `builds automatically; see docs/book.zh.toml)`
      );
    }
  }
  info(dim(`assembling the two-edition upload dir — ${path.relative(ROOT, dist)}`));
  fs.rmSync(dist, { recursive: true, force: true });
  fs.mkdirSync(dist, { recursive: true });
  fs.cpSync(BOOK_ROOTS.en, dist, { recursive: true });
  fs.cpSync(BOOK_ROOTS.zh, path.join(dist, "zh"), { recursive: true });
  if (!fs.existsSync(artifact) || !fs.existsSync(glue)) {
    die(
      "docs/wasm/ is missing the run-button pair (rut.wasm + rut-wasm.js) — run " +
        "`wasm-pack build crates/rut-wasm --target no-modules --out-dir pkg-book --release` and " +
        "copy crates/rut-wasm/pkg-book/{rut_wasm_bg.wasm,rut_wasm.js} to docs/wasm/ as rut.wasm/rut-wasm.js"
    );
  }
  for (const sub of ["", "zh"]) {
    fs.mkdirSync(path.join(dist, sub, "wasm"), { recursive: true });
    fs.copyFileSync(artifact, path.join(dist, sub, "wasm", "rut.wasm"));
    fs.copyFileSync(glue, path.join(dist, sub, "wasm", "rut-wasm.js"));
  }

  // ---------------------------------------------------------------------------
  // 2. sanity-check what we are about to ship — BOTH edition roots:
  //    index.html (the redirect to the intro), the run buttons'
  //    artifact, and a real rendered book (a bare index.html alone
  //    would mean an empty build slipped through)
  // ---------------------------------------------------------------------------

  const pages = {};
  for (const [lang, sub] of [["en", ""], ["zh", "zh"]]) {
    const root = path.join(dist, sub);
    const htmls = fs.existsSync(root)
      ? fs
          .readdirSync(root, { recursive: true })
          .filter((f) => String(f).endsWith(".html"))
          // the en root count excludes the nested zh/ subtree
          .filter((f) => String(f).split(path.sep)[0] !== "zh")
      : [];
    if (!fs.existsSync(path.join(root, "index.html")) || htmls.length < 10) {
      die(
        `${root} does not look like a rendered book (${htmls.length} html files) — ` +
          `build both editions (\`node scripts/deploy.cjs --book\`)`
      );
    }
    // the run buttons' artifact ships with EACH edition — a missing one
    // would 404 on every ▶ Run click (the buttons show the loud build
    // panel, but a deploy that knowingly ships it is a broken deploy)
    for (const f of ["rut.wasm", "rut-wasm.js"]) {
      if (!fs.existsSync(path.join(root, "wasm", f))) {
        die(
          `${root} is missing wasm/${f} (the ▶ Run buttons would 404) — build the artifact ` +
            `with \`wasm-pack build crates/rut-wasm --target no-modules --out-dir pkg-book --release\`, then ` +
            `\`node scripts/deploy.cjs --book\` (the lane copies the pair into both editions)`
        );
      }
    }
    pages[lang] = htmls.length;
  }
  ok(
    `book ready (2 editions): ${dist} — en ${pages.en} pages at /, zh ${pages.zh} pages at /zh/, ` +
      `wasm/rut.wasm in both — ${isProduction ? bold("PRODUCTION") : `preview ${bold(branch)}`}`
  );
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
const what = book ? "docs/dist-book (en + zh editions)" : "demo/dist";

ok(`deployed ${what} → Cloudflare Pages project ${bold(project)} (branch ${bold(branch)})`);
console.log(`
  ${bold(isProduction ? "production" : "preview")} deployment — see the URL wrangler printed above.

  Custom domain: ${bold(domain)}${book ? ` (en at /, zh at ${bold(domain + "/zh/")})` : ""} serves the PRODUCTION
  branch (${bold(productionBranch)}) of this project. If the domain is not
  attached yet (one-time):
    Dashboard → Workers & Pages → ${project} → Custom domains → Set up a
    custom domain → ${domain} (Cloudflare adds the CNAME${book ? ", flattened at the apex" : ""}).

  Re-deploy without rebuilding:  node scripts/deploy.cjs ${book ? "--book " : ""}--no-build
`);
