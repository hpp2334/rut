# type-name-law — the batch report

The record of the type-name-law batch: three phases, one shared tree,
base `725f35d`, all on `master`. Companion to
`docs/type-name-law-survey.md` (phase 0, `708abdb` — the census, the
probed five-kind matrix, the RHS hole, the sidekick pricing; its
§-numbers are the ones phase 1 and this report cite). Phase 1
(`7f9cb67`) is the law and the repeal; this phase is the record — the
RFC 0043 repeal revision, the RFC 0028 verification, the README
keyed-collections check, and this document.

The repealed state this batch undoes was landed by the hashmap-surface
batch (`bd104f8` the rows, `97670c0` the RFC amendment + the README
section). Those docs are that batch's history — untouched here
(the strbuild not-rewritten precedent). This report supersedes them as
the record of the surface.

---

## 1. The arc — a grammar ruling, a census, a repeal

The user's ruling (Sep 24 2026), verbatim:

> we should fix our grammar parse first, **same name type should be
> one**, and cannot conflict with class, struct and so on

— and the row spelling `pub type HashMap<K, i64> = PrimMapI64<K>;`
(x3, `rut/nmapset/nmapset.rut:570-572`) is **not legal grammar**. The
batch implements exactly that: the alias-row form is repealed, the
duplicate-name law is uniform again, and every consumer re-seats by
resolution.

Four beats:

1. **The census** (phase 0, `708abdb`). The checker probed end to end
   at base: duplicate type names diagnosed in exactly FOUR pass-1a
   sites, one message, per-module scope (§2 below); the C2
   concrete-shadows-generic LIFT lived only in `declare_alias`
   (collect.rs:251-263) and was order-asymmetric; the five-kind
   conflict matrix probed both orders found the only silent cells
   were the lifted ones; the RHS hole was root-caused to
   `validate_row_target`'s `Path` arm never resolving the target head;
   the single-concrete-head survival call answered **REMOVE**; and the
   sidekick pricing measured the prim columns' worth — **FAR**
   (§5 below).
2. **The law + repeal** (phase 1, `7f9cb67`). The eight coupled row
   seam pieces deleted, the C2 lift deleted, the RHS hole closed at
   pass 1b with **no new engine arm**, the tree re-seated
   (`nmapset`'s row table deleted in the same commit that makes the
   form illegal — no window), every checksum immovable, the movers
   disclosed (§6). VERSION 11, no bump — zero new TyKind, opcode,
   nat, or encoded vocabulary, nothing on the wire (the `bd104f8`
   precedent at 11).
3. **The movers.** The mechanism changed (not the spelling), so the
   rename theorem does not apply; what holds instead is that
   checksums fold the op stream, which does not move (§6).
4. **The record** (this phase). The RFC 0043 amendment reversed
   honestly with history kept (§9), RFC 0028 verified clean of
   row-shaped text, the README keyed-collections section amended to
   the post-repeal truth, and the menu banked (§8).

## 2. The checker law as landed — one name, one decl, five kinds

`duplicate type name \`X\`` is diagnosed by the module collector's
pass 1a at exactly four sites — `collect_enum` (collect.rs:132),
`declare_data`, i.e. struct **and** class (:166), `declare_alias`
(:249), `declare_trait` (:284) — each the plain symmetric conjunction
over `find_{alias,data,enum,trait}..is_some()`
(`crates/rut-lir/src/check/mod.rs:745-753`). That is the `bd104f8^`
shape restored: phase 1 deleted `declare_alias`'s two `map_or` legs
(leg 1 suppressed row-vs-row collisions, leg 2 suppressed
row-vs-generic-data) and the check is symmetric with the other three
declare sites again.

The five module-scope kinds — `type` (alias), `enum`, `struct`
(dataclass), `class`, `trait` (`crates/rut-parser/src/item.rs:25-30`)
— give ten unordered pairs × two orders, and the probed matrix
(survey §1.3, receipts under `batch-type-name-law/p0/matrix/`) is
**uniformly D** now: the silent cells (row×row both orders,
row×generic-data class/struct-first) died with the row grammar — a
generic alias head cannot even parse (`expected =, found \`<\``), so
the lifted shapes are unreachable. One name, one decl, in both
orders, for every pair of kinds. Pinned in
`crates/rut-driver/tests/type_aliases.rs`'s
`one_name_one_decl_diagnoses_in_both_orders` (alias×class and
alias×generic-struct in BOTH orders — the lifted leg-2 shapes — plus
alias×alias, alias×enum, alias×trait).

## 3. The RHS closure — the no-new-arm elegance

