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
  sinks) in Rust. **`std:core`** is the prelude surface: the builtin
  traits that are ordinary nominal impls — `Disposal`
  (`fn dispose(mut self) -> unit`, RFC 0011/0016) — plus the prelude
  builtins `own(x)` (the eager copy, RFC 0011 §1), `downcast<T>`
  (RFC 0014), and `assert`/`panic` (RFC 0034 §2); `==` needs no
  trait at all (builtin, RFC 0012 §4). **`std:collection` is a
  declaration file + Rust bodies**
  (RFC 0025, RFC 0026): its `.d.rut` declares the trait
  `Hashable { fn hash(self) -> u64; fn eq(self, other: Self) -> bool }`
  — hashing and key comparison are one contract — and
  the containers
  directly — `export host class Map<K requires Hashable, V> { .. }`, `Set<T>`
  — with no facade; builtin impls (string content, numerics/enum value,
  registered-struct vouchers) are host impl-registry
  entries, not rut syntax. Containers are library types, not VM
  builtins (RFC 0005). **`std:debug`**
  (RFC 0036) rides the same mechanism: `Location`, `here()`,
  `capture_stack_trace()`, and the `StackTrace` class. **`std:reflect`**
  (RFC 0037) too: the `Reflectable`/`Deserializable` protocols,
  `TypeInfo`, and engine admission — reflection for userland serde
  (`examples/json/`).
- **anything else** (`app:gfx`, `imaging`, `plugin:my_map`) — **embedder
  modules**: declaration files + Rust bodies the embedding application
  ships for its own domain — the same mechanism `std:collection` uses, in
  the embedder's namespace. Every specifier resolves through `load_module`
  (RFC 0035 §1).

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

`std:log` itself (rut source; `Logger` is a value class, so per-function
construction is free):

```rut
import { Level, emit } from "rt:log";     // native: enum + sink fn
export { Level };

export class Logger {
    private name: string;
    private level: Level = Level.Info;

    fn new(name: string) -> Self { return Self { name: name, level: Level.Info }; }

    fn set_level(self, l: Level) -> unit { self.level = l; }
    fn level(self) -> Level { return self.level; }

    fn debug(self, msg: string) -> unit { self.log_at(Level.Debug, msg); }
    fn info(self, msg: string) -> unit  { self.log_at(Level.Info, msg); }
    fn warn(self, msg: string) -> unit  { self.log_at(Level.Warn, msg); }
    fn error(self, msg: string) -> unit { self.log_at(Level.Error, msg); }

    private fn log_at(self, l: Level, msg: string) -> unit {
        if (Level.to_int(l) >= Level.to_int(self.level)) {
            emit(self.name, l, msg);
        }
    }
}
```

`rt:log` registers the enum `Level { Debug, Info, Warn, Error }` (RFC 0022
§2 — host-registered enums) and one fn, `emit(name: string, level: Level,
msg: string) -> unit`, which routes to `HostHooks.log` (RFC 0035 §1). An
uninstalled sink is a **silent no-op** — a script cannot accidentally spam
an embedded host's stdout; the host opts into logging explicitly.

## `std:debug` — `Location` & `StackTrace`

Diagnostics for error handling (RFC 0036): the *where* that errors and
traps carry. `std:debug` is a declaration file + Rust bodies, like
`std:collection` — the surface is tiny:

```rut
// std/debug.d.rut (excerpt — RFC 0036 §6)
export dataclass Location { file: string, line: i32, col: i32 }
export host fn here() -> Location;                  // folded at compile time (RFC 0033 §3)
export host fn str(v: Opaque) -> string;           // developer rendering (RFC 0007 §2)
export host fn type_name(v: Opaque) -> string;  // debug type name
export host fn capture_stack_trace() -> StackTrace; // skips its own frame
export host class StackTrace {
    fn render() -> string;                          // via loaded binaries'
    fn depth() -> i32;                              // SymbolTables; lazy
}                                                 // — degraded when stripped
```

The three price points (RFC 0036 §1): `here()` is a compile-time constant
(zero runtime — fold it like `type_id<T>()`); `capture_stack_trace()` is
an explicit, cheap walk of the frame stack (raw pcs, no formatting); trap
backtraces are automatic at unwind. Errors stay values (RFC 0001 P4) —
attaching a `Location` (free) or an `Option<StackTrace>` (paid) to an
error dataclass is user code:

```rut
import { here, capture_stack_trace, Location, StackTrace } from "std:debug";

dataclass LoadError {
    msg:   string,
    at:    Location,               // free — folded at compile time
    trace: Option<StackTrace>,     // paid only when asked for
}
```

`VmCtx::capture_trace()` is the native half (RFC 0022 §3); the host-facing
`vm.symbolicate(&raw)` restores names/spans from loaded binaries (RFC 0035
§3, RFC 0036 §3) — and against stripped `--release` binaries, the
`.rutc.map` sidecar does it offline (RFC 0036 §3).
