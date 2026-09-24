# hashmap-surface — the batch report

The record of the hashmap-surface batch: three phases, one shared
tree, base `071b8c5`, all on `master`. Companion to
`docs/hashmap-surface-survey.md` (phase 0 — the census, the ten probe
compiles, the mechanism pick; its §-numbers are the ones the flip and
this report cite). The law lives in the RFC 0043 amendment
(the row form, §1 — phase 2); the user-facing summary lives in
[`examples/README.md`](../examples/README.md) (the std keyed-collections
section); the bench history lives in `benches/README.md` (an appended
phase section — the historical log untouched).

---

## 1. The story, in one section

The directive was: "dont expose PrimMapI64 and so on, they should be
`HashMap<i64, ...`". The survey (phase 0, `8a443b5`) resolved the
"and so on" EXACTLY — nmapset's declared surface was five classes
(`HashMap` generic sidecar, `HashSet`, and the three prim val-columns
`PrimMapI64`/`PrimMapU64`/`PrimMapF64`), so there are exactly THREE
prim names to retire (no `PrimSet`, no bool column — §2), and
gate-proved the census at 96 hits over 21 files (§2). The same survey
probed the language as it exists and picked the mechanism: generalized
RFC 0043 **alias rows** — `pub type HashMap<K, i64> = PrimMapI64<K>;`
— over the generic facade (dead on two independent legs) and over
monomorphized storage (bigger engine surface, bit-identity measured not
structural). The pick's virtue is that bit-identity is a **theorem**:
the alias resolves to the row target's `TypeId` before any table work
(RFC 0043 §4's no-IR law), so every layout, impl registration, and
bench pin is identical by construction — at base, `HashMap<i32, i64>`
spelled as the row and instantiated as `PrimMapI64<i32>` must produce
the same IR and the same checksums (§4).Phase 1 (`bd104f8`) landed the pick in one bisectable commit: the
three engine seams (§3), the row table, the three prim classes' `pub`
drop, and EVERY census site migrated — the batch's 34-file commit
whose largest single file is the checker (`collect.rs` +293). The
theorem was made MEASURED on the same commit: the re-spelled
`nmap-primmap` source pinned bit-identical (checksum `734932704`, fuel
`17,950,301`, heap `324 B`), all five sibling pins exact, the full
suite 30 rows × {rut, qjs, node} exit 0 with `expected.json` UNTOUCHED
— the pin is the gate, nothing was re-derived (§4). Two survey claims
were CORRECTED on receipt, not worked around: the pub drop does NOT
hard-fail a straggler `use` of an internal name (the splice path is
flat and the collector is visibility-blind — the enforcer is the
Tier-1 grep plus the LSP's pub-only surface filter, a linked-surface
pub gate is future work), and the Tier-1 gate's survivors include a
THIRD carve-out the survey's Tier-2 list missed (the
`benches/README.md` historical perf-log sections) — both disclosed in
the gate's allowlist, not worked around (§5, §6).

Phase 2 (this commit) is the close-out: this report, the RFC 0043
amendment (the row form becomes the ALIASES' law, the no-generic-
aliases non-goal narrowed), and the README section (the one family,
documented where the std packages are). Docs only — the engine is
byte-identical to the landing, VERSION stays 11.

## 2. The census — the "and so on" resolved to exactly three names

- **What exists** (`rut/nmapset/nmapset.rut` at base): exactly five
  public classes — `HashMap<K requires i8 | … | bytes, V>` (the
  generic `[?V]`-sidecar row), `HashSet<T>`, and the three prim
  val-columns `PrimMapI64<K>` / `PrimMapU64<K>` / `PrimMapF64<K>`
  (64-bit column, no `[?V]` sidecar, no drain pass). **`PrimSet` never
  existed** — the set has always been the one host-table class.
