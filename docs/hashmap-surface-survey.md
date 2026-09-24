# hashmap-surface survey — phase 0

Batch: **hashmap-surface** (the public map/set surface becomes ONE
generic family — `HashMap<K, V>` / `HashSet<T>` — with the type
parameters selecting the representation; the prim-specialized names go
nmapset-internal). Phase 0 delivers the docs base every later phase
argues from: the census of the exposed prim-specialized names and every
consumer site, the mechanism pick probed against the language AS IT
EXISTS TODAY (scratch compiles, not assumed), the site-by-site
migration mapping with the grep gate's exact form, and the calls.
Docs only — zero code changes, zero engine changes, zero
representation changes.

THE USER DIRECTIVE (law, verbatim): *"dont expose PrimMapI64 and so
on, they should be `HashMap<i64, ...`"* — with the batch framing: the
PUBLIC map/set surface is the generic spelling (`HashMap<i64, V>`,
`HashMap<str, V>`, `HashSet<i64>`, …), ONE family of names; the
representation is UNCHANGED underneath and every bench row is
bit-identical (the nmap rename precedent `c066fbf`: all pins
bit-identical through a pure rename).

THE SLOT READING, ONCE (the code is the tiebreaker): in the landed
classes the prim suffix names the VALUE — `PrimMapI64<K>` is a map
KEYED by `K` with `i64` values (`put(self, k: K, v: i64)`,
`nmapset.rut:407`). The generic spelling therefore puts the val type
in the SECOND slot: `PrimMapI64<str>` → `HashMap<str, i64>`,
`PrimMapI64<i32>` → `HashMap<i32, i64>`, `PrimMapU64<K>` →
`HashMap<K, u64>`, `PrimMapF64<K>` → `HashMap<K, f64>`. The directive's
literal `HashMap<i64, ...` example is read as the ONE-FAMILY law it
states (the Prim names die; the family name carries the information),
not as a parameter reorder — a pure rename moves no slots (the
`c066fbf` precedent is exactly this). The key type parameter selects
the key lane (already true today — `KeyLane`, §1.1); the value type
selects the val storage (the new resolution law, §2.3).

THE LANDED STATE this survey censuses (base `071b8c5`): VERSION 11;
nmapset exposes the five classes (§1.1); the consumers are rut/json's
impl matrix, the LSP std surface + the vsix e2e pin, the driver tests
(`nmap_primmap.rs`, `nmap_valcolumn.rs`, `json_pkg.rs` + the jsonpkg
fixture), examples/05-todolist-web (the dispatch tables + three
sidecar-`i64` sites), the bench rows, demo's maps case, and the live
READMEs. Scratch probes live under
`/tmp/opencode/batch-hashmap-surface/p0/` (§8) — nothing from scratch
lands.

---

## 1. The census

### 1.1 The declared surface — the full "and so on" set

`rut/nmapset/nmapset.rut` (the tree's keyed-collection story since the
`mapset` slim-down removal) declares exactly five public classes:

| class | line | storage | the prim-specialized? |
|---|---|---|---|
| `HashMap<K requires i8 \| … \| bytes, V>` | :183 | host key table + the `[?V]` sidecar (reference `V` — cells stay cells, the `get -> ?V` aliasing law) | no — the general wrapper |
| `HashSet<T requires i8 \| … \| bytes>` | :341 | the host key table alone | no |
| `PrimMapI64<K requires …>` | :390 | the host table's NATIVE u64 val column (`map_val_set_u`/`map_val_get_u`, `as i64` reinterprets) | **YES — dies** |
| `PrimMapU64<K requires …>` | :447 | the native column, raw bits both directions | **YES — dies** |
| `PrimMapF64<K requires …>` | :496 | the native column through `map_val_set_f`/`map_val_get_f` (`to_bits`/`from_bits` at the boundary — rut has no float bitcast) | **YES — dies** |

The "and so on" resolves to EXACTLY these three. Verified negatives:

- **No `PrimSet`** — the set has one spelling already (`HashSet<T>`,
  :341); nothing to fold.
