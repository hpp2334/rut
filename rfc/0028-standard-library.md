# RFC 0028: The Standard Library — `core`, plus the swappable `pouch`/`calc`/`ink` packages

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

**`pouch`, `calc`, `ink` are optional in-tree packages — swappable
defaults, not required surface.** They ship in the toolchain's tree
(`rut/pouch/`, `rut/calc/`, `rut/ink/`) and a module that wants one
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

## The `core` prelude, v1.1 — removals diagnosed at the use site

The prelude surface is `assert`/`panic`, `on_drop`, `string_join`, the
`str`/`bytes` member
contracts (`s.len()`/`s.code()`/`s.encode()`,
`b.len()`/`b.decode()`, `bytes.zeroed(n)`/`bytes.from(a)` —
RFC 0004 §4), the `builtin impl i8..u64` numeric methods, and the
erasure statics `opaque.new`/`opaque.downcast<T>` (RFC 0014 revised —
builtin-surface: both are members of the `builtin primitive opaque`,
and the names are ambient).
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