- **What does not**: no bool val-column (blocked on the `?bool`
  nil-law checker gap, `nmapset.rut:373-379` at base — on record, not
  slipped in); no narrow-val columns (i32/u8/… — the column is 64-bit;
  a narrowing row would be a NEW class with new pins, a
  VERSION-adjacent conversation); no float keys (the equality
  contract, unchanged by the batch).
- The consumer census was COMPLETE, not sampled — the Tier-1 grep's
  96-hit set enumerates exactly the 21 files in the survey's tables
  (survey §1.2), spanning `rut/json`'s impl matrix, the LSP std
  surface + vsix e2e pins, four driver suites, examples/05, the bench
  row, demo, and the live READMEs — with `docs/*.md` carved out as
  past batches' history (12 files; the strbuild not-rewritten
  precedent).

## 3. The mechanism — alias rows, and why the identity is genetic

The public face after the flip (the ONLY new surface):

```
pub type HashMap<K, i64> = PrimMapI64<K>;
pub type HashMap<K, u64> = PrimMapU64<K>;
pub type HashMap<K, f64> = PrimMapF64<K>;
// the generic class HashMap<K requires …, V> stays the row-less
// fallback — `HashMap<str, i32>`, `HashMap<i64, Pt>`, bare `HashMap<K, V>`
```