- **No bool-val column** — deferred on record, not dropped silently:
  `?bool` cannot serve the `get -> ?V` nil law in today's checker
  (`p == nil` diagnoses the RFC 0004 §3 width rule; the repro is
  recorded at `nmapset.rut:373-379`). A bool val rides `PrimMapU64` as
  `0u64`/`1u64` today; under the family it rides the sidecar or the
  u64 row explicitly. Fixing `?bool` is MENU (a checker conversation,
  not a rename).
- **No narrow-val columns** (`i32`/`u32`/`u8`/… values) — the column
  is 64-bit; there was never a `PrimMapI32`. `HashMap<K, i32>` rides
  the sidecar today and keeps riding it (§2.4).
- **No float keys** — absent by design (no stable equality contract),
  unchanged by this batch.

The key machinery is the private `KeyLane` trait (:100-174,
implemented only for the closed set — "unimplementable by users"), and
the range-keyed methods (`put_range`/`get_range`/`has_range`/
`remove_range`, :286-335) sit on `HashMap` itself. Both are untouched
by the migration — they are already on the family name.

### 1.2 The consumer inventory — site by site

Every site that SPELLS a prim-specialized name, plus every live site
that spells the family names (the latter matter because the resolution
law changes underneath them, §2.5). Import-less uses are real:
the dep graph splices a source dep's FULL source into its consumer's
one compilation unit (`graph.rs` `Unit::Inline`), so todolist-web's
app.rut uses `PrimMapI64` with NO `use` (app.rut:106 documents this) —
the census greps by NAME, not by import.

**std pkgs**

| site | lines | what spells |
|---|---|---|
| `rut/nmapset/nmapset.rut` | :183/:341/:390/:447/:496 decls; :54/:71/:347/:357/:379 internal comments | the decls themselves (the names go private here; internal comments keep the story) |
| `rut/json/group-nmapset.rut` | :38 `use nmapset::{HashMap, HashSet, PrimMapI64, PrimMapU64, PrimMapF64}`; :196/:198, :223/:225, :260/:262 the three `impl JsonDeserialize for PrimMap*<K>` rows; :7-8 header | the impl matrix — the migration's hardest leg (§3.2) |
| `rut/json/json.rut` | :90 comment | "HashMap/HashSet/PrimMap via nmapset" |

**engine crates + integrations**

| site | lines | what spells |
|---|---|---|
| `crates/rut-lsp/src/std_surface.rs` | :14 doc comment; :104-105 comment; :111 `for want in ["HashMap", "HashSet", "PrimMapI64", "PrimMapU64", "PrimMapF64"]` | the std-surface test pins the five names as reachable bare — retunes to the two family names (§4.2) |
| `integrations/vscode-extension/test/e2e-wasm.js` | :203 `for (const want of ['HashMap', 'HashSet', 'PrimMapI64'])` | the wasm e2e completion pin — retunes with the vsix re-issue |
| `crates/rut-driver/tests/nmap_primmap.rs` | :2/:8-9/:90-91 comments; :95/:100, :220/:223, :277/:280, :319/:322, :358/:361, :403/:406, :422/:424 — seven embedded rut programs | the prim-map suite: parity-vs-sidecar legs, grow sweep, u64/f64 lanes, K admission, coexistence — the parity legs' CONTROL sides re-seat (§2.5) |
| `crates/rut-driver/tests/nmap_valcolumn.rs` | :233 `HashMap<i64, i64>` (the wrapper leg) | the column-vs-sidecar benchmark test — the sidecar leg re-seats |
| `crates/rut-driver/tests/json_pkg.rs` | :386 comment | u64-beyond-i64 rides PrimMapU64 |
| `crates/rut-driver/tests/data/jsonpkg/main.rut` | :17 use; :327 comment; :330-332, :341-343, :352-354 `decodeJson<PrimMap{I64,U64,F64}<str>>` | the json fixture's three Prim rows — re-spell, same checksums |
| `crates/rut-cli/tests/host_boxes.rs` | :60 | Rust std `HashMap<String, i64>` — HOST-side, not rut surface; excluded, recorded so nobody greps it later |

**examples/05-todolist-web**

