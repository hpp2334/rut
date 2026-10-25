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
//    pkgs — `[deps]` recursion, the rut.lock pins, the root-only
//    dev pass.
let loaded = block_on(rut_native::load_dir_with(
    std::path::Path::new("plugins/server"),
    &rut_native::HttpRemote::project_local(project_dir), // cache-only — prime first, or use online_local
))?;

// 2. The chain (rut-driver): offer the walk's yield, hand over the
//    host bodies, name the root — and ONE terminal step compiles.
//    `pkg(..)` is the ONE offering verb over the Offer classification;
//    `&Bundle` and `HostPkg` coerce in (`impl Into`).
let compiled = loaded
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone())) // first-pkg-wins
    .pkg(rut_std::math::pkg())                         // calc's bodies (Offer::Host)
    .pkg(rut_std::logger::pkg(|s| println!("{s}")))
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
    .pkg(server.build())
    .entrypoint("app")
    .compile()?;
```

**Rows under a surface's `mod` tree register scope-relatively.** A
`type = "host"` pkg may organize its surface as a module tree (`pub
mod net;` in the `.d.rut`, the rows in `net/mod.d.rut` — [project
structure](project-structure.md)); a row under `net/` declares as
`pkg::net::name` and its body registers under the path spelling
**without** the pkg scope — the scope prefixes once, at the install:

```rust
let mut http = rut_vm::HostPkg::new("http_host");
rut_vm::pkg_fn!(http, "net::http_status", (OpaqueRef,) -> i32,
    |vm, r| Ok(status_of(r)));                    // → http_host::net::http_status
rut_vm::pkg_async_fn!(http, "stream::http_body", (OpaqueRef,) -> Vec<u8>,
    |r| drain(r));                                // → http_host::stream::http_body (+ family)
