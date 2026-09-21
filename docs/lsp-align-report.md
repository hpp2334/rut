# rut-lsp-align — the batch report

Batch `rut-lsp-align`, phases 0–3, closed 2026-09-22. Base `9d78c10`
(phase 2), plus the skills-doc commit `860c235` riding in the same push.
Lane: `crates/rut-lsp*`, `integrations/vscode-extension`, the shipped
wasm artifact, `rfc/`, `docs/`. Engine semantics, `expected.json`, and
language surface: untouched all batch.

## 1. The premise, and what it dissolved

The batch started from "the LSP has a second grammar that rotted away
from the language". **Phase 0 dissolved the premise.** rut-lsp has no
second grammar — its pipeline is `normalize → lex → parse → classify`,
the classifier tables delegate to the parser's canonical
`RESERVED_KW`/`is_primitive_ty`, and every hover/completion walk walks
the real arena AST (`docs/lsp-survey-lsp-wasm.md` §1). The real
divergence was three smaller, fixable things:

1. a **stale shipped binary** — `bin/rut-lsp.wasm` frozen at `e41915d`
   (Sep 19, 40 commits of language movement behind, gitignored so
   `git status` never showed it): 15/53 corpus files flagged, **1,796
   false diagnostics**, every `?T` nullable a red squiggle ("expected a
   type name, found '?'");
2. three **parallel type renderers** in rut-lsp, two of which missed
   the `TyOpt` memo (`type Maybe = ?i32;` hovered as `type Maybe = ;`);
3. an **embedded std surface** covering 3 of 8 packages — a user
   project got zero `HashMap`/`HashSet`/`PrimMapI64` completion.

The extension-side survey (`docs/lsp-survey-extension.md`) inventoried
the TextMate grammar's eight misalignments (M1–M8: dead `dataclass`/
`where`/`string`, missing `struct`/`str`/`bytes`/`opaque`, no nullable
`?` affordance, uncolored contextual `type`/`builtin`).

## 2. What each phase landed

- **Phase 0** (`5c549a8`, `f55e248`) — the two surveys: corpus-proven
  misalignment lists, the fix designs, and two finds beyond the premise
  (the `rut-vm-threaded` wasm32 E0423 latent since Sep 13; tokio 1.53
  `compile_error!`s on wasm32 hiding behind it).
- **Phase 1** (`4b05618`) — the language lane: the shipped wasm
  REBUILT at HEAD (corpus 53/53 clean *through the shipped artifact*,
  ABI untouched); both missing `TyOpt` arms closed (`ty_src`, `ty_head`
  + 2 regression tests); std surface 8/8 (all eight packages, unit-
  pinned); the wasm32 workspace check PASSES (the surveyed one-liner
  made `pub(crate)`, plus the tokio deps move under
  `cfg(not(target_arch = "wasm32"))`); corpus gates widened to the four
  trees with the missing `corpus_parses_clean_lsp` zero-diagnostics
  assertion; 479 tests green.
- **Phase 2** (`9d78c10`) — the extension lane: grammar M1–M8 fixed in
  `syntaxes/rut.tmLanguage.json` (corpus-proven before/after spans in
  §7 of the extension survey); the standalone TextMate corpus gate
  (`test/grammar-corpus.js`, vscode-textmate/oniguruma, 53 files /
  49,053 tokens + 17 smoke assertions); `npm test` = grammar + host
  with the host launcher skipping loudly on boxes without `code`;
  version 0.2.1. Fixtures + the through-wasm gate explicitly deferred
  to phase 3.
- **Phase 3** (this commit) — the deferred e2e + the report:
  - `test/e2e-wasm.js`: the SHIPPED `bin/rut-lsp.wasm` driven over the
    full corpus (rut/ + examples/ + demo/src/examples/ +
    benches/workloads/, 53 files) through the extension's OWN binding
    (`src/wasm.ts` bundled standalone to `out/wasm.js` by a second
    esbuild entry) — **zero false diagnostics, 303 document symbols**,
    plus the survey's hover/completion smoke: the `?T` alias hover
    (`type Maybe = ?i32;` renders its target), `primitive str { … }`
    hover, member hover AND completion through a `?Circle` let-binding,
    bare completion of nmapset's `HashMap`/`HashSet`/`PrimMapI64` +
    nmap_host's `map_entry`, and the RFC 0044 one-dedicated-diagnostic
    law. Loud-fail on a missing artifact (`npm run build:wasm`) — the
    gate guards the shipped binary, it never silently skips it.
  - `test/fixtures/symbols.rut` refreshed to the current grammar
    (struct + `?T` field, `type Maybe` alias, `str`/`bytes`, union
    bound, `opaque`/`downcast`, impl-block methods) with the five
    host-asserted symbols intact.
  - `npm test` = grammar → compile → e2e → host (loud-skip without
    `code`). Fix-log: `docs/lsp-survey-extension.md` §8.

The alignment lock now exists at three independent layers, all over the
same 53-file corpus: the Rust suite (parser + LSP, zero-diagnostics),
the TextMate grammar gate, and the through-wasm e2e gate against the
exact binary users run. The wasm module and the native server share
`crates/rut-lsp` queries, so the faces cannot drift without this suite
going red.

