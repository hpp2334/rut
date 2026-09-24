# 05-todolist-web — a web page whose brain is rut

One page app — a todolist with a simulated server — where **every DOM
move and every timer goes through ten declared host crossings**, and
the page's brain is a **rut project** (`rut/`, 18 packages): the biz
program (`app/`), a toy widget framework (`t1/`), a per-kind component
vocabulary (`components/`), and an **atom store** (`store/` — the
tur-adapted state layer) — all mounted through the manifest, pure rut
with no `main`, no event loop, and no async keywords. The host
(web-sys on wasm32, a fake-DOM twin on native) owns the loop; rut owns
the state.

The page's UI is not hand-built DOM: the app composes **widgets**, and
the framework's keyed diff lowers them to those same ten crossings.
Biz speaks widgets, subjects and hooks; tags, class tokens, listener
ids and the patch loop are the framework's private business.

The example is **example-local** by the phase-0 survey's decision
(`docs/todolist-web-survey.md` §4): the `web` pkg lives in this
crate's `rut/web/` package, not in the std workspace.

## What it teaches

**1. The async pattern: a hand-written machine.** rut today has no
async/await (the engine stops at the M3 wall — survey §2.2), but a web
page is nothing *but* async. The pattern that fills the gap:

* **state lives in ONE container the host holds.** rut has no mutable
  module state (RFC 0003 §1 — module lets are load-time literals), so
  `main` builds `AppRoot{store, t1, toggles, removes}` and RETURNS it
  as an opaque; the pump hands it back on every turn:

  ```rut
  entry fn main() -> opaque;
  entry fn on_event(c: opaque, kind: i32, subject: str, detail: str) -> (?opaque, str);
  ```

* **every asynchronous fact is an event row.** One entry, one shape —
  kind 1 = EV_DOM, kind 2 = EV_TIMER; the host is the only writer of
  the queue (a FIFO with a re-entrancy guard — `events are queue,
  never stack`).
* **the turn answers the entry-err pair** (err-channel phase 3). A
  clean turn returns `(opaque(c), "")` — the container re-crosses, the
  state-crossing law made explicit. A SOFT FAILURE returns
  `(nil, why)`: the pump decodes the err, reports it, and **keeps
  draining** — the page lives, the container stays usable. A panic is
  still drift: it kills the pump loud and never crosses as data. The
  twin containment proofs live in `tests/todolist_app.rs`
  (`softfail.rut` isolates the law).
* **the "server" is a request table, not a scheduler.** An add never
  touches the list: `store.request_add` books a `Req` row (tag, kind,
  latency) and the UI books one `tim_after(req.latency, req.tag)` —
  the host's clock, RFC 0018. The list changes only when the timer's
  turn calls `store.answer(tag)` — the one commit point, per-kind
  latency (add 400 / toggle 250 / remove 120 ms) so deadline-ordered
  commits are *visible across kinds*. A rejected title and a lost id
  are ANSWERS (data the turn reads), not traps; an answer naming no
  booked row traps loud.
* **the round trip is visible in the tree**: the pending line
  `... title` paints immediately (the `t1-text--pending` variant), an
  in-flight row's title wears the same variant, and the committed row
  appears only when the answer lands as its own turn.

This shape ages well: at M3 the request table becomes real futures and
`tim_after` becomes a timer primitive — the t1 survey §5.6 spells the
migration; the app's turn law and store semantics do not change.

**2. The widget framework: biz never sinks to the DOM.** The framework
(`docs/t1-design.md` is its design — authoritative, including its
recorded deviations) makes a UI a VALUE:

* **widgets are plain rut data** — one fat `Widget` struct with
  payloadless kind/variant enums (the json ruling's shape, RFC 0006)
  and recursive `Vec<Widget>` children. `view(r)` is a PURE function
  from app state to a `Widget` tree; it runs no crossing and writes no
  atom.
* **the lowering is framework-private and total.** `view` returns go
  through `t1_render`, whose diff lowers through ONE table mapping
  each kind to its tag, class tokens and event — `check` lowers to
  `button[role=checkbox]` with the `t1-check--on/--off` token, `col`
  to `div.t1-col`. A control with children, a Text without a variant,
  or a listener without a subject is a LOUD lowering panic.
* **the keyed diff patches in place.** Match children by key
  (`.key(...)`, defaulting from `.hook(...)`) with PATH keys, fire
  only deltas: nothing changed = nothing fires, a toggle is exactly
  one class-token patch, a reorder is re-appends (appendChild moves),
  gone keys retire their listeners. No clear-and-rebuild paint
  exists — which is also what makes the CSS transitions in
  `index.html` possible: nodes survive long enough to animate.
* **semantic subjects, not ids.** Widgets fire NAMES (`"add"`,
  `"toggle:<id>"`); the framework owns the listener-id registry, so
  the app never sees a number the host minted. Dispatch is equality
  and map hits; a stale subject is a loud miss.
* **styling is a token contract.** `index.html`'s stylesheet keys
  ONLY the lowered tokens (the `t1-*` families, the fixed utility
  ladder, variant tokens); the app spells no class and the stylesheet
  names no widget field. The §4.1 table IS the styling contract, and
  the twin asserts it.

**3. The atom store: tur's store ideas under the turn law** (the
restructure survey §4 — authoritative). The state layer is a small
reactive store, adapted from tur's edgy (`reactive/store.rs`) — the
jotai/riverpod family of shapes — to rut's ONE law: **render happens
once per turn, through the keyed diff**. What that means concretely:

```rut
// rut/store/atom — the cell layer
pub class Rail {                  // the generation rail, name-keyed
    gens: HashMap<str, i64>;      // atom name -> current generation
    dirty: Vec<str>;              // the turn's dirty set (deduped)
    fn mark(mut self, name: str) -> nil;   // dedup-push + gen bump
    fn drain(mut self) -> Vec<str>;        // take + clear = the turn boundary
}
pub class Atom<T> {               // the field IS the cell
    fn get(self) -> T;            // plain read — no tracking
    fn set(mut self, x: T) -> nil; // store + rail.mark(self.name)
}
pub class StrAtom { /* Atom<str> with equality-skip on set */ }
```

* **atoms are the store's FIELDS** — `items$`, `reqs$`, `draft$`,
  `note$` on `TodoStore`, built once at boot. No handle minting, no
  module-scope atoms (illegal anyway, RFC 0003 §1). The NAME is the
  rail's key — the seam a future family would attach to.
* **deriveds declare their DAG.** rut has no closures to auto-track
  through, so a derived is a trait impl: `name()`, `deps()` (the
  DECLARED dependency list), `refresh()` (reads deps, stores value).
  The domain has exactly one: `counts$`, `deps = [items, reqs]`,
  producing `"N open | M done | K in flight"` — the counts line law is
  now proven THROUGH the derivation.
* **the flush is the turn boundary.** `paint()` calls
  `store.refresh()` ONCE, before `view`: drain the dirty set, walk the
  deriveds in declaration order, recompute each stale one AT MOST once
  (staleness = a dep's generation moved past what the derived's `seen`
  recorded). Writes mark; nobody recomputes until the refresh. A turn
  that writes nothing recomputes nothing.
* **subscriptions mark dirty — they never push to the DOM.** The
  atom layer adds bookkeeping BEFORE the view, never conditionality
  around the diff. The render law is untouched.

**What came from tur** (survey §1.2): the generation rail (a cached
derived is servable iff its recorded generations still match);
mark-dirty-on-write / recompute-on-read (tur's pull model with the
flush point pinned to the turn); the dirty SET as the flush payload
(N writes to one atom in a turn mark once); equality-skip on the
`str` lane (type-directed `==` — generic compare would be cell
identity for `Vec`, so `Atom<T>.set` never skips: recorded
asymmetry, pinned by a twin); atoms as cells a store MATERIALIZES,
never globals.

