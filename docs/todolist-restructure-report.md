# todolist-restructure — the batch report

The record of the todolist-restructure batch: four phases, one shared
tree, base `26b763e`, all on `master`. Companion to
`docs/todolist-restructure-survey.md` (phase 0 — the store census, the
mount decision, the finalized tree; its §7 deviation table is the
authoritative "what changed from the plan text"). The user directive
was two lines — fold the example's rut files into a `rut/` subdir as a
MEDIUM-SCALE project, and implement a tur-like store (jotai/riverpod
like) with per-file components — and the batch landed both without
moving one observable byte of behavior.

## 1. The story, in one section

The example's rut sources sat FLAT at the example root (`t1.rut` ~530
lines, `store.rut`, `todolist.rut`, `web.d.rut`) and mounted through
`src/mount.rs`'s inline `appkit` module — ONE fabricated module
existed only because the pre-dedup graph spliced each use's
transitive sources per use, and store and t1 both rode pouch. Four
commits:

1. **The survey** (phase 0, `024319c` — docs only). Three decisions
   made before anything moved. **The store census**: tur's edgy is
   id+seed atoms, values in a per-store KV with epochs, lazy derived
   recomputation over auto-tracked dep edges, a flush as the turn
   boundary — and the turn-law ruling picks the STORE ideas (the
   generation rail, mark-dirty-on-write/recompute-on-read, the dirty
   set as flush payload, equality-skip) while rejecting the
   REACTIVITY (live props/SubscriberGraph, watch, mutation atoms,
   auto-tracking, multi-store, cycle guard, families — each with its
   recorded reason). **The mount call**: the manifest route for real —
   `rut/rut.toml` project root via `load_dir_session` (RFC 0045's four
   passes) for the native lane, a per-package `register_module` mirror
   for wasm (the Session is I/O-free by law); `web.d.rut` becomes a
   `web/` host pkg per the calc precedent. **The tree**: per-package
   dirs under one-dir-one-module; two law-merges in t1 (builders into
   widget — inherent impls live in the type's module; diff/registry/
   render into the t1 core — field privacy); collision-free naming
   (`row` the component vs `todo_row`/`todo_list` the views). The test
   mapping corrected the plan's "15 sessions" to **18** (the
   err-channel pump tests) and pre-named ~8 new DOM-free atom twins.
2. **The tree** (phase 1, `2d0bf36` — structure only, zero behavior
   change). `rut/` = the project: the root manifest + `web/` + `t1/`
   (widget/lowering/core, core `inline = true`) + 8 component packages
   + `store/todos` (the OLD store verbatim — one risky thing per
   phase) + `app/` (app/todo_list/todo_row): **16 manifest packages**.
   `mount.rs` rewritten to the honest glue; the law gate rewritten to
   the new contract (the exact 13-package import set, per-package name
   pins, `store.refresh()` not yet in the positive law); the twins
   retargeted. The appkit-retirement proof landed all three legs
   (§3 below). 85 example tests green, assertions unchanged.
3. **The atom store** (phase 2, `6fbc080`). `rut/store/atom` (the
   Rail + `Atom<T>` + the `StrAtom` lane), `rut/store/derived` (the
   `Derived<S>` trait — dname/deps/stale/refresh — plus `Seen`
   dep-generations), `rut/store/todos` (the domain: `items$`/`reqs$`/
   `draft$`/`note$` as the store's FIELDS, `counts$` a DERIVED atom
   with deps declared `[items, reqs]`, the machine's write discipline
   — every Vec mutation ends in an EXPLICIT `set` because rut's
   by-reference cells make in-place mutation a silent write). The
   store split took the manifest count 16 → 18. The app rewires
   without moving: `paint()` gained exactly ONE line —
   `store.refresh()` before view — and the law gate now pins it; the
   status line is the SAME f-string bytes, now through the derivation.
   The 8 new DOM-free twins green beside the 9; **the 18 sessions
   green with `git diff` EMPTY on the file** — the freeze held, no
   deviation to record. Fuel/heap re-census (survey §4.7): nothing
   tripped, `limits()` stands (fuel 1M / heap 4 MiB), VERSION stays
   10.
4. **The close-out** (phase 3 — this commit). The live proof and the
   teaching: the agent-browser drive on the atom-store build (§4), the
   README rewritten to the landed shape (the medium-scale template,
   the atom-store teaching, the run/test recipe), this report.

The behavior law held end to end: the DOM contract (`#app`,
`#new-todo`, `#add-btn`, `#status`, `#list`, `#row-<id>`, child order
0=mark/1=title/2=del), the UI/theme, the status line's bytes, and all
18 session assertions are untouched across the whole batch.

## 2. The appkit-retirement proof (P1/P2/P3)

The workaround died with proof, not assertion (survey §2.5's plan,
executed in phase 1):

