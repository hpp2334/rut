# Host fns and declaration files

One linkage keyword, one implementer:

| keyword | implementation lives in | bound at link against |
|---|---|---|
| `host fn` / `host struct` | the **embedding Rust** — a typed registration | the load-time contract (below) |
| `prelude builtin` / `pub builtin` rows | **the engine itself** — compiler-lowered | nothing; the decl is a pure signature contract |

`extern` does not exist: rut→rut names resolve through use paths
([modules and visibility](modules-and-visibility.md)); host→rut entry
points are `entry fn` ([loading and the embed loop](loading.md)).

Surfaces live in **declaration files** — `.d.rut` — so compiling rut code
never requires any Rust, and a published package can ship its compiled
bodies plus a hand-written surface.

## Host pkgs

A package directory whose manifest declares `type = "host"` is a **host
pkg** — pure surface, no rut source:

```toml
# server/rut.toml
name = "server"
type = "host"
entry.type = "./server.d.rut"
```

```rut
// server/server.d.rut
pub host fn subscribe(bus: opaque, topic: str, handler: str) -> nil;
pub host fn emit(bus: opaque, topic: str, payload: str) -> nil;
```

Consumers reach a host pkg two ways:

- declared in the manifest's `[deps]`
  ([project structure](project-structure.md), [dependency kinds](dependency-kinds.md));
  resolution is recursive, **first mount wins**;
- mounted programmatically: `rut_driver::mount_dir(&mut session, dir)`
  ([embedding and native modules](embedding.md)).

The loader lowers the declared signatures into the mounted surface at load
time; the compiler type-checks calls against them. A host pkg packs into a
`.rutbundle` like any dep ([module bundles](bundles.md)).

## The surface grammar

```rut
host fn name(params) -> T;            // concrete signature; generics are a
                                      // compile error — a generic parameter
                                      // has no shape the boundary checks
host struct Name { fields }           // flat record; every field a crossing
                                      // type; no methods, no field
                                      // initializers — the shape IS the
                                      // whole surface
prelude builtin fn name<T>(params) -> T;   // engine fn, compiler-lowered
prelude builtin class Name<T> { .. }       // engine type member contract
prelude builtin trait Name<T> { .. }       // engine-woven contract
prelude builtin impl i32 { .. }            // engine methods on a primitive
pub builtin trait Name<T> { .. }           // the import-gated twin (below)
```

Rules:

- **The crossing set** — `host fn` signatures are concrete over: nil, the
  primitives, `str`, `bytes`, `opaque`, tuples/`?T` whose elements cross,
  and `host struct` records whose fields all cross. Returns may
  additionally use the answer optionals `?str` / `?bytes` / `?opaque`.
  Everything else (user classes, `Vec<T>`, `[T]`, trait objects, closures)
  is a compile error on the declaration
  ([value boundary](value-boundary.md)).
- **`any` is not in the language.** It is a reserved word; a `.d.rut`
  spelling it is the reserved-word diagnostic at parse time. Seal
  polymorphic values with `opaque(v)` /
  `opaque.downcast<T>(v)` ([opaque](opaque.md)).
- **`builtin` is the engine's reservation** — spelled only in the
  toolchain's own decl files (`core`, `calc`). A `builtin` in an embedder
  decl is a compile error. Users implement builtin traits with ordinary
  `impl` blocks; library contracts stay plain `trait`
  ([traits and dispatch](traits.md)).
- **`builtin` is a contextual keyword**: `.d.rut`-only; elsewhere it is a
  legal identifier.

### The two builtin spellings

A `builtin` decl spells one of two strict forms — a bare `builtin`
diagnoses *"`builtin` must be spelled `prelude builtin` (ambient) or
`pub builtin` (import-gated)"*:

- **`prelude builtin`** — the **ambient** engine surface: the name
  binds in every compilation unit, no `use` needed. The primitives and
  their `builtin impl` methods, `opaque`, `Weak`,
  `StackTrace`, and `panic`/`string_join`/
  `capture_stacktrace` are all ambient.