| site | lines | what spells |
|---|---|---|
| `rut/app/todo_list/todo_list.rut` | :20 use; :27 return tuple `(Widget, PrimMapI64<str>, PrimMapI64<str>)`; :29/:30 mints | the dispatch tables → `HashMap<str, i64>` |
| `rut/app/app/app.rut` | :106 comment (the import-less splice law); :134/:135 fields; :147/:148 mints; :245/:258 rebinds | the toggles/removes tables → `HashMap<str, i64>`; the :106 comment rewords (the splice law stays true) |
| `rut/t1/core/t1.rut` | :53 use; :68/:78 `els: HashMap<str, opaque>`; :69/:79 `regs: HashMap<str, str>`; :70/:80/:169/:334 `lids: HashMap<str, i64>` | els/regs: NO spelling change. **lids RE-SEATS** (§2.5): sidecar-`i64` today, val-column under the rows |
| `rut/store/atom/atom.rut` | :45 comment (the phase-0c split law); :67 use; :77 `Rail.gens: HashMap<str, i64>` | the gens rail **RE-SEATS**; the :45 comment's verdict stays cited, updated to point at the rows |
| `rut/store/derived/derived.rut` | :42 use; :81/:86 `m: HashMap<str, i64>` | the dep-gens table **RE-SEATS** |
| `tests/t1_harness.rut` | :19 use; :283-328 els/regs | no spelling change |
| `tests/app_law.rs` | :238 `vec!["todo_row(", "PrimMapI64"]` | the SOURCE-SCAN pin — retunes to the new spelling |
| `src/web_dom.rs` | :235 comment | PrimMapI64 mention — rewords |
| `README.md` | :109 `gens: HashMap<str, i64>` (the rail, already the family spelling); :277; :367 "the dispatch table is `PrimMapI64<str>`" | :367 re-spells; :109 becomes the val-column's own documentation |
| `Cargo.toml` | :17 comment | rewords |
| `rut/app/todo_list/rut.toml` | :7 comment | rewords |
| `gen/web_host_bg.wasm` | — | UNTRACKED build artifact embedding spliced source text — regenerated by the example's own build; not a gate path |

**benches**

| site | lines | what spells |
|---|---|---|
| `workloads/nmap-primmap/main.rut` | :3 use; :6-9 comments; :17 `PrimMapI64<i32>` | the row's ONE spelling → `HashMap<i32, i64>`; **checksum 734932704, fuel 17,950,301, heap 324 B must hold bit-identically** (the alias resolves to the same class, §2.3) |
| `workloads/nmap-primmap/rut.toml` | :1-8 comment | rewords; `[deps]` unchanged |
| `workloads/nmap-primmap.js` | — | the twin is name-clean (JS `Map`) |
| `workloads/expected.json` | :25 `"nmap-primmap": "734932704"` | UNTOUCHED — the pin is the gate |
| `workloads/nmapset-int/main.rut` | :17 `HashMap<i32, i32>` | already generic, V=i32 → sidecar, NO change |
| `workloads/nmapset-str/main.rut` | :15 `HashMap<str, i32>` | NO change |
| `workloads/nmap-hashset/main.rut` | :16/:54 `HashSet<i32>` | NO change |
| `workloads/nmap-knucleotide/main.rut` | :35/:59/:62/:73 `HashMap<str, i32>` | NO change |
| `workloads/refvals/main.rut` | :62/:130 `HashMap<i64, Pt>` | NO change (V=Pt → sidecar) |
| `workloads/strview/main.rut` | :30 `HashMap<str, i32>` + range methods | NO change |
| `workloads/kmer-view/main.rut` | :33/:56-69 `HashMap<str, i32>` + range methods | NO change |
| `README.md` | :92-100 the rows table; :153 the escape-hatch note "(`HashMap`/`HashSet`/`PrimMap*`)"; :2181-2259 the nmap-primmap perf log (the parity bullet, the fuel/heap pins) | the rows table re-spells the primmap row; :153's escape note updates to the family; the perf-log NUMBERS are history — a phase note appends, nothing rewrites |

**demo**

| site | lines | what spells |
|---|---|---|
| `demo/src/examples/maps.rut` | :11-12 comment; :16 use; :42-44 `scores: PrimMapI64<str>` | → `HashMap<str, i64>`; the case's expected output is value-identical (same semantics) |
| `demo/src/examples/index.ts` | :196 blurb "HashMap/HashSet/PrimMapI64" | → the family wording |
| `demo/README.md` | :106 "the nmap lane (HashMap/PrimMapI64/HashSet)" | → the family wording |

