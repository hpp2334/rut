# The rut CLI

The `rut` binary is the toolchain's command-line face. It is also a
**full host**: running a program mounts the base packages, binds the
standard native bodies, and drives the async loop — the same contract an
embedded host implements ([embedding and native modules](embedding.md)).

It is the **temporary-run** lane: running module directories and
bundles, `fmt`, `dump`, `pack`. The intended consumption path is depending on the engine
crates from your own project through a Cargo git dependency
([installation](../quick-start/installation.md)) and embedding the VM
([embedding and native modules](embedding.md)) — this page documents the
binary itself.

```sh
rut run <dir | mod.rutbundle> [--entry <fn>] [--fuel N] [--symbols <file.rutsym>]
rut fmt <file.rut | dir> [--check]
rut pack <dir> [-o out.rutbundle] [--strip]
rut fetch <dir>
rut dump <file.rut>
```

Running `rut` with no subcommand prints the usage line to stderr.

## `run`

Compiles and executes. Two input forms — every rut program is a module
directory ([project structure](project-structure.md)), so there is no
loose-file shape:

| input | pipeline |
|---|---|
| `dir` (a module directory with `rut.jsonc`) | load the whole graph, compile it, run the entry designation (below) ([project structure](project-structure.md)) |
| `mod.rutbundle` | the packed form of the same contract ([module bundles](bundles.md)) |

Anything else is refused at the door (exit 2): a loose `.rut` file is
not a runnable unit — give the directory a `rut.jsonc`
(`{"name": "…", "entry": {"lib": "./<file>.rut"}}` — JSONC: `//`
comments and trailing commas are legal), or run a packed
`.rutbundle`.

Flags and defaults:

