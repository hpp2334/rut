# AGENTS.md — notes for coding agents working in this repo

## Lint gate: warnings are errors; keep clippy and fmt green

The workspace denies every warn-level rustc/clippy lint
(`[workspace.lints] warnings = "deny"` in the root `Cargo.toml`,
reaching all members through `[lints] workspace = true`), and CI
(`.github/workflows/ci.yml`) gates on `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets`, clippy on the wasm32 target,
and `cargo nextest run`. Practical consequences:

- A new warning anywhere in a member crate fails the build — fix the
  cause; if a lint is genuinely wrong for a site, a scoped
  `#[allow(...)]` with a one-line reason is the house style (grep for
  existing examples before inventing wording).

## Browser drives: use `agent-browser`, not hand-rolled WebDriver

When a task needs a REAL browser (driving the demo playground, the
05-todolist-web page, verifying a tunnel URL, any "open it and check"
acceptance):

1. **`agent-browser` is installed** (`/usr/local/bin/agent-browser`) and
   is the default. It is headless, needs no desktop app, and takes
   plain commands:

   ```sh
   agent-browser open <url>        # navigate
   agent-browser snapshot          # a11y tree with @eN refs (ALWAYS refetch after nav/changes)
   agent-browser click @e3         # click / fill @e2 "text" / press Enter / find "text"
   agent-browser screenshot out.png
   ```

2. The session's `browser.*` MCP tools (desktop bridge) may report
   `[browser.disconnected]` in headless/batch sessions. That is NOT a
   reason to fall back to hand-rolled WebDriver plumbing — `agent-browser`
   works there.

3. Last resorts, in order:
   - the repo's own gates: `cd demo && npm run smoke` (headless, no
     browser) and `node tests/e2e-browser.mjs` from
     `examples/05-todolist-web` (node tier + Firefox-WebDriver tier);
   - raw geckodriver + headless Firefox via WebDriver HTTP only when
     EXTENDING those gates, not for ad-hoc drives.

### Practical notes

- The wasm pages boot asynchronously: the playground probes
  `GET /rut.wasm`, the todolist page mounts into `#app` after the glue
  instantiates. Over a tunnel that is a few seconds — poll/snapshot
  again instead of declaring the page broken on the first empty tree.
- Live demo lanes (rf. the READMEs): `demo/scripts/dev-channel.mjs`
  and `examples/05-todolist-web/scripts/dev-channel.mjs` serve the page
  on localhost and print a throwaway
  `https://<random>.trycloudflare.com` URL; `agent-browser` can drive
  that public URL directly, which is the closest thing to what the
  room will see.
- Deployed target: `cd demo && npm run deploy` ships the playground to
  Cloudflare Pages (production from `master`):
  <https://playground.rut.hpp2334.com>.

## Std publishing (pack → commit → publish)

The law: std ships as SOURCE ZIPS at immutable `std-vNN` git tags —
jsDelivr's gh lane serves committed repo files at a ref, so a tag names
committed bytes, and publish ADVANCES, never rewrites (an existing tag is
refused; a botched publish is fixed by the NEXT cut, never by re-tagging).
The pin table lives ONLY in `crates/rut-cli/template/rut.lock` — the lock is
the carrier: no script keeps a second pkg/sha/tag table (that duplication is
exactly how the retired bin lane rotted), and `cargo nextest run -p rut-cli
--test new_lane` holds the law.

Delivery is THREE ACTS with one boundary — no script ever types `git commit`:

1. **Cut**: `node scripts/pack-std.cjs` — derives the next tag (`git tag
   -l "std-v*"` max + 1), packs the 9 source zips into `dist/std-vNN/`
   AND the same bytes into the `dist/std/` working mirror (the path the
   lock urls serve), and re-pins `crates/rut-cli/template/rut.jsonc`
   (url token swap) + `template/rut.lock` (regenerated). It prints a
   NEXT banner and STOPS before git.
2. **Review + commit (a HUMAN act)**: read `git diff` — 18 binary zips
   (the `dist/std-vNN/` envelope + the `dist/std/` mirror) + 2 text
   files — and commit. This is the review point.
