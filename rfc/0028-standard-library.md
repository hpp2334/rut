# RFC 0028: The Standard Library — `core`, plus the swappable `pouch`/`calc`/`ink` packages

- **Status:** Draft
- **Date:** 2026-08-23
- **Revised:** 2026-09 — the package reframe: `core` is the only standard;
  every other in-tree package is a swappable default; use paths are bare
  package names (RFC 0002 §4).
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
`builtin class`/`builtin trait`/`builtin fn` (the engine implements it,
compiler-lowered; the prelude registers **no** host bodies, RFC 0025
revised): the builtin containers `Array<T>`/`Option<T>`/`Result<T,E>`
(RFC 0005) and `Opaque` (RFC 0014), the engine-woven traits —
`Iterator<T>` (RFC 0012; ordinary nominal impls for users,
compiler-backed impls for the engine's own types; v1.1 removed
`Disposal`/`Index` — `on_drop` and builtin indexing replaced them), and
— when the async plan lands — `Task<T>` and its run contexts, plus
`launch_task`/`LaunchedTask` as core builtin decls (RFC 0012 §7; there
is **no async module**) — plus the prelude functions `own(x)` (the eager
copy, RFC 0011 §1), `downcast<T>` (RFC 0014), `assert`/`panic` (RFC
0034 §2), and the `str`/`bytes` natives (`string_join`, `bytes_len`,
`bytes_decode`, `bytes_from`, `bytes_zeroed`); `==` needs no trait at
all (builtin, RFC 0012 §4). **The prelude is used, never ambient:
nothing from `core` is in scope until a module writes `use core::{
.. };`** — a missing use is a source diagnostic naming the fix.
(Primitive types and their conversion syntax — `i32`, `str`, `bytes(n)`,
`i32(x)` — are grammar, RFC 0007, not uses; so are the engine builtins
`Array`/`Option`/`Result`/`Opaque`, the engine-woven traits, and the
engine fns — the whole prelude, spelled in `core`'s `.d.rut` as
`builtin` decls so users and the LSP see their contracts, RFC 0025
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
and container **host fns over `Opaque` handles**; the containers
themselves (`pub class Map<K, V>`, `Set<T>`) are rut wrapper classes
in `pouch` source, and `Vec<T>` is the growable sequence class.
Containers are library types, not VM builtins (RFC 0005). **`ink`**
is the logger package (below). **`calc`** is the math helpers
(`checked_add`/`checked_sub`/`checked_mul`, the `Math` namespace).
**`debug`** (RFC 0036) rides the same mechanism: `Location`, `here()`,
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
surface over `Opaque` handles** — `Map`/`Set` instances are boxes, and
the rut-side wrappers (`pub class Map<K, V> { h: Opaque; .. }`) live in
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
use core::{ Opaque };
use rt::{ create_logger, logger_log };   // native module (host-registered)

pub class Logger {
    internal: Opaque;                  // the host owns the layout
}

impl Logger {
    pub fn new(name: str) -> Self { return Self { internal: create_logger(name) }; }

    pub fn debug(self, msg: str) -> nil { logger_log(self.internal, 0, msg); }
    pub fn info(self, msg: str) -> nil  { logger_log(self.internal, 1, msg); }
    pub fn warn(self, msg: str) -> nil  { logger_log(self.internal, 2, msg); }
    pub fn error(self, msg: str) -> nil { logger_log(self.internal, 3, msg); }
}
```

`rt`'s logger surface is a **native module** (RFC 0022/0026): the host
functions `create_logger(name: str) -> Opaque` and `logger_log(logger:
Opaque, level: i32, msg: str) -> nil`. The host owns the logger's
layout; rut only ever holds an `Opaque` handle (RFC 0014) and never
inspects it. The embedder half ships as `rut-std::logger::install_std_log`
(`create_logger` boxes the name; `logger_log` routes to the sink). An
uninstalled sink is a **silent no-op** — a script cannot accidentally spam
an embedded host's stdout; the host opts into logging explicitly.

## `debug` — `Location` & `StackTrace`

Diagnostics for error handling (RFC 0036): the *where* that errors and
traps carry. `debug` is a declaration file + Rust bodies, like
`pouch` — the surface is tiny:

```rut
// debug/debug.d.rut (excerpt — RFC 0036 §6)
pub host struct Location { file: str, line: i32, col: i32 }
pub host fn here() -> Location;                  // folded at compile time (RFC 0033 §3)
pub host fn str(v: Opaque) -> str;           // developer rendering (RFC 0007 §2)
pub host fn type_name(v: Opaque) -> str;  // debug type name
pub host fn capture_stack_trace() -> Opaque;       // skips its own frame —
pub host fn stack_trace_render(t: Opaque) -> str;  // the raw-pc walk, an
pub host fn stack_trace_depth(t: Opaque) -> i32;   // Opaque handle
```

No `host class`: the trace is an `Opaque` handle and `debug`'s rut
source wraps it — `pub class StackTrace { h: Opaque; .. }` whose
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

The prelude surface is `downcast`, `assert`/`panic`,
`make_ptr`/`on_drop`, `string_join`, and the `str`/`bytes` member
contracts (`s.len()`/`s.code()`/`s.encode()`,
`b.len()`/`b.decode()`, `bytes.zeroed(n)`/`bytes.from(a)` —
RFC 0004 §4).
`Option`/`Result`/`own` are removed (RFC 0005 §10) and `char` is gone
(RFC 0004 §4): a use site — type position, constructor, or literal —
diagnoses with the removal and its replacement. **No removed surface
keeps compatibility routing**: a removed head in an unresolvable
position is an ordinary unknown-name error.

Two surface mechanisms formalized this revision:

- **Namespace heads.** A native module may declare a namespace
  (`calc` → `Math`); the head binds at use exactly like the
  other prelude names, and `<namespace>.<member>` resolves against the
  module's functions, constants, and intrinsics. The compiler routes
  by the bound head — never by a hardcoded module string.
- **`Vec.pop` contract.** `pop` returns the removed element; an empty
  pop is a contract breach (trap), guarded by `len() > 0` — not an
  error value. The `next()`-cursor iterator ships no more; iteration is
  the nominal `impl Iterator<E> for T` (RFC 0012 §6).

`calc`'s `checked_add`/`checked_sub`/`checked_mul` return
`(value, ok)` tuples (RFC 0005 §10).
