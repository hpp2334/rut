// my_map.rs — the IMPLEMENTATION half of "plugin:my_map".
//
// The DECLARATION half is rut source: examples/host/plugin/my_map.d.rut
// (`pub host fn my_map_new(cap: i32) -> Opaque;` and friends — .d.rut
// files are the only place `host` may appear, RFC 0029). This file
// declares NO surface — it registers bodies, bound by NAME at embedder
// startup, and proven equal to the declaration at vm.load link:
//
//   checkpoint       when               what is checked            errors to
//   ---------------  -----------------  -------------------------  ---------
//   compile/verify   rutc check, load   every call vs the DECL     rut author
//                                       (types are concrete —     (rut line;
//                                       no Rust involved)         no Rust
//                                                                  linked)
//   link             vm.load            fn-table reflection ==    loader /
//                                       the declaration (pure     embedder
//                                       data compare, nothing
//                                       runs)
//
// Result: rut-side checking is decidable from declarations alone; drift
// between the halves never reaches a rut runtime.
//
// There is no `host class MyMap` and no ClassTable/GenericArgs anymore:
// every member is a host fn with a CONCRETE signature over the crossing
// set (RFC 0023 §1). The instance is an Opaque box holding this struct
// (RFC 0016 §5); keys are `str` (hashed host-side by content — a
// registered builtin impl); VALUES cross as `Opaque` — RFC 0014's
// checked erasure is the value story, `downcast<T>` recovers them on
// the rut side, and the Rust side stores and compares the boxes without
// ever inspecting them.
//
// (Module loading is M2 — this file defines the target embedding API:
// what a fn-only plugin should FEEL like from Rust.)

use std::collections::HashMap;

use rut::{NativeModule, OpaqueBox, RutValue, Trap, VmCtx};

/// The container — NOT generic. Host fn signatures are concrete, so
/// there are no reified type args to feed a builder (the old RFC 0026
/// machinery): values arrive as Opaque boxes and stay that way.
pub struct MyMap {
    inner: HashMap<String, RutValue>,   // content-hashed str keys,
}                                      // boxed values (opaque to us)

/// `pub host fn my_map_new(cap: i32) -> Opaque;`
fn my_map_new(_ctx: &mut VmCtx, cap: i32) -> Result<OpaqueBox<MyMap>, Trap> {
    Ok(OpaqueBox::new(MyMap {
        inner: HashMap::with_capacity(cap.max(0) as usize),
    }))
}

/// `pub host fn my_map_set(m: Opaque, k: str, v: Opaque) -> unit;`
///
/// `m` is the box's call-scoped borrow (RFC 0023 §2): `&mut this` for
/// the call's duration, borrow-flagged, cleared on return.
fn my_map_set(ctx: &mut VmCtx, m: &OpaqueBox, k: String, v: RutValue) -> Result<(), Trap> {
    let this: &mut MyMap = m.get_mut(ctx)?;
    this.inner.insert(k, v);               // rc-retained for the map's
    Ok(())                                  // lifetime
}

/// `pub host fn my_map_get(m: Opaque, k: str) -> Option<Opaque>;` —
/// miss => None; hit hands the stored box back (the wrapper's
/// `downcast<T>` does the checking, on the rut side — RFC 0014).
fn my_map_get(ctx: &mut VmCtx, m: &OpaqueBox, k: String) -> Result<Option<RutValue>, Trap> {
    let this: &MyMap = m.get(ctx)?;
    Ok(this.inner.get(&k).cloned())        // Option built host-side;
}                                          // the sum crosses natively

/// `pub host fn my_map_size(m: Opaque) -> i32;`
fn my_map_size(ctx: &mut VmCtx, m: &OpaqueBox) -> Result<i32, Trap> {
    Ok(m.get::<MyMap>(ctx)?.inner.len() as i32)
}

pub fn my_map_module() -> NativeModule {
    NativeModule::new("plugin:my_map")
        .fn_("my_map_new",  my_map_new)   // binds BY DECL NAME — a typo
        .fn_("my_map_set",  my_map_set)   // is a startup error, never a
        .fn_("my_map_get",  my_map_get)   // runtime one (RFC 0026 §4)
        .fn_("my_map_size", my_map_size)
}

// Embedder startup (RFC 0022 §1):
//
//     let mut vm = Vm::new(HostHooks { .. });
//     vm.register_module("plugin:my_map", my_map_module())?;  // bodies only
//     vm.load("app")?;   // verify vs declaration files; link impl == decl
//
// Memory (RFC 0016 §3): the instance lives in an Opaque cell as a boxed
// host value; at rc-0 its derived Drop drops the HashMap, which drops
// each stored RutValue (rc-decs — a Canvas's Rust Drop runs right then).
// Deterministic, no collector involvement.
//
// What went with `host class` (the honest trade, mirrored in the
// consumer's comments): admission bounds (`MyMap<Canvas, ..>` was once
// a rut-line error — any box fits now), unerased `keys() -> Vec<K>`
// (containers don't cross; iterate via get, or ship a rut-side index),
// and the `Hashable` vtable re-entry for dataclass keys (str content
// hashing is the native path).
