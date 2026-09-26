# 05-todolist-web — a web page whose brain is rut

One page app — a todolist with a simulated server — where **every DOM
move and every timer goes through ten declared host crossings**, and
the page's brain is a **rut project of TWO packages** (`rut/`): `ui`
(the framework — the atom store machinery, the t1 widget framework,
the component vocabulary, one module) and `biz` (the domain-as-
mutations and the app shell, one module), with the `web` crossing
declared flat at the example root (`web.d.rut`). Two `rut.toml` total.
Pure rut: no `main` loop on the rut side, no event loop, no async
keywords. The host (web-sys on wasm32, a fake-DOM twin on native) owns
the loop; rut owns the state.

The page's UI is not hand-built DOM: the app composes **widgets**, and
the framework's keyed diff lowers them to those same ten crossings.
Biz speaks widgets, handles and mutations; tags, class tokens,
listener ids and the patch loop are the framework's private business —
literally: the machinery traits are module-private to `ui`, and biz
cannot name them even to import them.

The `web` crossing is **example-local** by the phase-0 survey's
decision (`docs/todolist-web-survey.md` §4): `web.d.rut` sits FLAT at
the example root, outside any package directory (its pre-restructure
placement, kept — the two-package law counts manifests, and the host
crossing is not a rut package). Both lanes register it by hand
(`src/mount.rs`), the `rt` precedent.

## What it teaches

**1. The async pattern: a hand-written machine.** rut today has no
async/await (the engine stops at the M3 wall — survey §2.2), but a web
page is nothing *but* async. The pattern that fills the gap:

* **state lives in ONE container the host holds.** rut has no mutable
  module state (RFC 0003 §1 — module lets are load-time literals), so
  `main` builds `AppRoot{world, t1, toggles, removes}` and RETURNS it
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
  touches the list: the machine's `add_m` mutation books a `Req` row
  (tag, kind, latency) and the UI books one
  `tim_after(req.latency, req.tag)` — the host's clock, RFC 0018. The
  list changes only when the timer's turn runs the `answer_m` mutation
  — the one commit point, per-kind latency (add 400 / toggle 250 /
  remove 120 ms) so deadline-ordered commits are *visible across
  kinds*. A rejected title and a lost id are ANSWERS (data the turn
  reads — the `(line, err)` pair), not traps; an answer naming no
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
  cell.
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

**3. The two-package store: jotai's shape over parameterized trait
impls** (the restructure survey's addendum — authoritative; it
records, decision by decision, the reversal of the declared-DAG /
flush design). The store is the `ui` package's kernel, decoupled from
biz by construction — nothing in it names a todo, a list, or an app
(RFC 0014's motivating pattern: the reactivity library over opaque
cells). Biz owns HANDLES; the machinery is ui-private. The whole
surface is two verbs and four minting calls:

```rut
// the ui package's kernel — everything else about it is private
store.source<T>(v)                     -> Source<T>      the general cell
store.source_str(v)                    -> Source<str>    the str lane
store.derive<T>(fn (ctx) -> T)         -> Derived<T>     deps DISCOVERED
store.mutation<A, R>(fn (ctx, a) -> R) -> Mutation<A, R> the write program
store.get(a)                           -> T              the one READ
store.set(w, arg)                      -> R              the one WRITE
store.recompute_count() / store.deps_of(a)               observability
```

* **handles are keys, not objects.** `Source<T>` carries its id and
  seed; `Derived<T>` carries its id; `Mutation<A, R>` carries its id.
  The handles have NO methods — every operation goes through the
  store, so the store can see every read and every write.
* **the machinery traits are module-private** — `Readable<T>` and
  `Writable<A, R>` live unexported in `ui`, and `store.get`/`store.set`
  take TRAIT-TYPED parameters (`a: Readable<T>`, `w: Writable<A, R>`);
  impl-trait methods ride the trait's visibility (RFC 0012), the
  parameterized trait impls (`impl Readable<T> for Source<T>`,
  `impl Writable<A, R> for Mutation<A, R>`) are the dispatch. Biz
  cannot name the traits even to import them — which is how
  `Derived` being unwritable is enforced AT THE TYPE LEVEL (no
  `Writable` impl for `Derived<T>`), not by a runtime check first.
