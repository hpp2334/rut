# Examples index

The repo ships two kinds of example material. The `examples/` directory
holds **six runnable Cargo projects** (five console programs and one
browser page) plus **one parse-only corpus example**. Alongside
them, `demo/src/examples/` holds the **playground classics** — short,
self-contained programs the web playground runs in the browser, the
same programs the repo's gates compile and run in CI.
Each project has its own page in this chapter; the classics share
[The playground corpus](playground-corpus.md).

| Project | Run | What it demonstrates |
|---|---|---|
| [00 — Todolist](00-todolist.md) | `cargo run -p todolist` | an `entry fn` surface over a rut `class` — the host drives CRUD through `opaque` handles |
| [01 — Sort](01-sort.md) | `cargo run -p sort` | five sorting algorithms behind one dispatcher entry; `when` on strings, the mut-binding law, fuel budgets |
| [02 — Digest](02-digest.md) | `cargo run -p digests` | byte-level codecs and hashes (MD5/SHA/base64/CRC/FNV); the host as an independent test oracle |
| [03 — Plugin](03-plugin.md) | `cargo run -p plugin` | a module directory + `.rutbundle` chat-moderator plugin; re-entrant `vm.call`, both `opaque` directions |
| [04 — Custom async](04-custom-async.md) | parse-only — no runnable harness | a user launcher re-spelled over the closed `Future` surface (no user type can BE a future) with per-checkpoint stats and cancellation audits |
| [05 — Todolist web](05-todolist-web.md) | `cargo test -p todolist-web` + `node tests/e2e-browser.mjs` | a full page app whose brain is a two-package rut project — ten DOM/timer crossings over web_sys on wasm32 |
| [06 — GitHub viewer CLI](06-github-viewer-cli.md) | `cargo run -p rgh -- --repo=… --ref=… list` | `rgh` — an async rut brain over the std `http` lane; headers-then-stream downloads, fixture-lane tests |
| [The playground corpus](playground-corpus.md) | `cd demo && npm run smoke` | the classics: runnable programs, compiled and run by two gates |

All Cargo commands run from the repository root. The runnable crates
share one run pattern — compile the module, verify the binary,
bind host fns, then drive it through typed `vm.call` sites — so the
pages cross-link often: [00 — Todolist](00-todolist.md) establishes
the pattern and [06 — GitHub viewer CLI](06-github-viewer-cli.md)
stretches it the farthest.

## The two dependency tables

Every package carries a `rut.jsonc` manifest (see
[Project structure and rut.jsonc](../reference/project-structure.md)),
and a manifest relates a package to other packages through **two
tables**, all visible in the examples:

- **`deps`** — the transitively mounted dependencies, and the
  declaration table: a pkg may `use` exactly the heads its own
  manifest names. The common case: `03-plugin`'s `plugin/rut.jsonc`
  declares `server = { path = "../server" }`, and
  [05 — Todolist web](05-todolist-web.md)'s `biz` package declares
  `ui` (which itself pulls the collection packages). The std `json`
  package shows the wrapper shape — its `Vec`/map serialization
  wrappers are json's OWN source, and the container packages are
  plain `deps` riding along transitively.
- **`dev-deps`** — mounted only while building the package itself,
  never in a consumer's world; the rows are `path`-only.

There is no third table: the grammar is strict at the top level, so a
retired key (`peer-deps`) or flag (`inline`) refuses as the unknown
key it now is, naming the key.

The full rules — the declaration-scoped nameability law, the closed
bundle closure, and what bundle format the packer emits — are on
[Dependency kinds](../reference/dependency-kinds.md).

## Reading order

New to the language: read [00 — Todolist](00-todolist.md) and
[01 — Sort](01-sort.md) for the embedding shape, then
[The playground corpus](playground-corpus.md) for the language surface
in small doses. For the web story, [05 — Todolist web](05-todolist-web.md)
is the centerpiece and [06 — GitHub viewer CLI](06-github-viewer-cli.md)
is the networking counterpart. [03 — Plugin](03-plugin.md) is the one
to study for packaging and module loading, and
[04 — Custom async](04-custom-async.md) for what the future protocol
looks like from user code.