- **`pub builtin`** — the **import-gated** engine surface: the name
  resolves only through `use core::{ .. }`, the way a package's names
  do. Today's rows are every builtin trait — `Iterator`, `Future`,
  `RunContext` — and the disposal pair, `Disposal` and
  `DisposalContext` ([traits](traits.md), [async and
  await](async.md), [the Rc heap](rc-heap.md)). The engine's weave
  never consults the gate — it keys on the native-trait symbols — so a
  module with no imports still iterates, awaits, and launches; only
  spelling a name in source gates.

Using a `pub builtin` name without the use line is a resolution miss
that names the fix, never a bare "unknown name":

```text
`Disposal` is not in scope — `use core::{ Disposal }`
```

The split is per name: a unit that imports one gated name still sees
every ambient name for free, and an unused name in a `use core` line
stays a lint, not an error.

- **`opaque` wraps native state; there is no host class.** rut wraps the
  handle in a class of its own — the wrapper-class pattern
  ([native containers API surface](native-containers.md)). Destructors
  still run deterministically: when a box's rc hits 0, the payload's
  finalize hook runs, then Rust `Drop` ([the Rc heap](rc-heap.md)).
- **Workers**: an `opaque` box crosses isolates only if the host
  registered the boxed type as `send` — checked at the transfer, by type
  ([workers and channels](workers-and-channels.md)).

## Declaration mode

A `.d.rut` is parsed in **declaration mode**: declarations only, each
complete as a surface.

| allowed | notes |
|---|---|
| `use` / `pub` | visibility exactly as in a module; non-exported decls are known inside the file, nameable nowhere else |
| `let` | with load-time constant initializers |
| `enum` | the member list is the whole definition |
| `trait` | method signatures (+ `requires`) are the whole definition |
| `struct` | fields only, with load-time initializers |
| `host fn` / `host struct` | signatures only, concrete over the crossing set |
| `prelude builtin` / `pub builtin` rows | toolchain decl files only — the two spellings above |

Forbidden — the parser errors *"implementation in a declaration file"*:
any fn/class body, statements. Symmetrically, a `.rut` file that spells
`host` errors *"belongs in a `.d.rut`"*.

## Signatures are derived, not hand-written

The registered callable's Rust shape **is** the `.d.rut` row:

```rust
let mut hosts = rut_vm::interp::HostRegistry::new();
hosts.register::<_, (&str,), OpaqueRef, _>(
    "server::make", |vm: &mut Vm, name: &str| vm.alloc_opaque_str(name.to_string()),
);
// two-plus params: spell the marker tuple once, the closure stays plain
rut_vm::register!(hosts, "server::emit", (OpaqueRef, &str, &str) -> (),
    |_vm, _bus, _topic, _payload| Ok(()));
```

- The signature is a fixed array of type ids derived from the closure's
  parameter and return types; a param type with no crossing impl is a
  **compile error at the register site**.
- Both body shapes fit: `-> R` (infallible) and `-> Result<R, Trap>`
  (fallible). Errors travel an explicit trap channel — a trapped body
  propagates as `Err(Trap)` with a rut backtrace
  ([embedding and native modules](embedding.md)).
- `&str` / `&[u8]` params are zero-copy borrows scoped to exactly the
  call ([value boundary](value-boundary.md)).

## The load-time contract

Mounting a host pkg **declares**; the embedding Rust **binds**. The two
sides are checked against each other before any rut code runs:

```rust
hosts.verify_against(&session.expected_host_fns());  // panics on mismatch
let mut vm = rut_vm::interp::Vm::new(prog, &limits, hooks, hosts)?;  // joins the rest
```

A mismatch is an embedder wiring bug — a **panic**, never a rut
diagnostic — on exactly three classes:

1. **declared but unbound** — a rut call would trap mid-run;
2. **bound but undeclared** — no surface declares what the host installed;
3. **signature drift** — the decl says `(opaque, str, str) -> nil`, the
   binding took `(opaque, i64, str)`.

The law follows the mount: **mount what you bind**. An embedder that
mounts `calc` must bind its float fns; an embedder that needs only `core`
mounts only `core`.

## Slots, not strings

Compiling a declaration file assigns every host fn a **stable slot id** in
declaration order; the module binary carries the slot table. Calls compile
to `callnat { slot }` — names are binding-time labels, resolved once at
boot, never dispatch keys, never in IR. A typo'd binding is a startup
error, never a runtime one. IR cannot inline into native bodies; if a body
should be optimizable, it is rut source — the wrapper class is exactly
that place.

