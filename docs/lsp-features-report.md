# lsp-features — the batch report

Batch `lsp-features`, phases 0–5, closed 2026-09-23. Base `8d9ee8a`,
five phase commits + this close-out. Lane: `crates/rut-lsp*`,
`integrations/vscode-extension` (source + shipped wasm artifact + vsix),
`docs/`. Engine semantics, `expected.json`, the benches, and the
language surface: untouched all batch. Design doc:
`docs/lsp-features-survey.md` (phase 0, `bd0c720`) — its capability
table, infra census, and index design are the batch's contract; this
report is the close-out reading of the same table, re-verified through
the shipped artifact.

## 1. The capability table, BEFORE → AFTER

Method: the survey's honest method, re-run at close-out. Every AFTER
claim below was executed against the SHIPPED `bin/rut-lsp.wasm`
(md5 `8fda65cfae8dabebd681263bf66e64d7`, the 0.2.2 artifact) through
the extension's own binding (`out/wasm.js`, bundled `src/wasm.ts` — the
exact load path `extension.ts` uses, minus the vscode host), by the
close-out probe (`/tmp/opencode/batch-lsp-features/p5/probe.js`:
33 checks, 33/33 PASS — the durable subset of the same assertions lives
in `test/e2e-wasm.js`'s 22 smokes, which run on every `npm test`).
Export-list absence was read from the wasm module's own export table.
The native server and the wasm shim share the queries in
`crates/rut-lsp`, so a capability below exists in all three faces
(server capability flag + ABI export + registered extension provider)
or it does not exist.

### BEFORE (phase 0's table, at `8d9ee8a`, version 0.2.1)

Handled: `initialize`/`initialized`/`shutdown`, full-text
`didOpen`/`didChange`/`didClose`, pushed `publishDiagnostics`
(**parser-only** — checker errors invisible), `semanticTokens/full`
(14-type legend), `documentSymbol`, `hover` (with four recorded gaps:
no identifier hovers, no field-decl hovers, no inferred types, receiver
inference missing for non-`new` initializers), `completion`.

Not handled in ANY face: `definition`, `typeDefinition`, `references`,
`inlayHint`, `signatureHelp`, `documentHighlight`, `rename`,
`codeAction`, `formatting`, `foldingRange`, `codeLens`,
`prepareCallHierarchy` — verified absent by grep + the then-current
wasm export list (`rut_begin … rut_doc_len`, nine exports).

### AFTER (this close-out, version 0.2.2)

| Method (LSP) | Native server | wasm ABI | extension provider | Probe result through the shipped 0.2.2 artifact |
|---|---|---|---|---|
| everything BEFORE (`initialize`, sync, diagnostics, semanticTokens, documentSymbol, hover, completion) | ✔ | ✔ | ✔ | unchanged green (fixture clean; hover + member completion re-answered, 145 member items) |
| `textDocument/definition` | ✔ | `rut_definition` | `registerDefinitionProvider` | binding use → declaring ident byte-exact (`slot` → `let slot`); member read → field decl ident (`slot.item` → `item` in `Slot`); type use → decl (`Maybe` → the alias); primitives honestly `[]` |
| `textDocument/typeDefinition` | ✔ | `rut_type_definition` | `registerTypeDefinitionProvider` | binding → its type's decl ident (`slot` → `struct Slot`) |
| `textDocument/references` | ✔ | `rut_references` | `registerReferenceProvider` | `Circle`'s decl → the EXACT six use sites (impl, `?Circle` field, `Circle \| Point` bound, `?Circle` ret, `downcast<Circle>`, `let c: ?Circle`); `includeDeclaration` appends the decl; shadow sets + cross-file both directions pinned by the e2e |
| `textDocument/inlayHint` | ✔ | `rut_inlay` | `registerInlayHintsProvider` | corpus ground truth: `let d = hex_digits();` → `: Vec<str>` (kind 1 TYPE); `let v = hex_val(..)` → `: i32` + `c:` param hint (kind 2 PARAMETER); annotated lets never restated |
| `textDocument/signatureHelp` | ✔ | `rut_signature_help` | `registerSignatureHelpProvider` | verbatim `fn add(a: i32, b: i32) -> i32`, params as written, active slot 0/1 at first/second args; method receiver through its type (`fn grown(self, k: f64) -> Circle`, self never a slot); mismatch → null |

Still not handled in any face (probe-verified absent from the wasm
export table): `documentHighlight`, `rename`, `codeAction`,
`formatting`, `foldingRange`, `codeLens`, `prepareCallHierarchy` — see
§4, the menu.

The wasm ABI face grew from nine exports to fourteen
(`rut_definition`, `rut_type_definition`, `rut_inlay`,
`rut_references`, `rut_signature_help` added). The extension registers
nine providers (was four): semantic tokens, symbols, hover, completion,
definition, type definition, inlay hints, references, signature help.

## 2. What each phase landed

- **Phase 0** (`bd0c720`) — the survey: the probe-verified capability
  table (§1.1/§1.2), the infra census (§2: the shared
  `normalize → lex → parse → {classify, symbols, hover-index}`
  pipeline; the DefIndex; the thinness of `infer_local`; no module
  resolution in the LSP — flat name chains; the std surface's
  `include_str!` true paths; why the engine typer cannot be wired in),
  and the index design (§3: decl layer + span-first binding layer, the
  three-step resolution order, the update story that costs nothing
  under FULL sync).
- **Phase 1** (`0de7158`) — the span-first **binding pass** (every
  fn-local binding as `{name, decl_ident_span, scope_span, kind, ty}`,
  shadow-aware, scope-exit correct) + the **decl layer** (field/enum
  member name spans, `ModuleLet`s, `Use` name spans, `TyDef`/`FnDef`
  name spans — token recovery, no parser changes). **Hover completed**:
  field decl sites, identifier hovers with inferred types, M7 primitive
  blurbs, receiver inference for field-read/chained/for-of receivers.
  547 lib tests green; 12 artifact smokes.
- **Phase 2** (`4be0199`) — **go-to-definition + typeDefinition** as
  ONE pure query over three layers (binding pass → decl layer → the
  use graph into workspace + std surface; ambiguity = the LSP candidate
  list, never a wrong jump; std jumps land in the real `rut/...`
  sources). The survey's §1.2 correction honored: providers registered,
  `rut_definition`/`rut_type_definition` ABI + bindings. 564 tests; +4
  artifact smokes (def-within/def-cross/def-std/typeDefinition).
- **Phase 3** (`f03575c`) — **inlay hints**: TYPE hints on unannotated
  bindings (the binding pass's `Binding::inferred` + `LetDef::ty`),
  PARAMETER hints at call sites through the phase-1/2 callee machinery
  with FnDef params recorded at index time; **exact-arity only**
  (mismatch → no hints, never wrong); the consistency law gated (a type
  hint's text IS the hover's text; the tooltip IS the binding's hover
  markdown). Fixed en route: written tuple rets rendered empty
  (`ty_src`'s missing `TyTuple` arm). 581 tests; +3 artifact smokes.
- **Phase 4** (`393ddeb`) — **references + signature help**: references
  = the definition index read backwards under ONE law (a token is a
  reference iff go-to-definition from it lands on the target — the two
  faces cannot disagree); cross-file through the use graph's reverse
  edges, both directions; scope-guard verdict CONFIRMED (neither was
  new-heavy). Signature help with **token-level call-shape detection**
  (the AST is unreliable exactly when this fires): innermost unclosed
  paren, depth-0 commas only, verbatim signature, active parameter from
  the comma count, `more commas than params → no help, never wrong`.
  Own-surface finality (a same-named impl fn's params can never leak).
  608 tests; +3 artifact smokes (refs-shadow/refs-cross/sighelp — 22
  total).
- **Phase 5** (this commit) — the close-out: version 0.2.1 → **0.2.2**
  (the batch changed shipped extension source in every feature phase —
  providers, ABI faces, bindings), `npm run package` re-issued, the
  shipped wasm verified current, this report, and the README feature
  notes. No feature code, no engine code.

## 3. The honest limits

What the batch shipped is **parse-level analysis**: one lex+parse per
query over the AST + the LSP's own index. The limits below are
probed/pinned, not silent:

1. **Diagnostics stay parser-only.** Checker-level errors (type
   mismatches, unbound names) remain invisible — menu M5. Display-side
   "type inference" is the scoping ruling held all batch: AST
   heuristics (annotation → init shape → field/call chains), never the
   engine's monomorphizing checker.
2. **References' cross-file limits.** Member and local targets NEVER
   cross files (locals can't escape a body; `use` imports carry
   type/fn names, not members) — probe-verified: a member decl's
   reference set contains only its own file even when another open doc
   reads the same member name. Unimported flat-chain names are scanned
   in the asking file + the declaring file only — the flat chain does
   not pull the whole workspace in. No visibility modeling
   (`pub(mod)`/`pub(super)` unmodeled — a name-match may resolve where
   the compiler would reject). Generics resolve at the head (`Vec`,
   not `Vec<T>`'s instantiation).
3. **Signature help misses mid-typing on chained receivers.** A plain
   ident receiver only: `c.grown(2.0).grown(` — the innermost unclosed
   paren's head is `)`, not an ident, so the receiver cannot resolve
   mid-typing (probe-verified null; same for a multi-segment path
   receiver). Once the call is closed, the AST rules answer. Ambiguity
   is a miss; own-surface callees (builtin/primitive surfaces carry no
   recorded params) are final with no help.
4. **Param hints are exact-arity only.** Fewer/more args than the
   callee's recorded params → no hints and no help (never wrong hints);
   free calls resolve by unique-name match (ambiguity = miss).
5. **Primitive type tokens are un-jumpable by design.** Hover answers
   (the M7 static blurbs: width, range, suffix forms), but
   definition/typeDefinition on `i32`/`bool`/… is honestly empty —
   there is no declaration anywhere to jump to (probe-verified `[]`).
6. **The wasm face's references answer on OPEN docs** (an LSP-correct
   contract, recorded because it surprised the close-out probe: the
   queried uri must be synced via didOpen/analyze; `rut_add_def`-ed
   files supply cross-file TARGETS, and the extension indexes every
   workspace file).

## 4. The menu going forward

- **M5 — checker diagnostics** (`rut-lir` wiring): the largest open
  item, unchanged by this batch. Engine-adjacent; its own batch.
- **documentHighlight**: rides phase 4's references scan almost
  directly (the survey §4.5 priced it cheap when references landed) —
  the natural next candidate.
- **rename**: needs edit-application machinery (WorkspaceEdit across
  files, the workspace/edit protocol) — new-heavy; after
  documentHighlight, riding the same reference sets.
- **codeAction, formatting, foldingRange, codeLens,
  prepareCallHierarchy**: unpriced, not designed.
- **M7 leftovers: none open.** The primitive blurbs landed in phase 1;
  the one residual (§3.5, un-jumpable primitives) is by design —
  recorded so nobody "fixes" it into a lie.
- **The Path-not-Field discovery** (phase 2, recorded for future LSP
  authors): `ExprKind::Field` is the tuple-`.0` form ONLY — dotted
  member access (`wrap.c`) parses as a multi-segment `Path`. Anything
  that reads "field access" from the AST must walk Path segments (it is
  why the receiver rules chain through `path_ty`). The AST's own naming
  will mislead you exactly once; this note is the second time.
- **Engine-side inference** (real flow typing): still out of scope per
  the phase-0 scoping ruling; a separate language batch.

## 5. The release

- Extension version **0.2.1 → 0.2.2** (`package.json` — the batch
  changed shipped extension source in every feature phase: five new
  ABI faces + bindings in `wasm.ts`, five new provider registrations in
  `extension.ts`). `package-lock.json` untouched (its root version was
  already stale at 0.2.0 — the 0.2.1 bump never touched it either;
  dependency pins unchanged).
- `bin/rut-lsp.wasm` rebuilt via `npm run build:wasm`:
  md5 `8fda65cfae8dabebd681263bf66e64d7` — byte-identical to phase 4's
  re-issue (the build is deterministic; phase 5 changed no source, the
  shipped artifact was already current). Before = after md5.
- `rut-vscode-0.2.2.vsix` re-issued via `npm run package`; the embedded
  `extension/bin/rut-lsp.wasm` md5-verified INSIDE the vsix:
  `8fda65cfae8dabebd681263bf66e64d7`, byte-for-byte the shipped binary;
  the bundled `extension/out/wasm.js` md5
  `1760d628bd6ecf7f6cdd5014f7b4a377` equals the local `out/wasm.js`.
  The vsix and the artifact stay untracked per the extension's own
  gitignore, as every prior phase.

## 6. Gates at close-out (all run on the 0.2.2 tree)

- `cargo test --workspace`: **608 passed, 0 failed** (547 base + 17
  definition + 17 inlay + 27 phase-4 references/signature-help; all
  prior suites untouched-green).
- `cargo check --workspace --target wasm32-unknown-unknown`: **exit 0**.
- Extension `npm test`: grammar-corpus **PASS** (58 corpus files,
  56,450 tokens, 17 smoke assertions, 0 violations); e2e-wasm **PASS**
  (58 corpus files, 0 false diagnostics, 353 symbols, 22 smoke
  assertions) through the shipped `bin/rut-lsp.wasm`; `test:host`
  loud-skip (no `code` on this machine) exit 0 — a skip is a skip, the
  other gates ran.
- Close-out probe: **33/33 checks PASS** through the shipped artifact
  (§1's table is that transcript).
- Bench pins untouched: no engine files, no `expected.json` lines, no
  `benches/` edits — git tree clean of them, staged by explicit path.
- The foreign `batch-plan-impl` stash untouched; no parallel-session
  files staged.
