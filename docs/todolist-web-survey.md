# todolist-web survey — phase(0) of `todolist-web`

Survey and prerequisite gate for the `05-todolist-web` example, written
2026-09-22 against tree `d930d98` (master, clean). Every behavioral claim
below was executed against the actual tree, not inferred: the wasm32
check was run (twice, one forced recompile), the custom-async failure was
reproduced through the shipped CLI, and every cited surface was read in
the compiler, the VM, and the RFCs. Probe logs live under
`/tmp/opencode/batch-todolist-web/p0/`.

This phase ships **no code** — the deliverable is this document: the
numbering decision, the prerequisite verdicts, and the crossing-surface
spec that becomes phase 1's implementation contract.

---

## 1. Numbering decision — `05-todolist-web`

The plan named the example `04-todolist-web`. **04 is taken.**

The slot map at `d930d98`:

| Slot | Example | Status |
|---|---|---|
| `00-todolist` | console todolist (host-driven CRUD over `opaque`) | runnable, gated by `tests/session.rs` |
| `01-sort` | dispatcher + `Result` at the boundary | runnable |
| `02-digest` | codecs/hashes, host as oracle | runnable |
| `03-plugin` | `.rutbundle` plugin, re-entrant `vm.call` | runnable |
| `04-custom-async` | user `impl Task<T>` + user launcher | **parse-only corpus until M3** (by design, see §3) |

Decision: the new example takes the next free slot, **`05-todolist-web`**.
Rationale, honestly:

- The one rule with no give: never delete or break `04-custom-async`.
  Renumbering it would churn `examples/README.md`, the parser conformance
  corpus walk, the LSP corpus gates (all four trees are walked with a
  floor assertion), and an RFC citation — RFC 0012 §7 names
  `examples/04-custom-async` as *the* user-impl-of-a-builtin-trait test.
  Churn for zero gain.
- `00-todolist` is the console twin of this example (same app shape, DOM
  instead of stdout). `05-todolist-web` sits next to it in reading order
  without disturbing anything.

Phase 1/2 therefore create `examples/05-todolist-web/` and update
`examples/README.md`'s table (currently "Four Cargo projects" + the
parse-only paragraph) in the example's own commit.

---

## 2. Prerequisite verdicts

### 2.1 wasm32 lane — HEALTHY, no fix

`cargo check --workspace --target wasm32-unknown-unknown` at `d930d98`:
**exit 0**. Because a plain re-run would ride the cache, the check was
made honest by force: `cargo clean -p rut-vm-threaded -p rut-wasm -p
rut-lsp-wasm --target wasm32-unknown-unknown`, then re-check — the
cleaned crates genuinely recompiled (`Checking rut-vm-threaded` /
`Checking rut-lsp-wasm` / `Checking rut-wasm` in the log, 0 errors,
`Finished`). `rut-vm-threaded` — the crate the E0423 one-liner touched in
4b05618 — was additionally cleaned and checked alone: exit 0. The
`rut-lsp` tokio `cfg(not(target_arch = "wasm32"))` dep split holds
untouched. **Verdict: the lane needs nothing.**

Consequence for phase 1 worth recording now: the workspace wasm32 gate
will keep compiling whatever the new example adds. A `web-sys`-dependent
host crate therefore compiles for wasm32 from day one (web-sys is
wasm-only by construction) and must be **target-gated out of host
builds** (§4.3).

### 2.2 `04-custom-async` "unknown trait `Task`" — reproduced, root-caused, ROUTE AROUND

**Reproduction** (the shipped CLI, not a guess):

```
$ cargo run -q -p rut-cli -- run examples/04-custom-async/custom_async.rut
error: unknown trait `Task`
  --> line 122, byte 4592
 122 | impl Task<T> for CustomTask<T> {
     |      ^^^^^^^^^^^
exit 1
```

**Root cause** — the full chain, read in the compiler:

1. `examples/04-custom-async/custom_async.rut:25` does
   `use core::{ Task, TaskRunContext };` and line 122 writes
   `impl Task<T> for CustomTask<T>`.
