# RFC 0028: The Standard Library — `rt:*`, `std:*`, `std:log`, `std:debug`

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0022–0027 (host & FFI), RFC 0029 (declaration files)
- **Supersedes:** RFC 0005 §8 (pre-restructure)
- **Part:** E — Host & FFI

## Summary

The standard library splits in two:

- **`rt:*`** — native modules (RFC 0022): `rt:log`, and the IO/backing
  modules behind `std:fs`, `std:net`, `std:http`, `std:time`,
  `std:channel`, and `std:collection`.
- **`std:*`** — rut source, compiled like user modules; they import `rt:*`
  for anything that touches the host. The split keeps policy (levels,
  formatting, wrappers) in auditable rut code and mechanism (syscalls,
  sinks) in Rust. **`std:core`** is the prelude surface — uniformly
  `builtin` (the engine implements it, compiler-lowered; the prelude
  registers **no** host bodies, RFC 0025 revised): the builtin
  containers `Array<T>`/`Option<T>`/`Result<T,E>` (RFC 0005) and
  `Opaque` (RFC 0014), the engine-woven interfaces — `Disposal`
  (`fn dispose(mut self) -> unit`, RFC 0011/0016), `Index<T>` and
  `Iterator<T>` (RFC 0012; ordinary nominal impls for users,
  compiler-backed impls for the engine's own types) — plus the
  prelude functions `own(x)` (the eager copy, RFC 0011 §1),
  `downcast<T>` (RFC 0014), `assert`/`panic` (RFC 0034 §2), and the
  the `str`/`bytes` natives (`string_join`,
  `bytes_len`, `bytes_decode`, `bytes_from`, `bytes_zeroed`); `==` needs
  no trait at all (builtin, RFC 0012 §4). **The prelude is imported,
  never ambient: nothing from `std:core` is in scope until a module
  writes `import { .. } from "std:core"`** — a missing import is a
  source diagnostic naming the fix. (Primitive types and their
  conversion syntax — `i32`, `str`, `bytes(n)`, `i32(x)` — are grammar,
  RFC 0007, not imports; so are the engine builtins `Array`/`Option`/
  `Result`/`Opaque`, the engine-woven interfaces, and the engine fns —
  the whole prelude, spelled in `std:core`'s `.d.rut` as `builtin`
  decls so users and the LSP see their contracts, RFC 0025 revised.) **`std:collection` is a
  declaration file + Rust bodies + rut wrappers**
  (RFC 0025/0026, revised): its `.d.rut` declares the trait
  `Hashable { fn hash(self) -> u64; fn eq(self, other: Self) -> bool }`
  — hashing and key comparison are one contract, satisfied by rut impls
  and builtin registry entries (string content, numerics/enum value) —
  and the container **host fns over `Opaque` handles**; the containers
  themselves (`pub class Map<K, V>`, `Set<T>`) are rut wrapper classes
  in `std:collection` source. Containers are library types, not VM
  builtins (RFC 0005). **`std:debug`**
  (RFC 0036) rides the same mechanism: `Location`, `here()`,
  `capture_stack_trace()`, and the `StackTrace` class. **`std:reflect`**
  (RFC 0037) too: the `Reflectable`/`Deserializable` protocols,
  `TypeInfo`, and engine admission — reflection for userland serde
  (RFC 0037 §5).
- **anything else** (`app:gfx`, `imaging`, `plugin:my_map`) — **embedder
  modules**: declaration files + Rust bodies the embedding application
  ships for its own domain — the same mechanism `std:collection` uses, in
  the embedder's namespace. Every specifier resolves through `load_module`
  (RFC 0035 §1).

`std:collection`'s native half (RFC 0025/0026, revised) is a **host fn
surface over `Opaque` handles** — `Map`/`Set` instances are boxes, and
the rut-side wrappers (`pub class Map<K, V> { h: Opaque; .. }`) live in
`std:collection`'s source: containers are library types, not VM builtins
(RFC 0005). Keys hash by content (`str`) or through wrapper-defined
strategies; no `Hashable` contract crosses the boundary.

## `std:log` — the `Logger` class

There is no `console`, no `print`, no global output builtin (RFC 0002 §4
of the pre-restructure set; the rule stands: **no output builtins**). All
logging goes through an imported logger:

```rut
import { Logger } from "std:log";
import { now_ms } from "std:time";

fn work() -> unit {
    let log = Logger.new("app");        // class method -> construction; a bare
    log.info(f"started at {now_ms()}"); // value class, so construction is free
}
```

`std:log` itself (rut source — source-inlined, so its `Logger` methods
resolve at the call site, RFC 0035 §1):

```rut
import { create_logger, logger_log } from "rt:log";  // native module

pub class Logger {
    internal: Opaque;                  // the host owns the layout

    pub fn new(name: str) -> Self { return Self { internal: create_logger(name) }; }

    pub fn debug(self, msg: str) -> unit { logger_log(self.internal, 0, msg); }
    pub fn info(self, msg: str) -> unit  { logger_log(self.internal, 1, msg); }
    pub fn warn(self, msg: str) -> unit  { logger_log(self.internal, 2, msg); }
    pub fn error(self, msg: str) -> unit { logger_log(self.internal, 3, msg); }
}
```

`rt:log` is a **native module** (RFC 0022/0026): the host functions
`create_logger(name: str) -> Opaque` and `logger_log(logger: Opaque,
level: i32, msg: str) -> unit`. The host owns the logger's layout; rut
only ever holds an `Opaque` handle (RFC 0014) and never inspects it. The
embedder half ships as `rut-std::logger::install_std_log`
(`create_logger` boxes the name; `logger_log` routes to the sink). An
uninstalled sink is a **silent no-op** — a script cannot accidentally spam
an embedded host's stdout; the host opts into logging explicitly.

## `std:debug` — `Location` & `StackTrace`

Diagnostics for error handling (RFC 0036): the *where* that errors and
traps carry. `std:debug` is a declaration file + Rust bodies, like
`std:collection` — the surface is tiny:

```rut
// std/debug.d.rut (excerpt — RFC 0036 §6)
pub host dataclass Location { file: str, line: i32, col: i32 }
pub host fn here() -> Location;                  // folded at compile time (RFC 0033 §3)
pub host fn str(v: Opaque) -> str;           // developer rendering (RFC 0007 §2)
pub host fn type_name(v: Opaque) -> str;  // debug type name
pub host fn capture_stack_trace() -> Opaque;       // skips its own frame —
pub host fn stack_trace_render(t: Opaque) -> str;  // the raw-pc walk, an
pub host fn stack_trace_depth(t: Opaque) -> i32;   // Opaque handle
```

No `host class`: the trace is an `Opaque` handle and `std:debug`'s rut
source wraps it — `pub class StackTrace { h: Opaque; .. }` whose
`render`/`depth` forward to the host fns (rendering is lazy, via loaded
binaries' SymbolTables — degraded when stripped); `Location` is a `host
dataclass` — flat crossing data the host constructs (RFC 0025, revised).

The three price points (RFC 0036 §1): `here()` is a compile-time constant
(zero runtime — fold it like `type_id<T>()`); `capture_stack_trace()` is
an explicit, cheap walk of the frame stack (raw pcs, no formatting); trap
backtraces are automatic at unwind. Errors stay values (RFC 0001 P4) —
attaching a `Location` (free) or an `Option<StackTrace>` (paid) to an
error dataclass is user code:

```rut
import { here, capture_stack_trace, Location, StackTrace } from "std:debug";

dataclass LoadError {
    msg: str,
    at:    Location,               // free — folded at compile time
    trace: Option<StackTrace>,     // paid only when asked for
}
```

`VmCtx::capture_trace()` is the native half (RFC 0022 §3); the host-facing
`vm.symbolicate(&raw)` restores names/spans from loaded binaries (RFC 0035
§3, RFC 0036 §3) — and against stripped `--release` binaries, the
`.rutc.map` sidecar does it offline (RFC 0036 §3).

## The `std:core` prelude, v1.1 — removals diagnosed at the use site

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
  (`std:math` → `Math`); the head binds at import exactly like the
  other prelude names, and `<namespace>.<member>` resolves against the
  module's functions, constants, and intrinsics. The compiler routes
  by the bound head — never by a hardcoded module string.
- **`Vec.pop` contract.** `pop` returns the removed element; an empty
  pop is a contract breach (trap), guarded by `len() > 0` — not an
  error value. The `next()`-cursor iterator ships no more; the
  iteration protocol is RFC 0012's `__iterate` (v1.1).

`std:math`'s `checked_add`/`checked_sub`/`checked_mul` return
`(value, ok)` tuples (RFC 0005 §10).