* **P1 — separate mounts compile.** The app's session mounts the
  packages as SEPARATE modules; everything compiles; all 18 sessions
  green. This was T10's shape at app scale, and both pre-registered
  "proof obligations" (survey §3.2) greened FOR REAL — no inline
  fallback needed: (a) the spliced t1 core binds linked
  `widget`/`lowering` surfaces; (b) linked class-method calls cross
  link boundaries (the app's `.gap()/.hook()` chains, the store's Vec
  calls). **No dedup gap surfaced** — so no dep-kinds bug report; the
  record's prediction ("the single-splice wrapper can dissolve")
  held.
* **P2 — the gate proves the name survives nowhere.** `app_law.rs`
  asserts the exact import set (no `appkit` in it) and that the string
  survives nowhere in `src/`. It earned its keep during phase 1's own
  rewrite — it caught the mount file's doc COMMENT still naming the
  retired module.
* **P3 — the two lanes agree, byte for byte.** `tests/mount_lane.rs`
  pins `load_dir_session(rut/)`'s mounted-name set + host surface to
  the wasm mirror's — and the compiled binaries to byte-identical.
  **It caught a real gap immediately**: the bare manifest session
  lacked the core prelude the mirror mounts. Fixed by the 03-plugin
  embedder precedent — the manifest resolves packages, the EMBEDDER
  mounts core — **zero engine changes**. Phase 2 kept P3 green
  through the store split (same name set, same surface, same binary),
  and the batch ends with it green.

The law behind the plan held: a dedup gap found would have been a
dep-kinds bug report, never a workaround re-entered. None was needed.

## 3. The store's test story

The atom store is DOM-free by import law, so its tests are DOM-free
too — `tests/store.rs` mounts the `todos` package as the root of its
own manifest subgraph and drives the PROBE surface (`atom_*` /
`probe_*` entries the app never sees):

* **the 9 machine laws, unchanged** — a request never touches the
  list; the answer commits; toggle/remove round trips; per-kind
  latencies differ; a rejected title is an ANSWER not a trap; an
  unknown tag traps loud; a lost id answers through the table.
* **the 8 atom twins (new)** — set/get with no rail traffic on reads;
  a write marks its dirty set ONCE (the set dedups, the gen moves per
  mark); the counts line is derived and reads fresh WITHIN a turn
  (exactly one recompute); staleness is per DECLARED dep (`draft$`/
  `note$` writes recompute nothing — the DAG's truth pinned); nothing
  leaks across a turn boundary (no-op turns recompute zero times, gens
  move only on writes); deriveds recompute upstream first in one pass
  (the `ProbeUp`/`ProbeDown` fixture — derived-on-derived as a
  store-test surface, not app surface); equality-skip is the str
  lane's scope (a `Vec` atom's `set` marks every time — the recorded
  asymmetry); the status line is `counts$` + `note$` verbatim.
* **families have NO test — they do not exist** (§4.6); the rejection
  is the record.
* **the sessions are the equivalence proof**: 18 scripted twin
  sessions with `git diff` EMPTY through the store swap — the counts
  line law, every turn law, and the deadline ordering now run THROUGH
  the derivation, asserted byte-for-byte as before.
* **the browser is the third witness**: tier 1 (the real wasm + glue
  through the loader ABI) and tier 2 (the real Firefox page) run the
  SAME session as the twin, and the phase-3 drive re-proves the themed
  states live (§4).

Example suite: 93 = 32 (t1 bed) + 18 (sessions) + 17 (store) + 22
(host surface) + 3 (law gate) + 1 (lane pin).

## 4. The live proof (phase 3's drive)

The standing agent-browser gate, on the atom-store build (rebuilt
artifact + fresh wasm-bindgen glue, the dir served over http, Chrome
via `AGENT_BROWSER_SESSION=p3-todolist`):

* open → snapshot: the boot line
  `0 open | 0 done | 0 in flight — booted — type a title, press Add`,
  the field and the Add button the only interactives.
* fill `#new-todo` "milk" → `… — typing 'milk'`, counts unmoved (the
  draft atom is NOT a `counts$` dep — the declared DAG visible in the
  negative).
* click `#add-btn` → the pending line `... milk` paints in
  `span.t1-text--pending`, the field clears (the diff cleared it),
  and the counts line flips to `0 open | 0 done | 1 in flight` — the
  `reqs$` write through the DERIVATION, on the click turn.
* the commit lands on the timer turn (add latency 400 ms):
  `1 open | 0 done | 0 in flight — added 'milk' as #1`, row
  `#row-1` with the `t1-check--off` token.
* a second ("tea") commits the same way; rows keep book order.
* toggle `#row-1` → in-flight `… — requested toggle #1` (caught at
  +80 ms), then `1 open | 1 done | 0 in flight — toggled #1 to done`;
  the check token flips to `t1-check--on` and the title wears
  `t1-text--done` — ONE token patch, the keyed diff's whole work.
* remove `#row-2` → `0 open | 1 done | 0 in flight — removed 'tea'
  (#2)`; `#row-2` is GONE from the DOM, `#row-1` remains first —
  order preserved.
