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
  the example is untouched. (The retirement has since been EXECUTED by
  the todolist-restructure batch — the example's packages mount
  separately under the manifest route, appkit proven dead by
  P1/P2/P3; see `docs/todolist-restructure-report.md` §2.)
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
  trait can never be implemented for it, so a capability probe on a box
  finds nothing (`o is I` is `false`; `is` names the box, never the
  payload — the 2026-09 opaque-is law, RFC 0014); its cross-type law
  is RFC 0014's downcast (`opaque.downcast<T> -> ?T`, nil on a miss).
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

## The std `strbuild` package — the class contract, the mut law, the row

`rut/strbuild/` is the tenth std pkg (after `json`) — pure rut, zero
host fns, zero deps: the engine's `StrBuf` builder cell is AMBIENT
(RFC 0028 revised, builtin-surface), so the pkg compiles against
`mount_std_core` alone and a strbuild mount adds no `expected_host_fns`
entries and no dep edge of its own. The whole contract is one class:

```rut
pub class StringBuilder {
    out: StrBuf;   // the engine cell — the whole pkg is this field
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
RECORD — no nat, no call site (adding one is a VERSION conversation
for zero need). The rulings, user-visible as spells:

- **The mut law is RFC 0044 sharing, not copying.** A binding shares
  the ONE instance cell, whose `out` slot shares the ONE engine cell:
  appends through an alias — or through a `mut b: StringBuilder`
  parameter — land in the CALLER's document, visible through the
  original. Copies happen at exactly two engineered points:
  `build()`'s materialization and `grow`'s prefix move. `build()`
  twice answers the same text; the built `str` is immune to later
  appends.
- **`with_cap` is a hint, not a promise** — honored as the mint
  allocation (the block store class-rounds it), advisory as behavior:
  every cap answers identical content and `len`; a negative hint is
  the nat's `Invalid` trap.
- **`append_code` inherits the engine's rule**: a surrogate mints
  U+FFFD at the nat — the `str.from_code` rule, decided below the
  pkg.
- **`inline = true` is load-bearing**: a class-method module cannot
  be linked; the flag keeps the methods resolvable at every
  consumer's call site.

json is the reference consumer: its writer's accumulator IS a
`StringBuilder` field (`[deps] strbuild` — the peer groups are
untouched, they never touch the builder). Honest accounting rides
with it: the class face costs a measured +432,024 fuel (+1.370%) on
`json-roundtrip` over the raw cell — engine-lowering cost, menued for
a future engine batch (`docs/strbuild-phase1.md`,
`docs/strbuild-report.md`), checksums immovable throughout.

The scoreboard is the suite's surprise: on the pkg's own row,
**rut is AHEAD of QuickJS** — 0.42× net (352.6 vs 842.5 ms; node's
JIT leads at 233.1 ms). The JS idiom (`Array#push` + `Array#join("")`
per round) re-materializes the whole rope per round; the builder
appends in place and materializes once. Pins: fuel 162,285,165, VM
heap peak 22,028,108 B (`benches/README.md`'s strbuild sections have
the full entry + the optimization ledger).

The run recipe:

- **A module dir**: `[deps]` strbuild by path — `rut run <dir>`.
- **A loose file**: `rut run file.rut` with `use strbuild::` in the
  source — the CLI mounts strbuild by presence, same as json.
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
  became TRUE of the file (`docs/nmap-hostvals-report.md` §7; the
  measured record: fuel −30…−41%, prim-row VM heap to HUNDREDS of
  bytes, in `benches/README.md`'s nmap-hostvals section). (An
  alias-row form that routed the 64-bit vals to native-val-column
  classes was repealed the day it landed — one name = one type — and
  the column classes themselves were REMOVED with the takeover,
  unreached by any public spelling since the repeal; the columns'
  host crossings remain as legacy escape hatches —
  `docs/type-name-law-report.md` §5, §8, `docs/nmapset-hostops-report.md`.)
- **The map is an `opaque`, and `opaque` has two kinds.** The `t`
  field addresses a store entry of the Host kind — a HostOpaque (the
  map's own host payload, one per map, borrow-checked by Rust's TypeId
  on every crossing); the other kind, RutOpaque, is a rut value held
  host-side. Two kinds, two identity worlds — TypeId exists only on
  the Host path; rut-side `downcast` and `is` never consult it
  (RFC 0014's amendment, RFC 0023's nmap-hostvals amendment).
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
were real and priced (fuel +15-18 %, 324 B vs MiB —
`docs/type-name-law-survey.md` §4); the columns' host CROSSINGS stay
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
`docs/type-name-law-report.md` §6 and `benches/README.md`'s
type-name-law section; the sidecar's story ends with the nmap-hostvals
migration, whose own record is in `docs/nmap-hostvals-report.md`).
Run recipe:

- **A module dir**: `[deps]` nmapset by path — `rut run <dir>`.
- **A loose file**: `rut run file.rut` with `use nmapset::` in the
  source — the CLI mounts nmapset by presence, same as json.
- **A taste in ten lines**:
  `let mut m = HashMap<str, i64>.new(); m.put(k, v); let v = m.get(k);`
- **The bench rows**: `node benches/run.mjs --workload
  nmapset-int`, `--workload nmap-hashset`, `--workload
  nmap-knucleotide` (the `nmap-primmap` twin was RETIRED with the
  nmapset-hostops takeover — see `benches/README.md`'s section).

The earlier parse-only design corpus (`basic/`, `concurrency/`,
`workers/`, `network/`, `memory/`, `json/`, `gui/`, `host/`) was
removed: its implemented parts moved to the playground classics, and the
aspirational sketches live on in the RFCs that specified them (the
load-bearing snippets are inlined there). Git history keeps the rest.
