# rut examples — the runnable projects

Six Cargo projects, six workspace members — six runnable end to end
(five on the console, one in a browser), plus one parse-only corpus
example:

| Project | Run | Demonstrates |
|---|---|---|
| [`00-todolist/`](00-todolist/) | `cargo run -p todolist` | an `entry fn` surface over a rut `class` — the host drives CRUD through `opaque` handles |
| [`01-sort/`](01-sort/) | `cargo run -p sort` | a sorting library behind one dispatcher entry; `Result` at the boundary, known-input gates |
| [`02-digest/`](02-digest/) | `cargo run -p digests` | byte-level codecs and hashes (MD5/SHA/base64/CRC/FNV), the host as test oracle |
| [`03-plugin/`](03-plugin/) | `cargo run -p plugin` | a module directory + [`.rutbundle`](../docs/src/reference/bundles.md) chat-moderator plugin; re-entrant `vm.call`, both `opaque` directions |
| [`04-custom-async/`](04-custom-async/) | parse-only — no runnable harness yet (the disclosed follow-up); the dir carries its `rut.toml` for the shape law | a hand-written `impl Future<nil> for CustomFuture` plus a user launcher with per-checkpoint stats and cancellation audits — the user-impl-of-the-builtin-`Future`-trait test. User futures are launcher-drivable; `await` targets engine-woven futures in v1 (join not yet landed) |
| [`05-todolist-web/`](05-todolist-web/) | `cargo test -p todolist-web` + `node tests/e2e-browser.mjs` | the full page app: a todolist with a simulated server (request table + per-kind `tim_after` latency) whose brain is pure rut — ten DOM/timer crossings over web_sys on wasm32, the fake-DOM twin as the cargo gate, a through-the-artifact e2e in node and Firefox headless; the wasm mirror takes the `nmap_host` surface from the committed CDN artifact (`mount_bundle_bytes` over `include_bytes!`) |
| [`06-github-viewer-cli/`](06-github-viewer-cli/) | `cargo run -p rgh -- --repo=… --ref=… list` | `rgh` — a GitHub viewer over the jsDelivr CDN whose brain is rut (`rgh.rut`, an ASYNC free fn over the redesigned std `rut/http` lane): argv carving, the tree JSON decode, and the human-size formatter run in the VM; `send` resolves at headers, the list drains in one body await, the download walks the byte stream chunk by chunk through the sync `append_file` row; the embedder launches the brain (`boot` + `launch_future`), pumps the loop to idle, exits with the brain's i32; the offline suite rides the fixture lane keyed on method+URL with virtual-clock chunk arrival; the std closure mounts through the project manifest (`rut.toml` — the url carrier), with `http` riding the committed CDN bundle (sha256-pinned) and the generic owners on path rows |

Every rut program here is a module dir with a `rut.toml` — the
manifest is the deps carrier. `00`/`01`/`02` declare their third-party
pkgs as PATH rows (`pouch`, plus `json` for `02` — the generic owners;
the url flip for those is a separate, still-pending engine plan, and
`calc` takes no row anywhere: it is ambient, the embedder mounts std),
and their embedders (`src/main.rs`, `tests/session.rs`) mount through
the manifest lane (`load_dir_session`) instead of in-code `mount_dir`
calls — same host half, new mount lane. `04`'s manifest is the shape
law alone: parse-only, nothing loads that dir yet.

Short, self-contained programs — the classics — live in
[`demo/src/examples/`](../demo/src/examples/): the playground imports
them raw, and `crates/rut-cli/tests/playground.rs` compiles and runs
every one against its `.expected` sidecar. The parser conformance suite
([the frontend](../docs/src/reference/frontend.md)) walks both trees.

## Std from a CDN — what a url row delivers

