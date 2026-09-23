# rut-json — phase 0 survey: the decode model, the reader/writer internals, the number and depth policies, the impl matrix, the mount/version call, the migration census, the bench method

*Phase 0 of the rut-json batch (docs only — no code). Base `23f433f`,
binary `VERSION 8` (`crates/rut-core/src/binary.rs:431`), bundles v3,
8 std pkgs embedded. Scratch: `/tmp/opencode/batch-rut-json/p0/`.
Everything below cites the tree as it stands; the design sections
propose, they do not describe. Phases 1–3 implement; this document is
their contract.*

---

## 0. The law (the user's ruling — verbatim, not negotiable)

The converged surface — json is the 9th std pkg (after `core`, `calc`,
`nmap_host`, `nmapset`, `pouch`, `ink`, `rt`, `bench-cross`):

```rut
// rut/json — the 9th std pkg (after core, calc, nmap_host, nmapset,
// pouch, ink, rt, bench-cross)
fn encodeJson<T requires JsonSerialize>(v: T) -> (?str, ?EncodeJsonError);
fn decodeJson<T requires JsonDeserialize>(s: str) -> (?T, ?DecodeJsonError);
fn decodeJsonBytes<T requires JsonDeserialize>(b: bytes) -> (?T, ?DecodeJsonError);

trait JsonSerialize {
    fn encode(self, w: mut JsonWriter) -> ?EncodeJsonError;
}
trait JsonDeserialize {
    fn decode(r: mut JsonReader) -> (?Self, ?DecodeJsonError);
}
```

THE LOCKED RULINGS (embedded verbatim):

- **(?T, ?E) with the EXACTLY-ONE-NIL invariant**: success = (value,
  nil), failure = (nil, err). Never a non-nullable E.
- **JsonDeserialize uses `Self`, NOT a `<T>` param** (RFC 0012; a T
  param would permit decoding an A-reader into B — a footgun).
- **Streams are NEVER nilable**: `w: mut JsonWriter`,
  `r: mut JsonReader`. RFC 0044 sharing is the DEFAULT; `?` only where
  absence is a legitimate state. GENERAL LAW for this pkg.
- **`str` | `bytes` params ILLEGAL** (RFC 0043: unions are bound-only)
  — hence the two decode entries; **decodeJsonBytes is STRICT UTF-8**
  (`bytes.decode()` is lossy — never silently corrupt; invalid =
  `InvalidUtf8` err).
- **encode RETURNS `?EncodeJsonError` (not nil)**: the rc heap can be
  cyclic (RFC 0017) — depth-limit is EXPECTED failure, data not trap.
- **Error types: payloadless enum (kind) + fixed-field struct
  (details)** — RFC 0006 has NO data-carrying enums:

  ```rut
  enum DecodeErrorKind { Unexpected, Truncated, InvalidUtf8, WrongType, Depth, Trailing }
  struct DecodeJsonError { kind: DecodeErrorKind; at: i64; got: str; expected: str }
  enum EncodeErrorKind { Depth }   // + what the census adds (§2.3: NotFinite, KeyUnsupported)
  struct EncodeJsonError { kind: EncodeErrorKind; at: i64; path: str }
  ```

- **DEPENDENCY DIRECTION (the serde model, final)**: json -> pouch,
  json -> nmapset, ALL container impls IN json (`impl Vec<T>:
  JsonSerialize` etc.), orphan-legal because the TRAIT is local.
  Containers gain nothing. User types impl json's traits on their own
  types. REJECTED on record: runtime reflection (RFC 0037, too slow),
  engine-woven builtin traits, containers -> json (layering smell).
- **The impl matrix**: prims i64/f64/bool/str, ?T (nil -> null), [T],
  Vec<T> (pouch), HashMap/HashSet/PrimMap (nmapset; str keys per
  json's own shape; prim-key maps encode keys as strings — the policy
  is set in §2.6).

---

## 1. The census today

### 1.1 The std surface as it stands — eight packages, json the ninth

