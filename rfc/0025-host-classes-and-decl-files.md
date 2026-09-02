# RFC 0025: Host Classes & Declaration Files

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0022 (embedding), RFC 0012 (traits), RFC 0010
  (class methods & construction), RFC 0016 §3 (Drop mapping), RFC 0029 (declaration files)
- **Supersedes:** RFC 0005 §5 (pre-restructure; `extern class` renamed to
  `host class` — "host" names where the implementation lives)
- **Part:** E — Host & FFI

## Summary

Native surfaces are declared **in rut source** — *declaration files*
(`.d.rut`) whose declarations are pure surface (signatures, no bodies).
The specifier maps to a declaration file (`"app:gfx"` → `app/gfx.d.rut`,
`"plugin:my_map"` → `plugin/my_map.d.rut`, `"std:collection"` likewise):
compiled and verified like any module, generating no code of its own. rutc,
the LSP, and AOT binary builds see the surface with **zero Rust linked** —
the role tur's `index.d.ts` plays today, but in-language and type-checked.

Two declaration keywords, two linkage targets (RFC 0029 — both are
**`.d.rut`-only**; a `.rut` file that spells either is a compile error):

| keyword | implementation lives in | bound at link against |
|---|---|---|
| `host fn` / `host class` | the **embedding Rust** — a registered `NativeModule` | the ClassTable / fn table reflection |
| `extern fn` / `extern class` | **another rut compilation unit** — a published `.rutc` binary | the linked package binary |

```rut
// app/gfx.d.rut — declaration file for "app:gfx"
host fn newCanvas(w: i32, h: i32): Canvas;      // module-level host fn

export host class Canvas {                      // exported: nameable outside
    fn circle(self, x: f32, y: f32, r: f32): unit;
    fn flush(self): unit;
}

export host class Source<T> {                   // generic — instantiation
    fn get(self): T;                            // identity KEPT on the value:
}                                               // Source<i32> != Source<string>

host class Fence {                              // NOT exported: known inside
    fn signal(self): unit;                      // app/gfx (callable via its
}                                               // slot), nameable nowhere else
```

- A `host class` declaration may contain method signatures — instance
  methods (`fn set(self, k: K, v: V): unit;`) and **class methods**
  (`fn new(cap: i32): Self;`, no `self` — RFC 0010 §2), the latter the
  native construction surface: `MyMap.new(cap)` is an ordinary method
  call (RFC 0026). Host class
  methods spell the `self` receiver like rut
  methods (RFC 0010 §2; the Rust side's
  `this: &mut MyMap` parameter is that receiver). It may not
  declare fields: host instances box host values, not rut field
  blocks. An `extern class` (rut-package surface) has the same shape for
  the same reason — the package's fields are its business; you get
  class methods and instance methods.
- **Param bounds on host/extern decls are admission-only syntax**: `K:
  Hashable` constrains which instantiations compile (checked against
  the trait + its `requires` graph, RFC 0012 §2) and grants nothing
  else — no method calls on bare `K`, no static dispatch. User
  generics keep no bounds (RFC 0013 OQ-1 untouched); a bound would be
  pure forwarding anyway (RFC 0026).
- Host instances are `RutOpaque` heap objects (RFC 0016 §5) holding a boxed
  host value; the header `TypeId` carries the class **and** its generic
  instantiation (`Source<i32>` ≠ `Source<string>` — the tur bug fixed
  structurally, RFC 0015 §2).
- **Methods dispatch by slot, not name.** Compiling the declaration file
  assigns every host/extern member a stable slot id (declaration order); the
  module binary carries the slot table. Calls compile to `callnat {
  slot }` (RFC 0032) — member names are binding-time labels for the
  binding side only (RFC 0026), never dispatch keys, never in IR.
- **Visibility**: declaration files are ordinary modules — RFC 0003 §2
  applies. Non-exported declarations are *known* inside the module (callable
  via their slots) but *nameable* nowhere else; slots are always
  assigned (private members need them for intra-module calls).
- **Destructors map to Drop**: when rc hits 0, the host value's Rust
  `Drop` runs at that point (RFC 0016 §3) — textures, sockets, and files
  release deterministically, never "at GC someday".
- **Workers**: a `host class` value may cross isolates only if the host
  registered the type `send` (RFC 0021 §2) — checked at the transfer, by
  `TypeId`.
- Host fns returning `Opaque` accept any rut value (RFC 0014) — the
  checked escape hatch for data with no static shape.

## Open questions

- OQ-1: `host class` class methods (methods without `self` —
  host-namespaced functions today) — keep as plain host fns; do not
  duplicate the feature.
- OQ-2: slot stability across compiler versions — recompiling a
  declaration file must agree with an already-registered impl; the decl
  digest (RFC 0033 §1) covers slots, so disagreement fails link — but is
  renumbering allowed at all across binary versions?
