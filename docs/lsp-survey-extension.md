# Phase 0b — VS Code extension survey (rut-vscode vs the current language)

Batch: `rut-lsp-align`, Phase 0, TASK B.
Scope surveyed: `integrations/vscode-extension/**` (read-only) against the
**current** language ground truth: `crates/rut-parser/src/lib.rs`
(`RESERVED_KW`, `is_primitive_ty`), `crates/rut-lexer/src/lexer.rs`
(`reserved_word_msg`), and RFCs 0009 v1.1 / 0014 / 0042 / 0043 / 0044.
Corpus ground truth: `rut/**/*.rut`, `examples/**/*.rut`,
`demo/src/examples/*.rut`, `benches/workloads/**/*.rut`.
This file is the deliverable — no code was changed. Phase 2 implements.

Companion survey: `docs/lsp-survey-lsp-wasm.md` (TASK A — rut-lsp + wasm
lane; its classifier/legend findings are cross-referenced, not duplicated).

---

## 1. Inventory — the extension's grammar/syntax surface

18 tracked files (all read at HEAD `4aebe24`); `out/`, `bin/rut-lsp.wasm`,
`rut-vscode-0.2.0.vsix` exist on disk but are gitignored build artifacts.

| Artifact | What it is | Syntax-relevant content |
|---|---|---|
| `syntaxes/rut.tmLanguage.json` | TextMate grammar (`source.rut`) | 14 top rules: comments (line/block), f-string, raw-string, string, char, number, boolean, `nil`, `Self`, primitive-type, control-keyword, keyword, operator, punctuation |
| `language-configuration.json` | editor config | `//` + `/* */`, 3 bracket pairs, auto-close for `(`/`[`/`{`/`"`/`'`, indent on `{`, `// region:` folding markers |
| `package.json` | manifest | language id `rut` (`.rut`), grammar contribution, `semanticTokenScopes` map, activation `onLanguage:rut`; **no `contributes.snippets`** |
| `src/extension.ts` | activation + providers | diagnostics (200 ms debounce), semantic tokens (decodes LSP delta encoding), document symbols, hover, completion (trigger `.`), workspace index (`findFiles`, cap 500 → `rut_add_def`) |
| `src/wasm.ts` | ABI binding | `rut_begin/alloc/legend/analyze/forget/hover/complete/add_def` over the raw arena+JSON envelope — syntax-agnostic transport |
| `src/webassembly.d.ts` | wasm API typings | — |
| `esbuild.mjs`, `scripts/copy-wasm.mjs`, `scripts/run-vscode-test.mjs` | build/test tooling | bundle → `out/`; copy wasm → `bin/`; launch VS Code test host |
| `test/runTest.js` | Extension Host test | 5 assertions (see §4) |
| `test/fixtures/symbols.rut` | the one fixture | `enum`/`class`/`trait`/`fn`, `-> nil`, `i32` — **pre-RFC-0043/0044 surface only** |
| `README.md`, `.vscodeignore`, `.gitignore`, `.vscode/*`, `tsconfig.json`, `package-lock.json` | meta | — |

**Snippets: none.** No `snippets/` directory, no `contributes.snippets` in
the manifest. Noted as an inventory absence, not a misalignment.

Test fixtures: exactly one — `test/fixtures/symbols.rut` (21 lines).

