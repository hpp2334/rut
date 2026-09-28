# AGENTS.md — notes for coding agents working in this repo

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
  `pub fn main`) get a "▶ Run" button (docs/theme/rut-book.js) that
  runs the block in-browser on the rut wasm engine. The artifact is
  built from `crates/rut-wasm` (`cargo build -p rut-wasm --target
  wasm32-unknown-unknown --release`) and must land at
  `docs/wasm/rut.wasm`; budgets mirror the playground's
  DEFAULT_BUDGET (10,000,000 fuel / 4 MiB heap).
- `docs/wasm/` is gitignored build output (like `docs/book/`) — never
  commit the artifact; the `--book` deploy lane builds and copies it
  into the rendered book (mdbook 0.5 dropped
  `output.html.additional-resources`, so the lane does the copy).
- Anti-rot gate for book code: `cargo test -p rut-cli --test
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
