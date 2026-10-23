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
rut pack <dir> [-o out] [--mode source|bin] [--strip]
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
package (`{"name": "…"}` — the root module is `src/mod.rut` under it;
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
matching `rut.lock` (the std set's pins), and the root module
`src/mod.rut` — the same bytes as
[your first rut program](../quick-start/first-program.md).
`rut install hello` warms the cache once (nine source zips), then
`rut run hello` prints `hello, rut!`.

Behavior:

| item | behavior |
|---|---|
| package name | derived from the target's basename — bare `[a-zA-Z0-9_]+`, non-empty (the manifest's law). An illegal name is refused, never sanitized (exit 2) |
| clobber refusal | a target already holding a `rut.jsonc`, a `src/mod.rut`, or a `rut.lock` is refused (exit 2, naming the file) — other pre-existing files are fine |
| parents | created as needed — `rut new a/b/hello` works |
| files written | `rut.jsonc` — the derived name plus the whole swappable std set (nine packages) as unpinned url deps at the `@std-v14` tag (source zips); `rut.lock` — the set's pins, rendered through the lock grammar ([`install`](#install) is its re-writer); `src/mod.rut` — the ink hello |
| first run | **refuses**: `rut run`/`rut pack` never download — `rut install <dir>` fetches the std set (9 source zips) into `<dir>/.rut/cache` (with progress), `rut fetch <dir>` pre-warms the same way; then every run is cache-only |

The three created files, for `rut new hello` — all verbatim:

```text
{
  // hello — scaffolded by `rut new`: a runnable hello-world module
  // dir — the manifest carries the deps, and `src/mod.rut` is the root
  // module the loader mounts.
  //
  // THE STD SET: all nine swappable std libs, each a COMMITTED
  // source zip (dist/std, jsDelivr-served at the std-v14 tag) —
  // source compiles with YOUR toolchain, so a zip never goes stale
  // the way compiled IR does.
  // The pins live in `rut.lock` beside this file (machine-generated:
  // the scaffold wrote them; `rut install` re-pins after a dep edit —
  // the sha256 key is retired from manifest rows). Unused deps are
  // legal: host pkgs merge inert unless the program mounts their pkg,
  // and every source pkg compiles only as part of the closure — this
  // hello (ink only) runs clean with the full set declared.
  "name": "hello",
  "deps": {
    "ink": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/ink.zip" },
    "pouch": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/pouch.zip" },
    "flow": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/flow.zip" },
    "nmapset": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/nmapset.zip" },
    "json": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/json.zip" },
    "strbuild": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/strbuild.zip" },
    "calc": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/calc.zip" },
    "futures": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/futures.zip" },
    "http": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/http.zip" }
  }
}

// hello/rut.lock
// rut.lock — generated by `rut install`; commit it. Rows sorted by url;
// the sha256 is law at the mount door on every load.
{
  "format": "rutlock",
  "format_version": 1,
  "deps": [
    { "pkg": "calc",     "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/calc.zip",     "sha256": "d6d04afa561f39f06b5181411640e0cc0cb97a4095e0b7f8d1f5957534831686" },
    { "pkg": "flow",     "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/flow.zip",     "sha256": "47ac4b49fc809640ea067cf3c945bf7442f017a7d113e2567d2f30f69124838c" },
    { "pkg": "futures",  "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/futures.zip",  "sha256": "e143211a0a6e9ddb0a3ccb5e3e79796d85b2ee627ee4c63f046dc583a55eb711" },
    { "pkg": "http",     "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/http.zip",     "sha256": "9c6855cd58d7d888515fff691787a42708cef512423dc6040f8e214dbd00cbac" },
    { "pkg": "ink",      "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/ink.zip",      "sha256": "eb2b568316efb77286cc5d94bec24a94f45d528cf244f54b91cb0d619dc53cc1" },
    { "pkg": "json",     "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/json.zip",     "sha256": "b15d867fb3743a829cc0bf5ace1a642dd8b9024d681aa7015d6c166bb29dbc63" },
    { "pkg": "nmapset",  "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/nmapset.zip",  "sha256": "d9f722c65bf1d1a2d08078b478f342230a01fb07ef47f504edbd4defd73dbc6e" },
    { "pkg": "pouch",    "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/pouch.zip",    "sha256": "5740cb0b874a2c9a81f1e0d9534520fe4067e5ec48320b82cf7147e2d6b34d36" },
    { "pkg": "strbuild", "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/strbuild.zip", "sha256": "1eedb374bcb52419b9ec82d9ea6f23fb3534faa0a4c75e4d03afedc6d7dabe33" }
  ]
}

// hello/src/mod.rut
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
install: ink — downloading https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v14/dist/std/ink.zip
install: ink — downloaded (2883 bytes)
install: 1 url deps resolved — 1 downloaded, 0 cache hits (2883 bytes total)
rut.lock written: 1 rows (1 added, 0 changed, 0 dropped)
```

(a warm cache swaps the miss pair for one `install: ink — cache hit
(2883 bytes)` line — no fake re-download flash; a re-run over an
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
rut pack plugins/server                    # --mode source is the default: the .zip
# packed plugins/server -> plugins/server.zip (… bytes)
rut pack plugins/server --mode bin
# packed plugins/server -> plugins/server.rutbundle (18304 bytes)
rut pack plugins/server --mode bin --strip
# packed plugins/server -> plugins/server.rutbundle (...) + symbol table
# plugins/server.rutsym (... bytes, keep PRIVATE)
```

Packs a module directory into **one deterministic artifact** — same
input, same bytes — in one of two modes:

| mode | default out | what rides |
|---|---|---|
| `--mode source` (the default) | `<dir>.zip` | the **source zip** — the directory packed for travel: `rut.jsonc` at the archive root, the module tree under `src/`, and every dep embedded under `deps/<name>@<fp16>/` — a source dep as its own tree, a compiled dep (a `.rutbundle` url dep) as its `.rutbundle` file verbatim ([the source zip](bundles.md#the-source-zip--the-source-transport)) |
| `--mode bin` | `<dir>.rutbundle` | the **compiled wire** ([module bundles](bundles.md)): a **lib** pkg packs **compiled** (format_version 12) — the root and every source dep ride as `mod.rutc` binaries (bodies + surface — the linking truth) under `deps/<name>@<fp16>/` identity directories, host pkgs (and declared deps whose unit did not link) ride as declaration file sets, and the scope ledger's rows spell the identities so any loader can rebase the binaries onto its own numbering |

`-o` overrides the default sibling output in both modes. A root that
produces no compiled program — a `type = "host"` pkg — is refused in
bin mode: `pack a source pkg instead`; source mode packs it fine (the
surface rides as source). In bin mode a host pkg packs as a **decl
root** (its `.d.rut` surface riding as source at the canonical
`mod.d.rut` name, single-package) and refuses `--strip` there (`no
symbols to strip`); `run` accepts either bundle directly — though
running a host bundle refuses with intent: bind its rows from the
embedder ([module bundles](bundles.md)). A manifest without the
`format`/`format_version` rows gains them at the bin-mode pack door —
the same project publishes both ways without hand-editing (a source
zip's manifest does not claim the compiled wire; the rows drop
there).

**The source zip's manifest is rewritten at pack.** Every `[deps]`
descriptor becomes `{ "path": "deps/<name>@<fp16>" }` — `url` fields
and external `path` values drop (everything embeds), the row keys
stay, and comments outside the replaced descriptor blocks ride. The
dep dirname is the **tree identity**: the first 16 lowercase hex of
sha256 over the package's own rewritten manifest and its `src/**.rut`
sources — the tree flavor of the content identity, as against the
wire flavor's four canonical ride files (the two never
cross-compare); two identities of one name coexist side by side.

Pack-time tree gates: a `.rut` outside a `src/` shell refuses —
``rut code rides only under a `src/` shell — declare it (`mod …;`) and
move it under `src/`, or delete it`` — and an undeclared `.rut` under
`src/` refuses — ``nothing declares this module — `mod …;` it from its
parent or delete it``.

`--strip` is a bin-mode lane: `rut pack <dir> --strip` without
`--mode bin` refuses at the door — ``pack: --strip is a --mode bin
lane — a source zip carries source, not symbols; drop the flag or
pass --mode bin``. In bin mode it mangles every renameable name and
strips the symbolication tables from the emitted binaries, writing
the private symbol table at the sibling path `<out-without-ext>.rutsym`
— the sidecar is never an entry inside the bundle. It needs a
fully-compiled closure: a source group that could bind a compiled
group's surface is a pointed refusal.
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
| cache root | `<project>/.rut/cache` — project-local, hermetic; `$RUT_CACHE_DIR` overrides the root (CLI-side only — the walk never reads env). Entries named `<sha256(url)>.zip` or `<sha256(url)>.rutbundle` — the extension follows the url's own artifact kind |
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
| `1` | compile diagnostics; no binary emitted; binary decode or verify failure; VM boot failure; a trap at run; `fmt` refuses a file that does not parse; `fmt --check` found drift; a `pack` output-write failure |
| `2` | usage errors (missing arguments); unreadable input file; `run` on a `.d.rut`; `run` on an input that is neither a module directory nor a `.rutbundle`; a url dep missing from the bundle cache (the install-first refusal — `run`/`pack` never download); `fmt` finds no `.rut` files under a directory; a url dep the root's `rut.lock` does not pin, or whose fetched bytes break the lock's pin (the poisoned cache entry is evicted); `install`/`fetch` on a `.rutbundle`; `install --locked` drift, including the missing-lock refusal; `install` on a directory whose graph fails to mount against the computed pins; `new` onto an existing `rut.jsonc`/`src/mod.rut`/`rut.lock`, or an illegal package name; a `pack` refusal — `--strip` without `--mode bin`, a `.rut` outside the `src/` shell, an undeclared `.rut` under `src/`, a host root in bin mode, a url dep the lock does not pin |

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