The survey's hole (§1.5): a row alias with a nonsense target compiled
clean when unused — `validate_row_target` (mod.rs:950-1009) walked the
target *structurally* but its `Path` arm never resolved the head, so
resolution deferred to first expansion and compilation is
reachability-scoped (dead fns are never compiled).

Phase 1's fix is the elegant one: **the row form was the only
deferring path**. Plain aliases already resolve their targets eagerly
at declaration — pass 1b's `validate_alias` → `resolve_type`, driven
by collect.rs:53's all-aliases walk. Deleting the form closes the hole
with **zero new engine arm**: `type Foo = NotAType;` is diagnosed used
or not (pinned by
`alias_target_resolves_at_declaration_used_or_not`), the arity-wrong
head ditto, and the deferral doc comment retired with the form. The
survey's fix shape ("one arm, pass 1b") is banked, not landed — it
matters only if a row-like form ever returns (§8.6).

Two standing bounds, restated so nobody over-claims: dead-code
non-diagnosis is the pre-existing reachability law (an uncalled fn's
body is never compiled — this bounds every "the checker should have
caught this" claim); and the module-scope law does not reach the
`.d.rut` surface forms (§8.3).

## 4. The D1 retention — disclosed, not smuggled

`bd104f8`'s seam (c) carried TWO expansions: the row machinery AND a
plain-alias impl-target expansion (`plain_alias_target` —
`impl Paint for B2` where `B2` is a plain alias compiles through the
expanded decl). The survey's eight enumerated pieces are the ROW seam;
D1 is not one of them, is row-independent plain-alias behavior, and
**STAYS** — removing it would be an engine change beyond the
enumerated repeal. The disclosure lives in the phase-1 commit and
here; nothing else survived that is not enumerated plain-alias
behavior.

## 5. The PrimMap* fate — priced FAR, kept private

The survey's question: with the rows gone, could everything ride the
generic class's `[?V]` sidecar and the prim columns retire? Measured,
not assumed — interleaved A/B over `rut-bench-probe`, 5 rounds × 5
fresh-VM iters per side, medians of round medians (survey §4, receipts
under `batch-type-name-law/p0/bench/`):

| pair | column fuel | sidecar fuel | fuel Δ | column heap | sidecar heap | exec Δ |
|---|---|---|---|---|---|---|
| u64 val twin (same-width, checksums equal) | 17 800 301 | 21 103 285 | **+18.6 %** | 324 B | 3 539 324 B | +10.4 % |
| f64 val twin (same-width, checksums equal) | 17 800 309 | 21 103 293 | **+18.6 %** | 324 B | 3 539 324 B | +6.1 % |
| suite twins (`nmap-primmap` vs `nmapset-int`, checksums equal; i64-vs-i32 width caveat) | 17 950 301 | 20 703 284 | **+15.3 %** | 324 B | 1 966 551 B | +14.5 % |

**Call: FAR, not noise** — deterministic +15-19 % fuel, +6-14 % exec,
and 324 B vs 1.9-3.5 MiB heap (the `[?V]` relocation drain and
per-entry cells: three orders of magnitude on the allocation side).
**Fate: `PrimMapI64`/`PrimMapU64`/`PrimMapF64` stay nmapset-PRIVATE** —
no exposure of any kind (they have been private since `bd104f8`; the
repeal gives them no new public route). The landed mover below sits
inside the priced band, which is the pricing's confirmation. The
honest future shape for a column surface under one-name-one-decl is
**differently-named classes** (`I64Map<K>` / `FloatMap<K>` — one name,
one decl, native column inside): an RFC-sized menu item (§8.4), not
landed, not promised.

## 6. The movers — old → new, verbatim

`nmap-primmap` (`HashMap<i32, i64>`, zero source edit, pure resolution
re-seat — the spelling now IS the generic class):

| pin | old (the val column) | new (the sidecar) | move |
|---|---|---|---|
| checksum | `734932704` | `734932704` | **IMMOVABLE** (= `nmapset-int`, the design law) |
| fuel | `17,950,301` | `20,903,285` | **+16.4 %** (deterministic across 5 fresh-VM iters) |
| VM-heap peak | `324 B` | `3,539,324 B` | 324 B → 3.38 MiB — bit-exact the survey's measured sidecar-twin heap |

Inside the priced band (+15-18 % fuel, 324 B vs 1.9-3.5 MiB). The law
of the move: the MECHANISM changed rather than the spelling, so the
rename theorem does not apply; what holds is that checksums are
functions of the op stream, which does not move — **every checksum
immovable, fuel/heap move to the sidecar's measured cost**.

Siblings unmoved (sources and resolutions unchanged), verbatim:
`nmapset-int` 734932704 / 20,703,284 / 1,966,551; `nmapset-str`
1264308351 / 9,551,761 / 983,620; `nmap-knucleotide` 2198604 /
38,814,389 / 4,195,084; `nmap-hashset` 21500055 / 13,267,176 / 551.
`benches/workloads/expected.json` UNTOUCHED — checksums only, all
equal; a checksum move is a bug, the pin is the gate. The perf-log
record is `benches/README.md`'s appended type-name-law section
(history untouched).

Silent re-seats (examples/05, demo's maps — spellings re-seat by
resolution) move UI-scale fuel/heap with no pins; demo smoke
(281 passed / 0 failed) and todolist-web (all checks) are their gates.

## 7. The tree — phase 1's receipt, abridged

- **`rut/nmapset/nmapset.rut`** — the row table deleted (:570-572);
  header rewritten to the one-name law + the recorded pricing gap +
  the differently-named-column menu item; `PrimMap*` stay private.
- **json's impl matrix folded** (`rut/json/group-nmapset.rut`) — the
  migration's hardest leg, precisely pinned by probe P3: without the
  rows, a generic impl and a partial-instantiation impl over one class
  COLLIDE ("duplicate impl for the same (trait, type) pair", RFC 0012
  §2). The three val impls fold into the generic `impl
  JsonDeserialize for HashMap<K, V>`: i64/f64 ride json's scalar impls
  (byte-identical bodies), u64 gains the group's `impl
  JsonDeserialize for u64` carrying the deleted row's body VERBATIM
  (`read_number_text` + `gnu64`, WrongType "a u64") — the `key_spells`
  mechanism's val-side twin. The bytes-key diagnostic holds verbatim
  (`dec_hm_bytes_key` through the one generic impl's `key_spells`
  branch); fixture decode results unchanged.
- **Consumers keep their spelling** — `HashMap<i32, i64>` IS the
  generic class now: the jsonpkg fixture, `nmap_primmap.rs`'s seven
  programs, `nmap_viewkeys.rs`, examples/05, demo's maps all compile
  unchanged (headers/comments retuned where they described the row
  mechanism as live). `nmap_valcolumn`'s law gate retuned honestly:
  the family spelling is the sidecar, so its gate is the cross-storage
  law AGAIN (sidecar vs raw crossings, the pre-hashmap-surface shape;
  same pinned 2598000 — checksums are storage-independent).
- **LSP** — std_surface's class-pair pins unchanged, NEW negative
  assertion: nmapset's index carries ZERO alias-form entries (the
  three `pub type` rows were its only aliases; index row count −3);
  e2e-wasm's std-completion smoke gains the class-detail assertion.
  wasm rebuilt through build:wasm (bin/rut-lsp.wasm md5
  `4bd3bc1b60eb585b94be82fcd407f0ca`), grammar corpus 86 files
  0 violations, e2e through the rebuilt artifact zero false
  diagnostics + 22 smokes green, vsix re-issued 0.2.4 → 0.2.5
  (md5 `61d4a80bdee4bf16cf6e86072434b2a0`; both gitignored build
  outputs — the md5s are the receipt).
- **Law probes** — exit 1 with the exact texts (receipts under
  `batch-type-name-law/p1/law-probes.txt`): the head grammar is a
  parse error again, one-name-one-decl diagnoses in both orders, the
  alias target resolves at declaration.

## 8. THE MENU — the recorded gaps and banked items

Recorded as-found, none smuggled into the batch:

1. **Extern same-name exports are SILENT (last-use-wins).** Two pkgs
   both exporting `struct Pt`, consumer `use`s both: no diagnostic at
   collect or link — the name binding is last-use-wins
   (`extern_types` insert) while values carry their real types, so the
   failure surfaces at the USE site as the ambiguous
   `\`p\` is \`Pt\` but the initializer is \`Pt\`` (two TypeIds that
   print identically). `link.rs` checks duplicate modules (:57) and
   duplicate impl pairs (:263) — not names. Whether a use-site
   ambiguity diagnostic is wanted is a future-batch question (survey
   §7).
2. **Local-shadows-used is SILENT.** `use pga::{Pt}` plus a local
   `struct Pt`: the local wins for local spellings
   (`resolve_type` checks local `find_data` before `extern_types`),
   imported fns still return the foreign type. No shadowing
   diagnostic. Same future-batch question.
3. **The three `.d.rut` surface forms are OUTSIDE the module
   collector.** `SurfaceDataclass` (`host struct`), `BuiltinTy`
   (`builtin Name<..>`), `BuiltinTrait` (`builtin trait`) — pass 1a
   matches only Enum/Dataclass/Class/Trait/Alias and drops the rest,
   so the one-name law does not see them. They are host-constructed
   surface documentation, cannot collide with module items in the same
   compile; recorded as-found, extending the check there out of scope.
4. **Differently-named column classes for the prim gap.** `I64Map<K>`
   / `FloatMap<K>` — one name, one decl, native column inside — is the
   one-name-compatible shape for the survey's FAR pricing gap (§5).
   RFC-sized: new surface, new names, the LSP/exposure story — future
   engine work, not landed.
5. **Fully-generic aliases / partial application stay non-goals.** The
   §3 receipts stand: the legal row spelling forced re-spelling the
   concrete at every use (partial application — the one justification
   — is exactly what the form could not do); a rows-only family cannot
   mint (`call.rs:560` requires a declared class); anything kept keeps
   the whole seam. A general alias is a type constructor — RFC-sized
   if ever wanted (RFC 0043 §5).
6. **The RHS-fix shape, banked for any returning row-like form.** If a
   deferred-resolution alias form ever comes back, its target head
   must resolve at declaration — pass 1b, one arm, exactly where the
   survey pointed (survey §1.5); the deferral comment stays retired.
7. **The linked-surface pub gate is still future work.** The splice
   path is flat and the collector visibility-blind (`bd104f8`'s
   disclosure), so a pub drop is not compiler-enforced: `PrimMap*`'s
   privacy (and every pub drop) still rides the Tier-1 grep + the LSP
   pub-only surface pair, now with the negative assertions pinning it.
8. **Range-on-the-columns: MOOT, closed by the repeal.** The
   hashmap-surface menu's "range methods are stranded off the family
   spelling" item died with the columns' routing — the range surface
   (`put_range`/`get_range`/…) lives on the generic class, and every
   spelling resolves to it now (`nmap_viewkeys` spells the class and
   reaches it).

## 9. This phase — the record (docs only, zero code changes)

- **RFC 0043 repeal revision** (`rfc/0043-type-aliases-and-union-bounds.md`).
  The `97670c0` row-form amendment is REVERSED, honestly and with
  history kept: the header now carries BOTH revision entries — the
  row-form amendment as landed, and a second 2026-09-24 entry recording
  the repeal by the user's grammar ruling. The Summary and §1 restate
  the v1 non-generic alias law as THE law (the head admits no members;
  a generic head is a parse error; one name = one decl, both orders,
  five kinds) with the row form entering history as
  landed-then-repealed; §5's non-goal is un-narrowed (the row-form
  exception is gone; fully-generic aliases stay the type-constructor
  non-goal). Cross-links: this report, the survey.
