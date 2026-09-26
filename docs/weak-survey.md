# Weak — the survey (RFC 0017 v1, phase 0)

Census at base `8d59215` (the fuel-free landing). Everything the batch
touches, probed on the tree: the three death sites, the surface bill,
the checker/lowering arms, the VM shapes, the test lanes. The design
decisions are recorded in the conversation brief and restated here as
law: **(D1)** opaque store entries are weak-referenceable (the second
death hook lands); **(D2)** `Weak<?U>` is legal — `upgrade` answers
`??U` (MakeOpt wraps the box, the sticky-`?` law), `weak(nil)` traps;
**(D3)** §3's leak report / live counts are menu, not this batch.

## 1. The death sites — where the weak list is nulled

One mechanism, three places a referent can die. The hook is one call,
`weak_null_list(key)`, keyed by the slot word `p as usize` — the exact
keying `drop_fns` already uses for both shapes (`arena.rs:224/:246`).

| site | where | hook |
|---|---|---|
| plain cell | `release_ref_slot`, `n <= 1` arm (`arena.rs:242-251`) | BEFORE the on_drop pin check and `release_cell` — so `dispose` bodies and queued cleanups that call `upgrade()` see `nil`, deterministically |
| store entry | same fn, `is_entry` arm (`arena.rs:216-234`) | BEFORE the entry's on_drop check and `release_entry` (D1) |
| `Arena::drop` teardown (`arena.rs:167`) | survivors drop via `drop_in_place` | none needed — `weak_lists` is engine memory freed wholesale with the arena; dangling entries die with the map |

Ordering law: the nulling precedes anything that runs user code. A
`WeakBox` whose box-cell was already freed must not appear in the list —
box death unregisters itself (below); the tests pin box-first hygiene.

## 2. The surface bill

- **`sym.rs`**: `WEAK`, `UPGRADE` appended to `WELL_KNOWN` (next ids;
  the table is append-only — ids below never move).
- **`types.rs`**: `TyKind::Weak { elem: TypeId }`. No boot row, no
  `TY_WEAK` const — Weak is GENERIC (`Array { elem }` is the precedent:
  `mk_array` interns at use, `check/mod.rs:1348`); `mk_weak(elem)` joins
  `mk_opt` (`check/mod.rs:1366`). `TypeTable::is_ref` (`types.rs:449`)
  lists only `Nil | Prim | Fn` as non-ref — **Weak is automatically a
  ref type**, no change; the same predicate is the admission gate
  (`Weak<i32>` diagnoses at the instantiation).