**docs — the historical records (12 files, `PrimMap` hits):**
`rut-json-survey.md` (3), `rut-json-report.md` (1),
`demo-real-run-{survey,report}.md` (2/1), `err-channel-survey.md` (2),
`lsp-align-report.md` (2), `lsp-survey-{extension,lsp-wasm}.md` (7/7),
`orphan-rule-survey.md` (3), `todolist-web-report.md` (3),
`t1-design.md` (4), `todolist-restructure-survey.md` (2). These are
PAST BATCHES' records — the strbuild precedent (the historical
`31,543,783` pins were "that batch's record, not rewritten") applies:
they are carved out of the gate (§3.3) and keep the old spelling as
history. New batch docs (this survey, the phase report) use the family
spelling.

**clean:** root `README.md` (zero hits), `models.jsonc`,
`opencode.jsonc`, `rut/{core,calc,ink,pouch,rt,bench-cross,strbuild}`,
`examples/00-04`, `benches/probe` (mounts by pkg name only, :154 —
pkg names never change), the grammar corpus.

---

## 2. The mechanism pick (the crux, probed)

### 2.1 The probed state of the language (base 071b8c5, scratch compiles)

| probe | shape | result |
|---|---|---|
| A1 | `type SiMap = HashMap<str, i64>;` — a CONCRETE alias to a fully-applied instantiation, used as a type, constructed, method-called | **compiles + runs** (`a=1 len=1`) |
| A2 | `type M<K, V> = HashMap<K, V>;` — a generic alias head | **parse error**: `expected =, found '<'` |
| A3 | `type M<i64, V> = HashMap<i64, V>;` — the task's exact mixed head (concrete member + generic member) | **parse error**: `expected =, found '<'` |
| A4 | `type ScoreMap = PrimMapI64<str>;` — concrete alias onto the prim class | **compiles + runs** |
| B1 | a facade `Fac<K, V>` whose `get` ends `return map_val_get_u(..) as V;` — cast to a type param | **parse error** at `as V` |
| B2 | the get direction through the union bound: `V requires i64 \| u64 \| f64`, `put` stores `v as u64` (COMPILES — the set direction rides the widen law, exactly as round3 phase-0 recorded), `get` returns the raw `u64` where `?V` is expected | **checker error at the instantiation**: `return type mismatch: ?i64 expected, u64 returned`; the earlier no-call compile passed silently — bodies check per-instantiation, at the call site |
| C1 | cross-pkg alias export: alib declares `pub type Scores = PrimMapI64<str>;`, the consumer `use alib::{ Scores };` and declares `fn sink(m: Scores)` | **compiles** — the export row + the target-TypeId binding work |
| C2 | a module where the name `HashMap` is bound twice (class via use-both + an alias row of the same name) | **diagnostic**: `duplicate type name \`HashMap\`` — the seam the delta lifts |
| D1 | `impl Paint for B2` where `type B2 = Box2;` (a LOCAL class) | **diagnostic**: `impl target must be a struct or class of this module` — aliases do NOT expand in impl-target position |
| D2 | the same impl spelled on `Box2` directly | **compiles** — isolates D1 to alias expansion |

