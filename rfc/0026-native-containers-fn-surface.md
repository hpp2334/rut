# RFC 0026: Native Containers — a User-Defined Map (fn surface)

- **Status:** Draft (revised — **supersedes the generic `host class`
  design**; see RFC 0025's decision record)
- **Date:** 2026-09-13
- **Author:** hpp2334
- **Depends on:** RFC 0025 (revised — host fns & declaration files),
  RFC 0023 §1 (the crossing rule), RFC 0014 (`Opaque`)
- **Supersedes:** RFC 0005 §5.1 (pre-restructure), and this RFC's own
  earlier draft ("Generic Host Classes")
- **Part:** E — Host & FFI

## Summary

The two halves of a native module (full listing:
**`examples/host/plugin/my_map.d.rut`** — the declaration;
**`examples/host/my_map.rs`** — the implementation;
**`examples/host/my-map.rut`** — a consumer). No ClassTable, no
`GenericArgs`, no slot-per-instantiation: **host fns with concrete
signatures over `Opaque` handles**, wrapped in a rut class on the
consumer side.

**Declaration — rut source** (embedder-authored, ships with the plugin):

```rut
// plugin/my_map.d.rut — the declaration file for "plugin:my_map"
pub host fn my_map_new(cap: i32) -> Opaque;
pub host fn my_map_set(m: Opaque, k: str, v: Opaque) -> unit;
pub host fn my_map_get(m: Opaque, k: str) -> Option<Opaque>;
pub host fn my_map_size(m: Opaque) -> i32;
```

**Implementation — Rust, bodies only** (no surface data declared in Rust
at all):

```rust
pub struct MyMap {                              // NOT generic — values
    inner: HashMap<String, RutValue>,           // stay boxed Opaques
}

pub fn my_map_module() -> NativeModule {
    NativeModule::new("plugin:my_map")
        .fn_("my_map_new",  |_ctx, cap: i32| Ok(OpaqueBox::new(MyMap::with_capacity(cap))))
        .fn_("my_map_set",  |ctx, m: &OpaqueBox, k: String, v: RutValue| {
            m.get_mut::<MyMap>(ctx)?.inner.insert(k, v);  // call-scoped
            Ok(())                                         // borrow
        })
        .fn_("my_map_get",  |ctx, m: &OpaqueBox, k: String| {
            Ok(m.get::<MyMap>(ctx)?.inner.get(&k).cloned()) // Option is
        })                                                  // built natively
        .fn_("my_map_size", |ctx, m: &OpaqueBox| {
            Ok(m.get::<MyMap>(ctx)?.inner.len() as i32)
        })
}
```

## 1. The connection contract — two checkpoints

| checkpoint | when | checks | errors to |
|---|---|---|---|
| **compile/verify** | `rutc check` / `vm.load` verify | every call vs the **host decl**: concrete types over the crossing set. Checking needs **no Rust at all**. | rut author |
| **link** | `vm.load` | every host fn **referenced** by rut code has a bound, signature-equal impl — the fn-table reflection equals the decl (a pure data compare, nothing runs). | loader / embedder |

A name bound that no decl declares is an embedder **startup** error
(typo guard). Drift between the halves never reaches a rut runtime.

## 2. What crosses

The crossing rule (RFC 0023 §1) is the whole story: primitives, `str`
(hashed host-side by **content** — a registered builtin impl), `bytes`,
`Option`/`Result` over those, and `Opaque`. Map **values** cross as
`Opaque` — RFC 0014's checked erasure: `set` stores the box unopened,
`get` hands it back, and the consumer's `downcast<T>` does the checking
(`None` on mismatch — never a trap). The instantiation machinery of the
old design (reified `GenericArgs`, erased `RutValue` storage checked per
call, `TraitHandle` fat-ref keys reaching back into the VM for
`hash()`/`eq()`) is gone with `host class`: nothing native ever
re-enters the VM through a vtable, and no native signature has a type
parameter.

## 2a. Implementation status

The payload half is shipped: `rut_vm::OpaqueBox<T>` — `alloc(vm, value)`
mints the box immediately (shallow `size_of::<T>()` accounted against the
heap budget, RFC 0040), `from_value` is the checked view over a crossing
`Value::Opaque` (a wrong payload type is a `Trap` naming both sides),
`with`/`with_mut` are call-scoped borrows guarded by a flag in the box
(RFC 0023 §2 — a nested exclusive borrow traps `borrowed by an outer
host call`), and `into_value` hands the plain handle to rut. Bodies bind
by name via `Vm::register_host_fn` against `FuncCode.host`
(`"<spec>::<name>"`), the `rt:log` logger being the in-tree consumer.
The typed `NativeModule` builder sketched above (a `Crossing` marshal
trait over `Value`) and the embedder-side link table are the remaining
piece.

The consumer wraps the fns in a class (RFC 0025 — the `Logger` pattern):
the wrapper owns the name, the methods, and any impl blocks; each method
is one host fn call.

## 3. Slots, not strings

The `.fn_("get", ..)` label exists for the register step only: compiling
the declaration file assigns slot ids (`my_map_new→0, my_map_set→1, …`);
consumer calls compile to `callnat { slot }` (RFC 0032 — fold/CSE-safe,
no string in IR, no lookup at dispatch); `register_module` resolves each
label against the slot table **once**, before any rut code runs — a typo
is a startup error, never a runtime one. IR still cannot inline into
native bodies (honest FFI limit); if a host wants an optimizable body,
it is rut source — the wrapper class is exactly that place.

## 4. Notes

- **Who satisfies hashing**: `str` keys hash by content (builtin,
  native); dataclass keys are the rut wrapper's business — the wrapper
  can expose its own `Hashable`-keyed map over `str`ified keys, or the
  host ships a dedicated surface. No `Hashable` contract crosses.
- Builtin types flow back natively — a fn may return `Option<V>` built
  host-side; rut cannot tell it wasn't written in rut. Containers
  (`Vec`, `Array`) do not cross; iterate via per-element fns or keep
  the index rut-side.
- Re-entrancy guard stays for the fns that need it: while a host fn
  holds a box's `&mut` borrow, the borrow flag is set and a re-entrant
  call that touches the same box traps `borrowed by host` (RFC 0023 §2).
- Memory (RFC 0016 §3): the instance lives in the `Opaque` box; at rc-0
  its Rust `Drop` runs, dropping the `HashMap` and rc-dec'ing every
  stored box. Deterministic, no collector involvement.

## Open questions

- OQ-1: a `host dataclass` key surface for maps keyed by structured
  records (flat data crosses both ways; hashing stays host-side
  content-based) — needed before `std:collection`'s `Map`/`Set` ship.
