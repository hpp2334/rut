# Embedding and native modules

The host is a Rust program. It owns the VM, mounts packages, binds native
function **bodies** to declared **surfaces**, and drives rut entry points.
Compiling and type-checking rut code never requires any Rust: surfaces are
declared in rut source ([host fns and declaration files](host-fns.md)), and
the boot join proves every referenced native member has a bound,
signature-equal implementation before the first instruction runs.

There is no load-time execution: loading verifies and links; the host runs
entry points explicitly.

## Getting the crates

The engine ships as **git dependencies** — the crates are not on
crates.io. Name the repository once per crate, every crate on the
**same** `rev` (cargo then resolves them to one checkout):

```toml
[dependencies]
rut-driver = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-core   = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-parser = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
rut-vm     = { git = "https://github.com/hpp2334/rut.git", rev = "<commit-hash>" }
```

The build needs the nightly this repo pins (`rut-vm-threaded` uses
incomplete features) — your project's `rust-toolchain.toml` carries the
pin. Add `rut-std` the same way when the program uses std packages; the
full walk-through (hash lookup, nightly pin, a runnable smoke test) is
[installation](../quick-start/installation.md).

## The embed loop

```rust
use std::rc::Rc;

// 1. Mount the packages the program uses. core + calc are the base.
let mut session = rut_driver::Session::new();
rut_driver::mount_std(&mut session);        // core + calc
rut_driver::mount_std_async(&mut session);  // async_engine + async_host (optional)
rut_driver::mount_dir(&mut session, "plugins/server")?;
rut_driver::assemble_peers(&mut session)?;  // peer-gated impl groups

// 2. Compile the program against the mounted surfaces.
let out = rut_driver::compile_module_in(&mut session, &src,
                                        rut_parser::Mode::Impl, "app");
if !out.diags.is_empty() { /* render and exit */ }
let prog = rut_core::binary::decode(&out.binary.unwrap())?;
rut_vm::verify::verify(&prog)?;

// 3. Snapshot the mounts, then bind bodies BEFORE the Vm exists —
//    the registry is a pre-VM table and the ctx is the declared side.
let ctx = session.host_pkg_context();
let mut hosts = rut_vm::interp::HostRegistry::new();
hosts.install_host_pkg(&ctx, rut_std::math::pkg());
hosts.install_host_pkg(&ctx, rut_std::logger::pkg(|s| println!("{s}")));
// a hand-rolled pkg for your own host rows (or the raw `register!`
// escape hatch on the bare registry — same machinery, flat names)
let mut server = rut_vm::HostPkg::new("server");
rut_vm::pkg_fn!(server, "emit", (OpaqueRef, &str, &str) -> (),
    |vm: &mut rut_vm::interp::Vm, bus, topic, payload| -> Result<(), rut_vm::Trap> {
        // re-entrant rut calls are legal here (see "Native fn rules")
        Ok(())
    });
hosts.install_host_pkg(&ctx, server.build());
// The decl ↔ impl contract check. Panics, loudly, on any mismatch —
// an embedder wiring bug is never a rut diagnostic.
hosts.verify_against(&ctx.flatten());

// 4. Boot and drive.
let limits = rut_vm::interp::Limits {
    fuel: Some(1_000_000),
    heap_limit_bytes: Some(64 * 1024 * 1024),
    interrupt_every: 1024,
};
let mut vm = rut_vm::interp::Vm::new(
    Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts,
)?;
let answer: i64 = vm.call("compute", (41,))?;
```

The compile step is the **embedder's string lane**: the host owns the
source text — from its own config, a database, an editor buffer — and
`compile_module_in` compiles it against the mounted surfaces. It is not
a file-running lane: the CLI's `rut run` accepts only a module directory
or a packed `.rutbundle` ([the rut CLI](cli.md)).

The registry is consumed by `Vm::new`: every host thunk the program declares
is resolved against it once, at boot. A declared-but-unbound fn is a
**construction error**, never a mid-run trap.

## Url deps — the remote policy (`Loader` + `HttpRemote`)

The embedder owns exactly three things: the fs policy ([`Source`]),
the remote policy ([`DepRemote`]), and its own runtime. The door is
the **`Loader`** — non-generic, no remote default, and the check is
LAZY: `build()` validates only the path shape; the remote matters only
when the dep walk actually reaches a url row, and the panic fires
there (naming the dep, the url, and the fix). Url-free projects and
`.rutbundle`s are closed — they load with no remote named at all.

