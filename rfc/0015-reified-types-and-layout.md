# RFC 0015: Reified Types & Layout — `RutType`, repr C, Slots & Vtables

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** Part B (RFC 0004–0014)
- **Supersedes:** RFC 0002 §10, §3.2 + RFC 5002 §1–3 (pre-restructure)
- **Part:** B — Language surface (with internals sections)

## Summary

Every value's type is available at runtime as a `RutType` descriptor
(kind + width + for composites: field tables / vtable + for generics: the
instantiation descriptors). Slots are untagged 8-byte values — bytecode is
typed, so hot paths carry no tags; dataclass/bare-class values are inline
byte sequences spanning slots.

## 1. Reified runtime types

`RutType` descriptors are *not* first-class script values in v1 (OQ-1);
they are VM data. Language-facing surfaces of reification: the layout
builtins (§3), the `is` type tests (RFC 0012 §3), `Opaque` recovery
(RFC 0014), and the host boundary checks (RFC 0023).

## 2. Where reification is load-bearing

1. **Host boundary** — native fns declare parameter types once; the VM checks
   every call; no bridge-side coercion code (RFC 0022 §2).
2. **Opaque host types** — `host class Source<T>` (RFC 0025) carries its
   instantiation identity inside the handle: `store.get(src)` is checked
   statically *and* the handle knows its `T` (fixes tur's erased
   `Source<T>`).
3. **Type tests** — the `is` keyword (RFC 0012 §3), concrete and
   interface RHS alike;
   host fns declaring `dyn I` parameters get their arguments checked by
   the same machinery.
4. **Heterogeneous collections** — vtable dispatch (RFC 0012) needs the exact type
   reachable from every object header.
5. **Debugging** — `debug.type_name(x)`, stack traces (RFC 0036), formatter output.
6. **Serialization** — `std:reflect` walkers traverse `RutType`
   descriptors (RFC 0037).
7. **`Opaque` recovery** — `downcast<T>` (RFC 0014) checks the boxed cell's
   `TypeId`; `Opaque(v)` stamps it. Erasure without reification would be
   `any`; with reification it is a checked box.

## 3. Layout & type-identity builtins

```rut
let TID_POINT: u32 = type_id<Point>();
let SZ_POINT: u32  = size_of<Point>();     // 8 — two f32s, repr C
let AL_POINT: u32  = align_of<Point>();    // 4
```

- `type_id<T>(): u32` — identity of the *instantiated* type, unique per VM
  run and stable across modules (`Vec<f32>` ≠ `Vec<f64>`;
  `Array<i32, 3> ≠ Array<i32, 4>` — const-generic `N` is part of the
  identity, RFC 0005);
  `Point` = `Point` wherever declared). Comparable only — not a first-class
  type value (OQ-1).
- `size_of<T>(): u32`, `align_of<T>(): u32` — the value representation:
  primitives their width/alignment; dataclass/class their **repr C payload
  block** (§4) — what `own` copies clone (RFC 0011 §1) and what
  `StructRef` exposes to hosts (RFC 0024);
  `Array<T, N>` payload is `N × stride`, sized like any value type. Ref
  types (`Vec`, `string`, `Opaque`) report handle size, not payload —
  and so does
  **every `dyn` type** (`dyn Drawable`, `dyn Slice<i32>`: the
  slot stores the cell handle, RFC 0031 §4).
  (`u32`, not a word-sized type: rut has no `usize`, and no rut value or
  buffer may exceed 4 GiB in v1.)
- All three are **compile-time constants** — load-time expressions (RFC 0003 §1),
  folded by HIR from the type table, never executed (RFC 0033 §3).
- **Boxing widens** (RFC 0037 §3): `Opaque` of an int stores i64
  sign/zero-extended; a float, f64 — the §5 slot discipline. A kind
  branch plus `downcast<i64>` / `downcast<f64>` / `downcast<bool>` /
  `downcast<string>` is total in-branch.
