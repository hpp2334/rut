# the type-name-law survey (phase 0)

**Batch:** type-name-law · **Phase:** 0 (docs only) · **Base:** 725f35d · **VERSION:** 11 · **Date:** 2026-09-24

The user's rulings (Sep 24 2026), the law this batch implements:

1. The row grammar is **invalid**: `pub type HashMap<K, i64> = PrimMapI64<K>;` (x3) is not legal
   grammar.
2. "we should fix our grammar parse first, **same name type should be one**, and cannot conflict
   with class, struct and so on".

Phase 1 will remove the alias-row form and restore one-name-one-decl. This survey maps the
terrain it will cross — probed, not assumed. Every probe in §1–§4 ran on this exact tree;
receipts under `/tmp/opencode/batch-type-name-law/p0/`.

---

## §1 the checker census

### §1.1 where duplicate type names are diagnosed

All duplicate-type-name diagnosis lives in **pass 1a** of the module collector,
`crates/rut-lir/src/check/collect.rs` — four sites, one message, per-module scope
(`self.aliases / self.datas / self.enums / self.trait_decls` are collector state; `find_*` are
`crates/rut-lir/src/check/mod.rs:745-753`):

| site | line | guards against |
|---|---|---|
| `collect_enum` | collect.rs:132-135 | enum vs {enum, data, trait, alias} |
| `declare_data` (struct **and** class) | collect.rs:166-169 | data vs {data, enum, trait, alias} |
| `declare_alias` | collect.rs:249-263 | alias vs {alias, enum, trait, data} — **minus the lift (§1.2)** |
| `declare_trait` | collect.rs:284-287 | trait vs {trait, data, enum, alias} |

Diagnosis is a hard error and aborts the decl: `duplicate type name \`X\`` — the same text for
every kind pair, with no cross-kind wording. Plain fns have their own index check
(collect.rs:86, `duplicate fn`); methods too (collect.rs:883/933); impls check the
(trait, type) pair at link (`crates/rut-core/src/link.rs:263`, RFC 0012 §2).

### §1.2 the C2 concrete-shadows-generic lift — the code to remove

The lift is **entirely inside `declare_alias`** (collect.rs:251-263). Landed in bd104f8:

```rust
let is_row = !d.params.is_empty();
let clash = self
    .find_alias(d.name)
    .map_or(false, |a| !(is_row && !a.params.is_empty()))     // leg 1
    || self.find_enum(d.name).is_some()
    || self.find_trait(d.name).is_some()
    || self
        .find_data(d.name)
        .map_or(false, |dd| !(is_row && !dd.generics.is_empty())); // leg 2
```

- **leg 1** — alias-vs-alias collision suppressed when **both** are rows: three rows may share
  the family name (`HashMap<K, i64>` / `<K, u64>` / `<K, f64>`).
- **leg 2** — alias-vs-data collision suppressed when the new alias is a row **and** the
  existing data is generic: a row may share the generic class's name (the class is the
  fallback, the row is "strictly more specific").

**Before the lift** (bd104f8^, verified from git) it was the plain symmetric conjunction:

```rust
if self.find_alias(d.name).is_some()
    || self.find_data(d.name).is_some()
    || self.find_enum(d.name).is_some()
    || self.find_trait(d.name).is_some()
{
    self.err(sp, format!("duplicate type name `{}`", self.name(d.name)));
```

So the error before the lift was the same `duplicate type name` — the nmapset row block simply
could not have compiled beside `pub class HashMap`. **Phase 1 removes exactly the two
`map_or` legs** (restoring the four-way `is_some` conjunction, symmetric with the other three
declare sites). Note the enum/trait legs already have no lift — those pairs need no change.

**The lift is order-asymmetric** (probed, matrix/m_row_class_{ab,ba}): `class HashMap<T>` first
then `type HashMap<K, i64>` is **silent** (leg 2 fires on the alias side), but the row first
then the class **diagnoses** — `declare_data`:166 has no lift (`find_alias(name).is_some()`
without qualification). The landed nmapset shape (class at :183, rows at :570) sits on the
quiet side of its own asymmetry. Same asymmetry for generic structs (probed
m_row_structgen_{ab,ba}). One more asymmetry: leg 1 makes row-vs-row silent in **both** orders,
and a third row too (m_row_row3 — the nmapset x3 shape).

