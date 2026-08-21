# RFC 0005: Host & FFI — Embedding, Repr C, Templates

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0001 (G8), RFC 0002 (types, §3.1, §10), RFC 0003
  (§5 transfers), RFC 0004 (memory), RFC 0008 (VM)
- **Covers:** the Rust embedding API — native modules, the checked value
  boundary, zero-copy borrows, **repr C struct interop for dataclass AND
  class**, host types (`extern class` **declared in rut-source decl
  modules**), and **template (`f"..."`) handling across the boundary**.

## Summary

The host is a Rust program that owns a `Vm` (RFC 0008): it registers
native module **implementations** (surfaces are declared in rut-source
**decl modules**, §5), loads modules (no load-time execution), and
drives entry points. Every crossing is checked against reified types
(RFC 0002 §10) — the bridge boilerplate tur needed simply does not
exist. Fast paths exist where they are provable: repr C structs pass by
pointer, buffers borrow zero-copy, and templates arrive **structured**
(parts + typed values), not as pre-concatenated strings.

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
vm.register_module("app:gfx", gfx_module())?;      // bodies, bound against
vm.register_module("plugin:my_map", my_map_module())?;  // decl modules (§5)
vm.load("widgets")?;                               // verify + link, run nothing
let t = vm.spawn("main", &[])?;                    // suspend entry
vm.run_until_idle()?;                              // host owns the loop
```

`register_module` binds **implementations only** — every native surface
(what exists, its signatures, param bounds) is declared in rut source:
decl modules (§5). Compiling and checking rut code never requires any
Rust; `vm.load`'s link step proves every referenced native member has a
bound, signature-equal implementation (§5.1).

Native code never sees `&Vm` while rut runs (single-threaded); native fns
receive a `VmCtx` that permits re-entrant `vm.call` (§3 guards make that
safe) and registering wakeups.

## 2. Native modules & typed functions

Registration binds bodies against the module's decl module (§5) — the
string labels below are binding-time keys resolved against the decl's
slot table once, at startup; dispatch is by slot (§5.1):

```rust
fn gfx_module() -> NativeModule {
    NativeModule::new("app:gfx")                  // decl: app/gfx.rut
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
- Native registration supplies implementations for decl-module items:
  **`extern fn`s** (above), **class methods / factories** (§5.1), and —
  for the types themselves — the backing of **enum, interface, and
  builtin-impl registry entries** declared in decl modules
  (`std:collection`'s `Equal<T>`/`Hashable` + their builtin impls are
  the canonical case, §8).
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

## 5. Host types — `extern class` declarations & decl modules

Native surfaces are declared **in rut source** — a *decl module* whose
extern declarations are pure surface (signatures, no bodies). The
specifier maps to a decl module file (`"app:gfx"` → `app/gfx.rut`,
`"plugin:my_map"` → `plugin/my_map.rut`, `"std:collection"` likewise):
compiled and verified like any module, generating no code of its own.
rutc, the LSP, and AOT image builds see the surface with **zero Rust
linked** — the role tur's `index.d.ts` plays today, but in-language and
type-checked.

```rut
// app/gfx.rut — decl module for "app:gfx"
extern fn newCanvas(w: i32, h: i32): Canvas;   // module-level native fn

export extern class Canvas {                   // exported: nameable outside
    fn circle(x: f32, y: f32, r: f32): void;
    fn flush(): void;
}

export extern class Source<T> {                // generic — instantiation
    fn get(): T;                               // identity KEPT on the value:
}                                              // Source<i32> != Source<string>

extern class Fence {                           // NOT exported: known inside
    fn signal(): void;                         // app/gfx (callable via its
}                                              // slot), nameable nowhere else
```

- An `extern class` declaration may contain method signatures and a
  **factory signature** (`factory(cap: i32): Self;`) — the native
  factory keeps construction an ordinary type-call (§5.1). It may not
  declare fields: extern instances box host values, not rut field
  blocks.
- **Param bounds on extern decls are admission-only syntax**: `K:
  Hashable` constrains which instantiations compile (checked against
  the interface + its `requires` graph, RFC 0002 §6) and grants nothing
  else — no method calls on bare `K`, no static dispatch. User
  generics keep no bounds (RFC 0002 OQ-7 untouched); a bound would be
  pure forwarding anyway (RFC 0005 §5.1).
- Instances are `RutOpaque` heap objects (RFC 5004 §1) holding a boxed host
  value; the header `TypeId` carries the class **and** its generic
  instantiation (`Source<i32>` ≠ `Source<string>` — the tur bug fixed
  structurally, RFC 0002 §10.1 #2).
- **Methods dispatch by slot, not name.** Compiling the decl module
  assigns every extern member a stable slot id (declaration order); the
  module image carries the slot table. Calls compile to `CallNative {
  slot }` (RFC 0007) — member names are binding-time labels for the
  Rust side only (§5.1), never dispatch keys, never in IR.
- **Visibility**: decl modules are ordinary modules — RFC 0002 §1.1
  applies. Non-exported externs are *known* inside the module (callable
  via their slots) but *nameable* nowhere else; slots are always
  assigned (private members need them for intra-module calls).
- **Destructors map to Drop**: when rc hits 0, the host value's Rust
  `Drop` runs at that point (RFC 0004 §3) — textures, sockets, and files
  release deterministically, never "at GC someday".
- **Workers**: an `extern class` value may cross isolates only if the host
  registered the type `send` (RFC 0003 §5) — checked at the transfer, by
  `TypeId`.
- Host fns returning `Opaque` accept any rut value (RFC 0002 §3.1) — the
  checked escape hatch for data with no static shape.

### 5.1 Decl + impl — a user-defined map

The two halves of a native module (full listing:
**`examples/host/plugin/my_map.rut`** — the declaration;
**`examples/host/my_map.rs`** — the implementation;
**`examples/host/my-map.rut`** — a consumer):

**Declaration — rut source** (embedder-authored, ships with the plugin):

```rut
// plugin/my_map.rut — the decl module for "plugin:my_map"
import { Hashable } from "std:collection";

export extern class MyMap<K: Hashable, V> {   // K bound = admission only
    factory(cap: i32): Self;                  // native factory
    fn set(k: K, v: V): void;
    fn get(k: K): Option<V>;
    fn size(): i32;
    fn keys(): Array<K>;
}
```

**Implementation — Rust, bodies only** (no surface data declared in Rust
at all):

```rust
pub struct MyMap {                              // NOT generic — see below
    inner: HashMap<IfaceHandle, RutValue>,      // K: fat ref, V: erased
}

fn build_my_map(args: &GenericArgs, types: &TypeRegistry)
    -> Result<ClassTable, Trap>                 // once per instantiation
{
    let v_ty = args.of("V").ty();               // reified V — drives checks
    let k_ty = args.of("K").ty();               // for keys(): Array<K>

    ClassTable::new::<MyMap>(args)
        .factory(|ctx: &mut VmCtx, cap: i32| Ok(MyMap::with_capacity(cap)))
        .method("set",  |ctx, this: &mut MyMap, k: IfaceHandle, v: RutValue| {
            ctx.check_arg(&v, v_ty)?;           // value's TypeId == this
            this.inner.insert(k, v); Ok(())     // instantiation's V
        })
        .method("get",  |ctx, this: &MyMap, k: IfaceHandle|
            Ok(this.inner.get(&k).cloned()))    // -> builtin Option<V>
        .method("size", |ctx, this: &MyMap| Ok(this.inner.len() as i32))
        .method("keys", |ctx, this: &MyMap| { /* Array<K> — unerased */ .. })
        .build()
}

pub fn my_map_module() -> NativeModule {
    NativeModule::new("plugin:my_map")
        .implement("MyMap", build_my_map)       // binds BY DECL NAME
}
```

**The connection contract — two checkpoints:**

| checkpoint | when | checks | errors to |
|---|---|---|---|
| **compile/verify** | `rutc check` / `vm.load` verify | instantiation vs the **extern decl**: `MyMap<Canvas, ..>` is a rut-line error (`Canvas` does not implement `Hashable`); admission closes over the `requires` graph (implementing `Hashable` entails `Equal<Self>`, RFC 0002 §6). Checking needs **no Rust at all**. | rut author |
| **link** | `vm.load` | every extern member **referenced** by rut code has a bound impl, and the ClassTable (reflected member names + Rust shapes under the crossing rule) equals the decl's signatures — a pure data compare, nothing runs. | loader / embedder |

A name bound that no decl declares is an embedder **startup** error
(typo guard). Drift between the halves never reaches a rut runtime.

**What is `V`? Reified instantiation, erased storage.** A Rust generic
(`build_my_map<V>`) is impossible: Rust monomorphizes at *Rust* compile
time, but the builder runs at *rut* runtime, once per instantiation —
nobody can supply `V`. Instead:

- the rut side **reifies** each instantiation — `GenericArgs` carries
  K's and V's `TypeId`s (RFC 0002 §10); that is what the builder
  receives (`args.of("V").ty()`);
- the Rust side stores **erased** — `RutValue` (owning handle;
  `Value<'v>` in §3 is its call-scoped borrow), checked per call against
  the reified V. `get` needs no per-call check: values only enter via
  `set`, and the cell carries the instantiation's `TypeId`.

**Crossing rule for decl types → Rust shapes:**

| decl type | Rust shape |
|---|---|
| param constrained to an interface (`K: Hashable`) | `IfaceHandle` (RFC 5002 §2 fat ref) — concrete; its `Hash`/`Eq` are implemented **once** by the rut crate, vtable-dispatching into the value's own `hash()`/`eq()` (user impls are rut code; builtin/voucher impls are native trampolines). Content hashing for `string` keys, identity for `Rc<T>` keys — same Rust type, different attached vtable. |
| unconstrained param (`V`) | `RutValue` — erased owning handle; per-call check against the reified `TypeId` |
| concrete types (`i32`, `f32`, `Template`, …) | the Rust type — as in §2, embedder-pinned at Rust compile time |
| `Self` | the instance handle |

Rust generics survive **only** for concrete signatures (§2); generic rut
classes never see them. (OQ: a `.specialize(v_ty, builder)` escape hatch
for unboxed storage on hot instantiations — the erased builder is the
semantic baseline.)

**Slots, not strings.** The `.method("get", ..)` label exists for the
register step only: compiling the decl module assigns slot ids
(`factory→0, set→1, get→2, …`); consumer calls compile to
`CallNative { slot }` (RFC 0007 — fold/CSE-safe, no string in IR, no
lookup at dispatch); `register_module` resolves each label against the
slot table **once**, before any rut code runs — a typo is a startup
error, never a runtime one. IR still cannot inline into native bodies
(honest FFI limit); if a host wants an optimizable body, it is rut
source.

- **Who satisfies an interface constraint**: user classes and dataclasses
  (`implements` — RFC 0002 §5.1/§6), builtins via registered impls
  (content for `string`/numerics/`enum`, identity for `Rc<T>`), and
  registered structs via a `register_struct` content voucher (the Rust
  mirror is `Hash + Eq` — no rut-side methods needed). Interfaces
  themselves, `Opaque`, `Array`, `Option`/`Result` satisfy nothing.
- Builtin types flow back natively — a method may return `Option<V>` or
  build an `Array<K>` host-side (`keys()` yields `Array<K>`, **not**
  `Array<Hashable>`: keys are *unerased* at the boundary — rut cannot
  consume interface refs there, RFC 0002 §6.1); rut cannot tell it
  wasn't written in rut.
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
  `std:channel`, and `std:collection`.
- **`std:*`** — rut source, compiled like user modules; they import `rt:*`
  for anything that touches the host. The split keeps policy (levels,
  formatting, wrappers) in auditable rut code and mechanism (syscalls,
  sinks) in Rust. **`std:collection` is a decl module + Rust bodies**
  (§5/§5.1): its rut file declares the interfaces
  `Equal<T> { eq(other: T): bool }`,
  `Hashable requires Equal<Self> { hash(): u64 }`, and the containers
  directly — `export extern class Map<K: Hashable, V> { .. }`, `Set<T>`
  — with no facade; builtin impls (string/numerics/enum content,
  `Rc<T>` identity, registered-struct vouchers) are host impl-registry
  entries, not rut syntax. Containers are library types, not VM
  builtins (RFC 0002 OQ-3, resolved).
- **anything else** (`app:gfx`, `imaging`, `plugin:my_map`) — **embedder
  modules**: decl modules + Rust bodies the embedding application ships
  for its own domain — the same mechanism `std:collection` uses, in the
  embedder's namespace. Every specifier resolves through `load_module`
  (RFC 0008 §5).

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
