# the todolist-nonnull survey (phase 0)

**Batch:** todolist-nonnull · **Phase:** 0 (docs only) · **Base:** 81bc9f6 · **VERSION:** 11 · **Date:** 2026-09-24

The user's directive (Sep 24 2026): change `examples/05-todolist-web`, "remove so many nullable
types" like

```rut
struct AppRoot {
    store: ?TodoStore = nil; // THE ATOM STORE (was the plain ?Store)
    t1: ?T1Root = nil; // the framework's diff/registry state
    toggles: ?HashMap<str, i64> = nil; // subject -> todo id (§7.3)
    removes: ?HashMap<str, i64> = nil; // subject -> todo id (§7.3)
}
```

— "We only apply those fields really need to be nullable."

THE LAW this batch implements: a field is `?T` ONLY for TRUE ABSENT-STATE — nil is a MEANINGFUL
state (no selection, not logged in, not yet arrived). Phase-init convenience is NOT a reason.
Every surviving `?T` field carries a one-line comment naming its absent-state. Behavior
BYTE-IDENTICAL: the 93-test suite, the build, the browser drive — all green, output unchanged.

This survey maps the terrain phase 1 will cross — probed, not assumed. Every compile/fuel/op
receipt in §1–§5 ran on this exact tree through a scratch driver against copies of the real
`rut/` tree; receipts under `/tmp/opencode/batch-todolist-nonnull/p0/` (`probe-*.txt`; the
probe sources under `probe/`, `neg/`, and the five tree copies `wk/root/examples/05-todolist-web/rut-{base,conv,conv2,conv3,conv5}`).

**Gates at base (this tree, before this commit):** `cargo test --workspace` **782 passed / 0
failed**, of which the todolist gate is exactly **93** (app_law 3 + host_surface 22 + mount_lane 1
+ store 17 + t1_diff 18 + t1_lowering 14 + todolist_app 18 — count verified by `#[test]` grep and
by the run). Tree clean; this phase stages one .md file by explicit path.

---

## §1 the census

Every `?T` **field** in the examples/05 tree — 18 sites. Classification per the law:
**(a)** true absent-state (nil MEANS something — stays, keeps its naming comment), **(b)**
phase-init convenience (the convertible mass — nil is never observable), **(c)** blocked by a
real construction-order constraint.