- **`binary.rs`**: TyKind encode `16` / decode `16` (elem rides as
  `u32`, the `Opt` shape at `:657/:826`); `NativeTy::Weak` in the
  `Surface::core()` row + `core_native_type`; **VERSION 12 → 13** (new
  TyKind + two opcodes — the type-name-law batch's bump law); the
  `binary.rs:956` recompile hint text cites 13.
- **Opcodes, not Nats** — the batch's one structural find: the natives
  need the instantiated TypeId at runtime (`WeakNew` needs the
  `Weak{elem}` id for the box's `CellVal.ty`; `WeakUpgrade` needs the
  `?elem` id for the opt-box mint), and `CallNat` carries no type
  operand. `MakeOpt { dst, src, ty }` (`ops.rs:253`, encoded `:890`)
  is the exact precedent: **`Op::WeakNew { src, dst, ty }`** and
  **`Op::WeakUpgrade { recv, dst, ty }`**. No `Nat` enum change; the
  wire still grows, hence the bump.
- **`core.d.rut`**: the `builtin class Weak<T>` block (ambient, the
  StackTrace/StrBuf row); `core_surface.rs`'s lockstep test parses this
  file against `Surface::core()` — both sides move together.

## 3. Checker & lowering arms (all probed to line)

- `resolve.rs:199-203`: a pre-reserved **stub already diagnoses**
  `Weak<T>` spellings ("not supported in this build (RFC 0017, M5)") —
  the name was waiting. Replaced by real resolution: exactly one
  generic arg → resolve elem → `mk_weak`. (`Weak()`/`Weak<A,B>`
  diagnose arity.)
- `collect.rs:568-588`: the `extern_native_types` impl-target arm gains
  the Weak row — *"Weak takes no impl blocks — its member is engine
  builtin (`upgrade()`)"* (the StackTrace/StrBuf closed-contract text).
- Type-call mint, `call.rs:336-365` (`bytes(n)` / `StrBuf(cap)` arms):
  `name == sym::WEAK` → compile the one argument, `elem` = its type,
  admission `is_ref(elem)` (D2: `?U` admits — `Opt` is a ref), intern
  `Weak{elem}`, `Op::WeakNew`. The `opaque(v)` construction law verbatim.
- Member call, `call.rs:1132+` (the Trace/StrBuf arms):
  `TyKind::Weak { elem }` arm — `(sym::UPGRADE, 0)` → `opt_ty =
  mk_opt(elem)`, `Op::WeakUpgrade`, answer `opt_ty`; everything else →
  *"Weak has no method … — its member is `upgrade()`"*. For
  `Weak<?U>`, `mk_opt(elem)` gives `??U` — the sticky-`?` law, pinned
  as a test (D2).
- Generic bodies: monomorphized per instantiation, so `Weak<T>` inside a
  generic fn interns per concrete `T` — the same path `Array{elem}`
  rides in `Vec<T>`; no special handling.

## 4. VM shapes

- `CellData::WeakBox { referent: Cell<*const CellVal> }` (`cell.rs`,
  beside `StrView`/`ArrView` — the unretained-pointer row). `Cell` for
  interior mutability: the nulling walks boxes through shared `&`
  handles. The word stored is the FULL slot word (tagged for store
  entries) so `retain`/`release` route correctly.
- The list: `Arena.weak_lists: RefCell<HashMap<usize, Vec<*const CellVal>>>`
  — the `drop_fns` shape (`arena.rs:84`), lazy, uncharged engine
  bookkeeping. A cell with no weaks pays nothing (no per-cell field —
  the RFC's "header bit" realized as the map's existence; OQ-2's hook).
- `alloc_weak(v, ty)` (`heap/mod.rs` beside `alloc_trace`): nil → trap
  *"weak on nil"* (the `on_drop` text, `heap/mod.rs:502`); charge
  `CELL_OVERHEAD + 8`; register into the referent's list; failed charge
  unwinds before any registration.
- `op_weak_new` / `op_weak_upgrade` (`interp/ops.rs`, beside
  `op_make_opt` `:292`): the upgrade tail is MakeOpt's minus the
  boxing of null — **dead answers the null slot itself** (a true `nil`,
  never a box containing nil); alive → `retain` + `alloc_opt_value`.
  Receiver must be a `WeakBox` (trap *"upgrade on non-weak"*);
  dst swap + old release per the MakeOpt tail.
- Box death: `release_cell`'s data match unregisters before
  `drop_in_place`; no ref children (the payload is deliberately not a
  `Slot` child — the release walk never sees it).
- `own` on `TyKind::Weak` → share (`heap/mod.rs:639`, Trace's arm).
- Boundary: no `Value::Weak` — the crossing rejects (the Trace/StrBuf
  law; `value.rs` grows nothing). Hosts hold `OpaqueRef`.

## 5. Opaque-entry specifics (D1)

`retain`/`release` route tagged words to the entry's own rc
(`heap/mod.rs:484-490`) — upgrade's retain is shape-blind. The list key
is the tagged word; `weak_null_list` fires in the entry arm before the
pin check. Host payload boxes (`alloc_host_box`) become
weak-referenceable from script for free — the script-side half of
RFC 0017 §1's "host opaques expose their own Weak views" (the host-side
view stays future). `OpaqueEntry::Host` with a `finalize` hook: the
nulling precedes `release_entry`'s finalize — a cleanup can never
observe a live weak to a dying entry.

## 6. Test lanes

- **`crates/rut-driver/tests/weak.rs`** (new, the
  `opt_prim_store`/`stack_trace` lane): alive round-trip identity ·
  death → nil forever · the `Vec<Weak<Tile>>` cache shape ·
  node-cycle-fixed (Weak back-pointer, both cleanups observed via
  `on_drop`) · dispose-sees-dead ordering · immortal enum member ·
  weak over a str / a closure · `Weak<?U>` → `??U` · `weak(nil)` trap ·
  `Weak<i32>` admission diagnostic · box identity `==` · box-dies-first
  hygiene · weak over `opaque(v)` and over a host box (+ finalize
  ordering) · heap `used_bytes` baseline restore · OOM at mint leaves
  rc intact · negative: arity/impl-block/member diagnostics.
- **Updates**: `core_surface.rs` lockstep (both sides), the VERSION
  refusal tests (`stack_trace.rs:378`, `opt_prim_store.rs:551` — old
  artifacts still refuse; message text cites 13 nowhere these read),
  LSP std-surface pin (Weak in completions/index), negative assertions
  where the stub text was unpinned (nothing pins the old M5 stub).
- **Gates**: workspace, wasm32 check, LSP corpus + wasm rebuild + vsix
  re-issue (the surface move binds them — the hashmap-surface
  precedent).

## 7. Interactions & non-goals

- The todolist example runs fuel-free — irrelevant here (weak adds no
  fuel semantics). The heap budget counts WeakBoxes like any cell
  (RFC 0040 unchanged).
- `Arena::drop` torn-frame survivors: weak boxes over them stay
  "alive" to the end — consistent with the frozen trap-teardown
  behavior (the nmap-hostvals balance-law note).
- Non-goals: no collector hook wiring (OQ-2), no `alive()` member, no
  `Weak` iteration surface, no host-side Weak views, no leak report
  (D3 — menu).
- The demo corpus (`weak-cache.rut`, `node-cycle.rut`) still tells the
  strong-refs story and runs green; the Weak rewrite is `demo/*` — the
  foreign lane, never staged by this batch.

## 8. Phase order

P0 this survey → P1 the landing (engine + checker + lowering + surface
+ LSP + tests, one bisectable commit, VERSION 13) → P2 the record
(`docs/weak-report.md` + the RFC 0017 amendment: the `?T` spelling, the
op-not-Nat find, the arena-side list as the shipped realization, D1-D3).
