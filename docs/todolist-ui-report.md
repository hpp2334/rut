# todolist-ui-framework — the batch report

The batch: give the browser example a UI worth the name — a toy widget
framework in rut over the ten unchanged host crossings, the todolist
rewritten to compose it, and the proof that the whole thing runs —
beautifully — on a real page. One example: `examples/05-todolist-web`.
Base `7149d27` (the prior batch's landed app), all work on `master`.
Design of record: `docs/t1-design.md` (phase 0). Prior batch's report:
`docs/todolist-web-report.md` (the host surface and the app's turn
law — both untouched here).

## The premise

The phase-1-shape app was fluent in the wrong language: ten raw
crossings to say "a field, a button, a status line", clear-and-rebuild
paints that churned every row per answer, listener ids minted per
paint (and a test-side `Ledger` whose only job was predicting them),
packed `id*10+role` action numbers, and the browser's default look.
None of that is a bug — the laws all held. It is an expressiveness
tax, and the design's §1 census prices it honestly. The batch's bet:
a thin widget framework (one fat struct, a lowering table, a keyed
diff, a listener registry) makes the tax disappear WITHOUT widening
the host surface — and the bet paid.

## The phases

**Phase 0 — the design** (`75db1f9`): `docs/t1-design.md`. The pain
census (§1, evidence-cited against the current source), the widget set
(§2 — semantic options only, no tag/class vocabulary in biz), the
lowering + keyed diff + listener registry laws (§4), the styling
contract — the stylesheet keys ONLY lowered tokens (§5), the
`.hook(id)` testability law (§6), the turn integration — state stays
in `AppRoot`, one render per turn, semantic-subject dispatch (§7),
the recorded deviations (§8), and the tur.t1 re-check (§0) that fixed
the widget vocabulary (below).

**Phase 1 — the core** (`a27419f`): `t1.rut` — the fat `Widget`
struct + payloadless kind/variant enums (RFC 0006, the json ruling's
idiom), plain-fn constructors + fluent builders (`.w/.gap/.pad/.size/
.hook/.key/.subject/.value/.placeholder/.on`), the lowering TOTAL
over kinds (loud panics: a control with children, a variantless Text,
an off-ladder token, a listener with no subject), the keyed diff/patch
(match by key with PATH keys, patch-in-place deltas, new keys create,
gone keys remove, reorder = re-appends through the modelled live
order), the registry tied to the patch (register once per widget
LIFETIME, retire on remove, stale fire = the named loud trap), and
`t1_mount/t1_render/t1_subject`. The framework's bed: 32 twin tests —
14 lowering snapshots + 18 diff/lifecycle/trap proofs. The app still
painted its old way this phase; nothing consumed t1 yet.

**Phase 2 — the rewrite** (`7149d27`): `todolist.rut` composed of
WIDGETS ONLY — three components (`view`/`todo_list`/`todo_row`),
`view` pure, `paint` = `t1_render` ONCE per turn, semantic subjects
(`add`, `field`, `toggle:<id>`, `remove:<id>`) routed by equality and
map hits — the packed numbers and their `%10//10` arithmetic die; the
manual DOM quartets, the clear+rebuild paints, and the per-paint
listener churn die with them. The in-flight cue becomes a variant
patch (`t1-text--pending`) instead of mark-text surgery; a toggle is
exactly one class-token patch. `index.html` grows the designed
stylesheet keyed ONLY by lowered tokens — the beauty the diff bought
(transitions animate because keyed nodes survive). `tests/app_law.rs`
enforces the §2.1 law by reading the app's source (import set exactly
`{appkit}`, no DOM vocabulary anywhere in the file, widgets provably
the whole UI story). The 15 twin sessions adapted to the lowered DOM
with semantics identical; the stable hooks (`#app`, `#new-todo`,
`#add-btn`, `#status`, `#list`, `#row-<id>`, child order 0=mark
1=title 2=del) kept so phase 3 needed only selector-level e2e updates.

**Phase 3 — the proof + close-out** (this commit): the e2e gate
adapted to the lowered DOM (both tiers green), the real-page drive
with an agent (the "beautiful" acceptance, screenshots on disk), the
README's two-lesson rewrite, this report. NO app/framework/host/
engine changes — the deviations are recorded, not re-litigated.

## Phase 3's deliverables and the verification record

**The e2e, adapted (both tiers green).** `tests/e2e-browser.mjs` kept
its `runSession` byte-for-byte — the behavioral assertions (`[ ] milk|
[ ] tea`, deadline order across kinds, the counts lines) read exactly
as before. What changed is the ADAPTERS that translate the lowered
DOM at the selectors: committed rows are `#list .t1-row` (hooks
`row-<id>`, child order unchanged), a check's state is the
`t1-check--on/--off` token on the framework-owned
`button[role=checkbox]` — mapped back to `[x]`/`[ ]` so the session's
assertions keep their meaning — and a pending line (or an in-flight
row title, the same variant) is `span.t1-text--pending`. Tier 1's
node fake needed one FIDELITY fix: its `appendChild` pushed a
duplicate when the diff re-appended a live child, while a real DOM
MOVES it (the rust twin has modelled this at `fake_dom.rs` all along,
and tier 2 on the real page was correct) — the keyed diff's reorder
path was the first customer of re-append on the JS fake. Result:
**47 checks, 0 failures** (tier 1: 26, tier 2: 21 — tier 2 omits the
five ABI-level checks; both run the same session), the artifact
rebuilt fresh first (`cargo build -p todolist-web --target
wasm32-unknown-unknown --release`; stale relative to the phase-2
source).