### §1.3 the probed conflict matrix (same module, both orders)

Every pair of the five module-scope kinds, declared in both orders, probed live
(receipts: `probe-battery-receipts.txt`, matrix/). D = diagnosed `duplicate type name`, S =
silent:

| pair \ (first decl →) | plain alias | row | enum | struct | class | trait |
|---|---|---|---|---|---|---|
| **plain alias** | D / D | D / D | D / D | D / D | D / D | D / D |
| **row** | D / D | **S / S** | D / D | **S / D** | **S / D** | D / D |
| **enum** | D / D | D / D | D / D | D / D | D / D | D / D |
| **struct** | D / D | D / S | D / D | D / D | D / D | D / D |
| **class** | D / D | D / S | D / D | D / D | D / D | D / D |
| **trait** | D / D | D / D | D / D | D / D | D / D | D / D |

(cell = second-decl-aside / first-decl-aside; "row" = the row form `type X<K, i64> = ..;`)

Read: everything is diagnosed today **except** the lifted shapes — row×row (both orders), and
row×generic-data in the class/struct-first order only. A row over a **non-generic** class
diagnoses both orders (m_row_classng). Under one-name-one-decl the whole S column/row dies and
the matrix becomes uniformly D — which is what the other five kind pairs already are.

### §1.4 cross-package: what the same-name law meets across module boundaries

Three regimes, probed (xpkg/):

1. **Inline splice (generic-class pkg, `inline = true`)** — the dep's source joins the
   consumer's collector **flat**: a consumer-declared `class HMap<T>` colliding with the
   spliced pkg's `class HMap<T>` **diagnoses** `duplicate type name \`HMap\``, and so does a
   collision with the spliced pkg's *other* class (HStore). The C2 checks are the enforcer at
   package seams for the inline regime — removing the lift does not open a splice hole.
   (Corroborates the hashmap-surface report's "the splice path is flat and the collector
   visibility-blind".)
2. **Extern (non-inline) use** — two pkgs both exporting `struct Pt`, consumer `use`s both:
   **silent** at collect and at link. The name binding is last-use-wins
   (`extern_types` insert), while values carry their real types through fn signatures — so
   `let p: Pt = mk();` then fails at the **use site** with the ambiguous
   `let \`p\` is \`Pt\` but the initializer is \`Pt\`` — two distinct TypeIds that *print*
   identically. No ambiguity diagnostic exists at the `use` or at link.
   (`link.rs` checks duplicate *modules* (:57) and duplicate *impl pairs* (:263) — not names.)
3. **Local declaration shadowing a used name** — `use pga::{ Pt };` plus a local
   `struct Pt` is **silent**: `resolve_type` checks local `find_data` before `extern_types`
   (resolve.rs:236-333 order). The local wins for local spellings; imported fns still return
   the foreign type. No shadowing diagnostic.

These three are pre-existing laws independent of the row form; the ruling's "cannot conflict
with class, struct and so on" is a module-scope declaration law (§1.3). Whether the extern
regimes should *also* diagnose is recorded here as a question, not smuggled into phase 1 — the
type-aliases test suite pins current cross-module alias behavior (`type_aliases.rs`,
`cross_module.rs`).

### §1.5 the RHS hole — why `type Foo<K, i64> = NotAType<K>;` passes

Probed (rhs/):

| probe | result |
|---|---|
| `type B = NotAType;` (plain alias) | **diagnosed at declare** — pass 1b `validate_alias` → `resolve_type` → `unknown type \`NotAType\`` |
| `type Foo<K> = NotAType<K>;` (zero concrete members) | diagnosed — "needs at least one concrete head member" (mod.rs:814-822) |
| `type Foo<K, i64> = NotAType<K>;` **unused** | **silent — compiles clean** (exit 0 with a real `pub fn main`) |
| same, **used** in live code | diagnosed at the first *expansion*: `unknown type \`NotAType\`` with the span **pointing at the alias decl's target** (resolve.rs:360-364 fires through `expand_alias_row`:408) |
| `type Foo<K, i64> = W<K, i64, i64>;` (arity-wrong target), unused | **silent** |
| same, used | diagnosed at expansion, span at the decl (`\`W\` takes no generic arguments`) |
| either variant, used only from a **dead fn** | **silent** — dead fn bodies are never compiled (`let x: i64 = "not an i64"` in an uncalled fn is also silent) |
| row target with a dangling concrete member (`type Table<K, W> = W;`) | silent, expands fine (head members not appearing in the target are accepted) |
| row targeting a non-class (`type Foo<K, i64> = i64;`) | silent at declare **and** use — the "a type-alias row's target must be a class" law (collect.rs:724) fires only in *impl-target* expansion |
| bare file vs mounted pkg (`[deps]`, `use` present) | identical — the hole is checker-side, context-independent |

**Exact cause:** the row form's pass-1b validator `validate_row_target` (mod.rs:950-1009)
walks the target *structurally* — it diagnoses row-chains eagerly (the one head check it
makes, mod.rs:978-991), recurses into `<>` args, and resolves `Walk::Other` leaves — **but its
`Walk::Path` arm never resolves the target head at all**. The doc comment (mod.rs:775-776)
says "the target's own arity/concreteness checks fire at the first expansion (under the real
substitution)" — resolution is deferred to use, and compilation is reachability-scoped, so an
unused (or dead-code-used) row with a nonsense RHS is never diagnosed. The plain-alias arm has
no such deferral (mod.rs:843 resolves immediately) — the hole is row-specific.