| # | site | field | class | why |
|---|---|---|---|---|
| 1 | app.rut:133 | `AppRoot.store` | **(b)** | `main` mints `TodoStore.new()` (app.rut:150) BEFORE the literal; the note$ boot line runs before first paint. Nil is never observable. |
| 2 | app.rut:134 | `AppRoot.t1` | **(b)** | `t1_mount("app")` at the literal (app.rut:154); a complete T1Root (§1.2). |
| 3 | app.rut:135 | `AppRoot.toggles` | **(b)** | `HashMap.new()` before the literal; REPLACED whole each render (`r.toggles = out.1`, app.rut:317) — a re-assign, never an absence. |
| 4 | app.rut:136 | `AppRoot.removes` | **(b)** | same (app.rut:149, :318). |
| 5 | t1.rut:66 | `T1Root.parent` | **(b)** | `t1_mount` holds `ui_get(root_id)` before its literal (t1.rut:77). A miss is a LOUD crossing trap, never a rut-side nil. |
| 6 | t1.rut:67 | `T1Root.prev` | **(a)** | nil = "no previous tree — the first render takes the create path" (`t1_render`, t1.rut:112). The create/patch DISCRIMINATOR. No value exists at mount (§3). |
| 7 | t1.rut:68 | `T1Root.els` | **(b)** | minted in `t1_mount` before the literal (t1.rut:78). |
| 8 | t1.rut:69 | `T1Root.regs` | **(b)** | same (t1.rut:79). |
| 9 | t1.rut:70 | `T1Root.lids` | **(b)** | same (t1.rut:80). |
| 10 | atom.rut:135 | `Atom<T>.v` | **(b)** | `make(rail, name, init)` provides the value at construction (atom.rut:144); the file's own comment: "`v` is nil only pre-boot — `make` initializes at build." |
| 11 | atom.rut:137 | `Atom<T>.rail` | **(b)** | `make` takes the rail (atom.rut:143); one store, one rail — nil is never observable. |
| 12 | atom.rut:178 | `StrAtom.rail` | **(b)** | same (`StrAtom.make`, atom.rut:183-185). |
| 13 | todos.rut:163 | `TodoStore.rail` | **(b)** | NOTE: no `= nil` default at all — a REQUIRED optional, provided by `new()` (todos.rut:178, :181). Optional for no reason, not even the nil-default style. |
| 14 | todos.rut:550 | `ProbeStore.rail` | **(b)** | same (todos.rut:561, :565). |
| 15 | todos.rut:514 | `ProbeDown.up` | **(b)** | `ProbeDown.new(up)` takes the handle (todos.rut:520-522); `ProbeStore.new` builds `upv` BEFORE `downv` (todos.rut:562-563) — construction order already resolves. |
| 16 | widget.rut:44 | `Widget.tvar` | **(a)** | nil = "this kind carries no Text variant" — a Column/Row/Card has NO TVariant. The fat-struct discriminated union; the lowering's nil-check is a DESIGNED trap (`t1: Text without a variant`, lowering.rut:127-130) pinned by `t1p_trap_text_no_variant`. |
| 17 | widget.rut:45 | `Widget.bvar` | **(b/a)** see below | same shape (`t1: Button without a variant`, lowering.rut:134-137). |
| 18 | t1_harness.rut:32 | `Harness.root` (test fixture) | **(b)** | `Harness { root: t1_mount("app") }` at the literal (t1_harness.rut:40). |

**Counts: (a) = 3** (`prev`, `tvar`, `bvar`) · **(b) = 15** · **(c) = 0.**

A note on #17: `bvar` is (a) by the same argument as `tvar` — a non-Button widget has no button
variant, and a default (`BVariant.Primary`) would make every `col()` a primary button, which is
exactly the "nil means nothing" dishonesty the law forbids in the other direction. The census
marks it (a); nothing in the directive touches Widget, and the lowering's two variant traps are
twin-pinned behavior.

### §1.1 class (c) is EMPTY — and where the construction-order pressure actually lives

The task expected "(c) blocked by a REAL construction-order constraint — none or few". The
answer is **none**: value semantics forbid cycles, and every (b) field's value provably exists
before its record literal (§3). The ONE field where construction order genuinely cannot supply
a value is `T1Root.prev` — and `prev` is (a) anyway: the nil IS the state machine's
create-vs-patch bit. So no conversion in this batch is blocked; one survivor is *confirmed* by
the very same constraint. `neg/neg3_prev_shape.rut` proves the mechanism will not stop you from
"converting" prev — a plain `prev` forces a sentinel `Widget` minted for its own sake, whose
emptiness is indistinguishable from a real empty-keyed tree (the discriminator dies silently) —
which is why the call is recorded here and the checker cannot make it for you.

### §1.2 the shadow: locals, params, and the downcast's own `?`

The `?` pattern hides in three more masses the census must name (the directive's fields are the
head; these are the tail that makes the conversion honest or leaves it paying):

