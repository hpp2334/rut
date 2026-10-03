# Value boundary and borrows

The one place tagged values exist: the host boundary. Inside the VM,
registers are untagged slots ([the VM heap](vm-heap.md)); crossing into Rust
materializes a checked, borrow-guarded value. The currency of the boundary
is **typed Rust**: the registered closure's parameter types and the
`vm.call` return type are the whole contract. The internal marshaling formats
stay out of it: `Slot` never appears, and the raw `Value` enum surfaces only
as the one untyped read in the map below.

## The crossing set

What may cross is a **compile-time property of the surface**. A violation is
a compile error on the declaration or binding — never a call-time failure.

- **Parameters** cross over: the primitives (`i8`–`i64`, `u8`–`u64`,
  `f32`, `f64`, `bool`), `str`, `bytes`, and `opaque`.
- **Returns** cross over the same set **plus** the answer optionals
  `?str`, `?bytes`, `?opaque`.
- Tuples cross field-by-field; each field must itself cross.
- Entry fns follow the same rule in both directions: `?T` crosses iff `T`
  crosses, decoded nil-flattened.
- Everything else — structs, classes, `Vec<T>`, `[T]`, interface-typed
  values, closures — stays inside the VM. Seal polymorphic values in the
  erasure box: `opaque(v)` at the call, `opaque.downcast<T>(v)` after
  ([opaque — erasure and downcast](opaque.md)).

## Rust ↔ rut type map

| Rust type | rut type | direction | cost |
|---|---|---|---|
| `i8` `i16` `i32` `i64` | same | both | the raw slot bits |
| `u8` `u16` `u32` `u64` | same | both | raw bits (`u64` is never narrowed through `i64`) |
| `f32` `f64` `bool` | same | both | raw slot bits |
| `()` | `nil` | both | the zero word |
| `&str` | `str` | param only | **zero-copy** borrow of the block store, scoped to the call |
| `String` | `str` | both | owned copy |
| `&[u8]` | `bytes` | param only | zero-copy borrow |
| `Vec<u8>` | `bytes` | both | owned copy |
| `OpaqueRef` | `opaque` | both | the handle; the rc **transfers** across |
| `Opaque<T>` | `opaque` | both | typed payload view; a wrong `T` traps naming both sides |
| `Option<String>` | `?str` | return (answer lane) | `Some` mints the opt box; `None` is the flat nil |
| `Option<Vec<u8>>` | `?bytes` | return (answer lane) | as above |
| `Option<OpaqueRef>` / `Option<Opaque<T>>` | `?opaque` | return (answer lane) | as above |
| `Option<T>` (other `T`) | — | — | **no lane**: traps naming `?str`/`?bytes`/`?opaque` |
| `(A, B, …)` up to 8 | tuple | both | field-by-field under the record's own field types |
| `Value` | whatever's declared | return only | positional decode for generic tooling |

The optional read is **nil-flattening**: the null slot is `None`, a
some-slot decodes its payload as `T`. There is no separate optional
currency — nil *is* absence.

## Borrows and copies

`&str`/`&[u8]` params read the block store directly. The handler bound is
higher-ranked over the borrow's lifetime, so a borrowed param **cannot
outlive its call** — smuggling one out is a type error, not a runtime
check. Owned `String`/`Vec<u8>` params are the explicit "I keep this data"
copy.

Borrow guards, the whole rule:

- A borrow is **call-scoped**. The Rust lifetime prevents storing it past
  return; the object's guard flag additionally excludes conflicting access.
- While an exclusive borrow is held, a re-entrant `vm.call` that touches
  the same box traps `borrowed by an outer host call` instead of aliasing.
- Shared borrows stack; an active exclusive borrow excludes them.
- Guards clear on return. To keep data, the host copies.

## The two-channel law

A rut entry typed `-> (?T, err)` decodes positionally as
`(Option<T>, String)` — or generically as `Value::Tuple([Nil|payload,
Str(err)])`:

- A **returned err is data**: it crosses as the pair's second component;
  the host reads it and acts. The pair's convention belongs to the caller:
  empty err + a value = success; empty err + nil = "not found"; non-empty
  err = "failed".
- A **panic is drift**: it stays on the loud channel
  (`Result<_, Trap>` — `Err` means a trapped turn, a bug) and never fills
  an err field. The two channels are never merged.

## `opaque` — the one cell the host holds

A host payload and a host-held rut value share the one erasure box. The
store entry is one of two kinds:

| entry | holds | identity |
|---|---|---|
| `Host` | `Box<dyn Any>` + a type name + an optional finalize hook | `TypeId` guards the payload view |
| `Rut` | a rut cell slot | identity *is* the cell; rut-side `downcast` reads the cell's runtime kind |

The rut-side value addresses the entry directly (a tagged slot word); the
Rust-side typed view is:

| API | Meaning |
|---|---|
| `Opaque::alloc(vm, val)` | mint a box; `size_of::<T>()` charged to the heap budget |
| `Opaque::alloc_hosted(vm, val)` | mint a box whose payload opted into the release hook (`T: HostPayload`); `finalize` runs at entry death, before `Drop` |
| `Opaque::from_handle(&h)` | the checked view; a wrong payload `T` traps naming both sides, a rut-value box names the rut-side recovery |
| `o.with(\|v\| …)` | shared borrow of the payload; nested `with`s stack |
| `o.with_mut(vm, \|vm, v\| …)` | exclusive borrow; flows `&mut Vm` for in-crossing rc work |
| `o.handle()` | the erased `OpaqueRef`, for passing the box back across |

Death is deterministic: at rc-0 inside the release walk, `finalize(heap)`
runs first, then the payload's own `Drop` ([the Rc heap](rc-heap.md)).
Instances are per-map/per-logger, never per-op.

## Calling in

```rust
// entry fn half(x: f64) -> f64
let y: f64 = vm.call("half", (2.0,))?;

// entry fn find(id: i64) -> (?str, err) — the two-channel shape
let (name, err): (Option<String>, String) = vm.call("find", (7i64,))?;
if !err.is_empty() { /* "failed" */ }

// an opaque round trip
let b = rut_vm::Opaque::alloc(&mut vm, MyHandle::new())?;
let back: rut_vm::OpaqueRef = vm.call("stash", (b,))?;
```

- `vm.call::<A, R>(export, args)` converts the args under the export's
  declared parameter types, runs, and decodes the answer under the
  declared return. A shape mismatch traps naming both sides.
- `vm.resume::<R>()` continues a budget-parked call.
- Argument bundles are tuples up to arity 8 (`CallArgs`); each element
  must be a `CallArg` (`Copy` prims, `String`, `&str`, `Vec<u8>`, `&[u8]`,
  `OpaqueRef`, `Opaque<T>`).

## Cross-links

- Declaring and binding the surface: [host fns and declaration files](host-fns.md).
- Wrapping host state in rut classes: [native containers API surface](native-containers.md).
- Async bodies and the `Completer`: [the host futures bridge](host-futures.md).
- Budgets and traps: [resource limits](resource-limits.md).