3. **Publish**: `node scripts/publish-std.cjs` — the gates, then
   `git tag -a <tag>` + push, then a jsDelivr sha-verify of all 9 urls,
   then the e2e: `rut new → install → run` must print `hello, rut!`.

Gate modes (no act; each gate dies loudly on drift): `node
scripts/pack-std.cjs --check` is the offline, CI-shaped gate (repacks to a
temp dir and byte-compares against the committed envelope — no network, no
git writes); `node scripts/publish-std.cjs --dry-run` runs the publish
gates and PRINTS the tag + push without doing them (it still derives the
tag from origin).

History: the `.rutbundle` bin lane is retired. The frozen `dist/std*`
envelopes (`dist/std/`, `dist/std-v12/`, `dist/std-v13/`, …) stay committed
untouched — old readers and test fixtures still read them — and the old
pack script lives in git history.

## VS Code extension — `npm run release` (build → gates → pack)

The extension (`integrations/vscode-extension/`) ships two local,
gitignored artifacts: `bin/rut-lsp.wasm` (the in-process LSP, built by
wasm-pack) and `rut-vscode-<version>.vsix`. One lane produces both, in
order:

    cd integrations/vscode-extension
    npm run release        # preflight (cargo+wasm-pack on PATH) →
                           # build:wasm (wasm-pack --target web) →
                           # grammar corpus + through-wasm e2e (+ the
                           # skip-tolerant host suite) → vsce package →
                           # byte-verify the wasm inside the vsix

- **Two shells, one core**: `crates/rut-lsp` is the queries (no
  transport, no tokio); `crates/rut-lsp-server` is the stdio face and
  ships the `rut-lsp` binary (package `rut-lsp-server`, binary name
  unchanged — build with `cargo build -p rut-lsp-server`);
  `crates/rut-lsp-wasm` is the in-process module. Add a feature to the
  core, then wire it in both shells.
- **The ABI face lives in the crate**: `crates/rut-lsp-wasm/js/index.js`
  (the envelope parsing) and `js/index.d.ts` (every LSP/wire shape) are
  assembled into the wasm package by `scripts/build-lsp-pkg.mjs` (run by
  every `build:wasm`), and both hosts — this extension and the demo
  playground — import `crates/rut-lsp-wasm/pkg/index.js`. A host must
  never declare LSP shapes; it may only map them (the extension maps to
  `vscode.*`). The contract has two gates: `npm run typecheck`
  (`tsc --noEmit`, now part of `npm test` and the release lane) and the
  `wire-shape` block in `test/e2e-wasm.js`, which asserts each
  envelope's exact key set through the shipped artifact — the only gate
  that can catch a Rust-side rename, since the wire is data.
- **When the frontend moves, the lane moves**: any change touching the
  lexer, parser, `rut-lsp`, or the TextMate grammar must run this lane
  (or at minimum `build:wasm` + `test:e2e`) before the work is done —
  a new syntax reads as false diagnostics through a stale wasm, and the
  corpus gate catches exactly that.
- **The gates gate the shipped bytes**: the e2e drives `bin/rut-lsp.wasm`,
  the pack embeds that same file, and the lane byte-compares the copy
  inside the vsix — a stale artifact cannot pass.
- **No version writes, no git**: the script packs whatever `package.json`
  says; a bump is a deliberate human act
  (`npm version X.Y.Z --no-git-tag-version`), and older
  `rut-vscode-*.vsix` files are listed, never deleted.
- `npm run release -- --dry-run` runs the build and the gates, then stops
  before packing.
- The demo carries its OWN wasm copy (`demo/public/rut-lsp.wasm`,
  `cd demo && npm run build:lsp`) — this lane does not refresh it.

## Docs: the book lives in `docs/`, RFCs are gone

- The user manual is an mdbook site: source in `docs/src/` (TOC:
  `docs/src/SUMMARY.md`), built with `mdbook build docs` (install:
  `cargo install mdbook --locked`). Sections: Quick Start, Tutorial,
  Core Concepts, Examples, Reference. The deployed book is
  <https://rut.hpp2334.com>.