**Fix shape:** in `validate_row_target`'s `Path` arm, resolve the head like `row_member_of`
does (mod.rs:926-943) — one arm, pass 1b, diagnosing at the alias decl; the deferral comment at
mod.rs:775-776 retires with it. Under the ruling this is academic-but-recorded: phase 1
removes the row form entirely and the hole dies with it. **If any row-like form ever returns,
this is where its diagnostic belongs.**

---

## §2 the five-kind inventory

The complete module-scope name-introducing axis — `classify_item`
(crates/rut-parser/src/item.rs:25-30) and `classify_pub` (:85-89):

1. `type` — alias (plain / union / **row**, the last removed by phase 1)
2. `enum`
3. `struct` (dataclass)
4. `class`
5. `trait`

Ten unordered pairs × two orders = the matrix of §1.3 — that is the conflict law's complete
axis list, probed end to end.

**Anything else that introduces a type name?** Yes — three `.d.rut`-only surface forms
(crates/rut-ast/src/ast.rs:311-396): `SurfaceDataclass` (`host struct`), `BuiltinTy`
(`builtin Name<..>`), `BuiltinTrait` (`builtin trait`). They live in Decl-mode parses; the
module collector's pass 1a matches only Enum/Dataclass/Class/Trait/Alias and drops the rest
(`_ => {}`, collect.rs:43), so the C2 law does not see them today. They are surface
documentation (the host constructs them; no impl registers), they cannot collide with module
items in the same compile, and recording that fact is this survey's whole action for them —
extending the duplicate check there is out of scope. Everything else at module scope
(`fn`, `let`, `use`, `impl`, `host`, `builtin fn`) introduces non-type names with their own
duplicate checks (§1.1).

---

## §3 the single-concrete-head call: **REMOVE**

With one-name-one-decl, the rows die. Question: keep ONE concrete-head alias under a *unique*
name (partial application, `type I64Cols<T> = Table<T, i64>;`)? **No. Receipts:**

1. **The task's exact spelling is illegal today.** `type I64Cols<T> = Table<T, i64>;` has a
   zero-concrete *head* (the concrete `i64` lives in the *target*) → "type-alias row `I64Cols`
   needs at least one concrete head member — general generic aliases are a non-goal
   (RFC 0043 §5)" (probed, papp/A). Keeping it means *new* grammar (target-concrete members
   with no head member) — new parser + checker + resolution surface, the opposite of a minimal
   phase 1.