The std tree ships as committed per-package bundles
(`dist/std/<pkg>.rutbundle`, packed + freshness-gated by
`scripts/pack-std.cjs`), served by jsDelivr at an immutable
`@std-vNN` tag and pinned by sha256 — the pin is law at the mount door
on every load, offline or not (the committed artifact IS the cache; a
project manifest is the only url carrier, and the embedder seeds the
fetcher from it — [embedding](../docs/src/reference/embedding.md)).
The boundary is the engine's owner-anchored instantiation law: a
compiled bundle serves exactly what its own pack closure spelled, so
the CDN delivers **host surfaces** (the v6 decl bundles) and
**concrete-class libs** (`http` — 06 rides it), while the **generic
owners** (`pouch`, `nmapset`, `json`, `async_host`) stay path rows —
a consumer's `Vec<Todo>` or `launch_future<T>` shape compiles on
demand from source and can never ride a binary
([dependency kinds](../docs/src/reference/dependency-kinds.md),
[module bundles](../docs/src/reference/bundles.md)).

## Packages and manifests — the three dep kinds

Every package carries a `rut.toml` (the
[project structure reference](../docs/src/reference/project-structure.md)
— `03-plugin`'s `plugin/` and `server/` are the in-tree examples). Since
the dep-kinds batch, a manifest relates to other packages through three
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
  the example is untouched. (The retirement has since been EXECUTED by
  the todolist-restructure batch — the example's packages mount
  separately under the manifest route, appkit proven dead by
  P1/P2/P3.)
- **Required-by-default peers** — a missing required peer is a loud
  mount error naming pkg + peer + the fix; silence is reserved for
  absent *optional* peers, which is the feature.
- **Bundles pack v3** — [`03-plugin`](03-plugin/)'s `plugin/rut.toml`
  rides `format_version = 3` ([bundles](../docs/src/reference/bundles.md)):
  peer groups ride
  the archive; an older loader refuses rather than guess.

Not here, on purpose: registry/index deps and version-range selection
— `[peer-deps]` is a presence relation over mounted packages, not a
package manager.

## The orphan rule — one of the pair is local

Every `impl Trait for Type { .. }` needs **at least one of the pair
defined in your pkg** — the trait's pkg or the type's pkg (the
[traits reference](../docs/src/reference/traits.md), the compiler's law
since module VERSION 9). Write your impl in
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
  trait can never be implemented for it, so a capability probe on a box
  finds nothing (`o is I` is `false`; `is` names the box, never the
  payload — the 2026-09 opaque-is law, [opaque](../docs/src/reference/opaque.md));
  its cross-type law is [opaque's downcast](../docs/src/reference/opaque.md)
  (`opaque.downcast<T> -> ?T`, nil on a miss).
- **Generic impls classify by the head** — `impl JsonSerialize for
  Vec<T>` is json's to write (it does, peer-gated); your type
  parameter `T` never makes the impl yours.
- **Both foreign is an error**, named plainly:

  ```
  orphan impl: neither `JsonSerialize` nor `HashSet` is defined in this pkg — `JsonSerialize` is json's, `HashSet` is nmapset's; an `impl Trait for Type` needs at least one of the pair declared in its own pkg
  ```

The guarantee you get: a pkg's impl set is auditable — "who implements
`JsonSerialize` for `Vec<T>`" has one answer and a grep to prove it —
and adding a dependency adds the pairs its pkgs declare, never pairs a
stranger invented. The full law: [traits and
dispatch](../docs/src/reference/traits.md).

## The std `json` package — the surface, the rulings, the run recipe

`rut/json/` is the ninth std pkg (after `core`, `calc`, `nmap_host`,
`nmapset`, `pouch`, `ink`, `rt`, `bench-cross`) — pure rut, zero host
fns, the [stdlib rules](../docs/src/reference/stdlib.md) are its law.
Three entries and two traits:

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
  JsonReader`): [by-reference](../docs/src/reference/by-reference-and-nullable.md)
  sharing is the default; `?` only where
  absence is a legitimate state.
- **`decodeJsonBytes` is strict UTF-8** — `bytes.decode()` is lossy,
  so the bytes entry validates first (invalid octets → `InvalidUtf8`,
  never a silent U+FFFD). That is why there are two decode entries:
  [type aliases](../docs/src/reference/type-aliases.md) keep `str | bytes`
  params illegal.
- **The error shapes** are the kind/details split
  ([enums](../docs/src/reference/enums.md)):
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
  in your closure anyway. `rut run <dir>` runs it, and the peer gate
  (`assemble_peers`) applies the same gating rules.
- **The worked example**: `cargo run -p digests`
  ([`02-digest/`](02-digest/)) — its encode half is the lib's
  `impl JsonSerialize for Json`, golden-tested against serde_json
  (9/9).
- **The bench row**: `node benches/run.mjs --workload json-roundtrip`.

## The std `strbuild` package — the class contract, the mut law, the row

`rut/strbuild/` is the tenth std pkg (after `json`) — a rut class over
the HOST builder (the `ink`/`Logger` pattern): the `strbuild_host`
decl pkg declares the five rows (`sb_new`/`sb_push`/`sb_push_code`/
`sb_len`/`sb_finish`, registered under the pkg-name scope), the
bodies live in `rut-std` (`strbuild::pkg()`), and core ships zero
string-building machinery. The whole rut contract is one class:

```rut
pub class StringBuilder {
    out: opaque;   // the host box — the whole pkg is this field
}

impl StringBuilder {
    fn new() -> Self;                   // grow from small
    fn with_cap(cap: i32) -> Self;      // pre-size to an octet HINT
    fn append(mut self, value: str);    // the one appender (in place,
                                        //   amortized O(|s|))
    fn append_code(mut self, cp: u32);  // one codepoint (invalid
                                        //   scalars mint U+FFFD)
    fn len(self) -> i32;                // codepoints so far, O(1)
    fn build(self) -> str;              // the ONE materialization —
}                                       //   the builder KEEPS its buffer
```

That is the whole surface, closed on the record:
`clear`/`reserve`/`capacity`/`append_char`/`append_i64` are ABSENT ON
RECORD — no host fn, no call site (adding one is a surface conversation
for zero need). The rulings, user-visible as spells:

- **The mut law is [by-reference sharing](../docs/src/reference/by-reference-and-nullable.md),
  not copying.** A binding shares
  the ONE instance cell, whose `out` slot shares the ONE host box:
  appends through an alias — or through a `mut b: StringBuilder`
  parameter — land in the CALLER's document, visible through the
  original. Copies happen at exactly two engineered points:
  `build()`'s materialization and `grow`'s prefix move. `build()`
  twice answers the same text; the built `str` is immune to later
  appends.
- **`with_cap` is a hint, not a promise** — honored as the mint
  allocation (the host class-rounds it, engine-style), advisory as
  behavior: every cap answers identical content and `len`; a negative
  hint is the host fn's `Invalid` trap.
- **`append_code` inherits the engine's rule**: a surrogate mints
  U+FFFD at the host fn — the `str.from_code` rule, decided below the
  pkg.
- **Growth is budgeted**: every geometric grow consults the embedder's
  heap budget BEFORE it grows (`charge_public`), so the wasm cap
  governs builder growth exactly as it governs engine allocations.

json is the reference consumer: its writer's accumulator IS a
`StringBuilder` field (`[deps] strbuild` — the peer groups are
untouched, they never touch the builder).

The scoreboard is the suite's surprise: on the pkg's own row,
**rut is AHEAD of QuickJS** — 0.42× net (352.6 vs 842.5 ms; node's
JIT leads at 233.1 ms). The JS idiom (`Array#push` + `Array#join("")`
per round) re-materializes the whole rope per round; the builder
appends in place and materializes once. Pins: fuel 162,285,165, VM
heap peak 22,028,108 B (`benches/README.md`'s strbuild sections have
the full entry + the optimization ledger).

The run recipe:

- **A module dir**: `[deps]` strbuild by path — `rut run <dir>`.
- **A taste in ten lines**:
  `let mut b = StringBuilder.with_cap(1024); b.append(f"..."); ...; let s = b.build();`
- **The bench row**: `node benches/run.mjs --workload strbuild`.

## The keyed collections — one family, the values in the entries

The std `nmapset` package (no deps of its own) is rut's keyed-collection
surface, and it spells ONE way: `HashMap<K, V>` and `HashSet<T>` — one
class per name (one name = one type). The key type parameter admits
the closed set (`i8 | … | bytes`, compile-time at every
instantiation), and every val lives IN the host table's entries:

- **One crossing carries key AND value.** Since the nmap-hostvals
  value migration the wrapper is pure delegation (`HashMap { t:
  opaque }`) — put/get/has/remove/len are each ONE host crossing: a
  prim `V` crosses as the 8 slot bytes (zero cells, zero copy), a
  reference `V` shares its OWN cell into the entry — aliasing IS the
  cell (two gets of one key name ONE cell, writes through a recovered
  value land in the map, values release in-crossing on replace/remove
  and at the map's death). `HashMap<K, i64>`, `<K, u64>`, `<K, f64>`,
  an `i32` val, a record `Pt` val, a bare `V` in a generic body — the
  same machinery at different `V`. The old rut-side `[?V]` sidecar is
  HISTORY: handle-indexed and append-only after the nmapset-hostops
  takeover (whose headline was killing the grow drain — `refvals`
  exec −32%), deleted outright by the value migration — the migration
  is also when nmapset-hostops phase 2's "zero loops" claim first
  became TRUE of the file (the
  measured record: fuel −30…−41%, prim-row VM heap to HUNDREDS of
  bytes, in `benches/README.md`'s nmap-hostvals section). (An
  alias-row form that routed the 64-bit vals to native-val-column
  classes was repealed the day it landed — one name = one type — and
  the column classes themselves were REMOVED with the takeover,
  unreached by any public spelling since the repeal; the columns'
  host crossings remain as legacy escape hatches.)
- **The map is an `opaque`, and `opaque` has two kinds.** The `t`
  field addresses a store entry of the Host kind — a HostOpaque (the
  map's own host payload, one per map, borrow-checked by Rust's TypeId
  on every crossing); the other kind, RutOpaque, is a rut value held
  host-side. Two kinds, two identity worlds — TypeId exists only on
  the Host path; rut-side `downcast` and `is` never consult it
  ([opaque](../docs/src/reference/opaque.md) and
  [value boundary](../docs/src/reference/value-boundary.md) law).
- **`HashSet<T>`** is the host table alone, no machinery about vals.
- **May spell differently than you expect**: no float KEYS (the
  equality contract), no narrow-val columns, no bool val column (the
  `?bool` nil-law gap), and no iteration surface yet (json's map
  ENCODE waits on it).

The internal column classes (`Prim`-prefixed, one per val kind) were
REMOVED in the nmapset-hostops batch — they had been nmapset-private
implementation names, spelled nowhere in a public surface (grep-pinned,
LSP completing only the family), and went UNREACHED the day the alias
rows were repealed: every spelling resolved to the family class and its
sidecar, so the classes carried dead weight. Their native val columns
were real and priced (fuel +15-18 %, 324 B vs MiB); the columns' host
CROSSINGS stay
bound as legacy escape hatches, and a differently-named public column
class remains the recorded future shape. The set never had a prim-named
twin; `HashSet` has always been the one spelling.


One compat note: json's decode-side diagnostics did NOT move — a map
keyed by `bytes` diagnoses exactly as before ("this key type has no
JSON spelling"), because decode rides the ONE generic map impl and
its `key_spells` branch (json's three val impls folded into it when
the rows left; the `dec_hm_bytes_key` pin holds verbatim).

Consumer-visible behavior of a put/get/has is the exact class the
spelling names — `HashMap<i32, i64>` IS the generic class, one
crossing per op, values in the entries. The repeal's movers, on
record: checksums IMMOVABLE (the op
stream does not move — `nmap-primmap`'s pin stayed `734932704`),
fuel/heap re-seated to the sidecar's measured cost (17,950,301 →
20,903,285 fuel; 324 B → 3,539,324 B heap —
`benches/README.md`'s
type-name-law section; the sidecar's story ends with the nmap-hostvals
migration).
Run recipe:

- **A module dir**: `[deps]` nmapset by path — `rut run <dir>`.
- **A taste in ten lines**:
  `let mut m = HashMap<str, i64>.new(); m.put(k, v); let v = m.get(k);`
- **The bench rows**: `node benches/run.mjs --workload
  nmapset-int`, `--workload nmap-hashset`, `--workload
  nmap-knucleotide` (the `nmap-primmap` twin was RETIRED with the
  nmapset-hostops takeover — see `benches/README.md`'s section).

## The std `http` lane — two pkgs, one feature, an async-only face

`rut/http_host/` and `rut/http/` are the thirteenth and fourteenth
std pkgs (grep `rut/` — both count), redesigned around the host
future lane (`pub host async fn` + `register_async!`). They arrive
as a PAIR:

- **`http_host`** — the pure declaration surface: THREE async rows
  (the request, the drain, the stream read — each expands into its
  `__start`/`__yield`/`__take`/`__cancel` family the weave drives)
  and FIVE sync readbacks, all concrete over the crossing set:

  ```rut
  pub host async fn http_send(c: opaque, method: str, url: str,
                              headers: str, body: bytes) -> opaque;
  pub host async fn http_body(r: opaque) -> bytes;
  pub host async fn http_stream_next(s: opaque) -> ?bytes;

  pub host fn http_status(r: opaque) -> i32;      // 0 = transport error
  pub host fn http_err(r: opaque) -> ?str;        // nil unless status 0
  pub host fn http_read_err(r: opaque) -> ?str;   // sticky mid-read failure
  pub host fn http_resp_stream(r: opaque) -> opaque; // mints the reader
  ```

  The response handle is an `opaque` payload owning
  `{ status, err, chunk source }`; status **0 is RESERVED for
  transport failure** (0 is never a real HTTP status) — an HTTP
  status, any 4xx/5xx included, is not a transport failure, so `err`
  stays nil. `http_send` resolves at HEADERS (the wire body stays
  UNREAD); the chunk source is the sequenced-Completer sibling — a
  shared queue plus one pending completer slot per read — so the
  stream lane walks the body chunk by chunk with bounded memory end
  to end. THE ONE-SHOT LAW: `body()` (one future, the whole drain)
  xor `byte_stream()` per response — a late/second taker DEGRADES
  (empty drain / a dead reader whose `next` is an immediate EOF),
  never traps, disclosed. A mid-read wire death is DATA: `next`
  answers nil, `body` the short drain, `http_read_err` goes non-nil
  and stays (sticky) — never a trap; a failed completer is NOT the
  mid-read path.
- **`http`** — the rut face over the handles (the ink pattern,
  `inline = true` — the class-method law), ASYNC-ONLY and
  unsuffixed: only the operations that really wait are async points;
  everything else is sync construction sugar. `HttpClient.new()`
  with the five verbs as BUILD sugars (`get`/`post`/`put`/`patch`/
  `del` — sync, no I/O, each a one-step `RequestBuilder`), the
  chainable builder (`method`/`url`/`header` — repeatable —
  /`body`), `build()` freezing a re-sendable `Request`, and the
  async points: `send(cx) -> Response` (THE one — resolves at
  headers), `body(cx) -> bytes` (the drain), and on the minted
  `ByteStream`: `next(cx) -> ?bytes` (nil = EOF-or-failed;
  `error()` reads the same sticky fact). NO `text()` — callers
  await `body()` then `.decode()`; NO query/params struct; NO
  `_async` suffixes anywhere. The plan's usage chain compiles
  verbatim:

  ```rut
  let client = HttpClient.new();
  let resp = await client.request()
      .method(ClientQueryMethod.Post)
      .url("https://example.com/api")
      .header("Accept", "application/json")
      .body(payload)
      .build()
      .send(cx);
  // terse: await client.get(u).build().send(cx)
  ```

  One engine adaptation, disclosed: async METHODS are not woven in
  this build (the v1 diagnostic — the loop's tasks are async free
  fns), so each async point is a sync method whose body calls the
  private async free fn below it — the call mints the engine frame
  inside the method and the method returns it typed `Future<T>`; the
  caller's `await` consumes it (the `sleep` surface's exact
  crossing). The face the caller sees is the async-only one. And the
  DELETE verb is spelled `del`: `delete` is a REMOVED word in rut's
  grammar (the dynamic-property statement; the `out`-not-`print`
  precedent) — the same crossing under an honest name, the wire
  still sees the canonical `DELETE`.

The bodies live in rut-std, behind the DEFAULT-OFF `http` cargo
feature: `reqwest` 0.12 (blocking, rustls-tls, gzip, brotli —
redirects on) folds into `rut_std::http`, and the feature appears in
NO default graph — reqwest-blocking does not build on
wasm32-unknown-unknown, so rut-driver and rut-wasm stay clean. The
reqwest lane is ONE worker thread per request over the shared
blocking client: the thread sends, completes the send completer at
HEADERS, then keeps reading the body into the chunk source through a
fixed 16 KiB buffer (bounded memory; each read lands as one stream
chunk). Cancellation rides the cancel arms as the disclosed
best-effort: the thread finishes its blocking read and the late
result is simply never taken. The async rows register through
`rut_vm::register_async!` (the ONE closure law): the embedder's
whole side is one closure answering a `Completer` per row family —
the sync readbacks stay plain `register!`. An embedder that wants
its own transport — tests above all — calls
`http::pkg_with(f)` where `f: Fn(&str, &str, &str,
&[u8]) -> Result<FixtureReply, String>` maps (method, url, headers,
body) to a recorded reply — status + the CHUNK PLAN (deterministic
small chunks) — or `Err(message)` (the status-0 lane): the fixture
map's keys ARE the assertions, and the returned `HttpFixture` handle
is the virtual clock (the test loop settles the dues and advances to
`next_due()` — the send at tick 0, chunk i at tick i+1). Zero
network.

The run recipes:

- **A module dir**: `[deps] http = { path = ".../rut/http" }` —
  `rut run <dir>`; http's own `[deps]` pulls http_host AND strbuild
  (the builder's header accumulator). The fetch sites `await`, so
  the program's entry runs under the launcher (`use async_host::
  launch_future`).
- **The worked example**: [`06-github-viewer-cli/`](06-github-viewer-cli/)
  — `rgh` consumes the pair from an embedder (`http::pkg()`, the
  reqwest lane) plus example-local CLI-I/O rows, its brain an async
  free fn launched through `boot` + `launch_future`, with the fixture
  lane (`http::pkg_with`, keyed on method+URL) as its offline
  test gate. See that example's README for the division of labor and
  the laws.

```rut
use http::HttpClient;
use async_host::launch_future;
use ink::Logger;

async fn fetch(cx: RunContext, log: Logger, url: str) -> nil {
    let client = HttpClient.new();
    let r = await client.get(url).build().send(cx);
    let e = r.transport_error();
    if (e != nil) {
        let why: str = e;
        log.error(why);
        return;
    }
    let b = await r.body(cx);
    log.info(f"status={r.status()} ok={r.ok()} len={b.len()}");
}

pub fn main() {
    let log = Logger.new("demo");
    launch_future(fetch(log, "https://example.com"));
}
```

The earlier parse-only design corpus (`basic/`, `concurrency/`,
`workers/`, `network/`, `memory/`, `json/`, `gui/`, `host/`) was
removed: its implemented parts moved to the playground classics, and
git history keeps the rest.
