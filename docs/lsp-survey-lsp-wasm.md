# LSP + wasm survey — phase(0a) of `rut-lsp-align`

Task A survey, written 2026-09-22 against tree `4aebe24` (master, clean).
Read-only investigation; every behavioral claim below was executed against
the actual artifacts, not inferred from source. Probe scripts live under
`/tmp/opencode/lsp-survey/` (`wasm-probe{1..4}.js`, `probe5.js`, `probe6.js`)
and can be re-run against any wasm module with
`node <probe> <path-to.wasm>`.

---

## 1. Where rut-lsp's grammar lives

**rut-lsp has no grammar of its own.** There is no tree-sitter, no
hand-rolled second parser. `crates/rut-lsp` depends on `rut-lexer`,
`rut-ast`, and `rut-parser` and its pipeline (`analysis.rs::analyze`) is:

```
normalize → lex → rut_parser::parse(mode) → classify / symbols / hover-index
```

The places a second grammar could hide were checked and are clean:

- `semantic/legend.rs` — the keyword and primitive-name tables **delegate
  to the parser's canonical tables** (`rut_parser::is_reserved_kw`,
  `rut_parser::is_primitive_ty`); only the contextual words
  (`self`, `break`, `continue`, `as`, `super`, `default`) are duplicated,
  matching the parser's text-matched set.
- `semantic/names.rs`, `semantic/symbols.rs`, `semantic/recover.rs`,
  `hover/*`, `completion.rs` — all walk the real arena AST. No token-level
  re-parsing of type or expression syntax.
- New-language arms exist: `TyForm::Primitive` (`builtin primitive`,
  d64166f), `TyOpt` in the classifier and in `semantic/symbols.rs`'s
  `ty_text` (`?T` rendering, patched by 24d21f1/9aa63d4), `TyUnion` in
  classifiers.

So the divergence story is **not** "a second grammar rotted". It is:

1. the **shipped wasm binary** embeds a parser frozen at `e41915d`
   (Sep 19 09:52), 40 commits behind — the extension's users run the
   *old language*;
2. **three parallel type-renderers** inside rut-lsp, only two of which got
   the `?T` memo (details in M4);
3. the **embedded std surface** (`std_surface.rs`) covers 3 of 8 packages;
4. the LSP's diagnostic surface is **parser-only** — checker-level removals
   are invisible to LSP users (details in M5).

The extension additionally ships a TextMate grammar
(`integrations/vscode-extension/syntaxes/rut.tmLanguage.json`) — a regex
fallback grammar, owned by task B's survey; not covered here.

### Language-movement timeline vs the LSP/wasm lane

`e41915d` ("lsp: run the language core as in-process wasm", Sep 19
09:52:16) is the **last commit to touch crates/rut-lsp,
crates/rut-lsp-wasm, and integrations/vscode-extension**. Everything below
landed after it; the wasm binary was never rebuilt:

| commit | change | LSP source alignment |
|---|---|---|
| `d64166f` Sep 19 16:14 | `builtin primitive` surface + `opaque` statics | aligned (TyForm::Primitive + LSP arms) |
| `a094965` Sep 19 17:xx | `Opaque` → `opaque` rename sweep | aligned (vocabulary) |
| `24d21f1` Sep 19 18:02 | `?T` surface swap, TyPtr → TyOpt | partially aligned (names.rs/symbols.rs patched) |
| `4fbe8a2` Sep 19 18:xx | by-reference semantics flip | no LSP surface impact |
| `9aa63d4` Sep 19 19:36 | postfix `T?` removed + `bytes.clone()` | aligned (dedicated diag + `ty_text` `?T`) |
| `f3f9972` Sep 20 | checker type-union bounds | parse-level aligned (see M6) |
| `f38a56d`/`0ff0de8` | nmapset typed lanes, PrimMap classes | std surface NOT extended (M3) |
| `758110b` Sep 20 | RFC 0044 | n/a (RFCs aren't LSP inputs) |
| `913434e` Sep 21 | mapset removed from stdlib | no LSP impact (mapset was never in the surface) |

At **HEAD**, the LSP core is largely aligned; the user-visible breakage is
concentrated in the stale binary. Verified at HEAD over the full corpus
(§2): **53/53 files analyze clean** through a fresh build.

---

## 2. The corpus and its ground truth

Corpus = every `*.rut` under `rut/`, `examples/`, `benches/workloads/`,
`demo/src/examples/` — **53 files** (8 + 6 + 26 + 13).

Method: each file fed through `rut_analyze` of (a) the freshly built
`target/wasm32-unknown-unknown/release/rut_lsp_wasm.wasm` at HEAD and (b)
the shipped `integrations/vscode-extension/bin/rut-lsp.wasm` (mtime == the
`e41915d` commit time). URI suffix preserved so `mode_of` picks Decl for
`*.d.rut` (a probe with a fixed `.rut` URI initially mis-moded the `.d.rut`
files — fixed; the first "6 flagged .d.rut" result was probe error, not
fact).

**Ground truth: at HEAD the parser+LSP analyze the entire corpus with zero
diagnostics (53/53 clean).** The shipped binary flags **15 of 53 files,
1,796 false diagnostics** — see M1.

---

## 3. The misalignment list

Each item: the failing example (corpus file:line where the corpus has one),
what the current language says, what the broken artifact does.

### M1 — shipped wasm rejects the `?T` prefix nullable (RFC 0044) — THE headline

- **Language now:** `?T` is the one nullable spelling (prefix).
- **Corpus:** 15 files use it; worst offenders:
  - `examples/02-digest/digest.rut` — **808 false diags**
  - `benches/workloads/json-decode/main.rut` — **374**
  - `rut/nmapset/nmapset.rut` — **196**
  - `examples/01-sort/sort.rut` — **227**, `demo/src/examples/dataclasses.rut` — 31,
    `benches/workloads/binary-trees.rut` — 14, `rut/core/core.d.rut` — 47,
    `rut/pouch/pouch.rut` — 7, and 8 more (full table in §4).
- **Observed (shipped wasm):** `fn f(p: ?i32)` → `expected a type name,
  found '?'` — every nullable in modern rut is a red squiggle.
- **Fresh build:** 0 diags. 

### M2 — shipped wasm lacks the postfix-removal diagnostic

- **Language now:** `T?` diagnoses with the dedicated message
  `"the postfix spelling `T?` was removed — write `?T` (RFC 0044)"`
  (parser test `the_postfix_nullable_spelling_is_removed`).
- **Corpus:** no instance (corpus is clean), exercised by probe.
- **Observed (shipped):** `fn f(p: i32?)` → three generic parse errors
  (`expected ), found '?'`, …) pointing the user the wrong way.
- **Fresh:** the one dedicated RFC 0044 diagnostic.

### M3 — the embedded std surface covers 3 of 8 packages

- `std_surface.rs` embeds `rut/core/core.d.rut`, `rut/calc/calc.d.rut`,
  `rut/pouch/pouch.rut` only. `nmap`, `nmapset`, `ink`, `rt` (and
  `bench-cross`) are absent — `ink`/`rt` predate even `e41915d`, so this
  gap is not merely a consequence of the stale binary.
- **Corpus:** `benches/workloads/nmapset-str/main.rut:3`
  `use nmapset::{ HashMap };`; `benches/workloads/nmap-primmap/main.rut`
  uses the PrimMap lane classes.
- **Observed (clean instance, no workspace index — i.e. what a user
  project gets):** bare completion of `Hash` offers no `HashMap`,
  `HashSet`, `PrimMapI64` (probe6). With the file workspace-indexed the
  fresh module completes all three; the shipped module still drops
  `PrimMapI64`/`HashSet` because its parser dies partway through
  `nmapset.rut` at the first `?T` (M1 compounds M3).
- **Mitigation in-repo:** the extension's fs-walk (`rut_add_def`, 500-file
  cap) indexes workspace files, so developing rut itself masks the gap.
  For any *user* project the std surface is all they get.

### M4 — hover's type renderers missed the `TyOpt` memo (three copies problem)

rut-lsp contains **three parallel type renderers**:

| renderer | file | `TyOpt` arm? | used by |
|---|---|---|---|
| `ty_text` | `semantic/symbols.rs:178` | **yes** (`?{inner}`) | document symbols |
| `ty_src` | `hover/types.rs:114` | **no** (`_ => String::new()`) | alias hover (`build.rs:357`) |
| `ty_head` | `hover/types.rs:103` | **no** (same catch-all) | let-bound inference (`infer.rs:80,96,115`) |

