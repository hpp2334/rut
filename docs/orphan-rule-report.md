# The orphan rule — the batch report

The record of the orphan-rule batch: three phases, one shared tree,
base `963ed33`, all on `master`. Companion to
`docs/orphan-rule-survey.md` (phase 0 — the 76-impl census, the locked
rulings embedded verbatim, the design; its §-numbers are the ones the
landed code, tests, and diagnostics cite). The law lives in the RFC
0012 §2a amendment; the user-facing summary lives in
[`examples/README.md`](../examples/README.md) (the orphan-rule section)
and [`examples/02-digest/README.md`](../examples/02-digest/README.md)
(the type-local worked example). This report is the record.

## 1. The story, in one section

Rut's written law was RFC 0012 §2: "there is no orphan rule beyond
placement" — trait impls legal in any module, the only constraint one
impl per `(trait, type)` pair. The batch asked what it costs to give
the language the real coherence law — **at least one of the pair
defined in the pkg whose source declares the block** — without
breaking a single in-repo program, and landed it in three commits:

1. **What does the tree actually do today?** (phase 0, `df63414` —
   the survey.) The census: **76 impl blocks in 28 files** — 27
   inherent (all in-owner, the RFC 0012 §4 law holds repo-wide) and
   **49 trait impls classified 44 trait-local + 5 type-local — ZERO
   orphans**. 29 of the 49 have builtin self types, every one behind a
   local trait, the sanctioned direction. No migration plan, no
   grandfathering, no allowlist — the check would land against a clean
   corpus. Two live probes fixed the stakes: **probe A** (the
   inherent-splice hole — a consumer's inherent `impl Vec<T>` on
   pouch's spliced class compiles and dispatches, exit 0) and **probe
   B** (the consumer-side orphan — `impl JsonSerialize for HashSet<T>`
   against two foreign pkgs, zero diags, exit 0: the exact program the
   rule rejects). The origin-map design, the §2a diagnostic, and the
   VERSION 9 call went in as the phases' contract; the inherent-splice
   hole was recorded out of scope.
2. **The rule lands.** (phase 1, `21feca4`.) The `OriginLeaf` origin
   map — each spliced leaf's byte range in the combined unit text,
   tagged with the pkg whose source it is, pure metadata (the T13
   byte-identity guarantee untouched); bound names carry their
   exporter's spec (`Ctx.extern_origins`); the pass-2 gate in
   `collect_impl_trait`'s entry — after `resolve_trait_ref`, before
   the duplicate-pair check — classifies both sides of the head
   through the survey's §2.5 table and errs when neither side's origin
   is the block's own pkg; the dedicated diagnostic (survey §3.1
   verbatim); the RFC 0012 §2a amendment lands with the check (one
   commit-flavored unit of law); VERSION 8 → 9; the 11-test
   `orphan_rule.rs` suite; the existing pins updated exactly where the
   LAW moved them and nowhere else. Verified through the real CLI: the
   p0 probe that compiled clean at base now exits 1 with the diag.
   The wasm was rebuilt **byte-identical** to the committed artifact,
   the corpus re-proved through it (70 files, 0 false diagnostics),
   the vsix re-issued same-version 0.2.2.
