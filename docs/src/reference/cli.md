# The rut CLI

The `rut` binary is the toolchain's command-line face. It is also a
**full host**: running a program walks its directory, offers the tree
packages its surface names, binds the standard native bodies, and
drives the async loop — the same contract an embedded host implements
([embedding and native modules](embedding.md)).

It is the **temporary-run** lane: running module directories and
bundles, `fmt`, `dump`, `pack`. The intended consumption path is depending on the engine
crates from your own project through a Cargo git dependency
([installation](../quick-start/installation.md)) and embedding the VM
([embedding and native modules](embedding.md)) — this page documents the
binary itself. To get the binary: `cargo install` from git, `cargo install --path` from a
checkout, or a plain `cargo build --release`
([installation](../quick-start/installation.md)).

```sh
rut run [<dir | mod.rutbundle>] [--entry <fn>] [--fuel N] [--symbols <file.rutsym>] [--no-cache]
rut new <dir>
rut install [<dir>] [--locked]
rut fmt <file.rut | dir> [--check]
rut pack <dir> [-o out.rutbundle] [--strip]
rut fetch <dir>
rut dump <file.rut>
```

The arg surface is clap: `rut --help` prints the commands, `rut
<command> --help` the flags. `run` and `install` take their `<dir>`
optionally — omitted, both act on the working directory (`.`): inside
a module directory, `rut run` runs it and `rut install` pins it.
Running `rut` with no subcommand prints
the help to stderr and exits 2; a parse error — an unknown flag, a
dangling flag value, an unparsable `--fuel` — is the same exit 2 with
clap's message. The usage sketch above stays the quick map.

## `run`

Compiles and executes. Two input forms — every rut program is a module
directory ([project structure](project-structure.md)), so there is no
loose-file shape:

| input | pipeline |
|---|---|
| `dir` (a module directory with `rut.jsonc`) | **cache-first**: the run key is sha256 over the closure's sources, manifests, `rut.lock` pins, and the toolchain version — a hit in `.rut/cache/<key>.rutbundle` mounts the cached archive; a miss is the whole-program compile (the deterministic reference — [the compilation model](compilation-model.md)), whose archive then emits into the cache. `--no-cache` (alias `--repack`) forces the miss path; the fresh archive still refreshes the entry. A host-rooted directory never reaches the cache (it refuses at the door — a host root is a declaration leaf, nothing to run) |
| `mod.rutbundle` | the packed form of the same contract — decoded directly, no cache lane ([module bundles](bundles.md)) |