```rust
// the ENTIRE embedder load half for a project with url rows
let app = rut_driver::Loader::new(project_dir)
    .dep_remote(rut_driver::HttpRemote::project_local(project_dir))
    .build();
let (mut session, root) = block_on(app.load())?;   // Result<_, rut_driver::LoadError>
```

The standard remote is **`HttpRemote`** — cache-first: a hit NEVER
touches the network, a miss GETs (redirects on, loud status errors, a
size cap) on the remote's own private worker thread and writes the
cache back atomically. Cache layout law: `<root>/<sha256(url)>.rutbundle`.
The futures come back READY, so any executor works — including the
std-only noop-waker `block_on` poll loop the examples spell.

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
let remote = rut_driver::HttpRemote::offline(&cache_root);
for (url, bytes) in vendored_rows {                  // parsed from rut.json
    rut_driver::DepRemote::write(&remote, &url, &bytes)?;  // prime the cache
}
let (mut session, root) =
    block_on(rut_driver::Loader::new(project_dir).dep_remote(remote).build().load())?;
```

**Memory hosts** (wasm, in-process tests) hand-roll the trait — one
required method, sync impls are first-class via `std::future::ready`:

```rust
struct Mem(Vec<(String, Vec<u8>)>);

impl rut_driver::DepRemote for Mem {
    fn fetch(&self, url: &str)
        -> Pin<Box<dyn Future<Output = Result<Vec<u8>, rut_driver::RemoteError>> + '_>> {
        Box::pin(std::future::ready(
            self.lookup(url).ok_or_else(|| rut_driver::RemoteError::new(
                format!("no bytes for {url}")))))
    }
    fn lookup(&self, url: &str) -> Option<Vec<u8>> {
        self.0.iter().find(|(u, _)| u == url).map(|(_, b)| b.clone())
    }
    fn write(&self, url: &str, bytes: &[u8]) -> Result<(), rut_driver::RemoteError> {
        self.0.retain(|(u, _)| u != url);
        Ok(self.0.push((url.to_string(), bytes.to_vec())))
    }
}
```

The loader still owns WHAT the bytes are: the `sha256` pin is manifest
law, verified at the mount door on every load — fresh fetch, cache
hit, vendored map, test fixture.

**Mounting a bundle into an existing session** — the in-memory
counterpart of `mount_dir` (the offer law: no dev-deps, no gate, the
compile owns presence) — is
`rut_driver::mount_bundle_bytes(&mut session, &bytes) -> Result<String, String>`:
both root kinds mount (a v7 compiled root with its groups, the ledger
namespaced into the session; a v8 decl root as the pkg's host rows),
first-mount-wins, and the bundle root's package name comes back. Wasm
hosts `include_bytes!` the committed artifact and mount through this —
05-todolist-web's mirror lane takes the `nmap_host` surface from the
CDN artifact that way. For the url-dep *walk* (pins, closure checks,
the peer gate over archive groups) stay on the load/pack lanes —
`mount_bundle_bytes` is the offer, not the walk.

**What to take from a url** is the engine's instantiation law, read
from the embedding side: a compiled bundle serves host surfaces,
concrete-class libs, **and** — since the generic-source riding law —
the generic owners: a request the pack-time ledger lacks lowers the
ridden source in the consumer's session and compiles the monomorphized
body under the declaring pkg's spec, at the link, nothing persisted
(`Vec<MyTodo>`, `decodeJson<T>` — [module bundles](bundles.md) — the
std-CDN section). A legacy bundle without the riding refuses such a
request loudly (re-pack it), and a bundle-mounted json names its
pack-time dev closure in its ledger, so the consumer's closure must
contain those names.

## Driver API (`rut-driver`)

| API | Meaning |
|---|---|
| `Session::new()` | an empty mounting session |
| `mount_std(&mut s)` | mount `core` + `calc` |
| `mount_std_async(&mut s)` | mount `async_engine` + `async_host` |
| `mount_dir(&mut s, dir)` | mount a package directory (`rut.json`); returns its name |
| `assemble_peers(&mut s)` | append peer-gated impl groups ([dependency kinds](dependency-kinds.md)) |
| `compile_module(src, mode, name)` | full pipeline over one module against a fresh core+calc session |
| `compile_module_in(&mut s, src, mode, name)` | the same against a caller-built session; returns diags, AST/IR dumps, and the binary |
| `compile_graph(&s, root)` | compile a whole module directory graph |
| `s.host_pkg_context()` | the mounted surfaces' declared rows, partitioned per pkg — the declared side `install_host_pkg` checks against (build once per boot lane; owned) |
| `s.expected_host_fns()` | the same rows flattened to one table — the raw lane's `verify_against` input |
| `load_path_session(path)` | load a module **directory** or `.rutbundle`; returns `(session, root)` |
| `Loader::new(project)` / `.source(&src)` / `.dep_remote(r)` / `.build()` / `loaded.load()` | THE embedder door: a directory or `.rutbundle`, the fs + remote policies explicit, one `async load()` — the no-remote check is lazy (it fires only when a url row is reached, as a panic naming the fix) |
| `DepRemote` | the url-dep contract: `fetch(&self, url) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, RemoteError>> + '_>>` (required), `lookup`, `write` — the call site owns HOW bytes arrive (transport, cache, offline policy). Dyn-compatible (hand-rolled boxing, no async-trait), deliberately **not** `+ Send`: a browser `fetch` bridge is `!Send`, and sync impls (cache hits, fixtures) are first-class |
| `HttpRemote` | the standard remote: cache-first (`<root>/<sha256(url)>.rutbundle`), a miss GETs on a private worker thread and writes back atomically; `project_local` / `at` wire on, `offline` is cache-only, `path_for`/`evict` are concrete |
| `load_path_session_with(path, &remote)` / `load_dir_session_with(dir, &remote)` | the fetched lanes: url deps in `deps` are collected, fetched, and pinned at the mount door ([dependency kinds](dependency-kinds.md), [module bundles](bundles.md)) |
| `mount_dir_with(&mut s, dir, &remote)` | mount a package directory with url deps — the offer law unchanged (no dev-deps, no gate) |
| `pack_dir_with(dir, &remote)` / `pack_dir_opts_with(dir, opts, remote)` | pack over fetched url deps; same determinism law (same manifest + same pins ⇒ byte-identical) |
| `pack_dir(dir)` | pack a module directory into a deterministic `.rutbundle` — a **v7 compiled** root for a lib pkg, a **v8 decl** root for a host pkg; returns the bytes ([module bundles](bundles.md)) |
| `mount_bundle_bytes(&mut s, &bytes)` | mount a bundle's contents into an existing session — the offer law over bytes (wasm hosts `include_bytes!` the committed artifacts) |