| item | behavior |
|---|---|
| `--entry <fn>` | name the `entry fn` to run. Without the flag: exactly one `entry fn` runs the program; several refuse to guess (exit 1, naming the set); none reports `no `entry fn` — nothing to run` — a library shape compiles clean, the designation is where the run stops. A named fn that is not an entry is a loud error |
| `--fuel N` | cap the op budget per turn. Fuel is opt-in: without the flag the run is uncapped; a missing or unparsable value is a loud error (exit 2) — no silent default |
| `--symbols <file.rutsym>` | restore a [stripped artifact's](symbol-stripping.md) private symbol table — legal only against a compiled `.rutbundle` (the directory lane compiles fresh and needs no map; anything else is a usage error, exit 2). The table restores before the graph compiles, so linking and every trace see real names; sections naming modules the bundle does not carry warn |
| heap limit | fixed at 64 MiB |
| interrupt check | every 1024 ops |

Bodies bound by `run`:

| installer | purpose |
|---|---|
| `math::pkg()` | `calc`'s float fns |
| `logger::pkg` (sink: stdout) | the logger; silent no-op unless the program uses `ink` |
| `nmap::pkg()` | the native key table behind `nmapset` |
| `bench_cross::pkg()` | the crossing-benchmark rows |
| `async_host::pkg()` | the async launchers |
| `http::pkg()` | the std HTTP lanes |

Execution: `main` is called with no arguments; then the async driving
loop runs — drain the ready queue, advance the virtual clock to the next
timer deadline, repeat until no frames and no pending work remain. The loop is
capped, so a program that never idles fails loudly instead of hanging.

A `.d.rut` input is refused: a declaration file is a surface, not a
runnable module ([host fns and declaration files](host-fns.md)).

## `fmt`

```sh
rut fmt src/            # rewrite in place
rut fmt --check main.rut
```

The canonical formatter. Behavior:

- a **directory** argument is walked recursively; every `.rut` file is
  collected (sorted) and formatted;
- style comes from the nearest ancestor `rut.jsonc`'s `style` block; no
  manifest → defaults ([project structure](project-structure.md));
- `.d.rut` files are formatted in declaration mode;
- a file that does not parse clean is **refused** (diagnostics listed,
  nonzero exit) — fmt never reformats on a parse error;
- default mode rewrites in place and prints `formatted: <path>` per
  changed file;
- `--check` writes nothing: it prints `unformatted: <path>` for each
  would-change file and exits nonzero, or `fmt: N file(s) formatted` when
  everything is clean.

## `pack`

```sh
rut pack plugins/server -o server.rutbundle
# packed plugins/server -> server.rutbundle (18304 bytes)
rut pack plugins/server --strip
# packed plugins/server -> plugins/server.rutbundle (...) + symbol table
# plugins/server.rutsym (... bytes, keep PRIVATE)
```

Packs a module directory into a **deterministic** `.rutbundle` — same
input, same bytes. Without `-o`, the output is written beside the input
as `<dir-name>.rutbundle`. A **lib** pkg packs **compiled**
(format_version 9): the root and every linkable package ride as
`.rutc` binaries (bodies + surface — the linking truth), splice-needed
packages (generic exports, interface-typed parameters, `inline`) and host
pkgs ride as source groups, and a scope ledger lets any loader rebase
the binaries onto its own numbering. A root that cannot link is
refused — share the directory instead. A **`type = "host"` pkg packs
as a v10 decl root** (its `.d.rut` surface riding as source,
single-package), `--strip` refuses there (`no symbols to strip`), and
`run` accepts either bundle directly — though running a host bundle
refuses with intent: bind its rows from the embedder
([module bundles](bundles.md)).

`--strip` mangles every renameable name and strips the symbolication
tables from the emitted binaries, writing the private symbol table at
the sibling path `<out-without-ext>.rutsym` — the sidecar is never an
entry inside the bundle. It needs a fully-compiled closure: a source
group that could bind a compiled group's surface is a pointed refusal.
See [Symbol stripping and `.rutsym` sidecars](symbol-stripping.md).

## `dump`

```sh
rut dump main.rut
```

Prints the compiler's view of one file: the `== AST ==` section followed
by `== IR ==` (per-function typed register tables and op listings).
Compilation is the same base-mounted pipeline `run` uses, minus execution
and verification. `.d.rut` inputs dump in declaration mode — useful for
inspecting a host surface's slots.

## `fetch`

```sh
rut fetch app/            # CI: warm the cache while the network is up
```

Loads a module **directory** with the url-dep remote and discards the
session — every `deps` url is fetched (or found in the cache) and
pinned, so a later offline `run` hits only the cache. A `.rutbundle`
argument is refused: bundles are closed — there is nothing to fetch.
The cache and the eviction law are the same lane `run`/`pack` use:

| item | behavior |
|---|---|
| cache root | `<project>/.rut/cache` — project-local, hermetic; `$RUT_CACHE_DIR` overrides the root (CLI-side only — the driver never reads env). Entries named `<sha256(url)>.rutbundle` |
| cache-first | a hit never touches the network — offline reloads are the point |
| miss | GET (redirects on), non-2xx is a loud error, a size cap refuses without buffering, the write is atomic (tmp + rename) |
| poisoned entry | the `sha256` pin is the loader's law at the mount door; when it refuses, the CLI maps the url back to its cache path, deletes the entry, and exits 2 — the next run re-fetches and heals |

The wire is the driver's `http` feature (default on) — a build with
`default-features = false` (the wasm graphs) has no transport at all:
a cache miss errors loudly instead of networking.

## Declaration mode

The file extension selects the parser mode:

| file | mode | `run` | `fmt` | `dump` |
|---|---|---|---|---|
| `*.rut` | implementation | runs `main` | formats impls | AST + IR of the module |
| `*.d.rut` | declaration | refused (exit 2) | formats the surface | the surface's AST + IR |

## Exit codes

| code | meaning |
|---|---|
| `0` | success |
| `1` | compile diagnostics; no binary emitted; binary decode or verify failure; VM boot failure; a trap at run; `fmt` refuses a file that does not parse; `fmt --check` found drift; `pack` or output-write failure |
| `2` | usage errors (missing arguments); unreadable input file; `run` on a `.d.rut`; `run` on an input that is neither a module directory nor a `.rutbundle`; `fmt` finds no `.rut` files under a directory; a url dep's `sha256` pin refusal (the poisoned cache entry is evicted) |

Diagnostics go to stderr; `pack`'s success line goes to stdout.

## Notes

- The CLI mounts and binds on behalf of the program, but it never
  injects names the program did not declare: a program that never spells
  `use ink::` gets no logger; one that never mounts the async packages
  has no launcher and `await` stays cold-poll inline
  ([core and the swappable packages](stdlib.md)).
- The HTTP lane is native-only: the CLI build carries it, wasm builds do
  not.
- `run` on a module directory is the deployment path for development;
  `pack` + `run <bundle>` is the shipping path — both run the identical
  contract ([loading and the embed loop](loading.md)).
