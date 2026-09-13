# rut examples — the design corpus

rut has no compiler yet. These files define the target syntax: the RFCs
reference them instead of inlining code. Host examples span the
**declaration/implementation split** (RFC 0025, RFC 0029): native surfaces
are rut-source declaration files (`host/plugin/my_map.d.rut` — `host` is
a `.d.rut`-only keyword), Rust bodies bind against them
(`host/my_map.rs`), and consumers just import (`host/my-map.rut`,
`host/interop.rut`); other VM/Rust sketches stay in the RFCs.

The exceptions are the **runnable** Rust projects —
[`00-todolist/`](00-todolist/), [`01-sort/`](01-sort/), and
[`02-digest/`](02-digest/) — from the
first line of their `.rut` to the last `Value` out of the VM. Everything
else on this page is parse-only.

| File | Demonstrates | RFC |
|---|---|---|
| `basic/grammar-tour.rut` | dataclass, trait, class + class-method construction, impl blocks, shared cells + `own`, vtable dispatch, exhaustive `when`, `f""` | 0009–0012, 0016 |
| `basic/module-structure.rut` | declarations-only modules, load-time expressions, no load-time code | 0003 §1 |
| `basic/module-visibility.rut` | `pub` / `pub(mod)` / `pub(super)` / `pub(self)` — items **and** class members | 0003 §2, 0010 §2 |
| `basic/when.rut` | `when` pattern expressions, exhaustiveness | 0008 |
| `basic/option-result.rut` | builtin `Option`/`Result`, `.value`, `unwrap_or`, `?` | 0005 |
| `basic/error-context.rut` | `here()` / `capture_stack_trace()` on error values, lazy `render()`, stripped-binary degradation | 0036 |
| `basic/literals.rut` | numeric suffixes, plain/raw/format strings, builtin allocation calls, fixed arrays `Array<T, N>`, `dyn Slice<T>` boxing, `Vec`/`Array` → `dyn Slice<T>` widening | 0005, 0007 |
| `basic/dataclasses.rut` | reference semantics (aliasing by default), `own` divergence, field initializers, free functions, identity `==` | 0009, 0011, 0016 |
| `basic/classes.rut` | class-method construction (`new`/`from`/`parse`), `Self {}` literal, `Option<Self>` try-construction, member `pub` + sealing, explicit `self` receivers | 0010 |
| `basic/rc-and-dispose.rut` | aliasing + `own(x)`, `Disposal.dispose` at rc 0 | 0011, 0016 |
| `basic/traits.rut` | methods-only traits, `impl Trait for Type` blocks, `dyn I` object types, `requires`, dataclass implementors (hand `hash`/`eq`), `is` capability probe, composition over intersections | 0009, 0012 |
| `basic/type-tests.rut` | the `is` keyword: exact-class tests + trait capability probes; no `as`, no upcast, no downcast; implicit widening to `dyn I` | 0012 §3 |
| `basic/closures-generics.rut` | arrows, monomorphized generics | 0013 |
| `basic/opaque.rut` | `Opaque.new(v)` / `downcast<T>` / `is` erasure & recovery; zero-copy boxes, identity | 0014 |
| `concurrency/countdown.rut` | cold futures, `await` as sole suspension | 0018 |
| `concurrency/fetch-page.rut` | `await` + `?` composition, state splitting | 0018 §3 |
| `concurrency/spawn-cancel.rut` | tasks, cancellation-by-drop | 0019 |
| `concurrency/select.rut` | `select` races, `as` binding, `select_all` | 0019 |
| `workers/image-pipeline.rut` | isolate workers, channels, transferable endpoints | 0021 |
| `workers/image-worker.rut` | worker entry point args, channel-driven shutdown | 0021 |
| `memory/temp-file.rut` | deterministic destruction at rc 0 | 0016 §3 |
| `memory/weak-cache.rut` | `Weak(v)`/`upgrade()` | 0017 §1 |
| `memory/node-cycle.rut` | reference cycles and the collector | 0017 §2 |
| `memory/tree.rut` | recursive dataclasses (`Option<Node>`), composite fields as handle slots | 0009 §"Representation" |
| `algorithms/sieve.rut` | flat `Vec<u8>`/`Vec<i32>` primitive buffers | — |
| `algorithms/quicksort.rut` | in-place vec mutation, recursion | — |
| `algorithms/matrix-mul.rut` | flat `Vec<f32>` hot loops | — |
| `network/http-fetch.rut` | async client, `Result` at API boundaries | 0018 |
| `network/echo-server.rut` | accept loop + worker pool dispatch | 0021 |
| `network/echo-worker.rut` | per-connection serving in an isolate | 0021 |
| `host/interop.rut` | host classes via declaration files, zero-copy buffer borrows, `Template` for l10n | 0022–0028 |
| `host/plugin/my_map.d.rut` | **declaration file** for `plugin:my_map`: `pub host class MyMap<K: Hashable, V>`, slot table, admission-only param bounds | 0025, 0029 |
| `host/my-map.rut` + `host/my_map.rs` | the consumer + Rust **implementation** of the same declaration: erased `RutValue`/`TraitHandle` storage, reified instantiations, `.implement` binding checked at link, dataclass key, `Opaque` values, native `Option`/`Vec` returns | 0026 |
| `host/plugin/batch.d.rut` | **declaration file** for `plugin:batch`: `pub host class Batch` + `pub host fn submit(b: Batch) -> string` — a host callback whose parameter type **is** the host class | 0022, 0025, 0029 |
| `host/batch.rut` + `host/batch.rs` | the **round trip**: host constructs a `Batch`, rut filters/aggregates/pushes, then passes the instance BACK via the `submit` callback — `Handle<Batch>` call-scoped borrow in, receipt `string` out; deterministic Drop at rc 0 | 0022–0026, 0023 |
| `gui/dashboard/reactive.rut` | tur's `state`/`source`/`derive`/`mutation`/`watch`/`Store` in **user** rut, on `Opaque` | 0014 |
| `gui/dashboard/main.rut` | end-to-end app: declare graph, watch→render, bootstrap sources, live loop + worker | 0021 |

