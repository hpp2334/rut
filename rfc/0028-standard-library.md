# RFC 0028: The Standard Library — `core`, plus the swappable packages

- **Status:** Draft
- **Date:** 2026-08-23
- **Revised:** 2026-09 — the package reframe: `core` is the only standard;
  every other in-tree package is a swappable default; use paths are bare
  package names (RFC 0002 §4).
- **Revised:** 2026-09 (builtin-surface) — **builtin names are AMBIENT**:
  `pub builtin` is gone (plain `builtin` declares, no visibility), the
  erasure type is the primitive `opaque` (RFC 0014 revised), and the
  prelude binds in every compilation unit — the use-gate below is
  superseded.
- **Revised:** 2026-09 (rut-json) — **`json` joins the swappable set**
  as the ninth package (§ "`json` — the serde package" below): the
  DIRECT serde model, the container impls peer-gated through RFC 0045
  (its first real consumer), and the dependency direction decided on
  the record in that section.
- **Revised:** 2026-09 (strbuild) — **`strbuild` joins the swappable
  set as the tenth package** (§ "`strbuild` — the builder package"
  below): a pure-rut class face over the engine's `StrBuf` builtin
  cell, dep-free (the cell is ambient), and json's writer is its first
  consumer.
- **Author:** hpp2334
- **Depends on:** RFC 0022–0027 (host & FFI), RFC 0029 (declaration files)
- **Supersedes:** RFC 0005 §8 (pre-restructure)
- **Part:** E — Host & FFI

## Summary

The standard library splits in two:

