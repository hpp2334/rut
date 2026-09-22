# demo-real-run — phase 0: survey + the concept audit

- **Batch:** demo-real-run (phase 0 of 5, docs-only)
- **Base:** 6d330e5 (`phase(3): the proof`, todolist-ui-framework close-out)
- **Date:** 2026-09-23
- **Spec:** `~/.opencode/plan/demo-real-run.md` — "the playground really
  compiles+runs, and LSP highlight"
- **Method note:** the audit is **empirical**. Every example already runs
  through the native gate (`cargo test -p rut-cli --test playground`,
  green at base); every inline case in `cases.ts` was extracted to
  scratch and compiled+run through the same engine via
  `target/debug/rut run` (the CLI mounts ink/pouch exactly like the
  playground gate). Verdicts below cite observed output, not reading.
  The wasm build runs the same engine (one `Vm::call("main")` per
  `rut_run`), so native behavior is the wasm contract.

---

## 1. Census

### 1.1 `demo/src/runner.ts` — the mode logic (the fake)

- `Runner.boot()` (L103): `fetch("rut.wasm")` → 200 ⇒ instantiate the
  raw-ABI module, `mode: "wasm"`. **Anything else — 404, network error,
  instantiate throw — falls through to `mode: "preview"`** (L121–129):
  the banner admits it ("Output below is the annotated expectation, not
  a live run") but the UI shape is identical: output appears, panes
  fill, Run "works".
- `compile()` (L132): preview returns `diags: []` +
  `irDump: placeholder("LIR")` — a **fake clean compile** for any
  source, including source that does not parse.
- `run()` (L137): preview replays `currentCase.expected`, with a
  special-case lie for `fuel-demo` (pops the hint line, calls it
  `Trap::OutOfFuel`).
- `placeholder()` (L161): the IR-paste-text.
- The banner is the only mode signal; `StatusBar`, `Panes`, `Editor`
  render identically in both modes. Silent-by-construction.

Today `public/rut.wasm` is **missing** (gitignored, `.gitkeep` only),
and `target/wasm32-unknown-unknown/release/rut_wasm.wasm` is not built
either — so every fresh checkout is preview mode. The fake is not an
edge case; it is the default experience.

### 1.2 The case inventory

Two populations, only one of them gated:

| population | count | lives in | gated by |
|---|---|---|---|
| inline cases | 8 | `src/cases.ts` (source as TS string arrays + `expected: string[]`) | **nothing** |
| classics | 13 pairs | `src/examples/*.rut` + `*.expected` (imported raw via `asset/source`) | `crates/rut-cli/tests/playground.rs` — compiles+runs each `.rut` natively (ink+pouch mounted, fuel 5M/4 MiB) and diffs against the sidecar |

The classics metadata is `src/examples/index.ts` (id/name/blurb/rfcs
per example; playground order). `CASES` + `EXAMPLES` render as the two
groups in `CaseList`. The gate test is the sidecar-flip's proof of
concept: it already does run-then-verify — natively. Phase 1 moves the
verification into the page (and the wasm smoke).

### 1.3 The wasm api surface (`crates/rut-wasm/src/lib.rs`, mirrored by `src/wasm/rut-api.d.ts`)

Raw ABI, no wasm-bindgen; every result is `[u32 LE len][JSON]` at the
returned ptr; a 16 MB bump arena (`rut_alloc`):

| export | shape | notes |
|---|---|---|
| `rut_alloc(len)` | `-> ptr` | bump; `0` on exhaustion |
| `rut_compile(src_ptr, len)` | `-> envelope` | `{diags:[{start,end,msg}], ast, irDump, binary?}` — binary is **base64** module (RFC 0033); AST is the structured JSON tree (the AstTree pane's source of truth) |
| `rut_run(bin_ptr, len, fuel, heap)` | `-> envelope` | `{output:[], trap?, fuelUsed, heapBytes}`; decode+verify per call; fresh `Vm` per call — **one-shot, no resume** |
| `rut_run_b64(bin_b64, fuel, heap)` | `-> envelope` | convenience twin (unused by the demo) |

- Embedded library surface (`compile_playground`, L98): `core`+`calc`
  (`mount_std`), `rt` (lowered from `rt.d.rut`), `ink`, `pouch` — as
  in-memory source. **Not mounted: `nmapset`, `nmap_host`,
  `bench-cross`** — §3 D6.
- Host bindings at run: `install_std_log` (→ OUTPUT cell) +
  `install_std_math`. **Not installed: `install_std_nmap`**.
- **No `resume` export.** The engine *has* parked-frame resume
  (`Vm::resume`, `rut-vm/src/interp/mod.rs:659`; the trap message says
  "the frame is parked; add fuel and resume()") but the wasm module
  drops the `Vm` when `rut_run` returns. The UI's "↻ Resume" button
  re-runs `main` from scratch with `budget.fuel + 10M` (App.tsx L43–50,
  L113) — a fresh run, not a resume, in *both* modes.
- `trap` strings: trap `name()` — e.g. `OutOfFuel`.

### 1.4 Build / CI gates

- `npm scripts` (demo): `dev` (rspack serve), `build` (rspack build,
  copies `public/` → dist so the artifact ships), **`build:wasm`**
  (`cargo build -p rut-wasm --target wasm32-unknown-unknown --release`
  + inline `node -e` copy to `public/rut.wasm`). `build:wasm` is in no
  chain — nothing runs it; a stale/missing artifact is silent.
- **No CI at all**: no `.github/workflows/`. The only automated gate
  touching the demo is the native playground test above. So the wasm
  story has no enforcement anywhere — phase 1's preflight + phase 3/4
  gates are the first.
- The extension's precedent for the loud-fail copy:
  `integrations/vscode-extension/scripts/copy-wasm.mjs` — `existsSync`
  check, `process.exit(1)` printing the **exact cargo command**. The
  demo's inline `node -e` copy fails with a raw ENOENT stack instead;
  phase 1 should adopt the curated form.
- Extension pattern to copy for bundling: `src/wasm.ts` (the ABI
  binding, typed) is esbuild-bundled standalone to `out/wasm.js`
  (5,733 B) and driven headless in plain node by `test/e2e-wasm.js` —
  "the exact load path `src/extension.ts` uses, minus the vscode
  host". That is precisely the demo's shape: one binding module,
  artifact fetched at runtime, a headless gate driving it.

### 1.5 The editor today

`components/Editor.tsx`: a plain `<textarea>` + line-number gutter
(scroll-synced), Tab = 2 spaces. Zero highlighting, zero diagnostics,
zero deps. `styles.css` carries the layout. `Panes.tsx` = Output/AST/IR
tabs; `StatusBar.tsx` = Run, Resume, fuel/heap inputs, fuel-used/heap/
trap readouts.

### 1.6 The LSP binding option (for §3 D3)

`crates/rut-lsp-wasm` — same envelope protocol, plus a per-request
arena reset (`rut_begin`):

| export | purpose |
|---|---|
| `rut_begin()` / `rut_alloc(len)` | request arena |
| `rut_legend()` | token-type names, JSON array in index order |
| `rut_analyze(uri, src)` | store doc → `{diags, tokens:{data}, symbols}` — **diagnostics + full semantic tokens in one call** |
| `rut_forget(uri)` | close doc |
| `rut_hover/complete/definition/type_definition/inlay/references/signature_help(uri, …)` | the query set |
| `rut_add_def(uri, src)` | index a workspace file |

- Artifact: `bin/rut-lsp.wasm` = **719,287 B (~0.7 MiB)**, built by the
  extension's `npm run build:wasm` (cargo + `copy-wasm.mjs`,
  loud-fail). One artifact for every platform — the whole point of the
  wasm mode.
- The embedded std surface (`rut-lsp/src/std_surface.rs`) includes
  **all eight pkgs** — core, calc, nmap_host, rt, bench-cross, pouch,
  **nmapset, ink** — so a demo doc that says `use ink::{Logger}` /
  `use pouch::{Vec}` / `use nmapset::{HashMap}` resolves with **no
  `addDef` needed**.
- Init cost in-browser: one fetch (~0.7 MiB, same-origin, dev-server
  static or dist-copied) + one `WebAssembly.instantiate` + first
  `rut_analyze` of a doc-sized source. The extension's e2e analyzes 58
  corpus files in seconds in node; a single doc is milliseconds.
  Instantiation of a 0.7 MiB module is tens of ms. No fs, no workers
  required; single-threaded JS + `rut_begin` per request is the
  documented-safe protocol.
- Token wire format: flat `data` array, 5 u32s per token (LSP
  SemanticTokens), legend names via `rut_legend()`. Full-sync only —
  there is no delta endpoint; a doc is re-analyzed whole. That is the
  right granularity for doc-sized sources.

---

## 2. The concept audit — the demo vs TODAY's surface

**Authority:** the RFCs as amended (0009 v1.1 `struct`; 0014 amended
`?T` downcast; 0043 inline `requires`; 0044 sharing regime + `?T`), the
current std pkg set (`core`, `calc`, `nmap_host`, `nmapset`, `pouch`,
`ink`, `rt`, `bench-cross`), and the current grammar (parser/lexer
diagnostics are themselves the removals' documentation). The demo must
teach **today**, not history — including in comments (the phase-2 grep
gate counts comments, the todolist app-law precedent).

### 2.1 The surface status table (what the audit checked everything against)

| surface element | status today | evidence |
|---|---|---|
| `struct` | **current** — the record keyword (RFC 0009 v1.1) | parser item.rs |
| `dataclass` | **dead** — reserved word, loud error: "spell it `struct`" | lexer.rs:641 |
| `where` clauses | **dead** — removed (RFC 0043); parser diagnoses with the `<T requires B>` replacement; `where` is otherwise a legal identifier | item.rs:1097,1236 |
| `string` | never existed as a spelling — the primitive is `str` | core.d.rut |
| `char` | **dead** — 1-codepoint `str`; `s.code()` / `str.from_code(n)` | core.d.rut |
| `mapset` pkg, `Hashable` trait | **removed** — the map lane today is `nmapset` (`HashMap<K requires …prims…, V>`, `HashSet<T>`, `PrimMapI64/U64/F64`), keys by union-bound admission, no hashing trait | rut/nmapset/nmapset.rut |
| `Ptr<T>` / `*T` / `&v` / postfix `T?` | **dead** — all diagnose; the pointer shape today is the nullable `?T`, prefix-only (RFC 0044 §2.1) | lexer/parser; RFC 0044 |
| copy-by-value records/arrays, `own(x)` | **dead** — bindings share by reference; primitives + `fn` copy; `bytes.clone()` is the one copy (RFC 0044 §1) | RFC 0044; `dataclasses.rut` behavior |
| structural `==` | **dead** — `==` is identity for cells (prims by value, `str`/`bytes` by content, everything else slot compare) | RFC 0044 §3 |
| `opaque.downcast<T>` → `(T, bool)` | **dead** — returns `?T` (`nil` on a miss); refval-round2 migration already landed in `demo/` | opaque.rut; cases.ts `opaque` |
| generics `<T requires A \| B>` | **current** — inline union bounds, admission-only | RFC 0043; type-aliases.rut |
| `when` (only match), `->` arms, `else` | current | RFC 0008 |
| `Weak<T>` | **absent (deliberately)** — motivation only, no surface | rut-core/sym.rs:157 |
| `select`/`await`/`async` | grammar exists; **unusable in this demo host** (no host-futures bindings in rut-wasm) | rt.d.rut has only the logger |

### 2.2 The classics (13 examples) — verdicts

All 13 compile+run **green against their sidecars** at base
(playground test, 1 passed). Code is current; the findings are the
taught *comments/blurbs* and naming. Verdicts for phase 2:

| example | compiles/runs? | current? | verdict | findings (dead concept → replacement) |
|---|---|---|---|---|
| sieve | ✓ green | ✓ | **KEEP** | none — flat `Vec<u8>/Vec<i32>`, for-c, while |
| quicksort | ✓ green | ✓ | **KEEP** | none — in-place Vec mutation, recursion |
| matrix-mul | ✓ green | ✓ | **KEEP** | none — flat `Vec<f32>` hot loops |
| classes | ✓ green | ✓ | **KEEP** | none — class-method construction, `Self {}`, `?Version` nil-on-fail, `wrapping_add`; a garbled comment line ("must initialize fieldless-initialized") worth one pass |
| closures-generics | ✓ green | ✓ code | **REWRITE (blurb only)** | blurb says "arrows" — RFC 0013 §1: block bodies, **no arrow form**; say "anonymous fns (block bodies), fn types, capture by value" |
| dataclasses | ✓ green | ✓ code | **REWRITE (identity)** | code is the RFC 0009 v1.1 `struct` + RFC 0044 sharing story and is correct — but the *name* (`ex-dataclasses`, fn `dataclasses()`, title "dataclasses") and the blurb's "**own divergence**" teach a removed surface (`own` is gone). Rename to "structs" (`ex-structs`), blurb "reference semantics (sharing by default), identity `==`" |
| literals | ✓ green | ✓ code | **REWRITE (small)** | `let bin = bytes(64);` — created, never used: bytes is the demo's one hidden primitive. Use it (`bin.len()`, `bin.decode()`) or cut it (§2.4 gap). Comment says both `[T]`/`Vec` "implement `Iter` (RFC 0012)" — the trait is spelled `Iterator` (engine-woven, `__iterate`) in core today |
| opaque | ✓ green | ✓ | **KEEP** | the refval-round2 migration is complete and correct — `?T` downcast, alias law, `is`; the poster child |
| when | ✓ green | ✓ | **KEEP** | none — exhaustive pattern expressions, `else` law |
| node-cycle | ✓ green | ✓ code | **REWRITE (comments)** | header teaches "`next: *Node`" — the `*T` spelling **diagnoses** today; code correctly uses `?Node`. Trailing comment sells the `Weak` leak-fix as if it exists ("hold `b` from `a` through Weak … frees deterministically") — the code does no such thing and `Weak` is deliberately absent; say what today's truth is: the cycle stays live, `?T` is the pointer shape, collectors/weak are future (RFC 0017) |
| tree | ✓ green | ✓ code | **REWRITE (comments)** | header teaches "`*Node` is one word" — dead spelling; code uses `?Node` (correct). Also the only example with comma-separated struct fields (legal — the parser accepts `;` **or** `,` after a field, item.rs:1002 — note the style, prefer `;` for consistency) |
| weak-cache | ✓ green | ✓ code | **REWRITE (comments)** | header comment is **mangled mid-sentence** ("…`Weak<T>` exists for. Weak itself"); the M5 comment quotes a `Weak(alive)` spelling that is not surface. Blurb already honest ("shown with today's strong refs") — fix the header to one clean thought: strong-held cache = the cell lives; the miss answers `nil` |
| type-aliases | ✓ green | ✓ | **KEEP** | the RFC 0043 exhibit — aliases, trait bound, **union bound** `<T requires Ridge | Trench>`, admission-only semantics |

### 2.3 The inline cases (8 in `cases.ts`) — verdicts

Extracted and driven through `rut run` (native engine, ink+pouch
mounted — the same mounts the wasm host makes):

| case | runs? | output matches `expected`? | verdict | findings |
|---|---|---|---|---|
| hello-format | ✓ | ✓ exact | **KEEP** | f-strings, escapes, when-on-enum — current |
| **values-and-pointers** | ✗ **compile error** | ✗ (unreachable) | **REWRITE** | `let rp = &p;` / `let rq = &same;` → "`*x` / `&x` were removed — bindings share by reference now — pass `x` directly (RFC 0044)" — two hard errors. Worse, the case *teaches the dead regime*: "copy-by-value: q owns its own cell", "structural ==" and its expected (`q.x=1`, `q==same true`) are all pre-0044. The rewrite is §2.5's v2 (already written and run: today's lesson is *sharing* + identity `==` + `?Point` as the pointer) |
| opaque | ✓ | ✓ exact | **KEEP** | the migrated `?T` downcast shape — current |
| sieve | ✓ | ✓ exact | **KEEP** | mirrors the classic |
| when-exhaustive | ✓ | ✓ exact | **KEEP** | nested when, exhaustiveness |
| tuple-errors | ✓ | ✓ exact | **KEEP** | the `(T, err)` convention, for-of over `str` |
| closures-generics | ✓ | ✓ exact | **KEEP** | `map<T,U>`, anonymous fn capture |
| fuel-demo | ✓ (traps) | **✗ magnitude** | **REWRITE (expected only)** | real default-budget (10M fuel) run: **zero tick lines**, trap `OutOfFuel`. The sidecar promises `tick 1000000/2000000/3000000` + `...` + a resume hint — a magnitude that corresponds to ~40M fuel, i.e. the preview fabrication (empirical: 20M ⇒ 1 tick, 30M ⇒ 2 ticks; ~10 ops/iteration). The `...` line and the resume-hint line are preview-era artifacts and die; §3 D2/D5 |

Header drift: `cases.ts`'s banner comment still describes the
**copy-by-value v1.2** regime ("copy-by-value structs" — and, oddly,
"no arrows", which is right: RFC 0013 has no arrow form). The header
needs the RFC 0044 paragraph when the values case is rewritten.

### 2.4 GAPS — concepts today's surface has that the demo doesn't teach

| gap | what's missing | candidate example | prerequisite |
|---|---|---|---|
| **the map lane** | nothing demonstrates `nmapset` — `HashMap<K, V>` (prim/str/bytes keys via union bound), `HashSet<T>`, or the `PrimMapI64/U64/F64` unboxed lanes | word/count over a str (HashMap<str, i32>) + an id→payload lane (PrimMapI64) — the shape the todolist app actually uses | **`rut-wasm` must mount `nmapset` + install `install_std_nmap`** (D6): it uses `nmap_host::{map_new,…}` host fns; the wasm host mounts neither the pkg nor the table today |
| **bytes** | the only `bytes` value in the demo is an unused binding in literals.rut; `encode`/`decode`/`clone`/`len` (the one copy escape hatch!) never shown | either promote inside literals (use the value) or a small "binary data" case: `str.encode()` → mutate-free `bytes` → `decode()`, and `bytes.clone()` as THE copy | none |
| **str views** | RFC 0042 `s.slice(from,to)` (O(1) view), `s.code()`, `str.from_code(n)` — the codepoint story (`char` is gone) is stated but never exercised | a "text as data" case: slice/code/from_code round trip | none |
| **checked/wrapping arithmetic** | the core `builtin impl` methods (`wrapping_add`, `checked_add -> (T, bool)`) appear once (classes) with no teaching | fold into literals or the bytes case (`u8` boundary walk) | none |
| select/await | **not a demo gap** — grammar exists but this host has no host-futures; teach it never, honestly (README note in phase 4) | — | host futures (out of scope) |

### 2.5 The values-and-pointers rewrite — ground truth (run, not guessed)

Today's version of the case teaches RFC 0044 directly; this exact text
ran green in scratch:

```rut
use ink::{Logger};

struct Point { x: f32; y: f32 }

pub fn main() {
    let log = Logger.new("case");
    let mut p = Point { x: 1, y: 2 };
    let q = p;                  // share: q and p name ONE cell (RFC 0044)
    p.x = 4;                    // q.x is 4 now — sharing is the law
    let same = Point { x: 1, y: 2 };
    log.info(f"q.x={q.x} p.x={p.x}");
    log.info(f"q==same {q == same}, p==same {p == same}"); // cell identity ==
    let mut rp: ?Point = p;     // the pointer shape today: the nullable box
    rp.x = 9;                   // writes through the box (auto-deref) — shared
    log.info(f"rp.x={rp.x} p.x={p.x}");
    let rq: ?Point = same;
    log.info(f"rp==rp {rp == rp}, rp==rq {rp == rq}"); // box identity
}
```

Real output (every line moved vs the stale `expected`):

```
q.x=4 p.x=4
q==same false, p==same false
rp.x=9 p.x=9
rp==rp true, rp==rq false
```

Phase 2 lands this rewrite (or its edited twin) with this sidecar and
a disclosed commit body: copy-by-value/structural-`==`/`&v` →
sharing/identity-`==`/`?T` (RFC 0044).

---

## 3. Decisions

### D1 — Preview mode dies; wasm-or-loud (phase 1)

- `RunnerState.mode: "wasm" | "preview"` → `"wasm" | "error"`. Boot:
  fetch/instantiate failure is **not** a fallback — it is the page
  state. `error` renders a full-page panel: "rut.wasm missing or
  invalid — run `npm run build:wasm` in demo/ (RFC 0041 §3)", with the
  boot error text. The panes/runner never mount.
- `Runner.compile/run` lose the `if (this.api)` branches and the
  `currentCase` replay parameter entirely — an `error` Runner has no
  methods to call. `placeholder()` is deleted. The `fuel-demo`
  special-case (`lines.pop(); trap = "OutOfFuel"`) dies with preview.
- `build:wasm` gets the extension's curated loud-fail (a
  `scripts/copy-wasm.mjs` twin: existsSync → exit 1 with the exact
  cargo command), and `dev`/`build` grow a preflight assert on
  `public/rut.wasm` presence (loud, same command) — the artifact
  exists by construction.
