# the async survey (phase 1)

**Batch:** async/await — the Future-only vocabulary · **Phase:** 1 (P0) · **Base:** `b488178` (the big-files split) · **VERSION:** 13 — stays · **Date:** 2026-09-27

The plan's §1 rulings, all eight, in order — the spine this survey documents and the P1 landing implements:

1. **Async/await lands; suspend is scrubbed.** `suspend` was never a keyword — it exists only as RFC 0018's title/filename and prose (verified: parser `RESERVED_KW` has no `suspend`, `rut-parser/src/lib.rs:446-454`; the one guard is the LSP completion exclusion, `rut-lsp/src/completion.rs:326`). Removal = rename the RFC + scrub vocabulary; the LSP test stays as the guard.
2. **`launch_*` is the only trigger.** No bare-poll surface; the driving loop owns every Pending future. Nothing pends unowned by construction.
3. **RFC 0012 §7 is the frozen surface authority** — the cx protocol (`checkpoint`/`next_checkpoint`/`cancelled`), explicit first param `async fn(cx: RunContext, …)`, builtin traits engine-named but user-open, `launch` in core-adjacent surface, users write their own launchers.
4. **`Task` is dropped entirely.** The handle is not a new *concept*; the launched future's receipt is its own type. No `Task`, no `LaunchedTask`, no `on_finish`, no `Cancelled` enum, no `Result` wrapping, no bridge future.
5. **`Future<T>` is the trait name** — RFC 0018's own word; Rust precedent (Future is a trait driven by a context). `TaskRunContext` → `RunContext`.
6. **The handle must not be re-launchable.** `Future<T> -> Future<T>` is wrong (`launch_future(launch_future(f))` is a type error, never a runtime check). The handle is a distinct, non-launchable, non-awaitable type.
7. **The handle's name: `LaunchedFutureHandle<T>`**, single member `abort(mut self) -> bool`.
8. **launch_future / LaunchedFutureHandle / sleep are HOST surface, not core** — ordinary code over the open surface ("users may write their own launchers", RFC 0012 §7). Engine owns only the weaving, the driving primitives, and the RunContext backing. Each embedder mounts the standard set.

Cancellation is the context probe, never a value: abort → flag → frame resumes → `cx.cancelled()` → drop path (`on_drop` locals, RFC 0016 §3 order) → release. `await` consumes; one future, one awaiter; join lands with RFC 0019 — v1: `await` on a handle is a type error, and nobody can be parked awaiting a launched future, so cancellation has no propagation edges to define yet.

---

## §1 the VERSION verdict — 13 stays (the deciding question, traced)

**Ruling: RunContext's three methods lower without a single new op, native, or wire row. VERSION stays 13; bundles stay compatible; RFC 0032's amendment is the "op set untouched" note.** The trace, lane by lane, of how comparable surface lowers today:

- **The compiler-lowered free fns** (`assert`/`panic`/`on_drop`) are special cases in the call lowering that emit existing ops directly — `Op::Panic` (`rut-lir/src/lir/call/mod.rs:199-210`), `Op::Assert` (`:211-232`), `Op::OnDrop` (`:180-198`). No fn table row, no call.
- **The VM-bodied natives** (`capture_stacktrace`, `string_join`, every `StackTrace`/`StrBuf` member) lower to `Op::CallNat { nat: Nat::X }` — `Nat::CaptureTrace` (`call/mod.rs:233-245`), `Nat::StrJoin` (`:246-266`), `Nat::TraceLen/Name/Line/Col/Render` (`call/method.rs:278-336`). **This is the lane the batch must NOT touch**: `Nat` is a rut-core binary enum whose tag rides the wire as the CallNat operand's u8 (`rut-core/src/ops.rs:83`; encode `binary.rs:894` `e.u8(*nat as u8)`, decode `:944`). `Intrinsic` (the `builtin impl` numeric table, `ops.rs:73-77`) is the same class. A new row here is wire vocabulary growth — the VERSION-14 lane.
- **The session-registered engine fns** (the `add_extern_fn` lane): the driver binds used fns by packed `(scope, local)` (`rut-lir/src/check/externs.rs:8-10`; call site `rut-driver/src/lib.rs:168-171`). A native module's host fns materialize at link as bodyless `FuncCode` entries carrying `host_id` (`rut-driver/src/graph.rs:144-165`), and the VM dispatches them through the **same `Op::Call`** as ordinary rut→rut calls (`rut-vm/src/interp/run.rs:83-92`; the RFC 0025 join resolves `host_id` → `HostSlot` at `Vm::new`, `interp/mod.rs:229-241`). The wire sees only `Op::Call`. Host bodies receive `&mut Vm` (`rut-std/src/logger.rs:28-31` — `|vm: &mut Vm, name| vm.alloc_opaque_str(..)`) — the engine halves of launch/abort/sleep fit exactly (§3).

