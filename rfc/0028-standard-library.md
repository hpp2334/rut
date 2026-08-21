# RFC 0028: The Standard Library — `rt:*`, `std:*`, `std:log`

- **Status:** Draft
- **Date:** 2026-08-22
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
  sinks) in Rust. **`std:collection` is a declaration file + Rust bodies**
  (RFC 0025, RFC 0026): its `.d.rut` declares the interfaces
  `Equal<T> { eq(other: T): bool }`,
  `Hashable requires Equal<Self> { hash(): u64 }`, and the containers
  directly — `export host class Map<K: Hashable, V> { .. }`, `Set<T>`
  — with no facade; builtin impls (string/numerics/enum content,
  `Rc<T>` identity, registered-struct vouchers) are host impl-registry
  entries, not rut syntax. Containers are library types, not VM
  builtins (RFC 0005 pre-restructure OQ, resolved).
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

fn work(): void {
    const log = Logger("app");        // type-call -> factory; a bare value
    log.info(f"started at {now()}");  // class, so construction is free
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

    factory(name: string) { return Self { name: name, level: Level.Info }; }

    fn set_level(l: Level): void { this.level = l; }
    fn level(): Level { return this.level; }

    fn debug(msg: string): void { this.log_at(Level.Debug, msg); }
    fn info(msg: string): void  { this.log_at(Level.Info, msg); }
    fn warn(msg: string): void  { this.log_at(Level.Warn, msg); }
    fn error(msg: string): void { this.log_at(Level.Error, msg); }

    private fn log_at(l: Level, msg: string): void {
        if (Level.to_int(l) >= Level.to_int(this.level)) {
            emit(this.name, l, msg);
        }
    }
}
```

`rt:log` registers the enum `Level { Debug, Info, Warn, Error }` (RFC 0022
§2 — host-registered enums) and one fn, `emit(name: string, level: Level,
msg: string): void`, which routes to `HostHooks.log` (RFC 0035 §1). An
uninstalled sink is a **silent no-op** — a script cannot accidentally spam
an embedded host's stdout; the host opts into logging explicitly.