* screenshots at boot / pending-flight / done-flip / final (verified
  visually — the indigo check, the strike-through, the italic pending
  line all render), then **zero page errors and an empty console**
  over the whole session; server killed after.

Scratch (log + screenshots): `/tmp/opencode/batch-todolist-restructure/p3/`.

## 5. Honest limits

**What the atom store is NOT.** There is no reactivity beyond the
turn: no live props, no watchers, no fine-grained DOM updates —
anything reading a derived outside `paint`'s refresh sees its cache by
design. There are no families (the per-row state IS the items list;
the rail's name key is the recorded door), no mutation atoms, no
second store, no cycle guard (the declared DAG is acyclic by
construction; a cycle is a declaration bug the twins catch). The
generic `Atom<T>.set` never equality-skips — rut's `==` is
type-directed, so a generic compare would compile to CELL IDENTITY
for `Vec<Todo>` and strand the deriveds; only the concrete `StrAtom`
lane skips on content. The deps are DECLARED, not discovered — the
one place the design is deliberately less clever than tur, bought at
the price of writing the dep list by hand and pinning it by test.

**Where the survey deviated from the plan's sketch, and why** (the
survey §7 table, restated): the t1 six-file sketch became THREE
packages (inherent impls live in the type's module; land-visibility
has no scoped forms yet, so T1Root's private tables force one
stateful package); "components/row.rut siblings in one dir" became
one PACKAGE per component (one directory = one module = one entry,
RFC 0035 §1); the app's `list.rut`/`row.rut` sketches survive as the
CLASS names `TodoList`/`TodoRow` because the component `row` took the
package name (first-mount-wins would silently skip a duplicate);
`web.d.rut` became the `web/` host package (a loose decl file is only
hand-mountable); "families if kept" was decided in phase 0 —
rejected, so phase 2 never argued about it; "all 15 sessions" was
measured at 18 before phase 1 gated on it; and the plan's
"components -> t1" refined to components -> `widget` ONLY (the biz
law at the component layer). Each was a compiler law or a measured
fact, recorded where it happened — none re-litigated here.

## 6. The gates, as landed

* the drive green with screenshots, zero page errors (§4)
* `cargo test --workspace` exit 0 — 94 result lines, zero failures
* `cargo check -p todolist-web --target wasm32-unknown-unknown` exit 0
* `node tests/e2e-browser.mjs` — BOTH tiers, ALL CHECKS PASSED
* `cargo test -p todolist-web` — 93/93
* bench pins untouched; `gen/` gitignored (the tree carries exactly
  the three docs files §8 lists — two rewritten/new, one pointer)
* the tree at commit: only the two docs files this phase owns

## 7. The MENU (what this batch grew)

* **verified `str` host-fn returns** (engine, carried from the t1
  batch): once fixed, `ui_input_value` returns as a surface row and
  the event-carried detail becomes an optimization instead of a law.
* **scoped visibility forms** (engine, NEW): `ast.rs` models
  `pub(crate)`-family qualifiers but the parser does not accept them;
  with them, the t1 core's diff/registry/render could have stayed the
  six-file sketch with `pub(crate)` fields instead of one stateful
  package. The merge is honest and recorded — this is the door that
  would reopen it.
* **structural `==` for records/Vecs** (engine, NEW): would let the
  GENERIC `Atom<T>.set` equality-skip safely and retire the
  `StrAtom`/`Atom<T>` split (the twins pin today's asymmetry either
  way).
* **M3 async as the real fix for the machine** (carried): at M3 the
  request table becomes futures, `tim_after` becomes a timer
  primitive — and the STORE semantics (atoms, the declared DAG, the
  turn's one flush) survive unchanged.
* **atom families** (design, deliberately left shut): the rail's name
  key is where `todo:<id>` cells would attach, and the declared-deps
  table is where dynamic deps would have to become honest first.
  No job here; the door is visible, not open.

## 8. Files (phase 3)

| file | change |
|---|---|
| `examples/05-todolist-web/README.md` | rewritten to the landed shape — the medium-scale template (the tree, the manifest route, both lanes, the import discipline), the atom-store teaching (tur taken/rejected, the jotai/riverpod mapping, the honest limits), the 93-test gate table, the run/test recipe |
| `docs/todolist-restructure-report.md` | new — this report |
| `examples/README.md` | the dep-kinds batch's appkit-retirement note gains one pointer: the retirement it recorded as a necessity has since been EXECUTED here (P1/P2/P3) — the historical text itself is untouched |

## 9. The commit

`phase(3): the todolist-restructure's close-out — the live proof, the README, the report` — the
drive (§4), the example README's rewrite, this report, and the
examples/README pointer, one commit, explicit paths only. The foreign
work in the shared tree (the SKILL.md mod, the monitor files,
`models.jsonc`, the stash) is untouched and unstaged.