Conclusions, in the language's own words: RFC 0043 §1's alias is
non-generic (`TypeAliasFrame::step`, `item.rs:207-215`, parses `type
Ident = Ty;` — no parameter list exists to even reject), §5 lists
generic aliases as a non-goal, the parser enforces both (A2/A3); the
union bound is admission-only plus the widen law, and the GET
direction has no V value to widen FROM (B1/B2 — the round3 phase-0c
verdict re-confirmed with today's exact diagnostics); one static
receiver on a type param IS sanctioned now (`K.key_from_text(..)`,
json's group file) — but that rescues neither leg of the facade,
because the wall is STORAGE DIVERGENCE (one class body cannot hold the
column for `V=i64` and the sidecar for `V=Pt`), not call syntax.

### 2.2 The three candidates, assessed

1. **RFC 0043 aliases, per-instantiation rows** — TODAY: concrete-only
   (A1/A4 pass; A2/A3 parse-error). The generalization is a
   parser+checker delta with NO IR: the alias still resolves to the
   target's TypeId before any table work (RFC 0043 §4), so layouts,
   impls, fuel, and checksums are bit-identical BY CONSTRUCTION.
2. **A generic facade class with union-bound K (and V)** — DEAD
   today, on two independent legs: the get direction cannot re-type
   the column's raw bits to V (B1 parse, B2 checker), and the storage
   itself cannot diverge per instantiation (phase-0c; the atom pkg's
   `StrAtom` split is the same law applied one pkg over). Making the
   facade work needs the per-instantiation-divergence engine — a
   language conversation, priced in §2.6 as menu.
3. **Rename `HashMap` with prim columns as monomorphized storage** —
   the engine's monomorphizer/layout selects the column when
   `V ∈ {i64, u64, f64}`. Touches representation-selection machinery,
   needs new layout plumbing, and pins nothing BY CONSTRUCTION — the
   bit-identity would be a measured property, not a structural one.

### 2.3 THE PICK: alias rows (generalized RFC 0043 §1)

`HashMap` keeps meaning ONE family because the name resolves through a
ROW TABLE before layout ever runs:

```
// rut/nmapset/nmapset.rut — the public face (the ONLY new surface)
pub type HashMap<K, i64> = PrimMapI64<K>;
pub type HashMap<K, u64> = PrimMapU64<K>;
pub type HashMap<K, f64> = PrimMapF64<K>;
// the class HashMap<K requires …, V> stays the generic row (fallback)
// pub class HashMap<K requires i8 | … | bytes, V> { … }  — unchanged
```

- **Resolution law**: at a substitution-completing site, concrete row
  members match EXACTLY first (`HashMap<str, i64>` matches row 1 with
  `K = str` and resolves to `PrimMapI64<str>`), the
  generic class instantiates when no row matches (`HashMap<str, i32>`,
  `HashMap<i64, Pt>`, bare `HashMap<K, V>` in generic bodies).
  Ambiguity between a row and the class cannot arise — the row is
  strictly more specific; C2's `duplicate type name` lifts for
  alias-rows-sharing-a-class-name exactly and only in this
  concrete-shadows-generic shape.
- **The engine delta (the batch's bill, three seams, all
  parser/checker-level)**: (a) the alias head admits
  `(Ident | Type)` members (`item.rs:209` — the exact seam A2/A3
  bounced off); (b) resolution orders concrete rows before the
  class and expands the row to its target under the head's
  substitution; (c) impl targets expand aliases (D1→D2). No new
  `TyKind`, no opcode, no nat, no serialized surface row (RFC 0043
  §4: the in-memory surface is not serialized; bundles carry sources —
  nothing on the wire changes).
- **Why bit-identical by construction**: the row's target IS the
  prim class — the same TypeId the benches run today. `HashMap<i32,
  i64>` in nmap-primmap's source resolves to `PrimMapI64<i32>`'s
  descriptor, the same IR, the same crossings, the same fuel
  (17,950,301) and heap (324 B). This is `c066fbf`'s law — a pure
  rename at the resolution layer — with the alias table as the rename
  mechanism. json's impl registry keys by the target's TypeId, so
  `decodeJson<HashMap<str, i64>>` finds the impl registered through
  the same transparency (the impl re-spells per §3.2).
- **Why not the others**: the facade needs a language feature to
  express get AND dies on storage divergence anyway (§2.2.2);
  monomorphized storage is a bigger engine surface with measured
  (not structural) bit-identity (§2.2.3). The alias rows are the
  only pick whose bench identity is a THEOREM.

### 2.4 The row set today, and what deliberately has no row

`{i64, u64, f64}` — exactly the three classes that exist. No row for
`i32`/`u32`/`u8`/`u16`/`i8`/`i16`/`bool`/`str` values: narrow-val
columns were never built (the column is 64-bit; a narrowing row would
be a NEW class, NEW pins, a VERSION-adjacent conversation — MENU), and
the bool column is blocked on the `?bool` nil-law checker gap
(nmapset.rut:373-379 — MENU). No float KEYS (the equality contract —
unchanged). `HashSet` takes no rows (one spelling, one storage).

### 2.5 The re-seat consequence, disclosed before anyone chases it

Today FOUR live sites spell `HashMap<K, i64>` and get the `[?V]`
SIDECAR: todolist-web's `t1.rut` lids (:70/:80/:169/:334),
`derived.rut` (:81/:86), `atom.rut` `Rail.gens` (:77), and
`nmap_valcolumn.rs`'s wrapper leg (:233). Under the rows they RE-SEAT
to the val column — silently, by the directive's own uniformity. The
semantics are equal for prim vals (fresh opt per hit both ways; the
parity tests PIN this — `nmap_primmap.rs` 2598000), the demo's visible
output is value-identical, no bench row pins the sidecar-i64 shape
(nmapset-int is `V=i32`), and the re-seat is a PERF WIN (the column is
why PrimMap exists: −13.4% fuel, heap 1,966,551 → 324 B on the
identical op stream). The honest costs: (a) the `[?i64]`-sidecar
spelling becomes UNREACHABLE — anyone who truly wants it loses it (no
known consumer does; the four sites above all want ids/generations,
i.e. the faster lane); (b) the parity tests' control legs go VACUOUS —
`churn_sidecar`'s `HashMap<i32, i64>` resolves to the column and the
comparison compares the column with itself. Phase 1 retunes those two
legs (the raw-column legs of `nmap_valcolumn.rs` stay the meaningful
half; `nmap_primmap.rs` keeps the checksum pins and drops the
now-self-comparison, disclosed in its header).

### 2.6 The menu (engine work this batch does NOT do)

Narrow-val rows (`i32`/`u32`/… columns or narrowing wrappers — new
pins); the `?bool` checker fix un-blocking a bool row; per-
instantiation class divergence (would obsolete the three-class split
AND the atom pkg's `StrAtom`, and make the facade spellable — a
language-level RFC, not a rename); iteration surface for nmapset (the
json ENCODE half's recorded menu item — group-nmapset.rut:15-24);
prettier diagnostics for row misses (naming the row table).

---

## 3. The migration mapping

### 3.1 The old→new table (every spelling site, by §1.2's census rows)

| old spelling | new spelling | sites |
|---|---|---|
| `PrimMapI64<K>` (type position) | `HashMap<K, i64>` | todo_list.rut:27/:29/:30; app.rut:134/:135/:147/:148/:245/:258; maps.rut:44; jsonpkg fixture :330/:332; nmap_primmap.rs embedded srcs (:100/:223/:280/:406/:424); nmap-primmap/main.rut:17 |
| `PrimMapI64` bare (constructor) | `HashMap` | the same sites' `.new()` / `.with_capacity()` mints (the constructor name follows the spelling — `PrimMapI64.new()` → `HashMap.new()`, unambiguous in context) |
| `PrimMapU64<K>` | `HashMap<K, u64>` | jsonpkg fixture :341/:343; nmap_primmap.rs :322 |
| `PrimMapF64<K>` | `HashMap<K, f64>` | jsonpkg fixture :352/:354; nmap_primmap.rs :361 |
| `use nmapset::{ …, PrimMapI64, … }` | `use nmapset::{ HashMap, … }` (or DROP the use where the splice already scopes it — app.rut keeps none, todo_list/maps fold into their existing family use) | todo_list.rut:20; maps.rut:16; nmap-primmap/main.rut:3; jsonpkg fixture :17; nmap_primmap.rs :95/:220/:277/:319/:358/:403/:422; group-nmapset.rut:38 (→ `use nmapset::{HashMap, HashSet};`) |
| `impl JsonDeserialize for PrimMapI64<K>` | `impl JsonDeserialize for HashMap<K, i64>` (the row expands to the same target — D1's seam is the delta) | group-nmapset.rut :196/:223/:260 (and the :198/:225/:262 constructor mints) |
| comments/doc mentioning the Prim names (LIVE docs only) | family wording, the val column named in prose | nmapset.rut internal comments KEEP the names (private, §3.4); json.rut:90; std_surface.rs:14/:104-105; e2e-wasm.js:203; nmap_primmap.rs header; json_pkg.rs:386; web_dom.rs:235; Cargo.toml:17; todo_list/rut.toml:7; index.ts:196; demo/README.md:106; examples/05 README:367; benches/README.md:153 + the rows-table primmap entry; atom.rut:45; app.rut:106; nmap-primmap rut.toml + main.rut comments |
| `for want in […"PrimMapI64", "PrimMapU64", "PrimMapF64"]` | the two family names (the rows resolve to the targets — what a bare project needs is `HashMap`/`HashSet`) | std_surface.rs:111; e2e-wasm.js:203 |
| `vec!["todo_row(", "PrimMapI64"]` | `vec!["todo_row(", "HashMap<str, i64>"]` (the law-gate's set equality updated) | app_law.rs:238 |
| NO spelling change | — | nmapset-int/str/hashset/knucleotide/refvals/strview/kmer-view rows; t1.rut els/regs; t1_harness; expected.json; the LSP all-pkg surface rows themselves |

### 3.2 The json matrix, spelled out (the one non-mechanical leg)

`group-nmapset.rut` keeps its three decode rows, re-targeted through
the rows: `impl JsonDeserialize for HashMap<K, i64>` (expands to
`PrimMapI64<K>` — the SAME registration as today), likewise u64/f64;
the `use` slims to `{HashMap, HashSet}`. The fixture re-spells its
three `decodeJson<…>` rows; its checksums are value-defined over the
DECODED maps and cannot move (same keys, same values, same put
semantics). The orphan story is untouched (the impls stay in json's
unit, spliced by the peer gate).

### 3.3 The grep gate's exact form

Two tiers, run over the WORKING TREE (not `git grep` — untracked
consumer files must not hide), at every phase's end:

```sh
# TIER 1 — must be ZERO hits: every live surface
grep -rn "PrimMap" \
  rut/ crates/ integrations/ \
  examples/00-todolist examples/01-sort examples/02-digest \
  examples/03-plugin examples/04-custom-async examples/05-todolist-web \
  benches/workloads benches/README.md benches/probe \
  demo/src demo/README.md \
  README.md examples/README.md \
  --include="*.rut" --include="*.rs" --include="*.ts" --include="*.js" \
  --include="*.md" --include="*.toml" \
| grep -v "rut/nmapset/nmapset.rut" \
| grep -v "gen/web_host_bg.wasm"
#   exclusions: the private decls (the class bodies + the row table's
#   targets — §3.4) and the untracked regenerated wasm artifact.
#   docs/*.md is not in the path list — the historical records (§1.2).

# TIER 2 — the carve-out, allowed hits ONLY here:
#   rut/nmapset/nmapset.rut          (the private classes themselves)
#   docs/*.md historical batch records (12 files, §1.2)
#   /tmp scratch (not in the tree)
```

The Tier-1 allowlist is enforced by REVIEW, not by grep plumbing: the
only legitimate survivors are the private decls and history. The
negative set is also pinned: `PrimMapI64`/`PrimMapU64`/`PrimMapF64`/
`PrimSet` (never existed)/`PrimMap` (comment shorthand — dies with the
names).

### 3.4 Where the old names may survive

- **nmapset-INTERNAL only**: the three class declarations, their impl
  bodies, the row table's TARGETS, and the internal comments that tell
  the round3 story (`nmapset.rut` :54/:71/:347/:357/:379). A consumer
  that names them (`use nmapset::{PrimMapI64}`) FAILS once `pub`
  drops — that failure is the gate's enforcer.
- **Historical docs** (the 12 files, §1.2) — past batches' records.
- The untracked `gen/web_host_bg.wasm` until its next regeneration
  (not a source surface).

---

## 4. The calls

### 4.1 VERSION: NO BUMP (11 stays)

The delta is parser+checker resolution (§2.3) — zero new encoded
vocabulary: no `TyKind`, no opcode, no nat id, no boot type id, no
serialized surface row (the alias surface row RFC 0043 §4 added is
in-memory only; RFC 0038 bundles carry sources, so nothing on the wire
changes). Old v11 artifacts decode byte-identically both directions —
the no-bump reading WITH its shield intact. The representation is
unchanged underneath (the pick's theorem, §2.3), so no bench pin is
re-derived and no `expected.json` line moves.

### 4.2 The LSP: the std surface moves — wasm + vsix re-issued

The std surface's nmapset rows re-scope to the family: completions and
the bare-project index offer `HashMap`/`HashSet` (the rows resolve to
the same targets), and the Prim names LEAVE the surface.
`std_surface.rs`'s five-name test (:111) retunes to the two family
names + the `nmap_host` decl row (unchanged); the grammar corpus and
the e2e smokes ride the retuned surface (`e2e-wasm.js:203` — the
three-name pin becomes the family pair). House precedent (strbuild
phase 1): rebuild the wasm through `build:wasm`, re-run the corpus +
e2e through the REBUILT artifact, re-issue the vsix (0.2.3 → 0.2.4;
both artifacts gitignored build outputs — the md5s are the receipt).

### 4.3 The bench gates (bit-identity, the batch's proof)

- `nmap-primmap`: checksum `734932704`, fuel `17,950,301`, heap
  `324 B` — all three bit-identical with the re-spelled source (the
  pick's theorem made measurable).
- The four sibling pins hold: `nmapset-int` 734932704 (fuel
  20,703,284, heap 1,966,551), `nmapset-str` 1264308351 (9,551,761 /
  983,620), `nmap-knucleotide` 2198604 (heap 4,195,084),
  `nmap-hashset` 21500055 (heap 551) — none of their sources change.
- Full suite: 28 workloads × {rut, qjs, node}, exit 0, every checksum
  equal `expected.json`; `expected.json` UNTOUCHED by the batch.
- `cargo test --workspace` green (the retuned tests per §2.5), the
  wasm32-unknown-unknown check exit 0.

### 4.4 RFC 0043

Amended (phase 3, the report's verification pass): §1 grows the
per-instantiation row form with the concrete-shadows-generic
resolution law; §5's "no generic aliases" non-goal is narrowed to
"general generic aliases remain a non-goal — concrete-member rows are
the admitted form" (the nmapset realization cited, exactly as §3's
union-bound amendment cites its own). No new RFC.

---

## 5. The phase order — confirmed, with one sharpening

- **Phase 0** — this survey (docs only).
- **Phase 1** — the mechanism lands WITH its first consumer: the three
  parser/checker seams (§2.3 a-c), nmapset's row table + the `pub`
  drop, the test retunes (nmap_primmap/nmap_valcolumn per §2.5,
  jsonpkg fixture, json group matrix), the LSP surface move + wasm
  rebuild + vsix re-issue. One bisectable commit: the surface flips
  once, engine and first consumers together.
- **Phase 2** — the consumer sweep: examples/05 (the three re-seats +
  the dispatch tables + the law gate), demo, the nmap-primmap row
  re-spelling WITH its full pin run, the live READMEs — and the Tier-1
  gate at zero.
- **Phase 3** — the batch report + the RFC 0043 amendment
  verification (the strbuild report's shape).

Rationale: the engine delta is worthless unverified (phase 1 carries
the pkg's own tests as its proof); the consumer sweep is mechanical
ONLY after the surface settles (phase 2); the pins run where the
spelling moves, not in a separate gate phase. Confirmed against the
batch's original order — no better split found; the one sharpening is
binding the LSP/vsix re-issue to phase 1 (the surface move), not
phase 2.

---

## 6. Gates run this phase (docs only)

- `cargo test --workspace` — **779 passed / 0 failed** at base
  `071b8c5` (the landing record's number, reproduced; the full log +
  the aggregate under scratch).
- `cargo check --workspace --target wasm32-unknown-unknown` — exit 0.
- Tree clean after the commit; the staged set is this file ONLY, by
  explicit path (the parallel deploy lane's `demo/package.json` mod +
  untracked `scripts/` and `.wrangler/` untouched, not staged).

## 7. Scratch receipts

`/tmp/opencode/batch-hashmap-surface/p0/`: `probe-a1-concrete-alias.rut`
(+ `.log` via `probes-alias.log`), `probe-a2-generic-alias.rut`,
`probe-a3-mixed-head.rut`, `probe-a4-alias-to-prim.rut`,
`probe-b1-facade.rut` + `.log`, `probe-b2/` (module dir) +
`probe-b2-getdir.log`, `probe-c1-alias-pkg/` (alib + user) +
`probe-c1-alias-export.log`, `probe-c2-shadow.rut` + `.log`,
`probe-d1-impl-on-alias.rut` + `.log`,
`probe-d2-impl-direct.rut` + `.log`, `gates-*.log`. Zero repo files
touched by any probe; the CLI (`target/debug/rut`) mounts std by
presence for loose files, `nmap_host` via scratch `[deps]` for the
column probes.
