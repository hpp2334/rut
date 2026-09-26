# t1 — a toy widget framework in rut over the DOM crossings (the design)

- **Status:** phase-0 deliverable of the `todolist-ui-framework` batch — design
  only, no code. Phases 1–3 implement it.
- **Lives in:** example-local `t1.rut` (phase 1) under
  `examples/05-todolist-web/`, next to `store.rut` — the `rt` precedent for
  promotion is recorded in the survey's terms, not exercised here.
- **Reads first:** the batch plan (`todolist-ui-framework`), the survey
  (`docs/todolist-web-survey.md` §4–§5), RFC 0006 (enums — no data-carrying),
  RFC 0003 §1 (the state crosses), RFC 0009 (structs, recursive shapes legal),
  RFC 0042 (`str.slice` views).

## 0. The reference: tur.t1 RE-CHECKED — no longer empty

The phase-0 census input said `/home/a/Documents/Projects/tur.t1` was an empty
initialized repo. **It is populated now** (head `01d5fec`, a full
Flutter-parity JS rendering engine in Rust: `libs/`, `js/`, ~60 example cases,
`SKILL.md` agent cookbook). Per the standing rule its widget vocabulary and
builder shapes WIN where they fit. What was taken, concretely:

- **The names**: `Column`, `Row`, `Text`, `Button`, `Input` (→ our `Field`),
  the layout/control split. tur's `Container`/`Stack`/`Expanded` family stays
  OUT of t1's v1 — the todolist needs the set the plan names; more is menu
  (§8).
- **The single-props-object shape**: tur is `Text({ text: "hi" })` — every
  widget takes ONE options object, never positional soup (SKILL.md pitfall 2).
  rut has no object literals; the honest mapping is a struct whose fields ARE
  the props, defaulted, so construction names only what it means (§2.3).
- **Variant constants, not magic values**: tur's `MainAxisAlignment.Center`,
  `BoxFit.Contain` — namespaced enum members. t1: `Text.done`, `Btn.primary`
  as payloadless enums (RFC 0006's entire feature — and the json ruling's
  shape: payloadless kinds + structs).
- **`queryKey: ["count"]`** — tur's stable id for dev-tool targeting and
  integration tests on interactive/text elements. That is exactly this
  design's `.hook(id)` (§6): the testability law is tur's queryKey law,
  renamed to the batch's word.
- **Helper factories are plain functions** returning the element type
  (tur's `Button` helper in the counter case) — t1's constructors are the
  same shape: plain fns `text("milk")`, `btn("Add")` returning `Widget`.

What does NOT transfer, and why (recorded, not hand-waved):

- **tur has no re-render**: view fns run exactly once; props are live `Val<T>`
  atoms re-read per layout pass; "no diffing, no hooks" is a tur FAQ. t1
  CANNOT live there — the turn law re-enters rut per event and rut has no
  live-atom prop channel. So t1 re-renders `view(state)` per turn and pays
  for it with the keyed diff (§4). This is the one deep divergence from tur's
  model; it is forced by the host, not taste.
- **Children arrays / variadics**: tur's `Column({ children: [...] })` needs
  variadics or literals rut lacks — children append via fluent `.w(child)`
  (§2.3), per the plan.
- **JS-object props** (`Color.hex("#6366f1")`, `borderRadius: 8`): t1 has NO
  visual raw vocabulary at all — styling is the stylesheet's job keyed by
  class tokens (§5). The plan's law: semantic options only.
- **tur's closures-everywhere handlers** (`onClick: mutate(...)`) die at our
  boundary twice over: the closure law (RFC 0025) across host crossings, and
  the subject law even inside rut — widgets declare SEMANTIC subjects, the
  app routes them (§7.3).

## 1. The pain census (honest, evidence-cited)

Everything below is the CURRENT `todolist.rut` — the code phase 2 rewrites.
The laws it proves (the store round trips, the turn order, the loud traps)
are all PRESERVED; the census is about the expressiveness tax, not bugs.

1. **Manual DOM building.** The boot turn is ten raw crossings to say "a
   field, a button, a status line, an empty list" (`ui_create` + `ui_attr` +
   `ui_set_text` + `ui_append` quartets, `todolist.rut:81-97`). Every
   structural fact (a row is `li` > `button.mark` + `span` + `button.del`)
   is spelled tag-by-tag at the app level. The app is fluent in DOM when it
   should be fluent in todos.