// a bare spelling stays the ROOT row's shape ("map_new", not "net::map_new")
```

A path spelling for a row the surface declares flat (or vice versa)
fails the install's contract check by name — the declared rows are
the graph, the registration strings must spell the same tree.

### The `.compile()` law

`RutRun::compile()` is the single terminal step. In order:

1. **auto-offer the core prelude** — unless a pkg named `core` was
   offered (first-pkg-wins override);
2. **close the world** — every offered pkg mounts into the run's
   internal table; the walk's yield already carried its groups, and
   every module links — nothing appends after the walk;
3. **the host registry** — every `Offer::Host` installs against the
   mounted rows snapshot. A mounted scope whose declared rows the
   installer does not bind is the loud panic (the `.d.rut` ↔ host-impl
   contract); an unmounted scope's rows merge inert — an embedder
   wiring bug is never a rut diagnostic;
4. **the closure check** — every `use` name in every offered source
   resolves against the offered world AND its consumer's own manifest
   (nameability is declaration-scoped: `deps` ∪ root
   `dev-deps` ∪ `core`; a mounted-but-undeclared head is the dedicated
   D5 diagnostic, its host clause naming the builder doors for a hand
   offer); a miss is `Err(RunError)`, carrying the resolver's own
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
| `load_dir(dir)` | the walk: `[deps]` recursion (cycle guard, first-mount-wins, name-mismatch is an error), the root's `rut.lock` read fresh (an unpinned url refuses, naming `rut install`), the lock pins verified at the mount door, then the root-only `[dev-deps]` pass. Url deps are the loud no-fetcher error. |
| `load_dir_with(dir, &remote)` | the same walk; url `deps` rows are collected, gated against the root's `rut.lock`, fetched over the host's `DepRemote`, and pinned at the mount door |
| `load_dir_fetched(dir, &map)` | the walk over PRE-fetched url bytes — the sync core; tests and offline hosts hand a `url → bytes` map |
| `load_path_session(path)` / `load_path_session_with(path, &remote)` | dispatch on the shape: a directory walks (above); a `.rutbundle` loads closed (below) |

The yield is `rut_driver::Walk { pkgs, root }` — pure pkgs plus the
root's name. Offer each pkg through `pkg(..)` (first-pkg-wins — fold
the walk into the chain) or pick pieces with `walked.pkg(name)`.

**The offer** is ONE verb, `RutRun::pkg(..)`, over the `Offer`
classification — the same lib/host pair the manifest `type` field
spells:

| offer | what it is |
|---|---|
| `pkg(&bundle)` — `Offer::Lib(&Bundle)` | a decoded `.rutbundle` archive: `Bundle::parse(&bytes)` decodes + verifies ONCE (CRC, version, group decode), the handle is reusable across runs, and each offer only relocates + mounts |
| `pkg(rut_std::math::pkg())` — `Offer::Host(hp)` | host bodies — the embedding Rust's own rows |
| `pkg(pack_inline_pkg(name, text))` | ephemeral source as a one-member source bundle — a pack method that assembles, never compiles; the run compiles the member when it is offered |
| `pkg(mounted)` | the walked pkg data (deprecated bridge — offer bundles instead) |

An archive handle carries its strip restore too:
`bundle.restore_symbols(&map)` — applied once, before any offer; every
subsequent offer carries the restored names
([symbol stripping](symbol-stripping.md)).

The mounted pkg **data** behind the walk (the constructors tests and
hosts spell by hand) is `Mounted`:

| constructor | builds |
|---|---|
| `Mounted::source(name, text)` | the ordinary `.rut` body |
| `Mounted::decl(name, text)` | a `.d.rut` surface (declaration mode) |
| `Mounted::host(name, rows)` | the native rows — exported constants, bodyless host fns |
| `Mounted::compiled(name, program)` | a decoded `.rutc` binary pushed as-is |
| `Mounted::from_bundle(bytes)` | the pure parse — the bundle's pkgs as a `Walk` (below) |
| `lower_decl_module(text, children, file)` | a `.d.rut` lowered to host rows — the wasm hosts' surface lane; `children` is the mounted `mod` tree (`NAME/mod.d.rut` files, mount with `mount_mod_children` in Decl mode) and each row carries the path it lives under |

`Mounted` is pure data: the walker-found metadata (the `[deps]` table and
the declaration rows the nameability law reads) is pub data on it —
the graph only reads.

Two offer lanes walk a directory **for someone else's program**:

| lane | what it does |
|---|---|
| `rut_native::dir_pkgs(dir)` / `dir_pkgs_with(dir, &remote)` | offer one package directory — and, recursively, its `[deps]` — as walked pkgs. No dev-deps pass: the closure is the program's own `.compile()` law. |
| `rut_native::tree_pkg(name)` | one toolchain-tree package (`rut/calc`, `rut/futures`, …). A pkg whose `[deps]` pull a closure is refused — offer the whole walk (`load_dir`) instead, never a silently-truncated world. |

`core` is never offered — `.compile()` auto-rides it. `calc_host` and
the async pair are ordinary tree packages: the host offers
`rut_native::tree_pkg("calc_host")` when the program's surface needs it
(the `calc` lib beside it mounts through `dir_pkgs` — its `deps` pull
the host pkg) and binds `rut_std::math::pkg()` for the bodies
([core and the swappable packages](stdlib.md)).

### Hand mounts carry a manifest

A hand-offered world is lawed by the same declaration table as a walked
one — nameability is declaration-scoped, so a consumer may `use`
exactly the heads ITS manifest names — the manifest just travels
structurally instead of as a file. Two builder doors:

- **Real manifests** — parse the embedded jsonc and stamp it:
  `Mounted::manifest(parse_manifest(include_str!("../pkg/rut.jsonc")))`.
  The tree's own `rut.jsonc` stays the single source of truth; the wasm
  host mounts every std pkg this way.
- **Ad-hoc surfaces** — `Manifest::lib(name, &[heads])` stamps the
  structural form directly, no json text anywhere. 05's `web` package
  is the worked example: embedder-registered, no artifact, no path/url
  — a structural-only declaration of the heads its sources name.

A hand offer has no walk edge, so D5's mount edge renders the
structural clause — *"offered by the host"* — and the fix names the two
builder doors, never a file
([dependency kinds](dependency-kinds.md)). Declared-unused rows stay
legal: the law bounds uses by declarations, never declarations by uses.

The wasm playground rides the same law with a host-side default: the
page's compile lane accepts an optional consumer manifest (jsonc)
beside the source — `compile_manifest` on the wasm engine (a manifest
that does not parse returns as a span-0 diagnostic) — while plain
`compile` defaults the program's declaration to the offered lib
names (`ink_host`, `nmap_host`, `ink`, `pouch`, `nmapset`, `flow`,
`async_host`, `futures`, `strbuild_host`, `strbuild`, `json`, `calc` —
`core` stays row-free; the prelude is free by law), computed from the
world the host actually mounts. A `use` outside the declared set is D5.

## Url deps — the remote policy (`DepRemote` + `HttpRemote`)

The embedder owns exactly three things: the fs policy ([`Source`]),
the remote policy ([`DepRemote`]), and its own runtime. The `_with`
lanes take `&dyn DepRemote` and nothing else: the remote matters only
when the dep walk actually reaches a url row. Url-free projects and
`.rutbundle`s are closed — they load with no remote named at all.

The standard remote is **`rut_native::HttpRemote`** — cache-first, and
**cache-only by default**: a hit NEVER touches the network, and a miss
is the loud offline error (prime the cache with `rut install`, `rut
fetch`, or an embedder's `DepRemote::write`). Networking is the
EXPLICIT `online_local` / `online` choice: a miss GETs (redirects on,
loud status errors, a size cap, hard timeouts — 10 s to connect, 60 s
for the whole request) on the remote's own private worker thread and
writes the cache back atomically. Cache layout law:
`<root>/<sha256(url)>.rutbundle`. The futures come back READY, so any
executor works — including the std-only noop-waker `block_on` poll
loop the examples spell.

| constructor | behavior |
|---|---|
| `HttpRemote::project_local(project)` | `<project>/.rut/cache` — hermetic, project-local; **cache-only** — a miss never networks (the default) |
| `HttpRemote::at(root)` | an explicit cache root, cache-only |
| `HttpRemote::online_local(project)` | project-local **with the wire armed** — a miss GETs (under hard timeouts); the explicit opt-in for installers and plugin loaders |
| `HttpRemote::online(root)` | an explicit cache root with the wire armed — the only flavor that networks |
| `path_for(url)` / `evict(url)` | the concrete entry path / delete a poisoned entry |

**Guaranteed-offline loads** (tests, CI, wasm-adjacent hosts, and — by
default — every load): prime a cache-only remote, then load over it.
The committed std artifacts play the wire — `DepRemote::write` is the
stand-in for the GET:

```rust
let remote = rut_native::HttpRemote::at(&cache_root);
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
        Ok(self.0.push((url.to_string(), bytes::to_vec())))
    }
}
```

The walk still owns WHAT the bytes are: the pin is **lock law** — the
root's `rut.lock` (url → sha256), gated before any fetch and verified
at the mount door on every load — fresh fetch, cache hit, vendored
map, test fixture.

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

// from bytes (embedders, tests, wasm): decode + verify ONCE —
// the handle is reusable across runs (offer it with pkg(&bundle))
let bundle = rut_driver::Bundle::parse(&bytes)?;
```