* **freshness is PULL-ON-READ.** `get` recomputes a stale derived at
  most once per write-set: the rail is a generation map, a write bumps
  the written atom, a derived's recompute bumps its own gen (what
  downstream staleness keys on), and a get serves the cache until a
  recorded dep gen moves past the reader's `seen`. There is NO flush,
  no dirty set, no refresh — the survey's turn-boundary refresh is
  retired; `paint` has no flush line, and the view's first get
  computes what this turn's writes staled.
* **deps are DISCOVERED, never declared.** A derive is a first-class
  fn — `store.derive(fn (ctx) -> str { ... })` — and the `ctx.get`
  calls inside it ARE the dependency declaration, recorded as a side
  effect of answering each read, re-discovered fresh on every
  recompute. A stale declaration is impossible by construction;
  `store.deps_of(a)` reads the graph the store actually walked. The
  domain has exactly one derived: `counts$`, `"N open | M done |
  K in flight"` — the counts line law is proven THROUGH the
  derivation. Discovered edges admit cycles, so the cycle guard (an
  in-flight set) is back: a self-reading derive panics loud.
* **the ctx asymmetry is the purity law.** A derive's ctx exposes
  `get` ONLY — a derive is pure by construction. A mutation's ctx
  exposes `get` + `set` — every multi-step write is a mutation fn,
  and all writes land in the ONE write lane inside the store.
* **the str lane equality-skips; the general lane never does.** An
  equal-content write on a `source_str` cell marks nothing (`==`
  compiles StrCmp at this concrete lane); a generic `==` would be
  cell identity for `Vec`, so `Source<T>.set` bumps unconditionally —
  the recorded asymmetry (RFC 0044), pinned by twins on both sides.
* **the machine as mutations.** The todolist's verbs — `add_m`,
  `toggle_m`, `remove_m`, `answer_m` — are mutation fns: the multi-
  step writes (book a req, consume it, commit per kind) are VALUES biz
  composes with `store.set`. The answers stay DATA (the `(line, err)`
  pair); the timers stay OUT (mutations never touch the clock — the
  app books `tim_after` from the returned `Req`). The machine's
  counters (`next_todo$`, `next_req$`) are sources too: state lives
  in the store, and without a domain container there is nowhere else
  for state to live.
* **there is no domain container.** `World` is a bundle of HANDLES —
  the sources, the derived, the mutations — plus the store itself; it
  owns no rail, no flush, no recompute pass. The old `TodoStore`/
  `ProbeStore` domain-typed containers are gone (their machinery was
  the kernel all along; their handles are biz data now), and the law
  gate holds the tombstones: `TodoStore`, `Rail`, `Seen`, `Atom<`,
  `.refresh(` and friends must survive nowhere in biz.