- **`rt`** — native modules (RFC 0022): the host-fn surfaces behind the
  in-tree packages (`rt`'s `create_logger`/`logger_log` for `ink`), and
  the IO/backing modules a host may register for its own domain.
- **the in-tree rut packages** — rut source, compiled like user modules;
  they use `rt` for anything that touches the host. The split keeps
  policy (levels, formatting, wrappers) in auditable rut code and
  mechanism (syscalls, sinks) in Rust.

**`core` is the only standard.** It is the prelude surface — uniformly
`builtin` decls with **no `pub`**: `builtin primitive`/`builtin fn`/
`builtin impl`/`builtin trait` (the engine implements it,
compiler-lowered; the prelude registers **no** host bodies, RFC 0025
revised): the array grammar `[T]` (RFC 0005 §9), the engine primitives
`str`/`bytes`/`opaque` (RFC 0014 revised), the engine-woven trait —
`Iterator<T>` (RFC 0012; ordinary nominal impls for users,
compiler-backed impls for the engine's own types; v1.1 removed
`Disposal`/`Index` — `on_drop` and builtin indexing replaced them), and
— when the async plan lands — `Task<T>` and its run contexts, plus
`launch_task`/`LaunchedTask` as core builtin decls (RFC 0012 §7; there
is **no async module**) — plus the prelude functions `assert`/`panic`
(RFC 0034 §2), `on_drop` (RFC 0016 §3), `string_join` (RFC 0007), and
the primitive member contracts — `str`/`bytes` members, the `builtin
impl i8..u64` numeric methods (RFC 0004 §3, RFC 0032 §1.1 R2), and the
erasure statics `opaque.new`/`opaque.downcast<T>` (RFC 0014);
`==` needs no trait at
all (builtin, RFC 0012 §4). **The prelude is AMBIENT (builtin-surface,
2026-09 — this supersedes the use-gate earlier revisions shipped):
every builtin name is in scope in every compilation unit, no `use` is
needed.** A `use core::{ .. };` statement stays legal but is redundant,
and the driver's `compile_graph` binds core's prelude to every unit —
previously core's surface bound only when a module spelled `use
core::{…}`. The one exception is core's const: **`NAN` keeps its
use-gate by contract** — `use core::{NAN}` is explicit.
(Primitive types and their conversion syntax — `i32`, `str`, `bytes(n)`,
`i32(x)` — are grammar, RFC 0007, not uses; so are the engine builtins
`[T]`/`opaque`, the engine-woven trait, and the engine fns — the whole prelude, spelled in
`core`'s `.d.rut` as `builtin` decls so users and the LSP see their
contracts, RFC 0025
revised.)

**`pouch`, `calc`, `ink`, `nmapset`, `json`, `strbuild` are optional
in-tree
packages — swappable defaults, not required surface.** They ship in
the toolchain's tree (`rut/pouch/`, `rut/calc/`, `rut/ink/`,
`rut/nmapset/`, `rut/json/`, `rut/strbuild/`) and a module that wants one
declares it in its manifest `[deps]` (RFC 0041 §3); the community may
replace any of them wholesale — nothing in the engine knows their names.
**`pouch`** is a declaration file + Rust bodies + rut wrappers (RFC
0025/0026, revised): it declares the trait
`Hashable { fn hash(self) -> u64; fn eq(self, other: Self) -> bool }` —
hashing and key comparison are one contract, satisfied by rut impls
and builtin registry entries (string content, numerics/enum value) —
and container **host fns over `opaque` handles**; the containers
themselves (`pub class Map<K, V>`, `Set<T>`) are rut wrapper classes
in `pouch` source, and `Vec<T>` is the growable sequence class.
Containers are library types, not VM builtins (RFC 0005). **`ink`**
is the logger package (below). **`calc`** is the float math package:
the `Math` namespace of `f64` host functions (`sqrt`..`fma`, plus the
float helpers `abs`/`min`/`max`/`signum` — platform libm, RFC 0025)
each with an **f32 twin under a `_f` suffix** (`Math.sqrt_f(x: f32)
-> f32` — native f32 libm; the width is in the name, rut has no
overloading) and the f64 constants (`PI`, `E`, `INFINITY`, …). The integer numeric
methods (`wrapping_*`/`saturating_*`/`checked_*`) are **`core`'s** —
`builtin impl` methods on the primitives (RFC 0004 §3, RFC 0032 §1.1
R2) — and `NAN` is core's one const (`use core::{NAN}`). **`debug`**
`capture_stack_trace()`, and the `StackTrace` class. **`reflect`**
(RFC 0037) too: the `Reflectable`/`Deserializable` protocols,
`TypeInfo`, and engine admission — reflection for userland serde
(RFC 0037 §5).
- **anything else** (`gfx`, `imaging`, `my_map`) — **embedder
  packages**: declaration files + Rust bodies the embedding application
  ships for its own domain — the same mechanism `pouch` uses, in
  the embedder's own registration. Every package name resolves through
  `load_module` (RFC 0035 §1).

`pouch`'s native half (RFC 0025/0026, revised) is a **host fn
surface over `opaque` handles** — `Map`/`Set` instances are boxes, and
the rut-side wrappers (`pub class Map<K, V> { h: opaque; .. }`) live in
`pouch` source: containers are library types, not VM builtins
(RFC 0005). Keys hash by content (`str`) or through wrapper-defined
strategies; no `Hashable` contract crosses the boundary.

## `ink` — the `Logger` class

There is no `console`, no `print`, no global output builtin (RFC 0002 §4
of the pre-restructure set; the rule stands: **no output builtins**). All
logging goes through a used logger:

```rut
use ink::{ Logger };

fn work() -> nil {
    let log = Logger.new("app");        // class method -> construction; a bare
    log.info(f"started");               // value class, so construction is free
}
```

`ink` itself (rut source — source-inlined, so its `Logger` methods
resolve at the call site, RFC 0035 §1):

```rut
use rt::{ create_logger, logger_log };   // native module (host-registered)

pub class Logger {
    internal: opaque;                  // the host owns the layout
}

impl Logger {
    pub fn new(name: str) -> Self { return Self { internal: create_logger(name) }; }

    pub fn debug(self, msg: str) -> nil { logger_log(self.internal, 0, msg); }
    pub fn info(self, msg: str) -> nil  { logger_log(self.internal, 1, msg); }
    pub fn warn(self, msg: str) -> nil  { logger_log(self.internal, 2, msg); }
    pub fn error(self, msg: str) -> nil { logger_log(self.internal, 3, msg); }
}
```

`rt`'s logger surface is a **host pkg** (RFC 0022/0025/0026): the
package directory `rut/rt/` — `name = "rt"`, `host_scope = "rt:log"`,
`entry.type = "./rt.d.rut"` declaring `create_logger(name: str) ->
opaque` and `logger_log(logger: opaque, level: i32, msg: str) -> nil`.
The host owns the logger's layout; rut only ever holds an `opaque`
handle (RFC 0014) and never inspects it. The embedder half ships as
`rut-std::logger::install_std_log` — TYPED bindings; mounting `rt`
declares the surface and `Vm::verify_host_fns` checks the two against
each other before the first run (RFC 0025's load-time contract).
(`create_logger` boxes the name; `logger_log` routes to the sink.) An
uninstalled sink is a **silent no-op** — a script cannot accidentally spam
an embedded host's stdout; the host opts into logging explicitly.
`ink`'s manifest carries the dependency explicitly:
`[deps] rt = { path = "../rt" }` — mounting `ink` pulls `rt` along.

## `debug` — `Location` & `StackTrace`

Diagnostics for error handling (RFC 0036): the *where* that errors and
traps carry. `debug` is a declaration file + Rust bodies, like
`pouch` — the surface is tiny:

```rut
// debug/debug.d.rut (excerpt — RFC 0036 §6)
pub host struct Location { file: str, line: i32, col: i32 }
pub host fn here() -> Location;                  // folded at compile time (RFC 0033 §3)
pub host fn str(v: opaque) -> str;           // developer rendering (RFC 0007 §2)
pub host fn type_name(v: opaque) -> str;  // debug type name
pub host fn capture_stack_trace() -> opaque;       // skips its own frame —
pub host fn stack_trace_render(t: opaque) -> str;  // the raw-pc walk, an
pub host fn stack_trace_depth(t: opaque) -> i32;   // opaque handle
```

No `host class`: the trace is an `opaque` handle and `debug`'s rut
source wraps it — `pub class StackTrace { h: opaque; .. }` whose
`render`/`depth` forward to the host fns (rendering is lazy, via loaded
binaries' SymbolTables — degraded when stripped); `Location` is a `host
struct` — flat crossing data the host constructs (RFC 0025, revised).

The three price points (RFC 0036 §1): `here()` is a compile-time constant
(zero runtime — fold it like `type_id<T>()`); `capture_stack_trace()` is
an explicit, cheap walk of the frame stack (raw pcs, no formatting); trap
backtraces are automatic at unwind. Errors stay values (RFC 0001 P4) —
attaching a `Location` (free) or an `Option<StackTrace>` (paid) to an
error struct is user code:

```rut
use debug::{ here, capture_stack_trace, Location, StackTrace };

struct LoadError {
    msg: str,
    at:    Location,               // free — folded at compile time
    trace: Option<StackTrace>,     // paid only when asked for
}
```

`VmCtx::capture_trace()` is the native half (RFC 0022 §3); the host-facing
`vm.symbolicate(&raw)` restores names/spans from loaded binaries (RFC 0035
§3, RFC 0036 §3) — and against stripped `--release` binaries, the
`.rutc.map` sidecar does it offline (RFC 0036 §3).

## `json` — the serde package

The ninth package (`rut/json/`, landed by the rut-json batch): pure
rut source, `inline = true`, **zero host fns** — a json mount adds no
`expected_host_fns` entries, so RFC 0025's load-time exactness
contract is untouched by any json consumer. The surface:

```rut
fn encodeJson<T requires JsonSerialize>(v: T) -> (?str, ?EncodeJsonError);
fn decodeJson<T requires JsonDeserialize>(s: str) -> (?T, ?DecodeJsonError);
fn decodeJsonBytes<T requires JsonDeserialize>(b: bytes) -> (?T, ?DecodeJsonError);

trait JsonSerialize   { fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError; }
trait JsonDeserialize { fn decode(mut r: JsonReader) -> (?Self, ?DecodeJsonError); }
```

Decode is **DIRECT schema-driven**: the trait's `decode` reads its own
expectations straight off the cursor — no intermediate DOM is built,
so a `Vec<Row>` decode mints exactly the program's own values, once.
The `JsonReader` is a direct cursor over the source `str` (the early
`[?u32]` codepoint column is gone — the json-perf batch's phase 2);
its classify loops ride the host-side `str.scan` primitive end-to-end
and tokens carve as O(1) `StrView` slices (RFC 0042); the
`JsonWriter` accumulates through the `StrBuf` builder (amortized
in-place growth, one `finish()` materialization — no longer the
field-append rc==1 fast path, whose ~750× copy tax the json-perf
survey measured). Both moves are output-byte-identical
(`docs/json-perf-report.md`). Numbers: `i64` exact (overflow is a `WrongType`
error, never a silent wrap), `f64` two-tier — tier 1 IEEE-exact
(split-multiply, single rounding), tier 2 best-effort ±1 ulp,
disclosed; encode renders the shortest round-trip decimal. Depth is
capped at 128 both directions, recoverable — the encode side's
`Depth` error is the RFC 0017 story: the rc heap leaks strong cycles
by law, so walking a cyclic structure is EXPECTED failure, data not
trap. The `str`/`bytes` param pair is forced by RFC 0043 (unions are
bound-only), and `decodeJsonBytes` is STRICT UTF-8 — `bytes.decode()`
is lossy, and silently corrupting input is exactly the failure the
pkg bans; invalid octets answer `InvalidUtf8`. Errors are RFC 0006's
kind/details split — payloadless enums (`DecodeErrorKind` ×6;
`EncodeErrorKind` `Depth`/`NotFinite`/`KeyUnsupported`) beside
fixed-field structs (`at`, `got`/`expected` on decode; `at`, the
lazily-built `$.rows[3].name` path on encode).

**The dependency direction — decided, on the record.** json owns the
serde traits: they are ordinary nominal traits LOCAL to json, and ALL
container impls live IN json (`impl JsonSerialize for Vec<T>` and the
map/set rows in json's files) — orphan-legal because the trait is
local (RFC 0012's any-module impl law). Containers gain nothing,
know nothing, and user types impl json's traits on their own types.
The rejected alternatives:

- **Runtime reflection** (RFC 0037's `Reflectable`/`Deserializable`
  walk) — REJECTED, too slow: the measured dispatch volume is the
  dominant term of the very census that priced this decision
  (`benches/README.md`'s json-decode row: ~24% per-char str dispatch
  + ~64% mint machinery; a reflection walk re-creates the first and
  adds per-node vtable traffic to the second), and it would drag the
  reflect pkg into every serde consumer.
- **Engine-woven builtin traits** — REJECTED: a `JsonSerialize`
  lowered like `Iterator` would couple the engine to one text
  format, and require exactly the declared-surface VERSION event
  json's addition otherwise avoids (pkg additions never bumped
  VERSION — pouch, ink, nmapset, json all joined as source; the wire
  never moved).
- **Containers → json** (pouch/nmapset implementing json's traits) —
  REJECTED on layering: the keyed collections would depend on a text
  format, every container consumer would carry (or peer-gate) json,
  and the impls would sit in modules that do not own the trait.

**The dep-kinds interplay (RFC 0045's first real consumer).** The
container impls ride *peer groups*: json's manifest is RFC 0045 §2's
own example landed — `[peer-deps] pouch`/`nmapset` with `optional =
true` and impl-only integration `lib`s that mount only when the peer
is anywhere in the consumer's closure, plus the same peers as
`[dev-deps]` (the sanctioned both-kinds pairing) so json's own test
runs dispatch every group impl while a consumer without the peers
mounts json light. And the pkg is `inline = true` for ink/nmapset's
reason: json's entries are GENERIC FUNCTIONS, and a generic fn cannot
cross a module link boundary — the exported surface of a linked pkg
carries only monomorphic fns, so a linked json would answer `unknown
function decodeJson` in every consumer. Source-inlining composes json
into each consumer's unit, where the entries specialize per concrete
argument — which is DIRECT decode's own law.

The impl matrix (prims, `?T` null↔nil, `[T]` in the base; `Vec<T>`
in the pouch group; the map/set rows DECODE in the nmapset group —
map ENCODE waits on nmapset shipping an iteration surface), the
exactly-one-nil law on the `(?T, ?E)` entries, and the bench record
(`json-roundtrip`, checksum `1960875332163557684`) are the survey's
(`docs/rut-json-survey.md`) and the batch report's
(`docs/rut-json-report.md`); the user-facing summary lives in
`examples/README.md`.

## `strbuild` — the builder package

The tenth package (`rut/strbuild/`, landed by the strbuild batch): pure
rut source, `inline = true`, **zero host fns and zero deps** — `StrBuf`
is an AMBIENT builtin (RFC 0028 revised, builtin-surface), so the pkg
compiles against `mount_std_core` alone and a strbuild mount adds no
`expected_host_fns` entries and no dep edge of its own. The whole pkg
is one class over that cell:

```rut
pub class StringBuilder {
    out: StrBuf;          // the engine cell — the whole pkg is this field
}

impl StringBuilder {
    fn new() -> Self;                       // grow from small (StrBuf(0))
    fn with_cap(cap: i32) -> Self;          // pre-size to an octet HINT
    fn append(mut self, value: str);        // the one appender (StrBufPush)
    fn append_code(mut self, cp: u32);      // one codepoint (StrBufPushCode;
                                            //   invalid scalars mint U+FFFD)
    fn len(self) -> i32;                    // codepoints so far (StrBufLen)
    fn build(self) -> str;                  // the ONE materialization
}                                           //   (StrBufFinish — the builder
                                            //    keeps its buffer)
```

`inline = true` is load-bearing (ink/nmapset/json's flag): this is a
CLASS-METHOD pkg, and a class-method module cannot be linked — the
flag keeps the methods resolvable at every consumer's call site. RFC
0044 sharing is the law underneath: a binding shares the ONE instance
cell whose `out` slot shares the ONE engine cell — appends through an
alias (or through a `mut b: StringBuilder` parameter) land in the
caller's document; copies happen at exactly two engineered points
(`build`'s materialization and `grow`'s prefix move). The member
contract is closed on the record: `clear`/`reserve`/`capacity`/
`append_char`/`append_i64` have no nat and no call site — adding one
is a VERSION conversation for zero need. Mount order is 10th, after
json (the ninth-package precedent: no engine table orders std pkgs —
"the 10th" names the reading order and this paragraph). The engine
learns nothing new: no nat ids, no TyKind, no opcodes, no surface row
— VERSION stays 11, and json's writer is the pkg's first consumer
(`[deps] strbuild`, the mapping in `docs/strbuild-survey.md` §4).

## The `core` prelude, v1.1 — removals diagnosed at the use site

The prelude surface is `assert`/`panic`, `on_drop`, `string_join`, the
`str`/`bytes` member
contracts (`s.len()`/`s.code()`/`s.code_at(i)`/`s.encode()`,
`s.slice(from, to)` (RFC 0042), `b.len()`/`b.decode()`,
`bytes.zeroed(n)`/`bytes.from(a)` —
RFC 0004 §4), the tokenizer members (`s.scan(from, set)` — the fused
host-side scan/classify over a caller-owned `[u8]` class table,
returning the packed `(stop << 8) | class`; `s.starts_with(from,
head)` — the host-compared prefix test), the `StrBuf` growable builder
(`StrBuf(cap)`, `push`/`push_code`, O(1) `len`, `finish()` the one
materialization), the `builtin impl i8..u64` numeric methods, and the
erasure statics `opaque.new`/`opaque.downcast<T>` (RFC 0014 revised —
builtin-surface: both are members of the `builtin primitive opaque`,
and the names are ambient). The tokenizer members and `StrBuf` are the
json-perf batch's general-surface additions (VERSION 9 → 10
disclosed): machinery any tokenizer or encoder wants — json was the
first consumer, and the engine never learns what json is
(`docs/json-perf-report.md` §4).
`Option`/`Result`/`own` are removed (RFC 0005 §10), `char` is gone
(RFC 0004 §4), and the free fn spellings `downcast<T>(o)`/`make_ptr(v)`
are gone with them (builtin-surface): a use site — type position,
constructor, or literal —
diagnoses with the removal and its replacement (`downcast<T>(o)` names
`opaque.downcast<T>(o)`; the table lives in
`rut_core::binary::REMOVED_CORE`). **No removed surface
keeps compatibility routing**: a removed head in an unresolvable
position is an ordinary unknown-name error.

Two surface mechanisms formalized this revision:

- **Namespace heads.** A native module may declare a namespace
  (`calc` → `Math`); the head binds at use exactly like the
  other prelude names, and `<namespace>.<member>` resolves against the
  module's functions and constants. The compiler routes
  by the bound head — never by a hardcoded module string.
- **`Vec.pop` contract.** `pop` returns the removed element; an empty
  pop is a contract breach (trap), guarded by `len() > 0` — not an
  error value. The `next()`-cursor iterator ships no more; iteration is
  the nominal `impl Iterator<E> for T` (RFC 0012 §6).

The integer `checked_add`/`checked_sub`/`checked_mul` — now `core`'s
`builtin impl` methods — return `(value, ok)` tuples (RFC 0005 §10).