- `Opaque` mirrors the layout builtins at runtime: `o.type_id(): u32`,
  `o.size(): u32`,
  `o.as_bytes(): Vec<u8>` (a snapshot of the box's repr-C payload). Together
  they enable **layout-aware heterogeneous storage** — group entries by
  `type_id`, preallocate `size_of`-sized slabs, compare payloads byte-wise —
  while recovery still goes through checked `downcast<T>`. Constructing an
  `Opaque` box (or any value) *from* raw bytes is deliberately **not**
  provided: it could forge private fields and class invariants.
- Host struct mirroring runs on the same numbers (`register_struct`
  checks size/align/offsets at startup — RFC 0024).

## 4. Value layout — repr C

Both `dataclass` and `class` values are **C-layout structs** — one layout
rule, no exceptions:

- Fields in declaration order, natural C alignment, struct size padded to
  its alignment. No hidden header, no tag, no vtable pointer inline.
- `private`, `implements`, `dispose`, `static`, and generic parameters add
  **nothing** to the block — layout depends only on the field list.
- The block is what `own(x)` clones (RFC 0011 §1), what
  `size_of<T>()`/`align_of<T>()` report (§3), and
  what the host mirrors with `#[repr(C)]` structs (RFC 0024) — rut↔host
  struct interop is pointer identity, not field-by-field conversion.
  Inside the block, **primitive fields sit inline; composite fields are
  cell-handle slots** (RFC 0016 §1) — a `Point { x, y }` block is two
  f32s; a `Rect { min: Point, max: Point }` block is two handles. Only
  buffers of primitive elements (`Vec<f32>`, `Array<i32, N>`) hold
  values inline (RFC 0016 §4).
- Every dataclass/class value lives in a `RutCell` — `Header +
  (vtable, when the type has impls) + the same block` (§6): sharing
  never re-lays-out fields, and `own` clones the block as-is.
- `enum`, `Option`, `Result` are *not* repr C (tagged layouts, RFC 0016 §5)
  and never cross the FFI as structs — pass their payload fields. Their
  **cell layout** (tag + payload slots):
  one `u32` tag slot followed by the payload's slots — a primitive
  payload inline, a composite payload a cell handle — size = max variant
  payload padded to the tag's alignment, fixed per type, part of the
  field table, and what `size_of<T>()` reports for them. Still tagged,
  still never repr C. Dataless enum variants are immortal singleton
  cells (RFC 0016 §1).

## 5. Internals: slots

Interpreter registers hold untagged 8-byte slots — the bytecode is typed, so
hot paths carry no tags:

```rust
#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,                       // all int widths, sign/zero-extended
    pub f: f64,                       // f32 payloads widened
    pub b: bool,
    pub c: char,
    pub r: Option<NonNull<Header>>,   // every non-primitive: value cells,
}                                     // vecs/slices, strings, enums,
                                      // interface fat-refs, Opaque boxes
```

Every non-primitive register holds a cell handle (RFC 0016 §1) —
payloads live inside cells, laid out by the compile-time field table;
only buffers of primitive elements store values inline between slots
(RFC 0016 §4). The bytecode is typed, so cell clones (`own(x)` —
`StructCopy { dst, src, size }`) are plain memcpys of the payload with
retain/release emitted for handle-typed fields.

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
    // class fields follow inline at fixed offsets — the SAME repr-C block
    // as §4: the cell adds the prefix, it never
    // re-lays-out. Hosts read the block through StructRef (RFC 0024).
    // No base prefix — no inheritance (RFC 0010 §3).
}

#[repr(C)]
struct VTable {
    ty: TypeId,                      // exact runtime type (points into RutType)
    dispose: Option<unsafe fn(*mut RutCell)>,  // cached Disposal.dispose trampoline
    slots: [CodePtr],                // interface method slots, global ids
}
```

Interface method ids are assigned **globally per interface instantiation**
at compile time (`Slice<Point>` ≠ `Slice<string>`, RFC 0012 §2); a
class's — or a dataclass's (RFC 0009) — vtable fills every slot of
every interface instantiation it declares `implements` **or
auto-implements** (std:reflect's protocols — RFC 0037; registry entries
for the builtin generics likewise, RFC 0022 §2). Every value cell is
minted at construction (the literal or the constructor) with its vtable
already attached; widening a composite to
`dyn I` **reuses the same cell and vtable** — the interface ref is the
handle plus the vtable pointer, no allocation, no copy (RFC 0011 §3). A
call through an
interface is two loads and an
indirect jump:

```rust
// d.draw(g)  where d: dyn Drawable, draw has global slot 3
Op::CallIface { recv, slot: 3, args } => {
    let obj = unsafe { regs[recv].r.unwrap().as_ref() as &RutCell };
    let f = unsafe { (*obj.vt).slots[3] };
    self.call_code(f, recv, args)?;    // `self` receiver arrives in recv
}
```

Direct inherent call for contrast (interface members NEVER devirtualize, RFC 0012 §1):

```text
Op::Call     { func: "Circle$area", recv, args }   ; c.area() — area INHERENT
```

### Type test

```rust
impl TypeTable {
    /// The `is` keyword (RFC 0012 §3) and host-boundary argument checks
    /// (`want` is a TypeId: exact for concrete RHS, the interface
    /// instantiation's for `x is I`). The exact type lives in the vtable;
    /// the descriptor lists the implemented interfaces — a flat scan, no
    /// inheritance chain to walk (RFC 0010 §3).
    fn is_a(&self, exact: TypeId, want: TypeId) -> bool {
        if exact == want { return true; }
        self.desc(exact).implements.iter().any(|&i| i == want)
    }
}
```

`is` with a concrete `T` monomorphizes with `want` as a compile-time
constant, so the check is: load the object's vtable `TypeId`, compare —
lowered `tidof` + `icmp` (RFC 0032 §1.1; the `Op::IsIface` op exists
only for the interface-RHS capability probe — `tidof` → descriptor →
implements scan — never for concrete tests; `downcast`'s check is
`tidof` + `br` + guarded `unbox`, RFC 0014). The widening
itself — a composite to `dyn I` — needs no dispatch
machinery and no allocation: an interface value already *is* the object
ref whose header reaches the vtable; `Opaque(v)` is the only box mint
in the language (RFC 0014).

## Open questions

- OQ-1: first-class type values (`type_of(x)` as a manipulable value).
  (RFC 0037's `TypeInfo` is a descriptor *handle* — data about types,
  not a type value — so this stays closed.)