**How the cx members actually lower — the verified answer: inline field ops, not calls at all.** The cx is an engine-minted record cell whose layout the compiler owns (§4); its members inline the way `builtin impl` numeric methods inline off the receiver (`externs.rs:16-33`, bound ambient at `lib.rs:177-180` — the established ambient-member-without-a-call precedent), and the same way every `builtin primitive` member inlines per-receiver-kind (`call/method.rs` — the `TyKind::Trace` arm `:278-336` is the shape):

- `checkpoint(self)` → `GetF` of the state field; `next_checkpoint(mut self, v)` → `SetF` of it; `cancelled(self)` → `GetF` of the flag. Existing memory ops, zero call overhead, zero surface rows beyond the signature decls.
- The one genuine vtable call in the whole design is the polled `fut.yield(cx)` — `Op::CallI`, the existing trait-slot lane (`rut-vm/src/interp/ops.rs:450-482`: receiver's effective type off the cell → `prog.vtables[ty][slot]` → fid → `enter`). No new op.

**Trait registration rides the Iterator lane verbatim.** `Future`/`RunContext` publish as `NativeTrait` rows in core's surface (`binary.rs:185-192`; `Surface::core()` `:260-298`), bound ambient by the existing native pass (`lib.rs:240-244` → `add_extern_trait`, `externs.rs:66-68`). **Disclosed nuance:** `NativeTy`/`NativeTrait` are compile-time surface vocabulary — the wire grammar (encode/decode, `binary.rs:494-700`) carries the resolved program (interner, types, traits, trait_slots, vtables, consts, funcs, exports) and **no surface at all**, so even these enum rows cannot move the wire; the enum already carries retired rows (`Disposal`/`Index`, `binary.rs:186-189`) rather than bumping anything. The plan's stricter "binary enums change not at all" reading holds where it matters — the wire enums (`Op` tags, `Nat`, `Intrinsic`, `ConstVal` tags, `TyKind` tags) grow zero rows: the cx and frame types are ordinary `TyKind::Data` records (tag 8, `binary.rs:638-643`), **not** new `TyKind` variants (StackTrace needed `TyKind::Trace` = tag 14 because its payload is engine-abstract; the cx's payload is plain fields — Data suffices).

**Nothing forces 14.** The lockstep gate (`rut-driver/tests/core_surface.rs:1-14`) sees two new builtin trait decls in `core.d.rut` and `Surface::core()`; the VERSION const (`binary.rs:489`) and the decode grammar are untouched.

## §2 the done-probe — engine state read from compiled code, never a surface member

**Ruling: the probe is the engine-reserved state field on the frame cell, read by the await expansion with plain `GetF` + `Br`; completion is the DONE sentinel written into that field.** The expansion's middle lines are therefore:

```text
<calli fut.yield(cx)>          ; cold-poll — existing CallI, dst discarded
getf r_done, r_fut, f<STATE>   ; the probe — the engine-reserved field
br r_done -> s1, park          ; existing Br
```

Grounding against the VM's realities:

- **The field exists by construction**: the engine mints the frame type (§4) with the state slot at a fixed field index; `GetF { dst, obj, field, repr }` reads a record field by static index (`CellData::Record { fields: RefCell<Slots> }`, `heap/cell.rs:512-513`). The cx members of §1 touch the *same* state through the same op family — one engine-state convention, one accessor family.
- **The DONE sentinel**: the engine's drive wrapper marks the cell when a driven frame completes (the plan's sleep fragment: "done -> mark cell done"). Engine-side writes into live cells through shared handles are established machinery — the WeakBox nulling does exactly this from the release path (`cell.rs:538-546`). The woven completion block can equivalently write the sentinel itself (`SetF`, one op) so completion is self-declaring without engine involvement.
- **Park/resume reality**: `SavedFrame { func, pc, regs, ret_dst }` (`interp/mod.rs:88-93`) and the budget park — `resume_raw` ("RFC 0034 §4: the frame IS the loop state", `:801-806`) — already persist an interrupted frame without unwinding. The `InterpCursor` stash/restore (`:420-460`: "nested entry: stash the outer cursor, run the callee to its root ret on a clean frame stack, restore either way") is the exact re-entrant shape `drive(fut)` uses: stash, call the frame's `yield`, restore. Fuel per drive step rides the existing budget plumbing (`Limits { fuel, interrupt_every }`, `:59-61`; RFC 0034/0040).
- **The brtable's operand is an enum cell — the fragment's one tension, resolved here.** `Op::BrTable` dispatches on `cell_of(regs[idx]).as_enum_member()` (`run.rs:114-120`) — an enum-member cell, not a raw u32. So the engine mints a per-async-fn **checkpoint enum** (one member per state; `CellData::Enum { member }` cells are immortal singletons per (ty, member), `cell.rs:510-511`), the state field holds the singleton slot, `GetF` fetches it, `BrTable` dispatches on the member index. `checkpoint() -> u32` stays the frozen §7 *signature*; the engine owns both sides of the weave and answers with the minted singleton — the divergence is compiler-internal, invisible to the surface.
- **The frozen-set law holds**: the probe emits no call, no `Nat`, no trait row, no member on `Future`/`RunContext`. §7's member sets are exactly what the lockstep gate sees.
- **v1 await-legality refinement (pinned for P1)**: the probe reads the ENGINE state field, so `await`'s operand must be an engine-woven future (an async-fn call result or `sleep(..)`). A user `impl Future<T>` (example 04's shape) is launcher-drivable but has no engine state field — awaiting one diagnoses in v1 ("await targets engine-woven futures; user impls drive through launchers — join lands with RFC 0019"). This is the probe's soundness requirement stated as a diagnostic, consistent with the plan's own fragments (`await sleep(1000)`, `await` on async-call results).