- The banner text keeps the honesty law in wasm mode: "live —
  rut.wasm …" with the real engine slice noted.

### D2 — The sidecar flip: replay → verify (phase 1, disclosures in phase 2)

- After every real run, `App.run()` diffs `res.output` (+ `res.trap`)
  against `currentCase.expected`. `StatusBar` gains a verify chip:
  **"✓ matches expected"** / **"✗ N lines differ"** (and "no sidecar"
  is impossible for prepared cases; custom edits verify against the
  case they came from, cleared on divergence-by-edit — an edited source
  that no longer matches shows the diff, which is the honest signal).
- A failure renders the diff in the Output pane: line-paired
  expected/got with a marker per differing line; the trap line
  participates (`Trap::OutOfFuel` expected where pinned).
- **Sidecars that legitimately moved** (post-refval-round2, all
  disclosed in phase 2's commit body):
  - `values-and-pointers` (inline): all four lines move — §2.5.
  - `fuel-demo` (inline): re-record at the pinned default budget —
    zero ticks + trap (§2.3); `...` and the resume hint die.
  - **The 13 classics: none move** (gate green at base — the migration
    batches already updated them; `opaque.expected` is the downcast
    shape).
- Nondeterminism audit: every case is deterministic **given its
  budget** (f32 formatting is Rust-fmt stable; no clocks, no host
  entropy). `fuel-demo` is the one budget-*dependent* case: the
  verifier pins the **default budget** (10M / 4 MiB) for comparison;
  a user who raises the fuel box sees a diff by design — the sidecar
  pins the default, and the chip says which budget it verified at.

### D3 — Highlight: LSP semantic tokens are the source of truth; no TextMate layer (phase 3)

- Bind `rut-lsp-wasm` through a **standalone binding module** in the
  demo (`src/lsp/rut-lsp.ts`, the typed twin of the extension's
  `src/wasm.ts` — same envelope, `rut_begin` per request), artifact
  `public/rut-lsp.wasm` (gitignored, loud-fail, joined into the npm
  scripts as `build:lsp` or folded into `build:wasm` — one command,
  both artifacts). The `out/wasm.js` standalone-bundle + headless
  e2e pattern is the gate template (phase 3's smoke asserts token
  classes on known lines).
- On case select and on edit (**debounced ~150 ms** — full-sync
  re-analyze of a doc-sized source is milliseconds; the extension's
  e2e does 58 files in seconds): `rut_analyze` → tokens → an overlay
  `<pre>` of colored spans (D4), diags → squiggles (below).
- Token-class → CSS map lives in the demo, keyed by `rut_legend()`
  indices (never by hardcoded order).
- **Diagnostics squiggles: in scope** — they are the same response's
  `diags` (range + message), one extra span decoration with a
  title=tooltip; no second call, no new architecture. Compile diags
  from `rut_compile` stay the Run-time error surface; the LSP diags
  are the while-typing surface (parse-level noise mid-edit is
  acceptable and debounced).
- **TextMate instant-paint: rejected.** Cost census:
  `vscode-textmate` + `vscode-oniguruma` (+ its ~.5 MiB oniguruma
  wasm) + the grammar file — two new runtime deps and a second
  tokenizer to keep aligned with the semantic truth, to save
  first-paint latency that is already ~tens of ms and then
  irrelevant (tokens arrive between keystrokes). The extension needs
  TextMate because VS Code's grid owns the paint; the demo owns its
  paint — one tokenizer, the true one. An honest interim: monochrome
  text until the first analyze lands (sub-100 ms), never a fake.

### D4 — Editor: keep the zero-dep textarea + colored overlay; CodeMirror rejected (phase 3)

- The editor becomes the classic double-layer: a transparent-text
  `<textarea>` (caret + selection visible) exactly over a highlighted
  `<pre>` (the overlay), both `white-space: pre`, identical font
  metrics, scroll-synced (the gutter already scroll-syncs — same
  mechanism). Tab already inserts spaces, so no tab-stop mismatch
  class exists. Zero new deps — the plan's preference, and the census
  gives no dishonesty: spans come from the LSP (truth), repaint is
  synchronous with state, and the failure mode (a metric drift) is
  visible misalignment, not silent wrongness.
- CodeMirror would buy gutter markers, history, IME hardening — for a
  new dep and an API surface the demo doesn't need (one file, one
  view, run-button editing). The RFC 0041 §3 "noted path, not taken"
  stance stands, now with the audit's record: **revisit only if the
  overlay drifts in practice** (phase 4's browser drive is the test).
- Squiggle decorations render in the same overlay layer (a span with
  a wavy underline + title).

### D5 — Resume: expose the engine's parked-frame resume through rut-wasm (phase 1, with a disclosed fallback)

The engine already parks and resumes (`Vm::resume`); only the wasm
one-shot ABI throws the frame away. Decision: **phase 1 adds a resume
to the wasm module** — keep at most one parked `Vm` in the module
(static, like `OUTPUT`), export `rut_resume(extra_fuel, heap)` (+
`rut_drop_frame()` for case switches), have the demo's Resume call it
instead of re-running `main`. The run-then-verify chip then verifies
the **accumulated** output of run+resumes. This is a `rut-wasm`-crate
change — **not** an engine change — and it makes the fuel lesson real
(the frame actually continues; ticks 1..N then 2M, 3M… not a restart).

Fallback (if the parked-Vm state proves unworkable in phase 1 — e.g.
lifetimes across calls): relabel the button "↻ Re-run (+10M fuel)" so
the UI stops claiming a resume, and `fuel-demo`'s expected is the
re-run's full output at the raised budget. Disclose which one landed
in the phase 1 commit body.

### D6 — The nmapset gap needs a host-mount change in rut-wasm (phase 2 carries it)

The gap example (§2.4) compiles only if the demo host mounts
`nmapset` and installs `install_std_nmap` in `rut_run`'s host
registry — today it mounts core/calc/rt/ink/pouch only. That is a
`rut-wasm`-crate change (embedded `include_str!` twin of the pouch
mount + one registry line + `compile_playground`'s module set), **not**
an engine change, but it is a real prerequisite phase 2 must carry
alongside the new example, with the playground gate extended (mount
nmapset there too, so the sidecar is gated natively). Without it, the
demo's map lane silently doesn't exist — exactly the dishonesty this
batch kills.

---

## 4. Phase order — confirmed, with one amendment

**0 → 1 → 2 → 3 → 4 stands.** Reasons at the seams:

- **1 before 2** is right: the verifier is phase 2's enforcement (a
  rewritten example that lies shows red in the page); building the
  examples first would mean auditing them by hand.
- **2 before 3** is right: highlight gates (token classes on known
  lines) should assert against the **final** corpus — doing highlight
  first would re-gate it after every example churn.
- **3 before 4** stands (the browser drive proves highlight + real
  run together).
- **Amendment (recorded):** phase 2 explicitly carries the rut-wasm
  nmapset mount + `install_std_nmap` (D6) as part of the map-lane gap
  example, and extends the playground gate to mount nmapset. Phase 1
  explicitly carries D5's resume export (or its disclosed fallback).
  Neither touches the engine; both keep the batch law (loud, gated,
  disclosed).

Standing laws carried forward: REAL-or-LOUD (D1), sidecars verify
(D2), highlight rides the LSP (D3), no engine changes (D5/D6 stay in
the wasm/demo crates), artifacts gitignored with the exact build
command, explicit-path staging only (shared lane), scratch under
`/tmp/opencode/batch-demo-real-run/`.

---

## Appendix — the empirical log (phase 0, scratch)

All commands run at base 6d330e5, tree clean; scratch under
`/tmp/opencode/batch-demo-real-run/p0/`.

- `cargo test -p rut-cli --test playground` → **1 passed** (all 13
  classics compile+run, match sidecars).
- `cd demo && npm run build` → **compiled successfully** (no wasm
  artifact involved — the page would be preview mode; the fake is
  the default).
- Inline-case extraction (8 files) → `target/debug/rut run <case>`:
  hello-format / opaque / sieve / when-exhaustive / tuple-errors /
  closures-generics → **run green, output exact**;
  values-and-pointers → **2 compile errors** (`&p`, `&same`, RFC
  0044); fuel-demo @10M → **trap OutOfFuel, no output**; @20M → 1
  tick; @30M → 2 ticks.
- The §2.5 rewrite → **runs green**, output as printed there.
- Artifact sizes: `bin/rut-lsp.wasm` 719,287 B; extension
  `out/wasm.js` 5,733 B; `target/…/rut_wasm.wasm` — absent.
- Engine resume: `Vm::resume` at `crates/rut-vm/src/interp/mod.rs:659`
  (parked-frame message in the OutOfFuel trap), no wasm export.
- No `.github/workflows/` — no CI anywhere.
