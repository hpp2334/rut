# Reified types and layout

Every value's type is available at runtime as a type descriptor. Slots
are untagged 8-byte values; bytecode is typed, so hot paths carry no
tags. This page is the language-facing contract; the descriptor itself
is VM data, not a script value.

## Where reification is language-facing

| Surface | Mechanism |
|---|---|
| `is` type tests — concrete and interface RHS | the value cell's descriptor (see [Interfaces and dispatch](interfaces.md)) |
| `opaque.downcast<T>` recovery | the box's recorded runtime type (see [opaque — erasure and downcast](opaque.md)) |
| `type_id<T>()` | the compile-time type-identity constant |
| host boundary checks | every crossing is checked against the declared parameter type |
| diagnostics | stack traces and host tooling read the same descriptors |

## `type_id<T>()`

```rut
use ink::{ Logger };

struct Point { x: i32; y: i32; }

entry fn main() {
    let log = Logger("t");
    let TID_POINT: u32 = type_id<Point>();
    log.info(f"{TID_POINT} eq={type_id<Point>() == TID_POINT}");
}
```

```text
23 eq=true
```

- `type_id<T>() -> u32` — the identity of the **instantiated** type:
  unique per VM run, stable across modules, comparable only.
  `Vec<f32>` ≠ `Vec<f64>`; `Point` = `Point` wherever declared.
- It is a **compile-time constant** — a load-time expression (legal in
  module-level `let` initializers, see
  [Modules and visibility](modules-and-visibility.md)), folded from the
  type table, never executed.
- Value size and alignment are implementation details, not a language
  surface.
- Constructing a value *from* raw bytes is deliberately not provided:
  it could forge private fields and class invariants.

## Value representation — slot arrays

Every struct and class value lives in a heap **cell**:
header + (vtable, when the type has impls) + the payload.

- The payload is a **slot array: one untagged 8-byte slot per field, in
  declaration order**. Primitive fields are widened into their slot
  (sign/zero-extended; a float is stored at the canonical 64-bit
  width); composite fields are cell-handle slots.
- Visibility, generic parameters, and impl blocks add **nothing** — the
  payload depends only on the field list. Field access is by index;
  heap accounting charges `fields × 8` bytes.
- Buffers of primitive elements (`Vec<f32>`, `[i32]`) stay **flat**,
  packed to the element's machine width; composite elements are one
  handle slot each — `Vec<Point>` stores one handle per element.
- Enums are tagged cells (a tag slot plus a payload slot); dataless
  enum variants are immortal singleton cells.
- Nullable `?prim` element storage inside sequence backings is the raw
  payload plus a one-byte nil tag — no per-element cell.
- `bytes` at the engine level is a `u8` array cell; a `str` is an
  immutable, COW-shared UTF-8 block, and a slice of it is a small view
  cell (see [String slicing and views](string-views.md)).

## Cells and vtables

```text
RutCell := Header { rc, type id } VTable* Payload
VTable := { exact type id, dispose trampoline, interface member slots }
```

- The **exact runtime type** lives in the cell (via the vtable when
  present, the header otherwise). Every cell is minted at construction
  with its vtable already attached.
- Interface member ids are assigned **globally per interface
  instantiation** at compile time (`Slice<Point>` ≠ `Slice<str>`); a
  type's vtable fills every slot of every interface instantiation it
  was satisfaction-boxed at — the fill is synthesized by the boxing
  site's boundary check, and a type that never crosses an
  interface-typed boundary fills nothing.
- Widening a composite to an interface `I` **reuses the same cell and
  vtable**: the interface-typed value is the handle plus the vtable
  pointer — no allocation, no copy. The vtable reserves no
  base-prefix room: there is no inheritance.
- A call through an interface object is two loads and an indirect jump
  (the receiver's vtable, the member slot). Interface members never
  devirtualize; inherent calls bind directly:

```text
Op::CallI { recv, slot: 3, args }   // s.area() — itable slot 3
Op::Call     { func: "Circle$area", recv, args }   // c.area() — inherent
```

## The type test

`is` with a concrete right-hand side lowers to: load the object's
exact type id, compare — a compile-time constant comparison when the
static type already answers. The interface-RHS capability probe is:
exact-type compare, then a flat scan of the descriptor's filled
itable slots — no inheritance chain to walk (see
[Interfaces and dispatch](interfaces.md)).

`opaque` interacts with exactly one of these reads: `is` reads the
**box's** own type, so every payload probe misses; only
`opaque.downcast<T>`'s match test keeps reading the payload — the one
legitimate see-through.

## Boxing

Storing a value into an erasure box widens by the slot discipline: an
integer box stores the 64-bit sign/zero-extended value, a float the
64-bit width. A kind check plus
`opaque.downcast<i64>()` / `downcast<f64>()` / `downcast<bool>()` /
`downcast<str>()` is therefore total in-branch: within the successful
branch the value's type is the exact concrete type, and everything
downstream optimizes as if the value had never been erased.
