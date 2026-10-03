# Reified types and layouts

Types in rut are not a compile-time fiction. Every value's exact type is
available at runtime — generics included: a `Cache<i32>` and a
`Cache<str>` are *different types* to the VM, with distinct identities
and distinct method sets. This chapter explains where that reified
reality lives, what it is used for, and how values are actually laid
out.

## The mental model

Think of the VM as keeping a **type table**: one descriptor per
instantiated type — its kind, its field list, its itable fills, and for
composites the method tables its instances point at. Every heap cell's
header names an entry in that table. A value never "forgets" its type,
because its type is one field read away.

Types themselves are not first-class script values — you cannot put a
type in a variable or write a function over types. What you get is the
language-facing *surfaces* of reification:

- **`is` type tests** — `x is Circle` (exact) and `x is Drawable`
  (capability: does this value's type have a registered impl?).
- **Checked erasure** — `opaque.downcast<T>(o)` reads the box's runtime
  type and recovers the payload, or `nil` on a mismatch.
- **Host-boundary checks** — native functions declare parameter types
  once; every call is checked against them.
- **Debugging and traces** — error messages, backtraces, and formatter
  output name real runtime types.

## Why reification is load-bearing

The reason rut can check everything cheaply is that the runtime type is
always reachable:

1. **The host boundary needs no coercion code.** A native fn declares
   `(Opaque, str, str) -> nil` once; the VM checks each argument's
   runtime type at the crossing. There is no bridge-side
   "re-parse-and-pray" layer, because a wrong-shaped call fails loudly
   at the boundary.
2. **Erasure is checked, not blind.** `opaque(v)` stamps the box;
   `opaque.downcast<T>` consults the stamp. A box answers type tests *by
   the box* — `o is T` misses for every payload type — so the only way
   back to the payload is the checked recovery. Erasure without
   reification would be `any`; with reification it is a sealed box.
3. **Distinct instantiations.** `Vec<f32>` and `Vec<f64>` are different
   types everywhere — in the type table, in method resolution, at the
   host boundary. Nothing about a generic's type argument is erased.

## Slots: the untagged hot path

Inside the VM, registers and record fields are **untagged 8-byte
slots**. The bytecode is typed — every register's static type is
recorded in the function's signature and re-verified at load — so hot
paths carry no type tags at all:

```rust
#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,            // ints, bool: the canonical scalar width
    pub f: f64,            // floats: f32 widened
    pub r: *mut CellVal,   // every non-primitive: a cell handle
}
```

A primitive is its bits; a composite is a handle to a cell. The tagged
world — a value that carries its type with it — exists only at the host
boundary, where Rust code that cannot trust static types receives
checked, typed values.

## Record layout: one slot per field

Both `struct` and `class` values live in a cell: a header (refcount +
type id), a pointer to the type's vtable when the type has methods, and
the payload — **a slot array, one untagged slot per field, in
declaration order**.

- Primitive fields are widened into their slot; composite fields are
  cell handles.
- Visibility, generic parameters, and impl blocks elsewhere add
  *nothing* — the payload depends only on the field list.
- Field access is by index; there are no hidden members and no
  inheritance, so there is no base-class prefix and nothing to walk.
- Fixed arrays of primitives (`[f32]`) keep their elements **flat and
  packed** at machine width — the one place values live inline. Growable
  sequences are rut-library classes over such arrays.

Consequences:

- `size_of`/`align_of` do not exist, deliberately. Layout is an
  implementation detail, not an API — rut has no C-ABI struct surface
  for hosts to mirror. Hosts see records only through the checked
  boundary (see [the host boundary](host-boundary.md)).
- A record's memory cost is its field count times 8 bytes plus the cell
  header, which is exactly what the heap budget charges.
- Widening a value to an interface type allocates nothing: the interface-typed
  value *is* the same cell, reinterpreted through its vtable.

## Vtables: one per type, attached at construction

A cell's vtable is filled when the value is constructed and never
changes. It names the exact type and holds one entry per interface member
the type was satisfaction-boxed at, keyed by a global (interface
instantiation, member) id
— `Slice<Point>` and `Slice<str>` have separate slot sets, because they
are separate interface instantiations.

A call through an interface object is two loads and an indirect jump:

```text
d.draw(g)          ; d: Drawable, draw has global slot 3
  obj  <- d.cell
  code <- obj.vtable.slots[3]
  call code(d, g)
```

An inherent method call — `c.area()` on a concrete `c` — compiles to a
direct call with no table involved. The dispatch rules that choose
between the two are the subject of the next chapter,
[interfaces and dispatch](interfaces-and-dispatch.md).

## Type tests, lowered

Every type test is a small, pure read:

- **Concrete test** (`d is Circle`): load the value's runtime type id,
  compare. When the receiver's static type already answers, the compiler
  folds it to a constant.
- **Interface probe** (`x is Drawable`): load the type id, scan the
  descriptor's registered impl list. Pure in its inputs, so repeated
  probes deduplicate and invariant ones hoist.
- **Downcast** (`opaque.downcast<T>(o)`): the same type-id compare,
  followed by the guarded payload extract.

There is no runtime
layout introspection — the descriptors serve the VM, the checks, and
tooling, not userland metaprogramming.

## What this buys you

- Type errors are **runtime facts, not conventions**: a host call, a
  downcast, or an interface probe is checked against the same table the VM
  dispatches through.
- **One runtime truth per value** — the vtable that answers dispatch is
  the same one that answers `is`.
- The tag-free hot path keeps the interpreter's arithmetic and field
  traffic at raw slot speed while reification costs only what the
  program's dynamic features actually use.