2. **The legal row spelling buys nothing.** `type I64Cols<T, i64> = Table<T, i64>;` forces
   every use to re-spell the concrete: `I64Cols<str, i64>` — identical ergonomics to spelling
   `Table<str, i64>` (probed, papp/B-C). Partial application — the one thing that would justify
   the form — is exactly what the form cannot do.
3. **A rows-only family cannot mint.** `I64Cols.new()` and `I64Cols<str, i64>.new(...)` both
   fail `unknown name \`I64Cols\`` — the mint arm looks the base up in `ctx.datas`
   (call.rs:560); a family with no same-named class never resolves. Keeping unique-named rows
   would need a mint-arm extension *too* (probed: with a same-named class fallback it works —
   papp/E — which the one-name law forbids).
4. **Anything kept keeps everything.** The row seam is eight coupled pieces: the alias-head
   grammar (item.rs:201-263), `AliasData.params`, `AliasTarget::Row`, `alias_rows`,
   `expand_alias_row`, `row_mint_target`, `row_target_heads`, `impl_row_target` +
   `validate_row_target`/`row_member_of`. A "one useful row" keeps the entire surface — the
   exact thing the ruling retires — plus its §1.5 hole.
5. **The columns' real value is storage, not spelling** — and it is priced separately (§4),
   where a one-name-compatible menu item exists.

**Call: remove. No concrete-head alias survives; `HashMap<K, V>` / `HashSet<T>` are the whole
public surface.**

---

## §4 sidekick pricing — the PrimMap\* retirement call

Question: with the rows gone, could everything ride the generic class's `[?V]` sidecar (retire
the native prim columns) — i.e., is the column advantage within noise? **Measured, not
assumed** (bench/, interleaved A/B over `rut-bench-probe`, 5 rounds × 5 fresh-VM iters per
side, order alternating, medians of round medians; fuel/heap are deterministic):

| pair (n=100 000 churn: put/replace/hit/miss/remove/has/re-add) | column fuel | sidecar fuel | fuel Δ | column heap | sidecar heap | col exec | side exec | exec Δ |
|---|---|---|---|---|---|---|---|---|
| **u64 val twin** (mine: `HashMap<i32, u64>` row→`PrimMapU64` vs the same class rows-stripped → `[?V]`; checksums equal 5030300000) | 17 800 301 | 21 103 285 | **+18.6 %** | 324 B | 3 539 324 B | 78.1 ms | 86.2 ms | +10.4 % |
| **f64 val twin** (same design; checksums equal) | 17 800 309 | 21 103 293 | **+18.6 %** | 324 B | 3 539 324 B | 83.0 ms | 88.0 ms | +6.1 % |
| **suite twins** (pinned: `nmap-primmap` `HashMap<i32, i64>` vs `nmapset-int` `HashMap<i32, i32>` — same op stream, checksums equal 734932704; val-width caveat i64 vs i32) | 17 950 301 | 20 703 284 | **+15.3 %** | 324 B | 1 966 551 B | 76.8 ms | 88.0 ms | +14.5 % |

The instrument for the first two rows: a scratch vendor of nmapset with the three `pub type
HashMap<..>` rows stripped (`nmapset-norows`), so the *same class code* takes the sidecar path
while the real pkg takes the column — the only difference is the resolution, and the
checksums match across the pair, as the no-IR law demands. The suite twins corroborate at the
pinned values. Receipts: `bench/ab-{u64,f64,suite-twins}.json`, `bench/ab.mjs`, workload dirs.

**Call: FAR. +15–19 % fuel (deterministic, not noise), +6–14 % exec, and 324 B vs
1.9–3.5 MiB heap — three orders of magnitude on the allocation side (the `[?V]` relocation
drain and per-entry cell overhead).** `PrimMapI64/U64/F64` **stay nmapset-internal** (they are
already private since bd104f8). The gap is recorded and menu'd (§5 site 6, §6): the honest
shape for a future column surface under one-name-one-decl is *differently-named* classes
(e.g. `I64Map<K>` / `FloatMap<K>` — one name, one decl, native column inside), an RFC-sized
item, not phase 1.

---