**What the turn law REJECTED** (survey §1.3 — recorded, each with its
reason): tur's **live-props / SubscriberGraph** per-element
re-rendering (the `Val<T>` model, rejected a second time on purpose —
the keyed diff does that job once per turn); **`watch()`** and the
watch-loop guard (reactive push callbacks are reactivity, not state
cells; the app's async facts are the host's timer tags); **mutation
atoms** with the `{get,set}` ctx (rut's one `on_event` entry IS the
mutation surface); **auto-tracking via closure interception** (nothing
to intercept — deps are declared, which is the one place this design
is deliberately less clever than tur, and the twins pin the
declarations so a stale dep list is a failing test, not a silent
wrong render); the **multi-store** machinery (one app, one store, one
KV — the split collapses); the **cycle guard** (a declared DAG is
acyclic by construction; a cycle is a declaration bug the store tests
catch); **families** (§4.6: no tur precedent, no job here — the
per-row state IS the items list; `family.rut` does not exist).

**The jotai/riverpod mapping**, honestly drawn:

| the ecosystem's shape | this store's shape |
|---|---|
| `atom(initial)` at module scope | a field on `TodoStore`, built at boot (rut has no module state) |
| a derived atom's read graph, auto-tracked | `deps()` — declared, twin-pinned |
| write → subscribers invalidate → recompute on next read | write → rail mark → `refresh()` at paint, recompute at most once per turn |
| `atomWithStorage`-style effects / `watch` | nothing — the render is the only consumer, once per turn |
| `family(param)` keyed cells | rejected — the items list already keys rows by id |

