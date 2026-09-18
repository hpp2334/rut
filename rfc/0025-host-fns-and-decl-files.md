# RFC 0025: Host Fns, Host Dataclasses & Declaration Files

- **Status:** Draft (revised — **supersedes the `host class` design**)
- **Date:** 2026-09-13
- **Author:** hpp2334
- **Depends on:** RFC 0022 (embedding), RFC 0023 §1 (the crossing rule),
  RFC 0029 (declaration files), RFC 0014 (`Opaque`), RFC 0010 §2 (class
  methods), RFC 0016 §3/§5 (Drop mapping, host boxes)
- **Supersedes:** RFC 0005 §5 (pre-restructure), and this RFC's own
  earlier draft: `host class` / `extern class` / `extern fn` are
  **removed**; `host primitive` (removed earlier) stays removed
- **Part:** E — Host & FFI

## Summary

Native surfaces are declared **in rut source** — *host pkgs*: package
directories whose `rut.toml` names a declaration file (`entry.type`,
no `entry.lib`) whose declarations are pure surface (signatures, no
bodies). `rut/rt/` (the logger host) and `examples/03-plugin/server/`
are the in-tree shapes: `name = "server"` + `entry.type =
"./server.d.rut"`. Consumers reach them two ways: declared in the
manifest's `[deps]` (`server = { path = "../server" }` — resolved
recursively, first mount wins, RFC 0041 §3), or mounted programmatically
by the embedder (`Session::mount_dir` / `register_module`). A host pkg
is a real package — the loader lowers its `host fn` signatures into the
mounted surface at load time, and the compiler typechecks calls against
them: the role tur's `index.d.ts` plays today, but in-language,
type-checked, and packed into `.rutbundle`s like any dep (RFC 0038 v2).

One linkage keyword, one implementer:

| keyword | implementation lives in | bound at link against |
|---|---|---|
| `host fn` / `host struct` | the **embedding Rust** — typed bindings (`Vm::register_host_fn_sig`) | the load-time contract (below) |
| `builtin class` / `builtin trait` / `builtin fn` / `builtin impl` | **the engine itself** — compiler-lowered (ops / intrinsics / lowering hooks); the toolchain's std decl files only | nothing to bind; the decl is a pure signature contract |

`extern` is gone: rut→rut names resolve through use paths (RFC 0028) and the module loader
(RFC 0029 §5) and host→rut entry points are `entry fn` (RFC 0035 §3) —
`host` is the one foreign-body case left.

```rut
// server/server.d.rut — the chat-bus host pkg of 03-plugin
pub host fn subscribe(bus: Opaque, topic: str, handler: str);
pub host fn emit(bus: Opaque, topic: str, payload: str);
```

**There is no `host class`.** Native state crosses as an `Opaque` box
(RFC 0014, RFC 0016 §5) and rut wraps it in a class of its own — the
`Logger` pattern (RFC 0028), now the canonical native API shape:

```rut
// app side (or the consumer's own module): ordinary rut source
class Canvas {
    h: Opaque;
    fn circle(mut self, x: f32, y: f32, r: f32) -> nil { canvas_circle(self.h, x, y, r); }
    fn hits(self) -> i32 { return canvas_hits(self.h); }
    fn flush(mut self) -> nil { canvas_flush(self.h); }
}
```

The wrapper is ordinary rut: methods and impl blocks live there
(`impl Hashable for Canvas` in the wrapper's own module is legal,
RFC 0012 §2), bodies are auditable source the compiler can optimize
*around*, and every method costs exactly one host fn call — the same
single crossing a host-class method would have paid.

## The load-time contract (`.d.rut` ↔ host impl)

Mounting a host pkg *declares* its functions; the embedding Rust *binds*
bodies — **typed**, per the `.d.rut`: `Vm::register_host_fn_sig(name,
params, ret, body)`. The two sides are checked against each other at
load time, before any rut code runs (`Vm::verify_host_fns` against the
session's `expected_host_fns()` table, host-scope-aware: `rt` binds
`rt:log::*`). A mismatch is an embedder wiring bug — a **panic**, never
a rut diagnostic — on exactly three classes:

