# 05-todolist-web — the web host surface

The web-API crossing for one page app, **example-local** by the phase-0
survey's decision (`docs/todolist-web-survey.md` §4): `web.d.rut`
declares the 11 crossings, this crate binds the bodies — over **web_sys
on wasm32** (the page) and over a **fake-DOM twin on host** (the gate).

**This is phase 1: the host surface only — no app logic.** The todolist
app is phase 2's landing; `demo.rut` is the page shell's minimal smoke
program and `tests/harness.rut` is the twin's crossing harness.

## The surface (`web.d.rut`)

`ui_get` · `ui_create` · `ui_set_text` · `ui_attr` · `ui_append` ·
`ui_remove` · `ui_clear` · `ui_set_input_value` · `ui_listen` ·
`tim_after`

Elements cross as **opaque** handles (`OpaqueBox`, RFC 0014/0023); only
str/i64/bool cross as data; **closures never cross** — listeners
register by id and every asynchronous fact re-enters rut through the
ONE entry fn:

```rut
entry fn on_event(kind: i32, subject: str, detail: str);
// kind 1 = EV_DOM, kind 2 = EV_TIMER
```

Event rows carry the listened input's current value as `detail` when
the listened element IS an input. **Deviation from the survey's 11
rows (recorded): `ui_input_value(el) -> str` is absent** — the engine's
verified-return table cannot bind a `-> str` host fn today (`Ret for
String` reports the program-relative `TY_ANY`, so `verify_against`
reads it as signature drift, and `Ret` cannot be implemented outside
rut-vm: `Slot` is crate-private). Fixing that is an engine change, out
of this phase's scope; the event-carried detail covers the read.

Trap shapes (all tested in `tests/host_surface.rs`): loud unknown id;
kind mismatch naming both sides (`boundary: got \`ul\` where
\`HtmlInputElement\` binds`); DOM exceptions carried; the re-entrancy
guard (`web: event during a rut turn — events are queue, never stack`);
the RFC 0025 boot panics both ways; stale listener ids as host drift.

## Layout

| File | What |
|---|---|
| `web.d.rut` | the declared surface (RFC 0025, `host_scope = "web"`) |
| `src/state.rs` | the turn law: the queue, the one `on_event` pump, the guard |
| `src/hosts.rs` | the 11 registry bindings, generic over the backend |
| `src/backend.rs` | the backend seam both lanes implement |
| `src/web_dom.rs` | wasm32: web_sys bodies + the raw boot/pump ABI |
| `src/fake_dom.rs` | host twin: HashMap tree, virtual timer clock |
| `src/host.rs` | the native page owner (pump/fire/advance) |
| `src/mount.rs` | the session mount (core + pouch inline + `web`) |
| `tests/host_surface.rs` | the twin gate — every crossing + trap + contract |
| `index.html` / `loader.js` / `demo.rut` | the page shell (manual browser step) |

## Gates

- `cargo test -p todolist-web` — the twin gate (native).
- `cargo check --workspace --target wasm32-unknown-unknown` — the page
  compiles; web-sys is target-gated, so host builds never see it.

## Running in a browser (manual — this batch does not fake it)

```sh
cargo build -p todolist-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir . \
  ../../target/wasm32-unknown-unknown/release/todolist_web.wasm
# then serve this directory statically and open index.html
```

Without the glue, `loader.js` fails loud with the same commands.
