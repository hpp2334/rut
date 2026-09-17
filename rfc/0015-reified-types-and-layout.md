# RFC 0015: Reified Types — `RutType`, Slots & Vtables

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** Part B (RFC 0004–0014)
- **Supersedes:** RFC 0002 §10, §3.2 + RFC 5002 §1–3 (pre-restructure)
- **Part:** B — Language surface (with internals sections)

## Summary

Every value's type is available at runtime as a `RutType` descriptor
(kind + for composites: field tables / vtable + for generics: the
instantiation descriptors). Slots are untagged 8-byte values — bytecode is
typed, so hot paths carry no tags; dataclass/class payloads are arrays of
those slots.

## 1. Reified runtime types

`RutType` descriptors are *not* first-class script values in v1 (OQ-1);
they are VM data. Language-facing surfaces of reification: the
`type_id<T>()` builtin (§3), the `is` type tests (RFC 0012 §3), `Opaque`
recovery (RFC 0014), and the host boundary checks (RFC 0023).

## 2. Where reification is load-bearing

1. **Host boundary** — native fns declare parameter types once; the VM checks
   every call; no bridge-side coercion code (RFC 0022 §2).
2. **Opaque host values** — a host fn's `Opaque` parameter is checked
   against the box's runtime `TypeId` at the boundary (the boxed value
   keeps its type, RFC 0014); rut wrapper classes restore the nominal
   shape rut-side (RFC 0025, revised).
3. **Type tests** — the `is` keyword (RFC 0012 §3), concrete and
   trait RHS alike;
   host fns declaring trait-typed parameters get their arguments checked by
   the same machinery.
4. **Heterogeneous collections** — vtable dispatch (RFC 0012) needs the exact type
   reachable from every object header.
5. **Debugging** — `debug.type_name(x)`, stack traces (RFC 0036), formatter output.
6. **Serialization** — `reflect` walkers traverse `RutType`
   descriptors (RFC 0037).
7. **`Opaque` recovery** — `downcast<T>` (RFC 0014) checks the boxed cell's
   `TypeId`; `Opaque.new(v)` stamps it. Erasure without reification would be
   `any`; with reification it is a checked box.

## 3. Type-identity builtin

```rut
let TID_POINT: u32 = type_id<Point>();
```

- `type_id<T>() -> u32` — identity of the *instantiated* type, unique per VM
  run and stable across modules (`Vec<f32>` ≠ `Vec<f64>`;
  `Array<i32, 3> ≠ Array<i32, 4>` — const-generic `N` is part of the
  identity, RFC 0005);
  `Point` = `Point` wherever declared). Comparable only — not a first-class
  type value (OQ-1).
- It is a **compile-time constant** — a load-time expression (RFC 0003 §1),
  folded by HIR from the type table, never executed (RFC 0033 §3).
- **Boxing widens** (RFC 0037 §3): `Opaque` of an int stores i64
  sign/zero-extended; a float, f64 — the §5 slot discipline. A kind
  branch plus `downcast<i64>` / `downcast<f64>` / `downcast<bool>` /
  `downcast<str>` is total in-branch.
- `Opaque` mirrors identity at runtime, `o.type_id() -> u32`. Constructing
  an `Opaque` box (or any value) *from* raw bytes is deliberately **not**
  provided: it could forge private fields and class invariants.
- There is **no `size_of<T>()` / `align_of<T>()`** and no runtime payload
  footprint accessor: records are slot arrays (§4), so value size and
  alignment are implementation details, not a language surface
  (repr-C layouts were withdrawn — see §4).

## 4. Value representation — slot arrays

Both `dataclass` and `class` values live in a `RutCell` — `Header +
(vtable, when the type has impls) + the payload` (§6). The payload is a
**slot array: one untagged 8-byte `Slot` per field, in declaration order**.
Primitive fields are widened into their slot (sign/zero-extended; `f32`
stored as `f64` per §5); composite fields are cell-handle slots
(RFC 0016 §1). There is no byte-packed C block and no field-offset table:
field access is by index (`GetF`/`SetF`), `own(x)` clones slot by slot
(RFC 0011 §1), and heap accounting charges `fields × 8` bytes.

- Visibility, generic parameters, and out-of-body impl blocks add
  **nothing** — the payload depends only on the field list.
- Buffers of primitive elements (`Vec<f32>`, `Array<i32, N>`) stay flat
  and packed to the element's machine width (RFC 0016 §4); composite
  elements are one handle slot each.
- Builtin `Option`/`Result` and user enums are tagged cells (RFC 0016 §5),
  represented as a tag slot plus a payload slot; dataless enum variants
  are immortal singleton cells (RFC 0016 §1).