- **Corpus:** none yet — the corpus contains no nullable alias
  (`demo/src/examples/type-aliases.rut` aliases only `i64`/`Meters`);
  disclosed as a language-shape demo, not corpus-grounded.
- **Observed (fresh, i.e. true at HEAD):** `type Maybe = ?i32;` hovers as
  **`type Maybe = ;`** — the target silently vanishes. Likewise
  `let x: ?Circle = …` infers `""` via `ty_head`, so receiver hover on
  `x.` misses.
- **What it takes:** add the `TyOpt { inner }` arm to `ty_src`/`ty_head`
  (or collapse all three renderers into one — the honest fix; the copies
  have already drifted once).

### M5 — parser-only diagnostic surface: checker-level removals are invisible

`analysis.rs` publishes **lexer+parser diags only**. Type paths parse
generically, so removed names produce no LSP diagnostic:

- **Corpus/probe:** `fn f(p: Ptr<i32>) -> nil { }` — **zero diagnostics in
  BOTH binaries**, though `Ptr` was removed from the surface (renamed
  `Opt`/`?T` by 24d21f1). Same class: `mapset.*` (package deleted by
  913434e), `Hashable` bound (deleted by f38a56d).
- **Consequence:** stale-language code gets no squiggles for these; only
  parser-level breaks (like `?T` in the shipped binary) surface.
- **What it takes (phase 2 candidate):** wire `rut-lir`'s checker diags
  into the analysis pipeline, or at minimum a name-aware "unknown type
  path" heuristic. Not a regression — the M6 slice never had it — but it
  bounds what "aligned" can mean.

### M6 — union bounds / PrimMap lanes: aligned at HEAD (verified, not broken)

Explicitly checked because the premise listed them:

- `rut/nmapset/nmapset.rut:183`
  `pub class HashMap<K requires i8 | … | bytes, V> {` and the
  `PrimMapI64/U64/F64` lane classes: parse clean at HEAD; semantic tokens
  classify the bound row perfectly (`requires` → keyword, `Point | i32` →
  type; probe dump). Old binary flags the file via M1 only.
- `bytes.clone()`: completion after `b: bytes.` offers `clone` on the
  fresh build (from the embedded, current `core.d.rut`); **absent on the
  shipped build** (decl postdates it). No corpus instance yet — the corpus
  never calls `.clone()`; disclosed.
- RFC 0042 `.slice()`: predates `e41915d`; present and completing in both
  binaries; corpus use in `benches/workloads/kmer-view/main.rut` and
  `nmap-knucleotide/main.rut`. No misalignment.
- `opaque`: fresh hover renders the current `primitive str/bytes/opaque`
  form; shipped renders the old `builtin str` form; shipped completion on
  `o: opaque.` is empty vs fresh's `downcast`. Corpus use:
  `rut/nmap_host/nmap.d.rut:21` (`pub host fn map_entry(m: opaque, …)`).

### M7 — primitive type names don't hover (pre-existing, both binaries)

Hovering the `i32` of `p: i32` returns NULL in the fresh build too (class-
typed params hover fine, including through `?Point`, `[?Point]`,
`?[Point]` — verified). The int primitives have no surface decl to index.
Minor, but worth a line in the alignment backlog.

---

## 4. The .wasm staleness

### Evidence

- `integrations/vscode-extension/bin/rut-lsp.wasm` mtime **Sep 19 09:52 ==
  `e41915d`'s commit time**; the crate has 40 commits of parser/stdlib
  movement since; the binary was never rebuilt.
- `bin/`, `out/`, `*.vsix` are **gitignored** — the staleness ships
  silently; `git status` never shows it.

### Behavioral proof (shipped vs fresh, same probes)

| probe | shipped (`e41915d` era) | fresh (HEAD) |
|---|---|---|
| `fn f(p: ?i32)` | `expected a type name, found '?'` ×3 | clean |
| `fn f(p: i32?)` | 3 generic parse errors | 1 dedicated RFC 0044 diag |
| corpus (53 files) | **15 flagged, 1,796 diags** | **0 flagged** |
| `bytes.` completion | no `clone` | `clone` |
| `opaque.` completion | `[]` | `["downcast"]` |
| hover `str` | ``builtin str { … }`` (old form) | ``primitive str { … }`` |
| alias `type Maybe = ?i32` | NULL | **`type Maybe = ;`** (M4 — still broken at HEAD) |
| smoke.js | passes (its own snippets predate `?T`) | **passes** |