3. **The record.** (phase 2, this commit — docs only.) This report;
   the README touch (the orphan-rule section in `examples/README.md`;
   02-digest's "orphan-legal direction" prose now cites §2a as the
   compiler's law); the §2a amendment verified complete and given its
   one missing link — rut-json's serde model (RFC 0028) named as the
   motivating legal case. No code changed; the gates re-run as
   paranoia (§6).

## 2. The design as landed

### 2.1 The origin map

One new driver type, `OriginLeaf { lo: u32, hi: u32, spec: String }`
(home: `rut-lir/src/check/mod.rs`, `Ctx`'s module; re-exported from
the driver) — one spliced leaf's byte range in the combined unit text
and the pkg whose source it is. The graph's composition loop records
each accepted leaf as it appends (`extra.len()` before the push; the
leaf's own text exactly), and the unit's own leaf closes the map —
`(extra.len() + 1, ..)` after the final seam, `(0, len)` when `extra`
is empty; a decl unit clears the splice ranges (its combined text is
its own source alone). The splice dedup, the order, and the combined
text are all untouched: **the map is pure metadata**, and T13's
byte-identity guarantee holds by construction.

Threading: `compile_program_resolved` gains one parameter
(`origins: &[OriginLeaf]`; `compile_program` and every other caller
passes `&[]`), `Ctx` gains `origins` + `own_spec` (set in the
`allow_uses` style), and every origin question goes through one
lookup — `Ctx.origin_of(lo)`, a `partition_point` on `hi <= lo` so a
seam byte belongs to the **following** leaf, which makes the
own-source fallback exact.

The fallback is the **whole-unit law**: a unit compiled with no map
(the CLI/demo/wasm single-file paths, every `compile_program` caller)
has every definition's origin = its own spec, so the check reduces to
something any impl that compiles today satisfies. **The check is inert
where splicing does not happen** — pinned by `no_map_unit_is_inert`.

### 2.2 Bound names carry their exporter

The graph's bound tuples grew the exporter's spec —
`(ScopeId, Surface)` → `(ScopeId, Surface, String)` — and
`compile_program_resolved`'s registration records it beside the name:
`Ctx.extern_origins`, the parallel `HashMap<IdentId, String>` (the
survey §2.4's lower-churn shape). Core's native traits
(`Iterator`, `Index`, `Disposal`) arrive through core's own surface,
so their origin is `"core"` — the RFC 0012 §2 truth (builtin traits
are core decls), never "no pkg".

### 2.3 The check

rut-lir's check/collect, pass 2 — `collect_impl_trait`'s entry, after
`resolve_trait_ref` succeeds, before the duplicate-pair check: the
same home as every other impl law (placement, duplicates, coverage),
where the impl head is fully resolved on both sides and spans are
native. The block's OWN locality is `origin_of(impl item span)` —
**the origin, not the compiling unit, decides**: a pkg's own impls
stay legal in every unit that splices them (json's peer-gated groups
reclassify exactly as the census ruled: trait-local), while a
consumer's hand-written cross-pkg pair errs (`the_origin_not_the_unit_decides`).

The trait side classifies `find_trait` → `origin_of(decl span)`, else
`extern_trait_decls` → the carried exporter spec, else `extern_traits`
→ `"core"`. The type side classifies `find_data` →
`origin_of(decl span)`, else `extern_types` → the carried exporter
spec, else builtin — a primitive, `opaque`, and the `?T`/`[T]`
template arms (the rut-json sanctioned shapes; the head peels to "no
pkg"). Generic heads classify by the HEAD; the type parameters never
satisfy locality. The law, in one line: the block is an orphan iff
**both** origins differ from its own pkg. An orphan errs and returns:
placement precedes registration, so the block never reaches the
duplicate check or the impl table — a block that is both orphan and
duplicate reports the orphan (pinned in the rewritten duplicate-pair
test).

### 2.4 The diagnostic

One diag, span = the impl item span, plain message, no labels, no
notes — house style. The pkg×pkg rendering (probe B, pinned verbatim
by `consumer_impl_of_jsonserialize_for_hashset_errors` through the
real mounted pkgs — pouch/nmapset/json + `assemble_peers`):

```
orphan impl: neither `JsonSerialize` nor `HashSet` is defined in this pkg — `JsonSerialize` is json's, `HashSet` is nmapset's; an `impl Trait for Type` needs at least one of the pair declared in its own pkg (RFC 0012 §2a)
```

The builtin-head variant's tail: "`str` is a builtin, in no pkg; only
a trait of this pkg may be implemented for a builtin"; a `?T` head
renders as its written head ("`?T` is a builtin, in no pkg"). Surfaces:
the CLI renders it through `render_diags` (caret on the `impl` line),
the demo/wasm surface returns it structured, and the LSP — honest both
halves, survey §3.4 — never runs rut-lir's check today, so it cannot
surface yet and cannot regress; when the editor grows semantic
diagnostics this diag flows through `to_diagnostic` unchanged, and the
zero-false-diag invariant is protected by the census (a check that
fires on nothing in-repo cannot conjure a red squiggle), tested by the
corpus gate through the rebuilt wasm.

## 3. The coherence ledger — honest

The collision mechanics are **unchanged**: same two uniqueness checks
(the in-unit duplicate check, the §5 link-merge), same errors, no new
coherence engine, no global pair table at compile time. The rule adds
exactly one gate ahead of registration, and its yield is three things,
each at its true size:

1. **Discipline.** The `(trait, type)` pair set is auditable per pkg:
   a pkg's impls are either its own trait speaking about anyone, or
   anyone's trait speaking about its own types. "Who could possibly
   implement `JsonSerialize` for `Vec<T>`" has one answer (json) and a
   grep to prove it — before, "any unit in the closure", which is
   probe B.
2. **The open-world property.** Before, adding a pkg to a closure
   could change behavior at a distance: probe B's impl participated in
   dispatch and widening the moment its unit compiled. Post-rule, an
   impl exists only inside a pkg that owns a side, so a consumer's
   pair set is bounded by the pkgs it named — adding a dependency adds
   the pairs its pkgs declare, never pairs a stranger invented.
3. **The shape of future separate compilation.** If pkgs ever compile
   to artifacts consumed without re-splicing (the `.d.ir`/bundle
   direction RFC 0041 points at), pair-uniqueness must become
   decidable from origins, not from "who happened to be spliced into
   this unit". The origin map is the smallest data that makes the
   question well-formed; the rule is its first consumer. A
   prerequisite built early — not a claim that separate compilation
   lands here.

**What the rule does NOT do — said plainly, because the ledger's value
is its honesty:** it fixes **no soundness bug** (nothing in today's
dispatch miscompiled; probe B's impl was *wrong-shaped*, not
*wrong-code*); it makes **no codegen change** (dispatch, vtables,
static binds, and byte output are identical — the corpus through the
rebuilt wasm and the 29-row bit-identical bench suite are the proof,
not the claim); it does not make anything faster or smaller; it does
not subsume placement (the inherent-splice hole, §4, stays open).
Duplicates colliding as duplicate definitions after splicing remains
the consumer-side guard for group-provided pairs where the writer owns
a side, exactly as the peer amendment recorded it.

## 4. The inherent-splice hole — recorded, out of scope

Probe A still compiles, on purpose: an **inherent** `impl Vec<T> { .. }`
in a consumer, over pouch's spliced class, remains legal — pinned by
`inherent_impl_on_a_spliced_foreign_class_stays_legal` so the rule's
scope is a tested fact, not a hope. Why it stays open: the trait-impl
law's subject is the `(trait, type)` pair and its home is pass-2
collect, where both sides are resolved; the inherent law's subject is
placement, and tightening it would be a separately disclosed rule —
one the origin map already equips with its data (a spliced class's
declaration carries its leaf's origin; the check would be one
comparison). Recorded as the menu's first item, not silently dropped.

## 5. The VERSION ledger: 8 → 9

**The call: `VERSION 9`, landed WITH the check — the byte and the law
in one commit (`21feca4`).** The ledger, from the tree's own records
(`crates/rut-core/src/binary.rs`):

| Move | What | Why it moved |
|---|---|---|
| 5 → 6 | the `opt_prim_store` gate (RFC 0044 §5) | a **rejection addition** — sources the old compiler accepted, the new one refuses |
| 6 → 7 | `opaque.downcast<T>` → `?T` (refval-round2) | a declared-surface change |
| 7 → 8 | the StackTrace surface + the func table's `pos` field | format-affecting, both halves |
| 8 (stayed) | the entry-err widening | a **check relaxation** — accepting-more cannot strand an artifact; old compilers refuse the new sources instead |
| 8 → 9 | **the orphan rule (this batch)** | a **rejection addition** — the 5 → 6 precedent |

The orphan rule is a rejection addition: a source that compiles under
8 is refused under 9 — precisely the 5 → 6 precedent, and the honest
complement of the v8-still note's own principle (accepting-more cannot
strand an artifact; rejecting-more can strand a *source*). Disclosed
caveats, so nobody overclaims later: codegen is **invariant** under
the rule (it gates which sources reach emit, never what emit
produces), so no decode/verify change rides this — **the bump is
policy per the precedent, not format necessity**, and a v8 artifact
remains behaviorally correct forever. The byte moves because it is the
only provenance marker a `.rutc`/bundle carries about the law that
produced it. The `opt_prim_store` version gate and stack_trace's
stale-artifact pin moved with the bump.

## 6. The gates, as landed per phase

| phase | commit | workspace tests | wasm32 | bench suite |
|---|---|---|---|---|
| 0 | `df63414` | docs-only, untouched (green at base) | n/a | n/a |
| 1 | `21feca4` | 745 passed / 0 failed (incl. the 11-test orphan gate) | exit 0 | green — all 29 rows bit-identical on this tree AND on a df63414 worktree (fuel/heap-peak/trapped identical on every row — no fuel move anywhere is the proof); expected.json untouched; the corpus 0 false diags through the rebuilt wasm; the vsix re-issued same-version 0.2.2 |
| 2 | this commit | docs-only; re-verified — 745 passed / 0 failed | re-verified, exit 0 | re-verified (paranoia — docs cannot move pins): the full suite, 0 mismatches, exit 0; json-roundtrip `1960875332163557684` and the json-decode canary `4502015958359127277` exactly as pinned, fuels identical (60,933,262 / 111,322,915) |

Phase 2 changed no code. The README sections document landed behavior;
the RFC link amendment records the motivating case; nothing here moves
a pin. Scratch under `/tmp/opencode/batch-orphan-rule/p2/`.

## 7. The menu going forward

- **The inherent-splice tightening** (probe A) — an inherent impl on a
  spliced foreign class still compiles; the origin map is the data a
  future rule needs, and the fix is one comparison in the inherent
  placement arm. A separately disclosed rule: it re-opens the RFC 0012
  §4 law, and the census (all 27 inherent impls in-owner) says it
  would land clean — but placement is a different law from pairing,
  and it deserves its own survey-sized honesty.
- **LSP semantic diagnostics** — the per-document pipeline is still
  lex + parse + classify; when it grows a check pass (the
  lsp-features roadmap's owning call), the §2a diag flows through
  `to_diagnostic` unchanged, and the corpus gate is the template for
  proving no false positives in-editor.
- **Separate compilation from origins** — the §3 yield 3: when RFC
  0041's artifact direction matures, pair-uniqueness should be
  decidable from the origin data artifacts carry, and the origin map
  is the rehearsal for what that serialization looks like.
- **The corpus re-count ritual** — the gate is count-agnostic on
  purpose (`assert!(files.len() >= 50)`); new corpus files that write
  impls should keep the census's habit: classify both sides, expect
  one of the pair local. The rule makes the illegal shape impossible,
  which makes the census boring — the intended end state.

---

*Companions: `docs/orphan-rule-survey.md` (the contract — census,
design, §-numbers), `rfc/0012-traits-and-dispatch.md` §2a (the law),
`crates/rut-driver/tests/orphan_rule.rs` (the suite — the ruling,
executable), `examples/README.md` (the rule as users meet it).*