- Host pointer-identity struct interop (former RFC 0024) is **withdrawn**:
  a host sees records only through the boundary `Value` (RFC 0023), and
  struct fields cross as returned/borrowed handle values.

## 5. Internals: slots

Interpreter registers hold untagged 8-byte slots — the bytecode is typed, so
hot paths carry no tags:

```rust
#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,                       // ints and bool/char as i64 (RFC 0007):
    pub f: f64,                       //   the canonical scalar width
    pub b: bool,                      // (constructors write the full 8
    pub c: char,                      //   bytes — untagged discipline)
    pub r: *const CellVal,            // every non-primitive: value cells,
}                                     // vecs/slices, strings, enums,
                                      // Opaque boxes — a stable raw
                                      // pointer (RFC 0039 §3)
```

Every non-primitive register holds a cell handle (RFC 0016 §1) —
payloads live inside cells as slot arrays (§4); only buffers of primitive
elements store values inline between slots (RFC 0016 §4). The bytecode is
typed, so `own(x)` clones the payload slot by slot, retaining/releasing
each handle-typed field.

The tagged `Value` enum exists only at the host FFI boundary (RFC 0023).

## 6. Internals: cells & vtables

```rust
#[repr(C)]
struct RutCell {                     // heap object: EVERY user value
                                     // (dataclass or class — RFC 0009/0010)
    h: Header,                       // rc + type id (RFC 0016 §5)
    vt: *const VTable,               // exact type's vtable — class OR
                                     // dataclass (RFC 0009) — set at
                                     // construction (null when the type
                                     // has no impls)
    // class fields follow inline as one payload slot each — the slot array
    // of §4: the cell adds the prefix, it never re-lays-out.
    // No base prefix — no inheritance (RFC 0010 §3).
}

#[repr(C)]
struct VTable {
    ty: TypeId,                      // exact runtime type (points into RutType)
    dispose: Option<unsafe fn(*mut RutCell)>,  // cached Disposal.dispose trampoline
    slots: [CodePtr],                // trait method slots, global ids
}
```

Trait method ids are assigned **globally per trait instantiation**
at compile time (`Slice<Point>` ≠ `Slice<str>`, RFC 0012 §2); a
class's — or a dataclass's (RFC 0009) — vtable fills every slot of
every trait instantiation it has an impl for — **user impl blocks,
auto-fills** (reflect's protocols — RFC 0037; registry entries
for the builtin generics likewise, RFC 0022 §2). Every value cell is
minted at construction — the `Self { .. }` literal inside a class
  method (RFC 0010 §1) — with its vtable
already attached; widening a composite to
a trait `I` **reuses the same cell and vtable** — the trait-typed ref is the
handle plus the vtable pointer, no allocation, no copy (RFC 0011 §3). A
call through a
trait object is two loads and an
indirect jump:

```rust
// d.draw(g)  where d: Drawable, draw has global slot 3
Op::CallTrait { recv, slot: 3, args } => {
    let obj = unsafe { regs[recv].r.unwrap().as_ref() as &RutCell };
    let f = unsafe { (*obj.vt).slots[3] };
    self.call_code(f, recv, args)?;    // `self` receiver arrives in recv
}
```

Direct inherent call for contrast (trait members NEVER devirtualize, RFC 0012 §1):

```text
Op::Call     { func: "Circle$area", recv, args }   ; c.area() — area INHERENT
```

### Type test

```rust
impl TypeTable {
    /// The `is` keyword (RFC 0012 §3) and host-boundary argument checks
    /// (`want` is a TypeId: exact for concrete RHS, the trait
    /// instantiation's for `x is I`). The exact type lives in the vtable;
    /// the descriptor lists the trait impls — a flat scan, no
    /// inheritance chain to walk (RFC 0010 §3).
    fn is_a(&self, exact: TypeId, want: TypeId) -> bool {
        if exact == want { return true; }
        self.desc(exact).impls.iter().any(|&i| i == want)
    }
}
```

`is` with a concrete `T` monomorphizes with `want` as a compile-time
constant, so the check is: load the object's vtable `TypeId`, compare —
lowered `tidof` + `icmp` (RFC 0032 §1.1; the `Op::IsTrait` op exists
only for the trait-RHS capability probe — `tidof` → descriptor →
impls scan — never for concrete tests; `downcast`'s check is
`tidof` + `br` + guarded `unbox`, RFC 0014). The widening
itself — a composite to a trait `I` — needs no dispatch
machinery and no allocation: a trait-typed value already *is* the object
ref whose header reaches the vtable; `Opaque.new(v)` is the only box mint
in the language (RFC 0014).

## Open questions

- OQ-1: first-class type values (`type_of(x)` as a manipulable value).
  (RFC 0037's `TypeInfo` is a descriptor *handle* — data about types,
  not a type value — so this stays closed.)
