# AGENTS.md — notes for coding agents working in this repo

## Lint gate: warnings are errors; keep clippy and fmt green

The workspace denies every warn-level rustc/clippy/rustdoc lint
(`[workspace.lints] warnings = "deny"` in the root `Cargo.toml`,
reaching all members through `[lints] workspace = true`) — the local
gates are `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets`, clippy on the wasm32 target, `cargo nextest run`, and
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` (rustdoc
warnings are denied through the same `[lints]` table, and the command
carries `-D warnings` itself, so a toolchain change cannot turn the
lane green with warnings). There is no CI; the deny-by-default
workspace lints make any of these fail loudly on drift. Practical
consequences:

- A new warning anywhere in a member crate fails the build — fix the
  cause; if a lint is genuinely wrong for a site, a scoped
  `#[allow(...)]` with a one-line reason is the house style (grep for
  existing examples before inventing wording).

## Planning: read the plan-rule skill before writing any plan

Any "plan X" / "design X" / design-convergence work — any document meant
to drive implementation later — starts by reading
`.opencode/skills/plan-rule/SKILL.md` and following it. The core, in
one breath: a plan is a LAW plus its derived changes, never a patch
list — diagnose the old law that produces the observed behavior,
replace it with one named mechanism, derive every edit from it; carry
ground truth verified THIS session (`file:line`, grep-proven call
sites); supersede outright with NO backward compat (shims, dual paths,
kept-for-safety code forbidden; pinned tests update in the same
change); package as self-contained phases that each leave every gate
green, with disjoint-scope parallel tasks INSIDE a phase (never
parallel phases); and pass the skill's pre-flight audit before saving.
Plans live in `~/.opencode/plan/<slug>.md` (the slug names the law); a
saved plan is a proposal — how, and whether, it executes is the user's
call.

## Frontend layering: one parse entry, lsp-wasm talks only to rut-lsp

**Call the layer whose abstraction you consume.** Document meaning
(classify, symbols, mode derivation, position math) → rut-lsp's
`doc` module. Syntax only (reprint, transform) → the grammar's
entry pair `rut_ast::parser::parse` / `parse_with_recover` (fmt does
exactly this), or `Doc::parse` directly. Nobody outside `rut-ast`'s
`parser` module drives `normalize → lex → parse` by hand — the
pipeline lives in the `Doc`.