The container, manifest grammar, and reader (both root kinds) live in
the driver's `rut_driver::bundle` module — filesystem-free over a
one-method `Source` trait.

`mode` is `Mode::Impl` for `.rut` and `Mode::Decl` for `.d.rut`
([host fns and declaration files](host-fns.md)).

## VM API (`rut_vm::interp`)

| API | Meaning |
|---|---|
| `HostRegistry::new()` | an empty binding table |
| `HostPkg::new(scope)` | a pkg builder; rows register under bare names, the scope prefixes at the install |
| `pkg_fn!` / `pkg_async_fn!` | the builder's sugar: one spelling → one row / the five-row async family |
| `hosts.install_host_pkg(&ctx, pkg)` | install one built pkg: bind every row its mounted scope declares (drift/unbound ⇒ panic); an unmounted scope merges inert |
| `hosts.register::<_, (P…), R, _>(name, f)` | the raw lane: bind one body under a full `scope::name` string |
| `hosts.verify_against(&expected)` | the raw lane's net: panic on declared-unbound / bound-undeclared / signature drift |
| `Vm::new(prog, &limits, hooks, hosts)` | boot; joins every declared host thunk to its binding (a declared-but-unbound fn is a construction error) |
| `vm.call::<A, R>(export, args)` | call an export with Rust values, get a Rust value back ([value boundary](value-boundary.md)) |
| `vm.resume::<R>()` | resume a budget-parked call after refueling |
| `vm.run_ready()` | drain the async ready queue once; returns tasks run |
| `vm.next_deadline() -> Option<u64>` | the earliest timer deadline, if any |
| `vm.set_now(t_ms)` | advance the virtual clock |
| `vm.pending_tasks() -> usize` | unfinished async tasks |
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

One `pkg()` builder per module; each installs through
`HostRegistry::install_host_pkg` against the session's mount snapshot
(see [host fns](host-fns.md) for the asymmetric contract — an unmounted
scope merges inert, so hosts blanket-install their subset):

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
- [06 — GitHub viewer CLI](../examples/06-github-viewer-cli.md): mounts the
  std packages, binds example-local I/O rows, launches an async entry fn,
  and pumps the driving loop to idle.
