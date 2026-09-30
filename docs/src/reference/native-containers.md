# Native containers API surface

A "native container" is host state rut can hold: a Rust value boxed as an
`opaque` handle, with a set of **host fns** as its API and a **wrapper
class** on the rut side. No second class system, no per-instantiation
machinery: the declaration is host fns with concrete signatures over
`opaque` ([host fns and declaration files](host-fns.md)), and the wrapper
is ordinary rut source the compiler can optimize around.

## The pattern

Declaration — rut source, ships with the plugin:

```rut
// my_map.d.rut
pub host fn my_map_new(cap: i32) -> opaque;
pub host fn my_map_set(m: opaque, k: str, v: opaque) -> nil;
pub host fn my_map_get(m: opaque, k: str) -> ?opaque;
pub host fn my_map_size(m: opaque) -> i32;
```

Implementation — Rust bodies only; values stay sealed boxes:

```rust
use rut_vm::interp::{HostRegistry, Vm};
use rut_vm::{Opaque, OpaqueRef, Trap};
use std::collections::HashMap;

struct MyMap { inner: HashMap<String, OpaqueRef> }

pub fn my_map_module() -> HostRegistry {
    let mut hosts = HostRegistry::new();
    rut_vm::register!(hosts, "my_map::my_map_new", (i32,) -> OpaqueRef,
        |vm: &mut Vm, cap: i32| {
            Ok(Opaque::alloc(vm, MyMap::with_capacity(cap.max(0) as usize))?
                .handle().clone())
        });
    rut_vm::register!(hosts, "my_map::my_map_set", (OpaqueRef, &str, OpaqueRef) -> (),
        |vm: &mut Vm, m: OpaqueRef, k: &str, v: OpaqueRef| {
            let map = Opaque::<MyMap>::from_handle(&m)?;
            map.with_mut(vm, |_vm, map| {         // exclusive, call-scoped
                if let Some(old) = map.inner.insert(k.to_string(), v) {
                    drop(old);                    // releases the displaced box
                }
                Ok(())
            })
        });
    rut_vm::register!(hosts, "my_map::my_map_get", (OpaqueRef, &str) -> Option<OpaqueRef>,
        |vm: &mut Vm, m: OpaqueRef, k: &str| {
            let map = Opaque::<MyMap>::from_handle(&m)?;
            Ok(map.with(|map| map.inner.get(k).cloned())?) // Option<OpaqueRef>
        });
    rut_vm::register!(hosts, "my_map::my_map_size", (OpaqueRef,) -> i32,
        |_vm, m: OpaqueRef| {
            let map = Opaque::<MyMap>::from_handle(&m)?;
            Ok(map.with(|map| map.inner.len() as i32)?)
        });
    hosts
}
```

Consumer side — an ordinary rut class; every method is exactly one host
call:

```rut
use my_map::{ my_map_new, my_map_set, my_map_get, my_map_size };

pub class MyMap {
    h: opaque;
}

impl MyMap {
    pub fn new(cap: i32) -> Self { return Self { h: my_map_new(cap) }; }
    pub fn set(mut self, k: str, v: opaque) -> nil { my_map_set(self.h, k, v); }
    pub fn get(self, k: str) -> ?opaque { return my_map_get(self.h, k); }
    pub fn size(self) -> i32 { return my_map_size(self.h); }
}
```

Consumers recover values with `opaque.downcast<V>(box)` — `nil` on
mismatch, checked, never a trap ([opaque](opaque.md)).

## The connection contract — two checkpoints

| checkpoint | when | checks | errors to |
|---|---|---|---|
| **compile/verify** | compile / verify | every call vs the declared host rows: concrete types over the crossing set. Checking needs **no Rust**. | rut author |
| **link** | boot | every host fn referenced by rut code has a bound, signature-equal impl — a pure data compare ([host fns](host-fns.md)). | loader / embedder |

A name bound that no decl declares is an embedder startup error (typo
guard). Drift between the halves never reaches a rut runtime.

## What crosses

The crossing rule is the whole story ([value boundary](value-boundary.md)):
primitives, `str` (hashed host-side by **content**), `bytes`, tuples and
`?T` over those, and `opaque`. Map **values** cross as `opaque` — `set`
stores the box unopened, `get` hands it back. Nothing native re-enters the
VM through a vtable, and no native signature has a type parameter.

## The typed box view

| API | Meaning |
|---|---|
| `Opaque::alloc(vm, val)` | mint a box; shallow `size_of::<T>()` charged to the heap budget |
| `Opaque::alloc_hosted(vm, val)` | opt the payload into a `finalize(heap)` release hook |
| `Opaque::from_handle(&h)` | the checked view; a wrong payload `T` traps naming both sides |
| `o.with(\|v\| …)` | shared borrow; nested `with`s stack |
| `o.with_mut(vm, \|vm, v\| …)` | exclusive borrow; a re-entrant call touching the same box traps `borrowed by an outer host call` |
| `o.handle()` | the erased `OpaqueRef` to hand back to rut |

Guards live on the store entry and clear on return; to keep data, copy
([value boundary](value-boundary.md)).

## Notes

- **Hashing**: `str` keys hash by content — a registered builtin impl.
  Structured keys are the rut wrapper's business (stringify, or a wrapper
  defined strategy); no user hashing contract crosses.
- **Containers do not cross**: `Vec`/`[T]` stay inside the VM. Iterate via
  per-element fns, keep the index rut-side, or pack into `bytes`.
- **Builtin answers flow back natively**: a fn may return an optional
  built host-side (`?str`/`?bytes`/`?opaque` answer lanes); rut cannot
  tell it was not written in rut.
- **Memory**: the instance lives in the box; at rc-0 the payload's
  `finalize` hook runs first, then Rust `Drop` — dropping the container
  releases every stored handle deterministically, with no collector
  involvement ([the Rc heap](rc-heap.md)). An `OpaqueRef`'s own `Drop`
  releases its reference; dropping a stored handle is the release.

## In-tree consumers

- **the logger** — `ink_host`'s two host fns behind `ink`'s `Logger` wrapper
  class ([core and the swappable packages](stdlib.md)).
- **`nmapset`** — `HashMap`/`HashSet` over a native key table: every
  method is one host call, keys are the closed typed set, values are
  sealed boxes, and each key carries a stable host-side handle
  ([core and the swappable packages](stdlib.md)).
- **`json`**'s peer-gated container impls ride the same packages from the
  rut side ([core and the swappable packages](stdlib.md)).