1. **declared but unbound** — a rut call would trap mid-run;
2. **bound but undeclared** — no surface declares what the host
   installed (a typo'd binding);
3. **signature drift** — the pkg declares `(Opaque, str, str) -> nil`,
   the binding took `(Opaque, i64, str)`; the crossing values would be
   misinterpreted.

The law follows the mount: an embedder that mounts `calc` declares its
26 float fns and must bind them (`install_std_math`); an embedder that
needs only `core` mounts only `core`. Mount what you bind.

## Why `host class` went (the decision record)

- **The boundary stays small and fast.** Everything a host fn sees is a
  scalar, an immutable buffer (`str`/`bytes`), a sum of those, or an
  `Opaque` handle — never a user cell layout, never a borrow-guarded
  user structure, never a vtable the host reaches back through. The
  crossing rule (RFC 0023 §1) is a compile-time property of the surface
  again, and marshaling is a fixed constant per argument.
- **Data accessors are ops, capability objects are host fns.** Members
  of the builtin containers are compiler-lowered (`OptSome`, `SumIs`,
  …, RFC 0032 §1.1) — fold/CSE-able. A native call blocks all of that;
  keeping data out of the native surface is the performance rule.
- **One nominal fiction fewer.** `host class` was a second class system
  (slots, ClassTables, instantiation builders, erased-storage generics)
  duplicating what rut classes + `Opaque` already do; `calc`'s
  `Math` was a namespace fiction over it. The wrapper class gives back
  the name, the methods, and the impl blocks with zero new machinery.
- **What is honestly lost:** admission-only bounds on native
  instantiations (`MyMap<Canvas, ..>` was a compile error; now any
  `Opaque` fits and `downcast` yields `None` on mismatch — checked,
  never a trap), per-instantiation type identity across the boundary,
  and native trait-object keys (`Hashable` vtable re-entry). Str keys
  hash host-side by content (a registered builtin impl); struct keys
  are the rut-side wrapper's business.

## The surface grammar (RFC 0030 §3)

```rut
host fn name(params) -> T;          // concrete signature; generics are a
                                    // compile error — a generic parameter
                                    // has no shape the boundary checks
host struct Name { fields }         // flat record; every field a
                                    // crossing type; no methods, no
                                    // field initializers — the host
                                    // constructs and reads it through
                                    // the field table (the shape IS the
                                    // whole surface)
builtin fn name<T>(params) -> T;    // engine fn, compiler-lowered
builtin class Name<T> { .. }        // engine type member contract
builtin trait Name<T> { .. }        // engine-woven contract
```

- **`host fn` signatures are concrete** over the crossing set: nil,
  primitives, `str`, `bytes`, `Option`/`Result` over crossable types,
  `Opaque` — and `host struct` records whose fields are all
  crossable. Everything else (struct cells, user classes, `Vec`,
  trait-typed values, closures) stays inside the VM; violating shapes are compile
  errors on the declaration.
- **`builtin` is the engine's own surface**, spelled in the toolchain's
  decl files only (`core`, `calc`): the builtin containers
  (`Array`/`Option`/`Result`/`Opaque`), the engine-lowered prelude fns
  (`own`, `downcast`, `assert`, `panic`, the `str`/`bytes` natives —
  all of `core`'s functions; the prelude registers **no** host
  bodies), and the **engine-woven traits** (`Iterator`; v1.1 removed `Disposal`/`Index` — `on_drop` and builtin indexing replaced them — and the async plan adds `Task` plus its run contexts) — contracts the engine has built-in knowledge of
  (compiler-backed impls, lowering hooks for `x[i]`, `for (x of it)`,
  rc-0 disposal). Users implement those with ordinary `impl` blocks:
  `builtin trait` is the engine's reservation, not an access rule.
  Library contracts without engine knowledge (`pouch`'s
  `Hashable`) stay plain `trait`. No embedder decl may spell
  `builtin` — a `builtin` outside the mounted std surface is a compile
  error. The decl is a pure signature contract (users and the LSP see
  every member); there is nothing to register and no slots.
- **Methods dispatch by slot, not name.** Compiling the declaration
  file assigns every host fn a stable slot id (declaration order); the
  module binary carries the slot table. Calls compile to `callnat
  { slot }` (RFC 0032) — names are binding-time labels for the binding
  side only, never dispatch keys, never in IR.
- **Visibility**: declaration files are ordinary modules — RFC 0003 §2
  applies. Non-exported declarations are *known* inside the module
  (callable via their slots) but *nameable* nowhere else.
- **Destructors map to Drop**: the host value lives in the `Opaque` box;
  when its rc hits 0, the Rust `Drop` runs at that point (RFC 0016 §3) —
  textures, sockets, and files release deterministically, never "at GC
  someday".
- **Workers**: an `Opaque` box may cross isolates only if the host
  registered the boxed type `send` (RFC 0021 §2) — checked at the
  transfer, by `TypeId`.

## Open questions

- OQ-1: should `rutc` grow a lint that a wrapper class's `h: Opaque`
  field flows only into one native module's fns (a soft stand-in for
  the lost nominal identity)?
- OQ-2: slot stability across compiler versions — recompiling a
  declaration file must agree with an already-registered impl; the decl
  digest (RFC 0033 §1) covers slots, so disagreement fails link — but is
  renumbering allowed at all across binary versions?
