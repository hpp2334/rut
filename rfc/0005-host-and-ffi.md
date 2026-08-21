# RFC 0005: Host & FFI — Embedding, Repr C, Templates

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0001 (G8), RFC 0002 (types, §3.1, §10), RFC 0003
  (§5 transfers), RFC 0004 (memory), RFC 0008 (VM)
- **Covers:** the Rust embedding API — native modules, the checked value
  boundary, zero-copy borrows, **repr C struct interop for dataclass AND
  class**, host types (`extern class`), and **template (`f"..."`) handling
  across the boundary**.

## Summary

The host is a Rust program that owns a `Vm` (RFC 0008): it registers
native modules with **typed** functions, loads modules (no load-time
execution), and drives entry points. Every crossing is checked against
reified types (RFC 0002 §10) — the bridge boilerplate tur needed simply
does not exist. Fast paths exist where they are provable: repr C structs
pass by pointer, buffers borrow zero-copy, and templates arrive
**structured** (parts + typed values), not as pre-concatenated strings.

**Non-feature:** there is no `box<T>` / loan / `&mut`-in-the-language. A
loan needs an exclusivity proof; rut has no compile-time borrow checker
and no runtime aliasing control over inline values (they are plain
copies). The only shared-reference mechanism is `Rc<T>`; the only borrows
are **host-side, call-scoped, flag-guarded** (§3) — anything wider is
unsound and will not be added.

## 1. Embedding model

```rust
let mut vm = Vm::new(HostHooks { .. });            // RFC 0008 §5
vm.register_struct::<Vertex>("Vertex")?;           // §4 — layout contract
vm.register_module("app:gfx", gfx_module())?;      // §2
vm.load("widgets")?;                               // verify + link, run nothing
let t = vm.spawn("main", &[])?;                    // suspend entry
vm.run_until_idle()?;                              // host owns the loop
```

Native code never sees `&Vm` while rut runs (single-threaded); native fns
receive a `VmCtx` that permits re-entrant `vm.call` (§3 guards make that
safe) and registering wakeups.

## 2. Native modules & typed functions

```rust
fn gfx_module() -> NativeModule {
    NativeModule::new("app:gfx")
        .fn_("newCanvas", |ctx, w: i32, h: i32| Ok(Canvas::new(ctx, w, h)))
        .fn_("blit",      |ctx, c: Handle<Canvas>, layer: StructRef<Vertex>,
                           n: u32| { .. Ok(Value::Void) })
        .fn_("label",     |ctx, t: Template| Ok(log_localized(ctx, t)))
}
```

- Parameter and return types are declared **once**, as Rust types; the VM
  checks every call against them using the same `TypeId` machinery as
  `is<T>` (RFC 5002 §3). No coercion code, no `as number`, no
  `require_props_object`.
- Native modules may also register **enum types** (named `i32` constants,
  RFC 0002 §7) — they cross the boundary as identity + `i32` (§3), and
  rut imports them like any type — **class types**, generic ones included
  (§5.1), and **interface types** (with their own `requires` clauses and
  builtin impls — `std:collection`'s `Equal<T>`/`Hashable` are the
  canonical example, §5.1).
- Failures are `Result<_, Trap>` values — a native fn that errors traps
  cleanly with a message and a rut backtrace (RFC 0008 §2).
- Long-running host work must NOT block the loop: hand back a future
  (RFC 0003 §4) and let `await` integrate it.

## 3. The `Value` boundary & borrow guards

```rust
pub enum Value<'v> {
    Void, Bool(bool), Char(char),
    I8(i8) /* .. */ I64(i64), U8(u8) /* .. */ U64(u64), F32(f32), F64(f64),
    Str(StrRef<'v>),                       // immutable, may point into heap
    Bytes(Borrow<'v, [u8]>),               // zero-copy, call-scoped
    Array(Borrow<'v, RutArray>),           // typed elem, zero-copy
    Struct(StructRef<'v>),                 // repr C block — §4
    Rc(Handle), Iface(Handle), Opaque(Handle), Host(Handle),
    Opt(Option<Box<Value<'v>>>), Res(Result<Box<Value<'v>>, Box<Value<'v>>>),
    Template(Tmpl<'v>),                    // §6
}
```

