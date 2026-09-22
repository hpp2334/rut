# 05-todolist-web — a web page whose brain is rut

One page app — a todolist with a simulated server — where **every DOM
move and every timer goes through ten declared host crossings**, and
the page's brain (`todolist.rut` + `store.rut`) is pure rut with no
`main`, no event loop, and no async keywords. The host (web-sys on
wasm32, a fake-DOM twin on native) owns the loop; rut owns the state.

The page's UI is not hand-built DOM: the app composes **a toy widget
framework** (`t1.rut`, ~700 lines of rut) whose keyed diff lowers
widgets to those same ten crossings. Biz speaks widgets, subjects and
hooks; tags, class tokens, listener ids and the patch loop are the
framework's private business.

The example is **example-local** by the phase-0 survey's decision
(`docs/todolist-web-survey.md` §4): the `web` pkg lives in this crate,
not in the std workspace.

## What it teaches

**1. The async pattern: a hand-written machine.** rut today has no
async/await (the engine stops at the M3 wall — survey §2.2), but a web
page is nothing *but* async. The pattern that fills the gap:

* **state lives in ONE container the host holds.** rut has no mutable
  module state (RFC 0003 §1 — module lets are load-time literals), so
  `main` builds `AppRoot{store, t1, draft, note, …}` and RETURNS it as
  an opaque; the pump hands it back on every turn:

  ```rut
  entry fn main() -> opaque;
  entry fn on_event(c: opaque, kind: i32, subject: str, detail: str);
  ```

* **every asynchronous fact is an event row.** One entry, one shape —
  kind 1 = EV_DOM, kind 2 = EV_TIMER; the host is the only writer of
  the queue (a FIFO with a re-entrancy guard — `events are queue,
  never stack`).
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
`tim_after` becomes a timer primitive — survey §5.6 spells the
migration; the app's turn law and store semantics do not change.

**2. The widget framework: biz never sinks to the DOM.** The second
lesson is `t1.rut` (design: `docs/t1-design.md`, the batch's phase 0
— authoritative, including its recorded deviations). A UI is a VALUE:

* **widgets are plain rut data** — one fat `Widget` struct with
  payloadless kind/variant enums (the json ruling's shape, RFC 0006)
  and recursive `Vec<Widget>` children. `view(r)` is a PURE function
  from app state to a `Widget` tree; it runs no crossing.
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
  ONLY the lowered tokens (the `t1-*` families, four rungs of fixed
  utility ladder, variant tokens); the app spells no class and the
  stylesheet names no widget field. The §4.1 table IS the styling
  contract, and the twin asserts it.

**3. The host-boundary thinness law** (survey §4.5). The crossings
carry elements (as opaque handles, RFC 0014/0023), strings, i64s and
bools — nothing else. No app names, no data shipping, no callbacks
across the boundary, no scheduler in the host. t1 is the law's proof
of strength: an entire widget system — diff, registry, lowering —
fits over the ten unchanged crossings. The host never grew an
`ui_create_todo` or an `ui_unlisten`.

**4. Idiomatic current rut.** Dataclass-literal structs (`AppRoot`,
`Todo`, `Req`, `Widget`), shared-cell containers (`let mut r: ?AppRoot
= root`), `?T` nilables, `when` match, f-strings, `PrimMapI64<str>`
(nmapset) as the subject tables, `opaque.downcast` at the trust
boundary (RFC 0014), `entry fn` as the host-callable surface, and the
`appkit` inline-splice module — the mount offers the store + t1 as
ONE unit whose bound surface carries the crossings (why one: the graph
splices each use's transitive sources per use, and store and t1 both
ride pouch; spliced together they define everything once).

## Layout

| file | what |
|---|---|
| `web.d.rut` | the declared surface (RFC 0025, `host_scope = "web"`) — the contract, verified both ways at boot |
| `t1.rut` | **the widget framework** — widgets, the lowering table, the keyed diff/patch, the listener registry; the design is `docs/t1-design.md` |
| `todolist.rut` | **the page program** — the UI layer; `view` builds the widget tree, `on_event` is the one re-entry, `paint` = `t1_render` ONCE per turn |
| `store.rut` | **the "server"** — the list + the request table; pure rut, no `web::` crossings, DOM-free tests |
| `src/state.rs` | the turn law: the FIFO queue, the one pump, the re-entrancy guard (`events are queue, never stack`) |
| `src/hosts.rs` | the 10 registry bindings, written ONCE generic over the backend |
| `src/backend.rs` | the `DomBackend` seam both lanes implement |
| `src/web_dom.rs` | wasm32: web_sys bodies + the raw `rut_web_alloc/boot/pump/last_error` ABI |
| `src/fake_dom.rs` | the host twin: a HashMap element tree with the same semantics (appendChild moves) + a virtual timer clock |
| `src/host.rs` | the native page owner (boot/fire/advance/pump) |
| `src/mount.rs` | the session mount (`mount_app_session`: core + pouch + nmap_host + nmapset + the ONE inline `appkit` unit splicing store + t1 — all sources `include_str!`-embedded; wasm32 has no filesystem) |
| `index.html` / `loader.js` | the page shell: a static `#app` root + ~50 lines of JS that only hands over the app source; the stylesheet keys ONLY lowered tokens |
| `gen/` | wasm-bindgen's generated browser glue (gitignored) — produced by the build recipe below, never committed |
| `tests/store.rs` | the store's laws, DOM-free (9 tests) |
| `tests/todolist_app.rs` | the twin gate: 15 scripted sessions asserting the tree AND the turn order on the lowered DOM |
| `tests/t1_lowering.rs` / `tests/t1_diff.rs` | the framework's bed: 14 lowering snapshots + 18 diff/lifecycle/trap tests (32) |
| `tests/app_law.rs` | **the grep gate**: reads `todolist.rut` and fails loud if biz ever sinks to the DOM's vocabulary (3 tests) |
| `tests/host_surface.rs` | the phase-1 crossing + trap matrix (22 tests) |
| `tests/e2e-browser.mjs` | the through-the-artifact gate (see Gates) — plain node tier + real-browser tier |

## The surface (`web.d.rut`)

`ui_get` · `ui_create` · `ui_set_text` · `ui_attr` · `ui_append` ·
`ui_remove` · `ui_clear` · `ui_set_input_value` · `ui_listen` ·
`tim_after`

Trap shapes (all tested in `tests/host_surface.rs`): loud unknown id
(`web::ui_get: no element '#x'` — a wiring bug is loud, never a rut
optional); kind mismatch naming both sides (`boundary: got \`ul\`
where \`HtmlInputElement\` binds`); DOM exceptions carried verbatim;
the re-entrancy guard; the RFC 0025 boot panics both ways; stale
listener ids as host drift.

**Recorded deviations from the survey's 11 rows** (not re-litigated,
recorded per the batch law — see `docs/todolist-web-report.md`, and
`docs/t1-design.md` §8 for the framework's five):

* `ui_input_value(el) -> str` is absent — the engine's verified-return
  table cannot bind a `-> str` host fn today. Input values are
  **event-carried** instead: every event row on an input carries its
  current value as `detail` (thinner anyway — the thinness law's own
  spirit). The fix is an engine change (the MENU).
* event rows carry no key — the survey's `keydown`/Enter shape has no
  carrier; the app gates the empty draft client-side and uses the
  button's click.
* `on_event` grew a **leading container param** (phase 2): rut has no
  mutable module state, so the boot turn's returned opaque crosses
  back on every turn.
* the dispatch table is `PrimMapI64<str>` — nmapset's keys are the
  SUBJECT (the semantic event name), never the host's listener id.

## Gates

The test story has three lanes, all wired to run from one command:

```sh
cargo test -p todolist-web      # the twin + the law (81 tests)
node tests/e2e-browser.mjs      # the artifact (from examples/05-todolist-web)
```

**Lane 1 — the twin (the real gate, always on).** `cargo test -p
todolist-web`, 81 tests over the SAME `t1.rut`/`todolist.rut`/
`store.rut` sources the browser runs:

* **the framework's bed (32)** — 14 lowering snapshots pinning every
  kind and variant to its exact tag/token/attr tree (the §4.1 table
  as a test), and 18 diff/lifecycle/trap tests through the twin's
  handles: an unchanged render mints zero nodes, a toggle patches one
  token, a keyed reorder preserves element identity and listener ids,
  retirement tracks live ids only, stale fire = the loud trap with
  the page surviving.
* **the app sessions (15)** — scripted twin sessions asserting the
  tree, the counts line EXACT every turn, deadline-then-sequence
  commits (deadline order ACROSS kinds on the virtual clock), the
  client gate, duplicate rejects, lost ids, the stale-listener trap
  re-pointed at the registry (§4.3), the re-entrancy guard.
* **the grep gate (3)** — `tests/app_law.rs` reads `todolist.rut` and
  fails LOUD: the import set is exactly `{appkit}`; no `Node`, no
  `web::`, no `ui_`, no class writes, no tag literals or DOM method
  names (the whole file, comments included, no whitelist); and the
  positive half — `t1_render(`/`t1_subject(`/`.subject(`/`.key(`
  prove the widgets are the whole UI story.
* **the crossing matrix (22) + the store (9)** — the trap shapes and
  the server's laws, DOM-free.

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

**Lane 3 — the browser tier (when geckodriver + firefox exist).**
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
consistent. (The phase-1 recipe lacked the flag and named a file
the loader does not import — recorded as a phase-3 deviation.) The
CLI version must match the crate's wasm-bindgen (see Cargo.lock;
wasm-pack's fetched install works). `gen/` is gitignored, so the
build never dirties the tree — no cleanup chore, and a stale glue
just gets overwritten by the next build.

**What you'll see**: an indigo Add button beside a focused field under
"rut — the `web::` host surface". Type a title — the status line
echoes the typing (`... — typing 'milk'`). Press Add — an italic
`... milk` pending line paints immediately and the field clears; ~400
ms later the committed row replaces it — a checkbox, the title, a
quiet `del`. Add a second while the first flies — both pend, both
commit in book order. Toggle a row — its title goes italic `... tea`
for ~250 ms (the variant patch), then the check fills indigo, the
glyph slides in, and the title strikes through (the `--on` +
`--done` tokens; the flip animates because the keyed diff kept the
node alive). Remove a row — gone in ~120 ms, order preserved. Add
with an empty field — `type a title first`, no request booked.
Duplicate titles commit as a REJECT answer: `rejected 'milk' — already
on the list`. The status line carries the live counts the whole way:
`2 open | 1 done | 0 in flight — added 'jam' as #3`.

Without the glue, `loader.js` fails loud with exactly the commands
above. `node tests/e2e-browser.mjs` is the automated gate over the
same steps.
