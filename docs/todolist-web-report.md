# todolist-web — the batch report

The batch: take rut to the browser, honestly. Four phases, one shared
tree, one example: `examples/05-todolist-web`. Base `d930d98`, all
work on `master`.

## The premise

rut has no async/await (the engine stops at the M3 wall), yet a web
page is nothing but async. The batch's question: can a page app live
in rut TODAY — with the host owning the loop and rut owning the
state — in a shape that survives M3? Verdict: yes. The request-table +
`tim_after` + one-entry-pump pattern (the survey §5 sketch, spelled
for real by the app) is the hand-written machine the survey §5.6
migration note predicted; at M3 it retires into language async without
the store's semantics changing.

## The phases

**Phase 0 — survey** (`807a2ff`): numbering (`05` — `04` is taken by
the RFC 0012 §7 parse-only corpus), prerequisite verdicts (wasm32 lane
healthy; the `unknown trait Task` failure in 04 root-caused and ROUTE
AROUND, not fixed), the web_sys census (zero wasm-bindgen-family deps
anywhere), the crossing-surface spec (§4: the 11 rows, the trap
matrix, the thinness law), the design sketch (§5), and the deviations
ledger's first rows. Deliverable: `docs/todolist-web-survey.md`.

**Phase 1 — the host surface** (`9b6ae76`): `web.d.rut` declares the
crossings; the bodies bind twice — web_sys on wasm32 (the workspace's
FIRST wasm-bindgen-family dep, example-local, target-gated) and a
fake-DOM twin on host (same semantics, same trap shapes, a virtual
timer clock so `cargo test` stays a real gate). The crossing machinery
is written once, generic over a `DomBackend` seam; the turn law lives
in `state.rs` — ONE `on_event` entry, a FIFO queue, the re-entrancy
guard (`events are queue, never stack`); elements cross as
`OpaqueBox<El>`, released at rc 0; listeners register by id; RFC 0025
verified both ways at boot.

**Phase 2 — the app** (`b80ce95`): `todolist.rut` (the page program)
+ `store.rut` (the "server") + the opaque container (`main` returns
`AppRoot`, the pump re-passes it every turn). The store is a request
table (a request NEVER touches the list; `answer(tag)` is the one
commit point; per-kind latency add 400 / toggle 250 / remove 120 ms
makes deadline-ordered commits visible ACROSS kinds; rejects and lost
ids are ANSWERS, unknown tags trap loud). The UI paints pending rows
(`... title`) and in-flight marks (`[ ] ...`), registers listeners by
id into a `PrimMapI64<str>` dispatch table, rebuilds rows per paint
(stale ids trap loud), and mirrors event-carried input values into the
container's draft. 46 example tests (9 store + 15 app sessions + 22
host surface).

**Phase 3 — build, run, report** (this commit): the browser build made
real, the run verified in a REAL browser, the README, this report.

## Phase 3's deliverables and the verification record

**The browser build, for real.** The phase-1 shell documented the glue
step but did not run it. Phase 3 ran the whole recipe:

```
cargo build -p todolist-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir . --out-name web_host \
  ../../target/wasm32-unknown-unknown/release/todolist_web.wasm
```

and proved the artifact loads: the loader's `default()` → `initSync()`
sequence instantiates the module (initSync's `wasm !== undefined`
guard returns the already-built instance — the loader's two-call
pattern verified), `rut_web_boot` compiles, mounts, verifies, and runs
`main` (rc 0), and the full scripted session runs on the live page.

**The path taken: a real browser.** Firefox 154 headless + geckodriver
0.37.1 exist in this environment, so the honest fallback (node-only
smoke) was NOT needed — it ships as tier 1 of the gate instead, and
the browser drive ships as tier 2. `tests/e2e-browser.mjs`:

* **tier 1 (always, plain node)** — the REAL artifact + REAL glue
  through the loader's own ABI sequence on a ~60-line fake DOM: the
  ABI round trip (`pump` before boot → −1; `last_error` envelope
  reads once; exports + memory present), boot rc 0, then the full
  scripted session with real-clock timers.
* **tier 2 (when geckodriver + firefox exist)** — glue generated
  beside `loader.js` exactly per the manual recipe, the dir served
  over http (`application/wasm`), Firefox headless via raw WebDriver
  HTTP (no npm deps): the same session typed and clicked on the live
  DOM. The generated glue is removed afterwards — the committed state
  keeps the loader's loud-fail contract; the artifacts are build
  outputs, reproducible by the documented command.

The scripted session (both tiers, one `runSession`): boot shape
(status line, placeholder, empty list) → the client-side empty-draft
gate books nothing → typing carries the value as the event row's
detail → an add is a REQUEST (pending row now, field cleared) → two
outstanding adds commit in deadline-then-sequence order → a fast
toggle (250) and a slow add (400) land in deadline order across kinds,
the toggle's `[x]` visibly committing while `... jam` still flies →
the slow add lands last → the remove answers in ~120 ms and the row
is gone with order preserved → the counts line reads
`2 open | 0 done | 0 in flight`.