Both root kinds load (a compiled root with its groups and its
pack-time scope ledger, rebased at the close of the world; a decl
root as the pkg's host rows), first-mount-wins, and the bundle root's
package name is `loaded.root`. Wasm hosts `include_bytes!` the
committed artifact and parse through `Bundle::parse`. For the
url-dep *walk* (pins, closure checks, the archive's identity-keyed
groups) stay on the `load_dir*` lanes — a bundle mount is the offer,
not the walk. The container, the manifest grammar, and the reader are
[module bundles](bundles.md).

**What to take from a url** is the engine's instantiation law, read
from the embedding side: a compiled bundle serves host surfaces,
concrete-class libs, **and** — by instantiation independence — the
generic owners: a request the pack-time ledger lacks monomorphizes from the
template the binary carries and compiles the body under the declaring
pkg's spec, at the link, nothing persisted (`Vec<MyTodo>`,
`decodeJson<T>` — [module bundles](bundles.md) — the std-CDN section).
A bundle that owns an open generic surface but whose binary carries no
template bodies refuses at the mount (corrupt or doctored — re-pack
the directory), and a bundle-mounted json names its
pack-time dev closure in its ledger, so the consumer's closure must
contain those names.

## The walk API (`rut-native`)

| API | Meaning |
|---|---|
| `Source` | the one filesystem door, STRING keys: `read(key)`, `resolve(from, spec)`, `root()` — the key math is the impl's; a `Path` crosses nothing |
| `FsSource::at(dir)` | the real filesystem with the root baked in |
| `DepRemote` | the url-dep contract: `fetch(&self, url) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, RemoteError>> + '_>>` (required), `lookup`, `write` — the call site owns HOW bytes arrive (transport, cache, offline policy). Dyn-compatible (hand-rolled boxing, no async-trait), deliberately **not** `+ Send`: a browser `fetch` bridge is `!Send`, and sync impls (cache hits, fixtures) are first-class |
| `HttpRemote` | the standard remote: cache-first (`<root>/<sha256(url)>.rutbundle`), **cache-only by default** (`project_local` / `at` — a miss is the loud offline error), the wire only on `online_local` / `online` (a miss GETs under hard timeouts and writes back atomically); `path_for`/`evict` are concrete |
| `load_dir(dir)` / `load_dir_with(dir, &remote)` / `load_dir_fetched(dir, &map)` | the walk and its fetched lanes — the yield is `rut_driver::Walk` ([the walk vs the offer](#the-walk-vs-the-offer)) |
| `load_path_session(path)` / `load_path_session_with(path, &remote)` | dispatch on the shape: directory walk or closed `.rutbundle` |
| `load_bundle_session(path)` | load a packed `.rutbundle` from disk — the pure container lane plus the file read |
| `dir_pkgs(dir)` / `dir_pkgs_with(dir, &remote)` | the OFFER lane: a package directory walked for someone else's program (no dev pass, no gate) |
| `tree_pkg(name)` | one toolchain-tree package (`rut/<name>`); a `[deps]` closure is refused |
| `load_module_source(path)` | one `.rut` file's text — a loose file is a single module, no manifest |
| `prefetch_urls(&src, &dir, &remote)` | collect a manifest's url rows and fetch them — the `_with` lanes' input step |
| `pack_dir(dir)` / `pack_dir_with(dir, &remote)` / `pack_dir_opts(..)` family | pack a module directory into a deterministic `.rutbundle` — a **compiled** root for a lib pkg, a **decl** root for a host pkg (`type` routes; `format_version` is 12); returns the bytes ([module bundles](bundles.md)) |
| `default_out_path(dir)` | the conventional pack output path |

## The run chain API (`rut-driver`)

| API | Meaning |
|---|---|
| `.pkg(offer)` | the ONE offering verb over the `Offer` classification — `pkg(&bundle)` (`Offer::Lib`), `pkg(host_pkg)` (`Offer::Host`), `pkg(pack_inline_pkg(name, text))` for ephemeral source. FIRST OFFER WINS: a pkg whose name is already offered is ignored |
| `.entrypoint(name)` | the root pkg — the graph compiles it and its transitive uses |
| `.compile() -> Result<Compiled, RunError>` | the terminal step ([the law](#the-compile-law)) |
| `Compiled { graph, hosts }` | the linked graph (diags inside — check `graph.diags` before trusting `graph.program`) and the host registry every `Offer::Host` installed |
| `Bundle::parse(&bytes)` | decode + verify an archive ONCE (CRC, version, group decode) — the handle is reusable across runs; `bundle.restore_symbols(&map)` restores a stripped artifact's names on the handle ([symbol stripping](symbol-stripping.md)); `bundle.walk()` is the raw mount assembly |
| `Walk { pkgs, root }` | a walk's yield; `walked.pkg(name)` picks one pkg |
| `Mounted::source / decl / host / compiled / from_bundle` | the walked pkg data's constructors ([the offer](#the-walk-vs-the-offer)) |
| `RunError` | why a run refused to compose or compile — shape (bad name, missing entrypoint) or closure (a `use` nothing offered resolves); the message IS the diagnostic |
| `declared_host_fns(pkgs)` | the declared rows, async-expanded, flattened — the raw lane's `verify_against` input |
| `host_pkg_ctx(pkgs)` | the declared rows, partitioned by scope — for hosts that build registries OUTSIDE the chain (a bench probe's fresh `Vm` per iteration, a raw-row host's `verify_against`) |
| `compile_module(src, mode, name)` | the single-source lane: the core prelude auto-rides, the source offers as the root pkg, one `.compile()` — returns diags, AST/IR dumps, and the binary. The walked packages (`calc`'s `Math`, the async pair) are NOT here: a host that wants them offers `rut_native::tree_pkg(..)` |
| `compile_program(src, mode, name)` / `compile_program_resolved(..)` / `ir_dump_of(..)` | the raw pipeline pieces (tooling) |
| `lower_decl_module(text, children, file)` | lower a `.d.rut` surface to host rows (`children` = the mounted `mod` tree — a declared child with no mounted `NAME/mod.d.rut` is a loud error; registration is path-qualified, `pkg::net::connect`) |
| `bundle::{parse_manifest, Bundle, Layout, ..}` | the container codec, the `rut.jsonc` grammar, the reader (both root kinds) — pure, in-memory |
| `pack::{pack, pack_inline_pkg, PackOpts, ..}` | the pure emit half of packing (`rut-native` owns the walk + dev tables + the gate); `pack_inline_pkg(name, text)` assembles a one-member source bundle — never compiles |
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
| `HostPkg::new(scope)` | a pkg builder; rows register under bare names or scope-relative paths (`"net::connect"` — a row under the pkg's `mod net` tree; the scope itself is never part of the row name) and the scope prefixes once, at the install |
| `pkg_fn!` / `pkg_async_fn!` | the builder's sugar: one spelling → one row / the five-row async family (the spelling may carry the mod path: `"net::send"` → `net::send__start` et al) |
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
`.pkg(rut_std::math::pkg())` — and installs at `.compile()`
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
| `http::pkg()` | the std HTTP lanes (reqwest; native builds only); `http::pkg_with(f)` is the fixture twin, returning `(HostPkg, HttpFixture)`. The `http_host` surface is a mod tree — the rows register scope-relatively (`net::http_send` + the readbacks, `stream::http_body`/`http_stream_next`) |
| `bench_cross::pkg()` | the crossing-tax benchmark rows |

## In-tree examples

- [03 — Plugin](../examples/03-plugin.md): a Rust chat server driving a rut
  moderator plugin, loaded from a module directory and from a packed
  `.rutbundle`; re-entrant `emit` crossings.
- [06 — GitHub viewer CLI](../examples/06-github-viewer-cli.md): walks the
  std packages, binds example-local I/O rows, launches an async entry fn,
  and pumps the driving loop to idle.
