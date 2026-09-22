# lsp-features — phase(0) survey + design

Phase 0 of batch `lsp-features`, written 2026-09-22 against tree
`8d9ee8a` (master, clean). Docs-only: no server/engine/extension code
touched; both standing gates verified green on the untouched tree
(`cargo test -p rut-lsp` 44 passed incl. the 3-test corpus gate;
`test/e2e-wasm.js` PASS — 58 corpus files, 0 false diagnostics, 353
symbols, 7 smoke assertions through the shipped `bin/rut-lsp.wasm`).

Every capability claim in §1/§2 was executed against the shipped
artifact (probe scripts: `/tmp/opencode/batch-lsp-features/p0/probe*.js`,
re-runnable via `node <probe>` — they drive the wasm through the
extension's own binding, `out/wasm.js`, no tree changes), plus source
reading of the three faces. The scoping ruling stands: **"type
inference" is DISPLAY-side only** — surface what can be derived from the
parser's AST + the LSP's own heuristics; any engine-side inference
change is a separate language batch.

---

## 1. The capability table (honest, verified)

rut-lsp serves two faces from one query layer (`crates/rut-lsp`):
the native stdio server (`server.rs`, tower-lsp-server) and the wasm
shim (`rut-lsp-wasm`, raw ABI) driven by the VS Code extension
(`integrations/vscode-extension`), which registers vscode providers
per-feature. A capability exists in all three places or it does not
exist.

### 1.1 Handled today (dispatch + capability + ABI + provider all present)

| Method (LSP) | Native server | wasm ABI | extension provider | Notes |
|---|---|---|---|---|
| `initialize` / `initialized` / `shutdown` | ✔ | (n/a) | (n/a) | FULL-sync declaration; workspace scan spawned off the hot path |
| `textDocument/didOpen` / `didChange` / `didClose` | ✔ | `rut_analyze` / `rut_forget` | onDidOpen/Change/Close | FULL text sync, last change wins; 200 ms debounce extension-side |
| `textDocument/publishDiagnostics` (pushed) | ✔ | in `rut_analyze` | DiagnosticCollection | **parser-only** — checker-level errors invisible (menu M5, still open) |
| `textDocument/semanticTokens/full` | ✔ | in `rut_analyze` | registerDocumentSemanticTokensProvider | 14-type legend, in-file, no range/delta requests |
| `textDocument/documentSymbol` | ✔ | in `rut_analyze` | registerDocumentSymbolProvider | nested; `ModuleLet` included as a symbol |
| `textDocument/hover` | ✔ | `rut_hover` | registerHoverProvider | see §1.2 for exactly what resolves |
| `textDocument/completion` | ✔ | `rut_complete` | registerCompletionItemProvider | trigger `.`; member items share hover's receiver inference; bare = keywords + std surface (not locals) |

### 1.2 Not handled (no dispatch, no capability flag, no ABI export, no provider)

`definition`, `typeDefinition`, `references`, `inlayHint`,
`signatureHelp`, `documentHighlight`, `rename`, `codeAction`,
`formatting`, `foldingRange`, `codeLens`, `prepareCallHierarchy` —
verified absent in all three faces (grep + the wasm export list:
`rut_begin, rut_alloc, rut_legend, rut_analyze, rut_forget, rut_hover,
rut_complete, rut_add_def, rut_doc_len`).

Consequence the plan's phase 2 must absorb: **go-to-definition needs
extension-side wiring, not just a server method.** The plan's "no
extension code change expected" is wrong for the wasm face — VS Code's
ctrl+click does nothing without a registered definition provider, and
this extension registers providers explicitly. Phase 2's wiring list:
server impl + capability flag, a `rut_definition` ABI export, a binding
method in `wasm.ts`, `registerDefinitionProvider` in `extension.ts`,
and the e2e smoke group. Same shape (one export + one provider line)
for inlay hints in phase 3.

### 1.3 Hover TODAY, precisely (probe-verified through the shipped artifact)

What resolves (clean-parse sources, 0 diags in every case):

- member hovers on receivers whose binding resolves:
  - param with declared type — field reads AND methods
    (`p.x` → `x: f64 · in Point`; `c.area()` → `fn area(self) -> f64 · in impl Circle`);
  - `let c = Circle.new(..)` — method calls (long pinned by unit tests)
    and **field reads too** (`d.r` → `r: f64 · in Circle`; member
    completion on the same receiver returns `["area","grown","new","r"]`).
- type-name hovers: class/struct/trait/enum/builtin/alias (incl. `?T`
  targets), `str`/`bytes` via the embedded primitive decls, and
  use-imported std types (`Vec` → the pouch decl, origin `pouch:23`).
- free-fn and impl-method names by unique match; ambiguity renders a
  "multiple definitions" list. Enum member access (`Color.Red`) shows
  the whole enum.

What is NULL today (the batch's real hover gaps):

1. **primitive int/float/bool type tokens** — hovering `i32`/`u8`/`f64`/
   `bool` in type position returns nothing (they have no surface decl to
   index; `str`/`bytes` do). This is menu M7.
2. **every plain identifier hover** — `let`-bound locals, params at use
   or decl sites, for-of iter vars, module-level lets: all NULL. There
   is no local-variable hover at all, so no "fully inferred type on
   identifier hovers" either.
3. **field DECLARATION sites** — hovering `item` inside
   `struct Slot { item: ?Circle; }` is NULL (member lookup needs a dot
   context; a decl site has none).
4. **receivers bound from non-`new` initializers** — `let w = c.area;`
   (field read), chained call results (`c.grown(2.0).r`), and for-of
   iter vars as receivers: member hover AND member completion both miss
   (completion falls back to the bare keyword/std list — worse than a
   clean miss, it invites wrong members).

Correction to the landed menu note: "member hover resolving only on
METHOD-CALL sites (not field reads)" is **stale**. Field reads resolve
fine wherever the receiver's type resolves; the actual gap is receiver
*inference coverage* (cases 4 above). Phase 1 should be scoped against
that, not against a method-only rule.

---

## 2. The infra census (what exists to build on)

### 2.1 The shared pipeline

Both faces run one pure per-document pipeline
(`analysis.rs::analyze`): `normalize → lex → parse(mode) → {classify,
symbols, hover-index}`. `Mode::Decl` for `.d.rut`, `Mode::Impl`
otherwise. Every query (`hover_at`, `complete_at`) re-runs lex+parse
per request (`doc_ctx`) — documents are corpus-sized, this is cheap,
and it means **any new feature is one more pure function over
(tokens, AST, index)** with no cache-invalidation problem.

The LSP never runs `rut-lir`. Diagnostics are parser-only (M5 open).

### 2.2 The AST (`rut-ast`)

- **Spans on every node** (`ast.span(NodeId)`), names are `IdentId`s
  into an interner — **name idents carry no spans**. A declaration's
  name span is recoverable by scanning the token stream inside the
  node's span (the LSP already holds `toks` and does exactly this kind
  of scan in `lookup.rs`). No parser changes needed — consistent with
  the tooling-only lane.
- Relevant node shapes: `LetStmt { is_mut, name, destructure, ty: Option,
  init }`, `ParamData { name, ty: Option }`, `SelfParamData`,
  `FieldDeclData { name, ty }`, `MethodDeclData { name, params, ret,
  body }`, `AliasData { name, target }`, `Use { pkg, names }`,
  `ModuleLet { vis, name, ty, init }`, `Enum { name, members: [(IdentId,
  Option<i64>)] }`, `PathSeg { name }`.

### 2.3 The definition index (`hover/types.rs`, built by `hover/build.rs`)

`DefIndex` — **module-level declarations only**:

- `types: Vec<TyDef>` — name, form (class/struct/trait/enum/builtin/
  primitive/builtin-trait/host-struct/alias), generics, `fields`/`
  methods` as `MemberSrc { name, src, doc, line }` — **`line` is 1-based
  text only; members carry NO byte spans** — plus the whole-decl `span`.
  Enum members are name-only, no spans (build.rs notes "spans would
  need token recovery").
- `fns: Vec<FnDef>` — name, verbatim signature, doc, `owner` (Some for
  impl-block methods), **decl `span`**, line.
- `impls: Vec<ImplDef>` — trait/target heads (the use-both gate's data).
- `origin: String` — file path or std pkg label.
- **`ModuleLet` and `Use` are skipped** (`build.rs` line 360). Module
  lets appear in documentSymbol but not in the index.

So: the index answers "what does this NAME mean at module scope" for
types/fns/impl-methods, with spans good enough for a go-to-definition
response on those decls. It does not answer locals, params, fields,
enum members, module lets, or use-path edges.

### 2.4 Local inference (`hover/infer.rs::infer_local`)

Name-based, on-demand, per query: find the enclosing fn/method body
(smallest containing span), check its params by name (declared `ty`
only), then scan `let`s before the cursor — **flat walk, first match
wins** (no scope-exit, no shadow tracking). Returns a **type-head
string only** (`"Circle"`), never spans. `init_ty` handles exactly
three initializer shapes: `Type.new(..)`, struct literals, string
literals. This is the entire inference engine behind member hover AND
member completion today — and its thinness IS cases §1.3-4.

### 2.5 Module resolution: none in the LSP

There is no `use`-path → file machinery in rut-lsp. Cross-file answers
come from **flat name matching** over a lookup chain: [open doc → std
surface → workspace files], ambiguity = the candidate list. The
workspace slice is built by the **server's fs-walk**
(`collect_rut_files`, skips dot-dirs/`target`/`node_modules`, cap 500)
or the **extension's twin** (`workspace.findFiles('**/*.rut')`, same
exclusions and cap → `rut_add_def`, re-index by origin replaces). The
ENGINE does resolve `use pkg::Name` — `rut-driver` reads `rut.toml`
manifests (`name` fields; `[deps]`) — but none of that is plumbed into
the LSP.

### 2.6 The std surface (`std_surface.rs`)

All 8 stdlib packages embedded at compile time via `include_str!` of
the real sources (`core`/`calc`/`nmap_host`/`rt`/`bench_cross` decl
surfaces, `pouch`/`nmapset`/`ink` impl sources), indexed once into
DefIndexes with origin = pkg name. Two consequences the batch can ride:

- the embedded strings ARE the real file contents, so a stdlib
  definition jump can carry the true `rut/...` path + line, not a
  synthetic label;
- std decls are already in the lookup chain under their exact `use`
  pkg names.

### 2.7 The engine typer (`rut-lir/src/check`) — why display-side ≠ wire-in

The checker is `resolve + collect` followed by **fused
typecheck+codegen** into LIR, then monomorphization (`Inst`). Types end
up on LIR registers, not on AST nodes; the only NodeId-keyed type data
is `lambda_sigs`/`lambda_info`. Module `lets` are
`Vec<(IdentId, Option<Ty>, init)>` — spanless. Running the real checker
inside the LSP would mean running a monomorphizing compiler (with its
zero-diag gate) per keystroke to produce data it does not retain
per-node. **Confirmed: the scoping ruling is also the engineering
answer.** Display-side inference extends `infer_local`'s AST heuristics
with a proper binding pass (§3), not by wiring `rut-lir` in.

### 2.8 Position plumbing

`line_index.rs` converts byte offsets ↔ (line, UTF-16 char) both ways —
the LSP's position currency; shared by every face. New features need
nothing here.

---

## 3. The definition-index design

### 3.1 Where it lives

**Server-side per session, as an extension of the existing per-document
`DefIndex`** — both faces already hold a `Vec<DefIndex>` (open doc
rebuilt per query via `doc_ctx`; std surface + workspace in
`defs`/`state().defs`). No new cache layer, no new invalidation story:
FULL sync means a document's index is a pure function of its text, and
the per-query rebuild already absorbs edits for hover/completion today.

### 3.2 Two layers

**Decl layer (extend what exists).** One pass over module items —
already `hover::index`'s walk; extend it:

- field declarations: record the byte span (the `FieldDeclNode` has one;
  strip modifiers like `field_members` already does) — today only
  `line` survives;
- enum members: recover name spans by token scan inside the enum's span
  (the recovery build.rs anticipated; no parser change);
- `ModuleLet`: index name + span + declared/init type head (module-scope
  bindings are real definitions);
- `Use { pkg, names }`: record imported names and their span — they are
  definition targets too (ctrl+click on `Vec` in the use statement jumps
  to the pouch decl).

**Binding layer (new).** Per-fn (and per-method) local bindings:

```
Binding { name, decl_ident_span, scope_span, kind: Let|Param|IterVar, ty_head: Option<String> }
```

built in the same AST walk (fn bodies → blocks with their spans; the
block tree is already walked by `infer_local`/`walk_lets`). Resolution
is the standard rule `infer_local` lacks: **nearest containing scope,
latest declaration before the cursor** — shadowing-correct, scope-exit
correct. This pass is exactly what phase 1's hover work needs
(§4.1), so it lands there, **span-first**, and phase 2's
within-file definition is a lookup against it, not a rebuild.

### 3.3 Resolution order for `definition(pos, ident)`

1. **Within-file binding layer** — locals/params/iter-vars (shadow-aware).
2. **Within-file decl layer** — fns, types, impl methods, fields (via
   the enclosing type), enum members, module lets, use-statement names.
3. **Cross-file through the use graph** — parse the document's `Use`
   items; for each imported name, search the index chain restricted to
   the named pkg: a workspace file whose path/origin matches the pkg
   name (directory or file stem — matching how `rut.toml` `name` works
   without re-implementing the manifest loader), else the embedded std
   surface whose origin is the pkg. Un-imported documents fall back to
   the current flat chain (same ambiguity semantics as hover: multiple
   hits → a candidate list, never a wrong silent jump).
4. **Stdlib** through the embedded surface — and because the surface is
   `include_str!` of the real sources, the response carries the true
   `rut/...` path + line; a workspace that IS the rut repo gets a real
   jump into `rut/pouch/pouch.rut`.

Honest limits (document in README at close-out, not silently): no
visibility checking (the engine's `pub(mod)`/`pub(super)` rules are not
modeled — a name-match may resolve where the compiler would reject);
generics resolve at the head (`Vec` not `Vec<T>`'s instantiation).

### 3.4 Update story on edit

Nothing to build: FULL sync → the doc's text changes → the next query
re-parses and re-indexes (both faces already do exactly this for
hover). The wasm arena model (per-request reset) already caps the
memory story. If profiling ever shows per-query re-parse hurting on
huge files, memoize the Analysis per (uri, version) — a pure
optimization with no semantic risk. Not needed at corpus sizes.

---

## 4. Per-feature designs + costs

Cost scale: **S** = days-half, touches one face layer + gates;
**M** = a phase, shared-query work + ABI/provider wiring + e2e smoke
group + artifact re-issue; **L** = multi-phase or engine-adjacent.

### 4.1 Phase 1 — hover: fields, M7, inferred types (M)

All work in the shared query layer; both faces get it free; gates as
planned (corpus clean, e2e smoke groups, wasm rebuild, vsix re-issue).

- **Local binding pass** (§3.2) lands here, span-first. This is the
  phase's real deliverable even though the user sees only hover.
- **Identifier hovers** = binding-layer hit → render the inferred type:
  declared annotation if present, else the `init_ty` heuristic,
  rendered through the existing `ty_src` family (the `?T` memo is
  already correct there). Span = the ident.
- **Field-read receiver inference** — extend `init_ty`:
  field-read initializers (`let w = c.area` → resolve `c`, take the
  field's declared type), unique fn-call results (fn name match →
  declared ret type), for-of iter vars (iterable's element head:
  `[T]` → `T`, `Vec<T>` → `T`), chained method-call results via ret
  type. This closes §1.3-4 for hover AND completion at once.
- **Field declaration hovers** — decl-site ident → owning `TyDef`'s
  `MemberSrc` (the decl layer from §3.2). Also makes enum-member decl
  sites answerable.
- **M7 primitive hovers** — the int/float/bool primitives have no
  surface decl to point at; synthesize truthful static hover text
  (width, min/max, literal suffix forms — from `rut-core`'s documented
  surface, rendered in the same markdown shape). Self-contained; no
  engine reading.

### 4.2 Phase 2 — go-to-definition (M–L: the index is M; the three-face wiring is the L part)

- `textDocument/definition` per §3.3: server impl + capability,
  `rut_definition` export, `wasm.ts` method, `registerDefinitionProvider`,
  e2e group asserting request pos → response span == the declaring
  ident — including one cross-file jump and one stdlib jump (the std
  jump targets the real embedded source path).
- `textDocument/typeDefinition` where cheap: expression → resolve its
  type head (receiver rules of §4.1 / `ty_head` on annotations) → that
  type's decl. Land it in the same phase only if it falls out of the
  receiver-resolution work cleanly; it reuses 100% of the machinery.

### 4.3 Phase 3 — inlay hints (M, rides phases 1–2)

- Unannotated `let`s + for-of iter vars where inference yields a type
  head: `InlayHint { kind: Type, position: right of the ident, label:
  ": Circle", tooltip: the hover markdown }`. Skip when an explicit
  annotation exists (never restate).
- Param-name hints at call sites: token-level call-shape detection
  (`name(` / `recv.method(`), callee resolved by the §3.3 chain, param
  names rendered before args; land only exact-arity matches (mismatch →
  no hints, never wrong hints).
- Server capability + `rut_inlay` export + provider. VS Code renders
  natively; the user toggles `editor.inlayHints`. No grammar work.

### 4.4 Phase 4 — references + signatureHelp: SAME-index, priced cheap; both land, references first

Scope-guard verdict: **neither is new-heavy.** Both are queries over
the post-phase-2 index; no new analysis infrastructure.

- `references`: decl → uses = the binding/decl layers reversed. Within
  file: token scan for idents matching the name, filtered by
  scope-containment (shadow-correct, because the binding layer is).
  Cross-file: the use-graph-matched files, name-match with the same
  ambiguity honesty as hover (candidate list over matched files).
- `signatureHelp`: at a call's parens, resolve the callee (§3.3 chain),
  render its `FnDef.src`, `activeParameter` = token-counted commas
  between the open paren and the cursor. Slightly heavier than
  references only in call-shape detection; same-index otherwise.
- Defer trigger (recorded per the plan): only if phase 3's binding-pass
  work slips scope — land references alone and push signatureHelp; do
  not force both.

### 4.5 Explicitly out (recorded, not designed)

- M5 checker-diagnostic surfacing (rut-lir wiring) — stays on the menu;
  engine-adjacent, its own batch.
- Renames/`documentHighlight` — highlight would ride the phase-4
  references scan cheaply if wanted; rename needs edit-application
  machinery (workspace/edit protocol) — new-heavy, defer.
- Engine-side inference improvements (real flow typing) — separate
  language batch per the scoping ruling.

---

## 5. Phase order — confirmed, with one design requirement promoted

**1 hover → 2 definition → 3 inlay hints → 4 references/signatureHelp →
5 close-out. Confirmed.** The user-visible priority (hover fields, then
ctrl+click, then inline inference) matches the dependency flow, and each
phase lands user value through the shipped artifact with its own gate.

The one correction: **the local binding pass must land in phase 1,
span-first** (decl ident spans + scope spans, shadow-aware resolve —
§3.2). Phase 1's three deliverables all consume it (identifier hovers,
field-read receiver fixes, field-decl hovers), and phase 2's
within-file definition is then a lookup, not a rebuild. Building it
name-only in phase 1 (the `infer_local` shape) would be throwaway work.

Second correction (plan text, §phase 2): the extension DOES need code —
a definition provider registration plus the wasm ABI export/binding
(§1.2). The "no extension code change expected" line is dropped.

Standing rules carry unchanged: explicit-path staging (this phase
touches `docs/lsp-features-survey.md` only), foreign-lane guard, the
shipped wasm rebuilt + vsix re-issued per behavior phase, e2e-wasm the
loud gate with per-feature smoke groups, corpus floor ≥ 50 files at
zero false diagnostics.