**Already aligned** (so phase 2 doesn't churn them): the f-string rule
(`f"…"`, `{{`/`}}` escapes, `{expr}` holes re-including `source.rut`)
matches the lexer's §1.1 brace-balanced hole lexing (`examples/02-digest/
digest.rut:32` `d.push(f"{c}");` colors its hole); raw strings `r"…"`;
number suffixes `u8…u64, i8…i64, f32, f64` match `is_primitive_ty`
exactly; `nil` → `constant.language.null` (it is the null literal, RFC
0044); `Self`; `break`/`continue` (real statement keywords, parser
`stmt.rs:185/190`); contextual `as`/`super`/`self` (cast, `pub(super)`,
`self` param) — coloring them is correct; `when`/`of`/`is`/`host`/`select`
all current. The `semanticTokenScopes` map in `package.json` covers all
**14** legend token types 1:1 (`crates/rut-lsp/src/semantic/legend.rs`
`ALL`) — keyword, number, string, type, function, method, variable,
parameter, property, enum, enumMember, class, trait, operator. No drift.

---

## 2. Misalignment list — corpus-backed

Ground truth tables:
`RESERVED_KW` = let mut if else while for of return when enum class
**struct** trait impl requires use pub static async await extern is host
fn true false nil select — `type` left the table (RFC 0043: contextual
alias introducer), `where` is gone (RFC 0043), `dataclass` is a hard
lexer error ("spell it `struct`", RFC 0009 v1.1).
`is_primitive_ty` = bool | **str** | **bytes** | nil | f32 | f64 | i8…i64
| u8…u64. `opaque` is a boot primitive (`builtin primitive opaque`,
RFC 0014 — the renamed `any`; `any` is a hard lexer error).

### M1 — `struct` missing, `dataclass` still colored (RFC 0009 v1.1)
Grammar `#keyword` ships `dataclass` and lacks `struct`.
- Corpus: `demo/src/examples/dataclasses.rut:12` `struct Point {`;
  `examples/00-todolist/todolist.rut:16` `struct Todo {`;
  `demo/src/examples/type-aliases.rut:21` `struct Ridge { h: i32 }`.
  **10 corpus files** use `struct`; **0** use `dataclass` (it cannot
  compile — lexer hard error).
- Renders: every `struct` decl in the corpus gets plain identifier
  foreground (no keyword color); a user typing the dead `dataclass` gets
  keyword color on a word the compiler rejects.

### M2 — `where` colored as keyword though removed (RFC 0043)
Grammar `#keyword` ships `where`; it is not in `RESERVED_KW` and the
parser diagnoses it (bounds are inline now).
- Corpus: zero occurrences in code (only inside comments, where the
  comment rule already wins). No corpus file can exercise it — that is
  the point: it is dead weight plus a wrong signal for newly typed code.

### M3 — primitive list wrong: has `string`, misses `str`, `bytes`, `opaque`
Grammar `#primitive-type` = `u8…u64 | i8…i64 | f32 | f64 | bool | string`.
`string` is **not** a rut type (only `string_len`/`string_encode` host
fns — `\b` keeps those from matching, so the rule only misfires on a bare
identifier named `string`). `str` and `bytes` are primitives
(`is_primitive_ty`, RFC 0004); `opaque` is the boot primitive (RFC 0014,
the renamed `any`).
- Corpus: `rut/nmap_host/nmap.d.rut:61` `pub host fn map_entry_s(m: opaque,
  k: str) -> i32;` — `opaque` and `str` render as plain identifiers;
  `rut/core/core.d.rut:59` `fn slice(self, from: i32, to: i32) -> str;`;
  `demo/src/examples/opaque.rut:28` `let box1 = opaque(Point { x: 1, y: 2 });`.
  `string` as a type: **0** corpus occurrences (only an English word in a
  `benches/workloads/json-decode/main.rut:413` comment).
- Renders: the grammar colors a nonexistent type and misses three real
  ones on every `.d.rut` host crossing and every erasure-box use.

### M4 — byref/nullable flip: `?T` prefix-only, no grammar affordance (RFC 0044)
`?T` is the nullable — prefix only, binds tightest; `[?T]` = array of
nullables, `?[T]` = nullable array; `nil` is the null; `*T`/`&v`/`own()`
/`make_ptr()` are removed (each is a diagnosed hard error).
- Corpus: `rut/pouch/pouch.rut:24` `buf: [?T];`; `rut/pouch/pouch.rut:44`
  `let mut buf: [?T] = [nil; n];`; `rut/core/core.d.rut:35`
  `builtin fn on_drop<T>(p: ?T, cleanup: fn(?T)) -> nil;`.
- Renders: `?` falls through to the generic `#operator` rule →
  `keyword.operator` (same class as `+`), the `T` stays plain — the
  grammar still carries the pre-0044 reading where `?` is merely an
  operator. Cosmetic once semantic tokens load (they override in-range),
  but the first-paint grammar never marks the type position. Legacy
  `*T`/`&v` render as ordinary operator code with no hint they are
  compile errors — acceptable for a highlighter, recorded here for
  completeness.

### M5 — union bounds + type aliases: `type` introducer uncolored (RFC 0043)
`type X = A;` (transparent alias), `type X = A | B;` (union alias,
bound-only), inline `T requires A | B` bounds; `type` left the reserved
table and is a contextual introducer.
- Corpus: `demo/src/examples/type-aliases.rut:13` `type Meters = i64;`;
  `demo/src/examples/type-aliases.rut:40`
  `fn kind<T requires Ridge | Trench>(x: T) -> str {`;
  `rut/nmapset/nmapset.rut:390`
  `pub class PrimMapI64<K requires i8 | i16 | i32 | i64 | u8 | u16 | u32
  | u64 | bool | str | bytes> {`.
- Renders: `requires` ✓ (already a keyword), `|` ✓ (operator), but `type`
  reads as a plain identifier on every alias decl. Low severity — a
  contextual rule (`^\s*type\s+` lookahead) is the alignment, not a
  blanket keyword (which would miscolor ordinary identifiers named
  `type`).

### M6 — slices/views (RFC 0042): nothing live misrenders; one latent gap
`s.slice(from, to)` is method-call shaped — the `.` completion trigger and
semantic member coloring cover it; a slice *is* a `str` on the surface.
The lexer produces `Tok::DotDot`, but **the parser never consumes it**
(zero uses in `rut-parser`/`rut-ast`) and the corpus contains `..` only
inside a comment (`rut/nmapset/nmapset.rut:474`, `2^63..2^64-1`).
- Gap: the grammar's `#operator` rule has no `..` alternative. Harmless
  today; becomes a live misrender the day range/slice syntax lands.
  Alignment: add `..` to the operator alternation when the surface
  arrives (note only — not actionable in phase 2 as specified).

### M7 — PrimMap: no dedicated misalignment
`rut/nmapset/nmapset.rut:390/447/496` (`PrimMapI64/U64/F64`) exercise
generic params + inline union bounds — everything miscolored there is
already M3 (`str`/`bytes` plain). No PrimMap-specific grammar surface.

### M8 — `builtin` surface keyword uncolored (decl files)
The parser treats `builtin` contextually (`builtin fn` /
`builtin primitive` / `builtin impl` / `builtin trait`; `pub builtin` is
removed). The grammar doesn't color it.
- Corpus: `rut/core/core.d.rut:35` `builtin fn on_drop<T>(…) -> nil;`;
  `rut/core/core.d.rut:92` `builtin primitive opaque {`.
- Renders: plain identifier at the head of every boot-primitive decl.
  Same low severity / contextual-rule treatment as M5.

---

## 3. What "aligned" takes per artifact (edit list — no implementation)

1. **`syntaxes/rut.tmLanguage.json`** (the whole fix lives here):
   - `#keyword`: remove `dataclass`, remove `where`, add `struct`. (M1, M2)
   - `#primitive-type`: `string` → `str`, add `bytes`; add `opaque`
     (either into the same rule or a sibling `#builtin-type` — `opaque`
     is call-shaped at term level, so a separate name rule keeps
     `opaque(v)` calls readable while the type position still colors).
     (M3, M4-adjacent)
   - Optional (recommended): a `#type-question` rule coloring `?` (and
     `??` chains) in type position — TextMate approximation only; exact
     classification stays with semantic tokens. (M4)
   - Optional: contextual rules for `type` alias introducer (M5) and
     `builtin` (M8) — anchored (`^\s*type\s+`, decl head), never blanket.
   - Leave everything else untouched: f-string holes, raw strings,
     numbers, `nil`, `Self`, operator/punctuation sets are aligned.
2. **`package.json`**: no `semanticTokenScopes` change — the 14-entry map
   already matches the legend exactly. No snippet contribution required
   for alignment (add only if the batch wants them). Version bump for the
   grammar fix is a phase-2 release decision.
3. **`language-configuration.json`**: **no changes** — brackets,
   auto-close, indent and folding rules are all version-independent and
   still correct for `?T`/`[?T]`/`{}`/when-arm syntax.
4. **`test/fixtures/symbols.rut`**: extend with current-surface decls so
   the symbol/semantic assertions exercise the new language: a `struct`
   with a `?T` field, a `str`/`bytes` param, a
   `<T requires …>` union-bound generic, an `opaque`-typed binding.
   Keep the existing five asserted symbols so the old assertions hold.
5. **`test/runTest.js`**: add (a) a grammar smoke — assert `struct`
   keyword-scoped, `dataclass`/`where` NOT keyword-scoped, `str`/
   `bytes`/`opaque` primitive-scoped, `string` not primitive-scoped
   (via `vscode.provideDocumentSemanticTokens` + a TextMate-registry
   tokenize, or the `vscode-textmate` devDep); (b) the corpus gate of §4.
6. **Corpus-pass gate (extension side)** — mirror
   `crates/rut-lsp/tests/corpus.rs` through the wasm path: for every
   `rut/**/*.rut`, `examples/**/*.rut`, `demo/src/examples/*.rut`:
   `rut_analyze` yields **zero error diagnostics**; Impl-mode files yield
   document symbols; the token stream is non-empty and in-file. Runs
   inside `npm test` (`scripts/run-vscode-test.mjs` already launches the
   host with the extension folder as workspace — the corpus is a
   documented `../..` walk away, same roots as the Rust gate). The wasm
   module and the native server share `crates/rut-lsp` queries, so this
   gate cannot pass while the two faces drift — it is the alignment lock.
7. **`README.md`** (extension): after phase 2, refresh the "How
   highlighting splits" keyword/primitive claims (it is generic today,
   so this is polish, not a fix).
