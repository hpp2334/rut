# the async report (phase 2)

**Batch:** async/await — the Future-only vocabulary · **Phase:** 2 (P1
landing + the record) · **Base:** `45a2fc1` (the async survey) ·
**VERSION:** 13 — stays · **vsix:** 0.2.5 — untouched · **Date:** 2026-09-27

The landing, per the survey's contract (§7): async/await compiles and
runs — one bisectable commit, the M3 rejection gate replaced by the
weave, `suspend` scrubbed from the vocabulary, `Task` nowhere.

## 1. The weave (rut-lir)

An `async fn(cx: RunContext, ..)` compiles into ONE `FuncCode` — the
engine-woven `Future::yield` — over **existing ops only** (`Jmp`,
`GetF`/`SetF`, `Br`, `BrTable`, `EnumNew`, `CallI`, `Ret`, `ConstRaw`).
`lir/asyncfn.rs` is the weaver; `docs/async-survey.md` §2/§4 is the
contract it implements:

- **The frame cell.** A hidden `TyKind::Data` type per async fn
  (`#frame@<name>` — `#` is unspellable in source): `[0]`=state, `[1]`=
  cancelled, `[2]`=awaiter, `[3]`=pending, `[4..]`=locals. The type's
  field list grows as bindings compile (the `mk_data_inst` in-place
  patch law); `Op::NewCell` reads the final list at runtime, which is
  what makes recursive call sites correct without fixups.
- **The checkpoint enum.** One `TyKind::Enum` per fn (`#ckpt@<name>`),
  member index = resume state; the state field holds the member's
  immortal singleton (`Op::EnumNew`), and **null is the DONE/retired
  sentinel**. `Op::BrTable` dispatches on the member; a null slot names
  no member and falls to the default arm (the retire), so a re-drive of
  a finished frame answers silently.
- **Locals are cell-backed uniformly.** Every body binding goes through
  `bind_local` — a frame field plus a mirror `SetF` — and every
  assignment through `mirror_local`; each resume arm restores its
  in-scope set with `GetF`s. The park's `Ret` releases only staging
  registers; no liveness analysis decides which locals survive. The cx
  and the hidden frame edge are the two `NO_FIELD` locals — the driving
  loop re-mints the cx per drive, so the argv pair is always fresher
  than any mirror.
- **The await expansion** (`compile_await`): a probe (`GetF` state +
  `Br`; null = already done → straight to the resume arm), one cold-poll
  `CallI` through the awaited frame's `Future::yield` vtable row, a
  re-probe, then either the park (`SetF` the two wake edges, `EnumNew`
  the next state, `Ret`) or fallthrough into the resume arm (cancelled
  probe → pending-edge clear → restore → continuation). The drop path
  retires the state, clears the pending edge, and releases the
  ref-typed locals in binding order — the pending-drop drain is LIFO,
  so the `on_drop` callbacks fire in **reverse declaration order**
  (RFC 0016 §3).