## §5 the migration mapping — every consumer of the rows

The mapping's **shape**: the row *spellings* (`HashMap<K, i64>` etc.) survive phase 1
everywhere except the row *declarations* — the same text resolves to the generic class and
re-seats to the sidecar silently-by-law (the bd104f8 re-seat law, run in reverse). So: few
source edits, many silent resolution re-seats, and — because the mechanism changes rather than
the spelling — **the rename theorem no longer applies**: bit-identity is no longer guaranteed
by construction. What holds instead: checksums are functions of the op stream, which does not
move — **checksums stay immovable**; fuel/heap move to the sidecar's measured cost (§4).

| # | site | edit | expected movers |
|---|---|---|---|
| 1 | **the rows themselves** — nmapset.rut:570-572 (+ the head grammar, item.rs) | the three `pub type HashMap<K, ..> = PrimMap*<K>;` removed; the grammar reverts to plain/union aliases; `PrimMap*` stay private (no change there) | none (decl removal) |
| 2 | **json's impl matrix** — rut/json/group-nmapset.rut (:152 generic `HashMap<K, V>`, :201 i64, :238 u64, :279 f64) | **the migration's hardest leg, now precisely pinned**: today the three val impls register on the row *targets'* templates (four distinct (trait, type) pairs — that is *why* they coexist). Without rows, probed P3: generic + partial-instantiation impls over one class **collide** — "duplicate impl for the same (trait, type) pair (RFC 0012 §2)". Partial instantiation alone works (probed P1) but cannot coexist with the generic impl. So the three val impls **fold into the generic impl** with val-type-driven branches — the exact mechanism the key `key_spells` branch (:214, the bytes-key re-seat) already uses | json decode routes through one impl; the bytes-key diagnostic text must hold **verbatim** (the `dec_hm_bytes_key` pin); fixture decode results unchanged |
| 3 | **bench row** — benches/workloads/nmap-primmap | **no source edit** (`HashMap<i32, i64>` survives as the class spelling); expected.json untouched (checksums only) | checksum **734932704 immovable**; fuel 17 950 301 → ≈ 20 703 284; heap 324 B → ≈ 1 966 551 B (both = the pinned sidecar twin's values; same-width u64 twin says +18.6 %). Sibling rows (nmapset-int/str, nmap-hashset, nmap-knucleotide) already generic — unmoved. benches/README: append the phase section, history untouched (the strbuild precedent) |
| 4 | **driver tests/fixtures** — nmap_primmap.rs (seven embedded programs), nmap_valcolumn.rs (the raw-crossing law gate, :233 sidecar leg), json_pkg.rs:386 cases + the jsonpkg fixture's three decodeJson rows | sources compile unchanged (spellings survive); **nmap_valcolumn's law retunes** — "the row spelling rides the raw crossings" becomes false; the gate pins the sidecar spelling (or the raw host table alone). The vacuous-parity warning applies in reverse: after phase 1 no `HashMap<i32, i64>` spelling proves anything about columns (there are none on the public surface) | the fuel/heap assertions move as in #3; every checksum pin holds; the u64-beyond-i64 decode case (json_pkg.rs:387) must stay exact through the merged impl |
| 5 | **LSP** — crates/rut-lsp/src/std_surface.rs (:111-135 family pin, PrimMap\* absence :124-129) + integrations/vscode-extension/test/e2e-wasm.js (:203 completion pin) | the class-pair pins are **unchanged** (HashMap/HashSet stay, PrimMap\* stay absent); ADD negative assertions: the three row decls leave the index/completions (they were `pub type` until now) | index row count −3; no pin moves otherwise |
| 6 | **docs** — examples/README.md keyed-collections section ("the val type selects the storage"), rfc/0043 (§1 row form, §5 non-goal), docs/hashmap-surface-{survey,report}.md | README rewrites the storage-selection law to sidecar-only **plus the recorded §4 gap and the differently-named-column menu item**; RFC 0043 §1 row form reverts, §5 re-widens to *all* generic aliases; the hashmap-surface docs are **history — untouched** (the strbuild not-rewritten precedent) | docs only |
| 7 | **consumers riding the rows silently** — examples/05 (the four bd104f8 re-seated sidecar-i64 sites: t1 lids, atom gens, derived m, valcolumn wrapper), demo's maps case + blurbs | **no source edit** — the spellings re-seat back to the sidecar by resolution | UI-scale fuel/heap movers (up), no pins; demo smoke + todolist-web suites are the gates |

Corpus exposure check: the tree-wide grep for row-form declarations finds **exactly one
site — nmapset.rut:570-572** (removed in the same commit that makes it illegal; no window).
No corpus file spells an alias row; no legal file newly diagnoses.

---

## §6 the calls

- **VERSION: NO BUMP (expected).** The change is checker diagnosis + pkg source; phase 1 adds
  no TyKind, opcode, nat, or encoded vocabulary. `VERSION` gates the *module format*
  (crates/rut-core/src/binary.rs:470, `pub const VERSION: u32 = 11`); aliases resolve to the
  target's TypeId pre-IR (RFC 0043 §4's no-IR law), so nothing on the wire moves and old
  bundles decode both directions. Precedent: bd104f8 landed all three row seams at VERSION 11,
  no bump.
