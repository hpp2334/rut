// my_map.rs — the Rust side of `examples/host/my-map.rut`.
//
// The whole plugin, start to finish: a plain Rust struct, a method table,
// one `register_module` call in embedder startup. rut cannot see any of
// this — the map could equally wrap a C++ arena or a GPU buffer; only the
// surface below crosses.
//
// (rut has no VM yet — like the .rut corpus, this file defines the target
// embedding API: what writing a plugin should FEEL like from Rust.)

use std::collections::HashMap;

use rut::{ClassTable, InstBuilder, NativeModule, Trap, VmCtx};
use rut::value::{Array, Key, Rut};

/// The container. `V` is the host-side shape of the rut value: `i32`,
/// `f64`, `string`, or a handle type (`Opaque`, `Rc`, host classes) —
/// the boundary converts (RFC 0005 §3) before any method runs.
pub struct MyMap<K: Key, V: Rut> {
    inner: HashMap<K, V>,
}

impl<K: Key, V: Rut> MyMap<K, V> {
    fn with_capacity(cap: i32) -> Self {
        MyMap { inner: HashMap::with_capacity(cap.max(0) as usize) }
    }
}

/// Called ONCE per distinct `MyMap<K, V>` — the monomorphization point
/// (RFC 0002 §10). `Key` is the host-hash contract (§5.1): primitives,
/// `string`, and registered repr-C structs implement it (and it implies
/// `Hash + Eq + Clone`). Any other K fails HERE — at first use, with a
/// trap the embedder wrote, not deep inside the VM.
fn build_my_map<K: Key, V: Rut>(inst: &mut InstBuilder) -> Result<ClassTable, Trap> {
    ClassTable::new::<MyMap<K, V>>(inst)
        // native factory -> `MyMap<K, V>(cap)` type-call on the rut side
        .factory(|ctx: &mut VmCtx, cap: i32| Ok(MyMap::with_capacity(cap)))
        .method("set", |ctx, this: &mut MyMap<K, V>, k: K, v: V| {
            this.inner.insert(k, v);        // handle V (e.g. Opaque) is
            Ok(())                          // RC-retained for the map's
        })                                  // lifetime
        .method("get", |ctx, this: &MyMap<K, V>, k: K| {
            Ok(this.inner.get(&k).cloned()) // -> rut's builtin Option<V>,
        })                                  // the crossing is free (§5.1)
        .method("size", |ctx, this: &MyMap<K, V>| Ok(this.inner.len() as i32))
        .method("keys", |ctx: &mut VmCtx, this: &MyMap<K, V>| {
            let mut arr = Array::with_len::<K>(ctx, this.inner.len() as u32)?;
            for (i, k) in this.inner.keys().enumerate() {
                arr.set(ctx, i as u32, k.clone())?;   // built host-side;
            }                                         // rut can't tell
            Ok(arr)
        })
        .build()
}

/// The module the embedder registers — one line in startup
/// (RFC 0005 §1):
///
///     vm.register_module("plugin:my_map", my_map_module())?;
pub fn my_map_module() -> NativeModule {
    NativeModule::new("plugin:my_map")
        .generic_class("MyMap", 2, build_my_map)
}

// Memory story (RFC 0004, §5): nothing special. An instance lives in a
// RutOpaque cell as a boxed host value; at rc-0 its derived Drop drops
// the HashMap, which drops each V — handles release (RC-dec; a Canvas's
// Rust Drop runs right then), plain values just go. Deterministic, no
// collector involvement.