- **One crate, two halves**: `rut-ast` is the tree AND its grammar —
  `src/ast/` is the vocabulary (the store, node structs, handles, the
  `AstPrinter`, the `Doc`), `src/parser/` is the grammar (the frame
  machine, the window entry, keyword tables). They co-change on every
  syntax edit, so they live together; the halves stay nameable so the
  layer law below can forbid reaching into the grammar from read-only
  lanes. The `ast` module is a sealed `mod` (the same privacy wall as
  rut-lsp's `doc`), so the tree vocabulary is reachable ONLY
  through the flat crate-root re-exports.
- **The AST artifact law (v4)**: `Ast` stores PURE STRUCTURE — the
  typed nodes, their spans/verdicts, the interner vocabulary, the
  derived `TreeIndex`. NO src, no text, no tokens. The artifact is the
  `Doc { src, toks, ast }` — `src` is the NORMALIZED text every span
  indexes (never the caller's raw input), and `Doc::text(span)` is the
  join (text queries are span×src joins over the doc's own text;
  nothing else stores text). The leaf layer lives IN the doc: one
  deterministic linear pass (`rut_lexer::lexer::lex_normalized`)
  derives it cold, the splice maintains it edited.
- **The capsule law (v4)**: nodes are POD; mutation is MODELED — parse
  constructs, `Doc::edit` repairs; no field pokes anywhere. `Ast`'s
  mut doors (`new`/`alloc`/`intern`/`set_root`/`set_errors` + the edit
  lane's repair doors `reseal_root`/`remove_any`/`index_invalidate`)
  are `pub(crate)` — the parse/edit lane is INSIDE the crate, so the
  compiler enforces the capsule: outside crates can only read
  (`root`/`name`/`errors`/`is_clean`/`interner` and the geometry
  doors), even holding an owned `Ast`. Handles are stable for the
  tree's lifetime (slot-map arenas; removal punches holes — a stale
  handle dies loudly on read, never reads yesterday's node). Behavior
  lives on nodes and handles with the store as ctx (`h.get(ast)`,
  `root().items(ast)`, `slot.span(ast)`) — no smart objects, no
  per-node back-pointers, no builder ceremony. The same law governs
  the computed artifacts — `Doc` and `Document` (read doors only;
  consumers hold the doc and borrow) carry private fields. Plain
  records (node structs, index rows, driver/lir rows, rut-lsp's
  wire-bound `Analysis`) stay open — many construction sites, no
  invariant.
- **One grammar engine**: the one entry is `parse_item_window`
  (pub(crate)) — a token window in, root-level items out; a cold parse
  is just the window that covers everything. `Doc::parse` rides it,
  the edit lane rides it with the dirty tail into FRESH slots of the
  live store — no second parser to drift.
- **One mutator**: `Doc::edit(Edit)` is the only way a document
  changes. Inside (all private): `src.edit` → the token splice
  (`splice_tokens` → `TokenChange`) → the lex-fence prefix rebuilt →
  the dirty reuse unit found (v1 granularity: the ROOT ITEM; recovery
  backtracks past fences) → dropped subtrees punched into holes → the
  window reparse into fresh slots → splice under the surviving parent
  → the tree index INVALIDATED (the next read rebuilds it with the
  same deterministic walk a cold parse runs — the plan's in-place
  patch is consciously superseded; a finer reuse-unit granularity is a
  later knob). The wire shell adapts protocol → `Edit`
  (`Document::sync` = `Edit::between` + `doc.edit` + the lazy views
  dropped) — never diffs below the adapter. Edit ≡ cold is
  property-tested per stage: token splice ≡ cold lex
  (`crates/rut-lexer/tests/splice_props.rs`), edited doc ≡ cold parse
  (`crates/rut-ast/tests/doc_edit_props.rs`) — the tests' seeds derive
  from each file's bytes, never its path, so every checkout runs the
  same fixed sequences (doc K = 16, splice K = 64), and a found
  counterexample becomes a literal pin.
- **Doors are questions**: no raw `src()`/`toks()`/`line_starts()`
  handouts on `Doc` — text is `text(span)`, lines are
  `line_of`/`line_span`, tokens are `tokens_in(span)`, `name_at(pos)`
  the fence residue, `bindings()` the locals pass. The tree geometry
  lives on `Ast` (`node_at`/`parent`/`span_of`/`ancestors`/
  `is_incomplete` over `AnyHandle`); position questions walk the
  generic machinery (`node_at` → `ancestors` → `downcast`) — there is
  no `context_at` and no position-verdict enum: a feature is asked of
  a `(World, FileId)` pair — context, never a verdict. The editor's
  one classification of a caret is rut-lsp
  `resolve::Cursor` — a consumer of that geometry, not a second one.
- **Scoped token lane**: `name_at` is the fence residue, `tokens_in`
  the classifier's window — no other token interpretation crosses
  rut-ast. Outside the classifier (rut-lsp's `doc/tokens.rs` +
  `legend.rs`), no lsp code matches `Tok::`.
- **The span law (v3.1)**: a decl's span starts at its FIRST source
  token — modifiers included — and ends at its own terminator (`}` /
  `;`); a block's and a when-arm's span end at their own last token,
  and a block that never received its `{` spans nothing. The
  brace-bodied type decls (class/struct/interface/enum and
  the surface/builtin family) also record `body_span` (the `{..}`
  region, braces included) — the head slice's right cut.
- **The modifier-node law**: modifiers are spanned child nodes
  (`ModifierNode`/`ModKind`) riding the decl in source order; the old
  flag fields are gone — flags are DERIVED accessors on the node
  structs (`vis()`/`is_async()`/`is_static()`/`marker()`/`linkage()`/
  `ambient()`), single truth in the tree. Views derive `ambient`/vis
  the same way — nothing stored, nothing stale.
- **The LSP verbatim law**: decl text in hover/completion is a
  `Doc::text(span)` slice on the OWNING document; only source-less
  text (inferred or absent — e.g. inlay hints) is rendered.
  fmt reprints literals as span×src slices too (radix / underscores /
  escapes / hash levels survive) — the fmt lane re-derives nothing
  but parens and layout.
- **The parse entry law**: recovery is NOT a knob — the ENTRY is the
  choice, and both take the source BY VALUE (it moves into the
  artifact). `parse(src, mode) -> Result<Doc, Diag>` is fail-fast
  (`Err` = the first diag; `Ok` implies `is_clean()`);
  `parse_with_recover(src, mode) -> Doc` is ALWAYS a tree — both are
  thin wrappers over `Doc::parse`. There is no opts row: docs ALWAYS
  graft (the enriched `graft_table` rows ride every parse; the grammar
  never consults comments). Production lanes ride the recovering entry
  and pair it with `ast.is_clean()` / `ast.errors()`. The recovery
  ladder: part damage → `Error` slots beside the survivors (every slot
  enum carries `Error(Handle<ErrorNode>)`); identity damage → fences
  (an `ErrorNode` covering the unparseable region, in the list it
  belongs to); lex rows → root-level pre-seeded fences (the doc's
  managed prefix, rebuilt wholesale by the edit lane). `ast.errors()`
  is the flat ordered view of every diag (the panel); the
  positional view is the `ErrorNode`s in the streams.
- **`rut-lsp`'s `doc` module is the only grammar lane**, and its
  parse lane is `Document::parse` (mode from the filename convention
  `mode_of`, editor opts baked — recovering always). The module is
  SEALED (`mod doc;` — a private decl hides even its own `pub`
  items), so the queries spell `crate::doc::…` and no consumer
  reaches past the ABI; `rut-ast` stays as shared tree vocabulary,
  `rut-driver`/`rut-core` are other lanes. The grammar tests drive the
  parser via rut-ast's own unit-test tree (the sanctioned seam, like
  the futures LocalPool); the fail-fast `parse` door exists for "a
  diag means stop" callers — today only rut-ast's own tests — while
  fmt/doc/driver ride the recovering entry. The LSP side pins the
  same grammar through its public door (`tests/corpus.rs` analyzes the
  corpus fixtures envelope-style). (The wasm face
  `rut-lsp-wasm` layers one crate higher: its only language dep is
  `rut-lsp`.) And since the privacy wall the WHOLE crate is sealed
  the way `doc` is: **rut-lsp's public surface is `Lsp` +
  `env::Io`** (`WakeFn` rides beside it — the face implements `Io`
  for `JsEnv` across the crate boundary, and the test mocks
  implement it too); `doc` and every struct are private impl —
  the JSON envelopes are `String`s, so `wire` carries no public type
  either. The face's deps (rut-lsp + the wasm trio) are the layer
  proof: it spells `rut_lsp::{Lsp, env::…}` and nothing else.
- **The one-world law**: every file in the session's world — the
  open doc, the prelude, the walked deps, the workspace pushes — is
  one `File { doc, stamp, open, decls }` in one `World`
  (rut-lsp's `world/` — the store in its `mod.rs`, the per-node views
  in `views/`), keyed by origin; opening adopts the stamp its loader
  laid down, an open file's buffer is the truth for every reader, and
  `Lane::Scratch` files are visible only as themselves. The doors
  answer through typed views over `(doc, AnyHandle)` — a view's
  verbatim text is a span×src join on the owning file's document, its
  line is `doc.line_of`, its `ambient`/vis derives from the modifier
  chain. Chain preferences have ONE home each: `type_decl` the
  mounted-over-embed rule, `functions` the 0/1/many honesty rule
  (a `Vec`, not a row), `member`/`members` the member law,
  `use_exports` the mod-walk gates, `bare` the
  locals+ambient+auto-import merge — and `prefer_bodies` the
  body-over-decl preference. The feature doors (`definition`,
  `references` in `feature/`) consume the world; the plan's
  `get_declaration` is consciously ABSENT — the wire spells no
  declaration request, and `prefer_bodies`' single decision point
  makes the inverted walk a local change when one arrives. The world
  mutates in place per mount/sync/close — rebuilt never.
- **No mirrors**: the language laws rut-lsp lives by have ONE home,
  in the compiler's crates — `rut_lir::check::{vis_crosses, mod_head,
  mod_walk, mod_walk_pub}` (visibility, the mod-tree mount laws) and
  `rut_driver::mods::mount_edges` (the mount core). An LSP copy of
  any of them is forbidden: the adapters in `resolve::mods` carry the
  world's stamped files INTO the pure walks; they never re-spell a
  law beside it.
- **One caret, one answer**: the editor classifies a caret exactly
  once — `resolve::cursor` derives the `Cursor`; `resolve::walk::resolve`
  walks it to declarations answering the typed `resolve::Def`; a
  feature (hover, definition, references) classifies, resolves, and
  renders — it never walks on its own.
- **Downward only**: the crate layers point one way — `lib`/`actor` →
  `session` → `load`/`feature` → `resolve` → `world` → `doc` →
  (`rut-ast`, `rut-lexer`), with `world` allowed to call the shared
  laws in `rut-lir`/`rut-driver`. A lower layer never names a higher
  one:
  ```text
  src/
  ├── lib.rs, env.rs        public surface
  ├── actor/                event loop, defer queue, wire
  ├── session.rs            one method per request
  ├── load/                 dep walk, manifest and lock bridge, prelude
  ├── feature/              analysis, check, hover, definition, references, completion/, signature_help, inlay
  ├── resolve/              cursor, def, walk, scope, exports, member, mods, pkg, ty_infer
  ├── world/                World, File, Stamp, Lane, FileId, DeclMap, views/, ty.rs
  └── doc/                  Document, classifier, symbols, line index
  ```

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
  The playground's STD is async too: the page renders std-less first,
  then fetches the nine std zips from jsDelivr at a pinned commit sha
  into an in-memory store (`demo/src/lsp/std-pins.ts` names the pins)
  — completion/check light up when the deps panel says `mounted`, so
  wait for that row before judging a `use pouch::{` probe.
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
   then the e2e: `rut new → install → run` against a scratch `RUT_HOME`
   (the proof never touches a real store — install fills
   `$RUT_HOME/deps`, run/pack read it cache-only) must print
   `hello, rut!`. Install serializes per project via a kernel-enforced
   lock under `$RUT_HOME/locks/` (10-minute wait, holder named);
   readers never lock; the global store stays lock-free by content
   addressing.

Gate modes (no act; each gate dies loudly on drift): `node
scripts/pack-std.cjs --check` is the offline, CI-shaped gate (repacks to a
temp dir and byte-compares against the committed envelope — no network, no
git writes); `node scripts/publish-std.cjs --dry-run` runs the publish
gates and PRINTS the tag + push without doing them (it still derives the
tag from origin).

History: the `.rutbundle` bin lane is retired and old envelopes are
GONE — the `dist/std-v8`…`std-v16/` dirs and the `dist/std/*.rutbundle`
mirrors were purged. The only committed envelope is the current cut:
`dist/std-vNN/` + the `dist/std/` zip mirror. Old tags serve their own
immutable commits, but this toolchain reads wire 13 only — stale bytes
re-pack, they never load back. The old pack script lives in git history.

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

- **Two crates — core + face**: `crates/rut-lsp` is the queries + actor
  core — the event loop, the session state, the envelope `wire` —
  native-clean AND wasm32-clean, and sealed to `Lsp` + `env::Io` (see
  the privacy wall above): ALL of its tests sit in `crates/rut-lsp/tests/`
  behind the public door — JSON envelopes in and out through `Lsp`,
  driven on the native `LocalPool` (`Lsp::spawn_native`) over an `Io`
  mock, sharing the `tests/common/` harness. There is NO unit-test
  tree (`src/tests/` is gone): an assertion the envelopes cannot
  express does not ship. The one in-crate exception is
  `actor::event_loop`'s own `#[cfg(test)] mod tests` — the NEUTRAL-QUEUE/cancel machinery
  drives the loop's private handles directly (dummy cmds, the defer
  queue), which no envelope can reach.
  `crates/rut-lsp-wasm` is the wasm-bindgen
  face: one exported `RutLsp` class whose every method delegates onto
  the core's `Lsp` handle; its deps are rut-lsp + the wasm trio
  (`wasm-bindgen`/`js-sys`/`wasm-bindgen-futures`) — the dep line is
  the law's proof. Add a feature to the core's queries and wire it
  into the face's `src/lib.rs` ABI methods; the old stdio face lives
  only in git history.
- **The read-only env law (dep lane)**: the extension injects a
  READ-ONLY fs capability (`src/env.ts` — sync `read`/`readDir`/
  `isDir`/`notify` over `node:fs`, plus the optional clock
  `wakeAfter` — plain `setTimeout`, the one machinery addition; no
  write, no delete, no fetch, and no business callback exists on the
  `HostEnv` contract in the crate's `js/index.d.ts` — toward the host
  the actor only ever ANSWERS, there is no event push) and the
  crate's event loop owns ALL dep logic — the walk and its scheduling
  live in the ACTOR (`reloadDeps` posts `Cmd::ReloadDeps`; the clock
  is an injected capability); the extension's whole dep lane is
  watchers + `void rut.reloadDeps(roots).then(() => re-schedule
  analysis of open docs)`. Url deps mount from the global store (`$RUT_HOME` — the
  wasm cannot read env vars, so its store spelling arrives relative
  and the adapter anchors it at the real home); a cold store is ONE
  hint per dep naming `rut install` (the rust-analyzer contract).
  The extension writes NOTHING, anywhere — a pure reader.
- **The three-lane ABI law**: `crates/rut-lsp-wasm`'s public surface
  is exactly three lanes — the provider queries (`analyze`, `hover`,
  `complete`/`signature-help` with src, definition/inlay/references,
  `legend`, `forget`), the workspace-scan push (`indexFile`/
  `indexModAt`/`parseManifest` — the host's `findFiles` respects
  the editor's ignore rules the env cannot), and the dep pull
  (`setEnv` + `reloadDeps`, the deferred door the hosts ride;
  `resolveDeps` stays as the eager manual door — tests, cold boot,
  env-less hosts). No entry may bypass the env capability
  to mutate dep state: the old push family (`setLock`,
  `setDirectDeps`, `addDefNamed`, `addBundle`, `extractBundle`,
  `extractSrczip`, `docLen`, the stored-`complete` variant) is
  deleted; its behaviors live in the walk and its tests. The
  `complete`/`signature-help` entries take the source ALWAYS (the
  doc-sync law — a stored-only read races the analyze debounce;
  signature help fires the instant `(` is typed, so the stored doc
  may still be short the very paren that triggered the request).
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
  lexer, parser, `rut-lsp-wasm`, or the TextMate grammar must run this lane
  (or at minimum `build:wasm` + `test:e2e`) before the work is done —
  a new syntax reads as false diagnostics through a stale wasm, and the
  corpus gate catches exactly that.
- **The gates gate the shipped bytes**: the e2e drives `bin/rut-lsp.wasm`,
  the pack embeds that same file, and the lane byte-compares the copy
  inside the vsix — a stale artifact cannot pass.
- **Style gate: biome, 4-space.** `npm run lint` (`biome check .`,
  config `biome.json`: space/4, width 100, single quotes; `syntaxes/`
  and `schemas/` excluded) fronts `npm test` and the release lane's
  gate list — keep it green with `npm run format`.
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
  not `cargo test` — keep this for every test invocation. A test's
  scratch directory is a `rut_testkit::Scratch`; it dies with the test,
  and no test builds a temp path itself.
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
