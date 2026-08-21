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

use rut::{ClassTable, GenericArgs, NativeModule, Trap, TypeRegistry, VmCtx};
use rut::value::{Array, IfaceHandle, Rut};

/// The container. The CROSSING RULE (RFC 0005 §5.1): an *unconstrained*
/// generic param crosses as its own host shape (`V: Rut` — monomorphized
/// per concrete type: i32, f64, string, handles); a param *constrained to
/// an interface* crosses uniformly as `IfaceHandle` — RFC 5002 §2's fat
/// ref. So there is NO K generic here: every legal key arrives as an
/// `IfaceHandle`, and `IfaceHandle: Hash + Eq` is implemented ONCE by the
/// rut crate, by vtable dispatch into the key's own `hash()`/`eq()` —
/// user impls are rut code, builtin/struct-voucher impls are native
/// trampolines. Content hashing for `string` keys, identity for `Rc<T>`
/// keys: both fall out of which vtable the boundary attached, and
/// `HashMap` never knows the difference.
pub struct MyMap<V: Rut> {
    inner: HashMap<IfaceHandle, V>,
}

impl<V: Rut> MyMap<V> {
    fn with_capacity(cap: i32) -> Self {
        MyMap { inner: HashMap::with_capacity(cap.max(0) as usize) }
    }
}

/// Called ONCE per distinct `MyMap<K, V>` — the monomorphization point
/// (RFC 0002 §10). Generic over **V only**: K is not a host shape, it is
/// its constraint's shape (`IfaceHandle`).
///
/// The param CONSTRAINTS are declared in terms of the same registered
/// interfaces the rut side imports:
///
///     let k = generic_args.of("K");            // by NAME, not index
///     let i_hashable = types.interface_of("Hashable");
///     ClassTable::new::<MyMap<V>>(generic_args)
///         .constrain(&k, i_hashable)           // the whole admission rule
///
/// `Hashable requires Equal<Self>` (RFC 0002 §6) means the constraint
/// closes over BOTH: a K that hashes but cannot compare is unrepresentable.
/// Rejection is COMPILE (IR) time — the descriptor carries the constraint,
/// the front-end flags the instantiation line, and the load-time verifier
/// re-checks (RFC 0007 §8). Nothing ever escapes to a runtime trap.
fn build_my_map<V: Rut>(generic_args: &GenericArgs,
                        types: &TypeRegistry) -> Result<ClassTable, Trap> {
    let k = generic_args.of("K");                    // GenericParam —
    let i_hashable = types.interface_of("Hashable"); // name + resolved Ty

    let k_ty = k.ty();                // the instantiation's K — for keys()

    ClassTable::new::<MyMap<V>>(generic_args)
        .constrain(&k, i_hashable)    // K must implement Hashable (+ Equal<K>)
        // native factory -> `MyMap<K, V>(cap)` type-call on the rut side
        .factory(|ctx: &mut VmCtx, cap: i32| Ok(MyMap::with_capacity(cap)))
        .method("set", |ctx, this: &mut MyMap<V>, k: IfaceHandle, v: V| {
            this.inner.insert(k, v);        // dispatches k.hash()/eq() via
            Ok(())                          // the vtable; handle V (e.g.
        })                                  // Opaque) is RC-retained for
        .method("get", |ctx, this: &MyMap<V>, k: IfaceHandle| {   // its life
            Ok(this.inner.get(&k).cloned()) // -> rut's builtin Option<V>;
        })                                  // the crossing is free (§5.1)
        .method("size", |ctx, this: &MyMap<V>| Ok(this.inner.len() as i32))
        .method("keys", |ctx, this: &MyMap<V>| {
            // Array<K>, NOT Array<Hashable>: rut cannot consume interface
            // refs here (no interface downcast — RFC 0002 §6.1). The K Ty
            // captured above drives the element type; each key is unerased
            // back to its natural value — a `string` key yields
            // Array<string>, so `counts.get(k)` type-checks in the .rut.
            let mut arr = Array::with_ty(ctx, k_ty, this.inner.len() as u32)?;
            for (i, key) in this.inner.keys().enumerate() {
                arr.set(ctx, i as u32, key.unerase(ctx))?;   // fat ref ->
            }                                                // plain value
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

// Re-entrancy note: hash()/eq() may re-enter the VM (VmCtx). `set` holds
// `&mut this`, so the cell's borrow flag is set for the call — a key
// whose hash() tried `m.set(..)` again traps `borrowed by host`
// (RFC 0005 §3) instead of corrupting the table. No new machinery.
//
// Memory story (RFC 0004, §5): an instance lives in a RutOpaque cell as a
// boxed host value; at rc-0 its derived Drop drops the HashMap, which
// drops each IfaceHandle (RC-dec — a Canvas's Rust Drop runs right then)
// and each V. Deterministic, no collector involvement.
