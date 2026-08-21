# RFC 0015: Reified Types & Layout — `RutType`, repr C, Slots & Vtables

- **Status:** Draft
- **Date:** 2026-08-22
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
builtins (§3), `is<T>`/`upcast<T>` (RFC 0012 §3), `Opaque` recovery
(RFC 0014), and the host boundary checks (RFC 0023).

## 2. Where reification is load-bearing

1. **Host boundary** — native fns declare parameter types once; the VM checks
   every call; no bridge-side coercion code (RFC 0022 §2).
2. **Opaque host types** — `host class Source<T>` (RFC 0025) carries its
   instantiation identity inside the handle: `store.get(src)` is checked
   statically *and* the handle knows its `T` (fixes tur's erased
   `Source<T>`).
3. **Type tests & upcasts** — `is<T>` / `upcast<T>` builtins (RFC 0012 §3);
   host fns declaring interface-typed parameters get their arguments checked by
   the same machinery.
4. **Heterogeneous collections** — vtable dispatch (RFC 0012) needs the exact type
   reachable from every object header.
5. **Debugging** — `debug.type_of(x)`, stack traces, formatter output.
6. **Serialization** — stdlib walkers traverse `RutType` descriptors.
7. **`Opaque` recovery** — `downcast<T>` (RFC 0014) checks the boxed cell's
   `TypeId`; `Opaque(v)` stamps it. Erasure without reification would be
   `any`; with reification it is a checked box.

## 3. Layout & type-identity builtins

```rut
const TID_POINT: u32 = type_id<Point>();
const SZ_POINT: u32  = size_of<Point>();     // 8 — two f32s, repr C
const AL_POINT: u32  = align_of<Point>();    // 4
```

- `type_id<T>(): u32` — identity of the *instantiated* type, unique per VM
  run and stable across modules (`Array<f32>` ≠ `Array<f64>`;
  `Point` = `Point` wherever declared). Comparable only — not a first-class
  type value (OQ-1).
- `size_of<T>(): u32`, `align_of<T>(): u32` — the value representation:
  primitives their width/alignment; dataclass/class their **repr C field
  block** (§4) — the number `Array<T>` strides by and value copies copy;
  ref types (`Array`, `string`, `Rc<T>`) report handle size, not payload.
  (`u32`, not a word-sized type: rut has no `usize`, and no rut value or
  buffer may exceed 4 GiB in v1.)
- All three are **compile-time constants** — const-expressions (RFC 0003 §1),
  folded by HIR from the type table, never executed (RFC 0033 §3).
- `Opaque` mirrors them at runtime: `o.type_id(): u32`, `o.size(): u32`,
  `o.as_bytes(): bytes` (a snapshot of the box's repr-C payload). Together
  they enable **layout-aware heterogeneous storage** — group entries by
  `type_id`, preallocate `size_of`-sized slabs, compare payloads byte-wise —
  while recovery still goes through checked `downcast<T>`. Constructing an
  `Opaque` (or any value) *from* raw bytes is deliberately **not**
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
- The block is what assignment copies, what `Array<T>` stores inline
  (RFC 0016 §4), what `size_of<T>()`/`align_of<T>()` report (§3), and
  what the host mirrors with `#[repr(C)]` structs (RFC 0024) — rut↔host
  struct interop is pointer identity, not field-by-field conversion.
- An `Rc<C>` cell wraps the *same* block behind `Header + vtable pointer`
  (§6): boxing never re-lays-out fields.
- `enum`, `Option`, `Result` are *not* repr C (tagged layouts, RFC 0016 §5)
  and never cross the FFI as structs — pass their payload fields.

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
    pub r: Option<NonNull<Header>>,   // Rc cells, arrays, strings, bytes,
}                                     // interface values, Option/Result of refs
```

Dataclass and bare class values don't fit a slot — they are **inline byte
sequences** spanning consecutive slots (or a stack-frame region), laid out
by the compile-time field table; `Array<Point>` elements are contiguous
inline values with no per-element headers (RFC 0016 §4). The bytecode is
typed, so value copies (`StructCopy { dst, src, size }`) are plain memcpys
with ref-field retain/release emitted by the compiler.

The tagged `Value` enum exists only at the host FFI boundary (RFC 0023).

## 6. Internals: Rc cells & vtables

```rust
#[repr(C)]
struct RutClass {                    // heap object: an Rc<T> CELL (RFC 0011)
    h: Header,                       // rc + type id (RFC 0016 §5)
    vt: *const VTable,               // exact type's vtable — class OR
                                     // dataclass (RFC 0009) — set
                                     // at `Rc(v)`
    // class fields follow inline at fixed offsets — the SAME repr-C block
    // as the bare value (§4): boxing adds the prefix, it never
    // re-lays-out. Hosts read the block through StructRef (RFC 0024).
    // No base prefix — no inheritance (RFC 0010 §3).
}

#[repr(C)]
struct VTable {
    ty: TypeId,                      // exact runtime type (points into RutType)
    dispose: Option<unsafe fn(*mut RutClass)>,  // RFC 0011 §2 destructor
    slots: [CodePtr],                // interface method slots, global ids
}
```

Interface method ids are assigned **globally per interface instantiation**
at compile time (`Equal<Point>` ≠ `Equal<string>`, RFC 0012 §2); a
class's — or a dataclass's (RFC 0009) — vtable fills every slot of
every interface instantiation it declares `implements` (the cell is
minted by `Rc(v)` or by implicit boxing at an
interface widening). A call through an interface is two loads and an
indirect jump:

```rust
// d.draw(g)  where d: Drawable, draw has global slot 3
Op::CallIface { recv, slot: 3, args } => {
    let obj = unsafe { regs[recv].r.unwrap().as_ref() as &RutClass };
    let f = unsafe { (*obj.vt).slots[3] };
    self.call_code(f, recv, args)?;    // `this` passed as receiver register
}
```

Devirtualized direct call for comparison:

```text
Op::Call     { func: "Circle$area", recv, args }   ; c.area(), c: Circle
```

### Type test & upcast

```rust
impl TypeTable {
    /// `is<T>(x)` builtin and host-boundary argument checks. The exact
    /// type lives in the vtable; the descriptor lists the implemented
    /// interfaces — a flat scan, no inheritance chain to walk (RFC 0010 §3).
    fn is_a(&self, exact: TypeId, want: TypeId) -> bool {
        if exact == want { return true; }
        self.desc(exact).implements.iter().any(|&i| i == want)
    }
}
```

`is<T>` monomorphizes with `want` as a compile-time constant, so the check
is: load the object's vtable `TypeId`, compare, then (rarely) scan the
descriptor's flat `implements` list. `upcast<T>` needs **no runtime code at
all** — an interface value already *is* the object ref whose header reaches
the vtable — it erases to a plain `MovRef` (often to nothing).

## Open questions

- OQ-1: first-class type values (`type_of(x)` as a manipulable value).