**What the store is NOT** (the honest limits): there is no reactivity
beyond the turn — no live props, no watchers, no fine-grained DOM
updates; anything outside `paint`'s refresh sees stale caches by
design. There are no families, no mutation atoms, no second store.
The `Atom<T>.set` lane marks unconditionally (no structural `==`).
And the store stays STANDALONE — `atom`/`derived`/`todos` import no
widget, no web, no t1; the store never touches the clock or the DOM
(grep-enforced by the law gate's import set).

**4. The structure: a medium-scale rut project.** This example is the
tree a REAL rut project takes — not a flat handful of files, and not
one concatenated module. The law that shapes it: **one directory is
one module with ONE entry file** (RFC 0035 §1) — so per-file
components means per-package directories — and **use paths name
PACKAGES, never directories**.

```
examples/05-todolist-web/
├── index.html  loader.js  gen/     the page shell (gitignored gen/)
├── src/                            the Rust host: state/pump, hosts,
│                                   backends (web + fake twin), mount
├── tests/                          the gates (93 tests, see Gates)
└── rut/                            THE rut project
    ├── rut.toml                    name = "app"; entry.lib = ./app/app/app.rut
    │                               [deps] = every package below — the
    │                               manifest IS the module list
    ├── web/                        the DOM crossings as a host pkg
    │                               (entry.type, host_scope = "web")
    ├── t1/                         the framework core
    │   ├── widget/                 Widget + enums + fluent builders
    │   ├── lowering/               the tag/token ladder (pure)
    │   └── core/                   T1Root + mount/registry/diff —
    │                               inline = true (its private tables
    │                               are probed same-unit by the twin)
    ├── components/                 the widget vocabulary, ONE PACKAGE
    │   ├── col/ row/ text/         PER KIND (the one-dir-one-module
    │   ├── button/ checkbox/       law): each exports its constructor
    │   ├── field/ spacer/ card/    (text also done/pending/muted;
    │   ...                         button btn + quiet)
    ├── store/                      THE ATOM STORE (standalone)
    │   ├── atom/                   Rail + Atom<T> + StrAtom (inline)
    │   ├── derived/                the Derived trait + Seen (inline)
    │   └── todos/                  Todo, Req, TodoStore: the four
    │                               atoms, counts$, the machine (inline)
    └── app/                        the biz code
        ├── app/                    AppRoot + main/on_event/dispatch/
        │                           paint/view — the project entry
        ├── todo_list/              the keyed list + pending lines
        └── todo_row/               check + title + quiet del, row-<id>
```

* **the manifest route.** The native lane mounts the DIRECTORY:
  `load_dir_session("rut")` reads `rut/rut.toml` and runs RFC 0045's
  four passes (the deps walk, the root's dev-deps, the peer gate,
  compile) FOR REAL — the manifest, not a Rust fn, is the module list.
* **both lanes, one closure.** The wasm lane keeps the `Session`
  I/O-free (wasm hosts mount in memory), so `src/mount.rs` registers
  the same packages BY HAND — one `register_module` per package, the
  same `inline`/`host_scope` values the manifests state, everything
  `include_str!`. The mirror is `rut.toml`'s mirror and nothing more;
  `tests/mount_lane.rs` (the P3 proof) pins both lanes to the same
  mounted-name set, host surface, and compiled binary. LINKED
  (concrete exports): `widget`, `lowering`, the eight components, the
  two view builders. INLINE (spliced, per-package, deduped — a
  first-class manifest key with a stated shape reason, NOT the retired
  appkit hack): `t1`, `atom`, `derived`, `todos` (generic exports
  splice by law; the t1 core's private tables are probed same-unit).
* **the import discipline** (acyclic, minimal, law-enforced —
  `tests/app_law.rs`): **components import `widget` and nothing else
  of t1** (the lowering and the core stay framework-private — the biz
  law holds at the component layer too); **the store is STANDALONE**
  (`atom`/`derived`/`todos` name no widget, no web, no t1); **app ->
  components + store** (+ `t1`'s four names + `web`'s ONE crossing
  `tim_after`), and the gate pins the app's import set as a SET
  EQUALITY — `atom`/`derived` are not importable by biz, which is how
  "store standalone" is enforced. Collisions are avoided by naming
  (the component `row` exists, so the view packages are `todo_list`/
  `todo_row` — first-mount-wins would silently skip a duplicate).
* **what died for this shape**: the `appkit` inline-splice module —
  ONE module fabricated from the store + t1 sources because the
  pre-dedup graph spliced each use's transitive sources per use. The
  retirement is PROVEN, not asserted: P1 — the packages mount
  separately and everything compiles (every suite here); P2 — the law
  gate asserts the name survives nowhere; P3 — the lanes compile to
  byte-identical binaries. (P3 caught a real embedder gap — a bare
  manifest session lacked the core prelude — fixed embedder-side,
  zero engine changes; `docs/todolist-restructure-report.md` tells it.)

**5. The host-boundary thinness law** (t1 survey §4.5). The crossings
carry elements (as opaque handles, RFC 0014/0023), strings, i64s and
bools — nothing else. No app names, no data shipping, no callbacks
across the boundary, no scheduler in the host. t1 is the law's proof
of strength: an entire widget system — diff, registry, lowering —
fits over the ten unchanged crossings, and the atom store needs NONE
of them (it is DOM-free by import law). The host never grew an
`ui_create_todo` or an `ui_unlisten`.

**6. Idiomatic current rut.** Dataclass-literal structs (`AppRoot`,
`Todo`, `Req`, `Widget`), shared-cell containers (`let mut r: ?AppRoot
= root`), `?T` nilables, `when` match, f-strings, the val-column row
`HashMap<str, i64>` (nmapset) as the subject tables,
`opaque.downcast` at the trust boundary (RFC 0014), `entry fn` as the
host-callable surface, and the
manifest-mounted package tree — the medium-scale shape above.

## The two state machines, one container

`AppRoot` carries BOTH machines as fields (RFC 0003 §1, restructure
survey §4.5): the framework's `T1Root` (the patch state) and THE ATOM
STORE `TodoStore`. The draft and the note live IN the store as atoms
(`draft$`, `note$` — they were always state); the subject tables
(`toggles`/`removes`) stay plain maps rebuilt per render — they are
the dispatch mirror, not state. A turn, end to end:

```
event row -> on_event -> dispatch -> ATOM WRITES (set = store + mark)
          -> paint: store.refresh()   (the flush: deriveds recompute
                                      at most once, in declaration
                                      order, only if a dep's gen moved)
          -> view(r)                  (PURE reads: counts$ through the
                                      derivation, draft$/note$ atoms)
          -> t1_render                (the ONE keyed diff per turn)
```

## Layout

| path | what |
|---|---|
| `rut/rut.toml` | the project manifest — the module list both lanes mirror |
| `rut/web/` | the declared surface (`web.d.rut`, RFC 0025, `host_scope = "web"`) — the contract, verified both ways at boot |
| `rut/t1/widget/` | the `Widget` type, its kind/variant enums, the fluent builders (inherent impls live in the type's module), the predicates |
| `rut/t1/lowering/` | `Lowered` + `lower()` — the tag/token/ladder tables, pure `Widget -> Lowered` |
| `rut/t1/core/` | the framework state machine — `T1Root`, `t1_mount`, `t1_render`, `t1_subject`, the keyed diff, retirement; design: `docs/t1-design.md` |
| `rut/components/*/` | the widget vocabulary — one package per kind, importing `widget` only |
| `rut/store/atom/` | the cell layer: the generation rail, `Atom<T>`, the `StrAtom` lane |
| `rut/store/derived/` | the derived layer: the `Derived` trait (the declared DAG) + `Seen` dep-generations |
| `rut/store/todos/` | the "server": the four atoms, `counts$`, the booking/answer machine, the DOM-free probe surface |
| `rut/app/app/` | the page program: `AppRoot`, `main`/`on_event`/`dispatch`/`paint`/`view` |
| `rut/app/todo_list/`, `rut/app/todo_row/` | the two view builders |
| `src/state.rs` | the turn law: the FIFO queue, the one pump, the re-entrancy guard (`events are queue, never stack`) |
| `src/hosts.rs` | the 10 registry bindings, written ONCE generic over the backend |
| `src/backend.rs` | the `DomBackend` seam both lanes implement |
| `src/web_dom.rs` | wasm32: web_sys bodies + the raw `rut_web_alloc/boot/pump/last_error` ABI |
| `src/fake_dom.rs` | the host twin: a HashMap element tree with the same semantics (appendChild moves) + a virtual timer clock |
| `src/host.rs` | the native page owner (boot/fire/advance/pump) |
| `src/mount.rs` | the session mount — TWO LANES over one closure: `load_dir_session` on `rut/` (native) and the per-package `register_module` mirror (wasm) |
| `index.html` / `loader.js` | the page shell: a static `#app` root + ~50 lines of JS that fetch `./rut/app/app/app.rut` and hand over the source; the stylesheet keys ONLY lowered tokens |
| `gen/` | wasm-bindgen's generated browser glue (gitignored) — produced by the build recipe below, never committed |
| `tests/store.rs` | the store's laws, DOM-free (17: the 9 machine laws + the 8 atom twins) |
| `tests/todolist_app.rs` | the twin gate: 18 scripted sessions asserting the tree AND the turn order on the lowered DOM |
| `tests/t1_lowering.rs` / `tests/t1_diff.rs` | the framework's bed: 14 lowering snapshots + 18 diff/lifecycle/trap tests (32) |
| `tests/app_law.rs` | **the grep gate**: reads `rut/app/*.rut` and fails loud if biz ever sinks to the DOM's vocabulary (3 tests) |
| `tests/host_surface.rs` | the crossing + trap matrix (22 tests) |
| `tests/mount_lane.rs` | the P3 proof: manifest lane == mirror lane, byte-identical (1 test) |
| `tests/e2e-browser.mjs` | the through-the-artifact gate (see Gates) — plain node tier + real-browser tier |

## The surface (`rut/web/web.d.rut`)

`ui_get` · `ui_create` · `ui_set_text` · `ui_attr` · `ui_append` ·
`ui_remove` · `ui_clear` · `ui_set_input_value` · `ui_listen` ·
`tim_after`

Trap shapes (all tested in `tests/host_surface.rs`): loud unknown id
(`web::ui_get: no element '#x'` — a wiring bug is loud, never a rut
optional); kind mismatch naming both sides (`boundary: got \`ul\`
where \`HtmlInputElement\` binds`); DOM exceptions carried verbatim;
the re-entrancy guard; the RFC 0025 boot panics both ways; stale
listener ids as host drift.

**Recorded deviations from the original survey's 11 rows** (not
re-litigated, recorded per the batch law — see
`docs/todolist-web-report.md`, and `docs/t1-design.md` §8 for the
framework's five):

* `ui_input_value(el) -> str` is absent — the engine's verified-return
  table cannot bind a `-> str` host fn today. Input values are
  **event-carried** instead: every event row on an input carries its
  current value as `detail` (thinner anyway — the thinness law's own
  spirit). The fix is an engine change (the MENU).
* event rows carry no key — the original survey's `keydown`/Enter
  shape has no carrier; the app gates the empty draft client-side and
  uses the button's click.
* `on_event` grew a **leading container param**: rut has no mutable
  module state, so the boot turn's returned opaque crosses back on
  every turn.
* `on_event` grew the **entry-err return**: the store's `answer` lines
  split into the `(line, err)` pair — a commit is `(line, "")`, a
  rejection or a lost id is `("", why)` — and the app surfaces the why
  through `(?opaque, str)`'s err channel instead of pretending every
  answer succeeded.
* the dispatch table is the val-column row `HashMap<str, i64>` —
  nmapset's keys are the SUBJECT (the semantic event name), never the
  host's listener id.

## Gates

The test story has three lanes, all wired to run from one command:

```sh
cargo test -p todolist-web      # the twin + the law (93 tests)
node tests/e2e-browser.mjs      # the artifact (from examples/05-todolist-web)
```

**Lane 1 — the twin (the real gate, always on).** `cargo test -p
todolist-web`, 93 tests over the SAME sources the browser runs (the
manifest session's subgraphs and the app session mount `rut/` for
real):

* **the framework's bed (32)** — 14 lowering snapshots pinning every
  kind and variant to its exact tag/token/attr tree (the design §4.1
  table as a test), and 18 diff/lifecycle/trap tests through the
  twin's handles: an unchanged render mints zero nodes, a toggle
  patches one token, a keyed reorder preserves element identity and
  listener ids, retirement tracks live ids only, stale fire = the
  loud trap with the page surviving.
* **the app sessions (18)** — scripted twin sessions asserting the
  tree, the counts line EXACT every turn (now THROUGH the `counts$`
  derivation), deadline-then-sequence commits (deadline order ACROSS
  kinds on the virtual clock), the client gate, duplicate rejects,
  lost ids, the stale-listener trap re-pointed at the registry, the
  re-entrancy guard, the entry-err soft-fail containment.
* **the store (17)** — the machine's laws DOM-free (a request never
  touches the list; a rejected title is an answer; latencies differ
  per kind) plus the 8 ATOM TWINS: set/get with no rail traffic on
  reads; a write marks its dirty set once; the counts line derived and
  fresh within a turn; staleness is per DECLARED dep (write `draft` →
  `counts$` does not recompute); nothing leaks across a turn boundary;
  deriveds recompute upstream first in one pass; equality-skip is the
  str lane's scope; the status line is `counts$` + `note$` verbatim.
* **the grep gate (3)** — `tests/app_law.rs` reads `rut/app/*.rut`
  and fails LOUD: the import set is exactly `{web, todos, t1, widget,
  todo_list, todo_row, col, row, text, button, checkbox, field, card}`
  with per-package NAME pins (`use web::{tim_after}` is the entire
  crossing surface); no `Node`, no `ui_`, no class writes, no tag
  literals or DOM method names (whole file, comments included, no
  whitelist); and the positive halves — `t1_render(`, `t1_subject(`,
  `.subject(`, `.key(`, and `store.refresh()` in `paint` before
  `view`. The component layer's own import scoping is asserted too.
* **the crossing matrix (22) + the lane pin (1)** — the trap shapes,
  and `mount_lane.rs`: both lanes, same mounted-name set, same host
  surface, BYTE-IDENTICAL binaries.

**Lane 2 — the node tier (always once the artifact exists).**
`tests/e2e-browser.mjs` tier 1: builds nothing silently and skips
nothing silently — it builds the wasm if missing (loud-fail with the
exact command if it cannot), runs the real wasm-bindgen CLI
(version-matched to Cargo.lock), then drives the REAL artifact + REAL
glue through the loader's own ABI sequence (import glue → `default()`
→ `initSync()` → `alloc` → write → `boot` → `last_error`) on a
~60-line fake DOM: the full scripted session, headless, no browser
needed. Missing artifact or CLI → exit 1 with the exact build
command.

**Lane 3 — the browser tier (when a browser + driver exist).**
Tier 2 of the same script: the glue generated into `gen/` exactly as
the manual recipe says, the dir served over http, Firefox headless via
raw WebDriver HTTP (no npm deps), the same session typed and clicked
on the live DOM. `gen/` is gitignored, so nothing is cleaned up
afterwards — the tree stays at HEAD-clean and the page keeps working
locally. Missing binaries = loud skip with the manual recipe (exit
0); a failed check = exit 1. Both tiers share one `runSession`, so
the browser never drifts from the twin's story; only the selector
adapters know the DOM shape (committed rows are `#list .t1-row` with
child order 0=mark 1=title 2=del, a check's state is the
`t1-check--on/--off` token, a pending line is `span.t1-text--pending`).

## Running in a browser (manual)

```sh
cd examples/05-todolist-web
cargo build -p todolist-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir gen --out-name web_host \
  ../../target/wasm32-unknown-unknown/release/todolist_web.wasm
python3 -m http.server        # then open http://localhost:8000/
```

`--out-name web_host` is load-bearing: `loader.js` imports
`./gen/web_host.js`, and the generated glue fetches
`./web_host_bg.wasm` relative to ITSELF, so the subdir pair stays
consistent. The CLI version must match the crate's wasm-bindgen (see
Cargo.lock; wasm-pack's fetched install works). `gen/` is gitignored,
so the build never dirties the tree — no cleanup chore, and a stale
glue just gets overwritten by the next build.

**What you'll see**: an indigo Add button beside a focused field under
"rut — the `web::` host surface". Type a title — the status line
echoes the typing (`... — typing 'milk'`), and the counts never move
(the draft$ atom is not a `counts$` dep). Press Add — an italic
`... milk` pending line paints immediately, the counts line flips to
`1 in flight` (the reqs$ write through the derivation), and the field
clears; ~400 ms later the committed row replaces it — a checkbox, the
title, a quiet `del`, and `1 open | 0 done | 0 in flight`. Add a
second while the first flies — both pend, both commit in book order.
Toggle a row — its title goes italic for ~250 ms (the variant patch),
then the check fills indigo, the glyph slides in, and the title
strikes through (the `--on` + `--done` tokens; the flip animates
because the keyed diff kept the node alive). Remove a row — gone in
~120 ms, order preserved. Add with an empty field — `type a title
first`, no request booked. Duplicate titles commit as a REJECT answer:
`rejected 'milk' — already on the list`. Every one of those status
lines is the DERIVED atom recomputing once per turn.

Without the glue, `loader.js` fails loud with exactly the commands
above. `node tests/e2e-browser.mjs` is the automated gate over the
same steps.
