# 05-todolist-web — a web page whose brain is rut

One page app — a todolist with a simulated server — where **every DOM
move and every timer goes through eleven declared host crossings**, and
the page's brain (`todolist.rut` + `store.rut`) is pure rut with no
`main`, no event loop, and no async keywords. The host (web-sys on
wasm32, a fake-DOM twin on native) owns the loop; rut owns the state.

The example is **example-local** by the phase-0 survey's decision
(`docs/todolist-web-survey.md` §4): the `web` pkg lives in this crate,
not in the std workspace.

## What it teaches

**1. The async pattern: a hand-written machine.** rut today has no
async/await (the engine stops at the M3 wall — survey §2.2), but a web
page is nothing *but* async. The pattern that fills the gap:

* **state lives in ONE container the host holds.** rut has no mutable
  module state (RFC 0003 §1 — module lets are load-time literals), so
  `main` builds `AppRoot{store, actions, rows, draft}` and RETURNS it
  as an opaque; the pump hands it back on every turn:

  ```rut
  entry fn main() -> opaque;
  entry fn on_event(c: opaque, kind: i32, subject: str, detail: str);
  ```

* **every asynchronous fact is an event row.** DOM listeners register
  by id and re-enter through `on_event` (kind 1 = EV_DOM); the server
  is simulated with `tim_after` (kind 2 = EV_TIMER). One entry, one
  shape — the host is the only writer of the queue.
* **the "server" is a request table, not a scheduler.** An add never
  touches the list: `store.request_add` books a `Req` row (tag, kind,
  latency) and the UI books one `tim_after(req.latency, req.tag)`. The
  list changes only when the timer's turn calls `store.answer(tag)` —
  the one commit point, per-kind latency (add 400 / toggle 250 /
  remove 120 ms) so deadline-ordered commits are *visible across
  kinds*. A rejected title and a lost id are ANSWERS (data the turn
  reads), not traps; an answer naming no booked row traps loud.
* **the round trip is visible in the tree**: a dimmed `... title`
  pending row paints immediately, an in-flight toggle paints `[ ] ...`,
  and the committed row appears only when the answer lands as its own
  turn.

This shape ages well: at M3 the request table becomes real futures and
`tim_after` becomes a timer primitive — survey §5.6 spells the
migration; the app's turn law and store semantics do not change.

**2. The host-boundary thinness law** (survey §4.5). The crossings
carry elements (as opaque handles, RFC 0014/0023), strings, i64s and
bools — nothing else. No app names, no data shipping, no callbacks
across the boundary, no scheduler in the host. The web host cannot
grow an `ui_create_todo` — everything above the crossing set is rut.

**3. Idiomatic current rut.** Dataclass-literal structs (`AppRoot`,
`Todo`, `Req`), shared-cell containers (`let mut r: ?AppRoot = root`),
`?T` nilables, `when` match, f-strings, `PrimMapI64<str>` (nmapset) as
the dispatch table, `opaque.downcast` at the trust boundary (RFC 0014),
and `entry fn` as the host-callable surface.

## Layout

| file | what |
|---|---|
| `web.d.rut` | the declared surface (RFC 0025, `host_scope = "web"`) — the contract, verified both ways at boot |
| `todolist.rut` | **the page program** — the UI layer; `main` returns the app container, `on_event` is the one re-entry |
| `store.rut` | **the "server"** — the list + the request table; pure rut, no `web::` crossings, DOM-free tests |
| `src/state.rs` | the turn law: the FIFO queue, the one pump, the re-entrancy guard (`events are queue, never stack`) |
| `src/hosts.rs` | the 10 registry bindings, written ONCE generic over the backend |
| `src/backend.rs` | the `DomBackend` seam both lanes implement |
| `src/web_dom.rs` | wasm32: web_sys bodies + the raw `rut_web_alloc/boot/pump/last_error` ABI |
| `src/fake_dom.rs` | the host twin: a HashMap element tree with the same semantics + a virtual timer clock |
| `src/host.rs` | the native page owner (boot/fire/advance/pump) |
| `src/mount.rs` | the session mounts (`mount_app_session`: core + pouch + nmap_host + nmapset + store + web, all sources `include_str!`-embedded — wasm32 has no filesystem) |
| `index.html` / `loader.js` | the page shell: a static `#app` root + ~50 lines of JS that only hands over the app source |
| `gen/` | wasm-bindgen's generated browser glue (gitignored) — produced by the build recipe below, never committed |
| `tests/store.rs` | the store's laws, DOM-free (9 tests) |
| `tests/todolist_app.rs` | the twin gate: scripted sessions asserting the tree AND the turn order (15 tests) |
| `tests/host_surface.rs` | the phase-1 crossing + trap matrix (22 tests) |
| `tests/e2e-browser.mjs` | the through-the-artifact gate (see below) — plain node + optional real-browser tier |

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
recorded per the batch law — see `docs/todolist-web-report.md`):

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
  event row's subject (the listener id) verbatim.

## Gates

Three tiers, all wired to run from one command:

```sh
node tests/e2e-browser.mjs            # from examples/05-todolist-web
```

1. **The twin (the real gate, always on)** — `cargo test -p
   todolist-web`: the SAME `todolist.rut`/`store.rut` sources driven
   end-to-end on the fake-DOM twin — scripted sessions asserting the
   tree, the turn order, the listener-id churn, the trap matrix. 46
   tests; the fake twin keeps `cargo test --workspace` a real gate for
   a web example.
2. **The node tier (always once the artifact exists)** — builds the
   wasm (loud-fail with the exact command if it cannot), runs the real
   wasm-bindgen CLI (version-matched to Cargo.lock), then drives the
   REAL artifact + REAL glue through the loader's own ABI sequence
   (import glue → `default()` → `initSync()` → `alloc` → write →
   `boot` → `last_error`) on a ~60-line fake DOM: the full scripted
   session, headless, no browser needed.
3. **The browser tier (when geckodriver + firefox exist)** — generates
   the glue into `gen/` exactly as the manual recipe says, serves the
   dir over http, drives Firefox headless via raw WebDriver HTTP (no
   npm deps): the same session typed and clicked on the live page.
   `gen/` is gitignored, so nothing is cleaned up afterwards — the
   tree stays at HEAD-clean and the page keeps working locally.
   Missing binaries = loud skip with the manual recipe (exit 0); a
   failed check = exit 1.

Missing artifact or CLI → exit 1 with the exact build command: the
gate guards the artifact, it does not silently skip it.

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

**What you'll see**: an input and an Add button under "rut — the
`web::` host surface". Type a title — the status line echoes the
typing (`... — typing 'milk'`). Press Add — a dimmed `... milk` row
paints immediately and the field clears; ~400 ms later the committed
`[ ] milk` row replaces it with its own mark/del buttons. Add a second
while the first flies — both pend, both commit in book order. Toggle
milk — `[ ] ...` while the answer travels, then `[x]`. Remove milk —
gone in ~120 ms. Add with an empty field — `type a title first`, no
request booked. Duplicate titles commit as a REJECT answer:
`rejected 'milk' — already on the list`. The status line carries the
live counts the whole way: `2 open | 1 done | 0 in flight — added 'jam' as #3`.

Without the glue, `loader.js` fails loud with exactly the commands
above. `node tests/e2e-browser.mjs` is the automated gate over the
same steps.
