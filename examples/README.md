# rut examples — the runnable projects

Five Cargo projects, five workspace members — five runnable end to end
(four on the console, one in a browser), plus one parse-only corpus
example:

| Project | Run | Demonstrates |
|---|---|---|
| [`00-todolist/`](00-todolist/) | `cargo run -p todolist` | an `entry fn` surface over a rut `class` — the host drives CRUD through `opaque` handles |
| [`01-sort/`](01-sort/) | `cargo run -p sort` | a sorting library behind one dispatcher entry; `Result` at the boundary, known-input gates |
| [`02-digest/`](02-digest/) | `cargo run -p digests` | byte-level codecs and hashes (MD5/SHA/base64/CRC/FNV), the host as test oracle |
| [`03-plugin/`](03-plugin/) | `cargo run -p plugin` | a module directory + `.rutbundle` (RFC 0038) chat-moderator plugin; re-entrant `vm.call`, both `opaque` directions |
| [`04-custom-async/`](04-custom-async/) | parse-only — runnable at M3 | a hand-written `impl Task<T> for CustomTask<T>` plus a user launcher with per-checkpoint stats and cancellation audits; the user-impl-of-builtin-trait test. Becomes runnable when the async plan lands (the engine context is runtime-provided) |
| [`05-todolist-web/`](05-todolist-web/) | `cargo test -p todolist-web` + `node tests/e2e-browser.mjs` | the full page app: a todolist with a simulated server (request table + per-kind `tim_after` latency) whose brain is pure rut — ten DOM/timer crossings over web_sys on wasm32, the fake-DOM twin as the cargo gate, a through-the-artifact e2e in node and Firefox headless (survey: `docs/todolist-web-survey.md`) |

Short, self-contained programs — the classics — live in
[`demo/src/examples/`](../demo/src/examples/): the playground imports
them raw, and `crates/rut-cli/tests/playground.rs` compiles and runs
every one against its `.expected` sidecar. The parser conformance suite
(RFC 0030 §7) walks both trees.

## Packages and manifests — the three dep kinds

Every package carries a `rut.toml` (RFC 0041 §5 — `03-plugin`'s
`plugin/` and `server/` are the in-tree examples). Since the dep-kinds
batch (RFC 0045), a manifest relates to other packages through three
tables: **`[deps]`** — transitively mounted, unchanged; **`[peer-deps]`**
— **required by default** (the *consumer* supplies the peer; never
pulled transitively) with `optional = true` marking the presence-mounted
kind whose integration file — the descriptor's `lib`, an impl-only
`.rut` — mounts only when the peer is anywhere in the program's
closure; **`[dev-deps]`** — mounted only while building the pkg itself,
never in a consumer's world.

The motivating case is json's serde-model impls: `impl JsonSerialize
for Vec<T>` written in json must not force every json consumer to
mount pouch/nmapset. The pinned grammar:

```toml
# rut/json/rut.toml
name = "json"

[peer-deps]
pouch    = { path = "../pouch",    optional = true }
nmapset  = { path = "../nmapset",  optional = true }

[dev-deps]
pouch    = { path = "../pouch" }
nmapset  = { path = "../nmapset" }
```

(json develops against real pouch/nmapset via the both-kinds pairing;
its consumers mount json light — absent optional peers are silent,
and referencing the integration names the fix instead of a bare
unresolved-name.)

Decisions this batch landed, visible in the examples:

- **Dedup-by-origin at the splice** — two packages both riding a
  shared transitive inline pkg now compose instead of colliding, so
  [`05-todolist-web`](05-todolist-web/)'s `appkit` single-splice
  wrapper retires as a NECESSITY: it was the workaround for the
  duplicate-definition trap; it stays legal (one use, one leaf) and
  the example is untouched.
- **Required-by-default peers** — a missing required peer is a loud
  mount error naming pkg + peer + the fix; silence is reserved for
  absent *optional* peers, which is the feature.
- **Bundles pack v3** — [`03-plugin`](03-plugin/)'s `plugin/rut.toml`
  rides `format_version = 3` (RFC 0038's ledger): peer groups ride
  the archive; an older loader refuses rather than guess.

Not here, on purpose: registry/index deps and version-range selection
— `[peer-deps]` is a presence relation over mounted packages, not a
package manager (RFC 0045 OQ-1). The full record: the survey, the
diagnostics, and the test matrix in
[`docs/dep-kinds-report.md`](../docs/dep-kinds-report.md).

The earlier parse-only design corpus (`basic/`, `concurrency/`,
`workers/`, `network/`, `memory/`, `json/`, `gui/`, `host/`) was
removed: its implemented parts moved to the playground classics, and the
aspirational sketches live on in the RFCs that specified them (the
load-bearing snippets are inlined there). Git history keeps the rest.
