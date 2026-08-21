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
use rut::value::{Array, Iface, IfaceHandle, Rut};

/// The container. `K` crosses as an INTERFACE REF — the `Hashable`
/// constraint the rut side sees maps to `K: Iface` here: a handle that
/// hashes and compares by **vtable dispatch into the value's own
/// `hash()` / `eq()`** (RFC 0002 §6). Those may be rut code — a
/// dataclass key's impls (RFC 0002 §5.1) call back into the VM.
/// `V` is the host-side shape of any rut value (`i32`, `f64`, `string`,
/// handles): the boundary converts (RFC 0005 §3) before any method runs.
pub struct MyMap<K: Iface, V: Rut> {
    inner: HashMap<K, V>,
    // K: Iface implements Hash + Eq by dispatch — `insert`/`get` run the
    // key's hash()/eq() through the interface vtable on every probe.
}

impl<K: Iface, V: Rut> MyMap<K, V> {
    fn with_capacity(cap: i32) -> Self {
        MyMap { inner: HashMap::with_capacity(cap.max(0) as usize) }
    }
}

/// Called ONCE per distinct `MyMap<K, V>` — the monomorphization point
/// (RFC 0002 §10). The param CONSTRAINTS are declared here, in terms of
/// the same registered interfaces the rut side imports:
///
///     let k = generic_args.of("K");            // by NAME, not index
///     let i_hashable = types.interface_of("Hashable");
///     ClassTable::new::<MyMap<K, V>>(generic_args)
///         .constrain(k, i_hashable)            // the whole admission rule
///
/// `Hashable requires Equal<Self>` (RFC 0002 §6) means the constraint
/// closes over BOTH: a K that hashes but cannot compare is unrepresentable.
/// Satisfaction: user classes & dataclasses via `implements`; builtins via
/// registered impls (string/numerics/enum content, Rc<T> identity);
/// registered structs via the register_struct content voucher.
///
/// Rejection is COMPILE (IR) time — the descriptor carries the constraint,
/// the front-end flags the instantiation line, and the load-time verifier
/// re-checks (RFC 0007 §8). Nothing ever escapes to a runtime trap.
fn build_my_map<K: Iface, V: Rut>(generic_args: &GenericArgs,
                                  types: &TypeRegistry) -> Result<ClassTable, Trap> {
    let k = generic_args.of("K");                    // param by NAME
    let i_hashable = types.interface_of("Hashable"); // shared TypeId

    ClassTable::new::<MyMap<K, V>>(generic_args)
        .constrain(k, i_hashable)    // K must implement Hashable (+ Equal<K>)
        // native factory -> `MyMap<K, V>(cap)` type-call on the rut side
        .factory(|ctx: &mut VmCtx, cap: i32| Ok(MyMap::with_capacity(cap)))
        .method("set", |ctx, this: &mut MyMap<K, V>, k: K, v: V| {
            this.inner.insert(k, v);        // dispatches k.hash()/eq();
            Ok(())                          // handle V (e.g. Opaque) is
        })                                  // RC-retained for the map's
        .method("get", |ctx, this: &MyMap<K, V>, k: K| {      // lifetime
            Ok(this.inner.get(&k).cloned()) // -> rut's builtin Option<V>,
        })                                  // the crossing is free (§5.1)
        .method("size", |ctx, this: &MyMap<K, V>| Ok(this.inner.len() as i32))
        .method("keys", |ctx: &mut VmCtx, this: &MyMap<K, V>| {
            let mut arr = Array::with_len::<K>(ctx, this.inner.len() as u32)?;
            for (i, k) in this.inner.keys().enumerate() {
                arr.set(ctx, i as u32, k.as_handle().clone())?;   // built
            }                                                     // host-side;
            Ok(arr)                                               // rut can't tell
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
// drops each K (an IfaceHandle: RC-dec) and each V — handles release (a
// Canvas's Rust Drop runs right then), plain values just go.
// Deterministic, no collector involvement.

// `IfaceHandle: Clone` above is the thin clone of the reference, not the
// referent — same rule as rut's own Rc copy.