2. **`ui_clear` + rebuild paints.** Every committed change repaints by
   clearing the list and rebuilding every row (`paint`, `todolist.rut:207ff`
   — the survey §5.3 strategy, chosen because the crossings are minimal and
   the app was simple). Consequences: the whole subtree churns per answer
   even when one checkbox flipped; and the browser throws away real DOM
   state (focus, transitions) on every paint — the default look can never
   animate because nothing survives long enough.
3. **Listener-id churn per paint.** Rebuild means re-register: every paint
   mints two fresh listener ids per row (`ui_listen` in the row loop), and
   the app drops the previous paint's ids from its action table first. The
   twin suite pays too — the `Ledger` helper in `tests/todolist_app.rs:76ff`
   exists SOLELY to predict which numeric ids the next paint will mint
   (`led.paint(&[1, 2])`), and `a_stale_row_listener_traps_loud` is a test
   about id arithmetic, not about todos. Stale-id loudness is a LAW worth
   keeping (§4.3); minting ids per paint is not.
4. **The packed action-number dispatch.** `actions.put(f"{m}", t.id * 10 +
   ROLE_TOGGLE)` and its inverse `a % 10` / `a / 10` (`todolist.rut:58-63,
   154-156, 248-249`): the listener id is a str, the action a packed i64, and
   the semantics live in digit arithmetic. It is parse-free (rut's `str`
   surface has no `to_int` — the packing was the honest move) but it is
   exactly the vocabulary the plan bans at the widget level: numbers where
   subjects belong.
5. **The default browser look.** `index.html` has NO stylesheet — not a
   single `<style>` line. The page is Times New Roman with gray buttons;
   `class` attributes the app sets (`row`, `mark`, `del`, `pending`) name a
   stylesheet that does not exist. There is no styling surface for phase 2's
   "beautiful" to land on.

## 2. The widget set

### 2.1 The law

Biz code composes WIDGETS. It never names `Node`, a tag, a class string, a
`web::` crossing, or a listener id. The grep gate (phase 2) makes the law a
gate: `todolist.rut` (post-rewrite) contains none of `Node`, `web::`,
`ui_`, `class=`, tag literals. Widgets carry SEMANTIC options only:
`variant`, `gap`, `pad`, `hook`, `key`, `subject` — never `tag` or `class`.

### 2.2 The set (v1 — exactly what the todolist needs)

| widget | kind | semantic options | lowers to (§4.1) |
|---|---|---|---|
| `Column` | layout | `gap`, `pad`, `hook`, `key`, children | `div.t1-col` |
| `Row` | layout | `gap`, `pad`, `hook`, `key`, children | `div.t1-row` |
| `Spacer` | layout | `size` | `div.t1-spacer` |
| `Card` | layout | `pad`, `gap`, `hook`, `key`, children | `div.t1-card` |
| `Text` | control | `variant` (title/body/done/pending/muted) | `span.t1-text.t1-text--<v>` |
| `Button` | control | `variant` (primary/quiet), `subject` | `button.t1-btn.t1-btn--<v>` |
| `Checkbox` | control | `on: bool`, `subject` | `button[role=checkbox].t1-check.t1-check--on/off` |
| `Button.subject` / `Checkbox.subject` / `Field.subject` | — | the event's semantic name | listener spec (§4.1) |
| `Field` | control | `placeholder`, `value`, `subject`, `hook` | `input.t1-field` |

- `Text` variants: `title` (page header), `body` (a row title), `done`
  (struck, muted — a completed todo), `pending` (the `... title` in-flight
  row), `muted` (the counts line).
- `Button` variants: `primary` (Add), `quiet` (the row's mark and del — today
  `[ ]`/`[x]`/`del` in a real font).
- `Checkbox` is the committed row's done mark: `on` is app state, the widget
  renders `t1-check--on/--off`, the click fires its subject. It lowers to a
  framework-owned button, NOT `input[type=checkbox]`: a native checkbox
  toggles itself visually before the turn answers, and the crossings could
  not read it back anyway (values are event-carried; `ui_attr` can set the
  `checked` attribute but the real DOM's live property stops obeying the
  attribute once a user has clicked — the twin's plain attrs would never see
  the desync, the real page would). Framework-owned visuals = state is 100%
  the app's, repaint is exactly one class-token patch. The `input`-based
  alternative is recorded here as considered-and-rejected.