Per-file shipped damage (diag counts): digest.rut 808, json-decode 374,
sort.rut 227, nmapset.rut 196, core.d.rut 47, dataclasses 31, custom_async
18, node-cycle 17, binary-trees 14, classes 13, weak-cache 20, tree 14,
todolist 4, plugin.rut 6, pouch.rut 7.

### What fixing it takes

**Nothing is broken in the build pipeline — it just hasn't been run.**
Verified at HEAD:

- `cargo build -p rut-lsp-wasm --target wasm32-unknown-unknown` — clean
  (debug **and** `--release`; the wasm32 dependency graph compiles from
  scratch fine).
- `node crates/rut-lsp-wasm/smoke.js` — **ALL CHECKS PASSED** on the fresh
  module.
- ABI unchanged since `e41915d`: `wasm.ts` calls
  `rut_begin/alloc/legend/analyze/forget/hover/complete/add_def` — exactly
  the fresh module's exports. **Zero TS changes needed; the fix is:**

```
cd integrations/vscode-extension && npm run build:wasm   # cargo --release + copy-wasm.mjs
npm run package                                          # re-issue the .vsix
```

plus re-running the probe suite above as acceptance (corpus must go
53/53 clean through the new `bin/rut-lsp.wasm`).

---

## 5. The wasm32 cross-build break

`cargo check --workspace --target wasm32-unknown-unknown` **fails** — but
**not** in the LSP lane:

- **One hard error:** `rut-vm-threaded` E0423 at
  `crates/rut-vm-threaded/src/wasm/mod.rs:11` —
  `Table(core::marker::PhantomData)` cannot construct
  `pub struct Table<M: Machine>(core::marker::PhantomData<fn() -> M>)`
  (`api.rs:435`) because the **field is private to module `api`** and
  `wasm::build_table` is a sibling module.
- **Root cause of the target-gating:** `build.rs` sets
  `cfg(rut_threaded)` only for `x86_64`/`aarch64` non-wasm targets (wasm
  deliberately runs the portable loop backend — JITs lower the threaded
  shape 1.2–4.6× slower). Under `cfg(not(rut_threaded))` the portable
  backend compiles — and has been broken since its introduction in
  `5d9da24` (Sep 13), with the cfg split from `bd6e83d` (Sep 17). Host
  gates never see it because on host `rut_threaded` is set and the module
  is compiled out. The latent break is 9 days old, not new.
- **Impact on the LSP pipeline: none.** `rut-lsp-wasm` does not depend on
  `rut-vm-threaded`; `-p rut-lsp-wasm` cross-builds clean (§4). The break
  only blocks whole-workspace wasm32 builds/checks.
- Remaining wasm32 workspace noise: unused-import warnings in `rut-lir`
  (cosmetic).
- **What fixing takes:** one line — make the field `pub(crate)` (or add a
  `Table::new()` ctor in `api.rs`). `rut-vm-threaded` is outside this
  task's scope; recorded here for phases 1+2.

---

## 6. Corpus-pass harness proposal (the cheap honest gate)

Two cargo tests already exist; both stop short:

- `rut-parser/tests/corpus.rs::corpus_parses_clean` — asserts **zero
  diags** (the right assertion) but walks only `examples/` +
  `demo/src/examples/`.
- `rut-lsp/tests/corpus.rs::corpus_classifies` — walks the same two roots
  and **discards parse diagnostics** (`let (ast, _) = rut_parser::parse(…)`);
  it checks classification invariants only. The LSP's own gate would pass
  a corpus full of red squiggles.

**Proposal (phase-1 gate):**

1. Widen both tests' `roots` arrays to the four trees:
   `examples/`, `demo/src/examples/`, `rut/`, `benches/workloads/`
   (53 files today).
2. In `rut-lsp/tests/corpus.rs`, add
   `corpus_parses_clean_lsp()` mirroring the parser's assertion
   (`assert!(diags.is_empty())` per file, mode via the `.d.rut` suffix) —
   the missing honest "every corpus file parses clean" gate.