**The agent-browser proof (the acceptance).** The example served over
`python3 -m http.server` (127.0.0.1:8137), a named agent-browser
session (`p3-todolist-proof`, headless Chromium via CDP) drove the
REAL page — the real wasm, the real glue, the real stylesheet:

1. `open` → `snapshot -i`: the a11y tree shows exactly the textbox
   ("What needs doing?") and the Add button — the framework's field
   and primary button are the page's whole interactive surface.
2. Boot verified on live elements via `get text`/`eval`:
   `#app > div` = `t1-col t1-gap-12 t1-pad-16`, `#new-todo` =
   `t1-field`, `#add-btn` = `t1-btn t1-btn--primary`, `#status` =
   `t1-text t1-text--muted`, `#list` = `t1-card t1-gap-8 t1-pad-12`;
   the counts line reads `0 open | 0 done | 0 in flight — booted —
   type a title, press Add`. Screenshot:
   `/tmp/opencode/batch-todolist-ui/p3/shot-1-boot.png`.
3. `fill "#new-todo" "milk"` — the status line carried the typing;
   `click "#add-btn"` — the flight verified MID-FLIGHT on live
   elements: `1 in flight — requested add 'milk'`, `#list` = one
   `span.t1-text--pending` reading `... milk`, the field EMPTY (the
   diff cleared it through the input crossing). Screenshot:
   `.../shot-2-pending-flight.png` — the italic pending line in the
   elevated card, caught in flight.
4. The commit: `row-1` = `div.t1-row.t1-gap-8` with child order
   `button#mark-1.t1-check.t1-check--off[role=checkbox]` ·
   `span#lbl-1.t1-text--body` "milk" · `button#del-1.t1-btn--quiet`.
   A second add ("tea") flew the same pending token and committed
   after it — book order, `2 open | 0 done`.
5. `click "#mark-2"` — the in-flight themed state verified on the
   LIVE element: the check still `t1-check--off`, the title wearing
   `t1-text--pending` and reading `... tea`; after the answer
   (`toggled #2 to done`): `t1-check--on` + `t1-text--done` — the
   flip IS the one-token patch the design promised. Screenshot:
   `.../shot-3-done-flip.png` — the filled indigo check, the struck
   done title.
6. `click "#del-1"` — `requested remove #1` with the flying row's
   title reading `... milk`, then `removed 'milk' (#1)` and only
   `row-2` remains. The empty-draft gate on the live page: an Add
   with an empty field answers `type a title first` and books
   nothing.
7. Page errors: none; console clean. Server killed, browser session
   closed. Final screenshot: `.../shot-4-final.png`.

**Gates at this commit:**

* `node tests/e2e-browser.mjs` — exit 0, **47 checks, 0 failures**
  (tier 1: 26, tier 2: 21), both tiers green on the rebuilt artifact.
* `cargo test --workspace` — exit 0, **643 passed, 0 failed**, 85
  suites (phase 2's 643, unchanged: no rust code moved in this
  phase).
* `cargo check --workspace --target wasm32-unknown-unknown` — exit 0.
* bench pins (paranoia; the engine is untouched) — `node
  benches/run.mjs --runtime rut`, exit 0, fuel/heap bit-identical to
  the standing records: crossing-nop 104000032 / 236 B, json-decode
  111322915 / 32.78 MB, nmap-knucleotide 38814389 / 4.00 MB,
  nmapset-str 9551761 / 960.6 KB.
* the tree lands at HEAD-clean (the e2e's `gen/` glue is gitignored;
  the screenshots live in scratch; nothing outside this phase's three
  files is touched).

## The tur.t1 transfer record (design §0, as landed)

tur.t1 (head `01d5fec`) is a Flutter-parity JS rendering engine; its
widget vocabulary and builder shapes win where they fit. What came
over, concretely:

* **the names and the split** — `Column`, `Row`, `Text`, `Button`,
  `Input` (→ `Field`), and the layout/control divide. tur's
  `Container`/`Stack`/`Expanded` family stayed out of v1 (menu).
* **the single-props-object shape** — tur is `Text({ text: "hi" })`.
  rut has no object literals; the honest mapping is one defaulted
  struct whose fields ARE the props, so construction names only what
  it means.
* **variant constants, not magic values** — tur's
  `MainAxisAlignment.Center` became t1's payloadless variant enums
  (`TVariant`/`BVariant`), which lower 1:1 to the class tokens.