### 00-todolist — the runnable one

A Cargo project, a workspace member: **`cargo run -p todolist`**. The rut
side (`todolist.rut`) is a todo-list *library* — `pub class TodoList`
with full CRUD plus an **`entry fn`** surface, no `main`. The Rust side
(`src/main.rs`) drives it: `createContainer()` → an `Opaque` handle,
`create(container)` → `u32` handles, then CRUD calls crossing with
primitives, `Option`/`Result` only (RFC 0023 §2 — enforced on entry
signatures at compile time, RFC 0035 §3). All data stays in rut.
`tests/session.rs` gates the session under `cargo test --workspace`. See
[00-todolist/README.md](00-todolist/README.md).

### 01-sort — the algorithmic one

Also a Cargo project, a workspace member: **`cargo run -p sort`**. The
rut side (`sort.rut`) is a sorting *library* — insertion / bubble /
selection (loop-shaped) plus quicksort and merge sort (recursion:
in-place partitioning vs. out-of-place merging) — behind one
**dispatcher entry**, `sort(c, algo: string)`, a `when` over
string-literal pattern arms (unknown names come back as `Result.err`,
not a trap). The host holds one `Opaque` bank; `Vec<i32>` never crosses
(RFC 0023 §2) — results return as `serialize(c)`, a JSON array string,
so one compare checks a whole run. `fill(c, n, seed)` sizes a
deterministic LCG input in one call (wrapping `Math.wrapping_mul`/`Math.wrapping_add`, RFC 0004 §3),
and `tests/session.rs` gates every algorithm on known inputs, edge
cases (empty / single / duplicates / sorted / reverse / negatives),
cross-algorithm agreement on 500 pseudo-random values, and trap
cleanliness. See [01-sort/README.md](01-sort/README.md).

### 02-digest — the byte-level one

A Cargo project and workspace member: **`cargo run -p digests`** (the
package is `digests`, not `digest`, to keep the dependency graph clear
of RustCrypto's `digest` umbrella crate). The rut side (`digest.rut`)
is a byte-level library over `bytes` (the immutable binary primitive,
RFC 0004) — which crosses the host
boundary directly, unlike 01-sort's `Vec<i32>` (RFC 0023 §2): hex and
base64 codecs, MD5 / SHA-1 / SHA-256 / SHA-512 behind the same
dispatcher-entry pattern, the hashmap hash keys CRC-32 / FNV-1a 32+64 /
djb2 / sdbm, and a JSON codec (a hand-rolled tagged union — RFC 0006
enums carry no data — with children boxed in `Opaque`, RFC 0014).
SHA-512 is the u64 showcase: 64-bit rotations lean on `>>` being a
logical shift for unsigned types and `Math.wrapping_shl` truncating to the operand
width. The host is the **oracle**: every result is checked against
RFC 1321 / FIPS 180-4 / RFC 4648 vectors and the `md-5` / `sha1` /
`sha2` / `base64` / `crc32fast` / `serde_json` crates — the repo's
first crates.io dependencies, test-only in spirit, imported by the
embedder so the demo prints the verdict per row. See
[02-digest/README.md](02-digest/README.md).

| `json/json.rut` | user-defined JSON on `std:reflect`: the engine module (`JsonEngine`), `Serializable` contract, `stringify(v: dyn Serializable)`, `deserialize<T> … where T requires Deserializable`, structural sum policy, manual recursive descent | 0037 |
| `json/app.rut` | opt-in dataclasses (zero-method `impl Serializable for T {}`), initializer defaults, `Vec`/fixed `Array<T, N>` fields, manual curated class view (positional, module-private field unexposed), wire dataclass renames by hand, round-trip asserts | 0037 |

### gui/dashboard — a multi-file project

A tur-style web-app-shaped project (models / theme / reactive library /
setup / services / worker isolate / components / entry). `reactive.rut`
implements tur's `state` / `source` / `derive` / `mutation` / `watch` /
`Store` entirely in user rut on top of `Opaque` — proof that reactivity is
a library, not a language feature (RFC 0014). `state.rut` is the
tur-style *setup*: a `Dashboard` class whose construction method declares **state**
atoms (UI writes), **sources** (services push snapshots as data arrives),
and **derives** (computed views); components *dispatch mutations* rather
than call setters, and `main.rut` subscribes re-render with `watch`. The
instance holds only inert handles — all values live in the store by atom
id — so sharing it is trivially safe.
