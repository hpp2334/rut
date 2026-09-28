# repr(C) struct interop

rut has **no C-layout struct interop**. There is no `#[repr(C)]` mirror
contract, no host-side zero-copy view over rut record fields, and none of
the API that such a contract implies: `register_struct::<T>()`,
`StructRef<'v, T>`, `size_of<T>()`, `align_of<T>()`, or per-field offsets
exposed to the host. The title is kept because the question comes up on
every FFI: here is what actually exists.

## The layout rut records really have

A user record (struct or dataclass) is an **array of 8-byte slots**:

| rule | content |
|---|---|
| slot width | 8 bytes; primitive fields sit inline in their slot |
| composite fields | a **cell-handle slot** — a reference to the shared cell ([the Rc heap](rc-heap.md)) |
| declaration order | fields keep declaration order; no hidden members; visibility and out-of-body `impl` blocks change nothing |
| heap | non-moving — a slot's handle stays valid for the cell's life |
| where it lives | the module's reified type table, not a header contract ([reified types and layout](reified-types.md)) |

This layout is a VM-internal invariant, not a stable ABI. It is never
promised to Rust code byte-for-byte, and nothing in the toolchain checks a
host mirror against it.

## What the host sees instead

A host touches record data only through the checked boundary
([value boundary](value-boundary.md)):

- **Flat crossing records** — declare a `host struct` in the package's
  declaration file. Fields-only, every field a crossing type, no methods.
  The host constructs and reads the values through the declared field
  table; the shape is the whole surface.

  ```rut
  // server.d.rut
  pub host struct Point { x: f64, y: f64, tag: str }
  pub host fn measure(p: Point) -> f64;
  ```

- **Opaque state** — the rut side holds a class wrapping an `opaque`
  handle; the host owns the layout entirely
  ([native containers API surface](native-containers.md)).
- **Structured reads** — for reflection over arbitrary records (field
  names, types, children by index), use the reflection surface
  ([reflection](reflection.md)). It is descriptor-based and read-only; it
  never exposes raw memory.

## Why there is no shared-layout contract

- The boundary's crossing set is deliberately scalars, immutable buffers,
  and `opaque` handles; a borrowed user structure with host-writable
  fields would reintroduce aliasing the borrow guards exist to prevent.
- Records are shared cells, not values: two bindings alias one record, so
  a `&mut` C view would race with rut-side mutation.
- Layout stability belongs to the module binary and its type table
  ([module binary and verification](module-binary.md)) — a compile-checked,
  versioned artifact — not to a C header.

## Record values at the boundary

| rut shape | what the host sees |
|---|---|
| tuple (a record of crossing-typed fields) | a Rust tuple, positionally, field-by-field under the record's declared field types — arity 1–8 ([value boundary](value-boundary.md)) |
| `host struct` | the declared flat record, decoded through the field table; the shape is the whole surface |
| user record (struct/dataclass) | never crosses whole — pass it as `opaque`, mirror it as a `host struct`, or walk it with reflection |
| `Vec<T>` / `[T]` | never crosses; per-element fns, or `bytes` for raw payloads |

The positional tuple decode is the one place record *structure* reaches
Rust, and it is fully checked: a field whose type does not cross is a
compile error on the entry, and a runtime shape mismatch traps naming
both sides.

## What the toolchain checks instead

A shared-layout contract would be checked once, at registration. rut
replaces it with checks it can actually enforce:

| guarantee | enforced by |
|---|---|
| every value a host receives has the declared type | the boundary decode ([value boundary](value-boundary.md)) |
| surfaces and bindings agree | the boot join ([host fns](host-fns.md)) |
| a consumer and a published package agree | the decl digest at link ([module binary and verification](module-binary.md)) |
| registers hold their declared types | the verifier ([typed bytecode](typed-bytecode.md)) |

## Practical recipes

| need | shape |
|---|---|
| pass numeric payloads cheaply | cross the fields as primitives, or pack into `bytes` |
| share mutable native state | `opaque` box + wrapper class |
| read rut records from the host | reflection descriptors, or a `host struct` mirror |
| fixed binary records | `bytes` + `b.len()`/decode helpers ([primitive types](primitive-types.md)) |