The **install-first law**: `run` and `pack` never download. Before the
walk, every url dep the closure declares must already sit in the cache
— a missing set is one aggregated refusal (exit 2) naming each dep and
the fix: `run: N url deps not in the bundle cache — run `rut install`
first`. [`install`](#install) and [`fetch`](#fetch) are the only
downloading lanes; everything after the gate — the walk, the mount
door's pin check, the compile, the drive — is cache-only and hermetic.

Anything else is refused at the door (exit 2): a loose `.rut` file is
not a runnable unit — give the directory a `rut.jsonc` naming the
package (`{"name": "…"}` — the root module is `mod.rut` beside it;
JSONC: `//` comments and trailing commas are legal), or run a packed
`.rutbundle`.

Flags and defaults:

| item | behavior |
|---|---|
| `--entry <fn>` | name the `entry fn` to run. Without the flag: exactly one `entry fn` runs the program; several refuse to guess (exit 1, naming the set); none reports `no `entry fn` — nothing to run` — a library shape compiles clean, the designation is where the run stops. A named fn that is not an entry is a loud error |
| `--fuel N` | cap the op budget per turn. Fuel is opt-in: without the flag the run is uncapped; a missing or unparsable value is a loud error (exit 2) — no silent default |
| `--symbols <file.rutsym>` | restore a [stripped artifact's](symbol-stripping.md) private symbol table — legal only against a compiled `.rutbundle` (the directory lane compiles fresh and needs no map; anything else is a usage error, exit 2). The table restores before the graph compiles, so linking and every trace see real names; sections naming modules the bundle does not carry warn |
| heap limit | fixed at 64 MiB |
| interrupt check | every 1024 ops |

Bodies bound by `run` (each rides the chain as `pkg(Offer::Host(..))`):

| chain step | purpose |
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

## `new`

```sh
rut new hello            # scaffold a hello-world module directory
```

Scaffolds a hello-world module directory in **three files**: the
manifest with the full swappable std set as **unpinned** url deps, the
matching `rut.lock` (the std set's pins), and the root module — the
same bytes as [your first rut program](../quick-start/first-program.md).
`rut install hello` warms the cache once (nine bundles), then
`rut run hello` prints `hello, rut!`.

Behavior:

| item | behavior |
|---|---|
| package name | derived from the target's basename — bare `[a-zA-Z0-9_]+`, non-empty (the manifest's law). An illegal name is refused, never sanitized (exit 2) |
| clobber refusal | a target already holding a `rut.jsonc`, a `mod.rut`, or a `rut.lock` is refused (exit 2, naming the file) — other pre-existing files are fine |
| parents | created as needed — `rut new a/b/hello` works |
| files written | `rut.jsonc` — the derived name plus the whole swappable std set (nine packages) as unpinned url deps at the `@std-v11` tag; `rut.lock` — the set's pins, rendered through the lock grammar ([`install`](#install) is its re-writer); `mod.rut` — the ink hello |
| first run | **refuses**: `rut run`/`rut pack` never download — `rut install <dir>` fetches the std set (9 bundles) into `<dir>/.rut/cache` (with progress), `rut fetch <dir>` pre-warms the same way; then every run is cache-only |

The three created files, for `rut new hello` — all verbatim:

```text
{
  // hello — scaffolded by `rut new`: a runnable hello-world module
  // dir — the manifest carries the deps, and `mod.rut` is the root
  // module the loader mounts.
  //
  // THE STD SET: all nine swappable std libs, each a COMMITTED
  // compiled bundle (dist/std, jsDelivr-served at the std-v11 tag).
  // The pins live in `rut.lock` beside this file (machine-generated:
  // the scaffold wrote them; `rut install` re-pins after a dep edit —
  // the sha256 key is retired from manifest rows). Unused deps are
  // legal: host pkgs merge inert unless the program mounts their pkg,
  // and every source pkg compiles only as part of the closure — this
  // hello (ink only) runs clean with the full set declared.
  "name": "hello",
  "deps": {
    "ink": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/ink.rutbundle" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/pouch.rutbundle" },
    "flow": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/flow.rutbundle" },
    "nmapset": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/nmapset.rutbundle" },
    "json": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/json.rutbundle" },
    "strbuild": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/strbuild.rutbundle" },
    "calc": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/calc.rutbundle" },
    "futures": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/futures.rutbundle" },
    "http": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/http.rutbundle" }
  }
}

// hello/rut.lock
// rut.lock — generated by `rut install`; commit it. Rows sorted by url;
// the sha256 is law at the mount door on every load.
{
  "format": "rutlock",
  "format_version": 1,
  "deps": [
    { "pkg": "calc",     "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/calc.rutbundle",     "sha256": "0ded688375f71dc4241096faf4ac07bd5154f765705145908589d911fef918d0" },
    { "pkg": "flow",     "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/flow.rutbundle",     "sha256": "48493535461a15bbe7bcdea881e221cce62354ee92651c527533241853a3c730" },
    { "pkg": "futures",  "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/futures.rutbundle",  "sha256": "a3b44897107b3a04aa605ad94f674dedd254755f0310448feb8d1c5102cfa89b" },
    { "pkg": "http",     "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/http.rutbundle",     "sha256": "3494e2554e369119fa460213c00b3384ab6318892d09cc236cdea359c5388677" },
    { "pkg": "ink",      "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/ink.rutbundle",      "sha256": "7e9e7f58582ae0a3cd5b4ae76a16b7051c0c79f9479edc34a3ab8261ee250205" },
    { "pkg": "json",     "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/json.rutbundle",     "sha256": "811e6f5b98ebb2816ea21782e42441183a1c5d262850567164540cc3d7801467" },
    { "pkg": "nmapset",  "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/nmapset.rutbundle",  "sha256": "8b9b62ef68994115db7a8e75bf410fd4ab3a6c0213733b71df1001b4db83a7b1" },
    { "pkg": "pouch",    "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/pouch.rutbundle",    "sha256": "249aba803d8a70f05952041f667b5897f4733257f00d5d7b73c6032f4ba70373" },
    { "pkg": "strbuild", "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/strbuild.rutbundle", "sha256": "1c152f28ff2d55b9dbd9868c2abe15c7116840d6a4cc8bcd4f2cdf520f848dfd" }
  ]
}

// hello/mod.rut
use ink::{ Logger };

entry fn main() {
    let log = Logger("hello");
    let name = "rut";
    log.info(f"hello, {name}!");
}
```

## `install`

```sh
rut install                    # resolve the url deps of the working directory, write rut.lock
rut install app/               # resolve the url deps, write rut.lock
rut install app/ --locked      # CI: verify against rut.lock, write nothing
```

Resolves the closure's url deps and **pins them in `rut.lock`** — the
pin ledger beside the walk root's `rut.jsonc`, one per root, **committed**
(like Cargo.lock): it is what makes everyone's build resolve the same
bytes. The lane: collect every url row the closure declares (the walk's
own collection — a path dep's url rows are collected with it, so the
root's lock pins the whole closure), fetch each cache-first and
sequentially, mount-validate the whole graph against the **computed**
pins (install never writes a lock over bytes that do not load), then
write `<dir>/rut.lock` atomically. A `.rutbundle` argument is refused
(exit 2): bundles are closed — there is nothing to install.

The lock is machine-generated and **byte-deterministic** — rows sorted
by url, fixed field order (`pkg`/`url`/`sha256`), a header comment, a
trailing newline — so the same closure renders the same bytes and the
file diffs cleanly. The url is the identity (the fetch key; one url,
one row); `pkg` is display-only (the first dep name that spelled the
url). Parsing is JSONC-lenient like the manifest's (comments and
trailing commas legal — hand-edits load), and the value refusals are
path-targeted (`deps[1].sha256: must be 64 hex digits…`). An empty
closure still writes the empty-deps lock: the record that the closure
resolved to nothing.

The streams: progress on **stderr**, summaries on **stdout** —

```text
install: ink — downloading https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/ink.rutbundle
install: ink — downloaded (4193 bytes)
install: 1 url deps resolved — 1 downloaded, 0 cache hits (4193 bytes total)
rut.lock written: 1 rows (1 added, 0 changed, 0 dropped)
```

(a warm cache swaps the miss pair for one `install: ink — cache hit
(4193 bytes)` line — no fake re-download flash; a re-run over an
identical lock prints `rut.lock unchanged (1 rows, no drift)` and
writes nothing).

`--locked` never writes and never evicts: it fetches the same closure
and verifies every row against the existing lock — the CI gate.

| `--locked` case | behavior |
|---|---|
| lock fresh | `install: 1 url deps verified against rut.lock — 0 downloaded, 1 cache hits, 0 drift`, exit 0 |
| no lock, non-empty closure | the install recipe, exit 2: ``install: no rut.lock in <dir> — run `rut install` first`` |
| no lock, empty closure | passes lockless — a no-dep project needs no lock |
| a dep's url has no lock row | the `+` diff row, exit 2: ``+ <url> (dep `X` has no rut.lock row)`` |
| fetched bytes break the lock's pin | the `~` row, exit 2 — it names the url and BOTH hashes: ``~ <url> (dep `X` pins <lock sha>, the fetched bytes hash to <served sha>)`` |
| a lock row no dep names anymore | the `-` row, exit 2: ``- <url> (no dep names this url anymore)`` |

Any drift prints `install: --locked found drift, rut.lock untouched:`
and the diff block; the lock file on disk is byte-for-byte what it was.

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
(format_version 11): the root and every source dep ride as `mod.rutc`
binaries (bodies + surface — the linking truth) under
`deps/<name>@<fp16>/` identity directories, host pkgs (and declared
deps whose unit did not link) ride as declaration file sets, and the
scope ledger's rows spell the identities so any loader can rebase the
binaries onto its own numbering. A root that produces no compiled
program — a `type = "host"` pkg — is refused: pack a source pkg
instead. A **`type = "host"` pkg packs as a decl root** (its `.d.rut`
surface riding as source at the canonical `mod.d.rut` name,
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
Compilation is the same offered-world pipeline `run` uses, minus
execution and verification. `.d.rut` inputs dump in declaration mode —
useful for inspecting a host surface's slots.

## `fetch`

```sh
rut fetch app/            # CI: warm the cache while the network is up
```

Walks a module **directory** with the wire-armed remote and discards
the result — every `deps` url is fetched (or found in the cache) and the
graph is then loaded and thrown away, so a later offline `run` hits
only the cache. The load is the same law every load obeys: the walk
root's `rut.lock` must pin every collected url row, so `rut fetch` is
the *re*-warm lane — [`install`](#install) is the lane that writes the
lock a fetch (and every run) travels with. A `.rutbundle` argument is
refused (exit 2): bundles are closed — there is nothing to fetch.
The cache, the download policy, and the eviction law are `install`'s —
the online lanes; `run`/`pack` ride the same cache with the wire off:

| item | behavior |
|---|---|
| cache root | `<project>/.rut/cache` — project-local, hermetic; `$RUT_CACHE_DIR` overrides the root (CLI-side only — the walk never reads env). Entries named `<sha256(url)>.rutbundle` |
| cache-first | a hit never touches the network — offline reloads are the point |
| miss (install/fetch) | GET (redirects on) under hard timeouts (10 s connect, 60 s whole-request — a stalled host fails loudly, never hangs), non-2xx is a loud error, a size cap refuses without buffering, the write is atomic (tmp + rename) |
| miss (run/pack) | the loud install-first refusal — run and pack never download |
| poisoned entry | the pin is the loader's law at the mount door — the pin's source is the root's `rut.lock`; when the door refuses, the CLI maps the url back to its cache path, deletes the entry, and exits 2 — the next `install` (or `fetch`) re-fetches and heals |

The wire is `rut-native`'s `http` feature (default on) — a build with
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
| `2` | usage errors (missing arguments); unreadable input file; `run` on a `.d.rut`; `run` on an input that is neither a module directory nor a `.rutbundle`; a url dep missing from the bundle cache (the install-first refusal — `run`/`pack` never download); `fmt` finds no `.rut` files under a directory; a url dep the root's `rut.lock` does not pin, or whose fetched bytes break the lock's pin (the poisoned cache entry is evicted); `install`/`fetch` on a `.rutbundle`; `install --locked` drift, including the missing-lock refusal; `install` on a directory whose graph fails to mount against the computed pins; `new` onto an existing `rut.jsonc`/`mod.rut`/`rut.lock`, or an illegal package name |

Diagnostics go to stderr; the success summaries (`pack`'s packed line,
`install`/`fetch`'s resolved/verified lines and lock reports) go to
stdout.

## Notes

- The CLI offers and binds on behalf of the program, but it never
  injects names the program did not declare: a program that never spells
  `use ink::` gets no logger; one that never offers the async packages
  has no launcher and `await` stays cold-poll inline
  ([core and the swappable packages](stdlib.md)).
- The HTTP lane is native-only: the CLI build carries it, wasm builds do
  not.
- `run` on a module directory is the deployment path for development;
  `pack` + `run <bundle>` is the shipping path — both run the identical
  contract ([loading and the embed loop](loading.md)).