- **The call site** (`compile_async_call`): mints the frame
  (`NewCell`), stores the user params, arms the entry state; the call's
  value IS the frame cell — typed as the hidden frame type in a bare
  context (await's provenance) or as the `Future<ret>` object when the
  context spells it (the generic launcher's unification reads the
  instantiation's args, `T := ret`).
- **cx members inline as field ops** (`compile_cx_member`): the cx
  record's one field is the frame edge; `checkpoint` reads the state,
  `cancelled` reads the flag. No calls, no vtable rows for the members —
  the frozen surface is the signature set (RFC 0012 §7).
- **The sleep future**: `ensure_sleep_future` mints the sleep frame
  (`#frame@sleep`, `ms` in its local field), a two-state checkpoint
  enum, the `Future<nil>` impl row, and one `FnKey::HostThunk` — a
  bodyless `FuncCode` whose `host_id` names the embedder's body. The
  await expansion's `CallI` routes host-bodied targets through
  `call_host` (the `Op::Call` law, now also `op_call_i`'s).

## 2. The driving loop (rut-vm)

The VM grows the survey's queues (`VecDeque`/`BTreeMap` — wasm-clean):

- `vm.launch` / `vm.cancel(task) -> bool` — the engine halves of
  launch/abort; each queue entry owns one reference, `cancel` answers
  false on a retired frame.
- `vm.drive(fut) -> Drive` — the re-entrant `yield` call (the
  `InterpCursor` stash/restore — the `call_raw` nested-entry machinery)
  with a freshly minted cx (`RunContext` found by name at `Vm::new`);
  the answer reads off the state field, and a completed frame
  re-enqueues its awaiter edge. The wake pair (awaiter edge + pending
  edge) is one-directional and cleared on resume and in the drop path —
  no cycle forms.
- `vm.arm_timer` / `vm.now_ms` / `vm.set_now` — the deterministic
  virtual clock (no wall time in the engine); `vm.next_deadline`
  expires due timers into the queue; `vm.run_ready` drains;
  `vm.pending_tasks` is the idle test.
- Fuel rides the per-op budget unchanged; a drive that exhausts it
  propagates `OutOfFuel` with the frame parked at pc — the checkpoint,
  not the op, is the async layer's resume granularity (disclosed v1
  semantics, RFC 0018 §4).
- `Op::BrTable` over a null slot names no member → the default arm
  (the one VM op-semantics touch: a null index register previously
  could not reach the op). `take_binding` now leaves its entry: one
  binding may back several `FuncCode`s (the decl row and the minted
  thunk are two bodies of the same host fn).

## 3. The host set (rut-std + rut/async_*)

Exactly the survey §3 recommendation: `rut/async_engine/engine.d.rut`
(the crossing rows: `__launch(f: any)`, `__abort(f: any) -> bool`,
`__sleep(ms: u32) -> any`, `__sleep_yield(f: any, cx: any)` — host
scope `async_engine`) + `rut/async_host/async_host.rut` (an inline
package, the ink law — spliced into consumers so the generic
`launch_future<T>` and the generic receipt class resolve at each call
site) + `rut_std::async_host::install_std_async` (the four bodies —
everything routes through the VM's driving API; the engine owns the
queues, the host owns only the crossing). `rut-driver::mount_std_async`
mounts both modules; the CLI and the wasm playground mount + install
(and run the driving loop after `main`). The typed arrow
`launch_future<T>(f: Future<T>) -> LaunchedFutureHandle<T>` lives in
rut code where the type system holds it — ruling 6's law is a compile
error, verified by test.

## 4. What the tests prove (`crates/rut-driver/tests/async_*.rs`)

Through the recording native (`rt.log` line comparison — the fmt
batch's pattern): trigger→completion; park/resume through `sleep` with
the virtual clock; nested awaits (a two-future wake chain); abort after
park → the resumed probe → the drop path, then a second abort answers
false; abort before the first drive never runs the body (the s0 probe);
`on_drop` locals at a checkpoint fire in reverse declaration order;
`cx.cancelled()` read from a body; fuel charged per drive step.
Diagnostics (`async_diag.rs`): await outside async; await on a
non-Future; await on a non-engine (user-impl) type with the v1
restriction message; await on the receipt (join lands with RFC 0019);
re-launching the receipt is a type error (ruling 6); `await select`
gated (parses, RFC 0019); the cx-first-parameter law; `RunContext` has
no other members; and the open-surface story — a USER launcher over the
same rows compiles clean, the engine's standard set untouched.

## 5. The disclosed deferrals and v1 edges

- **join** (`await` on a receipt), **`await select`** semantics,
  **structured scopes** — RFC 0019 (select parses, compile-gated).
- **v1 await targets engine-woven futures** — the probe reads the
  engine-reserved state field; a user `impl Future` is launcher-drivable
  (`vm.drive` finds its vtable row) but not awaitable. The survey §2's
  diagnostic, tested.
- **Trait-object-spelled await operands** (an annotated
  `let f: Future<nil> = work(..)`, or `sleep`'s surface) are trusted —
  the static type erases the concrete frame; the probe reads the field
  at runtime. The verifier accepts the driven-half field ops on
  trait-object registers (repr-ref law keeps RC correct; the field
  index is checked against the concrete record at runtime).
- **One future, one driver** — rut does not move; sharing a frame
  between `launch_future` and `await` is a misuse the type system does
  not see (the plan's own consume law, disclosed).
- **`cx.next_checkpoint(v)` from user bodies** diagnoses (a runtime
  `u32` cannot name an enum-member singleton without new vocabulary) —
  the weave writes the states it owns. RFC 0019 OQ-4.
- **Fuel mid-drive** parks at pc; a re-drive re-enters at the
  checkpoint's arm (idempotent probes, re-run continuations) — the
  checkpoint is the async resume granularity.
- **examples/04 tier 1 landed** (the vocabulary port:
  `impl Future<nil> for CustomFuture`, the user launcher, the audit);
  the tier-2 live drive harness is the first disclosed follow-up (the
  driver tests cover the same shape from Rust).
- **async methods** diagnose (the loop's tasks are async free fns in
  v1); generic async fns diagnose. The `TraitReq.is_async` check-level
  plumbing is untouched and tested by the corpus.

## 6. Gates

- `cargo test --workspace`: **874 passed / 0 failed** (baseline 857 +
  17 new: 8 runtime + 9 diagnostics).
- `cargo check -p rut-driver --target wasm32-unknown-unknown`: clean.
- **VERSION stays 13** (`binary.rs` untouched); the wire grammar grew
  zero rows — no new ops, `Nat`, `Intrinsic`, or `TyKind`.
- **vsix 0.2.5 untouched** — no LSP surface change (`await` already
  parsed; `Future`/`RunContext` arrive via core decls like `Iterator`).
- Foreign lanes never staged: `demo/*`, `AGENTS.md`, untracked
  `scripts/*` untouched.
- The lockstep gate (`core_surface.rs`) holds: `core.d.rut`'s two new
  `builtin trait` decls are exactly `Surface::core()`'s two new
  `NativeTrait` rows.

## 7. The RFC amendments (the §6 map, as landed)

- **0012 §7** — rewritten as LANDED: the frozen `Future`/`RunContext`
  block, the engine-vs-host split, no `Task` rows, "users may write
  their own launchers".
- **0018** — `git mv` to `0018-async-and-await.md`; title + vocabulary
  scrubbed of `suspend`; §2/§3 rewritten to the checkpoint desugaring
  (the `Await`-opcode/`CoroutineFrame` sketch superseded); §4 is the
  driving loop as landed; the host driving surface recorded; the v1
  edges disclosed.
- **0019** — rewritten Future-only: landed-with-0018 = nothing (join is
  here); the deferred remainder = join-on-handle, `await select`,
  structured scopes; the keep-parse + gate recorded; OQ-4 added.
- **0028** — the "when the async plan lands" row → `Future`/`RunContext`
  in core; launch/sleep as embedder surface.
- **0032** — §2 rewritten: the op set is untouched, **VERSION stays 13**.
- **0034** — the driving loop's queues/verbs as landed, fuel-per-drive,
  the "non-suspended frames" prose scrubbed.
- **0035 §4** — the loop's columns gain the task queue + the host-fn
  driving surface.
- **0041** — the examples/04 row: vocabulary port landed, harness
  follow-up.