- **List helpers**: rows are just widgets with keys — a todo list is
  `Column.gap(8)` whose children are `Row`s carrying `.key(f"row-{t.id}")`.
  The KEY is the list helper: it is what the keyed diff (§4.2) matches on.
  A closure-taking `Each` (tur's shape) is legal rut-to-rut but stays OUT of
  v1 — data-shaped rows keep the whole tree inspectable by the twin (§6);
  recorded as a menu item (§8).

### 2.3 Widget = plain rut data — the representation, DECIDED

RFC 0006's entire feature is payloadless enums ("no data-carrying enums"),
and the json ruling (`examples/02-digest/digest.rut:652ff`) shows the house
idiom for sums: a tag enum + ONE struct with (defaulted) payload fields.
Both phase-0 scratch proofs compiled and ran trap-free under
`/tmp/opencode/batch-todolist-ui/p0/` (`builders.rut`, `recurs2.rut`):

```rut
// the kinds — payloadless (RFC 0006), the json ruling's JTag shape
enum WKind  { Column, Row, Spacer, Card, Text, Button, Checkbox, Field }
enum TVariant { Title, Body, Done, Pending, Muted }
enum BVariant { Primary, Quiet }

struct Widget {
    kind:     WKind  = WKind.Text;   // the tag
    // -- controls
    text:     str    = "";
    tvar:     ?TVariant = nil;        // Text's variant
    bvar:     ?BVariant = nil;        // Button's variant
    on:       bool   = false;         // Checkbox
    placeholder: str = "";
    value:    str    = "";            // Field (mirrors the draft)
    // -- semantic options (NEVER tag/class)
    subject:  str    = "";            // what this widget fires
    hook:     str    = "";            // stable test id (§6)
    key:      str    = "";            // keyed-diff key (defaults from hook)
    gap:      i32    = 0;   pad: i32 = 0;   size: i32 = 0;
    // -- layout
    children: Vec<Widget> = Vec.new();
}
```

Decisions inside the decision:

- **One fat struct, not a struct family.** A family (`struct Column`,
  `struct Text`, …) needs a common type for `children`, and with no
  data-carrying enums that common type is... a fat struct again. One struct,
  kinds as enums, irrelevant fields defaulted — the `Json` shape verbatim.
- **Recursive `Vec<Widget>` is legal.** RFC 0009 §6: "recursive shapes are
  legal because composite fields are pointer-sized". The digest comment
  claiming recursion is inexpressible is STALE (it predates the RC work);
  the scratch proof (`recurs2.rut`) builds and walks a 3-node `Widget` tree
  trap-free. No `opaque` boxing needed.
- **`?TVariant` optional fields**, not sentinel kinds: RFC 0006's enum has no
  "unset", and `nil` on a `?T` is the house absence (the downcast law).

### 2.4 Builders — fluent, tur-shaped

No variadics, so children append one at a time; constructors are plain
functions (tur's helper-factory idiom); chaining is `mut self -> Self`
(`impl Widget`, scratch-proven):

```rut
// t1.rut's public surface (phase 1) — everything biz names
fn col() -> Widget;  fn row() -> Widget;  fn card() -> Widget;
fn spacer(size: i32) -> Widget;
fn text(t: str) -> Widget;              fn done(t: str) -> Widget;
fn title(t: str) -> Widget;             fn pending(t: str) -> Widget;
fn muted(t: str) -> Widget;
fn btn(label: str) -> Widget;           fn quiet(label: str) -> Widget;
fn check(on: bool) -> Widget;
fn field() -> Widget;
// the fluent members (impl Widget): .w(child) append, .gap/.pad/.size,
// .hook(id), .key(id), .subject(s), .value(v), .on(b)
```

The phase-2 header row reads (the acceptance shape the doc is designing
toward):

```rut
let mut head = row().gap(8).hook("head");
head.w(field().placeholder("What needs doing?").value(r.draft)
        .subject("type").hook("new-todo"));
head.w(btn("Add").variant_primary().subject("add").hook("add-btn"));
```

## 3. The pain census → the framework's jobs

| pain (§1) | t1's answer |
|---|---|
| manual DOM building | the widget set; tags/classes are the LOWERING's private vocabulary |
| clear + rebuild paints | the keyed diff/patch (§4.2) — minimal ops over the 10 crossings |
| listener-id churn | register-on-appear, retire-on-remove; ids live as long as their widget (§4.3) |
| packed action numbers | semantic subjects; the framework owns id↔subject tables; biz routes subjects (§7.3) |
| default browser look | the stylesheet keyed by lowered tokens + the designed theme (§5) |

## 4. The lowering (framework-private, never exported to biz)

### 4.1 Widget -> Node

One lowering table, total over `WKind`, in `t1.rut` where biz cannot see it:

| WKind | tag | class tokens | listens |
|---|---|---|---|
| Column | `div` | `t1-col` | — |
| Row | `div` | `t1-row` | — |
| Spacer | `div` | `t1-spacer` | — |
| Card | `div` | `t1-card` | — |
| Text | `span` | `t1-text t1-text--title/body/done/pending/muted` | — |
| Button | `button` | `t1-btn t1-btn--primary/quiet` | `"click"` -> subject |
| Checkbox | `button` | `t1-check t1-check--on/off`, attr `role=checkbox` | `"click"` -> subject |
| Field | `input` | `t1-field`, attr `placeholder` | `"input"` -> subject (event-carried value) |

- The variant ENUM MEMBER is the token: `TVariant.Done -> "t1-text--done"` —
  a 1:1 map, no string munging anywhere.
- Attributes via `ui_attr` only: `id` (the hook, §6), `class` (the tokens,
  space-joined), `placeholder`, `role`. The app never spells any of them.
- Text content via `ui_set_text`; the field's VALUE via `ui_set_input_value`
  (the boundary law keeps it input-only — and Field is the one widget that
  IS an input, by construction).
- `key`: defaults to the `hook`; explicit `.key(...)` for keyless repeated
  rows. Keys are the diff's identity, hooks are the tests' handles; they
  coincide in this app (rows hook as `row-<id>`), which is fine — they answer
  different laws.

### 4.2 The keyed diff/patch over the 10 crossings

Per turn: build the new `Widget` tree (`view(state)`), diff against the
previous tree held in `T1Root`, apply the OPS. No crossing grows:

- **match by key** at each level of children: same key = same node —
  patch it in place (text/class/value/attr deltas only); new key = create
  (`ui_create` + attrs + `ui_append`); gone key = remove (`ui_remove`) and
  retire (§4.3).
- **reorder** = re-`ui_append` surviving keyed children in the new order —
  `appendChild` MOVES (the DOM's own law, the twin implements it:
  `fake_dom.rs:324`), so a reorder costs N appends and zero creates.
- **patch-in-place deltas**: text changed -> `ui_set_text`; class tokens
  changed -> `ui_attr(class)`; value changed -> `ui_set_input_value`;
  nothing changed -> NO crossing fires. This is what makes the §1.2 pain go
  away: a toggle no longer rebuilds the list, it patches one row's tokens.
- the ops are applied in parent-before-child order so handles stay live;
  `T1Root` holds the previous Widget tree (for diffing) and the key -> handle
  table (for applying) — plain rut data, both.

### 4.3 The listener registry — tied to the patch, stale = LOUD

- The framework owns ONE table: listener id -> subject (`HashMap<str, str>`
  in T1Root — the KEY is the id exactly as the crossing spells it, so
  `deliver` is one lookup with no `str`->`i64` parse anywhere; the value is
  the widget's subject).
- **register on appear**: the patcher creates a node whose lowering has a
  `listens` row — exactly once per widget LIFETIME, not per paint
  (`ui_listen(el, "click")` -> `regs.put(f"{id}", subject)`; the id crosses
  as i64, the table spells it str once at registration and never again).
- **retire on remove**: the patcher's remove op drops the subtree's ids from
  `regs` (`regs.remove(f"{id}")`). The Ledger's churn dance dies: a row's
  id is minted when the row APPEARS and lives until the row LEAVES.
- **stale fire = LOUD, the existing law**: `deliver` looks the firing id up
  in `regs`; a miss panics
  `t1: listener '<id>' answered no subject — stale or unknown`
  (the app's current trap shape, moved into the framework; the twin's
  `a_stale_row_listener_traps_loud` law is preserved verbatim, phase 2
  re-points it at the framework's message).
- **honest limit, recorded**: there is no `ui_unlisten` crossing and the 10
  do not grow. Retirement is rut-side table work. On the real page the
  backend's `Closure`s live for the page's life (web_dom.rs's documented
  phase-1 limitation) — but a retired id belongs to a DETACHED node, which
  no user can click; the twin CAN fire it by id, and when it does, the trap
  above is what fires. Loudness is the compensation; the leak stays a
  documented non-widened gap.

## 5. Styling — the stylesheet keyed by the LOWERED tokens

`index.html` gains one `<style>` block (phase 2). Every rule keys a token
the LOWERING emitted — biz never writes a class name, the stylesheet never
names a widget field; the variant->token map (§4.1) is the whole contract.

Designed default theme (the phase-2 acceptance):

- **Layout**: page max-width card on a soft background; `t1-col` stacks as
  flex column. `gap`/`pad`/`size` are DATA, so the lowering emits them as
  UTILITY tokens on the same class attribute: `gap(8)` -> `t1-gap-8` — a
  FIXED ladder the stylesheet enumerates (`t1-gap-4/8/12/16`, `t1-pad-*`,
  `t1-size-*`). No arbitrary values, no inline `style` attribute: the token
  set stays enumerable and twin-assertible, and the styling surface never
  leaks into biz as free-form CSS.
- **Typography**: system-ui stack; `t1-text--title` large/600; `body` normal;
  `muted` small/50% ink; `done` line-through + muted; `pending` italic + 60%
  ink (the `... title` row reads as in-flight, not broken).
- **States**: `t1-check--on` draws the checked glyph (accent background);
  `--off` the empty box; `t1-btn--primary` accent-filled, `--quiet` ghost;
  hover/focus rings on both (the real DOM's own affordances — the twin and
  tier 1 ignore CSS entirely, which is fine: they assert the TOKENS, not the
  pixels; the pixels are phase 3's agent-browser acceptance).
- **Transitions**: `background-color`, `opacity`, `transform` transitions on
  the check/button tokens — POSSIBLE now precisely because §4.2 keeps nodes
  alive across patches: a class-token change on a surviving node animates.
  Under the old clear+rebuild nothing survived long enough to animate; the
  diff is what buys the beauty.

## 6. Testability — `.hook(id)` and the twin as the framework's test bed

- `.hook(id)` is a WIDGET OPTION; the lowering emits it as the `id`
  attribute; the keyed diff preserves it (hooks survive patching by
  construction — same key = same node = same id). The twin's readers
  (`snapshot_by_id`, `text_of`, `attr_of`, `child_texts_of`) and `ui_get`'s
  search keep working UNCHANGED — the twin gains nothing, loses nothing
  (host-lane freeze respected). The e2e tiers keep asserting through the
  hooks; tier 2's CSS selectors ride `#<hook>` and `#<hook> .t1-btn`.
- Firing events in phase-2 tests: the `Ledger` dies. The twin suite reads
  `WebState.listeners` (id -> element + event, already host state, no host
  change) to find the id whose element carries the hook, then `fire(id)` as
  today. Numeric ids become plumbing; hooks become the tests' vocabulary.
- **The fake-DOM twin is the framework's test bed** (phase 1's suite):
  - **lowering snapshots**: build a widget tree, render once, assert the
    twin's tag/class/text tree (`snapshot_by_id` gives exactly that);
  - **diff correctness**: patch text-only / class-only / value-only and
    assert the minimal op (the element's OTHER reads unchanged);
  - **keyed reorder**: flip two rows, assert order swapped AND both rows'
    listener ids UNCHANGED — which is also the **minimal-op proof**: a
    rebuild would have minted fresh ids, so stable ids = no rebuild;
  - **listener-leak checks**: remove a keyed row, assert its ids left the
    registry, then fire one and assert the LOUD trap (§4.3);
  - **the trap matrix through the patcher**: unknown subject, wrong-kind
    payload (a `Text` with children is a lowering panic — the fat struct's
    one dishonesty, caught at the boundary), field value on a non-input,
    stale timer tags — all loud, all named.

## 7. Turn integration

### 7.1 State stays in AppRoot (RFC 0003)

No mutable module state; the container crosses; the pump re-passes it. t1
adds NO new state channel — the patch state is a FIELD:

```rut
struct AppRoot {
    store:   ?Store = nil;      // the "server" (untouched)
    t1:      ?T1Root = nil;     // the patch state — a FIELD of the container
    draft:   str = "";          // the field's event-carried value
    toggles: ?PrimMapI64<str> = nil;  // subject -> todo id (§7.3)
    removes: ?PrimMapI64<str> = nil;  // subject -> todo id (§7.3)
}

struct T1Root {
    parent: ?opaque = nil;      // the mounted root (#app)
    prev:   ?Widget = nil;      // the previous tree — the diff's left side
    els:    ?HashMap<str, opaque> = nil;  // key -> live handle (op application)
    regs:   ?HashMap<str, str> = nil;     // listener id (as crossed) -> subject (§4.3)
}
```

(The map types are nmapset's exact surfaces: `PrimMapI64<K>` is i64-VALUED
with closed-set keys — the app's `actions: PrimMapI64<str>` is str-keyed —
and `HashMap<K, V>` is the generic-value map for the handle/subject tables.
`opaque` handles held as fields and in `Vec<opaque>`/`HashMap` values across
turns are the crossing's own intended currency — web.d.rut's preamble.)

### 7.2 One render per turn

```rut
entry fn on_event(c: opaque, kind: i32, subject: str, detail: str) {
    // ... kind 1: biz dispatch (§7.3); kind 2: store.answer(subject) ...
    render(root.t1, view_of(root));   // THE render — once per turn, any turn
}

fn view_of(r: AppRoot) -> Widget {   // PURE: reads store + draft, builds widgets
    ...
}
```

`view_of` is pure data-in/data-out (twin-assertible on its own); `render` is
the only thing that talks to the DOM; the typing turn (§1's set_status trick)
stays cheap automatically — the diff patches one text node.

### 7.3 The semantic-subject dispatch (packed numbers die)

Widgets fire SUBJECTS. The flow of a row click:

1. the browser fires the row's listener id (minted ONCE when the row
   appeared);
2. the host delivers `(kind=1, subject="<id>", detail="")` — the crossing
   unchanged;
3. `t1.deliver` translates: `regs` maps the id -> the widget's subject, e.g.
   `"toggle:3"` (miss = LOUD, §4.3);
4. biz routes the WHOLE subject through ITS OWN tables — built at
   widget-construction time right next to the `.subject(...)` call, so the
   todo id and the role are MAP LOOKUPS, never arithmetic (the packing dies)
   and never string parsing (rut's `str` surface has no `to_int`; a
   prefix-slice dead end was considered and rejected — whole-subject routing
   keeps the "no parsing, the crossing's own currency" law):

```rut
// built in the row loop, beside .subject(f"toggle:{t.id}"):
toggles.put(f"toggle:{t.id}", t.id);
removes.put(f"remove:{t.id}", t.id);

// dispatch: page-level subjects by equality; row subjects by table hit —
// WHICH table hits names the role, WHAT it yields is the todo id
if (subject == "add")         { ... }   // book the add, paint
else if (subject == "type")   { r.draft = detail; ... }  // event-carried
else if (toggles.get(subject) != nil) { book_toggle(r, toggles.get(subject)); }
else if (removes.get(subject) != nil) { book_remove(r, removes.get(subject)); }
else { panic(f"t1: subject '{subject}' answers no action — stale or unknown"); }
```

The app-side tables retire with each render exactly like today's action
table (drop-then-refill where the widgets are built), so a stale subject — a
row removed while its event was in flight — is a LOUD miss, the same law the
packed table had.

## 8. What t1 does NOT do (deviations, limits, menu)

- **No crossing grows.** All 10 rows of `web.d.rut` stand; no `ui_unlisten`,
  no style crossing, no read-back (values stay event-carried). §4.3 records
  the retirement limit this forces.
- **No host/engine/example-neighbor changes.** `store.rut` byte-identical;
  the twin's FakeNode/Backend surface untouched (phase-2 tests READ it, not
  change it); bench pins + expected.json untouched.
- **tur's live-prop model does not transfer** (§0) — the diff is the
  honest rut-shaped substitute, and its cost is bounded by the turn law
  (one diff per event, small trees).
- **No closure-taking `Each`, no `Condition`/`Switch`** in v1 — biz builds
  rows with a loop and keys. Menu: closure-shaped list/branch helpers once
  phase 2 shows the loop idiom chafes.
- **Checkbox is framework-owned** (button, not native input) — §2.2's
  rejected-alternative note.
- **No arbitrary style values**: the gap/pad ladder is fixed tokens; a
  `style`-attr escape hatch is explicitly NOT in the API (it would smuggle
  the styling surface back into biz).
- **The twin and tier 1 ignore CSS** — they assert tokens and structure;
  the "beautiful" acceptance is tier 2 + agent-browser screenshots, phase 3.

## 9. Phase gates this design feeds (recap, unchanged from the plan)

- **Phase 1**: `t1.rut` (widgets, lowering, diff, registry, T1Root) +
  the twin framework suite (§6). `cargo test -p todolist-web` green;
  workspace + wasm32 green; no crossing/host/engine changes.
- **Phase 2**: `todolist.rut` rewritten on the widgets (§2.4's shape),
  `store.rut` untouched, the no-Node-in-biz grep gate, the 15 twin sessions
  adapted to hooks (§6), the stylesheet (§5).
- **Phase 3**: tier 2 e2e on the lowered DOM + agent-browser live drive;
  README teaches both lessons; deviations menu closed out.