**The loader's recipe had a real gap (fixed).** The phase-1 command
generated `todolist_web.js` — a file the loader does not import (it
imports `./web_host.js`). The working recipe needs
`--out-name web_host`; `loader.js`'s loud-fail message and the README
now name the command that actually works.

**Gates at this commit:**

* `cargo test --workspace` — exit 0, **525 passed, 0 failed**, 82
  suites (unchanged: no rust code moved in this phase).
* `cargo check --workspace --target wasm32-unknown-unknown` — exit 0,
  re-verified with a forced `Checking todolist-web` (cleaned first, no
  cache); the release artifact rebuilt fresh and the e2e re-run green
  against it.
* bench pins — `node benches/run.mjs --runtime rut` over four pinned
  workloads, runner checksum-verified, exit 0; fuel/heap bit-identical
  to the standing records: crossing-nop 104000032 / 236 B,
  json-decode 111330118 / 32.78 MB, nmap-knucleotide 38814389 /
  4.00 MB, nmapset-str 9551761 / 960.6 KB. No engine changes.
* `node tests/e2e-browser.mjs` — exit 0, **47 checks, 0 failures**
  (tier 1: 26, tier 2: 21 — tier 2 omits the five ABI-level checks;
  both run the same session), stable across three consecutive runs.

## Deviations (recorded, not re-litigated)

1. **`ui_input_value` → event-carried** (phase 1): the survey's 11th
   row `ui_input_value(el) -> str` is absent — the engine's
   verified-return table cannot bind `-> str` host fns (`Ret` for
   String reports the program-relative `TY_ANY`, so `verify_against`
   reads signature drift; `Ret` cannot be implemented outside rut-vm —
   `Slot` is crate-private). Input values ride the typing rows'
   `detail` instead. The fix is an engine change (the MENU).
2. **`on_event`'s leading container param** (phase 2): rut has no
   mutable module state (RFC 0003 §1), so the landed 3-arg entry could
   not serve a stateful app — the pump (`drain_queue`) now carries the
   ONE `OpaqueRef` `main` returned and passes it as the leading arg
   every turn. The TEN crossings stay byte-identical.
3. **`keydown`/Enter dropped** (phase 2): event rows carry no key, so
   the survey's Enter-to-submit shape has no carrier; the app gates
   the empty draft client-side and the button's click books the add.
4. **`PrimMap` str-keyed** (phase 2): the dispatch table is
   `PrimMapI64<str>` — nmapset's keys are the event row's subject (the
   listener id) VERBATIM; values are the packed action `todo*10+role`.
5. **the loader's `--out-name web_host` gap** (phase 3): phase 1's
   documented glue command produced a filename the loader does not
   import; the loud-fail message now names the working command.
6. **the browser tier's booking order** (phase 3): the twin proves the
   strong deadline-order shape (the SLOW request booked first, the
   fast one committing first) on the virtual clock; the browser tier
   books the fast toggle FIRST because a WebDriver click can take
   longer than the 150 ms latency gap, which would invert the order
   spuriously. The real clock still drives everything.
7. **the e2e's glue lifecycle** (phase 3): tier 2 generates the glue
   beside the loader and removes it afterwards — the committed tree
   stays at the loader's loud-fail contract by design (artifacts are
   reproducible build outputs, not source), and the script only ever
   deletes files it generated itself.

## The MENU (what this batch grew)

* **verified `str` host-fn returns** (engine): the `Ret for String`
  gap behind deviation 1 — once fixed, `ui_input_value` returns as a
  row of the surface and the event-carried detail becomes an
  optimization instead of a law.
* **richer event rows (key carrying)**: a `key` field on DOM event
  rows would bring back `keydown`/Enter-to-submit without widening
  the crossing set.
* **M3 async as the real fix for the machine**: the request table,
  the `tim_after` bookkeeping and the container pass-through are all
  standing in for language async (survey §5.6 has the migration road);
  at M3 the app's STORE semantics survive, the machinery retires.
* **host-suite CI**: tier 1 runs anywhere with cargo + node;
  `cargo test -p todolist-web` runs anywhere; tier 2 additionally
  wants geckodriver + firefox on the CI image — wiring the three
  tiers into CI is unbuilt (this batch ran them by hand).

## Files (phase 3)

| file | change |
|---|---|
| `examples/05-todolist-web/tests/e2e-browser.mjs` | new — the through-the-artifact gate (node tier + Firefox headless tier) |
| `examples/05-todolist-web/README.md` | rewritten — what the example teaches, the layout, the three gates, the manual browser recipe, what you'll see |
| `examples/05-todolist-web/loader.js` | the loud-fail message + comment now name the working glue command (`--out-name web_host`) |
| `examples/README.md` | the 05 row updated to the landed app (five runnable projects) |
| `docs/todolist-web-report.md` | new — this report |