- `Borrow<'v, _>` is **call-scoped**: the Rust lifetime prevents storing it
  past return; the VM additionally sets a *borrowed* flag on the object
  header, and rut-side mutation ops (`buf.set`, `arr.set` …) check it —
  so a re-entrant `vm.call` inside a native fn that tries to mutate a
  borrowed buffer traps with `borrowed by host` instead of racing. Guards
  clear on return. To keep data, the host copies — that is the whole rule.
- This is why RFC 0004 OQ-1 (non-moving heap) matters: non-moving keeps
  these borrows trivially sound forever.

## 4. Repr C structs — dataclass AND class

Both user value types are **C-layout** (RFC 0002 §10.2): fields in
declaration order, natural alignment, size padded to alignment; no hidden
members; `private`, `implements`, `dispose`, and `static` add **nothing**
to the layout. An `Rc<C>` cell is `Header + vtable-ptr + that same block`
(RFC 5002 §2) — `StructRef::fields_ptr()` hides the prefix.

The host mirrors the struct in Rust and registers the contract at startup:

```rust
#[repr(C)]
struct Vertex { x: f32, y: f32, color: u32 }

vm.register_struct::<Vertex>("Vertex")?;   // checks rut's Vertex layout:
                                           // size_of, align_of, field offsets
                                           // — mismatch = startup error
```

After registration, `StructRef<'v, Vertex>` in a native fn is literally
`&Vertex` — the host reads/writes fields at native speed (zero copies, no
per-field accessors). rut passes inline values by pointer to the frame /
array element storage; the borrow flag guards re-entrant mutation.

Script-side, the layout is queryable at compile time (RFC 0002 §3.2):
`size_of<T>()`, `align_of<T>()`, `type_id<T>()` — `Array<T>` strides,
`StructCopy` sizes, and host struct mirrors all agree on one number.

## 5. Host types — `extern class`

```rut
import { Canvas } from "app:gfx";

extern class Canvas {              // declared for typing; built by the host
    fn circle(x: f32, y: f32, r: f32): void;
    fn flush(): void;
}

extern class Source<T> {           // generic host type — instantiation kept
    fn get(): T;
}
```

- Instances are `RutOpaque` heap objects (RFC 5004 §1) holding a boxed host
  value; the header `TypeId` carries the class **and** its generic
  instantiation (`Source<i32>` ≠ `Source<string>` — the tur bug fixed
  structurally, RFC 0002 §10.1 #2).
- Methods are native fns keyed `(TypeId, name)`; a call compiles to
  `callh` with the handle as receiver. An `extern class` *declaration* may
  not declare fields or a factory — construction happens host-side
  (`newCanvas()`), or through a **native factory** bound at registration
  (§5.1), which keeps construction an ordinary type-call.
- **Destructors map to Drop**: when rc hits 0, the host value's Rust
  `Drop` runs at that point (RFC 0004 §3) — textures, sockets, and files
  release deterministically, never "at GC someday".
- **Workers**: an `extern class` value may cross isolates only if the host
  registered the type `send` (RFC 0003 §5) — checked at the transfer, by
  `TypeId`.
- Host fns returning `Opaque` accept any rut value (RFC 0002 §3.1) — the
  checked escape hatch for data with no static shape.

### 5.1 Registered host classes — a user-defined map

`extern class` declarations (above) are how rut *types* handles the host
hands out. The inverse also exists: a native module may **export the class
itself** — there is no rut-side declaration at all, just the import. This
is the embedder's extension mechanism, and it is exactly how
`std:collection`'s `Map<K, V>` / `Set<T>` are provided (RFC 0002 OQ-3,
resolved): **containers are library types, not VM builtins** — a library
type may live entirely on the host side.

```rut
import { MyMap } from "plugin:my_map";            // embedder-defined, all Rust

const counts: MyMap<string, i32> = MyMap<string, i32>(32);  // type-call
counts.set("key", 1);                              // native method
counts.get("key");                                 // -> Option<i32> (builtin)
```

