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

## The orphan rule — one of the pair is local

Every `impl Trait for Type { .. }` needs **at least one of the pair
defined in your pkg** — the trait's pkg or the type's pkg (RFC 0012
§2a, the compiler's law since module VERSION 9). Write your impl in
the pkg that owns a side: your own trait may speak about anyone's
type, and anyone's trait may speak about your own type — 02-digest's
`impl JsonSerialize for Json` is the type-local shape (json owns the
trait, `Json` is that file's).

- **Builtin types are in no pkg** — the primitives, `[T]`, `?T`,
  `opaque`. Only a trait of your own pkg may be implemented for them
  (json's twelve base impls — prims, `?T`, `[T]` — are the sanctioned
  shape); a foreign trait over a builtin head is an orphan. The
  asymmetry is deliberate: builtin *traits* (`Iterator`, `Index`,
  `Disposal`) are core's decls, so implementing one for your own type
  is the ordinary local case. `opaque` is a builtin too — a foreign
  trait can never be implemented for it; its cross-type law is
  RFC 0014's downcast (`opaque.downcast<T> -> ?T`, nil on a miss).
- **Generic impls classify by the head** — `impl JsonSerialize for
  Vec<T>` is json's to write (it does, peer-gated); your type
  parameter `T` never makes the impl yours.
- **Both foreign is an error**, named plainly:

  ```
  orphan impl: neither `JsonSerialize` nor `HashSet` is defined in this pkg — `JsonSerialize` is json's, `HashSet` is nmapset's; an `impl Trait for Type` needs at least one of the pair declared in its own pkg (RFC 0012 §2a)
  ```

The guarantee you get: a pkg's impl set is auditable — "who implements
`JsonSerialize` for `Vec<T>`" has one answer and a grep to prove it —
and adding a dependency adds the pairs its pkgs declare, never pairs a
stranger invented. The record: `docs/orphan-rule-report.md`.

## The std `json` package — the surface, the rulings, the run recipe

`rut/json/` is the ninth std pkg (after `core`, `calc`, `nmap_host`,
`nmapset`, `pouch`, `ink`, `rt`, `bench-cross`) — pure rut, zero host
fns, the RFC 0028 amendment is its law. Three entries and two traits:

```rut
fn encodeJson<T requires JsonSerialize>(v: T) -> (?str, ?EncodeJsonError);
fn decodeJson<T requires JsonDeserialize>(s: str) -> (?T, ?DecodeJsonError);
fn decodeJsonBytes<T requires JsonDeserialize>(b: bytes) -> (?T, ?DecodeJsonError);

trait JsonSerialize   { fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError; }
trait JsonDeserialize { fn decode(mut r: JsonReader) -> (?Self, ?DecodeJsonError); }
```

The locked rulings, user-visible as spells:

- **`(?T, ?E)` with EXACTLY-ONE-NIL**: success is `(value, nil)`,
  failure is `(nil, err)` — never a non-nullable `E`, never both
  halves filled. Your destructure reads the wire: `let (s, e) =
  encodeJson(v);` then check `e == nil`.
- **`Self`, not a `<T>` param, on `JsonDeserialize`** — you cannot
  decode an A-reader into B; the trait's static call IS the type.
- **The streams are never nilable** (`mut w: JsonWriter`, `mut r:
  JsonReader`): RFC 0044 sharing is the default; `?` only where
  absence is a legitimate state.
- **`decodeJsonBytes` is strict UTF-8** — `bytes.decode()` is lossy,
  so the bytes entry validates first (invalid octets → `InvalidUtf8`,
  never a silent U+FFFD). That is why there are two decode entries:
  RFC 0043 keeps `str | bytes` params illegal.
- **The error shapes** are RFC 0006's kind/details split:
  `DecodeJsonError { kind, at, got, expected }` and
  `EncodeJsonError { kind, at, path }` — `path` is the lazily-built
  `$.rows[3].name` spelling, assembled only on failure.

The fusion interplay: the happy path mints `(str, nil)` shaped
pairs — and in `return encodeJson(v);` the pair never exists: the
return-position destructure (02ced3e) dissolves the mint. Every
consumer of this pkg rides that lowering, which makes json the
fusion's real-world proof (the bench row's fuel is the witness).

The honest scoreboard: json is the bench suite's weakest lane vs
QuickJS — `json-roundtrip` runs **23.1×** qjs net (930.9 vs 40.3 ms
on the same 204 KB document). The pkg's DIRECT decode mints only the
program's own values, but every classify/carve is interpreter
dispatch, while the qjs twin rides the engine's own `JSON.parse` +
`JSON.stringify`. The row is the baseline future json-side engine
phases measure against (`benches/README.md`'s performance log has the
full entry).

The run recipe:

- **A module dir** (the full world): `[deps]` json by path — add
  `pouch`/`nmapset` to the same table when you want the `Vec<T>` /
  map-set impls; they are peer groups, mounted only when the peer is
  in your closure anyway. `rut run <dir>` runs it.
- **A loose file**: `rut run file.rut` with `use json::` in the
  source — the CLI mounts json and runs the peer gate
  (`assemble_peers`), same gating rules.
- **The worked example**: `cargo run -p digests`
  ([`02-digest/`](02-digest/)) — its encode half is the lib's
  `impl JsonSerialize for Json`, golden-tested against serde_json
  (9/9).
- **The bench row**: `node benches/run.mjs --workload json-roundtrip`.

The earlier parse-only design corpus (`basic/`, `concurrency/`,
`workers/`, `network/`, `memory/`, `json/`, `gui/`, `host/`) was
removed: its implemented parts moved to the playground classics, and the
aspirational sketches live on in the RFCs that specified them (the
load-bearing snippets are inlined there). Git history keeps the rest.