* **`queryKey` → `.hook(id)`** — tur's stable id for dev-tool
  targeting is exactly t1's testability law, renamed to the batch's
  word; the lowering emits it as the `id` attr, and the whole test
  suite (twin and e2e alike) speaks it.
* **plain-fn helper factories** — tur's `Button` helper shape became
  t1's `text("milk")`, `btn("Add")`, `check(on)` constructors.

What the turn law rejected (recorded, not hand-waved):

* **tur's no-re-render model** — view fns run once, props are live
  `Val<T>` atoms re-read per layout pass. The turn law re-enters rut
  per event and rut has no live-atom prop channel, so t1 re-renders
  `view(state)` per turn and pays for it with the keyed diff. The one
  deep divergence — forced by the host, not taste.
* **children arrays / variadics** — `Column({ children: [...] })`
  needs literals rut lacks; children append via fluent `.w(child)`.
* **JS-object visual props** (`Color.hex`, `borderRadius`) — t1 has
  no visual raw vocabulary at all; styling is the stylesheet keyed by
  class tokens.
* **closures-everywhere handlers** (`onClick: mutate(...)`) — dead
  twice over: the closure law (RFC 0025) across host crossings, and
  the subject law even inside rut (widgets declare SEMANTIC subjects;
  the app routes them).

## Deviations (recorded, not re-litigated)

The design §8's five, as landed:

1. **Checkbox-as-button** — `check` lowers to a framework-owned
   `button[role=checkbox]` with the `t1-check--on/--off` token, never
   a native input. App state is 100% rut's; a flip is one class-token
   patch and animates because the keyed diff keeps the node alive.
   (Recorded gap: the a11y tree reads the driven checkbox as
   unchecked — `aria-checked` rides no token yet; menu.)
2. **No `ui_unlisten`** — listener retirement is rut-side registry
   work: the patch's gone-key pass drops the removed subtree's
   `els`/`regs`/`lids` rows (the `lids` path→listener-id table is the
   one recorded addition to §7.1's `T1Root`); the crossings stay ten;
   a fire on a retired id is the registry's LOUD trap.
3. **The token ladder** — `gap/pad/size` lower to the FIXED rungs
   `t1-{gap,pad,size}-{4,8,12,16}`; 0 emits no token; anything off
   the ladder is a loud lowering trap. The token set stays
   enumerable and twin-assertible, and no `style` attr ever exists to
   smuggle CSS back into biz — the whole arbitrary-value surface is
   sixteen stylesheet rules, and it is empty.
4. **No `Each` in v1** — there is no closure-taking list combinator;
   rows are keyed widgets from a plain loop (`for (let t of
   store.todos())`), which is what lets the key law stay simple:
   explicit `.key`, else `.hook`, else positional.
5. **The appkit mount splice** — the mount offers the store + t1 as
   ONE inline module `appkit`, not two `use`s: the graph splices each
   use's transitive sources per use with NO cross-use dedup, and
   store and t1 both ride pouch — two uses would define `Vec` twice;
   spliced together they define everything once. The unit's bound
   surface carries the host crossings, which is how the app's one
   non-widget name (`tim_after`) resolves with no crossing import,
   and the law gate pins the import set so nothing else rides in.

## The MENU (what this batch grew)

* **cross-use dedup in the mount graph** — deviation 5's workaround
  is the engine work: splice each pkg's sources ONCE per unit instead
  of per use. It feeds the dep-kinds work directly (a `use` is
  already a graph edge; the dedup is a set-union over its closure)
  and would dissolve `appkit` back into two honest `use` lines.
* **`Each`** — the keyed-rows loop is fine for one list; a real UI
  wants the closure-taking combinator once the closure law gains the
  rut-side half. v1's plain loop keeps the door honest: `Each` must
  lower to exactly what the loop lowers to today.
* **`ui_unlisten` as a host-surface candidate** — retirement is
  rut-side table work today because the crossings are frozen at ten;
  a real unlisten crossing would retire listeners in the HOST (no
  stray fire can even reach rut) and let the registry drop the `lids`
  table. The trap law would not change.
* **an aria state token for the framework checkbox** — the drive's
  a11y snapshot reads the on-state checkbox as unchecked
  (`aria-checked` is nobody's job yet); a variant-companion attr
  emission is a small, honest follow-up.

## Files (phase 3)

| file | change |
|---|---|
| `examples/05-todolist-web/tests/e2e-browser.mjs` | the tier-2 selectors + tier-1 adapters translated to the lowered DOM (tokens → the session's `[x]`/`[ ]`, `t1-text--pending` lines, `.t1-row` rows); the tier-1 fake's `appendChild` models the real DOM's MOVE; `runSession` byte-stable |
| `examples/05-todolist-web/README.md` | rewritten — the example now teaches TWO lessons (the async pattern on the turn law + the widget framework over a thin host), the run recipe, the three-lane test story |
| `docs/todolist-ui-report.md` | new — this report |

Everything else — `t1.rut`, `todolist.rut`, `store.rut`, `index.html`,
`loader.js`, `web.d.rut`, the host lane, the engine, the bench pins —
byte-untouched.