8. **`src/extension.ts` / `src/wasm.ts`**: **no edits** — they are
   grammar-agnostic transport/provider wiring; every misalignment above
   is data (grammar JSON) or lives in the Rust classifier (task A's lane).

---

## 4. The extension's own test suite — coverage and the gate

What `test/runTest.js` covers today (5 checks, real VS Code host):
1. activation fires on `onLanguage:rut`;
2. `languageId === 'rut'`;
3. semantic-token legend contains `enumMember` and ≥ 100 tokens arrive
   (proves the wasm module loaded and answered);
4. document symbols include `Color, Point, Circle, Drawable, main`;
5. a broken doc (`fn broken(: nil {`) yields an `Error` diagnostic with
   `source === 'rut'`.

What it does **not** cover: the TextMate grammar (zero scope
assertions), any post-0043/0044 construct (the fixture predates them),
hover/completion behavior, and the corpus — it runs exactly one 21-line
fixture. It also silently depends on `bin/rut-lsp.wasm` existing:
without `npm run build:wasm`, activation no-ops with an info message and
checks 3–5 fail.

A corpus-pass gate on this side (design, per §3 item 6): enumerate the
same roots as `crates/rut-lsp/tests/corpus.rs` (>= 15 files today —
`examples/**` + `demo/src/examples/**`, plus the `rut/**` projects),
open each in the test host, and assert zero error diagnostics from
`rut_analyze` + non-empty symbols/tokens + a grammar-smoke tokenization
per file. That turns `npm test` into the extension-side twin of the
Rust-side corpus gate, using the same shared classifier — the cheapest
durable "cannot drift" lock for the grammar.

---

## 5. Foreign-work census (operational rule)

- `git status --porcelain=v2` at survey start (HEAD `4aebe24`,
  `master` == `origin/master`, +0/−0): **clean** — zero uncommitted or
  untracked non-ignored files anywhere, including
  `integrations/vscode-extension/`. Nothing was stashed, reverted,
  committed, or touched by this task; all 18 extension files were read
  at HEAD.
- Untracked-but-gitignored build artifacts inside `vscode-extension/`
  (`out/extension.js`, `bin/rut-lsp.wasm`, `rut-vscode-0.2.0.vsix`,
  `node_modules/`): consistent with the extension's `.gitignore` — build
  output, not work-in-progress. Left as-is.
- **One repo-level stash exists** and is flagged for the orchestrator:
  `stash@{0}` — *"On master: batch-plan-impl: auto-stash 2026-09-19
  (rut-lsp changes)"* (commit `ec9a8b4`), touching `Cargo.lock`,
  `crates/rut-lsp/Cargo.toml`, `crates/rut-lsp/src/lib.rs`
  (+26/−7). That is the rut-lsp/wasm lane (TASK A / the parallel
  session), **not** `vscode-extension/` — reported, not popped, not
  dropped.
- Conclusion: **no uncommitted foreign work inside
  `integrations/vscode-extension/`** at any point during this task.

---

## 6. Deviations

- None from the task spec. Notes: (a) the corpus for §2 citations was
  read beyond `rut/**/*.rut` into `examples/`, `demo/src/examples/`, and
  `benches/` — the task's "etc." — because the `struct`/`type`/`opaque`
  surface lives there; `rut/**` alone cites for M3/M4/M7/M8. (b)
  `cargo test --workspace` was run once before the commit and is green
  (exit 0) — this task's change is this docs file only.

---

## 7. Phase 2 fix log — grammar alignment (IMPLEMENTED)

Batch `rut-lsp-align`, phase 2, extension task. Scope: `syntaxes/rut.tmLanguage.json`
+ the new standalone corpus gate + runner wiring. `language-configuration.json`
was re-verified per §3.3 and **not churned** (brackets/auto-close/indent/folding
all still correct for `?T`/`[?T]`/when-arm syntax); `src/` untouched
(grammar-agnostic per §1); `package.json` `semanticTokenScopes` untouched
(14/14 exact per §1). Version bumped 0.2.0 → 0.2.1 (the §3.2 phase-2
release decision).

### 7.1 Dispositions M1–M8 (all fixes in `syntaxes/rut.tmLanguage.json`)

| # | Fix | Rule |
|---|---|---|
| M1 | `dataclass` removed, `struct` added | `#keyword` |
| M2 | `where` removed | `#keyword` |
| M3 | `string` removed; `str`/`bytes` added; `opaque` added as a **sibling** `#builtin-type` (`storage.type.builtin.rut`) — keeps call-shaped `opaque(v)` readable while type positions still color | `#primitive-type`, `#builtin-type` |
| M4 | new `#nullable-type`: `?`/`??` followed by `[A-Za-z_[<]` → `storage.type.nullable.rut` (TextMate approximation; exact classification stays with semantic tokens; must sit **before** `#operator` in the include order to win the tie on `?`) | `#nullable-type` |
| M5 | new `#type-alias`: anchored `^\s*(type)(?=\s+[A-Za-z_<])` → `keyword.other.rut` — contextual, never blanket (an identifier named `type` stays plain) | `#type-alias` |
| M6 | `..` added to the operator alternation (before `.` falls through to punctuation; single `.` stays `punctuation.separator`) | `#operator` |
| M7 | no dedicated surface — fixed transitively by M3 (the PrimMap bound lists were the `str`/`bytes` misrender) | — |
| M8 | new `#builtin-keyword`: `\b(builtin)(?=\s+(?:fn|primitive|impl|trait)\b)` → `keyword.other.rut` (decl-head contextual; the corpus's comment prose "builtin container"/"builtin traits" stays comment-scoped) | `#builtin-keyword` |

Corpus-sourced before/after spans (HEAD grammar vs fixed grammar, same
tokenizeLine, spans as `[text]→scope`; unchanged tokens elided):

| Site | Before → After |
|---|---|
| M1 `demo/src/examples/dataclasses.rut:12` `struct Point {` | `[struct Point ]→(none)` → `[struct]→keyword.other.rut` (also todolist.rut:16; 12 files use `struct`) |
| M2 `let where = 1;` (synthetic — `where` occurs only in comments; it is a legal identifier now) | `[where]→keyword.other.rut` → `[ where ]→(none)` |
| M3 `rut/nmap_host/nmap.d.rut:61` `pub host fn map_entry_s(m: opaque, k: str) -> i32;` | `[ opaque]→(none)`, `[ str]→(none)` → `[opaque]→storage.type.builtin.rut`, `[str]→storage.type.primitive.rut` |
| M4 `rut/pouch/pouch.rut:44` `let mut buf: [?T] = [nil; n];` | `[?]→keyword.operator.rut` → `[?]→storage.type.nullable.rut` (`T` stays plain at first paint; semantic tokens classify it once loaded) |
| M5 `demo/src/examples/type-aliases.rut:13` `type Meters = i64;` | `[type Meters ]→(none)` → `[type]→keyword.other.rut` |
| M6 `let r = 0..10;` (synthetic — the corpus's only `..` sits inside a comment, nmapset.rut:474) | `[.]→punctuation.separator.rut [.]→punctuation.separator.rut` → `[..]→keyword.operator.rut`; single `.` keeps `punctuation.separator.rut` |
| M7 `rut/nmapset/nmapset.rut:390` `PrimMapI64<K requires … \| str \| bytes>` | `[str]→(none) [bytes]→(none)` (inside the plain bound run) → `[str]→storage.type.primitive.rut [bytes]→storage.type.primitive.rut` |
| M8 `rut/core/core.d.rut:35` `builtin fn on_drop<T>(p: ?T, cleanup: fn(?T)) -> nil;` | `[builtin ]→(none)`, `[?]→keyword.operator.rut ×2` → `[builtin]→keyword.other.rut`, `[?]→storage.type.nullable.rut ×2` (also core.d.rut:92 `builtin primitive opaque {` → `[builtin]→keyword.other.rut [opaque]→storage.type.builtin.rut`) |

### 7.2 The standalone corpus gate — `test/grammar-corpus.js`

Per the phase-2 scope decision, the gate asserts the **TextMate grammar
directly** (vscode-textmate 9.2 + vscode-oniguruma 2.0.1, the same engine
VS Code embeds, added as devDependencies) — it does **not** go through
`rut-lsp.wasm` (that binary is being rebuilt by the parallel task; the
through-wasm e2e gate is phase 3 per §3.6). Plain `node`, no VS Code host.

- Corpus: `rut/` + `examples/` + `demo/src/examples/` + `benches/workloads/`
  — the same roots as `crates/rut-lsp/tests/corpus.rs` (**53 files**,
  49,053 tokens asserted per run; the walk asserts ≥ 50 files so it cannot
  fail open).
- Corpus-wide invariants: `dataclass`/`where` never keyword-scoped;
  `string` never type-scoped; every code-position `struct` →
  `keyword.other.rut`, `str`/`bytes` → `storage.type.primitive.rut`,
  `opaque` → `storage.type.builtin.rut`, `builtin` → `keyword.other.rut`,
  `?` → `storage.type.nullable.rut`; every `^\s*type\s+` line has its
  introducer keyword-scoped. Comments/strings (incl. f-string walls) are
  excluded from the positive checks and included in the negative ones.
  Bite counts prove non-vacuity: struct×20, str×200, bytes×49,
  opaque×206, builtin×16, `?`×85, type-alias lines×2, dead words×0.
- 17 smoke assertions pin what the corpus cannot exercise (dead-word
  identifiers, `?[T]`, `0..10`, single `.` stays punctuation, `type` as
  identifier, comment-wins rule order, f-string hole re-enters
  `source.rut`).
- Runner wiring: `npm test` = `test:grammar` (this gate) + `test:host`
  (`scripts/run-vscode-test.mjs`); the host launcher now **skips loudly**
  (exit 0) when no `code` executable exists instead of crashing — the
  five host checks themselves are unchanged.

### 7.3 Phase-2 deviations

- §3.4's recommended `test/fixtures/symbols.rut` extension is **deferred
  to phase 3** with the through-wasm gate: it only pays off through the
  wasm symbol/semantic path, which is task A's mid-flight lane; the old
  assertions keep passing as-is.
- §3.5's option of running the grammar smoke inside the host was replaced
  by the standalone runner (scope decision above); `test/runTest.js` is
  untouched.
- §3.7 README polish applied to the two stale claims: the "How
  highlighting splits" TextMate row now names the new keyword/primitive/
  nullable surface, and the Tests section documents the two-tier gate.
- The host-suite portion of `npm test` **skips** on machines without a
  `code` binary (this run included) — environment limitation, not a pass;
  the grammar gate (the phase-2 deliverable) ran and passed.

---

## 8. Phase 3 fix log — the through-wasm e2e gate + fixtures (IMPLEMENTED)

Batch `rut-lsp-align`, phase 3, extension task. Scope: the gate deferred
by §7.2 (§3.6's design), the fixture refresh deferred by §7.3 (§3.4's
design), the npm wiring, and the README's Tests claims. No grammar or
`src/` provider changes — the gate drives the already-aligned shipped
artifact; it adds no new code paths to the extension itself.

### 8.1 The gate — `test/e2e-wasm.js`

Exactly §3.6's corpus-pass design, realized against the rebuilt binary:

- **The real binding, not a copy.** `esbuild.mjs` grew a second entry:
  `src/wasm.ts` is bundled standalone to `out/wasm.js` next to
  `out/extension.js`, and the gate `require`s it and drives the SHIPPED
  `bin/rut-lsp.wasm` (the phase-1 rebuild, 568,271 B) through
  `RutWasm.load` — the same class `src/extension.ts` activates. A copy
  of the ABI would let binding drift hide; this cannot.
- **Corpus walk**: `rut/` + `examples/` + `demo/src/examples/` +
  `benches/workloads/` (53 files, floor ≥ 50, same roots as
  `crates/rut-lsp/tests/corpus.rs`). Per file: `rut_analyze` yields ZERO
  diagnostics; non-empty token stream; every Impl-mode file yields
  document symbols. URIs are the absolute paths — the `.d.rut` suffix
  survives, so `mode_of` picks Decl for the decl surfaces (a mode mistake
  would light them up as diagnostics — the zero-diag assertion locks it).
- **Result through the shipped artifact: 53/53 clean, 303 document
  symbols, 0 false diagnostics.** (The artifact this batch started with
  — frozen at `e41915d` — flagged 15/53 files with 1,796 false diags;
  see the companion survey §2/§4.)
- **Hover/completion smoke** (7 assertion groups, all against the
  shipped wasm): the 14-type legend (incl. `enumMember`); the fixture
  (§8.2) analyzes with zero diagnostics and yields its symbols; the
  RFC 0043 alias hover renders its nullable target — hovering `Maybe`
  on `type Maybe = ?i32;` returns `type Maybe = ?i32;` (M4's `ty_src`
  arm; the pre-align builds printed `type Maybe = ;`); `str` hovers its
  current `primitive str { … }` form (the stale binary said
  `builtin str`); member hover AND completion resolve through a
  `?Circle` let-binding — hover on `c.area()` returns
  `fn area(self) -> f64` in `impl Circle`, completion after `c.` offers
  `area` + `r` (M4's `ty_head` arm, mirroring `crates/rut-lsp`'s
  `nullable_let_binding_resolves_members` regression through the binary
  users actually run); bare completion (a user project with no workspace
  index) offers nmapset's `HashMap`/`HashSet`/`PrimMapI64` and
  nmap_host's `map_entry` (M3's acceptance, the std-surface 8/8 fix);
  and the RFC 0044 law — `fn f(p: i32?)` gives exactly ONE dedicated
  diagnostic naming RFC 0044, `fn g(p: ?i32)` analyzes clean (M1/M2).
- **Failure is loud, never a skip.** Missing `bin/rut-lsp.wasm` exits 1
  with `npm run build:wasm`; a missing `out/wasm.js` exits 1 with
  `npm run compile`. The gate's whole point is the shipped artifact —
  silently skipping on a stale/absent binary would re-create the exact
  rot this batch fixed.
- **Headless by design**: plain node, no VS Code host. Where `code`
  exists, `test:host` still runs its five UI-provider checks on top; on
  headless boxes it skips loudly (exit 0) as before — the e2e gate is
  what makes `npm test` an honest wasm gate everywhere.

### 8.2 The fixture — `test/fixtures/symbols.rut` refreshed

The §3.4 design, landed: the five host-asserted symbols keep their
names (`Color`, `Point`, `Circle`, `Drawable`, `main` — `test/runTest.js`
assertions unchanged), and the file grows the current surface: `impl
Circle` with a method (methods live in impl blocks — RFC 0012), the
`type Maybe = ?i32;` alias (the e2e gate's `?T` hover target), `struct
Slot { item: ?Circle; }` (RFC 0009 v1.1 `struct` + RFC 0044 nullable
field), `fn tag(s: str, b: bytes)` (RFC 0004 v1.1 primitives),
`fn kind<T requires Circle | Point>` (RFC 0043 inline union bound),
`fn widen(o: opaque) -> ?Circle` with `opaque.downcast<Circle>` (RFC
0014), and nullable locals in `main`. The fixture analyzes clean through
the shipped wasm (the e2e gate asserts it), so both suites now exercise
post-0043/44 language on every run.

### 8.3 Wiring and README

- `npm test` = `test:grammar` → `compile` → `test:e2e` → `test:host`:
  the TextMate lock, a fresh bundle (both entries), the through-wasm
  gate, then the host suite (loud-skip without `code`). No new
  dependencies; `package-lock.json` untouched.
- README Tests section documents the three-tier gate, the fixture's
  role, and the honest skip semantics.

### 8.4 Verification and deviations

- `npm test` on this box: grammar-corpus PASS (53 files / 49,053
  tokens / 17 smoke assertions / 0 violations), e2e-wasm PASS (53 files,
  0 false diagnostics, 303 symbols, 7 smoke assertions), host skipped
  loudly (no `code` binary — environment limitation, per §7.3; the five
  host checks are unchanged and their subject — the wasm module — is
  fully covered by the e2e gate driving the same artifact).
- Repo gates at the phase-3 commit: `cargo test --workspace` green (77
  suites, 479 tests), `cargo check --workspace --target
  wasm32-unknown-unknown` Finished/exit 0, and the full bench sanity
  pass — every row's checksum equal to `expected.json` on all three
  runtimes, fuel + VM-heap BIT-IDENTICAL to the standing pins (phases
  1+2 touched build cfg; the host paths did not move). Recorded in the
  batch report (`docs/lsp-align-report.md`).
- Deviation from §3.6's letter: the corpus gate runs headless through
  `RutWasm` directly instead of inside the Extension Host (opening 53
  documents through the UI). The assertions are the same and the module
  is the same binary; the host-side checks stay `test:host`'s five.
- `src/extension.ts`, `src/wasm.ts`, `syntaxes/rut.tmLanguage.json`,
  `language-configuration.json`, `package.json`'s
  `semanticTokenScopes`: untouched.