## Async host fns

```rut
// http_host.d.rut — an async row expands into a five-row family
pub host async fn http_send(c: opaque, method: str, url: str,
                            headers: str, body: bytes) -> opaque;
```

The embedder binds the family with one registration; the closure spawns
the work, hands out a completer clone, and returns:

```rust
rut_vm::register_async!(hosts, "http_host::http_send",
    (OpaqueRef, &str, &str, &str, &[u8]) -> OpaqueRef,
    |c: OpaqueRef, method: &str, url: &str, headers: &str, body: &[u8]|
        -> rut_vm::Completer<OpaqueRef> {
        let done = rut_vm::Completer::new();
        let w = done.clone();
        std::thread::spawn(move || {
            match do_request(c, method, url, headers, body) {
                Ok(r)  => w.complete(r),
                Err(e) => w.fail(e.to_string()),
            }
        });
        done
    },
    /* optional abort hook: */ move |_c| { /* best-effort cancel */ },
);
```

`register_async!(hosts, name, (P…) -> R, start [, abort])` emits the
family the compiler's weave joins:

| emitted row | meaning |
|---|---|
| `<name>` | the decl row itself; calling it directly traps — the weave mints the future at the call site |
| `<name>__start(P…) -> opaque` | runs the closure, boxes the `Completer` as the future's state cell |
| `<name>__yield(state, cx) -> i32` | resumption probe: `0` pending, `1` ready, `2` failed |
| `<name>__take(state) -> R` | marshals the answer onto the VM thread; a failed completer traps here |
| `<name>__cancel(state)` | the abort hook when given; otherwise the disclosed no-op |

| `Completer<T>` API | Meaning |
|---|---|
| `Completer::new()` | a fresh cell; `clone()` shares it (one `Arc`) |
| `complete(v)` / `fail(msg)` | settle from **any thread** (atomics + mutex only) |
| `poll() -> i32` | the driving loop's probe |
| `take_result() -> Result<T, Trap>` | VM thread only; a `fail` message becomes the trap at the `await` |

Thread law: the VM stays single-threaded; workers touch only the
completer. Cancellation is best-effort — a worker runs to its blocking
completion and the late answer is discarded. See
[the host futures bridge](host-futures.md) and [async and await](async.md).

## Declaration files as artifacts

A published package ships:

| artifact | role |
|---|---|
| `mod.rutc` | the compiled module binary: bodies + the surface, the linking truth ([module binary and verification](module-binary.md)) |
| `mod.d.rut` | the surface text — hand-written, editable, publishable (humans, LSP); documentation only, never the loader's source of truth |

- The surface rides the binary: a consumer binds `p`'s exports from
  `mod.rutc` alone. Nothing is rebuilt from `.d.rut` text — impl-to-fn
  ids, trait-table indices, and namespace/host rows are not derivable
  from text without guesswork.
- Link compares the consumer's bound surface against the binary's
  export table — a pure data compare. Drift is a load error naming
  both modules, never a runtime surprise.
- Publishing is not encryption: bodies are compiled and local names are
  stripped (`--release` drops member-name strings), but code is
  recoverable with effort. What publishing guarantees is API discipline.

## Surface resolution during compilation

When module `M` uses package `p`, the compiler resolves `p`'s surface,
first hit wins:

1. **source** — `p`'s rut source on the source path: compile it normally;
2. **bundle** — a mounted `.rutbundle`: its compiled `.rutc` (bodies +
   surface on the wire) — or its bundled source for a splice-needed
   group; explicit mounts outrank stray sources, never dev source;
3. **host registry** — for host pkgs, the `.d.rut` the embedder ships
   beside the registered implementation.

Bodies resolve only at link/run. Compiling `M` against a package whose
implementation is absent is legal and complete — checking passes; only
running requires the bodies.

## End-to-end shape

The chat-bus plugin is the canonical walkthrough: a Rust server binds
`server::subscribe`/`server::emit` over its own `EventBus` box, the plugin
subscribes rut export names per topic, and every `emit` re-enters rut to
format the wire line — see [03 — Plugin](../examples/03-plugin.md). For a
full application host with mounted std packages and an async entry, see
[06 — GitHub viewer CLI](../examples/06-github-viewer-cli.md).
