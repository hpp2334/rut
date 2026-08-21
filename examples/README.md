# rut examples — the design corpus

rut has no compiler yet. These files define the target syntax: the RFCs
reference them instead of inlining code. Host examples span the
**declaration/implementation split** (RFC 0005 §5): native surfaces are
rut-source decl modules (`host/plugin/my_map.rut`), Rust bodies bind
against them (`host/my_map.rs`), and consumers just import
(`host/my-map.rut`, `host/interop.rut`); other VM/Rust sketches stay in
the RFCs.

| File | Demonstrates | RFC |
|---|---|---|
| `basic/grammar-tour.rut` | dataclass, interface, class factory, `Rc<T>`, vtable dispatch, exhaustive `when`, `f""` | 0002 §1 |
| `basic/module-structure.rut` | declarations-only modules, const-expressions, no load-time code | 0002 §1.1 |
| `basic/module-visibility.rut` | `export` / `export(mod)` / `export(super)` / `export(self)` | 0002 §1.1 |
| `basic/when.rut` | `when` pattern expressions, exhaustiveness | 0002 §1.2 |
| `basic/option-result.rut` | builtin `Option`/`Result`, `.value`, `unwrap_or`, `?` | 0002 §3 |
| `basic/literals.rut` | numeric suffixes, plain/raw/format strings, constructors | 0002 §4, §4.1 |
| `basic/dataclasses.rut` | value semantics, field initializers, free functions | 0002 §5.1 |
| `basic/classes.rut` | factory type-calls, `Self {}` literal, `Option<Self>` try-factories, private, statics | 0002 §5.2 |
| `basic/rc-and-dispose.rut` | `Rc(v)` boxing, ref-copy aliasing, `dispose()` | 0002 §5.3 |
| `basic/interfaces.rut` | methods-only interfaces, `requires`, dataclass implementors, composition over intersections | 0002 §5.1, §6 |
| `basic/type-tests.rut` | `is<T>()`, `upcast<T>()`; no `as`, no downcast | 0002 §6.1 |
| `basic/closures-generics.rut` | arrows, monomorphized generics | 0002 §9 |
| `basic/opaque.rut` | `Opaque(v)` / `downcast<T>` / `is<T>` erasure & recovery; snapshot vs shared | 0002 §3.1 |
| `basic/layout.rut` | repr C layouts, `type_id<T>()` / `size_of<T>()` / `align_of<T>()`, `Opaque` layout accessors | 0002 §3.2, §10.2 |
| `concurrency/countdown.rut` | cold futures, `await` as sole suspension | 0003 §2 |
| `concurrency/fetch-page.rut` | `await` + `?` composition, state splitting | 0003 §2.1 |
| `concurrency/spawn-cancel.rut` | tasks, cancellation-by-drop | 0003 §3 |
| `concurrency/select.rut` | `select` races, `as` binding, `select_all` | 0003 §3 |
| `workers/image-pipeline.rut` | isolate workers, channels, transferable endpoints | 0003 §5 |
| `workers/image-worker.rut` | worker entry point args, channel-driven shutdown | 0003 §5 |
| `memory/temp-file.rut` | deterministic destruction at rc 0 | 0004 §3 |
| `memory/weak-cache.rut` | `Weak(v)`/`upgrade()` | 0004 §4 |
| `memory/node-cycle.rut` | reference cycles and the collector | 0004 §5 |
| `algorithms/sieve.rut` | unboxed `bytes`/`Array<i32>` | — |
| `algorithms/quicksort.rut` | in-place array mutation, recursion | — |
| `algorithms/matrix-mul.rut` | flat `Array<f32>` hot loops | — |
| `network/http-fetch.rut` | async client, `Result` at API boundaries | 0003 §2 |
| `network/echo-server.rut` | accept loop + worker pool dispatch | 0003 §5 |
| `network/echo-worker.rut` | per-connection serving in an isolate | 0003 §5 |
| `host/interop.rut` | extern classes via decl modules, repr C struct passing, buffer borrows, `Template` for l10n | 0005 |
| `host/plugin/my_map.rut` | **decl module** for `plugin:my_map`: `export extern class MyMap<K: Hashable, V>`, slot table, admission-only param bounds | 0005 §5 |
| `host/my-map.rut` + `host/my_map.rs` | the consumer + Rust **implementation** of the same decl: erased `RutValue`/`IfaceHandle` storage, reified instantiations, `.implement` binding checked at link, dataclass key, `Opaque` values, native `Option`/`Array` returns | 0005 §5.1 |
| `gui/dashboard/reactive.rut` | tur's `state`/`source`/`derive`/`mutation`/`watch`/`Store` in **user** rut, on `Opaque` | 0002 §3.1 |
| `gui/dashboard/main.rut` | end-to-end app: declare graph, watch→render, bootstrap sources, live loop + worker | 0003 §5 |

### gui/dashboard — a multi-file project

A tur-style web-app-shaped project (models / theme / reactive library /
setup / services / worker isolate / components / entry). `reactive.rut`
implements tur's `state` / `source` / `derive` / `mutation` / `watch` /
`Store` entirely in user rut on top of `Opaque` — proof that reactivity is
a library, not a language feature (RFC 0002 §3.1). `state.rut` is the
tur-style *setup*: a `Dashboard` class whose factory declares **state**
atoms (UI writes), **sources** (services push snapshots as data arrives),
and **derives** (computed views); components *dispatch mutations* rather
than call setters, and `main.rut` subscribes re-render with `watch`. The
instance holds only inert handles — all values live in the store by atom
id — so sharing it behind `Rc` is trivially safe.