**The downcast-forced `?` (stays — engine law, TRUE absent-state).** `opaque.downcast<T>`
ALWAYS yields `?T` — the checker wraps unconditionally
(`crates/rut-lir/src/lir/call.rs:395-422`, `mk_opt(want)`; nil on a mismatch, RFC 0014
amended). Every container read-back in the tree rides it: app.rut:172, todos.rut:603/:791,
t1_harness.rut ×21, softfail.rut:24/:44. The entry-err shape `(?opaque, str)` (err-channel
phase 3) is the same law on the return side. **The box payload's type id DISTINGUISHES `?S`
from `S`** — probed end-to-end (`probe-boxprobe.txt`): `opaque(?S)` downcasts through
`downcast<?S>` (7) and MISSES `downcast<S>` (−1); `opaque(S)` is the mirror. So the mint-side
ascriptions `let root: ?AppRoot = AppRoot { .. }` (app.rut:152), `let s: ?TodoStore = ...`
(todos.rut:598/:786), `let h: ?Harness` (t1_harness.rut:40), `let t: ?Tally` (softfail.rut:19 —
the fixture's own comment names this law) are LOAD-BEARING: change them and every `downcast<?>`
spelling must change with them. Phase 1 keeps them.

**The field-shadow locals (die with the fields).** 27 ascribed `?`-locals copy a field into a
local before use — the pattern's whole hiding place: app.rut:150, 182, 194, 217, 246, 259, 283,
286, 298 (9); t1.rut:95, 128, 138, 149, 167, 169, 334, 338 (8); todos.rut:134, 139, 178, 498,
503, 540, 561 (7); todo_list.rut:30 (1); atom.rut:163, 204 (2). After a field converts, an
ascribed `?T` local reading it is a silent WRAP (§2.2) — these must convert WITH the fields or
the ops delta inverts (§5).

**The param/trait mass (convertible, receipts in §5).** `t1.rut`'s eight fns take
`r: ?T1Root`; `create_subtree`/`patch_children` take `parent: ?opaque`; `dispatch`/`paint`/
`view` take `?AppRoot`; `todo_list` takes `?TodoStore`; `Seen.note/stale` take `?Rail`; and the
`Derived<S>` trait itself spells `stale(self, st: ?S)` / `refresh(mut self, st: ?S)`
(derived.rut:65, :71). None has an absent-state meaning — a store handle is never nil.

**What stays ? by law, in fns:** `request_toggle/request_remove -> ?Req` (todos.rut:236/:243) —
nil = "no such todo", the caller's loud-drift trap; the four `HashMap.get` misses; `Rail.gen_of`'s
miss; `Seen.stale`'s never-computed miss; the lowering's variant traps. All true absent-state.

---

## §2 the mechanism answer

### §2.1 what non-nilable construction requires in rut

Nothing new. The mechanism is ALREADY the record literal: a field without a `= default` must be
provided at EVERY literal, and the checker enforces both directions:

- **omission** of a non-defaulted field diagnoses `field initializer type mismatch`
  (`neg/neg1_missing_field.rut` — the exact text; the "missing field" error is a type mismatch
  against the missing initializer);
- **writing nil back** into a converted field diagnoses `field assignment type mismatch`
  (`neg/neg2_nil_assign.rut` — `hold.h = nil;`). Once a field is plain, nil can never re-enter —
  the law is checker-enforced for the field's whole life, not a lint.

So the constructor shape is: **the existing constructor fns become the only construction path
with zero spelling change** — `TodoStore.new()`, `t1_mount()`, `Atom.make()`,
`ProbeStore.new()`, `ProbeDown.new()` already assemble everything; their record literals simply
stop being allowed to omit. There are no OTHER literals: the tree constructs each converted
type in exactly one place (`AppRoot {` only at app.rut:152; `T1Root {` only at t1.rut:81;
`Atom {`/`StrAtom {` only in their `make`s; `ProbeDown {` only in `ProbeDown.new`). A `new()`
assembling everything vs literals at call sites — the tree already IS the former. Phase 1 needs
no new constructor vocabulary, no engine change, no VERSION bump (nothing on the wire — the
record-literal ops and the field layout of a plain cell are the existing vocabulary).

### §2.2 per-field construction-order resolvability — probed, not assumed

Five scratch trees, each a full copy of the real `rut/` tree with symlinks into the repo's
`rut/pouch` + `rut/nmapset` (the manifests' relative deps), each compiled through
`load_dir_session` + `compile_graph` (the manifest lane, RFC 0045's four passes):

| tree | shape | app graph | t1_harness root | harness root |
|---|---|---|---|---|
| `rut-base` | verbatim | OK | OK | OK |
| `rut-conv` | **minimal**: the 15 (b) FIELDS only | **OK (first try)** | OK | OK |
| `rut-conv2` | fields + the 27 shadow locals + `parent: opaque` params + `Seen` rail params + `todo_list` param | OK | OK | OK |
| `rut-conv3` | conv2 + the `Derived` trait's `?S -> S` + `Atom`/`StrAtom` locals | OK | — | — |
| `rut-conv5` | conv3 + t1's eight `r: T1Root` params + the fixture's `Harness.root: T1Root` + its 27 locals | **OK** | **OK** | — |

`rut-conv` compiling FIRST TRY is the construction-order answer in one line: every (b) field's
value already exists before its literal; the checker needed no reordering anywhere. The
coercion story the minimal shape rides — `T -> ?T` wraps silently at ascribed lets and
arguments (that is what kept every unchanged `let mut els: ?HashMap<..> = r.els;` compiling) —
is exactly what costs it ops (§5).

The bound of the conversion, also probed: **the container's `?` is sticky.** conv4 attempted
`let mut r: AppRoot = root;` in `on_event` (root = the downcast result) and got
`let \`r\` is \`AppRoot\` but the initializer is \`??AppRoot\`` — the checker WRAPS a `?T`
initializer into a T-ascribed let instead of unwrapping (double-optional). There is no
unguarded `?T -> T` let; the only plain-valued reads flow through the nil-guarded branches
(e.g. `let old: Widget = prev;` inside `t1_render`'s else, t1.rut:115). So `on_event`'s `r`,
the `dispatch`/`paint`/`view` params, and every `downcast<?>` spelling keep their `?` —
correctly: that `?` is the mismatch leg, TRUE absent-state. The maximal shape (conv5) plains
everything UP TO that boundary and stops there.

### §2.3 the after-shapes

- `AppRoot` — the directive's four, verbatim:
  ```rut
  struct AppRoot {
      store: TodoStore; // THE ATOM STORE (was the plain ?Store)
      t1: T1Root; // the framework's diff/registry state
      toggles: HashMap<str, i64>; // subject -> todo id (§7.3)
      removes: HashMap<str, i64>; // subject -> todo id (§7.3)
  }
  ```
  `main` becomes `let mut store: TodoStore = TodoStore.new();` and keeps
  `let root: ?AppRoot = AppRoot { .. }` (the box-payload law, §1.2).
- `T1Root` — `parent: opaque; els/regs/lids: HashMap<..>;` and
  `prev: ?Widget = nil; // the previous tree — the diff's left side` (survivor, comment already
  names the absent-state).
- `Atom<T>` — `v: T; rail: Rail;` (`name: str = ""` unchanged); `StrAtom` — `rail: Rail;`.
- `TodoStore`/`ProbeStore` — `rail: Rail;` (the required-optional oddity just dies);
  `ProbeDown` — `up: ProbeUp;`.
- `Widget` — unchanged (`tvar`/`bvar` survive with their comments).
- The fns — field-shadow locals plain; `t1`'s param chain plain (conv5 shape) or left `?T1Root`
  (conv3 shape) — both compile; the maximal shape is the one that collects the full ops win.
- The survivors' comment law is ALREADY satisfied: `prev`, `tvar`, `bvar` carry one-line
  absent-state comments today (t1.rut:67, widget.rut:44-45).

---

## §3 the dying sites

**No nil-check dies.** All 16 in-`rut/` `== nil`/`!= nil` sites (20 with the test fixtures) sit
on TRUE absent-state values — map `get` misses, `?Req` returns, `prev`, `tvar`/`bvar`, the
downcast legs, `Seen.stale`'s never-computed. None reads an `AppRoot`/`T1Root`-table/`Atom`/
`Rail` field. The error messages therefore do not move either.

What dies is the COERCION MACHINERY around each converted-field read — the silent
wrap/deref pairs the ?-shadowing compiles to:

1. the ascribed-local wrap: `let mut X: ?T = r.field;` compiles `MakeOpt` (+ release traffic)
   when the field is plain — §5 shows the minimal fields-only shape ADDING ops for exactly this
   reason (on_event +32, t1_render +13);
2. the opt-deref `GetF`: every read through a `?T` field/local pays an extra field-0 load —
   181 `GetF` ops leave `on_event` in the maximal shape (1584 → 1403);
3. the implicit unwrap in `Atom<T>.get()` (`return self.v` from `-> T` with `v: ?T`,
   atom.rut:155-157) — becomes a plain copy;
4. `let p: opaque = parent;` (t1.rut:173) — the unwrap-after-nil-blind-cast of the `?opaque`
   parent param; with `parent: opaque` the line deletes (`ui_append(parent, el)` directly).

Quantified per function, compiled op streams, base → maximal (conv5) — `probe-ops-*.txt`:

| fn | base | conv (fields only) | conv3 (fields+shadow+trait) | conv5 (maximal) | Δ maximal |
|---|---|---|---|---|---|
| `on_event` (app unit: dispatch+paint+view inline) | 9980 | 10012 | 9828 | **9743** | −237 (−2.4%) |
| `t1_render` | 4109 | 4122 | 4087 | **4072** | −37 (−0.9%) |
| `create_subtree` | 2032 | 2038 | 2020 | **2014** | −18 (−0.9%) |
| `todo_list` | 721 | 718 | 715 | 715 | −6 |
| `TodoStore.refresh` | 151 | 150 | 145 | 145 | −6 (−4.0%) |
| `Counts.refresh` | 163 | 163 | 158 | 158 | −5 |
| `ProbeStore.refresh` | 56 | 54 | 53 | 53 | −3 |
| `Counts.stale` / `ProbeUp.stale` | 84 / 84 | 85 / 85 | 82 / 82 | 82 / 82 | −2 / −2 |
| `ProbeDown.stale` | 9 | 8 | 8 | 8 | −1 |
| `atom_str_set` / `atom_items_set` | 357 / 216 | 359 / 216 | 355 / 214 | 355 / 214 | −2 / −2 |

`MakeOpt` on the app unit: 110 → 61; on `t1_render`: 17 → 14. Fuel, scripted store session
(boot; 3 adds incl. one duplicate-answer rejection; toggle; remove; board/counts/fly/gen/drain/
recompute reads; fixture flush), driven through the REAL entry surface on a plain Vm:
**3750 → 3667 = −83 fuel (−2.2%)**, and the session's eleven printed rows are BYTE-IDENTICAL
across base/conv/conv3 (`probe-storefuel-*.txt`, `OUTPUTS-IDENTICAL` diff) — fewer ops, same
output, at the entry surface, today, on the un-converted engine.

---

## §4 the interactions

**The 93-test suite's exposure — no half-built roots anywhere.** The tree mints each converted
type in exactly one place (§2.1) and every mint is complete-at-construction; no fixture mints a
half-built AppRoot/T1Root/TodoStore. Suite-by-suite: `store.rs` (17) drives `TodoStore.rail`
through every entry — signatures UNCHANGED (the store entry surface is frozen, todos.rut:593);
`t1_diff.rs` (18) + `t1_lowering.rs` (14) read the converted T1Root tables SAME-UNIT through the
`t1p_*` probes (`root.els/regs/prev` — the inline law) — compiles unchanged over every conv tree
(`probe-root-harnesses.txt`); `todolist_app.rs` (18) drives AppRoot through the full CRUD
session; `app_law.rs` (3) pins the app's import set — untouched; `host_surface.rs` (22) is
web-crossing-side — untouched; `mount_lane.rs` (1) pins the wasm mirror to the same sources —
both lanes compile the same files, so the pin holds by construction. The `softfail.rut` fixture
DELIBERATELY boxes `?Tally` (its header comment is §1.2's law in miniature) — untouched by a
fields conversion.

**The t1 framework's own contract — one late-init field, and it is the survivor.** `prev`
demands late init BY DESIGN: the first render's create path IS `prev == nil`
(t1.rut:112-113), pinned by t1_diff's first-render/identity tests and `t1p_prev_key`'s ""
pre-render (t1_harness.rut:354-366). The other four fields are mount-complete — the framework
demands nothing late. `t1_mount` returns plain `T1Root` TODAY (t1.rut:76) — the type has never
been optional at its own boundary; only its fields were.

**The browser-drive paths phase 1 must exercise** (the converted fields are on every one):
the add flow (draft atom → `request_add` → pending line paints → timer commit → row appears),
the toggle flow (toggles-table hit → `request_toggle` → one `t1-check--on/--off` token patch),
the remove flow (removes-table hit → `request_remove` → row retires, registry rows drop), the
typing mirror (detail → draft$ → the field's value patch), the duplicate-title rejection (the
err-channel leg), and deadline ordering across kinds. In suite terms:
`todolist_app.rs`'s `boot_builds_the_shell` / `an_add_is_a_request_until_the_timer_answers` /
`toggle_round_trip_paints_the_flight` / `remove_round_trip` / `the_full_crud_session`, and
`tests/e2e-browser.mjs`'s `runSession` (tier 1 headless over the REAL wasm artifact; tier 2 the
live page when geckodriver exists) — the artifact lane phase 1 must re-run after conversion
(`cargo check --workspace --target wasm32-unknown-unknown` is the mount-lane gate that keeps
`src/mount.rs`'s mirror compiling).

---

## §5 the phase order, and what phase 1 takes

**Confirmed: survey (this commit) → phase 1 = the conversion + the proof.** No engine work is
required or wanted; VERSION 11 does not bump (zero new TyKind/opcode/nat/encoded vocabulary —
the bd104f8 precedent).

Phase 1's shape, per the receipts:

1. **Take the maximal conversion (conv5's shape), not the minimal one.** The directive names the
   fields; the receipts name the rest: the minimal fields-only tree compiles and is behavior-
   identical, but it measurably ADDS ops (on_event +32, t1_render +13) because every
   field-shadow local wraps its now-plain read (§3.1). The honest de-null is fields + shadow
   locals + the t1 param chain + the `Derived` trait's `?S` — one commit, one direction.
2. **Leave the boundary ?**: the downcast spellings, the mint-side ascriptions
   (app.rut:152, todos.rut:598/:786, t1_harness.rut:40, softfail.rut:19), `on_event`'s `r`,
   `dispatch`/`paint`/`view`'s `?AppRoot`, the `?Req` returns, `prev`, `tvar`, `bvar`, the
   entry-err shape. The survivors' comments already name their absent-states.
3. **Prove byte-identity**: the 93-test suite green, output unchanged; `wasm32-unknown-unknown`
   check green (the mount-lane mirror); the e2e lane tier 1 re-run; the twins' expected bytes
   untouched (the store suite's pinned lines are the same rows the scratch session diffed equal).
4. Docs ride per the strbuild precedent (the README/example references to the shapes, if any
   name the fields — none found in `examples/README.md`'s todolist section that spells `?T`).

**Deviations from the task brief, recorded:** (i) `Widget.bvar` is classified (a) here with the
`tvar` argument; the brief's matrix implied it might be convertible mass — it is not (the
lowering trap is pinned behavior). (ii) The brief expected dying unwrap/nil-check SITES; the
honest answer is zero nil-checks die and the win is coercion ops + fuel (§3) — quantified with a
scratch scripted session because the suite itself carries no fuel instrumentation. (iii) The
ops-delta inverted under the literal reading (fields only) — the survey recommends the wider
shape rather than reporting the inverted number as the result. (iv) conv4's `??AppRoot`
diagnostic is recorded as the mechanism's own statement of where the conversion must stop.