- **LSP: the surface moves → wasm rebuild + vsix re-issue, bound to phase 1.** The index and
  completions lose the three pub row decls (the class pair survives; new negative assertions
  per §5.5). That is a surface move, so: `build:wasm` rebuild (rut.wasm + rut-lsp.wasm), the
  e2e corpus sweep + smokes through the rebuilt artifact, vsix re-issue. Per the
  hashmap-surface precedent (8a443b5's call), this binds to **phase 1** — the flip that moves
  the surface — not to a docs phase.
- **Corpus exposure: legal code stays diag-free.** The only in-tree row declarations are
  nmapset's three (§5) — removed in the phase-1 commit itself. Row *spellings* stay legal
  (they resolve to the class). The grammar corpus (86 files, the lsp corpus.rs +
  e2e-wasm sweep) contains no row decls — zero new diagnostics, zero false positives
  expected. The new diagnostics phase 1 *should* add are exactly the §1.3 matrix going
  uniformly D (plus, if kept beyond the row removal, the §1.5 fix shape).

---

## §7 deviations, limits, receipts

- **`rut check` is not a CLI subcommand** (the binary is `run | pack | dump` — rut-cli/src/main.rs).
  The evidence's "passes `rut check` EXIT 0" reproduces as: compile succeeds, and with a real
  `pub fn main` the program **runs and exits 0** (probed, rhs/q_notatype_unused). All checker
  probes here use `target/debug/rut run` (exit 1 + `error:` text on diagnosis); benches use
  `target/release/rut-bench-probe`.
- Dead-code non-diagnosis (§1.5) is a *pre-existing* reachability law, not row-specific — it
  bounds every "the checker should have caught this" claim and is recorded here because the
  RHS hole hides behind it.
- The cross-pkg extern regime (§1.4.2/3) is surveyed as-found; whether `use`-site ambiguity or
  local-shadows-used should diagnose is a question for a future batch, not this one.
- The bench twins' u64/f64 workloads are new scratch (the suite had no sidecar twins at those
  val types — the rows intercept every u64/f64 spelling); the vendor-pkg design and the
  checksum equality across each pair are the validity argument. The suite-twins row carries a
  val-width caveat (i64 column vs i32 sidecar) noted in §4.
- Nothing outside `docs/type-name-law-survey.md` is staged. The deploy lane's files
  (scripts/, .wrangler/, demo/) untouched.
- Scratch receipts: `/tmp/opencode/batch-type-name-law/p0/` — `probe-battery-receipts.txt`
  (the full matrix + RHS battery with sources and exits), `matrix/`, `rhs/`, `xpkg/` (incl.
  the flat-splice and extern-collision probes), `papp/` + `implpart/` (the §3 and §5.2
  receipts), `bench/` (workloads, vendor pkg, `ab.mjs`, three A/B JSONs),
  `gates-ct.log` (cargo test --workspace 779/0, exit 0), `gates-wasm32.log`
  (wasm32 check exit 0).