**What came from tur** (survey §1.2 + the addendum): pull-on-read
freshness (tur's model, the flush point dissolved entirely); the
generation rail (a cached derived is servable iff its recorded
generations still match); DISCOVERED deps (tur's tracker-stack shape —
the read records the edge); the in-flight cycle guard (returned:
discovered edges admit cycles); mutation atoms with the `{get,set}`
ctx (returned as the MutCtx — its rejection predates first-class fns);
equality-skip on the `str` lane (type-directed `==`; the general lane
never skips); cells a store MATERIALIZES, never globals.

**What stayed rejected** (survey §1.3, still): **live-props /
SubscriberGraph** per-element re-rendering (the `Val<T>` model,
rejected twice on purpose — the keyed diff does that job once per
turn); **`watch()`** and the watch-loop guard (reactive push callbacks
are reactivity, not state cells; the app's async facts are the host's
timer tags); **the multi-store** machinery (one app, one `Store`, one
KV — the split collapses); **families** (§4.6: no tur precedent, no
job here — the per-row state IS the items list).

**The jotai/tur mapping**, honestly drawn:

| the ecosystem's shape | this store's shape |
|---|---|
| `atom(initial)` at module scope | `store.source(v)` — a handle biz holds in `World` (rut has no module state) |
| a derived atom's read graph, auto-tracked | the derive closure's `ctx.get` calls — discovered fresh every recompute |
| write → subscribers invalidate → recompute on next read | write bumps the gen; `store.get` recomputes at most once per write-set |
| an action / write fn | `store.mutation(fn (ctx, a) -> R)` — the machine's verbs |
| `atomWithStorage`-style effects / `watch` | nothing — the render is the only consumer, once per turn |
| `family(param)` keyed cells | rejected — the items list already keys rows by id |

**What the store is NOT** (the honest limits): there is no reactivity
beyond the read — no live props, no watchers, no fine-grained DOM
updates; a value is recomputed only when someone `get`s it. There are
no families, no second store, no subscriptions. The general lane
never equality-skips. And the kernel stays DECOUPLED — the store
imports no biz name; biz reaches it only through the two verbs and
its own handles (grep-enforced by the law gate's import set).

**4. The structure: the two-package law.** A real rut project is not
a flat handful of files — but sixteen per-concept manifests turned out
to be the wrong extreme too: rut's landed visibility is `pub` or
module-private, so separate packages could not keep framework tables
private from each other, and the layering lived only in the law gate.
The redesign merges the tree into TWO packages, one directory, one
module, one entry file each (RFC 0035 §1) — the framework's private
tables are private FOR REAL inside `ui`'s one module, and the law
that remains is enforced by type visibility, not just by grep:

```
examples/05-todolist-web/
├── index.html  loader.js  gen/     the page shell (gitignored gen/)
├── web.d.rut                       the `web` host DECL surface — flat at
│                                   the root, no manifest of its own: the
│                                   embedder registers it in both lanes
├── src/                            the Rust host: state/pump, hosts,
│                                   backends (web + fake twin), mount
├── tests/                          the gates (94 tests, see Gates)
└── rut/                            TWO packages, TWO manifests
    ├── ui/                         THE FRAMEWORK package
    │   ├── rut.toml                name = "ui"; inline = true (generic
    │   │                           exports splice by law); [deps] pouch
    │   │                           + nmapset; entry.libs = the five
    │   │                           section files (RFC 0041 §5)
    │   ├── ui.rut                  the base: the law header + the use
    │   │                           set — then the libs splice in array
    │   │                           order, ONE module
    │   ├── store.rut               §1 the atom store kernel (Store,
    │   │                           Source/Derived/Mutation, the PRIVATE
    │   │                           Readable/Writable traits)
    │   ├── widget.rut              §2 the Widget type + builders
    │   ├── lowering.rut            §3 the lowering table (pure)
    │   ├── diff.rut                §4 T1Root + the keyed diff
    │   └── components.rut          §5 the component vocabulary
    └── biz/                        THE DOMAIN package AND the project
        │                           root (the ABI spec stays `app`)
        ├── rut.toml                name = "app"; [deps] ui + pouch +
        │                           nmapset; entry.libs = the four
        │                           section files
        ├── biz.rut                 the base: the law header + the use
        │                           set — ONE module once spliced
        ├── domain.rut              Todo/Req + the machine's pure scans
        ├── world.rut               World (the handles bundle) + boot
        ├── entries.rut             the DOM-free probe surface
        └── app.rut                 the app shell + the two view
                                    builders
```

* **one module, several files** (RFC 0041 §5, the multi-lib entry):
  each package's body is authored as section files named by
  `entry.libs` — the loader splices base-first, then the array order,
  `'\n'`-joined, into ONE module. One namespace, one visibility scope:
  a name private to `store.rut` is visible to `components.rut` and to
  nothing outside `ui` — exactly the single-file privacy law, kept by
  the splice. The manifest's array IS the canonical order (same
  manifest ⇒ same module); the mirror (`src/mount.rs`) and the wasm
  lane (`loader.js`'s `BIZ_LIBS`) spell the same list by hand, and
  `tests/mount_lane.rs` pins manifest-lane and mirror-lane binaries
  byte-identical through the splice.

* **the manifest route.** The native lane mounts the DIRECTORY:
  `load_dir_session("rut/biz")` reads `rut/biz/rut.toml` and runs
  RFC 0045's four passes (the deps walk — `ui`, and through it
  pouch/nmapset/nmap_host — the root's dev-deps, the peer gate,
  compile) FOR REAL — the manifest, not a Rust fn, is the module list.
* **both lanes, one closure.** The wasm lane keeps the `Session`
  I/O-free (wasm hosts mount in memory), so `src/mount.rs` registers
  the same packages BY HAND — one `register_module` per package, the
  same `inline`/`host_scope` values the manifests state, everything
  `include_str!`. The mirror is `rut/biz/rut.toml`'s mirror and
  nothing more; `tests/mount_lane.rs` (the P3 proof) pins both lanes
  to the same mounted-name set, host surface, and compiled binary.
  `web.d.rut` rides NO manifest path — both lanes register it by hand
  (the embedder mounts what the closure uses; the `mount_std_core`
  precedent). INLINE (spliced, deduped by origin — a first-class
  manifest key with a stated shape reason): `ui` (its generic exports
  splice by law; the framework's private tables are probed same-unit
  by the twins).
* **the import discipline** (acyclic, minimal, law-enforced —
  `tests/app_law.rs`): **biz imports `ui` and nothing else of the
  framework's guts** — exactly the store's public names, the widget
  type + mount/render/subject, and the component constructors; the
  machinery traits (`Readable`/`Writable`) are ui-private, so biz
  CANNOT name them even to import them, and the gate pins biz's
  import set as a SET EQUALITY. The old per-layer packages (`store/`,
  `t1/`, `components/`, `app/`) are merged, not gone: each is a
  SECTION of its package, and the app_law gate still holds the layer
  laws (components' vocabulary unchanged, the store standalone of
  biz, biz DOM-free).

**5. The host-boundary thinness law** (t1 survey §4.5). The crossings
carry elements (as opaque handles, RFC 0014/0023), strings, i64s and
bools — nothing else. No app names, no data shipping, no callbacks
across the boundary, no scheduler in the host. t1 is the law's proof
of strength: an entire widget system — diff, registry, lowering —
fits over the ten unchanged crossings, and the store kernel needs
NONE of them (its verbs never cross; the ui-side crossing users are
the mount and the patch, and biz itself spells exactly ONE crossing
name, `tim_after`). The host never grew an `ui_create_todo` or an
`ui_unlisten`.

**6. Idiomatic current rut.** Parameterized trait impls with
trait-typed fn params (`impl Readable<T> for Source<T>`;
`fn get<T>(self, a: Readable<T>)` — the store's whole trick),
module-private traits behind `pub` methods (RFC 0012's impl-trait
visibility), first-class fns as values (`store.derive` / `store.mutation`
take fn literals; `opaque.downcast` unwraps the erased program at the
trust boundary, RFC 0014), dataclass-literal structs (`World`,
`AppRoot`, `Todo`, `Req`, `Widget`), shared-cell containers (the
container crossing is NON-NULLABLE end to end: `opaque(AppRoot)` —
`downcast<AppRoot>` — nil-checked unwrap `let mut r: AppRoot = root`),
`?T` nilables, `when` match, f-strings, the
val-column row `HashMap<str, i64>` (nmapset) as the subject tables,
and the two-package manifest tree — the medium-scale shape above.

## The two state machines, one container

`AppRoot` carries BOTH machines as fields (RFC 0003 §1): the
framework's `T1Root` (the patch state) and THE WORLD (the handles
bundle — the store, the atoms, the derived, the mutations). The draft
and the note live IN the store as sources (`draft$`, `note$` — they
were always state); the subject tables (`toggles`/`removes`) stay
plain maps rebuilt per render — they are the dispatch mirror, not
state. A turn, end to end:

```
event row -> on_event -> dispatch -> STORE WRITES (store.set: plain
                                      sources, or the machine's
                                      mutations)
           -> view(r)                (PURE reads: the first get
                                      computes what this turn's
                                      writes staled — pull-on-read,
                                      at most once per write-set)
           -> t1_render              (the ONE keyed diff per turn)
```

There is no flush step — the old paint's `store.refresh()` line is
retired; freshness is the read's job now.

## Layout

| path | what |
|---|---|
| `rut/biz/rut.toml` | the root manifest — name `app`, deps `ui`/pouch/nmapset; the module list the native lane walks |
| `rut/ui/rut.toml` | the framework manifest — name `ui`, `inline = true`, deps pouch/nmapset |
| `web.d.rut` | the `web` crossing's DECL surface (RFC 0025, `host_scope = "web"`) — flat at the example root, registered by hand in both lanes, verified both ways at boot |
| `rut/ui/*.rut` | the framework, one module, five files (`entry.libs`, RFC 0041 §5): `ui.rut` the base (law header + use set); `store.rut` §1 the store kernel (the private `Readable`/`Writable` traits, the handles, `Store`); `widget.rut` §2 the `Widget` type + fluent builders; `lowering.rut` §3 the lowering table; `diff.rut` §4 `T1Root` + the keyed diff (design: `docs/t1-design.md`); `components.rut` §5 the component vocabulary |
| `rut/biz/*.rut` | the domain + app, one module, four files: `biz.rut` the base (law header + use set); `domain.rut` `Todo`/`Req` + the pure scans; `world.rut` `World` + boot; `entries.rut` the machine mutations' DOM-free probe entries; `app.rut` `AppRoot`/`main`/`on_event`/`dispatch`/`paint`/`view`, the list + row builders |
| `src/state.rs` | the turn law: the FIFO queue, the one pump, the re-entrancy guard (`events are queue, never stack`) |
| `src/hosts.rs` | the 10 registry bindings, written ONCE generic over the backend |
| `src/backend.rs` | the `DomBackend` seam both lanes implement |
| `src/web_dom.rs` | wasm32: web_sys bodies + the raw `rut_web_alloc/boot/pump/last_error` ABI |
| `src/fake_dom.rs` | the host twin: a HashMap element tree with the same semantics (appendChild moves) + a virtual timer clock |
| `src/host.rs` | the native page owner (boot/fire/advance/pump) |
| `src/mount.rs` | the session mount — TWO LANES over one closure: `load_dir_session` on `rut/biz` (native) and the per-package `register_module` mirror (wasm); both register `web.d.rut` by hand |
| `index.html` / `loader.js` | the page shell: a static `#app` root + ~50 lines of JS that fetch the biz module's files (`BIZ_LIBS`, the manifest's mirror) and hand over the spliced source; the stylesheet keys ONLY lowered tokens |
| `gen/` | wasm-bindgen's generated browser glue (gitignored) — produced by the build recipe below, never committed |
| `tests/store.rs` | the store's laws, DOM-free (17: the 9 machine laws + the 8 atom twins) |
| `tests/todolist_app.rs` | the twin gate: 18 scripted sessions asserting the tree AND the turn order on the lowered DOM |
| `tests/t1_lowering.rs` / `tests/t1_diff.rs` | the framework's bed: 14 lowering snapshots + 18 diff/lifecycle/trap tests (32) |
| `tests/app_law.rs` | **the grep gate**: reads the biz module (base + libs, spliced) and fails loud if biz ever sinks to the DOM's vocabulary or names a retired store API (4 tests) |
| `tests/host_surface.rs` | the crossing + trap matrix (22 tests) |
| `tests/mount_lane.rs` | the P3 proof: manifest lane == mirror lane, byte-identical (1 test) |
| `tests/e2e-browser.mjs` | the through-the-artifact gate (see Gates) — plain node tier + real-browser tier |