2. `rut/core/core.d.rut` declares **no** `Task`/`TaskRunContext`. Its
   only builtin trait is `Iterator<E>` (line 229). What the file *does*
   say is the contract: "`host fn` is exclusively the EMBEDDER'S
   surface… the engine-woven trait `Iterator`" — the async surface is
   simply not in it yet.
3. The resolver (`crates/rut-lir/src/check/resolve.rs:10-84`) knows
   exactly one engine-woven trait: `resolve_trait_ref` matches
   `NativeTrait::Iterator` specially (line 17), consults declared and
   extern traits, and falls through to
   `format!("unknown trait `{}`", …)` at line 73. There is no `Task`
   arm. **The error is the resolver correctly reporting that the async
   plan has not landed.**
4. The file predicted this itself — its header (lines 4-13): "PARSE-ONLY
   CORPUS until the async plan lands (M3). The engine async surface this
   file names — the `Task<T>` trait and its `TaskRunContext` — is
   declared in core by the async plan (RFC 0012 §7)… the member set is
   the async plan's to freeze." `examples/README.md` line 13 repeats it:
   "parse-only — runnable at M3".

**What the async plan owes (the M3 map, from the RFCs + compiler):**

| Surface | RFC | State today |
|---|---|---|
| `async fn` / `await` / `select` grammar | 0018 §2 | **parses** (`"async"`, `"await"` reserved; `item.rs:33`, `expr.rs`), then hard-diagnosed at lowering: "`async` functions are not supported in this build —cold-poll futures land in M3 (RFC 0018)" (`lir/mod.rs:280`) and "coroutines are not supported in this build (RFC 0018—020, M3)" (`lir/expr.rs:173`) |
| coroutine frames, `Flow::Wait`, poll loop | 0018 §3-4, 0035 §4 | absent — no `Suspend`/`Yield` op, no resume path in `rut-vm/src/interp/` (the only "coroutine" hits are forward references in comments) |
| `spawn`/`cancel`/`select`, `Task<T>` handle | 0019 | absent (RFC 0019's `Task<T>` is a spawned-future handle — note it is a *different* `Task` story than the §7 trait) |
| `Task<T>` + `TaskRunContext` + `launch_task` declared in core | 0012 §7 | absent — exactly the missing decls the example names; the engine auto-impl for desugared frames would weave them |
| host-futures bridge (`RutAwaitable`, waker → wake queue) | 0020 | absent |
| inline skips | — | every inliner arm returns `false` for `is_async` bodies with the comment "`async` is diagnosed when the body is compiled" (`lir/call.rs:1686,1779,1874`) |

**Verdict: ROUTE AROUND — no fix commit.** The reasoning, both halves:

- *It does not block the todolist.* The todolist-web app does not need
  language async. Fixing the `Task` trait alone would not make anything
  runnable — the very next diag is "`async` functions are not supported
  in this build". Making the example *async* in the language sense means
  landing M3: coroutine frames, the poll loop, the run contexts, the
  host-futures bridge — a milestone the RFCs gate behind the async plan,
  explicitly out of this batch's scope.
- *A partial fix would be worse than none.* Declaring `Task`/
  `TaskRunContext` in `core.d.rut` today would front-run the async plan's
  frozen member set — the one thing the example's header says is *not*
  ours to freeze — and would wire an unbodied trait into the resolver
  (an `Iterator`-style engine arm with nothing behind it) purely to turn
  one honest parse-only failure into a different honest parse-only
  failure.

The route-around shape is §6's design: the app's asynchrony lives in the
**host's turn law** (timers + one re-entry point), and the rut side keeps
its request state in an explicit table — the hand-written stage machine
`04-custom-async` documents, spelled for real. When M3 lands, the same
example is the migration demo (§6.6). Nothing in `04-custom-async` was
touched.

### 2.3 Workspace gates at `d930d98`

- `cargo test --workspace`: green (77 suites, 0 failures).
- Tree clean; no parallel-session files touched at any point; everything
  this phase staged by explicit path (`docs/todolist-web-survey.md`).

---

## 3. web_sys census

**Zero web bindings anywhere in the workspace.** `grep web-sys|js-sys|
wasm-bindgen` over `Cargo.lock` → no hits; over every crate manifest →
no hits. The wasm lane's two shims are deliberately raw:

- `crates/rut-wasm` — "wasm shim exposing compile/run over a raw ABI
  (RFC 0041 §3) — **no wasm-bindgen, plain exports**"
  (`rut_alloc`/`rut_compile`/`rut_run`/`rut_result_*` over linear
  memory; the demo's `runner.ts` instantiates it with `{}`).
- `crates/rut-lsp-wasm` — same raw-ABI philosophy for the LSP; deps are
  `rut-lsp` (default-features off) + `ls-types` + `serde_json`.

So phase 1 introduces the workspace's **first** wasm-bindgen-family
dependency. Profile, so the gate stays green:

```toml
# examples/05-todolist-web/Cargo.toml — example-local, never a workspace dep
[lib]
crate-type = ["cdylib"]

[target.'cfg(target_arch = "wasm32")'.dependencies]
web-sys = { version = "0.3", features = [
    "Window", "Document", "Element", "HtmlElement", "HtmlInputElement",
    "Node", "NodeList", "EventTarget", "Event", "InputEvent", "KeyboardEvent",
] }
# wasm-bindgen + js-sys arrive transitively (web-sys 0.3.105 / js-sys
# 0.3.103 are current as of this writing; pin exactly what lands in
# Cargo.lock in phase 1's commit).
```

- Version: web-sys **0.3.x** (0.3.105 current, Sep 2026). Features are
  1:1 with the WebIDL types touched — this is the minimal set for §5's
  surface (`Window` for `set_timeout`, `Document` for
  `get_element_by_id`/`create_element`, `Element`/`Node` for the tree
  ops, `HtmlInputElement` for the input value, `EventTarget` +
  event types for listeners).
- Target-gating keeps host builds identical: on
  `x86_64-unknown-linux-gnu` the dependency does not exist, and the
  host-side test twin (§5.4) binds the *same* `.d.rut` surface to a fake
  DOM — no web-sys on the host lane at all.
- A `web_sys::Closure` (wasm-bindgen) is needed only inside the host
  crate's listener plumbing; rut never sees it — the closure law below.

The page shell: phase 1 ships a minimal static `index.html` + a small
hand-written loader (instantiate the cdylib, call its exported
`boot`/`dispatch` ABI — the `rut-wasm` pattern of raw exports, but with
web-sys available *inside* the module, the loader stays ~20 lines).

---

## 4. The crossing-surface spec (phase 1's contract)

A **`web` host pkg**, example-local: the surface is
`examples/05-todolist-web/web.d.rut` (mounted by the example's host via
`Session::register_module`, `host_scope = "web"`), the bodies live in the
example's host crate behind `#[cfg(target_arch = "wasm32")]`. This is the
`rt` precedent exactly: `rut-wasm` already mounts `rt`'s `.d.rut`
programmatically and sets its host scope in code
(`rut-wasm/src/lib.rs:101-107`), and `rut_std::logger` shows the binding
shape. Example-local is a phase-0 decision: the toolchain tree
(`rut/`, `rut-std`) stays web-free until a *second* web example exists;
promotion to `rut/web/` + a cfg-gated `rut-std` module is the recorded
future road, not a phase-1 deliverable.

### 4.1 The surface (`web.d.rut`)

```rut
// web.d.rut — the DOM crossing for one page app. Every signature is
// concrete over the crossing set (RFC 0023 §1): str, i32, i64, bool,
// opaque. Elements cross as opaque handles (RFC 0014) — the one cell
// shape an embedder may hand out; rut holds them across turns.

pub host fn ui_get(id: str) -> opaque;
pub host fn ui_create(tag: str) -> opaque;
pub host fn ui_set_text(el: opaque, text: str);
pub host fn ui_attr(el: opaque, name: str, value: str);
pub host fn ui_append(parent: opaque, child: opaque);
pub host fn ui_remove(parent: opaque, child: opaque) -> bool;
pub host fn ui_clear(el: opaque);
pub host fn ui_input_value(el: opaque) -> str;
pub host fn ui_set_input_value(el: opaque, v: str);
pub host fn ui_listen(el: opaque, event: str) -> i64;
pub host fn tim_after(ms: i64, tag: str);
```

### 4.2 Names, bodies, laws

| fn | body (web_sys) | law |
|---|---|---|
| `ui_get` | `document.get_element_by_id(id)` | missing id is a **trap**: `web::ui_get: no element '#x'` — a wiring bug is loud (the nil-does-not-bind-OpaqueRef law), never a rut-side optional |
| `ui_create` | `document.create_element(tag)` | a DOM-rejected tag traps with the `JsValue` message |
| `ui_set_text` | `set_text_content(Some(text))` | |
| `ui_attr` | `set_attribute(name, value)` | `Err` (DOM exception) traps carrying its message |
| `ui_append` | `append_child(child)` | hierarchy `Err` traps (e.g. appending an ancestor) |
| `ui_remove` | `remove_child(child)` | `false` when not a child — a real DOM negative, not an error; no trap |
| `ui_clear` | `replace_children()` (or the `while first_child remove` form) | one web call, not app logic — the re-render reset |
| `ui_input_value` / `ui_set_input_value` | `HtmlInputElement::value` / `set_value` | a non-input element traps naming both sides: `boundary: got 'div' where HtmlInputElement binds` (the expect_kind law) |
| `ui_listen` | `add_event_listener_with_callback(event, Closure)` | returns a **listener id** (`i64`, from 1). The host keeps a registry `(id → element, event, Closure)`; the Closure pushes `(listener_id, detail)` into the host's event queue. The registry owns its Closures for the page's life (detaching is out of scope; documented limitation) |
| `tim_after` | `window.set_timeout(closure, ms)` | the closure enqueues `(EV_TIMER, tag)`; **this is the simulated-latency primitive** — the "server" answers after `ms` |

**The closure law (RFC 0025):** closures are named in the crossing set's
exclusions — "trait-typed values, closures stay inside the VM". So no
host fn ever takes or returns a callable. Listeners are registered by
*string identity* (`ui_listen` + the id), and the callback direction is
the host's alone: it holds the `Closure`, and re-enters rut through the
one re-entry point below. Rut code reads events as data.

**The re-entry point** — rut-side, an ordinary `entry fn` (RFC 0035 §3),
frozen numeric event kinds:

```rut
entry fn on_event(kind: i32, subject: str, detail: str);
// kind 1 = EV_DOM    (subject = listener id as str, detail = "" or the
//                      input's current value for "input" events)
// kind 2 = EV_TIMER  (subject = the tim_after tag, detail = "")
```

The host calls it with `vm.call::<_, ()>("on_event", (kind, subject,
detail))` — the typed-`call` crossing (`String`: `CallArg`; `i32`:
`CallArg`; `()`: `Ret` — all existing impls, `boundary.rs`). One entry,
one shape: every asynchronous fact the page produces arrives as an event
row.

### 4.3 Trap shapes (all of them, so phase 1 can enumerate the tests)

1. **unknown id** — `ui_get` on a missing element → trap (above). Loud,
   fail-fast; the page's ids are the host's own contract.
2. **kind mismatch** — an element-typed op on the wrong DOM type → trap
   naming both sides (the `expect_kind` convention, `boundary.rs:43-81`).
3. **DOM exception** — `create_element`/`set_attribute`/`append_child`
   `Err` → trap carrying the exception's message (never silently
   swallowed; the repo's loud-fail culture).
4. **re-entrancy** — the host keeps an `in_turn: bool` guard set around
   every `vm.call`. An event firing while the guard is up (possible only
   via synchronous DOM APIs like `focus()` dispatching inside a turn) →
   trap `web: event during a rut turn — events are queue, never
   stack`. The queue absorbs it instead where semantically sane
   (phase 1 picks: queue, and the guard exists to make the choice
   visible). Single-thread law, RFC 0034 — wasm has one thread, so the
   guard is a plain bool.
5. **binding contract** — declared-but-unbound / bound-but-undeclared /
   signature drift → `HostRegistry::verify_against` panics at boot
   (existing law, RFC 0025). Nothing new; listed so phase 1 wires it.
6. **listener/timer id drift** — a stale id reaching the dispatcher is a
   host-side internal error → panic in the host crate (an embedding bug,
   not a rut diagnostic — same class as 5).

### 4.4 RFC 0023/0025 conformance checklist

- [x] every signature concrete over the crossing set: `str`, `i32`,
  `i64`, `bool`, `opaque` — no generics (generic host fn = compile
  error, RFC 0025), no user types, no closures, no `Vec`/arrays.
- [x] optional-ish answers spelled the tree's way: **loud traps for
  wiring bugs** (nil-does-not-bind), plain `bool` where the DOM itself
  has a negative (`ui_remove`). No `?T` in host signatures — the v1.1
  law is optionals cross as tuples (boundary doc, RFC 0007 §7), and the
  precedents (nmap's packed scalars, 00-todolist's `has_title`+
  `title_of` split) both avoid them; this surface needs neither.
- [x] names are `<scope>::<name>` = `web::<name>` (the
  `expected_host_fns` rule: `host_scope` or pkg name — `session.rs:208`);
  slot ids assigned by declaration order at `.d.rut` compile (RFC 0025).
- [x] element handles are `opaque` (RFC 0014) — rut stores, passes, and
  drops them; the `OpaqueRef` drop releases the JS reference
  deterministically at rc 0 (RFC 0016 §3). The host keeps no strong
  handle of its own beyond the registry's listener closures.
- [x] no `builtin` spelled in the embedder decl (RFC 0025's reservation
  rule).
- [x] the host registers bodies *before* `Vm::new` (the eager join),
  never mutates them mid-run.

### 4.5 What the host must NOT grow (the thinness law)

- no app names: nothing like `ui_add_todo_row` — rows are rut loops over
  the §4.1 primitives.
- no data shipping: the todo list lives in rut; only the strings one
  crossing needs at a time cross (RFC 0023 §2: owned copies are the
  explicit "I keep this data" — the host keeps none).
- no state machine: `tim_after` is a timer, not a scheduler; the
  request/response story is rut's (§6).
- no DOM reads rut didn't ask for: if a future app needs
  `text_content()`/`get_attribute`, they join as *new host fns* in a
  later phase — not as a side effect of an existing call.

---

## 5. Design sketch (phase 2's app, phase 1's harness)

### 5.1 The turn law — where "async" lives pre-M3

RFC 0018's own law names the honest shape: **"the host owns time"** —
there is no microtask queue, nobody polls, the host drives. Pre-M3 the
VM cannot suspend, so the app is a **turn-based machine**: each DOM
event or timer fire is one `vm.call("on_event", …)` turn; between turns
rut is inert and the host holds only the event queue. This is precisely
the `04-custom-async` narrative — a resumable machine whose state lives
in explicit fields — spelled for real, with the host in the scheduler's
chair.

### 5.2 The simulated server

A rut-side `class Server` (or module fns) simulating latency:

- `add_todo(title)` does **not** mutate the list. It books a request:
  `reqs.push(Req { id, kind, title })` and calls
  `tim_after(400, "req:<id>")` — one host fn, one timer. The request
  table (ids → pending work) is the app's explicit continuation table.
- The timer fires → `EV_TIMER` turn → the server looks up the request,
  commits it to the list, re-renders (§5.3), and optionally books a
  second timer for a "sync" badge — multi-stage requests are just more
  rows in the table, which is the point.
- Failure shapes (a rejected title, an id the server "lost") dispatch
  through the same table — cancellation is data the turn reads (RFC 0018
  §4's cancellation-as-data law, at app scale).

Why host timers and not a rut-side counter: a rut busy-wait cannot yield
(fuel would burn, the page would freeze — there is no suspension to
park on). `tim_after` is a real web API through the thin wrapper; zero
host intelligence; latency is visible and honest in the UI.

### 5.3 The UI loop

One render strategy, minimal crossings: `ui_clear(list)` then rebuild
rows (`ui_create` + `ui_set_text` + `ui_attr` + `ui_append` per row) on
every committed change; the input row persists outside the rebuilt
subtree. Events: `ui_listen` on the add button (`"click"`), the input
(`"input"` for enable/disable, `"keydown"` for Enter), and each row's
done-checkbox + delete button (`"click"`, listener ids resolved through
rut's own listener table — rut registered the ids, rut owns the
dispatch).

Boot: the host compiles `todolist_web.rut` (mounted: std core+calc,
`pouch` as inline source — the wasm-embeds-pouch precedent is
`rut-wasm/src/lib.rs:109`), binds the `web::` bodies, `verify_against`,
`Vm::new`, one `vm.call("main", ())` turn that builds the static DOM and
registers listeners; then the page is purely event-driven.

### 5.4 Testing without a browser (phase 1's harness, phase 2's gate)

The same `web.d.rut` surface binds to a **fake DOM** on the host lane: a
`HashMap`-backed element tree implementing the §4.1 semantics (parent/
child, attributes, input value, a scripted timer queue that fires
synchronously at test-controlled points, and the same trap shapes). The
gate runs the *real* `todolist_web.rut` against it and asserts the CRUD
session end to end — `00-todolist/tests/session.rs` is the template, and
it is what keeps `cargo test --workspace` a meaningful gate for a web
example. The wasm32 side is covered by `cargo check --workspace --target
wasm32` (compile gate) plus the existing e2e-wasm lane's shape; a real
browser run stays a manual step this batch does not fake.

### 5.5 Budgets

`Limits { fuel: Some(1_000_000), heap_limit_bytes: Some(4 MiB),
interrupt_every: 1024 }` to start — the 00-todolist numbers; phase 2
raises fuel if the render loop's crossings need it (each turn is bounded,
so latency never feeds fuel).

### 5.6 The M3 migration note (why this shape ages well)

When the async plan lands, this app is the before/after demo: the
request table becomes `async fn` frames (`await sleep(400)` — RFC 0020's
`sleep` is exactly `tim_after` with a waker instead of a tag), the
`on_event` dispatch becomes `await`-points, and the host's queue+pump
becomes `run_until_idle()`/`next_deadline()` (RFC 0035 §1). The `web::`
surface itself does not change — crossings were concrete over the
crossing set, so only the rut-side control flow migrates. That is the
test of the thinness law.

---

## 6. Deviations from the plan text

1. **`04-todolist-web` → `05-todolist-web`.** The plan's number was
   written against a slot map that has since filled; resolved to the
   next free slot (§1). The plan's *content* is unaffected.
2. **No prerequisite fix commits.** Both prerequisite checks came back
   healthy-or-by-design: wasm32 green on HEAD (§2.1), and the
   custom-async failure is the documented pre-M3 parse-only contract,
   root-caused to the async plan's absence and routed around, not
   papered over (§2.2). This commit is therefore the survey alone.
3. **The `web` pkg is example-local, not a tree pkg.** Decision + the
   recorded promotion road in §4; phase 1 may revisit with the
   orchestrator if the demo wants to `use web::` from the playground.
4. **Event re-entry is one three-string `entry fn`**, not a listener-
   callback ABI: the crossing set excludes closures, so the callback
   direction is host-owned by construction (§4.2). The plan's "event
   listeners → async bridge" lands as exactly this bridge.
