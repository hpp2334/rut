# The rut CLI

The `rut` binary is the toolchain's command-line face. It is also a
**full host**: running a program mounts the base packages, binds the
standard native bodies, and drives the async loop — the same contract an
embedded host implements ([embedding and native modules](embedding.md)).

It is the **temporary-run** lane: loose-file experiments, `fmt`,
`dump`, `pack`. The intended consumption path is depending on the engine
crates from your own project through a Cargo git dependency
([installation](../quick-start/installation.md)) and embedding the VM
([embedding and native modules](embedding.md)) — this page documents the
binary itself.

```sh
rut run <file.rut | dir | mod.rutbundle> [--fuel N]
rut fmt <file.rut | dir> [--check]
rut pack <dir> [-o out.rutbundle]
rut dump <file.rut>
```

Running `rut` with no subcommand prints the usage line to stderr.

## `run`

Compiles and executes. Three input forms:

| input | pipeline |
|---|---|
| `file.rut` | one file is one module unit — compile it against the base mounts, then decode → verify → run `main` |
| `dir` (a module directory with `rut.toml`) | load the whole graph, compile it, run the root's `main` ([project structure](project-structure.md)) |
| `mod.rutbundle` | the packed form of the same contract ([module bundles](bundles.md)) |

Flags and defaults:

| item | behavior |
|---|---|
| `--fuel N` | cap the op budget per turn. Fuel is opt-in: without the flag the run is uncapped; a missing or unparsable value is a loud error (exit 2) — no silent default |
| heap limit | fixed at 64 MiB |
| interrupt check | every 1024 ops |

Single-file convenience: a loose file that declares `use ink::` (or any
tree package) gets that package mounted automatically — the CLI scans the
source for `use <name>::` across `ink_host`, `ink`, `pouch`, `nmapset`, `json`,
`strbuild`, `async_engine`, `async_host`, `http_host`, and `http`, then
assembles peer groups, so a loose file gets json's peer-gated container
impls exactly like a module-directory program
([dependency kinds](dependency-kinds.md)).

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
timer deadline, repeat until no frames and no tasks remain. The loop is
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
- style comes from the nearest ancestor `rut.toml`'s `[style]` block; no
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
```

Packs a module directory into a **deterministic** `.rutbundle` — same
input, same bytes. Without `-o`, the output is written beside the input
as `<dir-name>.rutbundle`. The bundle is **compiled** (format_version
5): the root and every linkable package ride as `.rutc` binaries
(bodies + surface — the linking truth), splice-needed packages
(generic exports, trait-object parameters, `inline`) and host pkgs
ride as source groups, and a scope ledger lets any loader rebase the
binaries onto its own numbering. A root that cannot link is refused —
share the directory instead. `run` accepts the bundle directly
([module bundles](bundles.md)).

## `dump`

```sh
rut dump main.rut
```

Prints the compiler's view of one file: the `== AST ==` section followed
by `== IR ==` (per-function typed register tables and op listings).
Compilation is the same base-mounted pipeline `run` uses, minus execution
and verification. `.d.rut` inputs dump in declaration mode — useful for
inspecting a host surface's slots.

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
| `2` | usage errors (missing arguments); unreadable input file; `run` on a `.d.rut`; `fmt` finds no `.rut` files under a directory |

Diagnostics go to stderr (`run` renders them with source spans on the
single-file path); `pack`'s success line goes to stdout.

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
