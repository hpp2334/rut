# Loading and the embed loop

Loading verifies, links, and mounts a program — it **executes nothing**.
The host owns time, I/O, and lifetime: the VM never reads the filesystem
or a clock on its own, and the host decides when (and whether) any rut
code runs.

## From path to machine

```sh
rut run app/            # a directory with rut.toml
rut run plugin/plugin.rutbundle
```

The loader side (`rut-driver`) turns a path into a booted `Vm` in five
steps:

| step | what happens |
|---|---|
| 1. mount | A **directory is one module**: its `rut.toml` names the package (`name`), its entry (`entry.lib` / `entry.type` / `entry.libs`), and its `[deps]`/`[peer-deps]`/`[dev-deps]` ([Project structure and rut.toml](project-structure.md)). A `.rutbundle` mounts identically from a zip ([Module bundles](bundles.md)). The graph walks `[deps]` recursively — cycle guard, first-mount-wins, name-mismatch is an error — then runs one peer gate over the closed set ([Dependency kinds](dependency-kinds.md)). |
| 2. resolve surfaces | Use paths resolve against mounted modules, exact and single-step: a package name resolves or the diagnostic names the consumer manifest. A `.d.rut` surface compiles through the checker and publishes signatures only. |
| 3. compile the graph | Each module compiles (sources, in dependency post-order); `inline = true` packages splice into their consumers instead of linking; host packages synthesize bodyless thunks from their declared surfaces; a mounted bundle's compiled packages push their decoded binaries at fresh, rebased scopes ([The compiler pipeline](compiler.md)). |
| 4. link + flatten | Module-local type/function/const ids rebase into the global tables; the shared boot prefix passes through; name tables merge; duplicate `(trait, type)` impl pairs and duplicate module names are link errors. Cyclic use is a compile-graph error, never a runtime event. |
| 5. verify + boot | The load verifier re-checks every function ([Module binary and verification](module-binary.md)); the `Vm` constructor joins the program's host thunks against the embedder's `HostRegistry` — a declared-but-unbound host fn is a boot error. |

Loading a `.rutc` binary (`vm.load_binary`-style paths, and the wasm
runner) skips steps 2–3 and lands directly in verify + boot.

## What the host can call

Callable names are the program's **`entry fn`s** plus the conventional
`main`. Their signatures were checked against the host-crossing rule at
compile time — primitives, `str`, `bytes`, `opaque`, `?T` over a crossing
type, and crossing tuples — so a bad surface can never surprise the
embedder at call time. An `entry fn -> (?T, err)` decodes positionally at
`vm.call` as a `(value, err)` pair (below).

Use paths never execute anything: importing a package mounts its
surface; only the host's calls run code.

## The embedder surface

Everything the host needs is a method on `Vm` (plus the registries built
before it):

| call | purpose |
|---|---|
| `HostRegistry::register(name, f)` | bind a host-fn body; signatures derived from the Rust shape |
| `hosts.install_host_pkg(&ctx, pkg)` | the installer lane: one built `HostPkg` per host pkg, checked against the mount snapshot (`session.host_pkg_context()`) |
| `pkg_fn!` / `pkg_async_fn!` | the builder's sugar — one spelling → one row / the five-row async family ([embedding and native modules](embedding.md)) |
| `mount_std_core(session)` / `mount_std(session)` / `mount_std_async(session)` | mount the builtin packages (`core`; `core` + `calc`; the async pair) |
| `load_path_session(path)` / `load_bundle_bytes(bytes, origin)` | mount a directory / an in-memory bundle |
| `load_path_session_with(path, &fetch)` | the fetched lane: `[deps]` url rows ride the host's `DepFetch`, the `sha256` pin verified at the mount door |
| `compile_graph(&session, root)` | compile + link the mounted graph |
| `Vm::new(prog, &limits, hooks, registry)` | boot; fails if a declared host fn is unbound |
| `vm.call(export, args) -> Result<Value, Trap>` | sync entry — typed arg/ret adapters over the boundary |
| `vm.resume()` | continue a parked frame after `add_fuel` |
| `vm.run_ready()`, `vm.next_deadline()`, `vm.pending_tasks()`, `vm.drive(fut)` | the async driving verbs |
| `vm.set_now(ms)`, `vm.arm_timer(deadline, fut)` | the virtual clock |
| `vm.add_fuel(n)`, `vm.fuel_used`, `vm.heap_usage()` | budgets and telemetry ([Resource limits and fuel](resource-limits.md)) |
| `vm.call_host_row(row, args)` | invoke a registered host row directly (tests, tooling) |

Errors are values (`Result`), bugs are traps, and the host is always in
charge of time, I/O, and lifetime.

## The frame loop

An embedder that owns an event loop (a GUI, a game frame, a web page)
runs one engine frame per tick:

```rust
fn on_vsync(&mut self) {
    self.process_host_events();                 // input, network, ...
    self.rut.run_ready()?;                      // drive ready tasks to completion
    if let Some(d) = self.rut.next_deadline() { // earliest armed sleep
        self.schedule_wake(d);                  // host parks until then,
    }                                           // then advances the clock
    self.render_frame();
}
```

`pending_tasks()` is the idle test — zero means the program has nothing
runnable. There is no job executor inside the VM: parking, waking, and
timing are these three verbs plus `set_now`
([Async and await](async.md)). A program that mounts no async packages
simply has no launcher; `await` stays cold-poll inline
([The async model](../core-concepts/async-model.md)).

## The soft-fail law (the err channel)

An `entry fn -> (?T, err)` never produces `Err`. The pair decodes at
`vm.call` as `Ok` — **with data in the pair**: a non-empty `err` string
means the turn failed softly; an empty `err` means success (nil value =
"not found"). `Err` remains exclusively the trapped turn: a bug, wiring
drift, exhausted fuel — the loud channel.

The law for host event loops (**the pump pattern**): per queued event,
call the turn entry, then —

- **`Ok`**: decode the pair. A non-empty err is *reported* (surfaced to
  the user/log, retained) and the loop **keeps draining**: the container
  survives, the queue proceeds, the page stays alive. A returned err must
  never poison the container or kill the drain — it is data the host acts
  on.
- **`Err`**: the pump aborts **loud** — a trapped turn is a bug, not
  data. The `Vm` itself survives for the next good turn.

On a raw JSON boundary (the wasm host), the envelope carries `"err"`
beside `"trap"`: a soft-fail run reads `{"trap": null, "err": "…"}`, a
success reads `"err": null`, and a panic sets `trap` with `"err": null`.
`trap` never fills `err`. Fuel and heap reporting are unchanged by this.

## Parse-time robustness

The same embeddability pillar holds at the other end of the pipeline: the
[frontend](frontend.md) is stack-bounded with explicit nesting budgets —
malformed or hostile module source yields diagnostics, never a host stack
overflow, and a module with diagnostics never reaches the VM.
