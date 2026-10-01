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
| [04 — Custom async](04-custom-async.md) | parse-only — no runnable harness | a hand-written `impl Future<nil> for CustomFuture` plus a user launcher with per-checkpoint stats and cancellation audits |
| [05 — Todolist web](05-todolist-web.md) | `cargo test -p todolist-web` + `node tests/e2e-browser.mjs` | a full page app whose brain is a two-package rut project — ten DOM/timer crossings over web_sys on wasm32 |
| [06 — GitHub viewer CLI](06-github-viewer-cli.md) | `cargo run -p rgh -- --repo=… --ref=… list` | `rgh` — an async rut brain over the std `http` lane; headers-then-stream downloads, fixture-lane tests |
| [The playground corpus](playground-corpus.md) | `cd demo && npm run smoke` | the classics: runnable programs, compiled and run by two gates |

All Cargo commands run from the repository root. The runnable crates
share one session pattern — compile the module, verify the binary,
bind host fns, then drive it through typed `vm.call` sites — so the
pages cross-link often: [00 — Todolist](00-todolist.md) establishes
the pattern and [06 — GitHub viewer CLI](06-github-viewer-cli.md)
stretches it the farthest.

## The three dependency kinds

Every package carries a `rut.jsonc` manifest (see
[Project structure and rut.jsonc](../reference/project-structure.md)),
and a manifest relates a package to other packages through **three
tables**, all visible in the examples:

- **`deps`** — ordinary dependencies, transitively mounted. The
  common case: `03-plugin`'s `plugin/rut.jsonc` declares
  `server = { path = "../server" }`, and
  [05 — Todolist web](05-todolist-web.md)'s `biz` package declares
  `ui` (which itself pulls the collection packages).
- **`peer-deps`** — required by default: the *consumer* supplies the
  peer and the peer is never pulled transitively. Marking a peer
  `optional = true` flips it to a presence relation: its integration
  file (impl-only code the peer makes compilable) mounts only when the
  peer is anywhere in the program's closure. The in-tree example is
  the std `json` package's serde-model impls — `impl JsonSerialize for
  Vec<T>` is written in json but must not force every json consumer to
  mount the collection packages, so those live as optional peers and
  as `dev-deps` for json's own tests. [02 — Digest](02-digest.md)
  consumes json *light*; [06 — GitHub viewer CLI](06-github-viewer-cli.md)
  calls `assemble_peers` and mounts the impl groups for real because
  the collections are in its closure.
- **`dev-deps`** — mounted only while building the package itself,
  never in a consumer's world. json develops against the real
  collection packages through the both-kinds pairing while its
  consumers mount it without them.

The full rules — the loud missing-required-peer error, the silence of
absent optional peers, dedup-by-origin at the splice, and what bundle
format the packer emits — are on
[Dependency kinds](../reference/dependency-kinds.md).

## Reading order

New to the language: read [00 — Todolist](00-todolist.md) and
[01 — Sort](01-sort.md) for the embedding shape, then
[The playground corpus](playground-corpus.md) for the language surface
in small doses. For the web story, [05 — Todolist web](05-todolist-web.md)
is the centerpiece and [06 — GitHub viewer CLI](06-github-viewer-cli.md)
is the networking counterpart. [03 — Plugin](03-plugin.md) is the one
to study for packaging and module loading, and
[04 — Custom async](04-custom-async.md) for what the future trait
looks like from user code.
