# rut examples — the runnable projects

Four Cargo projects, four workspace members, each runnable end to end —
from the first line of its `.rut` to the last `Value` out of the VM —
plus one parse-only corpus example:

| Project | Run | Demonstrates |
|---|---|---|
| [`00-todolist/`](00-todolist/) | `cargo run -p todolist` | an `entry fn` surface over a rut `class` — the host drives CRUD through `opaque` handles |
| [`01-sort/`](01-sort/) | `cargo run -p sort` | a sorting library behind one dispatcher entry; `Result` at the boundary, known-input gates |
| [`02-digest/`](02-digest/) | `cargo run -p digests` | byte-level codecs and hashes (MD5/SHA/base64/CRC/FNV), the host as test oracle |
| [`03-plugin/`](03-plugin/) | `cargo run -p plugin` | a module directory + `.rutbundle` (RFC 0038) chat-moderator plugin; re-entrant `vm.call`, both `opaque` directions |
| [`04-custom-async/`](04-custom-async/) | parse-only — runnable at M3 | a hand-written `impl Task<T> for CustomTask<T>` plus a user launcher with per-checkpoint stats and cancellation audits; the user-impl-of-builtin-trait test. Becomes runnable when the async plan lands (the engine context is runtime-provided) |

Short, self-contained programs — the classics — live in
[`demo/src/examples/`](../demo/src/examples/): the playground imports
them raw, and `crates/rut-cli/tests/playground.rs` compiles and runs
every one against its `.expected` sidecar. The parser conformance suite
(RFC 0030 §7) walks both trees.

The earlier parse-only design corpus (`basic/`, `concurrency/`,
`workers/`, `network/`, `memory/`, `json/`, `gui/`, `host/`) was
removed: its implemented parts moved to the playground classics, and the
aspirational sketches live on in the RFCs that specified them (the
load-bearing snippets are inlined there). Git history keeps the rest.