3. Raise the size floors (`>= 15` → `>= 50`).
4. No new infrastructure, no node dependency — it rides the standard
   workspace suite (~ms/file; the whole gate is noise next to the VM
   suites). Keep the wasm probes (`/tmp/opencode/lsp-survey/`) as the
   dev-side acceptance for the *rebuilt binary* (they exercise the exact
   artifact users run, which a cargo test cannot).

---

## 7. Foreign-work census (lane: vscode-extension / wasm / rut-lsp / rfc)

`git status` clean for tracked files; the census is about non-tracked /
non-commit state. Nothing was stashed, reverted, or touched.

1. **Stash `stash@{0}`** — *"batch-plan-impl: auto-stash 2026-09-19
   (rut-lsp changes)"*. Touches `Cargo.lock`, `crates/rut-lsp/Cargo.toml`
   (+features/`ls-types`), `crates/rut-lsp/src/lib.rs` (+`std_surface`,
   `cfg(server)` gate). **Verdict: fully superseded** — `git diff
   stash@{0} HEAD` on those paths shows the stash's target content is
   already in HEAD via `e41915d`; the only residual difference is a
   two-line comment reword. Looks like a leftover auto-stash from the
   session that produced `e41915d`. Safe to drop, but that's the
   orchestrator's call — left untouched.
2. **Extension lane untracked-but-ignored artifacts** (all consistent with
   the `e41915d` build session of Sep 19 09:35–09:52; nothing newer):
   - `integrations/vscode-extension/bin/rut-lsp.wasm` (531,964 B, the
     stale module — the subject of §4),
   - `integrations/vscode-extension/out/` (esbuild output),
   - `integrations/vscode-extension/rut-vscode-0.2.0.vsix` (189 KB).
   Tracked sources (`src/extension.ts`, `src/wasm.ts`, `package.json`,
   …) are identical to HEAD; mtimes match the `e41915d` session. **No
   active uncommitted foreign work in the lane.**
3. **Worktree `/tmp/opencode/ab2`** (branch `ab2` @ `74923a4`, Sep 14,
   "vm: host-constructed Opaque boxes") — git reports it *prunable*
   (directory gone). `74923a4` **is an ancestor of HEAD**, so the branch
   holds no unique work; pruning is safe whenever the orchestrator
   wishes.

---

## 8. Deviations and method notes

- No files outside `docs/lsp-survey-lsp-wasm.md` were created or modified;
  probes live under `/tmp/opencode/lsp-survey/`.
- Building (`cargo build/check`, both targets) writes only to `target/`;
  the shipped binary and extension tree were read, never overwritten (the
  fresh wasm was built to `target/…`, **not** copied into the extension).
