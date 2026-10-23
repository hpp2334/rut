# Loading and the embed loop

Loading verifies, links, and mounts a program — it **executes nothing**.
The host owns time, I/O, and lifetime: the VM never reads the filesystem
or a clock on its own, and the host decides when (and whether) any rut
code runs.

## From path to machine

```sh
rut run app/            # a directory with rut.jsonc
rut run plugin/plugin.rutbundle
```

Turning a path into a booted `Vm` is the walk (`rut-native`) feeding
the run chain (`rut-driver`) feeding the builder door (`rut-vm`) —
five steps:

| step | what happens |
|---|---|
| 1. the walk | A **directory is one module**: its `rut.jsonc` names the package (`name`) and its surface (`entry.type` — optional on a host pkg, where `src/mod.d.rut` is the default root and a spelled entry wins, resolved under `src/`), its body is the `src/mod.rut` module tree, and its `deps`/`dev-deps` ride the manifest ([Project structure and rut.jsonc](project-structure.md)). A `type = "host"` package has no body: its `.d.rut` surface is itself a module tree — `mod NAME;` edges mount `NAME/mod.d.rut` children beside the declaring file, under `src/` (or the single-file `NAME.rut`), recursively — and each row registers path-qualified (`myhost::net::connect`; a flat pkg's rows keep today's bare shape). `rut_native::load_dir` walks `deps` recursively — cycle guard, first-mount-wins, name-mismatch is an error — gates every url dep against the root's `rut.lock` (an unpinned url refuses, naming `rut install`) and verifies the fetched bytes against the lock's pin at the mount door, then runs the root-only `[dev-deps]` pass ([Dependency kinds](dependency-kinds.md)). A url dep's kind routes its lane: a `.zip` is the SOURCE transport — unzipped (CRC verified in-read, DEFLATE accepted, zip64 refused), unpacked ONCE content-addressed at `<project>/.rut/unpacked/<sha-of-bytes>/` (atomic, zip-slip-guarded), and mounted as an ordinary directory package — its rewritten dep rows resolve inside the unpacked tree; a dep dir there holding a `.rutbundle` file mounts through the wire lane, the dirname's fingerprint verified against the content. A `.rutbundle` loads closed from the zip — groups mounting under their content identities ([Module bundles](bundles.md)). Both kinds are closed: the walk never follows a packed artifact's own url rows. The yield is `rut_driver::Walk { pkgs, root }`. |
| 2. the offer | `pkg(..)` — the ONE offering verb — offers every walked pkg (first-pkg-wins; fold the walk into the chain) and `Offer::Host` hands over the host bodies. `core` auto-rides unless a pkg named `core` was offered; the closure check refuses a `use` name nothing offered resolves — and a head that IS mounted but undeclared by its consumer (nameability is declaration-scoped: `deps` ∪ root `dev-deps` ∪ `core`; [Dependency kinds](dependency-kinds.md)). |
| 3. compile the graph | `.compile()` mounts the offers into the run's internal table, then compiles each module (sources, in dependency post-order) — every module links; there is no splice lane and nothing appends after the walk. Every use path resolves through the run's one resolver door, so a mounted-but-undeclared head is the dedicated D5 diagnostic naming the mount edge and the fix; host packages synthesize bodyless thunks from their declared surfaces; a bundle's compiled packages push their decoded binaries at fresh, rebased scopes ([The compiler pipeline](compiler.md)). |
| 4. link + flatten | Module-local type/function/const ids rebase into the global tables; the shared boot prefix passes through; name tables merge; interface declarations merge by name and duplicate module names are link errors. Cyclic use is a compile-graph error, never a runtime event. |
| 5. verify + boot | The load verifier re-checks every function ([Module binary and verification](module-binary.md)); `Vm::builder().compiled(compiled).build()` joins the program's host thunks against the `Compiled`'s `HostRegistry` — a declared-but-unbound host fn is a boot error. |

Loading a `.rutc` binary (`vm.load_binary`-style paths, and the wasm
runner) skips steps 2–3 and lands directly in verify + boot.

## What the host can call

Callable names are exactly the program's **`entry fn`s**. Their
signatures were checked against the host-crossing rule at
compile time — primitives, `str`, `bytes`, `opaque`, `?T` over a crossing
type, and crossing tuples — so a bad surface can never surprise the
embedder at call time. An `entry fn -> (?T, err)` decodes positionally at
`vm.call` as a `(value, err)` pair (below).

Use paths never execute anything: importing a package mounts its
surface; only the host's calls run code.

## The embedder surface

Everything the host needs is the chain plus a handful of `Vm` methods
(the registries build before the `Vm` boots):

| call | purpose |
|---|---|
| `rut_native::load_dir(dir)` / `load_dir_with(dir, &remote)` | the walk: a directory's whole closure as `rut_driver::Walk` |
| `rut_native::load_bundle_session(path)` / `Bundle::parse(&bytes)` | load a packed `.rutbundle` from a file / decode its bytes into a reusable handle |
| `rut_native::dir_pkgs(dir)` / `tree_pkg(name)` | the offer lanes: a dep-style directory walk / one toolchain-tree package |
| `HostRegistry::register(name, f)` | bind a host-fn body on the raw lane; signatures derived from the Rust shape |
| `pkg_fn!` / `pkg_async_fn!` | the `HostPkg` builder's sugar — one spelling → one row / the five-row async family ([embedding and native modules](embedding.md)) |
| `RutRun::new().pkg(..).entrypoint(..).compile()` | the run chain: offers (`Offer::Lib(&bundle)` / `Offer::Host(hp)`), the root — one terminal step compiles (`Result<Compiled, RunError>`) |
| `bundle.restore_symbols(&map)` | restore a [stripped artifact's](symbol-stripping.md) private symbol table on the decoded archive handle — once, before any offer |
| `Vm::builder().compiled(c).limits(l).hooks(h).build()` | boot; joins every declared host fn to its binding (a declared-but-unbound fn is a construction error) |
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
    self.rut.run_ready()?;                      // drive ready frames to completion
    if let Some(d) = self.rut.next_deadline() { // earliest armed sleep
        self.schedule_wake(d);                  // host parks until then,
    }                                           // then advances the clock
    self.render_frame();
}
```

`pending_tasks()` is the idle test — zero means the program has nothing
runnable. There is no job executor inside the VM: parking, waking, and
timing are these three verbs plus `set_now`
([Async and await](async.md)). A program whose world offers no async
packages simply has no launcher; `await` stays cold-poll inline
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
