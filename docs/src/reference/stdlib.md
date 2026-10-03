# core and the swappable packages

The standard library splits in two:

- **`core`** — the only standard. The prelude surface, uniformly
  engine-implemented: it registers **no** host bodies and needs no mount
  beyond the base one.
- **the swappable packages** — in-tree rut/host packages that a module
  declares in its manifest `deps`
  ([project structure](project-structure.md)). The community may replace
  any of them wholesale; nothing in the engine knows their names.

Anything else — `gfx`, `imaging`, your own `my_map` — is an **embedder
package**: a declaration file plus Rust bodies registered the same way
([embedding and native modules](embedding.md)).

## core — the prelude surface

The builtin fns, primitives, and containers are in scope in every
compilation unit; no `use` is needed. A `use core::{ … };` statement
stays legal but is redundant for them. The exceptions — the
**import-gated** spellings, resolving only through
`use core::{ .. }`: the const `NAN`, the disposal context
`DisposalContext`, the weak reference `Weak<T>`, and the closed async
pair (`Future<T>`, `RunContext`) — the engine's weave itself never
needs the import, only source that spells the names (a `cx: T`
parameter, a launcher's `f: Future<T>`, a `downcast<Future<..>>`).
The bracket markers (`[disposal]`/`[iterable]`) need NO import — the
marker word is the designation, not a name. The two builtin spellings
(ambient vs import-gated) are documented in
[Host fns and declaration files](host-fns.md).

### Functions

| signature | meaning |
|---|---|
| `panic(msg: str) -> nil` | abort with the message |
| `string_join(parts: [str]) -> str` | join in one pass |
| `capture_stacktrace() -> StackTrace` | opt-in stack snapshot: raw frames only, symbols resolved lazily per access |
| `str.from_code(n: u32) -> str` | the 1-codepoint string |

### Primitives and their members

```rut
builtin primitive str {
    fn len(self) -> i32;                     // codepoints
    fn code(self) -> u32;                    // the FIRST codepoint (traps on empty)
    fn code_at(self, i: i32) -> u32;         // traps out of bounds
    fn encode(self) -> bytes;                // the UTF-8 octets
    fn slice(self, from: i32, to: i32) -> str; // O(1) view, codepoint bounds
    fn scan(self, from: i32, set: [u8]) -> i64;  // fused classify; (stop << 8) | class
    fn starts_with(self, from: i32, head: str) -> bool;
}

builtin primitive bytes {
    fn len(self) -> i32;                     // octets
    fn decode(self) -> str;                  // UTF-8, lossy
    fn clone(self) -> bytes;                 // the one copy escape hatch
}
// type-methods: bytes.zeroed(n) -> bytes, bytes.from(a: [u8]) -> bytes

builtin primitive opaque {
    fn downcast<T>(o: Self) -> ?T;           // nil on mismatch; `x is T` probes
}
```

Construction keeps its builtin forms: `opaque(v)` seals, `Weak.new(v)`
wraps (the class-method construction; the `Weak` name itself is
import-gated — `use core::{ Weak }`) ([opaque](opaque.md),
[weak references](weak-refs.md)).

### Builtin classes

| class | members |
|---|---|
| `StackTrace` | `len() -> i32`, `name(i) -> str`, `line(i) -> i32`, `col(i) -> i32`, `render() -> str` |
| `Weak<T>` | `Weak.new(v)` (traps on nil; reference types only), `upgrade() -> ?T` — `nil` once the referent died. **import-gated** — `use core::{ Weak }` |
| `DisposalContext` | no members — the engine-minted parameter of a `[disposal]` member; it exists so the context can grow without touching the marker's signature |

### Engine contracts — markers and the closed async pair

The engine's contracts are spellings on types, standing alone — no
interface machinery behind them:

| contract | spelling | notes |
|---|---|---|
| `[iterable]` | `fn <free>(self, emit: fn(E) -> bool)` on an inherent impl | `for (x of it)` calls that ONE designated member; `emit` returning `false` stops; the element type falls out of the marked member's signature. No import — the marker IS the designation; the builtin sequences' fused loops never mark one |
| `[disposal]` | `fn <free>(mut self, cx: DisposalContext)` on an inherent impl | the cell-death contract: the engine calls it at refcount zero ([the Rc heap](rc-heap.md)); `DisposalContext` is **import-gated** — `use core::{ DisposalContext }` |
| `Future<T>` | `pub builtin class` — closed | every async producer's answer ([async and await](async.md)); **import-gated** — `use core::{ Future }` (a launcher's `f: Future<T>`, `downcast<Future<..>>`). No constructor, no impl lane — engine-minted only |
| `RunContext` | `pub builtin class` — closed; `checkpoint() -> u32`, `cancelled() -> bool` | the cx the weave INJECTS into every async body; **import-gated** — `use core::{ RunContext }` (a `cx` probe's type). No constructor — engine-minted per async call |

The ENGINE weaves without any of them — async frames, the minted cx,
and the fused `for..of` loops never consult user scope: a module that
imports nothing still iterates builtins, awaits, and launches.

### Numeric methods (per integer width `i8`–`u64`)

```
wrapping_add/sub/mul/shl   — modular, no trap
saturating_add/sub/mul     — clamped at the width's ends
checked_add/sub/mul        — the tuple (value, ok); ok is false exactly
                             when the mathematical result escaped the width
```

Ambient like the primitives themselves: `x.wrapping_add(y)` needs no
`use`. The trap-on-overflow `+`/`-`/`*`/`<<` stay the default operators.

### Constants

`NAN` (f64) — core's one const, behind `use core::{NAN}`. All other float
constants are `calc`'s.

## The swappable set

| package | kind | surface |
|---|---|---|
| `ink_host` | host pkg | `create_logger(name: str) -> opaque`, `logger_log(log: opaque, level: i32, msg: str)` |
| `ink` | inline rut pkg | the `Logger` class over `ink_host` |
| `pouch` | inline rut pkg | the growable sequence `Vec<T>` |
| `nmap_host` / `nmapset` | host pkg + inline rut pkg | the native key table; `HashMap`/`HashSet` |
| `json` | inline rut pkg (pulls `strbuild_host`) | `encodeJson` / `decodeJson` / `decodeJsonBytes` + the serde interfaces and their wrapper families |
| `strbuild_host` / `strbuild` | host pkg + inline rut pkg | the builder rows; the `StringBuilder` class |
| `calc` | host pkg | the `Math` namespace |
| `async_engine` / `async_host` | host pkg + inline rut pkg | the launcher rows; `launch_future` / `sleep` |
| `http_host` / `http` | host pkg + rut pkg | the std HTTP lanes |
| `bench-cross` | host pkg | the crossing-tax benchmark rows |

### `ink_host` and `ink` — logging

There is no global output builtin. All logging goes through a
used logger; the host owns the sink, and an uninstalled sink is a silent
no-op — a script cannot accidentally spam an embedded host's stdout.

```rut
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("app");
    log.info(f"started");
}
```

```text
started
```

| method | level passed to `ink_host` |
|---|---|
| `debug(msg)` | 0 |
| `info(msg)` / `log(msg)` | 1 |
| `warn(msg)` | 2 |
| `error(msg)` | 3 |

Embedder side: `hosts.install_host_pkg(&ctx, rut_std::logger::pkg(|s| println!("{s}")))` (the ctx is `session.host_pkg_context()` — [embedding](embedding.md)).
Mounting `ink` pulls `ink_host` along (`deps`).

### `pouch` — `Vec<T>`

The growable sequence, written in rut over the fixed `[T]` array:

| member | meaning |
|---|---|
| `new()` / `with_capacity(cap)` / `filled(v, n)` | construction (`[v; n]` under the hood) |
| `push(v)` / `pop() -> T` | `pop` returns the removed element; **traps on empty** — guard with `len() > 0` |
| `get`/`set` via `v[i]`, `for (x of v)` | compiler-lowered; element access aliases the stored cell |
| `from([T])` / `as_array() -> [T]` | bridges to the fixed array |
| `freeze() -> bytes` | the `Vec<u8>` → immutable `bytes` copy (exactly the live length) |
| `slice(from, to) -> ?Vec<T>` | compiler-lowered O(1) fixed-length window; detaches on grow |

### `flow` — the push pipeline

`Flow<E>` chains the push contract: a type is iterable when it marks
an `[iterable]` member, and a Flow wraps one drive in
adapter stages — a closure per STAGE, never per element. Pipelines are
the clarity tier; the fused builtin loops stay the perf tier.

Entry and sink are **interfaces, satisfied structurally**:
`IntoFlow<E>` is `fn into_flow(self) -> Flow<E>` — `Flow` itself
carries the identity member (a chain re-enters as a source), and the
builtin structural targets (which can never carry members) enter
through the spelled adapter wrappers `ArrFlow<T>`, `StrFlow`,
`BytesFlow`, and `VecFlow<T>`. `FromFlow<E>` is the no-self sink
member `fn from_flow(it: Flow<E>) -> Self` — carried by the wrappers
`VecFlow<T>` (with `collected()` unwrapping) and `SetFlow<T>` (with
`items()`). Nothing is auto-inserted: the call site spells the
wrapper, every time.

```rut
use pouch::{ Vec };
use flow::{ Flow, IntoFlow, FromFlow, VecFlow };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("flow");
    let nums: Vec<i32> = Vec.new();
    nums.push(1); nums.push(2); nums.push(3); nums.push(4);
    let src: VecFlow<i32> = VecFlow(nums);       // the spelled manufacture
    // entry → adapters → consumers: one drive, stage by stage
    let picked: Flow<i32> = src.into_flow()
        .map(fn(x: i32) -> i32 { return x * 10; })
        .filter(fn(x: i32) -> bool { return x > 15; });
    log.info(f"picked={picked.count()}");
}
```

```text
picked=3
```

| surface | meaning |
|---|---|
| `x.into_flow() -> Flow<E>` | the entry — `IntoFlow<E>`'s member; `Flow` carries the identity row, the wrappers `ArrFlow<T>` / `StrFlow` / `BytesFlow` / `VecFlow<T>` carry the rest |
| `VecFlow<T>(vec)` / `ArrFlow<T>(arr)` / `StrFlow(s)` / `BytesFlow(b)` | the adapters — newtype wrappers manufactured at the call site; a primitive or bare container cannot carry members, the wrapper does |
| `map<R>(f: fn(E) -> R) -> Flow<R>` | transform — introduces a new type variable, so annotate the lambda or spell the type argument; a fn path infers |
| `filter(p: fn(E) -> bool) -> Self` | keep the elements the predicate admits |
| `take(n)` / `skip(n)` | the first `n` / everything after the first `n` — the counter is a record the drive mutates, so a drained stage stays drained |
| `count() -> i32` / `for_each(f)` / `fold<R>(init, f) -> R` / `enumerate() -> Flow<(i32, E)>` | the one-drive consumers |
| `W<T>.from_flow(it: Flow<E>) -> W<T>` | the sink — `FromFlow<E>`'s no-self member, carried by `VecFlow<T>` and `SetFlow<T>`; `collected()` / `items()` unwrap the materialized container |
| `for (x of chain)` | chains are iterable — the `[iterable]` member on `Flow<E>` feeds the ordinary desugar |

**Import the interfaces you spell and the pipeline type**: `use flow::{
Flow, IntoFlow, FromFlow }` — the use-both law names each one.

**Deferred, documented**: `IntoFlow` for the mapset (nmapset ships no
host-table walk — json's map encode grows with it) and a map sink
(nmapset's `HashMap` has no wrapper family in flow yet — json's
decode-only `JMap*` wrappers are the precedent such a family would
follow).

### `nmapset` — `HashMap`/`HashSet`

Thin wrappers over the native key table (`nmap_host` rows): the whole
state is one `opaque` handle, every method one host call.

```rut
pub class HashMap<K requires i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64 | bool | str | bytes, V> {
    pub fn new() -> Self;
    pub fn with_capacity(n: i32) -> Self;
    pub fn put(mut self, k: K, v: V) -> bool;      // true = newly inserted
    pub fn get(self, k: K) -> ?V;                  // the STORED cell, not a copy
    pub fn has(self, k: K) -> bool;
    pub fn remove(mut self, k: K) -> bool;
    pub fn len(self) -> i32;
}
pub class HashSet<T requires ..same key set..> { new, with_capacity, put, has, remove, len }
```

Laws: the key set is closed (no floats — no stable equality; encode a
custom key canonically to `bytes`); `get` answers the stored cell, so two
gets of one key name one value until a replace; values release host-side.
`str` keys hash by content; `str` keys also carry `*_range(parent, off,
len)` members keyed by a byte range of the parent string — a range key is
the same key as its content, with no key cell minted on a probe
([string views](string-views.md)).

### `calc` — the `Math` namespace

```rut
use calc::{ Math };
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("t");
    let x = 3.0f64;
    let y = 4.0f64;
    let d = Math.sqrt(x * x + y * y);
    let a = Math.abs(x);
    let hf = Math.sqrt_f(2.0);            // f32 twin — the width is in the name
    log.info(f"{d} {a} {hf}");
}
```

```text
5 3 1.4142135
```

Functions (each with an `_f` f32 twin): `sqrt`, `floor`, `ceil`, `round`,
`trunc`, `exp`, `ln`, `log2`, `log10`, `sin`, `cos`, `tan`, `asin`,
`acos`, `atan`, `sinh`, `cosh`, `tanh`, `pow`, `atan2`, `hypot`,
`copysign`, `fma`, plus the float helpers `abs`, `min`, `max`, `signum`.
Constants: `Math.PI`, `TAU`, `E`, `SQRT_2`, `LN_2`, `LN_10`, `LOG2_E`,
`LOG10_E`, `INFINITY`, `NEG_INFINITY`, `EPSILON`, `MAX`, `MIN`,
`MIN_POSITIVE`. Integer numeric methods are core's, not `calc`'s.

### `json` — the serde package

Pure rut, `inline = true`, zero host fns — a json mount adds no host
bindings.

```rut
fn encodeJson<T requires JsonSerialize>(v: T) -> (?str, ?EncodeJsonError);
fn decodeJson<T requires JsonDeserialize>(s: str) -> (?T, ?DecodeJsonError);
fn decodeJsonBytes<T requires JsonDeserialize>(b: bytes) -> (?T, ?DecodeJsonError);

pub interface JsonSerialize   { fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError; }
pub interface JsonDeserialize { fn decode(mut r: JsonReader) -> (?Self, ?DecodeJsonError); }
```

- **The pair law**: `(?T, ?E)` with exactly one nil — success = `(value,
  nil)`, failure = `(nil, err)`. Encode returns `?EncodeJsonError`
  because a cyclic structure is expected, recoverable data, not a trap.
- **Direct decode**: no intermediate document; a type's `decode` reads its
  expectations straight off the cursor. A `Vec<Row>` decode mints exactly
  the program's values, once.
- **Satisfaction is structural**: your type satisfies
  `JsonSerialize`/`JsonDeserialize` by spelling the members on its own
  inherent impl — `impl Row { pub fn encode(self, mut w: JsonWriter)
  -> ?EncodeJsonError { .. } }`. json owns the interfaces; user types
  satisfy them without asking.
- **The wrapper law**: primitives and bare containers never satisfy —
  no inherent members to have. The wrapper families are the spelled,
  per-call manufacture: `encodeJson(JsonI64(64))`, never
  `encodeJson(64)` — nothing is auto-inserted, forever.

  | wrapper | wraps | members |
  |---|---|---|
  | `JsonI64` / `JsonF64` / `JsonBool` / `JsonStr` | `i64` / `f64` / `bool` / `str` | both directions; `get()` unwraps |
  | `JsonU64` | `u64` | decode-only — satisfaction is per-interface, so it satisfies `JsonDeserialize` alone (an out-of-`i64`-range producer has no encode row); `get()` unwraps |
  | `SOpt<T requires JsonSerialize>` | `?T` | both; `nil` → `null`, `null` → `nil`; `get()` unwraps (the encode-direction bound means a decode-only `T` cannot ride it) |
  | `JsonArr<T requires JsonSerialize>` | `[T]` | both; `to_arr()` unwraps |
  | `JsonVec<T requires JsonSerialize>` | `Vec<T>` | both (the pouch peer group); `to_vec()` unwraps |
  | `JMapStr<V>` / `JMapI64<V>` / `JMapBool<V>` / `JMapU64<V>` / `JMapBytes<V>` | `HashMap<K, V>` per key shape | decode-only (the nmapset peer group); `to_map()` unwraps — `JMapBytes` answers the bytes-keys disclosure |
  | `JSet<T requires JsonDeserialize>` | `HashSet<T>` | decode-only (the nmapset peer group); `to_set()` unwraps |

- **Numbers**: `i64` accepts integer lexemes only (overflow/decimal
  lexeme = `WrongType`, never a silent wrap); `f64` parses IEEE-exact in
  the common range, ±1 ulp beyond (disclosed); encode renders the
  shortest round-trip decimal.
- **Depth**: capped at 128 both directions, recoverable
  (`DecodeErrorKind::Depth` / `EncodeErrorKind::Depth`).
- **Errors**: payloadless kind enums + fixed-field structs —
  `DecodeErrorKind { Unexpected, Truncated, InvalidUtf8, WrongType,
  Depth, Trailing }`, `EncodeErrorKind { Depth, NotFinite,
  KeyUnsupported }`; decode details carry `at`/`got`/`expected`, encode
  details carry `at` and the lazily built `$.rows[3].name` path.
- `decodeJsonBytes` is strict UTF-8 (`InvalidUtf8`), never lossy.
- **Ownership**: json owns the interfaces; all container wrappers live
  in json, gated by peer groups — `JsonVec` activates when `pouch` is
  anywhere in the consumer's closure, the `JMap*`/`JSet` rows when
  `nmapset` is ([dependency kinds](dependency-kinds.md)). A consumer
  without the peers mounts json light.
- The writer accumulates through `strbuild`'s `StringBuilder`; the reader
  rides the core `str.scan`/`starts_with` primitives and O(1) string
  views.

### `strbuild` — the builder

The builder is a host package now (the `ink`/`Logger` pattern): the
`strbuild_host` decl pkg declares the five rows (`sb_new` / `sb_push` /
`sb_push_code` / `sb_len` / `sb_finish`, registered under the
pkg-name scope), and the `strbuild` package wraps them in the
`StringBuilder` class. Core ships no string-building machinery; a
strbuild mount pairs with the bodies:

```rut
use ink::{ Logger };
use strbuild::{ StringBuilder };

entry fn main() {
    let log = Logger.new("t");
    let k = "name";
    let mut b = StringBuilder.with_cap(1024);   // octet hint
    b.append(f"{k}=");
    b.append_code(0x21);
    let s = b.build();                          // the ONE materialization
    log.info(s);
}
```

```text
name=!
```

| member | meaning |
|---|---|
| `new()` | grow from small |
| `with_cap(cap: i32)` | pre-size to an octet hint (negative traps; the hint is advisory — identical behavior for every cap) |
| `append(mut self, value: str)` | amortized O(\|s\|), in place |
| `append_code(mut self, cp: u32)` | one codepoint; invalid scalars mint U+FFFD |
| `len() -> i32` | codepoints so far, O(1) |
| `build() -> str` | fresh immutable str; the builder keeps its buffer |

Sharing is the default: appends through an alias (or a `mut` parameter)
land in the caller's document; copies happen at exactly two engineered
points — `build`'s materialization and growth's prefix move. Growth
consults the embedder's heap budget BEFORE growing (the geometric
next-capacity is charged), so the wasm 4 MiB cap governs builder growth
exactly as it governs engine allocations.

Embedder side:
`hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg())`. Mounting
`strbuild` pulls `strbuild_host` along (`deps`); mounting `json`
pulls both (its writer rides the builder).

For the common accumulator shape no builder is needed at all:
`out = f"{out}{t}"` appends in place, linear in the total output
([f-strings](literals-and-inference.md)).

### `calc`'s company: `async` and `http`

- `async_engine` declares the engine rows (`__launch`, `__abort`,
  `__sleep`, `__sleep_yield`); `async_host` restores the typed surface:
  `launch_future(f: Future<T>) -> LaunchedFutureHandle<T>`,
  `LaunchedFutureHandle.abort() -> bool`, `sleep(ms: u32) -> Future<nil>`.
  Each embedder mounts the pair **and** installs
  `rut_std::async_host::pkg()`; a session that mounts neither
  has no launcher ([launched futures](launched-futures.md), [host futures](host-futures.md)).
- `http_host` declares the transport rows (three async, five sync
  readbacks); `http` wraps them in `HttpClient` / `RequestBuilder` /
  `Request` / `Response` / `ByteStream` — async only at the points that
  really wait:

  ```rut
  use http::{ HttpClient, ClientQueryMethod };

  let resp = await HttpClient.new().request()
      .method(ClientQueryMethod.Post)
      .url("https://example.com/api")
      .header("Accept", "application/json")
      .body(payload)
      .build()
      .send(cx);
  ```

  `send` resolves at headers; `body(cx)` drains the wire;
  `byte_stream()`/`next(cx)` walk it in chunks. Status `0` is reserved
  for transport failure. The reqwest lane is native-only.

## Mounting

- `mount_std` mounts `core` + `calc`; `mount_std_async` adds the async
  pair. Swappable packages mount by directory or bundle ([module
  bundles](bundles.md)); the `rut` CLI mounts the tree packages a loose
  file names by `use` ([the rut CLI](cli.md)).
- `inline = true` packages (ink, pouch, nmapset, json, strbuild,
  async_host) are source-inlined into each consumer — required for
  class-method surfaces, whose inherent impls cross no module link
  boundary yet ([the frontend](frontend.md)).
- Generic exports link on their own: instantiation happens where the
  body lives, consumers request it, and `Vec<i64>` is one type
  program-wide ([the compiler pipeline](compiler.md)).