- The task premise of a "wasm32 cross-build break" resolved into two
  separate facts, both reported: the LSP pipeline builds fine (the
  premise's implied break does not reproduce), while the *workspace-wide*
  wasm32 check breaks in `rut-vm-threaded` (real, latent since Sep 13).
- The premise that rut-lsp contains a diverged grammar resolved to: it
  doesn't — the divergence is a stale *binary*, a std-surface coverage
  lag, and a three-copies rendering drift (M4). Said plainly so phases
  1–2 budget accordingly.
- Gate: `cargo test --workspace` → 77 suite results, 474 passed, 0
  failed, exit 0 (PIPESTATUS-verified) immediately before the commit.

---

## 9. Phase-1 fix log (`rut-lsp-align` task 1, 2026-09-22)

Every M-item above that phase 1 owns is closed; tree was `c066fbf`
(post-rename: `nmap_host`), one commit, rust+docs+tests only.

### M1 + M2 + §4 — the shipped wasm: REBUILT, REPACKAGED

`npm run build:wasm` (cargo `--release` → `copy-wasm.mjs`) and
`npm run package` re-issued `bin/rut-lsp.wasm` (568,271 B, was
531,964 B at `e41915d` era) and `rut-vscode-0.2.0.vsix`. Acceptance
(`/tmp/opencode/rut-lsp-align/acceptance.js`, the survey §2 method):
**corpus 53/53 clean through the shipped `bin/rut-lsp.wasm`** (was
15 flagged / 1,796 diags); `fn f(p: ?i32)` clean (M1); `fn f(p: i32?)`
→ exactly one dedicated RFC 0044 diag (M2); `primitive str` hover;
`opaque.downcast` + `bytes.clone` complete; `node smoke.js` — ALL
CHECKS PASSED. ABI untouched (`wasm.ts` calls the same 8 exports).

### M4 — TyOpt hover arm: CLOSED (both missing renderers)

- `hover/types.rs` `ty_src`: `TyOpt { inner } => format!("?{}", …)` —
  `type Maybe = ?i32;` hovers `type Maybe = ?i32;` (was
  `type Maybe = ;`).
- `hover/types.rs` `ty_head`: recurses into the payload — `let c: ?Circle`
  now resolves member hover on `c.` (was `""`). The third renderer
  (`symbols.rs` `ty_text`) already had the memo; all three now agree.
- Regression tests in `hover/tests.rs` (`nullable_alias_hover_renders_
  the_target`, `nullable_let_binding_resolves_members`). The honest
  collapse-into-one-renderer refactor stays backlog (no behavior left
  on the table).

### M3 — std surface: ALL EIGHT packages embedded

`std_surface.rs` now embeds `core`, `calc`, `nmap_host`, `rt`,
`bench_cross` (Decl mode) + `pouch`, `nmapset`, `ink` (Impl mode,
`entry.lib` shape) — labels are the `rut.toml` `name` fields, i.e. what
`use` paths spell. Unit tests pin the count and the M3 acceptance
exactly: bare completion (no workspace index) offers `HashMap`,
`HashSet`, `PrimMapI64/U64/F64`, and `nmap_host`'s `map_entry`.

### §5 — the wasm32 workspace check: PASSES (two blockers, not one)

- The surveyed E0423: `api.rs`'s `cfg(not(rut_threaded))` `Table`
  tuple field is now `pub(crate)` — the one-liner, wasm32-target-only
  (the host never compiles that arm; bench guard below).
- One the survey missed (probably shadowed by the E0423 abort):
  **tokio 1.53.1 `compile_error!`s on wasm32** for any feature outside
  `sync,macros,io-util,rt,time` — rut-lsp's default `server` face
  requests `io-std`, so `cargo check --workspace --target
  wasm32-unknown-unknown` still died after the one-liner. Fix: the
  server face's crates (`tower-lsp-server`, `tokio`) moved under
  `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`, the
  `server` module gated `all(feature = "server", not(target_arch =
  "wasm32"))`, and `main.rs` grows a wasm32 stub `main` (the stdio
  server is meaningless there — no stdin; wasm users get `rut-lsp-wasm`).
  Host builds/tests unchanged (same deps, same features, native).
- Gate: `cargo check --workspace --target wasm32-unknown-unknown` →
  Finished, exit 0 (only the known cosmetic warnings).

### §6 — the corpus gate: WIDENED as proposed

- `rut-parser/tests/corpus.rs::corpus_parses_clean`: roots now the four
  trees (`examples/`, `demo/src/examples/`, `rut/`,
  `benches/workloads/`), floor `>= 50`.
- `rut-lsp/tests/corpus.rs`: same roots/floor, plus the missing honest
  assertion — new `corpus_parses_clean_lsp` (the old `analyzed` helper
  discards diags) asserts ZERO parse diagnostics per file, `.d.rut` →
  Decl mode. No new infrastructure; 53 files ride the standard suite.
- Workspace gate: `cargo test --workspace` → 77 suites, **479 passed**
  (474 + 5 new), 0 failed, exit 0.

### Bench guard (the E0423 fix is wasm32-only — proven)

`benches/run.mjs --runtime rut` over five pinned rows; fuel and
VM-heap high-water **bit-identical** to the phase-2/3 records
(`crossing-nop` 104000032/236, `json-decode` 111330118/34377147,
`kmer-view` 37614177/4194916, `nmapset-int` 20703284/1966551,
`sieve` 19592209); checksums agree with `expected.json` (`refOk` on
every row that carries one; kmer-view has none by design — its
2198604 parity with knuc holds). The host never compiles the patched
arm, and the pins prove it.

### Scope notes

- `vscode-extension/` tracked sources untouched by task 1 — only the
  gitignored artifacts (`bin/`, `out/`, `.vsix`) were rebuilt via the
  sanctioned npm scripts (task 2 owns the lane; its in-flight edits
  were present and left alone).
- M5 (checker-level diags) and M7 (primitive hovers) remain open by
  design — phase-2 candidates, unchanged.
