# demo-no-sidecars — phase 0: survey

- **Batch:** demo-no-sidecars (phase 0 of 2, docs-only)
- **Base:** b44c3cf (`phase(3)`: the todolist-restructure close-out)
- **Date:** 2026-09-24
- **Directive (the law):** *"./demo should remove all `*.expected`
  files."* The **verifier survives** — the demo-real-run batch's whole
  point (real runs diffed against ground truth, the chip + diff UI, the
  smoke asserting every case real+verified) is untouched; only the
  **STORAGE** of the expected values moves.
- **Target (recommended and pinned here):** expectations **inline in the
  case definitions** — each case's expected lines live as data next to
  its source, one source of truth per case.
- **Method note:** empirical. Both demo gates were **run at base**
  (§Appendix): the headless smoke — **272 passed, 0 failed** — and the
  native classics gate `cargo test -p rut-cli --test playground` —
  green. Every consumer below was traced by grep over the whole repo
  (extension, integrations, scripts, CI) and read, not assumed. Each
  sidecar's byte count and md5 are recorded here so phase 1's migration
  is **provable verbatim**, not asserted.

---

## 1. The census

### 1.1 The corpus — 17 files, all in `demo/src/examples/`, all committed

`find demo -name '*.expected'` = exactly these 17 (untracked tree
included; zero strays anywhere else under demo/, **including
node_modules** — checked). None is generated; all are committed,
hand-maintained ground truth whose honesty is **enforced by the native
gate** (`playground.rs` really runs each `.rut` and diffs — the files
are "the program's actual output, not a hand-written promise", the
classics header's own law). The bundled copies inside the **untracked**
build outputs (`demo/dist/main.js`, `demo/dist-smoke/rut-api.cjs`) are
rspack `asset/source` inlines — rebuilt artifacts, never migrated.

| # | file | case id | lane | lines | bytes | md5 |
|---|---|---|---|---|---|---|
| 1 | `bytes.expected` | `ex-bytes` | gap-filler | 5 | 121 | `ea1f7e63be616e90138ea399c22370aa` |
| 2 | `checked-arith.expected` | `ex-checked-arith` | gap-filler | 7 | 142 | `f055ef2015c4cf1f68c8240933d3f715` |
| 3 | `classes.expected` | `ex-classes` | classic | 1 | 16 | `e2c6a03ae198bcf6cf48030044b18537` |
| 4 | `closures-generics.expected` | `ex-closures-generics` | classic | 2 | 42 | `6135a8d22d551805ff9434e1d970b81c` |
| 5 | `literals.expected` | `ex-literals` | classic | 1 | 53 | `d0907abd5e9cceba0a0b2299587de071` |
| 6 | `maps.expected` | `ex-maps` | gap-filler | 9 | 171 | `0007d642f5ca2be1d70341642454ac29` |
| 7 | `matrix-mul.expected` | `ex-matrix-mul` | classic | 1 | 24 | `970b0b4074e65492378774506f2f54b3` |
| 8 | `node-cycle.expected` | `ex-node-cycle` | classic | 1 | 22 | `7d47a8e331618d71675b47b7af871883` |
| 9 | `opaque.expected` | `ex-opaque` | classic | 10 | 174 | `e1ea46cb1fb8bd264f4035ab8fa79895` |
| 10 | `quicksort.expected` | `ex-quicksort` | classic | 1 | 25 | `1498ba055d3679fa32b4bb4c04ed6d81` |
| 11 | `sieve.expected` | `ex-sieve` | classic | 1 | 29 | `b991fec91e488d12427091be9b27be9a` |
| 12 | `str-views.expected` | `ex-str-views` | gap-filler | 5 | 101 | `a1f782eaa37ed4506f91dd47906a1f24` |
| 13 | `structs.expected` | `ex-structs` | classic | 2 | 79 | `5c6e8283672f4240ce53d3f9cc45659c` |
| 14 | `tree.expected` | `ex-tree` | classic | 1 | 9 | `a6ae0aca22984d89aa938092a5528a0e` |
| 15 | `type-aliases.expected` | `ex-type-aliases` | classic | 1 | 46 | `c441f99683486615c8beac62aa3e222a` |
| 16 | `weak-cache.expected` | `ex-weak-cache` | classic | 2 | 28 | `b49758325f539d39eeafa15bc8854572` |
| 17 | `when.expected` | `ex-when` | classic | 1 | 6 | `d15dbfcb847653913855e21370d83af1` |

51 lines, 1,088 bytes total. Byte-level audit of all 17: every file
ends `\n`; **zero** CRs, **zero** backticks, **zero** `${`, **zero**
backslashes (template-literal-safe with **no escapes at all**); exactly
**one trailing-whitespace line in the whole corpus —
`quicksort.expected`'s only line ends with a space**
(`sorted: 1 2 2 3 5 7 8 9 `). That space is **load-bearing data** (the
engine emits it; the smoke diffs it); the migration must preserve it
and the md5 proof below is how.

### 1.2 The lanes (25 cases, two storage forms today)

| lane | count | where the source lives | where the expected lives |
|---|---|---|---|
| inline-string cases | 8 | `src/cases.ts` (TS string-array `.join("\n")`) | **already inline**: `expected: string[]` in the same entry |
| classic-file cases | 13 | `src/examples/*.rut` (real files, imported raw) | `src/examples/*.expected` sidecars (imported raw) |
| gap-fillers | 4 | `src/examples/*.rut` | `src/examples/*.expected` sidecars |

- The **8 inline** (`hello-format`, `values-and-pointers`, `opaque`,
  `sieve`, `when-exhaustive`, `tuple-errors`,
  `closures-generics`, `fuel-demo`) have **no sidecar files and never
  did** — `expected` has been data in `cases.ts` since the sidecar flip
  (02937e6). Phase 1 does not touch a byte of their data.
- The **13 classics** date to 142d378 (`type-aliases` added by 2696642;
  `dataclasses`→`structs` renamed in place by 3210668).
- The **4 gap-fillers** (`bytes`, `checked-arith`, `maps`,
  `str-views`) landed with "real sidecars" in 3210668. Same storage
  shape as the classics; same treatment.
- **The two history-sanctioned expected moves** — `values-and-pointers`
  and `fuel-demo`, both re-recorded by 02937e6 with the old values
  recorded verbatim in that commit body (the dead-regime 4-line pointer
  story; the ~40M-fuel `tick 1000000/2000000/3000000/...` fabrication)
  — are **exactly the inline arrays in `cases.ts` today**
  (`q.x=4 p.x=4` / `q==same false, p==same false` / `rp.x=9 p.x=9` /
  `rp==rp true, rp==rq false`; and the single `Trap::OutOfFuel` line
  pinning the DEFAULT budget). They are already in the target shape;
  **their values carry into whatever lands byte-identically** — the
  smoke's §2/§3 pins them today and will pin them after.

### 1.3 Every consumer (grep-swept over the whole repo)

| consumer | how it touches sidecars | phase 1 |
|---|---|---|
| `src/examples/index.ts` | **the only demo source**: 17 raw imports (`import x from "./x.expected"`) + `lines()` (strips trailing `\n`s, splits) | imports die; expected goes inline (§2) |
| `crates/rut-cli/tests/playground.rs` | **HARD consumer, outside demo/**: reads each `*.rut`'s `with_extension("expected")` **from disk**; a missing sidecar is a **panic** (L35–36). Its corpus pin (`files.len() >= 12`) counts `.rut` files only and survives; the sidecar read does not | migrate to an inline Rust table (§2.4) |
| `scripts/smoke.mjs` | **zero direct reads** — asserts on `c.expected` from the bundle (§2: all 25 real runs; §3: `fuel.expected` by-design diff). BUT §5's `files.length >= 40` counts `demo/src` = **52 today → 35 after** −17: the pin **goes red unless re-pinned** | labels say "sidecar" (cosmetic); **pin 40→35**; gains the §7 gate (§3) |
| `rspack.config.ts` (L50), `rspack.smoke.config.ts` (L38) | `asset/source` rule `test: /\.(rut|expected)$/` — what makes the imports strings | narrowed to `\.rut$` — a leftover `.expected` import then **fails the build loudly** |
| `src/examples/modules.d.ts` | `declare module "*.expected"` ambient block | block + its comment deleted (tsc then rejects any leftover import too) |
| `src/verify.ts` | takes `expected: string[]` as a **parameter** — fully storage-agnostic | **no change** |
| `src/runner.ts` | never sees expected (compiles/runs only) | **no change** |
| `src/cases.ts` | 8 entries already carry expected as data; header comment says "`expected` is the case's SIDECAR" | data untouched; header wording only |
| `App.tsx` (L189), `StatusBar.tsx` (L72 chip), `Panes.tsx` (L65–73 diff) | consume `currentCase.expected` / `DiffRow`s — storage-agnostic | **no functional change** (`⟨no sidecar line⟩` wording optional) |
| `README.md` | documents the sidecar story (L36, 65, 90, 103–105, 132–133) | rewritten to inline-expected |
| `src/smoke/smoke-entry.ts` | re-exports `CASES`/`EXAMPLES`; comment says "raw sidecar imports" | comment only |
| `.gitignore` / CI | **no** `.expected` rules; **no** CI workflows exist (no `.github/workflows`) | nothing |
| everything else | grep over `integrations/`, `vscode-extension/`, `scripts/`, `benches/`, `examples/`: **zero** demo-sidecar references | nothing |

## 2. The target shape (pinned)

### 2.1 The field: `RutCase.expected: string[]` — unchanged

The interface is already the target: expected lines as data. Nothing
about the contract moves; only the classics' **source of that data**
migrates from sidecar files into the case definitions. One source of
truth per case: `cases.ts` entries carry theirs in the entry; `EXAMPLES`
entries carry theirs in the entry.

### 2.2 The classics go inline in `examples/index.ts` — template literals

`lines()` keeps its job (normalize + split) and gains the leading-newline
strip the template form needs:

```ts
/** the sidecars' exact shape: one leading newline (after the backtick)
 *  and trailing newlines are the template's delimiters, then line-split */
function lines(block: string): string[] {
  return block.replace(/^\n/, "").replace(/\n+$/, "").split("\n");
}
```

Each entry carries its block **in the entry** (spatially obvious
one-source-of-truth), content at **column 0** — no indentation inside
the block, because indenting would *add bytes to the data* (drift by
typography; a comment at the first block says why):

```ts
{
  id: "ex-sieve",
  name: "sieve",
  blurb: "Sieve of Eratosthenes — flat Vec<u8>/Vec<i32> primitive buffers",
  rfcs: "0005",
  source: sieveSrc,
  expected: lines(`
25 primes up to 100, last=97
`),
},
```

Why template literals over the array-of-strings form the 8 inline cases
use: the block is the sidecar's bytes **pasted verbatim** (review diffs
show the real lines; quicksort's trailing space stays *visible* instead
of hiding inside quotes as `"sorted: 1 2 2 3 5 7 8 9 "`); the audit
found **zero** escaping hazards (no backtick/`${`/backslash anywhere in
the corpus — §1.1), so no block needs an escape. The 8 inline cases
keep their array form — their data is untouched, and converting it is
gratuitous churn the batch does not need.

**Verbatim proof mechanism:** this survey records each file's md5
(§1.1). Phase 1's proof: for every case, the `lines()`-normalized
inline block must reproduce the retired file's bytes exactly (the
normalization strips the trailing `\n` every file has, splits, and is
what `verifyAgainstExpected` has always compared against). The smoke
re-proves it behaviorally — 25 real runs diffed against the inline
data, green — but the md5 table is the byte-level receipt.

**Alternatives considered, on record:**
- **Computed expectations** (re-derive expected instead of storing):
  REJECTED — computing a program's output without running it means a
  second implementation of the engine; the values ARE the recorded
  ground truth of real runs.
- **Dropping verification** (just run, don't diff): REJECTED — it
  undoes the real-run law, the demo-real-run batch's whole point. The
  verifier survives; that is the directive's own premise.
- **Renamed sidecars** (`.out`, `.txt`, a JSON dir): REJECTED — a
  loophole around a filename law; the gate (§3) is filename-based on
  purpose.
- **Array-of-strings for the classics**: allowed but rejected (§2.2 —
  invisible trailing whitespace, escape noise, per-line quoting churn).

### 2.3 The smoke's assertion-source swap

There isn't one — **by design**. The smoke already asserts on
`c.expected` from the bundle for all 25 cases (§2) and on
`fuel.expected` for the by-design resume diff (§3). When `EXAMPLES`
starts carrying inline data, the smoke reads it unchanged. Phase 1's
smoke edits are: the §2/§3/header wording ("sidecar" → the inline
expected), the §5 file-count pin **40 → 35** (with the comment: the
corpus shrank by the 17 retired sidecars — 52 files under `demo/src`
at base, 35 after), and the new §7 gate (§3). `smoke-entry.ts` gets the
matching comment fix. The 272-check count may drift by the new §7
check; the count is not itself pinned anywhere.

### 2.4 `playground.rs` — the one consumer outside demo/ (pinned)

The native classics gate keeps its law intact — compile + run + **byte
diff** — and takes its ground truth from an **inline Rust table**,
values carried verbatim (same md5 receipts):

```rust
/// the classics' expected outputs — carried VERBATIM from the retired
/// `demo/src/examples/*.expected` sidecars (md5 receipts in
/// docs/demo-no-sidecars-survey.md §1.1); enforced here natively and
/// by the demo smoke through wasm — each gate enforces its own copy
/// against the same engine, so neither copy can silently rot
const EXPECTED: &[(&str, &[&str])] = &[
    ("classes", &["count=2 area=12"]),
    // ... all 17, file order
];
```

The test looks its case up by file name and keeps the exact
got-vs-want failure shape. Rejected alternatives, on record:
**weakening the gate** to compile+run-only (a green gate's law does not
get trimmed to dodge a storage refactor — the native byte diff is the
only native-pipeline output pin in `cargo test`); **parsing the TS from
Rust** (fragile, a second parser for a data shape); **any renamed
sidecar file** (§2.2). The cost of the second copy — 51 lines
duplicated into Rust — is accepted and recorded: the TS copy is
enforced by the smoke, the Rust copy by cargo, both against the same
engine, so neither can drift from reality; they can only ever be
updated together.

### 2.5 The full phase-1 edit list (the migration mapping, compressed)

17 sidecar files → 17 inline blocks in `examples/index.ts` (md5-verified
verbatim) · `playground.rs` → `EXPECTED` table (same 17 values) ·
`rspack.config.ts` + `rspack.smoke.config.ts` rules → `\.rut$` ·
`modules.d.ts` → `*.expected` block deleted · `smoke.mjs` → wording,
§5 pin 40→35, new §7 · `cases.ts` header + `smoke-entry.ts` + `README`
→ wording · `verify.ts`/`runner.ts`/`App`/`StatusBar`/`Panes` → **zero
changes** · `git rm` the 17 files. The 8 inline cases and both
sanctioned values: untouched bytes.

## 3. The grep gate (exact form)

The law: **`find demo -name '*.expected'` is empty — untracked files
included.** Mechanism, decided honestly:

- **fs walk, not git.** `git ls-files` is blind to untracked strays —
  exactly the failure the gate exists to catch (a future "temp"
  sidecar must red the gate the day it appears, committed or not). The
  smoke's §5 `walk()` already does a raw fs walk; the new §7 walks
  **all of `demo/`**, checks **basenames only** (never reads file
  contents — the walk covers binary artifacts safely), and asserts
  zero `.expected` names:

```js
// ---- 7. the no-sidecars gate (the batch's law, committed) ----
// `find demo -name '*.expected'` must stay EMPTY — untracked included
// (a raw fs walk, not git ls-files, which is blind to untracked strays).
// node_modules is pruned: vendored third-party land, not the demo's
// surface (zero strays there today — checked at base — so even the
// literal command is empty; the prune keeps a transitive dep's data
// file from ever red-ing our gate). dist/ and dist-smoke/ stay IN:
// they are inside demo/ and only ever carry build-emitted names.
const strays = walkDemo(DEMO).filter((p) => p.endsWith(".expected"));
check(strays.length === 0, "find demo -name '*.expected' is empty (untracked included)",
      strays.join(", "));
```

- **Structural backstops** (independent of the smoke): the narrowed
  rspack rule makes any leftover `import ... from "*.expected"` fail
  the build loudly, and the deleted `modules.d.ts` block makes tsc
  reject it too. Files (§7), imports (build), types (tsc) — three
  independent deaths for three reference shapes.
- **No content regex, and why (considered, rejected):** §5's
  DEAD_SHAPES scan cannot join this gate — `.expected` as a *filename
  token* (`"./sieve.expected"`) is textually indistinguishable from
  the *live property access* (`c.expected`, `r.expected`,
  `fuel.expected`); any single-line regex either false-positives on
  the fields the verifier legitimately keeps or misses references.
  The walk + the build cover every real reference; the regex would
  only add noise.
- **The human form** (README, phase 1):
  `find demo -path demo/node_modules -prune -o -name '*.expected' -print`
  — prints nothing.

## 4. Phase order — confirmed

This is a **2-phase batch**; the order is:

1. **Phase 0 (this commit, docs-only):** this survey. No code, no
   deletions, no data movement — the corpus, both gates and the tree
   are exactly as base left them (the parallel session's foreign files
   untouched; scratch under `/tmp/opencode/batch-demo-no-sidecars/p0/`).
2. **Phase 1 (removal + proof):** the §2.5 edit list, then ALL gates
   green before the commit: `npm run smoke` (now with §7, expecting the
   same 25/25 real+verified plus the new gate), `cargo test -p rut-cli
   --test playground` (on the `EXPECTED` table), `npm run build` (tsc +
   both rspack configs prove the narrowed rules and the deleted ambient
   block). The commit message carries the verbatim receipt (the md5
   table's confirmation) and the gate outputs. Single commit
   `phase(1): …`, pushed. Nothing in phase 1 touches the engine — the
   wasm artifacts are rebuilt, not re-authored; the 272-check law
   survives with its assertion source re-homed, not weakened.

The dependency is one-directional: phase 1 needs this survey's census
(the §5 count pin 40→35, the md5 receipts, the playground.rs decision)
and cannot land before it. Nothing in phase 0 depends on phase 1.

## Appendix — the empirical log (phase 0)

- `git status` at start: the parallel session's foreign files present
  and untouched throughout (`.opencode/skills/batch-plan-impl/SKILL.md`
  modified; `.opencode/skills/monitor-batch-plan-impl/` and
  `models.jsonc` untracked; one stash). This commit stages exactly one
  path: `docs/demo-no-sidecars-survey.md`.
- `find demo -name '*.expected'` → 17 files (§1.1); zero elsewhere,
  node_modules included. `find demo/src -type f` → 52 (35 after the
  removal — the §5 pin's arithmetic).
- `cd demo && npm run smoke` at base → **272 passed, 0 failed**
  (sections 1–6: the runner law, 25 cases real+verified, the REAL
  resume, real diags, the dead-surface grep gate, the LSP highlight
  census + overlay reconstruction).
- `cargo test -p rut-cli --test playground` at base → **1 passed,
  0 failed** (`classics_run_and_match_their_expected_sidecars`).
- Byte audit (§1.1): all 17 end `\n`; no CR / backtick / `${` /
  backslash; one load-bearing trailing space (`quicksort.expected`).
- History receipts: classics + gate 142d378; type-aliases 2696642;
  gap-fillers + rename 3210668; the sidecar flip and both sanctioned
  expected moves 02937e6 (survey `docs/demo-real-run-survey.md` §2.5/D2).
