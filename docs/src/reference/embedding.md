# Embedding and native modules

The host is a Rust program. It walks the program's packages off disk (or
offers them by hand), composes **one run** fluently, binds native
function **bodies** to declared **surfaces**, and drives rut entry
points. One value flows end to end: **`Pkg` → `RutRun` → `Compiled` →
`Vm`**. Compiling and type-checking rut code never requires any Rust:
surfaces are declared in rut source ([host fns and declaration
files](host-fns.md)), and the boot join proves every referenced native
member has a bound, signature-equal implementation before the first
instruction runs.

There is no load-time execution: composing and compiling verify and
link; the host runs entry points explicitly.

## Getting the crates

The engine ships as **git dependencies** — the crates are not on
crates.io. Name the repository once per crate, every crate on the
**same** `rev` (cargo then resolves them to one checkout):

```toml
[dependencies]
rut-driver = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-native = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-core   = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-parser = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-vm     = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
```

`rut-native` is the walk and the world — everything with a filesystem
or a wire ([the walk vs the offer](#the-walk-vs-the-offer)). A host
that offers every
`Pkg` by hand (a wasm page, an in-memory test) depends on the driver
alone and skips it. The build needs the nightly this repo pins
(`rut-vm-threaded` uses incomplete features) — your project's
`rust-toolchain.toml` carries the pin. Add `rut-std` the same way when
the program uses std packages; the full walk-through (hash lookup,
nightly pin, a runnable smoke test) is
[installation](../quick-start/installation.md).

## The embed loop

```rust
// 1. The walk (rut-native): a directory's whole closure as walked
//    pkgs — `[deps]` recursion, the sha256 pins, the peer gate.
let loaded = block_on(rut_native::load_dir_with(
    std::path::Path::new("plugins/server"),
    &rut_native::HttpRemote::project_local(project_dir),
))?;

// 2. The chain (rut-driver): offer the walk's yield, hand over the
//    host bodies, name the root — and ONE terminal step compiles.
let compiled = rut_driver::RutRun::new()
    .pkgs(&loaded)                                     // first-pkg-wins
    .host_pkg(rut_std::math::pkg())                    // calc's bodies
    .host_pkg(rut_std::logger::pkg(|s| println!("{s}")))
    .entrypoint(&loaded.root)
    .compile()?;
if !compiled.graph.diags.is_empty() { /* render and exit */ }
rut_vm::verify::verify(compiled.graph.program.as_ref().unwrap())?;

// 3. Boot and drive — `Vm::builder()` is the only construction door.
let limits = rut_vm::interp::Limits {
    fuel: Some(1_000_000),
    heap_limit_bytes: Some(64 * 1024 * 1024),
    interrupt_every: 1024,
};
let mut vm = rut_vm::interp::Vm::builder()
    .compiled(compiled)          // the run's product; also .limits/.hooks/.hosts
    .build()?;
let answer: i64 = vm.call("compute", (41,))?;
```

A hand-rolled pkg for your own host rows rides the same chain step
(the raw `register!` escape hatch on the bare registry — same
machinery, flat names — stays for hosts that bind outside the chain):

```rust
let mut server = rut_vm::HostPkg::new("server");
rut_vm::pkg_fn!(server, "emit", (OpaqueRef, &str, &str) -> (),
    |vm: &mut rut_vm::interp::Vm, bus, topic, payload| -> Result<(), rut_vm::Trap> {
        // re-entrant rut calls are legal here (see "Native fn rules")
        Ok(())
    });

let compiled = rut_driver::RutRun::new()
    /* ..offers.. */
    .host_pkg(server.build())
    .entrypoint("app")
    .compile()?;
```

### The `.compile()` law

`RutRun::compile()` is the single terminal step. In order:

1. **auto-offer the core prelude** — unless a pkg named `core` was
   offered (first-pkg-wins override);
2. **close the world** — every offered pkg mounts into the run's
   internal table, then the mount-by-mount world runs its ONE
   peer-gate append pass (a walk's yield already carried its groups;
   the pass is idempotent per pkg);
3. **the host registry** — every `.host_pkg(..)` installs against the
   mounted rows snapshot. A mounted scope whose declared rows the
   installer does not bind is the loud panic (the `.d.rut` ↔ host-impl
   contract); an unmounted scope's rows merge inert — an embedder
   wiring bug is never a rut diagnostic;
4. **the closure check** — every `use` name in every offered source
   resolves; a miss is `Err(RunError)`, carrying the resolver's own
   diagnostic;
5. **parse → check → LIR → link.**

Shape/closure failures (a bad package name, a missing entrypoint, an
unresolved use) are `Err(RunError)`. Compile diagnostics are NOT
errors: they ride inside `Compiled::graph` exactly as they always did —
check `compiled.graph.diags` before trusting `compiled.graph.program`.

The registry is consumed by `.build()`: every host thunk the program
declares is resolved against it once, at boot. A declared-but-unbound
fn is a **construction error**, never a mid-run trap.

## The walk vs the offer

Two doors produce `Pkg`s; both feed the same chain.

**The walk** ([`rut-native`](#getting-the-crates)) reads a module
directory — or a packed `.rutbundle` — and yields the closure:

| lane | what it does |
|---|---|
| `load_dir(dir)` | the walk: `[deps]` recursion (cycle guard, first-mount-wins, name-mismatch is an error), the `sha256` pin verified at the mount door, the root-only `[dev-deps]` pass, ONE peer-gate pass over the closed set. Url deps are the loud no-fetcher error. |
| `load_dir_with(dir, &remote)` | the same walk; url `deps` rows are collected, fetched over the host's `DepRemote`, and pinned at the mount door |
| `load_dir_fetched(dir, &map)` | the walk over PRE-fetched url bytes — the sync core; tests and offline hosts hand a `url → bytes` map |
| `load_path_session(path)` / `load_path_session_with(path, &remote)` | dispatch on the shape: a directory walks (above); a `.rutbundle` loads closed (below) |

The yield is `rut_driver::Loaded { pkgs, root }` — pure pkgs plus the
root's name. Offer it with `.pkgs(&loaded)` (every pkg,
first-pkg-wins) or pick pieces with `loaded.pkg(name)`.

**The offer** is a `Pkg` value handed to the chain by hand:

| constructor | builds |
|---|---|
| `Pkg::source(name, text)` | the ordinary `.rut` body |
| `Pkg::decl(name, text)` | a `.d.rut` surface (declaration mode) |
| `Pkg::host(name, rows)` | the native rows — exported constants, bodyless host fns |
| `Pkg::compiled(name, program)` | a decoded `.rutc` binary pushed as-is |
| `Pkg::from_bundle(bytes)` | the pure container parse — the bundle's pkgs as a `Loaded` (below) |
| `lower_decl_module(text, file)` | a `.d.rut` lowered to host rows — the wasm hosts' surface lane |

`Pkg` is pure data: the walker-found metadata (the `[deps]` table, the
`[peer-deps]` declarations, the peer-integration group texts) is pub
data on it, and a host whose world has no filesystem (wasm) appends
peer groups by hand — the graph only reads.

Two offer lanes walk a directory **for someone else's program**:

| lane | what it does |
|---|---|
| `rut_native::dir_pkgs(dir)` / `dir_pkgs_with(dir, &remote)` | offer one package directory — and, recursively, its `[deps]` — as walked pkgs. No dev-deps pass, no peer gate: presence is the program's own `.compile()` law. |
| `rut_native::tree_pkg(name)` | one toolchain-tree package (`rut/calc`, `rut/futures`, …). A pkg whose `[deps]` pull a closure is refused — offer the whole walk (`load_dir`) instead, never a silently-truncated world. |

`core` is never offered — `.compile()` auto-rides it. `calc` and the
async pair are ordinary tree packages: the host offers
`rut_native::tree_pkg("calc")` when the program's surface needs it and
binds `rut_std::math::pkg()` for the bodies ([core and the swappable
packages](stdlib.md)).

## Url deps — the remote policy (`DepRemote` + `HttpRemote`)

The embedder owns exactly three things: the fs policy ([`Source`]),
the remote policy ([`DepRemote`]), and its own runtime. The `_with`
lanes take `&dyn DepRemote` and nothing else: the remote matters only
when the dep walk actually reaches a url row. Url-free projects and
`.rutbundle`s are closed — they load with no remote named at all.

The standard remote is **`rut_native::HttpRemote`** — cache-first: a
hit NEVER touches the network, a miss GETs (redirects on, loud status
errors, a size cap) on the remote's own private worker thread and
writes the cache back atomically. Cache layout law:
`<root>/<sha256(url)>.rutbundle`. The futures come back READY, so any
executor works — including the std-only noop-waker `block_on` poll
loop the examples spell.

| constructor | behavior |
|---|---|
| `HttpRemote::project_local(project)` | `<project>/.rut/cache` — hermetic, project-local; networks on a miss (the `http` feature, default on) |
| `HttpRemote::at(root)` | an explicit cache root, wire on |
| `HttpRemote::offline(root)` | **cache-only** — a miss never networks; the guaranteed-offline lane (CI gates, wasm-adjacent hosts) |
| `path_for(url)` / `evict(url)` | the concrete entry path / delete a poisoned entry |

**Guaranteed-offline loads** (tests, CI, wasm-adjacent hosts): prime an
`offline` remote, then load over it. The committed std artifacts play
the wire — `DepRemote::write` is the stand-in for the GET:

```rust
let remote = rut_native::HttpRemote::offline(&cache_root);
for (url, bytes) in vendored_rows {                  // parsed from rut.jsonc
    rut_native::DepRemote::write(&remote, &url, &bytes)?;  // prime the cache
}
let loaded = block_on(rut_native::load_dir_with(project_dir, &remote))?;
```

**Memory hosts** (wasm, in-process tests) hand-roll the trait — one
required method, sync impls are first-class via `std::future::ready`:

```rust
struct Mem(Vec<(String, Vec<u8>)>);

impl rut_native::DepRemote for Mem {
    fn fetch(&self, url: &str)
        -> Pin<Box<dyn Future<Output = Result<Vec<u8>, rut_native::RemoteError>> + '_>> {
        Box::pin(std::future::ready(
            self.lookup(url).ok_or_else(|| rut_native::RemoteError::new(
                format!("no bytes for {url}")))))
    }
    fn lookup(&self, url: &str) -> Option<Vec<u8>> {
        self.0.iter().find(|(u, _)| u == url).map(|(_, b)| b.clone())
    }
    fn write(&self, url: &str, bytes: &[u8]) -> Result<(), rut_native::RemoteError> {
        self.0.retain(|(u, _)| u != url);
        Ok(self.0.push((url.to_string(), bytes.to_vec())))
    }
}
```

The walk still owns WHAT the bytes are: the `sha256` pin is manifest
law, verified at the mount door on every load — fresh fetch, cache
hit, vendored map, test fixture.

**The filesystem policy** is `rut_native::Source` — string keys, all
key math in the impl. `FsSource::at(dir)` is the real filesystem with
the root baked in; a `Path` lives only inside its impls. A test or a
memory host answers `read`/`resolve` with lookups instead of files.
The `load_dir*` lanes bake `FsSource` in; a custom `Source` hosts the
walk through `dir_pkgs`-shaped collectors of your own.

## Module bundles

A packed `.rutbundle` is the same contract zipped — loading it never
fetches (bundles are closed; their closure rode inside at pack time):

```rust
// from a file: the walk adds only the file read
let loaded = rut_native::load_bundle_session(Path::new("vendor/plugin.rutbundle"))?;

// from bytes (embedders, tests, wasm): the pure container parse
let loaded = rut_driver::Pkg::from_bundle(&bytes)?;
```

Both root kinds load (a compiled root with its groups and its
pack-time scope ledger, rebased at the close of the world; a decl
root as the pkg's host rows), first-mount-wins, and the bundle root's
package name is `loaded.root`. Wasm hosts `include_bytes!` the
committed artifact and parse through `Pkg::from_bundle`. For the
url-dep *walk* (pins, closure checks, the peer gate over archive
groups) stay on the `load_dir*` lanes — a bundle mount is the offer,
not the walk. The container, the manifest grammar, and the reader are
[module bundles](bundles.md).

**What to take from a url** is the engine's instantiation law, read
from the embedding side: a compiled bundle serves host surfaces,
concrete-class libs, **and** — by the generic-source riding law —
the generic owners: a request the pack-time ledger lacks lowers the
ridden source in the consumer's world and compiles the monomorphized
body under the declaring pkg's spec, at the link, nothing persisted
(`Vec<MyTodo>`, `decodeJson<T>` — [module bundles](bundles.md) — the
std-CDN section). A bundle that owns an open generic surface but
carries no riding source refuses at the mount (`pouch` owns an open
generic surface but its bundle carries no riding source — re-pack the
directory), and a bundle-mounted json names its
pack-time dev closure in its ledger, so the consumer's closure must
contain those names.

## The walk API (`rut-native`)

| API | Meaning |
|---|---|
| `Source` | the one filesystem door, STRING keys: `read(key)`, `resolve(from, spec)`, `root()` — the key math is the impl's; a `Path` crosses nothing |
| `FsSource::at(dir)` | the real filesystem with the root baked in |
| `DepRemote` | the url-dep contract: `fetch(&self, url) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, RemoteError>> + '_>>` (required), `lookup`, `write` — the call site owns HOW bytes arrive (transport, cache, offline policy). Dyn-compatible (hand-rolled boxing, no async-trait), deliberately **not** `+ Send`: a browser `fetch` bridge is `!Send`, and sync impls (cache hits, fixtures) are first-class |
| `HttpRemote` | the standard remote: cache-first (`<root>/<sha256(url)>.rutbundle`), a miss GETs on a private worker thread and writes back atomically; `project_local` / `at` wire on, `offline` is cache-only, `path_for`/`evict` are concrete |
| `load_dir(dir)` / `load_dir_with(dir, &remote)` / `load_dir_fetched(dir, &map)` | the walk and its fetched lanes — the yield is `rut_driver::Loaded` ([the walk vs the offer](#the-walk-vs-the-offer)) |
| `load_path_session(path)` / `load_path_session_with(path, &remote)` | dispatch on the shape: directory walk or closed `.rutbundle` |
| `load_bundle_session(path)` | load a packed `.rutbundle` from disk — the pure container lane plus the file read |
| `dir_pkgs(dir)` / `dir_pkgs_with(dir, &remote)` | the OFFER lane: a package directory walked for someone else's program (no dev pass, no gate) |
| `tree_pkg(name)` | one toolchain-tree package (`rut/<name>`); a `[deps]` closure is refused |
| `load_module_source(path)` | one `.rut` file's text — a loose file is a single module, no manifest |
| `prefetch_urls(&src, &dir, &remote)` | collect a manifest's url rows and fetch them — the `_with` lanes' input step |
| `pack_dir(dir)` / `pack_dir_with(dir, &remote)` / `pack_dir_opts(..)` family | pack a module directory into a deterministic `.rutbundle` — a **compiled** root for a lib pkg, a **decl** root for a host pkg (`type` routes; `format_version` is 10); returns the bytes ([module bundles](bundles.md)) |
| `default_out_path(dir)` | the conventional pack output path |

## The run chain API (`rut-driver`)

| API | Meaning |
|---|---|
| `RutRun::new()` | the chain's head; compose with the chain methods, end at `.compile()` |
| `.pkg(pkg)` / `.pkgs(&loaded)` | offer — FIRST OFFER WINS: a pkg whose name is already offered is ignored |
| `.host_pkg(host_pkg)` | hand over one host pkg's bodies; installed at `.compile()` against the rows snapshot |
| `.symbols(&map)` | restore a stripped artifact's symbol table into the offered compiled pkgs ([symbol stripping](symbol-stripping.md)) |
| `.entrypoint(name)` | the root pkg — the graph compiles it and its transitive uses |
| `.compile() -> Result<Compiled, RunError>` | the terminal step ([the law](#the-compile-law)) |
| `Compiled { graph, hosts }` | the linked graph (diags inside — check `graph.diags` before trusting `graph.program`) and the host registry every `.host_pkg(..)` installed |
| `Loaded { pkgs, root }` | a walk's yield; `loaded.pkg(name)` picks one pkg |
| `Pkg::source / decl / host / compiled / from_bundle` | the offer constructors ([the offer](#the-walk-vs-the-offer)) |
| `RunError` | why a run refused to compose or compile — shape (bad name, missing entrypoint) or closure (a `use` nothing offered resolves); the message IS the diagnostic |
| `declared_host_fns(pkgs)` | the declared rows, async-expanded, flattened — the raw lane's `verify_against` input |
| `host_pkg_ctx(pkgs)` | the declared rows, partitioned by scope — for hosts that build registries OUTSIDE the chain (a bench probe's fresh `Vm` per iteration, a raw-row host's `verify_against`) |
| `compile_module(src, mode, name)` | the single-source lane: the core prelude auto-rides, the source offers as the root pkg, one `.compile()` — returns diags, AST/IR dumps, and the binary. The walked packages (`calc`'s `Math`, the async pair) are NOT here: a host that wants them offers `rut_native::tree_pkg(..)` |
| `compile_program(src, mode, name)` / `compile_program_resolved(..)` / `ir_dump_of(..)` | the raw pipeline pieces (tooling) |
| `lower_decl_module(text, file)` | lower a `.d.rut` surface to host rows |
| `bundle::{parse_manifest, Bundle, Layout, ..}` | the container codec, the `rut.jsonc` grammar, the reader (both root kinds) — pure, in-memory |
| `pack::{pack, PackOpts, ..}` | the pure emit half of packing (`rut-native` owns the walk + dev tables + the gate) |
| `sha256_hex(bytes)` | pin arithmetic (pure) |

`mode` is `Mode::Impl` for `.rut` and `Mode::Decl` for `.d.rut`
([host fns and declaration files](host-fns.md)).

The driver is provably pure: no fs, no net, no walk, no `std::path`
anywhere in `rut-driver` — everything with a filesystem or a wire
lives in `rut-native`.

## VM API (`rut_vm::interp`)

| API | Meaning |
|---|---|
| `Vm::builder()` | the construction door: `.compiled(x)` (anything `IntoVmParts` — the driver's `Compiled`) or `.program(rc)` for embedders holding a decoded binary, each plus `.limits(l)` / `.hooks(h)` / `.hosts(registry)`, ending at `.build() -> Result<Vm, VmError>`. Unset limits ⇒ `Limits::default()` — uncapped, the mechanism law; explicit setters win over the compiled parts |
| `HostRegistry::new()` | an empty binding table (the raw lane) |
| `HostPkg::new(scope)` | a pkg builder; rows register under bare names, the scope prefixes at the install |
| `pkg_fn!` / `pkg_async_fn!` | the builder's sugar: one spelling → one row / the five-row async family |
| `hosts.register::<_, (P…), R, _>(name, f)` | the raw lane: bind one body under a full `scope::name` string |
| `hosts.verify_against(&expected)` | the raw lane's net: panic on declared-unbound / bound-undeclared / signature drift |
| `vm.call::<A, R>(export, args)` | call an export with Rust values, get a Rust value back ([value boundary](value-boundary.md)) |
| `vm.resume::<R>()` | resume a budget-parked call after refueling |
| `vm.run_ready()` | drain the async ready queue once; returns frames run |
| `vm.next_deadline() -> Option<u64>` | the earliest timer deadline, if any |
| `vm.set_now(t_ms)` | advance the virtual clock |
| `vm.pending_tasks() -> usize` | unfinished async work |
| `vm.fuel_used` / `vm.heap_usage()` | budget meters |
| `vm.alloc_opaque_str(s)` | mint an `opaque` box over host-built text (the logger's named logger) |
| `vm.call_host_row(row, &[Value])` | dispatch a registered row by name (test/tooling reads) |

The canonical async driving loop (single-threaded; the host owns it):

```rust
loop {
    vm.run_ready()?;
    match vm.next_deadline() {
        Some(d) => vm.set_now(d),          // advance to the next timer
        None if vm.pending_tasks() == 0 => break,
        None => std::thread::sleep(std::time::Duration::from_millis(2)),
    }
}
```

Cap the loop in embedders that cannot prove termination, so a program that
never idles fails loudly instead of hanging.

## Limits

```rust
pub struct Limits {
    pub fuel: Option<u64>,          // ops per turn; None = unlimited
    pub heap_limit_bytes: Option<u64>,
    pub interrupt_every: u32,       // ops between interrupt checks
}
```

Exhaustion and interrupts surface as traps ([resource limits](resource-limits.md)).
A trap inside re-entrant work propagates to the embedder parked at the host
op; `vm.resume::<R>()` re-runs the turn.

## Native fn rules

- A body is `FnMut(&mut Vm, P…) -> R` or `-> Result<R, Trap>`; the Rust
  parameter types **are** the declared row (compile error at the register
  site for a type that does not cross — [value boundary](value-boundary.md)).
- Native fns run **outside** the op budget. The host is trusted to be fast —
  or to hand the work to a future ([async host fns](host-fns.md)).
- Re-entrancy: a body may call `vm.call` on rut exports. Nested calls draw
  the same fuel pool; borrows held by the outer body are guarded
  ([value boundary](value-boundary.md)).
- Failures are values: return `Err(Trap)` — `Trap { kind, msg }`. The fn
  traps cleanly and the error propagates as an `Err` to the embedder with
  the rut backtrace intact. Traps never unwind Rust.

| `TrapKind` | raised by |
|---|---|
| `OutOfFuel` | budget exhaustion |
| `OutOfMemory` | heap limit |
| `Interrupted` | interrupt flag |
| `Overflow` / `DivByZero` | arithmetic |
| `IndexOutOfBounds` | sequence access |
| `Panic` | `panic` |
| `BadUnbox` | a failed unbox |
| `NilDeref` | nil where a reference is required |
| `Invalid` | everything else, including boundary mismatches |

## The engine's own natives

The VM boots with internal native calls in the same dispatch family — the
`str`/`bytes` members, the `f"..."` concatenation lowering, array
length/slice, and stack-trace capture. They are call slots
fixed at boot, never IR-level special forms; hosts see them exactly like
their own registered modules.

## Host-side helpers (`rut-std`)

One `pkg()` builder per module; each rides the chain —
`.host_pkg(rut_std::math::pkg())` — and installs at `.compile()`
against the rows snapshot (see [host fns](host-fns.md) for the
asymmetric contract — an unmounted scope merges inert, so hosts
blanket-install their subset):

| builder | binds |
|---|---|
| `math::pkg()` | `calc`'s float functions, both widths |
| `logger::pkg(sink)` | `ink_host`'s two rows, routed to a `FnMut(&str)` sink |
| `nmap::pkg()` | the native key table behind `nmapset` ([stdlib](stdlib.md)) |
| `strbuild::pkg()` | the string builder's rows behind `strbuild` ([stdlib](stdlib.md)) — growth is charged against the embedder's heap budget |
| `async_host::pkg()` | the launcher rows (`__launch`/`__abort`/`__sleep`/`__sleep_yield`) |
| `http::pkg()` | the std HTTP lanes (reqwest; native builds only); `http::pkg_with(f)` is the fixture twin, returning `(HostPkg, HttpFixture)` |
| `bench_cross::pkg()` | the crossing-tax benchmark rows |

## In-tree examples

- [03 — Plugin](../examples/03-plugin.md): a Rust chat server driving a rut
  moderator plugin, loaded from a module directory and from a packed
  `.rutbundle`; re-entrant `emit` crossings.
- [06 — GitHub viewer CLI](../examples/06-github-viewer-cli.md): walks the
  std packages, binds example-local I/O rows, launches an async entry fn,
  and pumps the driving loop to idle.