Resolution is CONCRETE-ROWS-FIRST: at every substitution-completing
site, concrete row members match exactly-first (parameters bind the
site's argument, concrete members compare equal by `TypeId`) and the
spelled name expands to the row TARGET under the head's substitution
BEFORE any table work. The class instantiates only when no row
matches; the C2 `duplicate type name` check lifts exactly for this
concrete-shadows-generic shape; several rows may share one family
name. `impl Trait for HashMap<K, i64>` re-targets through the row at
the NODE level (probe D1→D2: at base, aliases did not expand in
impl-target position), and the static-call mint
(`HashMap<str, i64>.new()` and the expected-type inference arm) rides
the same law.

The engine bill is exactly three parser/checker seams — the alias head
admitting `(Ident | Type)` members (`item.rs`), the concrete-first
resolution, the impl-target expansion — with zero new `TyKind`,
opcode, nat, or encoded vocabulary. That is the theorem (§1) and also
the VERSION call: **no bump** (11 stays), old v11 artifacts decode
byte-identically in both directions, nothing on the wire.

## 4. The theorem, proven

- `nmap-primmap` — the one bench row whose SOURCE re-spells (source
  only; the pins read the same class): checksum `734932704`, fuel
  `17,950,301`, VM-heap peak `324 B` — bit-identical.
- The sibling pins hold untouched: `nmapset-int` 734932704 / 20,703,284
  / 1,966,551; `nmapset-str` 1264308351 / 9,551,761 / 983,620;
  `nmap-knucleotide` 2198604 / 38,814,389 / 4,195,084;
  `nmap-hashset` 21500055 / 13,267,176 / 551.
- Full suite: 30 rows × {rut, qjs, node}, exit 0, every checksum equal
  `expected.json` — expected.json UNTOUCHED, the pin is the gate.
- Wasm/driver retunes absorbed in place: `cargo test --workspace`
  **779 passed / 0 failed** (the base count exactly — no test lost);
  the LSP surface re-scoped pub-only with negative assertions; wasm
  rebuilt (`d9ca8870aa5c439e07bdb23c49dfc906`), corpus 86 files zero
  false diagnostics, e2e 22 smokes green, vsix re-issued 0.2.3 → 0.2.4
  (`af9b86ba7bbd9f769234664d8551b888`; both gitignored build outputs
  — the md5s are the receipt); demo smoke 281/0; todolist 93/0.

## 5. The gate, and its disclosed allowlist

The two-tier grep (survey §3.3) ran over the WORKING TREE at every
phase's end. Tier 1 (every live surface) greps zero except the
disclosed allowlist, enforced by REVIEW:

- `crates/rut-lsp/src/std_surface.rs` and
  `integrations/vscode-extension/test/e2e-wasm.js` — the two negative
  assertions that PIN the absence; the gate's own enforcement code.
- `rut/nmapset/nmapset.rut` — the private decls themselves, the row
  table's TARGETS, and the internal comments telling the round3 story.
- **`benches/README.md`'s historical perf-log sections (:2016+)** —
  past batches' records, the strbuild not-rewritten precedent. This is
  the THIRD carve-out, found by the phase-1 gate run and disclosed
  then: the survey's Tier-2 list under-counted it. The live rows-table
  entries ABOVE the log were migrated; the log's own era-records keep
  their spellings.
- Untracked regenerated artifacts (`gen/web_host_bg.wasm`) — not a
  source surface.

## 6. The honest limits

- **The straggler-use non-failure.** The survey predicted that
  dropping `pub` hard-fails a straggler `use nmapset::{PrimMapI64}`;
  probed on the landing tree, it does NOT — the splice path is flat
  and the collection pass is visibility-blind, so the name is still
  spliceable into `pub`-less use sites without an engine error. The
  surface is held instead by the working gate: the Tier-1 grep (zero
  hits recast as the allowlist above) plus the LSP's pub-only std
  surface (the internal names have left completions and the index,
  pinned by negative assertions). A compiler-enforced pub gate — one
  that catches `use`-splices by name-resolution visibility — is
  recorded as future work in the menu (§7).
- **The re-seats are silent BY LAW, and four sites took them.** Every
  `HashMap<K, i64>` spelling today resolves to the val column, not the
  `[?i64]` sidecar — todolist's t1 lids, `derived`, `atom`'s gens, and
  the valcolumn wrapper leg. Semantically equal for prim vals, and a
  perf win (the column is the column), but the sidecar-`i64` shape
  loses its spelling AND the always-faster lane is now the only lane —
  with no test that can tell the difference, which is the point to
  keep honest: the migration has no consumer that wanted the sidecar,
  and no deferred cost beyond the lost spelling.
- **The vacuous-parity-test warning.** `nmap_primmap.rs`'s
  churn-sidecar parity legs compared the column with itself after the
  flip — a test that cannot fail is not a gate. Phase 1 dropped the
  vacuous self-comparisons (checksum pins kept) and retuned
  `nmap_valcolumn.rs` to pin the row spelling against the RAW
  crossings (the re-seat's own proof). The general law recorded here:
  ANY parity test spelled through the rows on a row-val is vacuous —
  gate on the raw crossings, not on a spelling alias of them.
- **The viewkeys re-spell.** `nmap_viewkeys.rs` (strings-round1's
  range-method suite) exercised ONLY on the sidecar-`i64` spelling;
  the range surface lives only on the generic class, so the rows would
  strand it. The suite re-spells `HashMap<str, i32>` (the no-row val it
  parity-matches); every pinned literal unchanged (`2572351`
  included). Range-methods-on-the-column is a menu item (§7).
- **The bytes-key diagnostic did not move.** json's decode side
  re-spells its route through the rows on the general map's own
  `key_spells` branch, so the no-spelling bytes-key diagnostic text
  held verbatim (`dec_hm_bytes_key`'s pin) — a consumer-visible
  diagnostic that correctly survived the surface unification unchanged.

## 7. The menu — the prettier generalizations, priced as future engine work

Whatever else banked from the batch's ledger; NOTHING here blocks any
current consumer:

- **Generic aliases in full** (`type Vec2<T> = Vec<T>` — every member
  generic): the admitted row form carries only CONCRETE members; a
  fully-generic member would be a type constructor with a
  substitution story the checker does not have. The row table
  (concrete members only) covers this batch's need; the general form
  is engine work with its own RFC.
- **The unified facade / per-instantiation class divergence**: one
  class body holding the column for `V=i64` and the sidecar for
  `V=Pt` — would obsolete the three-class split AND the atom pkg's
  `StrAtom` split, and make the survey's dead generic facade
  spellable. A language-level RFC, surveyed and priced
  (survey §2.2.2, §2.6), not a rename.
- **Narrow-val rows** (`i32`/`u32`/… columns or narrowing wrappers) —
  new classes, new pins; **the `?bool` checker fix** un-blocking a
  bool row (`nmapset.rut:373-379`, the gap on record since base).
- **Iteration surface** for nmapset — the json DECODE-to-ENCODE half's
  standing item (`group-nmapset.rut:15-24`, RFC 0028's `json` section): map ENCODE
  waits on rows/key access that today's class surface does not ship.
- **Range methods on the columns** — the viewkeys stranding, generic.
- **Prettier row-miss diagnostics** — a row-table lookup failure
  currently diagnoses through the class's own generic spelling; naming
  the row table in the message is editor-grade polish.
- **The linked-surface pub gate** (§6's first item) — compiler-level
  enforcement of the internal names' absence, replacing the
  grep+LSP pair.

## 8. This phase — the RFC amendment + the README section

- **RFC 0043 amended** (`rfc/0043-type-aliases-and-union-bounds.md`):
  §1 grows the ROW FORM with the concrete-rows-first resolution law
  and the impl-target expansion — the law the flip actually ran —
  and §5's non-goal is narrowed from "no generic aliases" to "v1
  aliases name one type OR one row-shaped family; the fully-generic
  alias remains a non-goal". Nothing else in the law changes: the
  amendment VERIFIES the landed mechanism against the text (§2's
  three-concrete-rows table updates it in place) and cross-links the
  report as the working realization.
- **`examples/README.md` gains the keyed-collections section** (the
  std-pkg home, after json's and strbuild's sections — the nmapset
  package had no per-pkg section until the flip gave it one user-
  visible shape): the one family (`HashMap<K, V>` / `HashSet<T>`),
  the val type selects the storage (the row table: `i64`/`u64`/`f64`
  ride the columns; everything else rides the `[?V]` sidecar), the
  internal names are nmapset-internal (stated as the LAW in the
  section, spelled nowhere in it — the Tier-1 gate covers every live
  doc), and the bytes-key diagnostic unchanged. Run recipe +
  bench-row pointers included.

## 9. Gates (run on this tree)

- `cargo test --workspace` — **779 passed / 0 failed** (the phase-1
  count exactly — docs-only commit, the engine untouched).
- `cargo check --workspace --target wasm32-unknown-unknown` — exit 0.
- Bench pins, paranoia re-run through the probe on the landed release
  binaries — every one BIT-IDENTICAL to the phase-1 record:
  `nmap-primmap` checksum 734932704, fuel 17,950,301, heap 324 B (the
  row-identity gate, reproduced); `nmapset-int` 734932704 /
  20,703,284 / 1,966,551; `nmapset-str` 1264308351 / 9,551,761 /
  983,620 (~960.6 KiB); `nmap-knucleotide` 2198604 / 38,814,389 /
  4,195,084 (~4.00 MiB); `nmap-hashset` 21500055 / 13,267,176 / 551.
  All 15 cross-runtime rows (5 workloads × rut/node/qjs) checksum-
  equal `expected.json`, ok=yes, no traps. The full suite's
  30-workload expected.json verdict is the phase-1 landing's recorded
  gate (engine-identical tree); this close-out re-runs the pins, not
  the sweep.
- Docs-only commit — staged by explicit path (the parallel deploy
  lane's `demo/package.json` mod + untracked `scripts/` and
  `.wrangler/` untouched, not staged); tree clean after the commit.

Scratch gate receipts are light for a close-out:
`/tmp/opencode/batch-hashmap-surface/p2/` (gates-cargo-test.log,
gates-wasm32.log, gates-benchpins.log, tier-1 re-run). The landing's
full probe receipts remain under `p0/` and `p1/`.
