# todolist-restructure — the survey + design (05 as a medium-scale rut project)

- **Status:** phase-0 deliverable of the `todolist-restructure` batch — survey
  and design only, no code. Phases 1–3 implement it.
- **Lives in:** example-local `examples/05-todolist-web/` — the flat rut files
  fold into a `rut/` subdirectory tree (the user directive, both parts law).
- **Reads first:** the batch plan (`todolist-restructure`), the t1 design
  (`docs/t1-design.md` — authoritative for the framework's laws), the
  dep-kinds report (`docs/dep-kinds-report.md` §6 records this batch by name),
  RFC 0003 §1–2 (the state container + visibility), RFC 0035 §1 (loading, the
  module law), RFC 0038 §1 (module vs project), RFC 0041 §5 (manifests),
  RFC 0045 (dep kinds, the splice dedup).
- **Base:** `26b763e` (post json-perf; VERSION 10). Measured at phase-0 time:
  `cargo test --workspace` exit 0; `cargo test -p todolist-web` green at
  **84 tests** — 9 store + 18 todolist_app + 22 host_surface + 18 t1_diff +
  14 t1_lowering + 3 app_law.

## 0. The laws this design serves (restated, with one correction)

- **BEHAVIOR FROZEN at the observable level.** Every `todolist_app.rs`
  session keeps identical semantics; both e2e tiers green; the DOM contract
  unchanged (`#app`, `#new-todo`, `#add-btn`, `#status`, `#list`,
  `#row-<id>`, child order 0=mark/1=title/2=del); the UI pixel-identical;
  the counts line law and every turn law hold. The old "store.rut
  byte-untouched" law is SUPERSEDED by the directive — the store becomes
  the atom system; the tests prove equivalence.
- **Correction to the plan's text:** the plan says "all 15 sessions". The
  count was 15 when the t1 batch landed (7149d27); the err-channel batch's
  phase 3 (b367af1) added three pump tests
  (`a_boom_turn_is_data_the_pump_reports_and_keeps_draining`,
  `a_panicked_turn_still_kills_the_pump_loud`,
  `a_rejected_add_crosses_the_err_channel_and_the_page_lives`). The frozen
  set is **18 sessions**; phases 1–2 gate on all 18, unchanged.
- **THE TURN-LAW RULING** (t1 batch, binding): tur's no-re-render `Val<T>`
  live-props model stays REJECTED. The atom store adapts the STORE ideas,
  not the reactivity: atoms are state cells; events WRITE atoms; derived
  atoms recompute via dependency tracking with dirty-marking WITHIN a turn;
  render stays ONCE PER TURN through the keyed diff. Subscriptions mark
  dirty — they never push to the DOM.
- **THE WIDGET LAW SURVIVES THE MOVE:** the grep gate
  (`tests/app_law.rs`) is updated to the new import shape and stays
  enforced (§5.1) — exact import set, whole-file scan, no whitelist.
- **THE APPKIT WORKAROUND RETIRES FOR REAL:** dep-kinds landed
  dedup-by-origin (T10; `docs/dep-kinds-report.md` §6: "the single-splice
  wrapper can dissolve into store + t1 uses; nothing in the engine needs
  it"). The restructure mounts modules SEPARATELY and proves it (§2.5). A
  dedup gap found is a dep-kinds bug report, never a workaround.

## 1. The reference: tur.t1's STORE, re-checked

The t1 batch censused tur's RENDERING engine (`docs/t1-design.md` §0). The
store was not censused; it is now, at head
(`libs/tur-engine/src/core/edgy/` — `reactive/store.rs` 1181 lines,
`reactive/mod.rs`, `mutation/`, `watch/`). This is the substrate the user
directive points at ("tur like store, jotai/riverpod like").

### 1.1 The shapes (what tur's edgy actually is)

- **Atoms are id + seed, value-free.** `Seed` is `Source(initial)` /
  `Derived(closure)` / `Mutate(closure)`; minting (`decl`) allocates an
  `AtomId` from one shared counter and registers the seed — no value lands
  anywhere until a store touches the atom (`store.rs:51-60, 367-380`).
- **Values live in a store KV.** `StoreKv.values: HashMap<AtomId, Slot>`,
  where `Slot { value, epoch }` records the invalidation generation the
  value was computed at (`store.rs:295-312`). The instance-wide machinery
  (`SharedReactive`: seeds, the derived graph, flush state, subscribers) is
  store-free; every read/write passes the caller's KV (`store.rs:314-339`).
- **Deriveds recompute lazily, per store.** `read_by_id` auto-tracks deps
  (any read inside a running derived's `tracker_stack` frame records an
  edge), serves the cached slot iff its epoch equals the atom's current
  generation, else recomputes: run the closure, rebind the dep edges
  (diff old vs new), record the slot at the generation RE-READ after the
  closure ran — a write during compute lands the slot already-stale
  (`store.rs:384-434, 436-542`).
- **Writes invalidate through the dependents closure.** `write_by_id`
  equality-skips (`prev == value` → no-op), then walks the dependents BFS
  marking each derived stale and bumping its generation, and sets the
  app-dirty flag (`store.rs:550-598`). Invalidation is push (marking);
  recomputation is pull (next read) — tur never recomputes eagerly.
- **Flush is the turn boundary.** `flush()` drains stale sources +
  stale deriveds for the layout driver; `has_pending` gates the frame
  (`store.rs:696-714`). The element tree declares its deps per layout pass
  into the `SubscriberGraph`, and the driver asks `dirty_subscribers` —
  that is tur's per-element live-props reactivity, the exact model the
  turn-law ruling rejects for rut.
- **Watchers and mutation atoms.** `watch(readable, cb)` registers a
  watcher dispatched through the mutation queue at most once per flush
  epoch, with a watch-loop guard rejecting a callback write that
  re-invalidates what its watcher watches (`store.rs:560-637, 1081-1128`).
  Mutation atoms are closures invoked with a per-store `{get, set}` face
  (`store.rs:639-694`).
- **Families/parameterization: tur has NONE.** There is no
  parameterized-atom shape anywhere in edgy — no `atom family(param)`
  equivalent, no keyed-cell factory. The closest is the engine-atom pattern
  (a designated backing source + a derived handle reading it,
  `store.rs:1134-1152`). jotai's `family` has no tur precedent to copy.

### 1.2 What transfers to the turn law (taken, concretely)

- **The generation rail.** Write bumps a per-atom generation; a cached
  derived value is servable iff its recorded generation matches. This is
  THE freshness mechanism, and it survives the turn law intact (§4.3):
  rut's store keeps `gens` by atom name and each derived records the
  generations it last computed at.
- **Mark-dirty-on-write, recompute-on-read.** Writes push invalidation
  through the dependents closure; computation happens on read. Under the
  turn law the "read" is the turn's one refresh before render — the same
  shape with a deterministic flush point.
- **The dirty set as flush payload.** tur's `stale_sources` /
  `stale_deriveds` sets are exactly what rut's rail drains at refresh;
  write-dedup (a set, not a list) means N writes to one atom in a turn
  mark once.
- **Equality-skip on writes.** Taken for primitive- and str-valued atoms;
  NOT taken for struct/Vec atoms (rut has no structural `==` for them —
  recorded, §4.2).
- **Atoms are cells the store materializes — not globals.** tur's
  seed/decl split maps onto rut's container law: the atoms are FIELDS of
  the store object built at boot, never module state (RFC 0003 §1: no
  mutable module state; module `let` initializers are load-time
  expressions, so an `atom(...)` call cannot live at module scope anyway).

### 1.3 What cannot transfer — the rejections (recorded, not hand-waved)

- **The SubscriberGraph / live-props reactivity.** Per-element dependency
  declaration feeding per-element re-render is the `Val<T>` model the
  ruling rejects (a second time, deliberately). The keyed diff does that
  job once per turn. NOT TRANSFERRED.
- **`watch()` and the watch-loop guard.** Reactive push callbacks are
  reactivity, not state cells; the app's asynchronous facts are the host's
  timer tags, and no watch use case exists. NOT TRANSFERRED.
- **Mutation atoms + the `{get,set}` ctx object.** rut's own event rail —
  one `on_event` entry, subjects, the dispatch — IS the mutation surface.
  A mutation-atom layer would re-encode the entry law badly. NOT
  TRANSFERRED.
- **Auto-tracking via `tracker_stack`.** tur discovers derived deps by
  intercepting reads inside a running closure. rut's deriveds are declared
  as (deps, recompute) registrations — there are no closures to intercept
  (§4.4). The dependency GRAPH survives; its discovery mechanism becomes a
  static declaration the twins can assert on. This is the deepest
  adaptation, and it is the one place the design is deliberately less
  clever than tur.
- **The multi-store machinery.** `SharedReactive` vs `StoreKv` exists so
  one realm can hold several stores with per-store values and
  cross-store coherence generations. rut has exactly ONE store per app
  (the container). The split, the materialize-on-touch semantics, and the
  cross-store epoch walks all collapse — one store, one KV, one gens map.
  REJECTED AS MACHINERY (a simplification, recorded).
- **The cycle guard (`in_flight_derives`).** tur needs it because closure
  graphs are discovered at runtime. rut's dep edges are declared and
  acyclic by construction (the twins pin the declared DAG, §5.3); a cycle
  is a declaration bug caught by the store tests, not a runtime guard.
  NOT TRANSFERRED.
- **Families.** Nothing to transfer even if we wanted to (§1.1); the
  jotai-shaped question is answered on its own merits in §4.6 — rejected
  for this app.

## 2. The mount call — census and decision

### 2.1 Today's anatomy (what exists, cited)

- `src/mount.rs:83-86` embeds four sources verbatim: `nmap_host` decl,
  `nmapset`, `store.rut`, `t1.rut` (all `include_str!` — wasm32 has no
  filesystem).
- `src/mount.rs:114-127` registers **`appkit`** — ONE inline module whose
  source is `format!("{STORE_RUT}\n{T1_RUT}")`. The why is recorded at
  `mount.rs:66-81`: the graph spliced each use's transitive sources per
  use with NO cross-use dedup, and store and t1 both ride pouch (t1 also
  nmapset) — importing both would define `Vec` twice. Spliced together
  they define everything once, and the unit's bound surface carries the
  host crossings — which is how `tim_after` resolves in `todolist.rut`
  with no crossing import (`todolist.rut:87-99`).
- The `web` surface is hand-lowered: `mount.rs:19-31` lowers `web.d.rut`
  (embedded verbatim, `mount.rs:11`) with `host_scope = "web"` and
  registers it — the `rt` precedent. `web.d.rut` sits FLAT at the example
  root, outside any package directory.
- The wasm lane boots through the same mount: `web_dom.rs:222-236` builds
  the session via `mount::mount_app_session`, binds `install_web_hosts` +
  `install_std_nmap`, compiles the app source that crosses from JS
  (loader.js fetches `./todolist.rut`, `loader.js:48`).
- The native twin lanes: `make_host()` in `tests/todolist_app.rs:48-72`
  (the app session, both body sets, `verify_against`), and
  `tests/t1_support/mod.rs:30-57` — which additionally registers `t1` a
  SECOND time, `inline: true`, so the harness's unit splices the framework
  text and its probes can read `T1Root`'s tables directly
  (`t1_harness.rut:269-324` reads `root.els/regs/prev` — cross-module
  field reads that only compile because the splice makes them the same
  unit).

### 2.2 The manifest route (what dep-kinds landed)

- `load_dir_session(dir)` (`crates/rut-driver/src/loader.rs:44-81`) reads a
  directory's `rut.toml`, mounts the root module, walks `[deps]`
  recursively (first-mount-wins, cycle guard, name-mismatch errors), walks
  the ROOT's `[dev-deps]`, then runs the ONE post-closure peer gate —
  RFC 0045 §3's four passes, loader-owned.
- `compile_dir(dir)` (`loader.rs:694-697`) = `load_dir_session` +
  `compile_graph` — the one-call manifest route.
- The splice dedups by origin spec: "two sibling uses sharing a transitive
  inline pkg splice that pkg exactly once" (`graph.rs:10-12`, the skip at
  `graph.rs:253`). Proven by T10 (`dep_dedup::
  t10_sibling_imports_of_a_shared_inline_pkg_compile`) plus its control
  proving the collision was real pre-dedup (`docs/dep-kinds-report.md`
  §2, T10 row).
- A dep LINKS when its exports are concrete: no generic type export, no
  trait-typed fn parameter, not flagged `inline` (`graph.rs:339-356`).
  Linked deps exporting a record type are proven machinery —
  `crates/rut-driver/tests/modules.rs` pins `uses_and_links_a_type` and
  `graph_threads_a_type_through_a_chain` (type identity unifies at link,
  by name).
- The toolchain tree's own convention is one PACKAGE per directory with
  one entry file (`rut/pouch/`, `rut/calc/`, `rut/json/`, …, RFC 0041 §2),
  including a host pkg whose directory is a `.d.rut` + manifest
  (`rut/calc/`, `examples/03-plugin/server/`), and bundle-shaped manifests
  on directories that also pack (`examples/03-plugin/plugin/rut.toml`).

### 2.3 The module law that shapes everything

One directory is one module with ONE entry file; `use` paths are
inter-module; there is no intra-module include form (`loader.rs:4-9`;
RFC 0035 §1; RFC 0038 §1: "a consumer directory with dependencies is a
*project*, not a module"). Two consequences the tree must absorb:

1. **A directory holding N sibling `.rut` files is not mountable as N
   modules.** The only multi-`.rut`-per-directory shape rut knows is the
   RFC 0045 peer group — impl-only files, peer-gated, declaring no public
   names (RFC 0045 §3). Components' builder fns are exactly public names,
   so the peer-group shape is illegal AND dishonest here. Considered and
   rejected. **Per-file therefore means per-package**: each file lives in
   its own directory as its own package (`components/row/row.rut`), which
   is the same shape the toolchain tree itself uses.
2. **Multi-file packages don't exist yet.** RFC 0003 §2 describes "modules
   form a tree per package (files in directories)" with `pub(mod)`, and
   the AST models the visibility forms (`rut-ast/src/ast.rs:221-233`) —
   but the parser lands `pub` only (`rut-parser/src/item.rs:24`), and the
   loader lands one entry per package. The module tree is future; this
   design does not depend on it. (When it lands, the per-component
   packages can collapse into one `components/` package without changing
   any use-path — the package names are the interface.)

### 2.4 The decision: manifest route for the native lane, in-memory mirror for wasm

- **The native lane (tests, any fs host) mounts the manifest.** The
  example gains `rut/rut.toml` — a project-root manifest (RFC 0041 §5):
  `name = "app"`, `entry.lib = "./app/app.rut"`, `[deps]` naming every
  sub-package with relative paths (`./components/row`, `./t1/widget`, …).
  `load_dir_session(examples/05-todolist-web/rut)` mounts the whole
  closure with RFC 0045's passes for real; the twin tests compile the
  root spec via `compile_graph` (or `compile_dir` where a session is not
  needed). This exercises the dep-kinds machinery as a CONSUMER — the
  plan's own queue note ("it exercises the dep-kinds machinery as a
  consumer"), and it is the honest medium-scale shape: the manifest, not
  a Rust fn, is the module list.
- **The wasm lane keeps `Session::register_module`, one call per
  package.** The Session is I/O-free by law (wasm hosts mount in memory,
  `loader.rs:3-9`); `load_dir_session` is fs-only. So `mount.rs` becomes
  the manifest's MIRROR: one `register_module` per package with the same
  fields (source, `inline`, `host_scope`), all `include_str!` — the
  rut-wasm ink/rt precedent, unchanged in kind. A package's `inline` and
  `host_scope` are read from the same values the manifest states, and a
  native consistency test pins the two lanes together (§5.2, P3): the
  manifest closure's mounted-name set and `expected_host_fns()` must
  equal the mirror's.
- **What dies from `mount.rs`:** the `appkit` concatenation
  (`mount.rs:114-127`), the `STORE_RUT`/`T1_RUT` consts and their
  `format!` seam — the workaround and its reason both. What stays:
  `limits()` (`mount.rs:53-59`), `compile_app` for the test-local harness
  sources, and the body installs (`hosts.rs`, `rut_std::nmap`).
- **`web.d.rut`'s legal placement:** its own package directory
  `rut/web/{rut.toml, web.d.rut}` — `entry.type = "./web.d.rut"`,
  `host_scope = "web"`, no `[deps]` — the `rut/calc/` and
  `examples/03-plugin/server/` precedent for a host pkg. A decl file
  loose at the example root is mountable only by hand; the module law
  makes the directory the unit of placement, so the crossing surface
  becomes a first-class package the app's manifest can name. The mirror
  sets the same `host_scope`.

### 2.5 The appkit-retirement proof plan (phases 1–2 execute; pinned here)

- **P1 — separate mounts compile.** The app's session mounts `store` and
  the t1 packages as SEPARATE modules (no concatenation anywhere); the
  app compiles against them; all 18 sessions green. This is T10's exact
  shape (sibling uses of shared inline pkgs) at app scale, and the shared
  pouch/nmapset splices compose once by dedup.
- **P2 — the gate proves there is no appkit.** The updated law gate's
  exact-import set (§5.1) contains no `appkit` name; the string survives
  nowhere in `src/` (a one-line assert in the same test).
- **P3 — the two lanes agree.** A native test: `load_dir_session(rut/)`'s
  mounted-name set + `expected_host_fns()` equals `mount_app_session`'s
  mirror. The manifest is the truth; the mirror cannot drift.
- **If any dedup gap surfaces** (a double-definition, a missing splice),
  it is filed against the dep-kinds record and the phase stops — no
  workaround re-enters `src/mount.rs`. That is the batch law, restated
  where the temptation will live.

## 3. The tree, finalized

Every rut file's residents and imports. Package names are bare
(`[a-zA-Z0-9_]+`, RFC 0041 §5); the directory layout is filesystem
organization only — use paths name packages, never directories.

```
examples/05-todolist-web/
├── rut.toml?                          # none at the example root — the rut
│                                      # project root is rut/ itself
├── rut/                               # THE rut project (RFC 0038 §1: a
│   │                                  #   consumer dir with deps = a project)
│   ├── rut.toml                       # name = "app"; entry.lib = "./app/app.rut"
│   │                                  # [deps] = every package below (relative)
│   ├── web/
│   │   ├── rut.toml                   # name = "web"; entry.type = "./web.d.rut"
│   │   │                              # host_scope = "web" — the calc/server
│   │   └── web.d.rut                  #   precedent (host pkg, §2.4)
│   ├── components/                    # the widget vocabulary, per-file (the
│   │   │                              #   directive) — one package per kind;
│   │   │                              #   each exports its constructor(s), ALL
│   │   │                              #   imports: { widget } only
│   │   ├── col/{rut.toml, col.rut}
│   │   ├── row/{rut.toml, row.rut}
│   │   ├── text/{rut.toml, text.rut}      # text + done/title/pending/muted
│   │   ├── button/{rut.toml, button.rut}  # btn + quiet
│   │   ├── checkbox/{rut.toml, checkbox.rut}  # check
│   │   ├── field/{rut.toml, field.rut}
│   │   ├── spacer/{rut.toml, spacer.rut}
│   │   └── card/{rut.toml, card.rut}
│   ├── t1/                            # the framework core, 3 packages
│   │   ├── widget/
│   │   │   ├── rut.toml               # name = "widget"
│   │   │   └── widget.rut             # Widget struct + WKind/TVariant/
│   │   │                              #   BVariant enums + the fluent impl
│   │   │                              #   (w/gap/pad/hook/key/subject/value/
│   │   │                              #   placeholder/on) + the predicates
│   │   │                              #   (kind_name/takes_*/key_of) pub
│   │   ├── lowering/
│   │   │   ├── rut.toml               # name = "lowering"
│   │   │   └── lowering.rut           # Lowered + lower() + the tag/token/
│   │   │                              #   ladder tables; imports { widget }
│   │   └── core/
│   │       ├── rut.toml               # name = "t1"; inline = true (§3.2)
│   │       └── t1.rut                 # T1Root + t1_mount + t1_subject +
│   │                                  #   t1_render + create/patch/retire +
│   │                                  #   els helpers; imports { widget,
│   │                                  #   lowering } (+ pouch, nmapset)
│   ├── store/                         # THE ATOM STORE (§4)
│   │   ├── atom/
│   │   │   ├── rut.toml               # name = "atom"; inline = true (generic)
│   │   │   └── atom.rut               # Rail + Atom<T> (+ pouch, nmapset)
│   │   ├── derived/
│   │   │   ├── rut.toml               # name = "derived"; inline = true
│   │   │   └── derived.rut            # Derived trait + refresh machinery;
│   │   │                              #   imports { atom }
│   │   └── todos/
│   │       ├── rut.toml               # name = "todos"; inline = true
│   │       └── todos.rut              # Todo, Req, TodoStore (the atoms
│   │                                  #   items/reqs/draft/note + deriveds
│   │                                  #   counts$), the booking/answer
│   │                                  #   machine, the test entry fns;
│   │                                  #   imports { atom, derived }
│   └── app/                           # the biz code (was todolist.rut)
│       ├── app/
│       │   ├── rut.toml               # name = "app" — the project entry
│       │   └── app.rut                # AppRoot + main/on_event/dispatch/
│       │                              #   paint/view; imports { todos, t1,
│       │                              #   widget, web, col, row, text,
│       │                              #   button, checkbox, field, card,
│       │                              #   todo_list, todo_row }
│       ├── todo_list/
│       │   ├── rut.toml               # name = "todo_list"
│       │   └── todo_list.rut          # TodoList: keyed rows + pending adds
│       │                              #   + the subject tables (returned);
│       │                              #   imports { todos, widget, card,
│       │                              #   text, todo_row }
│       └── todo_row/
│           ├── rut.toml               # name = "todo_row"
│           └── todo_row.rut           # TodoRow: check + title + quiet del,
│                                      #   keyed row-<id>; imports { todos,
│                                      #   widget, row, checkbox, text,
│                                      #   button }
├── src/                               # Rust glue: mount.rs rewritten (§2.4)
├── tests/                             # unchanged files, retargeted (§5.3)
├── index.html  loader.js  gen/        # loader.js: fetch path → ./rut/app/app.rut
```

### 3.1 The two law-merges (deviations from the plan's six-word sketch)

The plan sketches `t1/` as widget/builders/lowering/diff/registry/render.
Two compiler laws force merges; both are recorded rejections of the
six-file spelling:

1. **builders → widget.** "Inherent impls live in the type's module"
   (`rut-lir/src/check/collect.rs:566`; RFC 0012 §2; the pouch header
   states the same law). The fluent `.w()/.gap()/.hook()/…` methods
   cannot live in a package other than `widget`. The free CONSTRUCTORS
   (`col()`, `text()`, …) are not methods — they move to `components/`
   per the directive, which is where they belonged anyway (the framework
   exports the type + the lowering; the vocabulary fns are the component
   layer).
2. **diff/registry/render → t1 core.** All of them live in `T1Root`'s
   fields: `els`/`regs`/`lids` are written by mount, render's patch, and
   retirement; `prev` by render (`t1.rut:432-438`, `504-714`). rut's
   landed visibility is `pub` or module-private — the scoped forms are
   AST-modeled but not parsed (`ast.rs:221-233` vs `item.rs:24`), and
   `pub` fields would publish the framework's private tables to biz,
   breaking the very law the gate enforces ("Framework-private, like the
   rest of the tables" — `t1.rut:44-46`). One stateful package keeps the
   fields private and the state machine coherent. The harness's field
   reads (t1p_ probes) are why this package keeps `inline = true` (§2.1:
   the probes read `root.els/regs/prev` — same-unit only).

`lowering` splits out cleanly (pure `Widget -> Lowered`, no state), and
`widget` splits out cleanly (data + fluent + predicates). That is the
whole sketch, honestly resolvable under landed law: **widget / lowering /
t1-core**.

### 3.2 Link vs splice per package (and the honest fallback)

- **LINKED (concrete exports — the `modules.rs` Point path):** `widget`,
  `lowering`, the eight components, `todo_list`, `todo_row`. Type
  identity for `Widget` unifies at link; components never see the
  framework's machinery — only the type.
- **INLINE (`inline = true`):** `t1` (core) — the state package whose
  private fields the harness probes; `atom`/`derived` — generic exports
  splice by law (`Atom<T>` is generic; `graph.rs:339` auto-splices
  generic exports even without the flag; the flag keeps it explicit, the
  nmapset/ink/json precedent); `todos` — its fields are typed by `atom`'s
  generics, so it rides the same composition path.
  Inline is a first-class manifest key with a stated shape reason — it is
  NOT the appkit hack: the hack was ONE module fabricated from many to
  dodge a double-splice; this is per-package splicing with dedup
  (`graph.rs:253`) composing each unit once.
- **Phase-1 proof obligations, stated plainly:** (a) a linked dep bound
  into a spliced unit (t1's text referencing linked `widget`'s surface)
  is the graph's ordinary composition but is not pinned by an existing
  example test — phase 1 either greens it or the package flips to
  `inline = true` (the fallback costs nothing but unit size); (b) linked
  class METHOD calls across a link boundary — the store tests compile
  `todos` as the root (roots always link, `graph.rs:87`), so this path is
  exercised by the store suite before the app ever needs it; same
  fallback applies. Both fallbacks are mechanism, not workaround — the
  gate (§5.1) still proves no appkit exists.

### 3.3 The import graph (acyclic, minimal)

```
web (decl) ──────────┐
widget ──────────────┤
  ↑        ↑          │
lowering  components(8) │
  ↑        ↑   ↑      │
  t1 ──────┘   │      │
todo_list ─────┤      │
todo_row ──────┤      │
  ↑        ↑   │      │
  (app) ←──┘   │      │
store: atom ← derived ← todos
                 ↑      │
              (app) ────┘
app → { web, widget, t1, todos, todo_list, todo_row, col, row, text,
        button, checkbox, field, card }
```

- Acyclic by construction (tree + fan-in; RFC 0035 §1's link-time DAG law
  would catch a cycle at load anyway).
- Minimal, and the LAW at each layer: **components import `widget` and
  nothing else of t1** — the lowering/core stay framework-private (the
  plan's "components -> t1" refines to components -> widget; the biz law
  now holds at the component layer too). **store is standalone** —
  `atom`/`derived`/`todos` import no widget, no web, no t1; the store
  never touches the clock or the DOM (the store.rut:1-10 law, kept).
  **app -> components + store (+ t1 + web)** — the app is composition and
  dispatch, and its one crossing name is `tim_after` (§5.1).
- Name collisions: use paths are ONE flat namespace, and
  first-mount-wins silently skips a duplicate mount (`loader.rs` walk) —
  a collision would be a silent wrong binding, so the tree avoids them by
  spelling: the component package `row` exists, so the app's row view is
  `todo_row` (and its list is `todo_list`). The plan sketch's
  `app/list.rut`, `app/row.rut` file names survive as the CLASS names
  `TodoList`/`TodoRow`; the packages carry the unambiguous names.

## 4. The atom store design (tur-adapted, turn-law-shaped)

### 4.1 Atom identity: the field IS the cell; the name is the key

The jotai question "keyed cells in the store map? handles?" has one
honest rut answer: **atoms are the store object's FIELDS** — `items`,
`reqs`, `draft`, `note` — typed `Atom<T>`, built once at boot inside
`TodoStore`. No runtime handle minting, no `HashMap<str, ?Cell>` of
heterogeneous cells (values would cross as `?Any` downcasts — the
boundary's currency, not a state model's). Module-scope atom constants
are illegal anyway (RFC 0003 §1: module `let` initializers are load-time
expressions; a constructor call is not). The NAME (a `str`) is the atom's
key in the two places keys exist: the rail's generations map and the
turn's dirty set. A `str` key on a field-backed atom is redundant
bookkeeping by design — it is what the derived-dep tables and the twins
read, and it is the seam where a future family would attach (§4.6).

### 4.2 The cells and the rail (`rut/store/atom/`)

```rut
pub class Rail {
    gens: HashMap<str, i64>;      // atom name -> current generation
    dirty: Vec<str>;              // the turn's dirty set (deduped on push)
    fn mark(mut self, name: str) -> nil;   // dedup-push + gen bump
    fn gen_of(self, name: str) -> i64;
    fn drain(mut self) -> Vec<str>;        // take + clear (the turn boundary)
}

pub class Atom<T> {               // generic -> splices (nmapset's law)
    v: ?T = nil;                  // nil only pre-boot; initialized at build
    name: str = "";
    rail: ?Rail = nil;
    pub fn get(self) -> T;               // plain read — no rail traffic
    pub fn set(mut self, x: T) -> nil;   // store + rail.mark(self.name)
    pub fn make(rail: ?Rail, name: str, init: T) -> Self;
}
```

- **get/set discipline.** Reads are plain field reads — no tracking, no
  live channel (the turn law: nothing reads state outside a turn).
  Writes go through `set`, and `set` does exactly two things: store the
  value, mark the rail. Equality-skip (tur's `prev == value`) applies to
  `str`/primitive atoms only; `Vec<Todo>` has no structural `==` in rut,
  so those atoms mark on every set (recorded; the dirty set's dedup makes
  the cost one mark per turn either way).
- **Where writes may happen:** the domain machine (`todos`' booking and
  answer fns) and the event dispatch (`app.rut`'s `on_event`/`dispatch`).
  `view` NEVER writes an atom — view is pure reads plus its one
  sanctioned §7.3 write (the subject tables, which are not atoms).

### 4.3 Derived atoms and the within-turn refresh (`rut/store/derived/`)

rut has no closures to auto-track through (§1.3), so a derived declares
its deps and its recompute as data + a trait impl — jotai's
`derive(() => ...)` with the read graph made explicit:

```rut
pub trait Derived {
    fn name(self) -> str;
    fn deps(self) -> Vec<str>;              // the declared DAG
    fn refresh(mut self, st: ?TodoStore) -> nil;  // reads deps, stores value
}
```

Each domain derived is a small class implementing `Derived`, holding its
cached value and the dep generations it last computed at (`seen`).
`TodoStore` holds the deriveds in DECLARATION ORDER (topological:
counts$ reads items/reqs; nothing derived reads a derived in v1 — order
is still stored, so derived-on-derived needs no re-sort). The refresh:

```rut
pub fn refresh(mut self) -> nil {      // TodoStore.refresh — THE flush
    self.rail.drain();                 // the turn's writes, taken once
    for (let d of self.deriveds) {     // declaration order
        if (self.stale(d)) { d.refresh(self); }   // recompute at most once
    }
    // the rail drained to empty above; seen-gens now match gens
}
```

- **Freshness within a turn.** `paint()` calls `store.refresh()` BEFORE
  `view(r)` (`app.rut`):

  ```rut
  fn paint(r: ?AppRoot) -> nil {
      let mut store: ?TodoStore = r.store;
      store.refresh();          // the within-turn dirty flush — ONE line
      let tree = view(r);
      let mut root: ?T1Root = r.t1;
      t1_render(root, tree);
  }
  ```

  A derived dirtied this turn recomputes exactly once, and reads see
  post-write values (write-then-refresh = fresh — tur's pull model with
  the flush point pinned). The staleness test is the generation rail:
  `stale(d)` iff any dep's current gen ≠ the gen recorded in `d.seen` —
  tur's `Slot.epoch` check, minus the multi-store walks.
- **No cross-turn leakage.** `drain()` empties the dirty set at every
  refresh; a turn that writes nothing marks nothing, staleness stays
  false, and the deriveds recompute ZERO times (pinned by the recompute
  counter in §5.3's new tests). Generations move monotonically; nothing
  survives a turn boundary except clean caches.
- **Derived freshness is not a render gate.** Refresh runs, then render
  runs — ONCE PER TURN, unconditionally, exactly as today
  (`todolist.rut:247-251`). The atom layer adds bookkeeping before the
  view, never conditionality around the diff.

### 4.4 The declared DAG is the dependency tracking

tur discovers edges at runtime (the tracker stack). rut declares them at
the domain (`deps(counts$) = ["items", "reqs"]`) and the twins pin the
declaration: the store tests assert which writes dirty which deriveds
(write `items` → counts$ stale; write `draft` → counts$ fresh), so a
stale declaration is a failing test, not a silent wrong render. This is
the one place the design is deliberately dumber than tur (§1.3); the
compensations are that the graph is enumerable, inspectable by the
twins, and needs no runtime interception machinery at all.

### 4.5 The RFC 0003 interplay: the store is a FIELD; atoms are cells

```rut
struct AppRoot {
    store: ?TodoStore = nil;   // THE atom store (was ?Store)
    t1: ?T1Root = nil;         // the framework's patch state — a FIELD (§7.1)
    toggles: ?PrimMapI64<str> = nil;   // §7.3 subject tables, NOT atoms
    removes: ?PrimMapI64<str> = nil;   //   (the dispatch mirror, rebuilt
}                                      //    per render as today)
```

- No module state anywhere (RFC 0003 §1): the container crosses, the
  pump re-passes it, exactly as since b80ce95. The atom store rides as a
  field beside `t1` — two state machines, one container, the shape the
  t1 design already ruled on.
- **draft and note move INTO the store** as atoms (`draft$`, `note$`):
  the directive makes the store the state-cell home, and both were
  always state (the field's event-carried value; the status tail).
  `AppRoot.draft`/`AppRoot.note` die. Boot: `note$` initializes to
  "booted — type a title, press Add" — the status line is pixel-identical
  from turn one.
- The counts line becomes derivation: `counts$` produces
  `"N open | M done | K in flight"` (deps: items, reqs); the status line
  is `f"{counts$} — {note$.get()}"` — the SAME f-string output the 18
  sessions assert verbatim. The counts line law is now proven THROUGH the
  derived layer (the plan's phase-2 wording), by the same tests.
- Errors: a derived's recompute is plain rut — a panic in one is the
  turn's loud trap, the same law as every panic since phase 2 (soft
  failures are the entry-err channel's business; deriveds have no err
  channel and need none — they cannot fail on data this shape).

### 4.6 Family: REJECTED (the decision the plan deferred)

A family (parameterized atoms — a cell per key) has no tur precedent
(§1.1) and no job here: the per-row state IS the items list, row identity
is the todo id inside the value, and every family read would really be a
list read. Families would mint unbounded rut-side cells with no per-key
retirement law, to replace a `Vec` that already works. The door stays
visible, not open: the rail's name key is where `todo:<id>` cells would
attach if a future app needs per-key identity, and the declared-deps
table is where dynamic deps would have to become honest first. Recorded
as a rejection like `Val<T>` — the design ships `atom/derived/todos`,
and `family.rut` does not exist.

### 4.7 Two censuses the design closes

- **VERSION 10's str surface (`code_at`/`scan`/`starts_with`/`StrBuf`).**
  The store uses NONE of it: keys are whole-equality names, the dispatch
  keeps the no-parsing whole-subject law (`t1-design.md` §7.3), and every
  string the store builds rides f-strings — which land on `StrBuf`
  internally since e880345 anyway. The json-perf surface has no second
  consumer here; recorded so nobody "upgrades" the store onto scan.
- **Budgets.** `mount::limits()` stays fuel 1M / heap 4 MiB
  (`mount.rs:53-59`). The atom rail adds per-turn work (marks, one drain,
  a handful of gen compares); the linked/spliced composition may shift
  per-turn fuel at the margins. Phase 2 re-censuses the 18 sessions'
  fuel/heap and re-pins `limits()` with the json-perf discipline (old
  numbers verbatim in the commit body) if anything trips. VERSION stays
  10 — no binary-format change is in this batch.

## 5. The gates

### 5.1 The grep gate's new contract (`tests/app_law.rs`, phase 1)

The law is unchanged — biz never sinks to the DOM's vocabulary; the
import shape it enforces changes. Three checks, still no whitelist, still
whole-file:

1. **The exact import set** becomes the enumerated package list
   `{ web, todos, t1, widget, todo_list, todo_row, col, row, text,
   button, checkbox, field, card }` — asserted as a SET equality (source
   order no longer pinned; the set is). No `appkit`, no crossing package,
   no store internals (`atom`/`derived` are NOT importable by biz — the
   set omits them, which is how "store standalone" is enforced).
2. **Per-package name pins** (new, mechanical): the `use` lines' name
   lists are asserted exactly —
   `use web::{tim_after};` is the app's ENTIRE crossing surface (the one
   non-widget name, now imported honestly by name); `t1` contributes
   exactly `{ T1Root, t1_mount, t1_render, t1_subject }`; `todos` the
   domain types; each component its constructor(s). A second crossing
   name cannot ride in.
   The FORBIDDEN scan amends one token: bare `"web::"` leaves the list
   (the sanctioned use line contains it) and is replaced by the name pins
   above; everything else stays — `Node`, `ui_` (which still bans every
   `ui_*` crossing name), `class=`, quoted classes, tag literals, DOM
   method names.
3. **The positive halves** stay: `t1_render(` once per paint,
   `t1_subject(` the event door, `.subject(`, `.key(`, `check(`. One
   addition: `store.refresh()` appears in `paint` before `view` — the
   within-turn flush is part of the render law now.

The gate also stops reading only `todolist.rut`: the app file's new path
is `rut/app/app.rut`, and the whole-file scans run over `rut/app/*.rut`
(the three biz files) — components keep their own layer honest by import
scoping (§3.3), which the same test asserts (a component's imports name
`widget` only).

### 5.2 The appkit-retirement proof

See §2.5 — P1 (separate mounts compile + 18 sessions), P2 (the gate
proves no appkit), P3 (manifest lane == mirror lane). Executed in phase 1;
P3's consistency test lands with the mount rewrite.

### 5.3 The test mapping (which session covers what)

| suite (tests/) | covers | mount | changes |
|---|---|---|---|
| `store.rs` (9) | `rut/store/` — the booking/answer machine + the atom rail's laws | core+pouch, root spec `todos` (manifest session, subgraph only — DOM-free stays) | entry signatures unchanged; grows the NEW atom tests below |
| `todolist_app.rs` (18) | the whole closure; every observable law | the manifest session / the mirror; both body sets | UNCHANGED assertions (the freeze) |
| `t1_lowering.rs` (14) + `t1_diff.rs` (18) | `rut/t1/` (widget/lowering/core) via `t1_harness.rut` | app session + `t1` inline (the §2.1 field-read shape, now via the package) | retargeted include paths; probes unchanged |
| `host_surface.rs` (22) | `rut/web/web.d.rut` + the crossing/trap matrix | the app session | path retarget only |
| `app_law.rs` (3) | the biz law over `rut/app/` | n/a (source scan) | the §5.1 contract |
| `e2e-browser.mjs` (2 tiers) | the artifact through the loader ABI (tier 1) / the real page (tier 2) | the wasm mirror | `APP_SRC` + loader.js fetch → `./rut/app/app.rut`; selectors frozen (DOM contract) |
| `harness.rut`, `t1_harness.rut`, `softfail.rut` | test-local sources — they stay in `tests/` | — | import lines retarget |

**NEW twin tests for atom semantics** (in `tests/store.rs` — DOM-free is
the atom layer's own level; ~8 new tests, phase 2):

- **set/get roundtrip** — per atom kind (str, Vec<Todo>): `set` then
  `get` returns the value; `get` performs no rail traffic.
- **write marks once** — N `set`s on one atom in a turn → the drain
  yields the name exactly once; the gen advanced by the mark count.
- **derived freshness within a turn** — write `items` + `reqs`, then
  read `counts$` (after `refresh`): fresh values; the recompute counter
  shows exactly ONE recomputation for the turn.
- **staleness is per-dep** — write `draft` only → `counts$` does NOT
  recompute (its `seen` still matches); write `items` → it does. Pins
  the declared DAG's truth (§4.4).
- **no cross-turn leakage** — after `refresh`, the rail drains empty;
  a no-op turn (no writes) recomputes nothing and leaves every cached
  value bit-identical; gens move only when writes happen.
- **topo order** — a derived-on-derived scenario (added as a store-test
  fixture, not app surface) recomputes upstream before downstream in one
  refresh pass.
- **equality-skip scope** — a `str` atom set to its current value marks
  nothing; a `Vec` atom set marks (documented asymmetry, §4.2).
- Family isolation has NO test — the family was rejected (§4.6); the
  rejection is the record.

## 6. The phase order, confirmed

The plan's 0 → 1 → 2 → 3 stands. One risky thing per phase is already
true of it, and no phase's gates depend on a later phase's shape:

- **Phase 1 — the restructure (structure only).** The tree of §3, the
  mount of §2.4 (appkit dies, P1–P3 land), the gate contract of §5.1,
  loader.js/e2e paths, twin tests retargeted — and the store STAYS the
  old class exactly as-is (moved verbatim into `todos.rut`'s slot; the
  atom rewrite does not begin here). Gates: `cargo test -p todolist-web`
  green (84, assertions unchanged except paths), workspace green, wasm32
  exit 0, e2e BOTH tiers green on the rebuilt artifact, bench pins
  untouched, tree clean.
- **Phase 2 — the atom store.** Store-first: `atom` + `derived` + the
  `todos` rewrite + the NEW store tests (§5.3) — then the app rewiring
  (AppRoot §4.5, `paint`'s refresh line, draft/note as atoms, counts$
  through derivation). The 18 sessions green UNCHANGED is the phase's
  whole point. Fuel/heap re-census per §4.7.
- **Phase 3 — browser proof + close-out.** The agent-browser drive (the
  standing user rule: screenshots through the turn laws on the atom
  build), the README's structure + atom-store teaching, the report with
  the retirement proof and honest limits.

Amendments to the plan text, all recorded above: the 15→18 session-count
correction (§0); the two t1 law-merges (§3.1); the per-package component
shape the module law forces (§2.3); `web/` as a package (§2.4); the
family decision taken NOW, rejected (§4.6) so phase 2 has no
`family.rut` to argue about; the phase-1 linked-path proof obligations
with their inline fallbacks (§3.2).

## 7. Deviations from the plan text (consolidated)

| plan said | this design says | why |
|---|---|---|
| `t1/` six files (widget/builders/lowering/diff/registry/render) | three packages: `widget` (builders merged in), `lowering`, `core` as `t1` (diff+registry+render merged) | inherent impls live in the type's module (`collect.rs:566`); `T1Root`'s fields are framework-private and land-visibility has no scoped forms yet (§3.1) |
| `components/row.rut` siblings in one dir | one package per component: `components/row/{rut.toml, row.rut}` | one directory = one module = one entry (RFC 0035 §1, `loader.rs:4-9`); peer groups are impl-only and cannot carry public builders (RFC 0045 §3) |
| `app/ (app/list/row)` | packages `app`, `todo_list`, `todo_row` | bare use-paths are one flat namespace; `row` is taken by the component; first-mount-wins would silently skip the duplicate (§3.3) |
| `web.d.rut` "or per the module law's placement" | `rut/web/` package, `entry.type` + `host_scope = "web"` | the calc / 03-plugin-server host-pkg shape; a loose decl file is only hand-mountable (§2.4) |
| "families if kept" | REJECTED, decided in phase 0 | no tur precedent, no job in this app; the rail's name key is the recorded door (§4.6) |
| "all 15 sessions" | 18 (15 + the err-channel pump tests, b367af1) | measured; §0 |
| components -> t1 | components -> `widget` only | the biz law at the component layer; lowering/core stay framework-private (§3.3) |

Scratch this phase: `/tmp/opencode/batch-todolist-restructure/p0/`.

## Addendum (Sep 2026): the reversal — the store the generic-impls batch shipped

The design above was written against one premise: rut had no way to
intercept a read. This batch — the parameterized trait impls (RFC
0012's amendment; commits b1735de, bf0872b, 7b379b2) — landed the
machinery that premise lacked, and the example shipped the store this
design rejected. This addendum records the reversal, decision by
decision. Evidence for everything below is the spike lane
`examples/05-todolist-web/tests/spike0.rs` (+ `spike0_store.rut`, the
mini-store), **11/11 green**: closure-erasure roundtrip, `?T`/tuple
generic args, generic trait impls + fat refs, and the mini-store's
pull-on-read, discovered edges, str content-skip, derived-on-derived,
cycle guard, derived-not-writable.

### A.1 §1.3's rejections, overturned

Three of §1.3's "NOT TRANSFERRED" rows are dead records now:

- **Auto-tracking via the ctx's reads — TRANSFERRED.** The recorded
  reason was "there are no closures to intercept" (§1.3, §4.4). That
  premise died with RFC 0013's first-class fn types plus this batch's
  dispatch: a derive IS a closure now —
  `store.derive(fn (ctx) -> str { ... })` — and **the `ctx.get` calls
  inside it are the dependency declaration**, discovered fresh every
  recompute (tur's `tracker_stack` shape, `store.rs:384-434` — the
  exact mechanism §1.1 censused and §1.3 declined). There is no
  interception machinery at all: `DeriveCtx.get` records the edge as a
  side effect of answering the read, the read riding a module-private
  `Readable<T>` template and the mint (`find_or_mint_impl`, RFC 0012's
  amendment) typing the frame.
- **Mutation atoms + the `{get,set}` ctx — RETURNED, as the MutCtx.**
  §1.3 rejected the mutation-atom layer because the event rail was the
  mutation surface. The shipped shape keeps the event rail (one
  `on_event`, subjects, dispatch — untouched) and adds the tur face
  under it: `store.mutation(fn (ctx, title: str) -> Req { .. })` mints
  an erased mutation; `store.set(mutation, arg)` is the machine's verb
  lane — the ONE write door. The ctx asymmetry is the purity law:
  a derive's ctx exposes `get` ONLY, a mutation's ctx exposes `get` +
  `set`.
- **The declared DAG (§4.4) — RETIRED.** "The twins pin the declared
  DAG; a stale declaration is a failing test" is replaced by something
  stronger: **impossible-to-stale by construction**. With discovered
  edges there is no declaration to get wrong — `deps_of(a)` reads the
  graph the store actually walked, and `stale()` follows the
  discovered edges (one derived-dep level deep, upstream first), so a
  chain cannot serve a stale value no matter what anyone declares. The
  observability the twins wanted survives as surface
  (`store.recompute_count()`, `store.deps_of(a)`), not as a
  declaration to assert on.

### A.2 What the reversal does NOT touch

§1.3's remaining rejections stand: **live-props/SubscriberGraph** (the
turn-law ruling rejects it again — rendering stays the keyed diff,
once per turn), **`watch()`** (still no use case), **families**
(§4.6 — the handles ARE named locals now, which is the family answer
without the machinery), **the multi-store split** (one store, one KV —
§1.3's simplification, kept).

### A.3 §4.3's turn-boundary flush — RETIRED for pull-on-read

The design's freshness law was the flush: `drain()` the dirty set,
refresh stale deriveds in declaration order, then render (§4.3). The
shipped law is tur's, with no flush point at all: **`get` recomputes a
stale derived at most once per write-set** — the generation rail of
§1.2 was the keeper and it is still the whole freshness mechanism (a
cached value is servable iff its recorded gens match; a recompute
re-records what it read). The write lane marks; the read lane pulls;
there is no drain, no dirty set, no `store.refresh()` line in
`paint` — §5.1's positive half for the refresh line is retired with
it. **The render stays once per turn**, unconditionally, exactly as
the freeze demands: the flush retirement changes WHEN freshness
happens (first read after the writes), never HOW MANY times the diff
runs.

- **The cycle guard tur needed RETURNS** (§1.3 rejected it because
  declared edges were acyclic by construction — discovered edges admit
  cycles). An in-flight set makes a re-entrant read a LOUD trap: a
  self-reading derive panics naming the cycle, it does not loop or
  serve garbage. The spike pins it (`spike_e_cycle_guard`) alongside
  the legal form of the same machinery (derived-on-derived refreshes
  upstream-first on a single pull, `spike_e_derived_on_derived_...`).
- **`Derived` is not writable, at the type level.** The runtime trap
  exists, but the static refusal comes first: `Derived` implements no
  `Writable`, so `store.set(derived, ..)` does not compile — the trait
  law (RFC 0012: impl-trait methods ride the trait's visibility) doing
  the type-system work the design wanted from the twin tests.

### A.4 The TWO-PACKAGE law

The §3 tree — sixteen packages, sixteen `rut.toml` — shipped as TWO:

- **`rut/ui`** — the framework: the atom-store kernel, the t1 widget
  framework, and the component vocabulary in ONE module (`inline =
  true`; generic exports splice by law — the atom/nmapset precedent).
  Pure of biz: nothing in it names a todo.
- **`rut/biz`** — the domain and app in one module, and the PROJECT
  ROOT (name = "app"): sources, derived counts line, the machine as
  mutations, views, shell. Deps: `ui`, `pouch`, `nmapset`.

The §3 per-file component packages, the `web/` host package (§2.4),
and the appkit-retirement proof plan (§2.5) are overtaken: there is no
sixteen-manifest closure to dedup, and `web.d.rut` RETURNED to the
example root — it rides no manifest path in either package; **the
embedder registers it in both lanes** (`src/mount.rs`), which is the
honest shape for a surface that exists only where a host does.

- **TodoStore died; the ProbeStore fixture died with it.** §4.1's
  "atoms are the store object's FIELDS" and §4.5's `store: ?TodoStore`
  are reversed: the store is a **decoupled-biz kernel** — RFC 0014's
  opaque-KV pattern (values and erased fns live as `opaque` boxes in
  id-keyed maps; `opaque.downcast<T>` at the read and dispatch
  boundaries) — knowing no domain type. Biz holds HANDLES: `items$`,
  `counts$`, `add_m` are minted at boot (`store.source<T>(..)`,
  `store.derive(..)`, `store.mutation(..)`) and passed in a handles
  bundle. §4.2's `Rail`/`Atom<T>` classes and §4.3's per-domain
  `Derived` impls are gone — the machinery is one generic kernel, and
  the domain writes closures instead of classes.
- The boot ABI is unchanged (the root spec stays `app`: the host boots
  `main`, re-passes the container to `on_event`), and the counts line
  comes out of `store.get(counts$)`.

### A.5 The visibility ruling

rut's landed visibility is **`pub` or module-private** (§3.1's
recorded reason — the scoped forms are AST-modeled, unparsed). The
two-package law is what turns that limitation into the design's
enforcement:

- **One module per package is what makes the machinery private for
  real.** The store's eyes — `Readable<T>`, `Writable<T>`,
  `DeriveCtx`, `MutCtx` — are module-private IN `ui`, and `ui` is one
  module, so **biz cannot name them even to import them**. §4's whole
  worry about `pub` fields leaking framework tables dissolves: there
  is no second module inside `ui` to leak into.
- **The import-set law gate (`tests/app_law.rs`) is the second
  enforcement half.** The gate (§5.1's contract, retargeted) asserts
  biz's import set is EXACTLY the two-package vocabulary — `ui`'s
  public names (`Store`, `Source`, `Derived`, `Mutation`, the widget
  vocabulary), `pouch`/`nmapset`, and ONE crossing name (`tim_after`);
  the machinery traits appear nowhere, and the forbidden scans (no
  tags, no tokens, no DOM vocabulary) are unchanged. Compiler law and
  gate law agree: the kernel is reachable only through
  `store.get`/`store.set`.

### A.6 The net

The design's stack — declared DAG, flush, dirty set, Rail, Atom,
TodoStore, sixteen manifests — is gone. What replaced it is smaller
AND closer to tur: a generic opaque-KV kernel with discovered deps,
pull-on-read, and a cycle guard; two packages; one crossing name. The
turn law survived every reversal: render once per turn, events write,
the DOM never hears about state — the model changed under the law,
never the law itself.

---

## Addendum (Sep 2026): the surface moves onto the handles — tur's law

The two-verb surface ("biz may use exactly `store.get`/`store.set`")
was the right law with the wrong spelling: it kept the STORE itself a
biz-visible object, one `world.store` reach-around away from
everything the law protected. tur (github.com/hpp2334/tur — the
todolist case) shows the completed shape: the app never touches a
store object; atoms and mutations are free-floating values the app and
the widgets pass around, and a component's `onClick` IS a mutation
prop. Ported:

- **The handles carry their store** (a private back-ref) and the verbs
  with it: `a.get()`, `a.set(v)`, `m.run(arg)`. The store's own
  get/set became ui-MODULE-PRIVATE — the compiler now enforces what
  the grep gate pinned. biz spells the store exactly twice, both
  legal: world_boot mints through it; the host-test surface reads its
  observability.
- **Events are props**: `.on_click(m)`/`.on_input(m)` on the widget
  vocabulary; `t1_event` resolves the firing listener to the widget's
  OWN mutation and runs it (the framework never holds a store);
  `capture(m, id)` binds a row's argument — the §7.3 subject tables
  and the dispatch fn are gone, nothing to stale.
- **The machine absorbs the turn steps**: add/toggle/remove event
  wrappers (minted at boot) write the status notes the dispatcher
  wrote, in the same order, and return the BOOKING as a box —
  `opaque(Req)` when a timer must be booked, `opaque(nil)` when not.
  The sessions' status lines are byte-identical (all 18 pinned).
- The import set sheds `nmapset` (the subject tables were HashMap's
  last biz use) and gains `capture`/`t1_event`.

What did NOT change: pull-on-read, the cycle guard, the one write
lane, the turn shape (one paint, one keyed diff), the 94-test suite's
bytes. The law survived another reversal the same way it survived the
others — the model changed under the law, never the law itself.
