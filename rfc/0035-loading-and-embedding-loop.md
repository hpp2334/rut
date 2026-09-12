# RFC 0035: Loading, Host Hooks & the Embedding Loop

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0034 (VM core), RFC 0033 (binaries), RFC 0029
  (declaration files), RFC 0022 (native modules), RFC 0021 (workers)
- **Supersedes:** RFC 0008 §4–5, §8 + RFC 0003 §6 + RFC 5003 §5
  (pre-restructure)
- **Part:** F — Toolchain & artifacts

## Summary

The loader, the hook surface, and the loop that replaces tur's flush
pipeline. Loading executes nothing (RFC 0003 §1); the host owns time
(RFC 0001 G8).

## 1. Host hooks & module loading

```rust
struct HostHooks {
    load_module: fn(&mut Vm, &str) -> Result<ModuleHandle, Trap>, // §2
    now_ms:      fn(&Vm) -> i64,             // monotonic; virtualizable
    set_timer:   fn(&mut Vm, TaskId, i64),   // sleep → timer wheel
    interrupt:   fn(&mut Vm) -> bool,        // RFC 0034 §4 — cooperative stop
    log: Option<fn(&Vm, name: &str, level: Level, msg: &str)>,  // std:log
}                                           // sink; None = silent no-op
```

- **Loading executes nothing**: `load` verifies and links a binary; the
  host then calls an entry explicitly (`vm.call(name, ..)`, sync or
  suspend). Callable names are the module's **`entry fn`s** (plus the
  conventional `main`); their signatures were checked against the host
  crossing rule at compile time (RFC 0023 §1), so a bad surface never
  reaches the embedder as a runtime surprise.
- Import specifiers (`"std:fs"`, `"imaging"`, `"./state"`) are resolved
  entirely by `load_module` — the VM has no filesystem access and no
  builtin loader policy (RFC 0003 OQ-2 stays a host decision).
- **Native specifiers resolve through the surface pipeline** (RFC 0029
  §5): source → mounted `.rutbundle` (RFC 0038 §5) → `.d.ir`
  (version-matched) → `.d.rut` — compiled and
  verified like any module; declaration surfaces are pure, so checking
  needs no Rust and no package bodies. The **link** step then proves every
  surface member *referenced* by the binary has its implementation:
  signature-equal under the crossing rule for `host` decls (ClassTable
  reflection, RFC 0026 §1) or decl-digest-equal for `extern` decls
  (published binary export table, RFC 0029 §6) — pure data compares,
  nothing runs. Missing: `no implementation bound for
  'plugin:my_map.MyMap'` (a load error, not a runtime trap).
- Cyclic imports are a link error (module set must be a DAG at the binary
  level).
- `type_id` rebasing: link-time rebase maps module-local type indices into
  the global table (RFC 0033 §1).

## 2. Workers hook

`spawn_worker` (RFC 0021 §3) is a host hook: it constructs a new `Vm`
(sharing the type table and loader, not the heap) with **its own
`Limits`** — transfers count against the child budget (RFC 0040 §4) —
loads the worker
module, transfers arguments (move semantics on every cell — transfer at
rc==1, else copy — RFC 0021 §4), and returns `Sender`/`Receiver` pairs backed by OS
channels. Channel ops inside the VM are ops (`chsend`/`chrecv`,
RFC 0032 §1); when no value is ready, `chrecv` parks the frame like
`await` (wakers are the channel's, not timers').

## 3. Embedder surface (summary)

| call | purpose |
|---|---|
| `Vm::new(host)` / `Vm::new_in(host, limits)` | construct; budgeted form takes `Limits` (RFC 0040) |
| `vm.set_heap_limit(n)` / `vm.set_fuel(n)` / `vm.add_fuel(n)` / `vm.fuel_remaining()` / `vm.heap_usage()` | live limit control & telemetry (RFC 0040) |
| `vm.load(spec) / vm.load_binary(bytes)` | verify + link (§1, RFC 0033 §2) |
| `vm.call(name, &[Value]) -> Result<Value, Trap>` | sync entry |
| `vm.spawn(name, args) -> TaskHandle` | suspend entry; poll via RFC 0034 §4 |
| `vm.run_until_idle() / vm.poll(deadline) / vm.resume()` | stepping |
| `vm.register_module(name, native)` | native bodies (RFC 0022 §2) |
| `vm.register_struct::<T>()` | repr-C layout check (RFC 0024) |
| `vm.heap_stats()` | live per-type object counts (RFC 0017 §3) |
| `vm.symbolicate(&raw) -> Vec<TraceEntry>` | trace names/spans from loaded binaries (RFC 0036 §3) |

Errors are values (`Result`), bugs are traps, and the host is always in
charge of time, IO, and lifetime — the embeddability pillar (RFC 0001 G8).
Parse-time robustness is the same pillar: the frontend is stack-bounded
with an explicit depth budget (RFC 0030 §4 C3) — malformed or hostile
module source yields diagnostics, never a host stack overflow.

## 4. The tur frame loop

tur today: boa promise jobs + `WorkerMsg::Wake` + flush-driven sleep
futures. With rut the frame loop becomes:

```rust
// one engine frame (native side)
fn on_vsync(&mut self) {
    self.process_host_events();               // input, clipboard, ...
    self.rut.run_until_idle()?;               // resume ready tasks to completion
    if let Some(d) = self.rut.next_deadline() { // earliest timer
        self.worker_ctx.schedule_wake(d);     // host parks until then
    }
    self.render_frame();
}
```

No job executor, no `promise_to_future`, no `.then` under `Context`, no
tslib `_ts_generator` downleveling — `await` is an opcode.

## Open questions

- OQ-1: `chrecv` fairness across multiple awaiting receivers — FIFO per
  channel proposed (RFC 0021 OQ-3 dependency).