Registration — written by the embedding application (tur, say), not by
the rut stdlib. Full listing: **`examples/host/my_map.rs`** (the `.rut`
consumer side is `examples/host/my-map.rut` — the example is a pair).
`std:collection` ships the constraint vocabulary: the native interfaces
`Equal<T> { eq(other: T): bool }` and
`Hashable requires Equal<Self> { hash(): u64 }` (RFC 0002 §6), plus
builtin impls — `string`/numerics/`enum` by content, `Rc<T>` by identity
(object-keyed maps) — so common keys work with no user code:

```rust
fn build_my_map(generic_args: &GenericArgs, types: &TypeRegistry)
    -> Result<ClassTable, Trap>
{
    // Called ONCE per distinct MyMap<K, V> — the monomorphization point
    // (RFC 0002 §10).
    let k = generic_args.of("K");                    // param by NAME
    let i_hashable = types.interface_of("Hashable"); // shared TypeId

    ClassTable::new::<MyMap<K, V>>(generic_args)
        .constrain(k, i_hashable)   // K must implement Hashable — and,
                                    // via `requires Equal<Self>`, Equal<K>
        .factory(|ctx: &mut VmCtx, cap: i32| Ok(MyMap::with_capacity(cap)))
        .method("set",  |ctx, this: &mut MyMap<K, V>, k: K, v: V| Ok(this.insert(k, v)))
        .method("get",  |ctx, this: &MyMap<K, V>, k: K| Ok(this.get(&k)))  // Option<V>
        .method("size", |ctx, this: &MyMap<K, V>| Ok(this.len() as i32))
        .method("keys", |ctx, this: &MyMap<K, V>| Ok(this.keys()))         // Array<K>
        .build()
}

fn my_map_module() -> NativeModule {
    NativeModule::new("plugin:my_map")
        .generic_class("MyMap", 2, build_my_map)
}
```

- **Constraints are interfaces — checked at compile (IR) time, never
  runtime.** The module descriptor carries each generic param's
  constraints; an instantiation like `MyMap<Canvas, i32>` is rejected
  at the rut line (`Canvas` does not implement `Hashable`) and
  re-checked by the load-time verifier against the descriptor
  (RFC 0007 §8). Admission closes over the `requires` graph
  automatically: implementing `Hashable` entails `Equal<Self>`.
- **Who satisfies a constraint**: user classes and dataclasses
  (`implements` — RFC 0002 §5.1/§6), builtins via registered impls
  (content for `string`/numerics/`enum`, identity for `Rc<T>`), and
  registered structs via a `register_struct` content voucher (the Rust
  mirror is `Hash + Eq` — no rut-side methods needed). Interfaces
  themselves, `Opaque`, `Array`, `Option`/`Result` satisfy nothing.
- Methods are keyed `(TypeId, name)` exactly like `extern class` methods,
  and the header `TypeId` carries the instantiation:
  `MyMap<i32, i32>` ≠ `MyMap<string, Opaque>`.
- Builtin types flow back natively — a method may return `Option<V>` or
  build an `Array<K>` host-side; rut cannot tell it wasn't written in rut.
- Hashing/`eq` on a user type may be **rut code**, reached through the
  interface vtable — a method call that re-enters the VM (`VmCtx`,
  §1). Re-entrancy is already guarded: `set` holds `&mut this`, the
  cell's borrow flag is set, and a hash impl that calls `m.set(..)`
  again traps `borrowed by host` (§3) instead of corrupting the table.
- The **native factory** makes construction an ordinary type-call
  (`MyMap<string, i32>(32)`), same rule as `Array<f32>(n)`: construction
  is a function everywhere, and a host class simply supplies the function.
  Helper fns like `newCanvas()` remain the shape for host-computed or
  side-effecting construction.

See **`examples/host/my-map.rut`** (consumer side) and
**`examples/host/my_map.rs`** (embedder side).

## 6. Templates — `f"..."` across the boundary

Problem: `f"..."` normally renders to a `string` at the call site
(RFC 0002 §4.1 — `concat("a=", str(a))`), which destroys structure. Hosts
that need the structure — **localization**, structured logging, analytics
— must not re-parse strings.

Design: a builtin **`Template`** value type, built **only** by format
literals, chosen by expected type:

- `f"hi {name}, n={n}"` in a `string`-expected position behaves exactly as
  RFC 0002 §4.1 (desugars to `concat` — zero new cost on the hot path).
- The **same literal** in a `Template`-expected position (host fn
  parameter annotated `Template`, or an explicit `const t: Template =
  f"..."`) compiles to the `tmpl` op: a `Template { parts: Array<string>,
  args: Array<Opaque> }` — literal chunks and **boxed values with their
  runtime types** (`Opaque`, RFC 0002 §3.1), not pre-rendered text.
- `Template` API: `t.str(): string` renders with rut's own `str()` rules
  (identical output to the `string` path); `t.parts()`, `t.args()`,
  `t.type_id(i)` for hosts/stdlibs doing per-arg formatting. Nothing else
  — like `Opaque`, a template can't do anything until someone renders it.
- At the FFI, a `Template` parameter arrives as `Tmpl { parts: &[StrRef],
  args: &[Value] }` — the host formats per-locale, reorders placeholders,
  or logs structured fields, with **typed** args (`i64` stays `i64`, so
  locale decimal separators are the host's choice, not baked into a
  string).

```rust
.fn_("label", |ctx, t: Tmpl| {                    // host side
    let s = ctx.localize(t.parts(), t.args())?;   // ICU-style formatting
    Ok(Value::Void)
})
```

Serialization note: `Template` is a storage/carrier type — it crosses
isolate channels like any builtin (RFC 0003 §5), and a worker can return
one where the main VM expects `Template`.

## 7. Traps, budgets, interrupts

Native fns run **outside** the op budget (RFC 0008 §6) — the host is
trusted to be fast or hand back a future (§2). Traps raised inside a
native fn propagate as `Err(Trap)` with the native frame attributed in the
backtrace; `vm.call` re-entrancy nests budgets per outer frame. This is
the boundary where bugs become host problems — RFC 0001 P4's rule.

## 8. Standard modules — `rt:*` native, `std:*` rut source

The standard library splits in two:

- **`rt:*`** — native modules (this RFC): `rt:log`, and the IO/backing
  modules behind `std:fs`, `std:net`, `std:http`, `std:time`,
  `std:channel`, and `std:collection` (`Map<K, V>`, `Set<T>` —
  registered host classes, §5.1 — plus the constraint interfaces
  `Equal<T>` / `Hashable requires Equal<Self>` and their builtin impls).
- **`std:*`** — rut source, compiled like user modules; they import `rt:*`
  for anything that touches the host. The split keeps policy (levels,
  formatting, wrappers) in auditable rut code and mechanism (syscalls,
  sinks) in Rust.
- **anything else** (`app:gfx`, `imaging`, `plugin:my_map`) — **embedder
  modules**: native modules the embedding application registers for its
  own domain. `std:collection` is this pattern, shipped — an embedder
  adding `plugin:my_map` is doing precisely what the stdlib did. Every
  specifier resolves through `load_module` (RFC 0008 §5).

**`std:log` — the `Logger` class.** There is no `console`, no `print`, no
global output builtin (RFC 0002 §1). All logging goes through an imported
logger:

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

`rt:log` registers the enum `Level { Debug, Info, Warn, Error }` (§2 —
host-registered enums) and one fn, `emit(name: string, level: Level,
msg: string): void`, which routes to `HostHooks.log` (RFC 0008 §5). An
uninstalled sink is a **silent no-op** — a script cannot accidentally spam
an embedded host's stdout; the host opts into logging explicitly.

## Open questions

- OQ-1: `StructRef` mutability — v1 hands out `&T` (shared) + explicit
  `&mut T` only when the rut side provably cannot observe (moved values)?
  Proposed: `&T` only; writes go through returned values.
- OQ-2: host-side suspend construction (`newCanvas` returning a future of a
  handle) — just a future resolving to a `Host` value; needs an example.
- OQ-3: `extern class` static methods (host-namespaced functions today) —
  keep as plain native fns; do not duplicate the feature.
- OQ-4: should `register_struct` also permit *packed* layouts
  (`#[repr(packed)]`) or is repr C the single contract? Proposed: C only.