## §3 where the standard host set lives — the `rt`/`nmap_host` lane, mounted per-embedder

**Ruling: a tree-shared native module — decl surface `.d.rut` + bodies in rut-std behind an install fn — mounted by each embedder BY CHOICE. Never core; no new engine registration API.**

- **Why not core**: core's own law — "core registers no host bodies at all; `host fn` is exclusively the EMBEDDER'S surface (a registered NativeModule, RFC 0025)" (`rut/core/core.d.rut:10-12`; enforced by the lockstep test, `core_surface.rs:5-8` "core declares NO `host` surface"). Ruling 8 says the same thing from the design side.
- **The established shape to copy**: `rut/rt/rt.d.rut` lowered at compile time via `lower_decl_module` and registered per-embedder (`rut-wasm/src/lib.rs:113-119`, `host_scope = "rt:log"`); `rut/nmap_host/nmap.d.rut` the same (`:123-130`) — "This host's choice, not the engine's: the driver knows none of these names" (`:106-108`). Bodies live in rut-std behind install fns taking `&mut HostRegistry` (`install_std_log`, `rut-std/src/logger.rs:22-39`; `install_std_nmap`; the CLI installs its set before `Vm::new`, `rut-cli/src/main.rs:172-185`).
- **The engine halves fit the host-fn ABI as-is**: bodies receive `&mut Vm` (logger.rs:28-31), so `launch_future` (alloc the handle cell + push the ready queue), `LaunchedFutureHandle.abort` (`task_of` → mark cancelled → re-enqueue), and `sleep` (mint the future + arm the timer) are ordinary registered fns over the VM's new queue API (`drive`/`next_deadline`/`cancel`, plan §3). No `CallNat` rows, no engine knowledge of the names.
- **Recommendation concretely**: `rut/async_host/async_host.d.rut` (the `nmap_host` naming precedent) + `rut_std::…::install_std_async`; the CLI, the wasm playground, and the tests mount it explicitly. A no-launcher session loses nothing — `await` is cold-poll inline, synchronous when nothing pends (the plan's own law).

## §4 the frame cell — an ordinary Record cell, a per-type vtable row, cell-backed locals

**Ruling: the async frame is an engine-minted `TyKind::Data` type whose cell is an ordinary `Record`; `Future::yield` rides the existing per-type vtable; the locals are cell-backed from the start.** No new `CellData` variant, no per-object vtable pointer:

- **The cell**: `CellVal { ty, data, refs, bytes }` (`heap/cell.rs:7-15`) + `CellData::Record { fields: RefCell<Slots> }` (`:512-513`); the inline `Slots` buffer (`:21-26`, `INLINE_SLOTS = 4`) covers small frames, larger spill to `Heap(Vec<Slot>)`. Refcount and heap accounting ride the existing cell machinery free. Minting is `alloc_record`/`alloc_record_zeroed` (`heap/mod.rs:392,399`). Field 0 (fixed index) is the engine's state slot (§2).
- **The vtable row**: vtables are per-TYPE — `prog.vtables[ty][slot] -> func id` (`binary.rs:378-379`, encoded `:529-537`); `CallI` reads the receiver's effective type off the cell (`ops.rs:454-462`). No per-object vtable exists or is needed — the frame's concrete type IS its `Future` impl identity. The engine's weave is exactly an **impl block the compiler writes on the user's behalf** — `impl Future<T> for <hidden frame type> { fn yield(cx) { …desugared body… } }` — riding the ordinary registration path (`collect_impl`; `TraitReq` already tracks `is_async` and enforces signature coverage, `rut-lir/src/check/collect_impl.rs:650-656`). Example 04's user impl registers through the same registry — that is the file's stated purpose.
- **pc/regs persistence**: the *resume point* is the state value (the checkpoint enum singleton, §2) — the brtable IS the resume dispatch, so "the pc is the state" is literal: the state field names the brtable arm. The locals are **cell-backed** (the body's bindings lower to the frame cell's fields, the "frame cells (park/resume at pc)" law), so the park `ret` — a real `Ret`, well-formed code — releases only staging registers, never a live local. No generator liveness splitting: every local is cell-backed uniformly, no analysis decides which escape. `SavedFrame`/`InterpCursor` remain what they are today — the budget-interruption and re-entrant-call machinery (RFC 0022/0034), unchanged. The alternative (park-without-ret, parked `SavedFrame`s on a side stack) was rejected: it would make the ready queue carry `(slot, SavedFrame)` pairs instead of the plan's plain `VecDeque<Slot>`, and it forks the return path.
- **The cx**: a second engine-minted Data record — the (task, frame) handle — fields for the state/flag view and the task edge; minted at async call sites "like `self`" (the plan). Its members inline per §1. `cancelled` reads the task's flag; `abort` writes it (engine-side cell write, §2).

## §5 examples/04-custom-async — in-batch, two tiers; the live harness defers first

**Ruling: the vocabulary port is mandatory in-batch; the live `vm.drive` harness is the batch's first disclosed deferral if room runs out.**

- The file exists: `examples/04-custom-async/custom_async.rut` — and it is **old vocabulary**: `use core::{ Task, TaskRunContext }` (`:25`), `impl Task<T> for CustomTask<T>` (`:122`), `LaunchedTask` prose (`:148`). Its own header says PARSE-ONLY CORPUS until the async plan lands (`:4-13`).
- **Tier 1 (mandatory, cheap)**: port the source to the frozen vocabulary — `Future<T>`/`RunContext`, the user `impl Future<T> for CustomFuture<T>` (the file's stated purpose: the user-impl-of-builtin-trait test, `:10-13`), the user launcher shape. Leaving `Task` spelled in the tree contradicts ruling 4 ("no `Task` concept anywhere, ever") — the port is part of the vocabulary scrub, not an optional example refresh.
- **Tier 2 (the deferral candidate)**: the LIVE harness — a Rust-side test that drives the custom future over `vm.drive` with a recording sink (the fmt batch's `rt.log`-line semantic pattern). It needs P1's VM queues landed and is real test-lane work; if the batch runs long it is the disclosed follow-up, with tier 1 still landing.

## §6 the anchors, re-verified at base `b488178`

- `await` parses: `Pfx::Await` prefix (`rut-parser/src/expr.rs:159-163`), expression build `:190-192`; `await select {..}` (`SelectFrame`, `:1124+`); `async`/`await`/`select` in `RESERVED_KW` (`rut-parser/src/lib.rs:446-454`).
- The M3 rejection gate is one site: `rut-lir/src/lir/compile.rs:135-161` ("cold-poll futures land in M3 (RFC 0018)") — `is_async` rides fn + method decls (`:136-137`). It becomes the landing.
- Builtin-trait lane: `NativeTrait` (`binary.rs:185-192`), ambient binding (`lib.rs:240-244`), `add_extern_trait` (`externs.rs:66-68`); trait descriptors from used modules (`add_extern_trait_decl`, `externs.rs:83-104`); impl registrations (`add_extern_impl`, `externs.rs:110-119`). `Iterator`'s `for`-desugaring binds statically to the registered impl (`lir/stmt.rs:420,467-472`) — the await expansion's `CallI` is the dynamic sibling.
- Frame reality: `SavedFrame { func, pc, regs, ret_dst }` (`interp/mod.rs:88-93`); `InterpCursor` re-entrant call (`:420-460`); budget park/resume (`:801-838`); ops `Jmp/Br/BrTable/Ret/Call/CallM/CallI` (`rut-core/src/ops.rs:210-234`); `BrTable` reads enum-member cells (`run.rs:114-120`).
- Host crossing gate: `crosses_boundary` (`rut-core/src/types.rs:488-502`) — primitives/str/bytes/Opaque/`?T`/tuples only; boot classes (`TyKind::Trace`, `TY_STACK_TRACE` `:273`) do NOT cross, enforced on host-fn registration (`graph.rs:126-139`). The cx/frame designs respect this by never crossing (engine-minted, compiler-owned).
- Lockstep gate: `core.d.rut` ↔ `Surface::core()` (`rut-driver/tests/core_surface.rs:1-14`); `suspend` excluded from LSP completion (`rut-lsp/src/completion.rs:326`).
- Embedder mounting lanes: wasm (`rut-wasm/src/lib.rs:110-130`), CLI (`rut-cli/src/main.rs:172-185`), rut-std installs (`logger.rs:22-39`), host join at `Vm::new` (`interp/mod.rs:213-241`).

## §7 what P1 implements (this survey is the contract)

1. **core.d.rut + `Surface::core()`**: `builtin trait Future<T>` / `builtin trait RunContext` as `NativeTrait` rows (§1); the lockstep test's trait table grows the two names with their method sets.
2. **rut-lir**: the desugaring per the plan's fragment with this survey's pinned mechanics — checkpoint brtable over the minted checkpoint enum (§2), await expansion `calli`/`getf`/`br` (§2), cancelled-probe → drop path (RFC 0016 §3 order), value-discarded completion; async call sites mint frames + the compiler-written impl (§4); await legality (inside-async-only; non-Future operand; handle operand → "join lands with RFC 0019"; non-engine-woven operand → §2's diagnostic); `await select` keeps parsing, compile-gated "RFC 0019 — not in this build"; cx members inline as field ops (§1).
3. **rut-vm**: ready/timer queues (`VecDeque`/`BTreeMap` — wasm32-clean), `drive` (the InterpCursor-shaped re-entrant yield call, Done|Parked answered off the state field), `next_deadline`, `cancel`; DONE-sentinel marking; `on_drop` ordering across resumptions; fuel per drive step (§2).
4. **Host set**: `async_host.d.rut` + `install_std_async` in rut-std (§3), mounted by CLI/wasm/tests; `launch_future` consumes; `LaunchedFutureHandle<T>` non-launchable/non-awaitable.
5. **Tests**: the plan's P1 list, plus example 04 tier 1 (§5).
6. **RFC amendments**: the plan's §6 map, with RFC 0032 taking the "op set is untouched — VERSION stays 13" note.

**Gates**: `cargo test --workspace` green at identical-or-better counts (857 baseline); wasm32 check clean; **VERSION 13** (this survey's verdict); vsix 0.2.5 untouched; demo/AGENTS.md/untracked scripts never staged.