- **RFC 0028 verified, NO amendment needed.** Its `nmapset` passages
  describe the closed key union, the peer-group wiring, and the
  impl-matrix split — all still true; every "row" in it is an
  impl-matrix/perf-log table row, nothing alias-row-shaped. Nothing
  stale to amend.
- **The README keyed-collections section**
  (`examples/README.md`) — verified against the post-repeal truth and
  the stale sentences amended: the section title (the val type no
  longer picks the storage), the storage-selection law (ONE class per
  name, the `[?V]` sidecar for EVERY `V`), the row-per-val bullet
  (no rows exist), the "generic row" phrasing, the decode-impls
  sentence (ONE generic impl after the fold), and the consumer-visible
  identity paragraph (`HashMap<i32, i64>` IS the generic class;
  checksum immovable, fuel/heap re-seated per §6). Still true, left
  alone: the `HashSet` bullet, the can't-spell list (no float keys, no
  narrow-val columns, no bool column, no iteration surface), the
  Prim-internal-names law, the bytes-key diagnostic text, the run
  recipe, the taste, the bench-row commands.

## 10. Gates (run on this tree)

Docs-only commit — the engine is byte-identical to the phase-1
landing; the pins are paranoia, not proof-of-change:

- `cargo test --workspace` — **782 passed / 0 failed**, exit 0 (the
  phase-1 count exactly).
- `cargo check --workspace --target wasm32-unknown-unknown` — exit 0.
- Bench pins paranoia re-run on the landed release binaries — all five
  nmap probe rows bit-identical to the phase-1 record (§6's table,
  including `nmap-primmap` 734932704 / 20,903,285 / 3,539,324 B), all
  cross-runtime checksums equal `expected.json`.
- Tree clean after the commit; single commit; staged by EXPLICIT PATH
  ONLY (docs + rfc + examples/README — nothing else).

Scratch receipts: `/tmp/opencode/batch-type-name-law/p2/` (gate logs);
the phase-0/phase-1 receipts remain under `p0/` and `p1/`.