## The surface (`web.d.rut`, at the example root)

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
* `on_event` grew the **entry-err return**: the machine's `answer`
  lines split into the `(line, err)` pair — a commit is `(line, "")`,
  a rejection or a lost id is `("", why)` — and the app surfaces the
  why through `(?opaque, str)`'s err channel instead of pretending
  every answer succeeded.
* the dispatch table is the val-column row `HashMap<str, i64>` —
  nmapset's keys are the SUBJECT (the semantic event name), never the
  host's listener id.

## Gates

The test story has three lanes, all wired to run from one command:

```sh
cargo test -p todolist-web      # the twin + the law (94 tests)
node tests/e2e-browser.mjs      # the artifact (from examples/05-todolist-web)
```

**Lane 1 — the twin (the real gate, always on).** `cargo test -p
todolist-web`, 94 tests over the SAME sources the browser runs (the
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
  tree, the counts line EXACT every turn (through the `counts$`
  derivation's pull-on-read), deadline-then-sequence commits
  (deadline order ACROSS kinds on the virtual clock), the client
  gate, duplicate rejects, lost ids, the stale-listener trap
  re-pointed at the registry, the re-entrancy guard, the entry-err
  soft-fail containment.
* **the store (17)** — the machine's laws DOM-free (a request never
  touches the list; a rejected title is an answer; latencies differ
  per kind) plus the 8 ATOM TWINS, third shape: pull-on-read
  freshness (a get after a write-set recomputes exactly once; two
  writes before one get still cost one recompute; NO flush exists);
  staleness is per DISCOVERED dep (write `draft$` → `counts$` does
  not recompute; `atom_deps` pins counts$'s edges to exactly its
  reads); the str lane's content-skip marks nothing and the vec lane
  never skips; the counts line is derived and the status line reads
  it; deriveds recompute upstream first and feed downstream FRESH;
  an upstream-only recompute still refreshes downstream; a discovered
  cycle dies loud.
* **the grep gate (4)** — `tests/app_law.rs` reads the biz module
  (base + libs, spliced the manifest's way)
  and fails LOUD: the import set is exactly the two-package
  vocabulary (the store's public names + the widget type +
  `t1_mount`/`t1_render`/`t1_subject` + the component constructors +
  `web`'s ONE crossing `tim_after`) with per-name pins; no `Node`, no
  `ui_`, no class writes, no tag literals or DOM method names (whole
  file, comments included, no whitelist); the widgets are the whole
  UI story and PULL is the freshness (one `t1_render(`, no `refresh`
  anywhere in biz, the view reads and never writes); and the
  machinery tombstones — `TodoStore`, `Rail`, `Seen`, `Atom<`,
  `.refresh(` and friends survive nowhere in biz.
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

Demo-day shortcut — the same serve plus a **Cloudflare QUICK tunnel**
for the room (`node scripts/dev-channel.mjs`, zero npm deps):

```sh
node scripts/dev-channel.mjs  # preflights (or loudly builds) the gen/ glue,
                              # serves this dir on :8000, and prints a
                              # throwaway https://<random>.trycloudflare.com
                              # URL (needs `cloudflared` on PATH; Ctrl+C
                              # tears down server + tunnel together)
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
(the draft$ cell is not a counts$ dep). Press Add — an italic
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
lines is the derived counts recomputing once per write-set, on read.

Without the glue, `loader.js` fails loud with exactly the commands
above. `node tests/e2e-browser.mjs` is the automated gate over the
same steps.