- There is NO `rfc/` directory anymore — its content was distilled into
  the book. When code comments or older branches cite "RFC NNNN", map
  the topic to a `docs/src/reference/*.md` page instead of looking for
  the file. Do not reintroduce RFC-numbered citations in user-facing
  prose (docs, READMEs); historical mentions inside source comments may
  stay.
- Docs deploy: `node scripts/deploy.cjs --book` (pages project
  `rut-book` → `rut.hpp2334.com`); the demo target
  (`node scripts/deploy.cjs`) is unchanged.
- Book run buttons: qualifying ```rut blocks (those containing
  `entry fn main`) get a "▶ Run" button (docs/theme/rut-book.js) that
  runs the block in-browser on the rut wasm engine. The artifact is
  the wasm-bindgen PAIR built from `crates/rut-wasm`
  (`wasm-pack build crates/rut-wasm --target no-modules --out-dir
  pkg-book --release`) and must land at `docs/wasm/` as `rut.wasm` +
  `rut-wasm.js` (the classic-script glue — the book page has no module
  loader); the heap mirrors the default (4 MiB), while
  the book host EXPLICITLY opts into 10M fuel — the mechanism default
  is OFF (engine `Limits::default()` is uncapped) and the
  playground's user-facing default is OFF too (opt-in via the fuel
  box; `DEFAULT_BUDGET.fuel = 0` is the ABI's uncapped encoding).
- `docs/wasm/` is gitignored build output (like `docs/book/`) — never
  commit the artifact; the `--book` deploy lane builds and copies it
  into the rendered book (mdbook 0.5 dropped
  `output.html.additional-resources`, so the lane does the copy).
- Tests run through `cargo nextest run` (cargo-nextest is installed),
  not `cargo test` — keep this for every test invocation.
- Anti-rot gate for book code: `cargo nextest run -p rut-cli --test
  book_blocks` compiles + runs every runnable ```rut block in
  docs/src natively and must pass whenever book code blocks change.
  Blocks that cannot run yet are skipped in the test's SKIP table
  with the reason — clear entries there when fixing docs content.
- When you change language surface or CLI behavior, update the matching
  book pages in the same change — `mdbook build docs` fails on dead
  links and missing TOC files, so run it before committing doc edits.
- Bilingual book (en + zh-CN): English is the single source of truth —
  all prose changes happen in `docs/src/` (and `docs/book.toml`), then
  the matching `docs/po/zh_CN.po` entries are updated in the SAME
  change. The zh-CN terminology contract in `docs/po/GLOSSARY.md` is
  binding for every catalog entry; untranslated entries fall back to
  the English text by design, and the zh edition must build even at
  0% translated. `docs/po/` is source, never gitignored;
  `docs/po/rut.pot` is committed and re-extracted with one byte-stable
  command after any prose change (it feeds translators' diffing):
  `MDBOOK_OUTPUT='{"xgettext": {}}' mdbook build docs -d docs/po/_extract && sed -i 's/^"POT-Creation-Date:.*/"POT-Creation-Date: 1970-01-01 00:00:00+00:00\\n"/' docs/po/_extract/messages.pot && mv docs/po/_extract/messages.pot docs/po/rut.pot && rm -rf docs/po/_extract`
  then fold new/changed strings into `docs/po/zh_CN.po` with a PO
  tool. Both editions must build before committing doc changes:
  `mdbook build docs` (en) and
  `node scripts/deploy.cjs --book --dry-run` (builds the zh edition
  via `docs/book.zh.toml` — mdbook 0.5 has no `-c`, so deploy.cjs
  wraps it in a throwaway shim dir — then assembles `docs/dist-book/`:
  en at the root, zh at `/zh/`, `wasm/rut.wasm` in both — and runs the
  sanity gates without uploading). The header EN | 中文 toggle
  (`docs/theme/lang-toggle.js`, both editions) links each page to its
  sibling edition; keep it wired into BOTH books' `additional-js`.
