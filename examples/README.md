# rut examples — the design corpus

rut has no compiler yet. These files define the target syntax: the RFCs
reference them instead of inlining code. Host examples span the
**declaration/implementation split** (RFC 0025, RFC 0029): native surfaces
are rut-source declaration files (`host/plugin/my_map.d.rut` — `host` is
a `.d.rut`-only keyword), Rust bodies bind against them
(`host/my_map.rs`), and consumers just import (`host/my-map.rut`,
`host/interop.rut`); other VM/Rust sketches stay in the RFCs.

| File | Demonstrates | RFC |
|---|---|---|
| `basic/grammar-tour.rut` | dataclass, interface, class factory, `Rc<T>`, vtable dispatch, exhaustive `when`, `f""` | 0009–0012 |
| `basic/module-structure.rut` | declarations-only modules, const-expressions, no load-time code | 0003 §1 |
| `basic/module-visibility.rut` | `export` / `export(mod)` / `export(super)` / `export(self)` | 0003 §2 |
| `basic/when.rut` | `when` pattern expressions, exhaustiveness | 0008 |
| `basic/option-result.rut` | builtin `Option`/`Result`, `.value`, `unwrap_or`, `?` | 0005 |
| `basic/literals.rut` | numeric suffixes, plain/raw/format strings, constructors | 0007 |
| `basic/dataclasses.rut` | value semantics, field initializers, free functions | 0009 |
| `basic/classes.rut` | factory type-calls, `Self {}` literal, `Option<Self>` try-factories, private, statics | 0010 |
| `basic/rc-and-dispose.rut` | `Rc(v)` boxing, ref-copy aliasing, `dispose()` | 0011 |
| `basic/interfaces.rut` | methods-only interfaces, `requires`, dataclass implementors, composition over intersections | 0009, 0012 |
| `basic/type-tests.rut` | `is<T>()`, `upcast<T>()`; no `as`, no downcast | 0012 §3 |
| `basic/closures-generics.rut` | arrows, monomorphized generics | 0013 |
| `basic/opaque.rut` | `Opaque(v)` / `downcast<T>` / `is<T>` erasure & recovery; snapshot vs shared | 0014 |
| `basic/layout.rut` | repr C layouts, `type_id<T>()` / `size_of<T>()` / `align_of<T>()`, `Opaque` layout accessors | 0015 |
| `concurrency/countdown.rut` | cold futures, `await` as sole suspension | 0018 |
| `concurrency/fetch-page.rut` | `await` + `?` composition, state splitting | 0018 §3 |
| `concurrency/spawn-cancel.rut` | tasks, cancellation-by-drop | 0019 |
| `concurrency/select.rut` | `select` races, `as` binding, `select_all` | 0019 |
| `workers/image-pipeline.rut` | isolate workers, channels, transferable endpoints | 0021 |
| `workers/image-worker.rut` | worker entry point args, channel-driven shutdown | 0021 |
| `memory/temp-file.rut` | deterministic destruction at rc 0 | 0016 §3 |
| `memory/weak-cache.rut` | `Weak(v)`/`upgrade()` | 0017 §1 |
| `memory/node-cycle.rut` | reference cycles and the collector | 0017 §2 |
| `algorithms/sieve.rut` | unboxed `bytes`/`Array<i32>` | — |
| `algorithms/quicksort.rut` | in-place array mutation, recursion | — |
| `algorithms/matrix-mul.rut` | flat `Array<f32>` hot loops | — |
| `network/http-fetch.rut` | async client, `Result` at API boundaries | 0018 |
| `network/echo-server.rut` | accept loop + worker pool dispatch | 0021 |
| `network/echo-worker.rut` | per-connection serving in an isolate | 0021 |
| `host/interop.rut` | host classes via declaration files, repr C struct passing, buffer borrows, `Template` for l10n | 0022–0028 |
| `host/plugin/my_map.d.rut` | **declaration file** for `plugin:my_map`: `export host class MyMap<K: Hashable, V>`, slot table, admission-only param bounds | 0025, 0029 |
| `host/my-map.rut` + `host/my_map.rs` | the consumer + Rust **implementation** of the same declaration: erased `RutValue`/`IfaceHandle` storage, reified instantiations, `.implement` binding checked at link, dataclass key, `Opaque` values, native `Option`/`Array` returns | 0026 |
| `gui/dashboard/reactive.rut` | tur's `state`/`source`/`derive`/`mutation`/`watch`/`Store` in **user** rut, on `Opaque` | 0014 |
| `gui/dashboard/main.rut` | end-to-end app: declare graph, watch→render, bootstrap sources, live loop + worker | 0021 |

### gui/dashboard — a multi-file project

A tur-style web-app-shaped project (models / theme / reactive library /
setup / services / worker isolate / components / entry). `reactive.rut`
implements tur's `state` / `source` / `derive` / `mutation` / `watch` /
`Store` entirely in user rut on top of `Opaque` — proof that reactivity is
a library, not a language feature (RFC 0014). `state.rut` is the
tur-style *setup*: a `Dashboard` class whose factory declares **state**
atoms (UI writes), **sources** (services push snapshots as data arrives),
and **derives** (computed views); components *dispatch mutations* rather
than call setters, and `main.rut` subscribes re-render with `watch`. The
instance holds only inert handles — all values live in the store by atom
id — so sharing it behind `Rc` is trivially safe.
