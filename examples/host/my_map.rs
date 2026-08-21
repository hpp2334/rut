// my_map.rs — the IMPLEMENTATION half of "plugin:my_map".
//
// The DECLARATION half is rut source: examples/host/plugin/my_map.rut
// (`export extern class MyMap<K: Hashable, V> { .. }`). This file
// declares NO surface — it registers bodies, bound by NAME at embedder
// startup, and proven equal to the declaration at vm.load link:
//
//   checkpoint       when               what is checked            errors to
//   ---------------  -----------------  -------------------------  ---------
//   compile/verify   rutc check, load   instantiation vs the DECL  rut author
//                                       (MyMap<Canvas,..> is a    (rut line;
//                                       rut-line error; no Rust   no Rust
//                                       involved)                 linked)
//   link             vm.load            ClassTable reflection ==  loader /
//                                       extern decl (names +      embedder
//                                       shapes; pure data
//                                       compare, nothing runs)
//
// Result: rut-side checking is decidable from decls alone; drift between
// the halves never reaches a rut runtime.
//
// (rut has no VM yet — like the .rut corpus, this file defines the target
// embedding API: what writing a plugin should FEEL like from Rust.)

use std::collections::HashMap;

use rut::{ClassTable, GenericArgs, NativeModule, Trap, TypeRegistry, VmCtx};
use rut::value::{Array, IfaceHandle, RutValue};

/// The container — **not generic**. A Rust generic would need V named at
/// RUST compile time, but the builder below runs at RUT runtime, once
/// per instantiation; nobody can supply V. Instead (RFC 0005 §5.1):
///
///   * rut side: each MyMap<K, V> instantiation is REIFIED — GenericArgs
///     carries K's and V's TypeIds (RFC 0002 §10).
///   * rust side: storage is ERASED — `RutValue` (owning handle;
///     `Value<'v>` is its call-scoped borrow, §3), checked per call
///     against the reified V.
///
/// K needs no erasure: the decl's `K: Hashable` bound means K crosses
/// uniformly as `IfaceHandle` (RFC 5002 §2's fat ref) — `IfaceHandle:
/// Hash + Eq` is implemented once by the rut crate, dispatching into the
/// key's own hash()/eq() through the vtable. User impls are rut code;
/// builtin/voucher impls are native trampolines. Content hashing for
/// `string` keys, identity for `Rc<T>` keys — both fall out of which
/// vtable the boundary attached; this HashMap never knows the difference.
pub struct MyMap {
    inner: HashMap<IfaceHandle, RutValue>,
}

impl MyMap {
    fn with_capacity(cap: i32) -> Self {
        MyMap { inner: HashMap::with_capacity(cap.max(0) as usize) }
    }
}

/// Instantiation builder — called ONCE per distinct `MyMap<K, V>` at
/// first use (the monomorphization point, RFC 0002 §10). The admission
/// the DECL states (`K: Hashable`) was already checked at compile/verify
/// against the rut instantiation — this side needs only the reified
/// types to drive per-call checks and host-built values.
fn build_my_map(args: &GenericArgs, _types: &TypeRegistry) -> Result<ClassTable, Trap> {
    let v_ty = args.of("V").ty();                 // reified V: drives the
                                                  // per-call check in set
    let k_ty = args.of("K").ty();                 // for keys(): Array<K>

    ClassTable::new::<MyMap>(args)
        // binds slot 0 (the decl's factory -> 0)
        .factory(|ctx: &mut VmCtx, cap: i32| Ok(MyMap::with_capacity(cap)))
        // slot 1. "set" is a BINDING-TIME label — resolved against the
        // decl's slot table here, once, at startup. Dispatch is by slot.
        .method("set", |ctx, this: &mut MyMap, k: IfaceHandle, v: RutValue| {
            ctx.check_arg(&v, v_ty)?;             // value's TypeId == this
            this.inner.insert(k, v);              // instantiation's V (link
            Ok(())                                // already proved decl
        })                                        // match — this is the
        .method("get", |ctx, this: &MyMap, k: IfaceHandle| {   // runtime
            Ok(this.inner.get(&k).cloned())       // mirror). v is
        })                                        // RC-retained for the
        .method("size", |ctx, this: &MyMap|       // map's lifetime
            Ok(this.inner.len() as i32))
        // slot 4. Array<K>, NOT Array<Hashable>: k_ty drives the element
        // type and each key is UNERASED to its natural value (a `string`
        // key yields Array<string>) — rut cannot consume interface refs
        // here (no downcast, RFC 0002 §6.1). `get` needs no per-call V
        // check: values only enter via set, and the cell carries this
        // instantiation's TypeId.
        .method("keys", |ctx, this: &MyMap| {
            let mut arr = Array::with_ty(ctx, k_ty, this.inner.len() as u32)?;
            for (i, key) in this.inner.keys().enumerate() {
                arr.set(ctx, i as u32, key.unerase(ctx))?;  // fat ref -> value
            }
            Ok(arr)
        })
        .build()
}

/// Runtime half of the contract. `.implement` binds BY DECL NAME; link
/// proves the ClassTable equals the extern decl (member names, shapes
/// under the crossing rule — a pure reflection compare; no builder
/// runs). A name absent from the decl is an embedder STARTUP error
/// (typo guard); a decl member with no impl fails at link only if some
/// rut module actually references it.
pub fn my_map_module() -> NativeModule {
    NativeModule::new("plugin:my_map")
        .implement("MyMap", build_my_map)
}

// Embedder startup (RFC 0005 §1):
//
//     let mut vm = Vm::new(HostHooks { .. });
//     vm.register_module("plugin:my_map", my_map_module())?;  // bodies only
//     vm.load("app")?;   // verify vs decl modules; link impl == decl
//
// Re-entrancy: hash()/eq() may re-enter the VM (VmCtx); `set` holds
// `&mut this`, so the cell's borrow flag traps a key whose hash() tried
// `m.set(..)` again — `borrowed by host` (§3) instead of corrupting the
// table. No new machinery.
//
// Memory (RFC 0004 §5): the instance lives in a RutOpaque cell as a
// boxed host value; at rc-0 its derived Drop drops the HashMap, which
// drops each IfaceHandle and RutValue (RC-decs — a Canvas's Rust Drop
// runs right then). Deterministic, no collector involvement.