## 3. The shared-index race, and the repair

Phases 1+2 ran as parallel sessions in ONE worktree sharing ONE git
index; twice, one task's staged files surfaced in the other's commit
window. The repair that held for phases 2–3: **stage by explicit path
only** (`git add <paths>`), treat any staged-but-uncommitted foreign
file as untouchable (never stash, revert, or commit another session's
work), and when a split does happen, repair by rebase of OWN commits
only. The durable record is in the batch skill
(`860c235`: shared-index hazard rules — explicit-path staging,
foreign-staged awareness, split+rebase repair protocol); this batch's
operational rules inherit it. Phase 3 staged by explicit path and
committed only the seven files listed in §7.

## 4. Verification (all at the phase-3 commit)

- `cargo test --workspace`: 77 suites, **479 passed, 0 failed**.
- `cargo check --workspace --target wasm32-unknown-unknown`: Finished,
  exit 0 (known cosmetic warnings only).
- Full bench sanity, all 26 workloads × rut/qjs/node: exit 0, every
  checksum-carrying row agrees with `expected.json` (70 comparisons),
  cross-runtime agreement holds, and the probe rows are **bit-identical
  to the standing pins** — knuc 38814389/4195084, nmapset-str
  9551761/983620, nmapset-int 20703284/1966551, nmap-primmap
  17950301/324, nmap-hashset 13267176/551, json-decode 111330118/
  34377147, crossing-nop 104000032/236, alloc 22000020/228,
  sieve 19592209, kmer-view 37614177/4194916, strview 10801744/2032173
  (fuel/VM-heap peak; sieve's heap was never pinned). Phases 1+2
  touched build cfg — the host paths did not move. (kmer-view
  deliberately carries no `expected.json` line; its 2198604 parity
  with knuc is its gate. The `hashmap-int 63758210` pin died with its
  row in the pre-batch mapset removal `913434e` and is not expected.)
- `npm test` on this box: grammar-corpus PASS (53 files, 49,053 tokens,
  17 smoke assertions), e2e-wasm PASS (53 files, 0 false diagnostics,
  303 symbols, 7 smoke assertion groups through `bin/rut-lsp.wasm`),
  host skipped loudly (no `code` binary — see §5).

## 5. Honest limitations

- **No `code` in this environment.** The Extension Host suite (its five
  UI-provider checks) cannot run here and skipped loudly — that is a
  skip, not a pass. The e2e gate drives the same shipped wasm through
  the same binding the host would (minus the vscode layer), so the
  artifact is fully covered; what is NOT covered headless is the vscode
  provider glue (`registerSemanticTokens`'s delta decoding, the debounce
  path, `findFiles` indexing). On a machine with `code`, `npm test`
  runs all three tiers.
- `bin/rut-lsp.wasm`, `out/`, and the `.vsix` are **gitignored build
  artifacts**: a fresh clone must run `npm run build:wasm` +
  `npm run compile` before `npm test` is meaningful. The e2e gate fails
  loudly with those exact commands rather than skipping — an honest
  red, not a false green.
- The LSP diagnostic surface is still **parser-only** (survey M5):
  checker-level removals (`Ptr<i32>`, `mapset.*`, deleted bounds)
  produce no squiggles in either face. This batch aligned the language
  that IS served; it did not widen what can be served.
- The three type renderers agree behaviorally now, but remain three
  copies (the collapse refactor stays backlog).

## 6. The menu (re-based, cheapest first)

1. **M5 — checker-diagnostic surfacing**: wire `rut-lir`'s checker
   diags into the LSP analysis pipeline (or a name-aware
   "unknown type path" heuristic) so removed-surface code finally gets
   squiggles. The largest honest gap left in "aligned".
2. **M7 — primitive hovers**: `i32`/`str`-as-type-position hovers are
   NULL for the int primitives (no surface decl to index); `str`/
   `bytes` resolve via the embedded decls, ints have nothing to point
   at. Small, self-contained.
3. **Host-suite CI**: run `npm test` on a runner with `code` so the
   five UI-provider checks (and the three-tier suite as a whole) gate
   pushes instead of relying on developer boxes.
4. (carried from the surveys) collapse `ty_text`/`ty_src`/`ty_head`
   into one renderer; `opaque` param hover (returns NULL today even on
   the aligned binary).

## 7. Artifact map (phase 3's own commit)

- `integrations/vscode-extension/test/e2e-wasm.js` — the through-wasm
  e2e gate (new)
- `integrations/vscode-extension/test/fixtures/symbols.rut` — the
  current-grammar fixture refresh
- `integrations/vscode-extension/esbuild.mjs` — second entry:
  `src/wasm.ts` → `out/wasm.js` (the gate drives the real binding)
- `integrations/vscode-extension/package.json` — `test` = grammar +
  compile + e2e + host; `test:e2e` script
- `integrations/vscode-extension/README.md` — the three-tier Tests
  section
- `docs/lsp-survey-extension.md` — §8, the phase-3 fix log
- `docs/lsp-align-report.md` — this report