| pkg | entry | source | host fns | mounted by |
|---|---|---|---|---|
| `core` | `entry.type` decl | `rut/core/core.d.rut` | none (compiler-lowered prelude) | `mount_std_core` — unconditionally, engine-side |
| `calc` | `entry.type` decl | `rut/calc/calc.d.rut` | `Math.*` in `rut-std::math` | `mount_calc` (inside `mount_std`) |
| `nmap_host` | `entry.type` decl | `rut/nmap_host/nmap.d.rut` | `nmap_host::*` in `rut-std::nmap` | hosts / dep walk (nmapset's `[deps]`) |
| `rt` | `entry.type` decl | `rut/rt/rt.d.rut` | `rt::log` etc. in `rut-std::logger` | hosts / dep walk (ink's `[deps]`) |
| `bench_cross` | `entry.type` decl | `rut/bench-cross/bench_cross.d.rut` | nops in `rut-std::bench_cross` | hosts / benches |
| `pouch` | `entry.lib` source | `rut/pouch/pouch.rut` | none | consumer `[deps]` / `mount_dir` |
| `nmapset` | `entry.lib` source, `inline = true` | `rut/nmapset/nmapset.rut` | none of its own (rides nmap_host) | consumer `[deps]` / `mount_dir` |
| `ink` | `entry.lib` source | `rut/ink/ink.rut` | none of its own (rides rt) | consumer `[deps]` / `mount_dir` |
| **`json`** | **`entry.lib` source (this design, §2.7)** | **`rut/json/json.rut` + two group files** | **none — pure rut, zero host fns** | consumer `[deps]` / `mount_dir` + peers |

json's zero-host-fn shape matters: unlike ink (which drags `rt`) or
nmapset (which drags `nmap_host`), a json mount adds no
`expected_host_fns` entries — the probe's `verify_against` exactness
law (RFC 0025) is untouched by a json consumer. Only the group files
require the *peers'* presence, and peers are rut-only too.

### 1.2 The mount chains — where json lands (CLI and probe verified)

Three chains mount the tree pkgs; all three are census points for
phase 1:

1. **Graph loads (module dirs/bundles)** — `load_dir_session`
   (`crates/rut-driver/src/loader.rs:55`): pass 1 the `[deps]` walk,
   pass 2 the root's `[dev-deps]`, pass 3 `run_peer_gate`
   (`loader.rs:575`, one post-closure pass, appends group texts via
   `session.append_source`), pass 4 compile. Nothing to change here —
   json's manifest (§2.7) rides RFC 0045 verbatim; `json-decode`'s
   successor row (§2.10) is the first std-pkg-with-peers consumer.
2. **CLI single-file convenience** — `crates/rut-cli/src/main.rs:104–117`:
   on `src.contains("use {name}::")` the CLI mounts tree pkgs from the
   hardcoded list `[("ink", "rut/ink"), ("pouch", "rut/pouch")]` via
   `mount_dir`. **`mount_dir` records peer declarations but never runs
   the gate** (`loader.rs:635–664`: "an OFFER to someone else's
   program… the peer gate does not run here"). A loose file that
   `use json::` today would get json's base only and NO container
   impls — its `Vec<T>` encode would fail admission loudly at compile.
3. **Probe single-file convenience** — `benches/probe/src/main.rs:142–158`:
   same shape, mounting ink then pouch for loose `.rut` workloads; dir
   workloads go through `load_path_session` and the full graph passes.

**Decision (the loose-file gate):** phase 1 adds a small driver-level
entry — `rut_driver::assemble_peers(&mut session)` — a public wrapper
that runs the existing gate's append pass over the session's recorded
peer declarations (the mount paths are already in the registry's
`PeerDecl`s; the session side needs the pkg dirs `mount_dir` visited,
one `BTreeMap<String, PathBuf>` beside the peer registry). The CLI and
probe call it after their tree mounts. No wire change, no loader-semantics
change — the same append pass, offered to embedders whose world grows
incrementally. The fallback (loose files get base-only json, container
impls require a module dir) is recorded but rejected: the demo cases
and `benches/workloads` rely on the CLI being a full host, and a std
pkg whose containers don't work in a loose file is a trap for every
first-time user.

**The order ruling:** json slots **after `pouch` and `nmapset`** in
every ordered list — the dep graph's new edges (json -> pouch,
json -> nmapset) make that the only graph-respecting position; `ink`
is independent and stays where it is. Concretely: the CLI list becomes
`[("ink", …), ("pouch", …), ("nmapset", …), ("json", …)]` (append two
entries — json is last); the probe the same; the LSP index array
(§2.9) inserts json after `nmapset`.

### 1.3 The JSON code in the tree today — the migration census targets

**`examples/02-digest/digest.rut` (1048 lines) — the private json.**
A hand-rolled DOM decoder/encoder, written before the lib and kept as
the err-channel batch's proof case:

- the DOM: `class Json` (tagged union by hand: `JTag` enum + `num`/
  `str`/`arr`/`keys`/`vals` fields, children boxed in `opaque`,
  RFC 0014, to break recursion);
- the cursor: `struct Cur { cs: Vec<str>; pos: i32; depth: i32 }` —
  the input is `split_chars`-split into 1-char str cells ONCE, and
  every peek/compare is a str-cell `==` chain (`is_digit` is up to ten
  one-char compares per digit);
- the parser: `jparse_value`/`jparse_lit`/`jparse_string`/
  `jparse_number`/`jparse_array`/`jparse_object` + `jskip` +
  `valid_number` + `join_chars` (per-char accumulator builds), depth
  limit **64** (`digest.rut:840`, `873`);
- the encoder: `jencode` (recursive, per-level f-string concat) +
  `jquote` (per-char `Vec<str>` + join);
- the entries: `json_dec` (`-> (?opaque, str)` — the err-channel
  phase 3's shape), `json_enc`, `json_roundtrip`;
- the consumers: `examples/02-digest/src/main.rs` (drives all three
  entries and cross-checks against Rust's `serde_json` — the tree's
  own conformance precedent) + `tests/session.rs` + the README.

**`benches/workloads/json-decode/main.rut` (483 lines) — the pinned
bench row.** The digest parser ported verbatim (same DOM, same cursor,
same `(opaque, str)` pairs) driven over a generated 1200-row document
(~204 KB), reps 3, plus a downcast fold. Pins:
checksum `4502015958359127277` (rut/qjs/node agree), fuel
**111,322,915**, heap **34,377,027 B (32.78 MiB)** — re-pinned ONCE
when the return-position destructure fusion deleted the `MakeRecord`
mints (old fuel verbatim: 111,330,118; heap 34,377,147 B). The row is
the opaque-churn baseline and the pair economy's canary.

**What does NOT exist yet** (phase 1 builds): no `rut/json/` dir, no
`JsonValue` anywhere, no number-parse surface in core (`f"{x}"` goes
f64→str via Rust's shortest-round-trip Display, `native/str.rs:190`
— but there is no str→f64 at all; `lex_int` in the bench row is the
int-parse precedent, pure rut).

### 1.4 What the checker/engine already answers — and the two gaps

Verified in the tree (the phase-1 verify items rest on these):

- **Impl targets are `TyPath` or `TyArray` — there is NO `Opt` arm.**
  `collect_impl` (`crates/rut-lir/src/check/collect.rs:428–513`)
  matches `TypeKind::TyPath` (local data, native types, used types,
  primitives) and `TypeKind::TyArray` (`impl [T] { .. }` — the
  structural-type-constructor precedent, exactly json's shape).
  `impl JsonSerialize for ?T` would fall into the `_` arm and
  diagnose. **Phase-1 verify item 1:** a `TyKind::Opt` arm mirroring
  the `TyArray` one (generic through the element parameter, template
  id for pair-uniqueness). Engine-side, additive, no wire change.
- **Trait static calls exist; a type-parameter receiver does not.**
  `compile_trait_static_call` (`crates/rut-lir/src/lir/call.rs:918`)
  resolves `Type.decode(r)`-shaped calls when the receiver names a
  known type (`call.rs:807–826`: str/bytes/native/enum/data — a
  generic parameter is none of those). **Phase-1 verify item 2:** a
  single-seg path naming an in-scope type parameter admits the static
  receiver, so `decodeJson`'s body can spell `T.decode(r)`. The
  no-self trait method itself is RFC 0012-sanctioned (§2's `fn
  scale(v: f64);`, §7's `fn yield(cx: TaskRunContext);`) — json's
  `decode` is its first *pkg-trait* use.
- **Generic impl bodies admit per instantiation.** `impl .. for
  Vec<T>` is "a template: its methods monomorphize per instantiation
  through `target_data`" (`collect.rs:425–427`); the union-bound
  capability check (`call.rs:840–848`) already typechecks every
  instantiation of a bounded receiver. `impl JsonSerialize for
  Vec<T>`'s body calling `elem.encode(w)` leans on the same law —
  phase 1 verifies the checker agrees for the *unbounded* `T`
  (pouch's `Vec<T>` has no element bound) with a `Vec<Vec<i64>>`
  fixture before building the matrix out.
- **`b[i]` on bytes is legal** (element `u8`; "indexing needs a
  sequence (Vec, Array, or bytes)", `lir/expr.rs:132`) — the strict
  UTF-8 DFA (§2.2) is writable in pure rut with no core addition.
- **`string_join` exists** (`core.d.rut:45`, one pass, RFC 0007) —
  and the strings-round1 census measured it SLOWER than the
  accumulator on token-shaped streams (§2.3's evidence).
- **`StrView` shipped** (RFC 0042): `s.slice` is an O(1) view;
  views-of-views flatten; the ascii flag inherits. The reader's
  token carve (§2.2) is one `slice` per token, zero copies.
- **`mut` spells on the binding, not the type.** The locked surface
  says `w: mut JsonWriter`; the tree's precedent (`digest.rut:704`
  `fn jskip(mut cur: ?Cur)`, pouch's `pub fn push(mut self, …)`) puts
  `mut` on the name. The contract is embedded verbatim above; the
  phase-1 spelling is `mut w: JsonWriter` — the same contract, the
  tree's spelling. Not a deviation: a normalization note.

### 1.5 The RFC frame

RFC 0006 (no data-carrying enums — the kind/details error split is
forced, not chosen), RFC 0007 (the accumulator fast path, §2.3's
writer), RFC 0012 (nominal impls, any-module trait impls = the orphan
legality of the container matrix, the peer-gated impl-groups
amendment which already uses json's `impl JsonSerialize for Vec<T>`
as its example, the no-self method), RFC 0017 (cycles leak by law —
encode's Depth is expected failure), RFC 0028 (core is the only
standard; every other in-tree pkg is a swappable default — json is
one more), RFC 0037 (reflection — rejected on record: the
`Reflectable` walk is per-node trait dispatch, the measured
dispatch volume is exactly what the json census prices as the
dominant term), RFC 0042 (views), RFC 0043 (bound-only unions — the
two decode entries), RFC 0044 (sharing default; the `(?T, err)` pair
law + the mint fusion amendment), RFC 0045 (the peer/dev mechanism
json's manifest mounts through — §2's grammar example is literally
`rut/json/rut.toml`).

---

## 2. The design decisions

### 2.1 The decode model — DIRECT schema-driven; JsonValue defers

**Decision: DIRECT.** The trait's `decode(r)` reads its own
expectations straight off the cursor — no intermediate DOM. A
`JsonValue` (the digest/serde_json shape) is NOT built first.

The evidence is this tree's own json census (strings-round1 phase 0,
`benches/README.md` Finding 0b): the json-decode row runs 11.8× its
qjs twin (722.39 vs 61.45 ms net) and the measured split is
**~24% per-char str dispatch + ~64% mint machinery** (Json records,
3 empty Vecs + an opaque box per value node) — dispatch VOLUME, not
algorithmic work. A DOM-first model doubles cell traffic: the parser
walks and mints the tree, then every consumer re-walks it (the bench
row's fold re-walks every number and string a second time —
`lex_int`/`fold_str` over the same cells). DIRECT halves the walk and
deletes the DOM mints entirely: a `Vec<Row>` decode mints exactly the
program's own values, once.

What JsonValue would still be needed for: schema-less data (digest's
case — a generic tree it edits and re-encodes). **JsonValue defers to
the menu (§4)** unless a consumer need appears that DIRECT cannot
spell: none is found in the census. digest itself migrates its decode
half only when JsonValue lands (§2.10) — the honest cost of DIRECT,
borne by one example, not by the lib's consumers.

### 2.2 The reader internals — the int-codes cursor: ADOPTED

The reader is a class over a codepoint column, not over str cells:

```rut
pub class JsonReader {
    cs: Vec<u32>;   // the doc's codepoints, split ONCE at open
    pos: i32;       // codepoint index (at/`at` are codepoints everywhere)
    depth: i32;
}
```

**ADOPT the int-codes cursor** — the −36% menu item
(`benches/README.md`: "int-code cursor + slice tokens, 499.1 ms,
−36.0%, fuel −33.3%"; the per-char str dispatch it deletes is ~189 ms
≈ 24% of the row, twice the build term, "invisible to views"). The
decode path's op stream IS the measured workload's op stream: walk,
classify, carve. The cursor turns `is_digit` into two int compares
and the char cells into `u32` slots; `[?u32]` backings store raw
payloads + nil tags (the OQ-3 partial landing), so the split phase's
pushes box nothing. The split is paid once per `decodeJson` call
(`for (c of s) { cs.push(c.code()); }` — the same one-pass shape the
bench row already uses as setup); reps-shaped consumers split once
and re-open cursors exactly like the bench does today.

Tokens carve through RFC 0042: a token is ONE `s.slice(start, pos)`
O(1) view — but the slice source is the ORIGINAL str, so the reader
also carries `src: str` beside `cs` (the 8-byte word per codepoint
buys the dispatch; the views buy the payloads).

**decodeJsonBytes: validate-then-same-path.** The bytes entry runs a
strict UTF-8 DFA over `b[i]` octets first (pure rut, ~15-state
classifier; invalid → `(nil, err)` with `kind = InvalidUtf8`, `at` =
the offending octet's index, `got` the hex byte text) — never
`bytes.decode()` on unvalidated input, because `decode` is LOSSY
(`core.d.rut:83`) and silent U+FFFD corruption is exactly the failure
the ruling bans. Valid bytes then go `b.decode()` (exact on validated
input) and re-enter the SAME str cursor — one reader, one codepath,
two entries.

**The error plumbing:** internal reader steps return the small stuff
(`?DecodeErrorKind` + an `at`) and the entry assembles the
`DecodeJsonError` struct once at the top — `got`/`expected` f-strings
are failure-path-only, the happy path never formats. `Trailing`
after the value + whitespace; `Truncated` at end-of-input mid-token;
`Unexpected` for grammar misses (`got` the offending codepoint as
text); `WrongType` for the schema-vs-doc mismatches of §2.4; `Depth`
at the §2.5 limit.

### 2.3 The writer internals — the accumulator; string_join rejected on the measurement

```rut
pub class JsonWriter {
    out: str;        // THE accumulator — the rc==1 in-place append path
    depth: i32;
    // the lazy path stack: Vec<str> keys / Vec<i32> indices pushed by
    // begin_*/popped by end_* — assembled into `path` ONLY on failure
}
```

**Decision: the f-string accumulator, not `Vec<str>` parts +
`string_join`.** The strings-round1 census priced this exact choice:
`string_join` instead of the accumulator measured **+3.6% SLOWER**
(808.0 vs 779.8 ms) on token-shaped build streams — "the rc==1
in-place append path is already the optimal build spelling". The
writer appends (`out = f"{self.out}{tok}"` through `mut self`
methods, the RFC 0007 §2 fast path RFC 0042 §4 gates to owned cells);
a byte-buffer shape has no rut surface to spell it with (str is the
text type; `bytes` would re-mint the UTF-8 question the reader just
solved). Method surface (public — user impls drive it; that is the
serde model): `write_str` (escaped), `write_raw` (pre-rendered
numbers/literals), `begin_object`/`end_object`/`begin_array`/
`end_array` (comma bookkeeping + depth in/out), `key` (the
`"k":` half), `finish` (returns the out string). The encoder's
hot path does zero per-char work until a string needs escaping.

**Escape policy (what a rut str must escape for JSON):** exactly the
RFC 8259 set — `"` → `\"`, `\` → `\\`, and the C0 controls
(U+0000–U+001F): `\b \f \n \r \t` shorthands, other controls
`\u00xx` hex. Everything else verbatim: DEL (U+007F) is legal JSON
unescaped, non-ASCII codepoints pass through as UTF-8. **Surrogates
are a non-question in rut**: a rut str is a codepoint sequence
(UTF-8 backed), so unpaired surrogates cannot occur in any `str` a
writer is handed — there is nothing to surrogate-pair-escape on
encode. The READER closes the loop: a `\uD800` escape with no valid
pair is invalid JSON that rut cannot represent — `Unexpected` err
(`got` the escape text), never a silent replacement char (the same
never-silently-corrupt law as the bytes entry).

**Census additions to `EncodeErrorKind`:** `NotFinite` (f64
inf/nan — JSON has no spelling; never the `null` cop-out serde_json's
arbitrary-precision mode uses, and never a trap: the ruling makes
encode's failure an expected `?E`) and `KeyUnsupported` (a `bytes`
key, §2.6). `at` = the codepoint offset in `out` at the failure;
`path` = the lazily-built `$.rows[3].name`-shaped spelling.

### 2.4 The number policy

Decode (schema-driven — the TYPE expected decides the grammar):

| target | lexeme | verdict |
|---|---|---|
| `i64` | integer, fits i64 | exact |
| `i64` | integer, overflows i64 | **`WrongType`** (`expected: "an i64"`, `got:` the lexeme) — never a silent wrap |
| `i64` | has `.` or exponent | `WrongType` (`expected: "an i64"`, `got:` the lexeme) |
| `f64` | integer lexeme | exact widen to f64 (binary-exact; decimal digits ≤ i64 range) |
| `f64` | float lexeme | the two-tier parse below |
| `bool`/`str` | number/other | `WrongType` with the same expected/got shape |

**The f64 parse — two tiers, pure rut, no core addition.** Tier 1
(exact, the fast path): mantissa digits folded into i64 (exact ≤ 18
digits; a 19-digit mantissa that overflows i64 falls to tier 2),
exponent |e| ≤ 22: multiply through a split 32-bit-halves u64 product
of mantissa × 10^k (10^k is EXACTLY an f64 for k ≤ 22 — 5^22 < 2^53 —
so the product is exact) and round ONCE. A single rounding of an
exact product is correctly-rounded — tier 1 is IEEE-exact, same
guarantee class as serde_json's fast path. Tier 2 (the honest
documented edge): > 18 significant digits or |e| > 22 — repeated
scaling by the table (double rounding possible), **best-effort ±1
ulp, disclosed here and in the pkg docs**. The menu records the
alternative (a core `f64.parse` builtin — a declared-surface change,
a VERSION event, §2.9's law: not this batch).

Encode: `i64` → `f"{v}"` decimal (i64::MIN included — no +0/−0
wrinkle exists in decimal). `f64` → `f"{v}"` — Rust's `{}` on f64 is
the **shortest string that round-trips** (Grisu/dragon fallback), so
rut's own f-string IS the minimal-valid-JSON rendering (`1` for
1.0 is a legal JSON number decoded back as `1.0`; `-0.0` renders
`-0`, preserved). Round-trip law pinned as a test: for sampled f64
including edges (0.1, 1e22, MAX, MIN_POSITIVE, −0.0),
`decodeJson<f64>(encodeJson(v).0)` returns `v` bit-exact or the
encode errors.

### 2.5 The depth limit — 128, both directions, recoverable

**`JSON_DEPTH_MAX = 128`** — a json-pkg const, not an engine limit.
Rationale: serde_json's battle-tested default; digest's private 64
was arbitrary (its doc depth is 4; the bench's is 5 — both fit under
either); 128 JSON levels ≈ ≤ 512 rut frames (value→container→element
nesting) which is trivially inside any fuel/heap budget the VM
enforces, while 128 covers every real-world config document the
census can name. Encode enforces it on `begin_object`/`begin_array`
(depth > MAX → `Depth` err) — **that is the RFC 0017 story: the rc
heap leaks strong cycles by law, so an encode walking a cyclic
structure is EXPECTED to answer `(nil, Depth-err)`, data not trap**;
the writer never trusts the caller's type to be a tree. Decode
enforces it on container entry (`Depth` err, `at` the bracket).
Exactly-one-nil enforcement is the test suite's (§3), per RFC 0044's
amendment: the invariant is the caller's convention, the engine never
polices the pair — json polices its OWN pair with fixtures.

### 2.6 The impl matrix — FINAL

| type | lives in | encode | decode |
|---|---|---|---|
| `i64` | base `json.rut` | decimal | exact per §2.4 |
| `f64` | base | shortest round-trip; non-finite → `NotFinite` | two-tier §2.4 |
| `bool` | base | `true`/`false` | literal only (`1`/`0` are `WrongType`) |
| `str` | base | escaped §2.3 | any JSON string |
| `?T` (T: JsonSerialize/…Deserialize) | base (`?T` impl-target arm, §1.4 item 1) | `nil` → `null`, else T | `null` → `nil`, else T — **this row is what makes every optional field one `?T`, no Option/Result revival** |
| `[T]` | base (the `TyArray` target precedent) | `[v0,v1,…]` | length-known array, elements via T |
| `Vec<T>` | **group `group-pouch.rut`** (peer: pouch, optional) | array | array → Vec.push |
| `HashMap<K,V>` / `HashSet<T>` | **group `group-nmapset.rut`** (peer: nmapset, optional) | object / array | object / array |
| `PrimMapI64<K>` / `PrimMapU64<K>` / `PrimMapF64<K>` | group-nmapset | object | object |

**The prim-key policy (the ruling delegated to this survey):** JSON
object keys are strings; json renders and parses them by K's own
shape. `str` keys verbatim (escaped); integer keys as their decimal
spelling (`42`, `-7`; u64 same); `bool` keys as `true`/`false`;
`f64` keys via the §2.4 encoder (so `1.5` round-trips; `1.0` encodes
`1` and decodes back as f64 key `1.0` — value-equal, spelling
normalized: DISCLOSED). `bytes` keys: **`KeyUnsupported`** encode err
(JSON has no binary key spelling; base64 would be a POLICY this pkg
does not invent — menu). Decode of a prim-key map parses each key
text with K's parser; unparsable → `WrongType` (`expected: "an
integer key"`, `got:` the key text); duplicate keys that normalize
together (`"01"` and `"1"` both → 1): **last wins**, matching
nmapset's entry semantics. All other K widths (i8..u32) are NOT in
v1's matrix — they are pure additions (more impls, no surface
change) and ride the menu.

The user story the matrix must demo (a phase-2 fixture):
`dataclass Row { id: i64; name: str; tags: ?Vec<str>; score: f64 }` +
`impl JsonSerialize for Row` / `impl JsonDeserialize for Row` in the
USER's module — orphan-legal because the trait is json's, the type is
the user's, and RFC 0012 §2 blesses both directions.

### 2.7 The package — files, manifest verbatim

```
rut/json/
  rut.toml            # verbatim below (RFC 0045 §2's own example)
  json.rut            # base: traits, reader, writer, prims, ?T, [T], entries, errors
  group-pouch.rut     # impl-only: JsonSerialize/Deserialize for Vec<T>
  group-nmapset.rut   # impl-only: HashMap/HashSet/PrimMapI64/U64/F64 rows
```

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

Groups are impl-only by law (RFC 0045 §3): no new public names, the
base owns the traits and the reader/writer classes — and because the
group text is APPENDED to json's own `Module.source`, the groups see
json's private fields exactly like `json.rut` does (one module, two
trigger conditions). The self-build (`cargo`-style: the pkg's own
tests) gets pouch+nmapset from dev-deps, so every group impl is
compiled and dispatched in json's own test runs; a consumer without
the peers mounts json light and their `HashMap` rows simply never
exist (the D2 diagnostic if referenced — RFC 0045 §4's matrix, all
six rows already landed).

### 2.8 The exactly-one-nil invariant — enforcement story

RFC 0044's amendment fixes the law: the pair convention is not
boundary-enforced by the engine. json enforces it on ITSELF, in the
pkg test suite (§3): every entry × every fixture asserts the
disjunction — success ⇒ `.1 == nil` and `.0 != nil`; failure ⇒
`.0 == nil` and `.1 != nil` — and the internal discipline that makes
it hold by construction: fallible internals return `?E` (single
nilable) or `(T, ?E)`-shaped partials, the entry is the ONLY place
the pair is minted, and the mint dies in the caller's return-position
destructure (02ced3e — `let (s, e) = encodeJson(v);` mints no record;
this pkg is the fusion's real-world proof and its canary fixtures
double as the fusion's regression watch).

### 2.9 The mount + version call

**Mount order:** after `pouch` and `nmapset`, everywhere (§1.2's
ruling and the three census points). The graph side needs NOTHING —
RFC 0045's passes already implement json's manifest; the loose-file
side needs the two list entries + `assemble_peers` (§1.2's decision);
the LSP side is §2.9's touchpoint below.

**VERSION: NO bump — additive.** The binary `VERSION` gates the
MODULE WIRE format (`rut-core/src/binary.rs:431`), and the three
bumps in the tree's history are all wire/declared-surface events:
5→6 (opt_prim_store gate), 6→7 (`opaque.downcast` → `?T` — a declared
core-surface change), 7→8 (the `StackTrace` builtin class — a new
engine-woven name crossing module surfaces). json adds: zero builtin
decls, zero engine-woven names (its traits are ORDINARY nominal
traits — that is the serde-model ruling), zero new cell kinds
(`JsonReader`/`JsonWriter` are ordinary rut classes in a module;
`StrView` already shipped), zero host fns (§1.1). Pkg additions never
bumped VERSION: pouch, ink, nmapset all joined as source dirs. The
stale-artifact rejection machinery is untouched because no module
binary changes shape.

**The LSP std-surface touchpoint (9 pkgs):**
`crates/rut-lsp/src/std_surface.rs` embeds all eight sources
(`include_str!`, lines 18–25), indexes them in the array at lines
35–44, and the test `all_eight_packages_embed_and_index` (line 63)
asserts the count and the origin list. Phase 1: `pub const JSON:
&str = include_str!("../../../rut/json/json.rut");` + the array entry
`("json", JSON, Mode::Impl, "rut/json/json.rut")` after `nmapset` +
the test renamed/asserted to nine. The group files are NOT embedded —
they declare no public names (nothing to complete) and their impls
belong to peers' types; the base is what a hover wants. The wasm face
(`rut-lsp-wasm`) rebuilds with the crate; the extension re-issues its
vsix per the d930d98 precedent (artifact embeds the new wasm, no
version bump — the shipped source is unchanged).

### 2.10 The migration census — who moves, when

| code | lines | migrates? | phase | how |
|---|---|---|---|---|
| `digest.rut` `jencode` + `jquote` | ~55 | **YES** | 2 | `impl JsonSerialize for Json` on the existing DOM: `begin_object`/`key`/`write_str`/`write_raw` replace the f-string concat and the per-char quote vec; the private depth-64 limit retires in favor of the writer's 128 (DISCLOSED at migration: digest accepts deeper docs than before — a relaxation, never a break) |
| `digest.rut` parser half (`Cur`, `jparse_*`, `jskip`, `valid_number`, `join_chars`, `split_chars` use) | ~290 | **NOT YET** | menu | DIRECT has no DOM to decode INTO; digest's decode half is schema-less by design — it waits for the JsonValue menu item (§4). Until then digest's private decoder stays as the err-channel batch's historical proof |
| `digest.rut` entries (`json_dec`/`json_enc`/`json_roundtrip`) | ~30 | follows the halves | 2 (enc) / menu (dec) | `json_enc`/`json_roundtrip` re-spell onto the lib's writer; `json_dec` keeps its private parser until JsonValue |
| `examples/02-digest` host + tests | — | tests ride along | 2 | the serde_json cross-checks now validate the LIB's output through digest — the conformance precedent pointed at the new code |
| `benches/workloads/json-decode` (483 lines) | 483 | **NO — stays** | — | it is a PINNED engine-churn baseline (the opaque-mint canary, the pair-fusion's bit-exact witness, checksum 4502015958359127277, fuel 111,322,915). Migrating it would change its semantics (checksum), invalidate every opaque-fastpath comparison made against it, and erase the canary. Retirement is a MENU decision (§4), never a side effect |
| **new row `json-roundtrip`** | ~200 | **lands new** | 2 | the lib's bench (§2.11): decode + re-encode over the SAME generated doc, typed values end to end — the first row that exercises std json directly, and the peer model's own proof (its `rut.toml` declares `[deps] json` + `[deps] pouch`; the gate assembles the group because pouch is present) |

### 2.11 The bench method — pre-registered

**The row.** `benches/workloads/json-roundtrip/` — a module dir
(`rut.toml`, `[deps]` json + pouch by path). The doc generator is
json-decode's `gen_doc` ported VERBATIM (same LCG, seed 42, same
draws, same order — the generator is data, and identical data is what
makes cross-row comparisons meaningful); scale identical (1200 × 6 ×
3). Per rep: `decodeJson<Vec<DocRow>>(&doc)` then `encodeJson(rows)`
then the fold. The JS twin (`json-roundtrip.js`) generates the same
doc, `JSON.parse`s it, folds the same VALUES (BigInt.asIntN on the
i64 paths — the json-decode twin's exact discipline).

**The checksum.** Defined over the ROUND-TRIPPED VALUES, not
lexemes: strings fold codepoints (`wrapping_mul(31).wrapping_add`),
i64s fold their value, f64s fold their bit pattern reinterpreted i64
— mirrored bit-for-bit on the JS side, asserted equal across reps,
logged via `Logger…info(f"CHECKSUM {n}")` and extracted by `run.mjs`
as today. At FIRST LANDING the row pins checksum + fuel + heap into
`benches/workloads/expected.json` — the sanctioned row-ADDITION edit
(the json-decode row's own precedent, README "Performance log —
json-decode: the opaque-mint baseline"), with the twin agreement
(rut/qjs/node) recorded in the landing commit body.

**The rules, stated up front (the batch's disclosure law):**

1. Semantics identical ⇒ checksums MAY hold. Any run that preserves
   the workload's observable value stream must show the SAME
   checksum; a moved checksum means the values moved, and is a FINDING
   (or a bug), never a re-pin.
2. Fuel/heap WILL move — they count ops and bytes, not values. Any
   optimization or migration that changes them re-pins ONCE per
   change, with the OLD values verbatim in the commit body (the
   precedent: json-decode fuel 111,330,118 → 111,322,915, −7,203,
   heap 34,377,147 → 34,377,027 B, checksum untouched).
3. Performance claims only behind same-session paired runs, 5×7
   interleaved minimum on any flagged row, ±1% = parity (the
   bench README's standing law).
4. The old `json-decode` row's pins are NOT touched by this batch —
   its workload does not migrate (§2.10). Its row appears in this
   batch's reports only as context.

### 2.12 The phase order

- **Phase 1 — the base pkg.** `rut/json/` (manifest verbatim +
  `json.rut`: traits, errors, reader §2.2, writer §2.3, numbers §2.4,
  depth §2.5, prims + `?T` + `[T]`, the three entries), the two
  checker arms (§1.4 items 1–2), `assemble_peers` + the CLI/probe
  list entries, the LSP ninth pkg, the pkg test suite incl. the
  exactly-one-nil matrix (§3).
- **Phase 2 — the containers + the bench.** The two group files
  (impl matrix §2.6), the self-build fixtures through dev-deps, the
  `json-roundtrip` row (§2.11), digest's encode-half migration
  (§2.10), the conformance corpus (§3) grown into the fixtures.
- **Phase 3 — the record.** README/RFC amendments (a short RFC 0028
  revision note: the ninth pkg; an RFC 0044/0045 cross-reference
  touch-up only if wording moved), the report.

---

## 3. The test matrix (phase 1's fixtures, phase 2's corpus)

Mount: the pkg tests run as json's SELF-build — dev-deps guarantee
pouch/nmapset presence, so every group impl dispatches in every run.

1. **The exactly-one-nil matrix** — every entry ×: a valid doc
   (success pair), each error kind (failure pair), an empty input
   (`Truncated`), trailing garbage (`Trailing`), a 200-deep nest
   (`Depth`), invalid UTF-8 bytes (`InvalidUtf8`, bytes entry only).
   Asserts the full disjunction on every row.
2. **Conformance corpus** — the digest host's serde_json cross-check
   idea, grown: a corpus of tricky docs (escapes, controls, unicode,
   big/small numbers, −0.0, exponents, deep nests, duplicate keys,
   lone surrogates) with the expected decode/encode verdicts pinned
   as fixtures (no Rust twin needed at runtime — the expected values
   are baked, the twin runs offline).
3. **Number edges** — §2.4's table row by row: i64::MIN/MAX, the
   first overflowing integer, 18/19/20-digit mantissas, e±22/±23,
   `1e2` into i64 (`WrongType`), round-trip sampling for f64.
4. **Key policy** — §2.6: str/int/bool/f64 keys encode+decode;
   `bytes` key → `KeyUnsupported`; `"01"`/`"1"` duplicate → last
   wins.
5. **Cycle encode** — a self-referencing structure → `(nil, Depth)`
   — the RFC 0017 expected-failure proof, no trap, no hang.
6. **The group gate, end to end** — a fixture workspace with and
   without the peers: without, the `Vec` rows are inert and a
   reference diagnoses D2; with, the impls dispatch (RFC 0045 §4's
   matrix, now pointed at real groups).
7. **The checker-arm fixtures** — `impl JsonSerialize for ?T` lands
   green only with §1.4 item 1; `T.decode(r)` only with item 2; both
   fixtures live in the pkg suite so the arms can never rot.

---

## 4. Deviations, limits, menu

**Deviations from the law: none.** The rulings are embedded verbatim
(§0) and every design section implements them. Two normalizations,
recorded not deviated: `w: mut JsonWriter` spells `mut w:
JsonWriter` at implementation (the tree's mut-on-binding form, §1.4);
`EncodeErrorKind` gains `NotFinite` + `KeyUnsupported` — the ruling's
own "+ what your census adds" clause, exercised.

**Phase-1 verify items (engine-adjacent, additive, no wire change):**
the `TyKind::Opt` impl-target arm (§1.4 item 1 — confirmed absent in
`collect.rs`); the type-parameter static receiver for `T.decode(r)`
(item 2 — confirmed absent in `call.rs`); the per-instantiation
admission of `elem.encode(w)` inside `impl JsonSerialize for Vec<T>`
(item 3 — precedent exists in the template law, fixture-gated);
`assemble_peers` (§1.2 — a driver-level wrapper, not engine).

**Limits (honest):** f64 decode tier 2 is ±1 ulp best-effort (§2.4) —
the only place json's output can differ from an engine-grade parser,
disclosed; `f64` object KEYS normalize spelling (§2.6); digest's
decode half stays private until JsonValue (§2.10).

**The menu (deferred, priced, none blocking):** `JsonValue` — the
DOM, digest's decode-half unlock, lands as an ordinary json type
(+ a `parseValue` entry) when a consumer needs schema-less; a core
`f64.parse` builtin — the strict tier-2 fix, correctly a declared-
surface VERSION event, not a pkg patch; the remaining prim widths
(i8..u32, f32) and `[?T]`/enum impls — pure matrix additions; bytes
views for the reader's byte-entry fast path; pretty-print options on
the writer; base64 key policy; the old json-decode row's retirement
(sanctioned row removal + disclosure) once the opaque-fastpath
comparisons it anchors are history.
