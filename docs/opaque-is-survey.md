# opaque-is — phase 0: survey

- **Batch:** opaque-is-law (phase 0 of 2, docs-only)
- **Base:** 2bee5f6 (`phase(2)`: the demo-journey-fixes close-out) — VERSION 10, the downcast law landed (refval-round2, `downcast<T> -> ?T`), the `is` operator live at checker + both engines
- **Date:** 2026-09-24
- **Directive (the law, verbatim case):**

  ```rut
  pub fn main() {
      let log = Logger.new("case");
      let box1 = opaque(Point { x: 1, y: 2 });
      let box2 = opaque("hello");
      log.info(f"box1 is Point: {box1 is Point}");
      log.info(f"box2 is Point: {box2 is Point}");
  ```

  *"They should be both false. That means opaque is just opaque if they
  not downcast."* An opaque answers `is X` FALSE for every payload type
  X; `is` never sees through the box; `downcast<T>() -> ?T` (RFC 0044)
  is the ONLY recovery. `box is opaque` (the one check that names what
  it is): pinned TRUE per the core surface's own typing, recorded
  either way.
- **Method note:** empirical. Every claim below was probed at base with
  the CLI (scratch under `/tmp/opencode/batch-opaque-is/p0/`, zero repo
  files touched — §Appendix), and both pinned gates that carry
  see-through rows were re-run green at base (`e2e case3` 3 passed,
  `playground` 1 passed) so the rows that move are proven live, not
  assumed. **The headline finding the probes forced:** the runtime `is`
  path DOES inspect the payload today (the bug), but the see-through
  read is LOAD-BEARING — `downcast<T>` is lowered through the very same
  read — so the phase-1 fix must land at the `IsType`/`IsTrait` op
  bodies and must NOT touch `effective_ty` (§1.5–§1.6, §2.2).

---

## 1. The census — what `is` does on an opaque operand TODAY

### 1.1 The directive's case, run at base (probe 1, §Appendix A)

```
$ rut run case.rut
box1 is Point: true      <- the violation: the runtime saw the payload
box2 is Point: false     <- right answer, wrong reason (str ≠ Point, not box law)
```

`box1 is Point` compiles, passes typeck **silently** (no reject, no
warn), and answers TRUE at runtime. The user's case is not hypothetical:
it is **already a pinned driver test** — `crates/rut-cli/tests/e2e.rs`
`case3_opaque` (the directive's case verbatim plus its downcast tail)
asserts `"box1 is Point: true"` at `e2e.rs:143`, green at base. The
wrong semantics ship as a teaching row.

### 1.2 The checker path — `crates/rut-lir/src/lir/expr.rs:203–263`

`ExprKind::Is` lowers in three arms (single emitter of `Op::IsType` in
the whole compiler — `expr.rs:261`):

1. **Trait RHS** (`:213–231`): receiver `TyKind::TraitObj` of the trait
   → fold; other TraitObj → `Op::IsTrait`; **receiver `TyKind::Opaque`
   → `Op::IsTrait`** (`:228–230`) — the capability probe is emitted
   THROUGH the box; concrete receiver → registry fold.
2. **`opaque` RHS** (`:233–249`): keyed on the NAME (`n == "opaque"` +
   the `extern_native_types` boot row) — receiver Opaque folds
   `ConstRaw 1` (**TRUE, already the law**), anything else folds
   `ConstRaw 0`. Never reaches the runtime.
3. **Concrete RHS** (`:251–262`): `want = resolve_type_now(ty)`; the
   by-value gate rejects prim RHS with the existing error — probe 3:
   `` `is` needs a concrete (cell) or trait type —`i32` is by-value ``
   (a COMPILE error, so `box is i32` never reaches the runtime); then
   **receiver `TyKind::Opaque` → emit `Op::IsType { dst, obj, want }`
   — NO fold; the runtime decides** (`:259–261`); non-opaque receiver →
   static fold `rt == want`.

So the typing rule that answers `box1 is Point` today is "concrete RHS,
cell-typed receiver, runtime TypeId compare" — with the runtime's
compare keyed on a type it has no business reading (§1.3). The IR
 receipt (probe 1b, §Appendix A): both `is Point` tests survive as two
live `istype` ops; `box1 is opaque` is `const 1` — the fold, never
emitted to the VM.

### 1.3 The runtime path — the VM DOES inspect the payload

Both engines share the read; neither has its own opinion:

- `crates/rut-vm/src/interp/ops.rs:15–23` — `effective_ty(cell)`:

  ```rust
  match &cell.data {
      CellData::OpaqueBox { val_ty, .. } => *val_ty,   // <- THE SEE-THROUGH
      // a host payload box's rut type is the box itself (RFC 0023):
      // `o is opaque` is true, `o is T` misses for every rut T
      CellData::HostBoxed { .. } => cell.ty,           // <- already the law
      _ => cell.ty,
  }
  ```

  The heap mints BOTH box tiers with the cell's own type `TY_OPAQUE`
  (`alloc_opaque`, `crates/rut-vm/src/heap/mod.rs:390–393`;
  `alloc_host_box` right below) — the payload's type lives ONLY in
  `val_ty`. The `HostBoxed` arm's own comment states the law; the
  `OpaqueBox` arm contradicts it one line above. Same surface, two
  answers.
- `Op::IsType` bodies — `step.rs:247–251` and its threaded twin
  `threaded.rs:720–727` — `let ty = self.effective_ty(cell);
  Slot::bool(ty == want)`. For a rut-minted box this compares the
  PAYLOAD's tid: `box1 is Point` → `TY_POINT == TY_POINT` → true.
- `Op::IsTrait` bodies — `step.rs:253–270`, `threaded.rs:729+` — the
  scalar-receiver miss falls back to `effective_ty`, so a trait probe
  on a box scans the PAYLOAD's vtable. Probe 2:
  `box1 is Hashable: true` (Point's impl found through the box).

### 1.4 What `is opaque` answers today — TRUE, with one wart

- `box1 is opaque: true` (probe 2) — the typeck fold (`§1.2` arm 2).
  The pinned answer already holds; no code change belongs to it.
- `point is opaque: false` — the ordinary concrete fold. Correct.
- **The wart — the alias spelling diverges** (probe 2): with
  `type BoxTy = opaque` (RFC 0043), `box1 is BoxTy: false` TODAY. The
  name-keyed fold misses the alias, so the RHS falls to the concrete
  arm → `IsType { want: TY_OPAQUE }` → the runtime compares the payload
  tid against `TY_OPAQUE` → false. Same type, two spellings, two
  answers. (§2.2: the phase-1 runtime fix repairs this for free.)

### 1.5 The downcast coupling — the see-through is LOAD-BEARING

`opaque.downcast<T>(o)` lowers through
`emit_opaque_downcast` (`crates/rut-lir/src/lir/call.rs:395–431`):
`Op::TidOf` on the box (`:400`) → `Eq` against `want` → the ALIAS
handoff (`MovRef`, the box IS the `?T`) or nil. And `Op::TidOf`
(`step.rs:239–246`, `threaded.rs:705–718`) special-cases `HostBoxed`
to the `HOST_BOX_TID` sentinel and then calls **`effective_ty`** — for
a rut box that is the payload tid. **That read IS the match test.**
`effective_ty`'s `OpaqueBox` arm is what makes `downcast<Point>(box1)`
hit. Change that arm and every recovery in the language collapses to
nil while every gate goes red. The census therefore splits the
see-through into its two consumers:

| consumer | reads | verdict |
|---|---|---|
| `Op::TidOf` (downcast's engine) | payload tid (`val_ty`) | **legitimate — the recovery path itself; untouched** |
| `Op::IsType` (`is X`) | payload tid | **the bug — this phase's target** |
| `Op::IsTrait` (`is I` on a box) | payload's vtable | **the bug — same phase** |
| `op_call_i` dispatch (`ops.rs:341–347`) | payload tid | unreachable for boxes: the checker rejects methods on `opaque` — probe 4: `` `opaque` has no methods in this build —recover with `opaque.downcast<T>(o)` (RFC 0014) `` — defense only, left alone |

### 1.6 Blast radius of the wrong read (what phase 1 may and may not touch)

- `TidOf`: NO compiler emitter except downcast (`grep TidOf
  crates/rut-lir/` → `call.rs:400` only); no test pins `tid(box)`.
  Untouched.
- Host boxes: every row already law-shaped (`effective_ty` →
  `cell.ty`); `host_boxes.rs` (`c is opaque` true, downcast nil)
  moves NOTHING.
- The `?T` alias handoff (`ops.rs:196–203`, `threaded.rs:447–450`) and
  `Op::Unbox` (compiler-guarded safety net) never consult
  `effective_ty`. Untouched.

### 1.7 Where the `is` law lives in the RFCs

- **`rfc/0014-any.md:73–78` — the law sentence itself**: "`o is T` /
  `o is I` **see through the box** (the boxed value's type): concrete
  `T` is the exact-type test, a trait RHS is the capability probe …".
  The directive's law REVERSES this sentence — this is a law change
  against RFC 0014's current text, not a bug against it (the summary's
  "forgetting the static type and **keeping the runtime type**",
  `:27–32`, is the same see-through stance in compressed form; phase 1
  re-words both to "recoverable only through `downcast`").
- **`rfc/0012-traits-and-dispatch.md` §3 (`:176–178`)** — the keyword's
  home: "`x is Opaque` is legal … On an `Opaque` receiver, `o is T` /
  `o is I` **see through the box** … (RFC 0014)" — delegates to 0014;
  the delegation flips with it. The §3 static-folds clause (`:180–182`)
  is why `is opaque` may stay a fold.
- **`rfc/0015-reified-types-and-layout.md` §6 (`:121+`)** — the
  internals the answer comes from (exact TypeId compare); a one-line
  note that a box's answering type is the box is optional there.
- **`rfc/0001-rut-overview.md:247`** — generic mention, no edit.
- The recovery half of the law is already landed and owned by the
  refval-round2 amendment (`rfc/0014:197+`) + RFC 0044 — phase 1 only
  makes `is` stop competing with it.

### 1.8 The pins that relied on the wrong semantics (checksums that move)

Re-run green at base, rows enumerated, old values recorded for the
disclosure:

| pin | row today | becomes |
|---|---|---|
| `crates/rut-cli/tests/e2e.rs:143` (`case3_opaque`) | `"box1 is Point: true"` | `"box1 is Point: false"` |
| same line | `"box2 is Point: false"` | unchanged (reason changes, byte value doesn't) |
| `crates/rut-cli/tests/playground.rs:55` (`EXPECTED["opaque"]`) | `"is str: true"` | `"is str: false"` |
| `crates/rut-cli/tests/playground.rs:61` | `"3 boxes; first is Point: true"` | `"3 boxes; first is Point: false"` |
| `demo/src/examples/index.ts` ("opaque" case `expected`) | same two rows (the demo smoke's copy) | same two answers |
| `demo/src/examples/opaque.rut:3,45,90` | "`is` sees through the box" teaching comments | re-worded (§2.5) |

Semantic changes with NO pinned row (disclosed so nobody rediscovers
them): `box is I` trait probes through the box flip true → false
(no test in the tree pins one — grep swept `rut-cli`,
`rut-driver`, `rut-vm`, `rut-lsp` tests); the alias repair
(`box1 is BoxTy` false → true) is a fix on an unpinned spelling.

---

## 2. The design (pinned)

### 2.1 The law, as one table

| expression on an opaque box | pinned answer |
|---|---|
| `o is X` — X concrete, X ≠ opaque | **FALSE**, always, every payload type |
| `o is opaque` | **TRUE** — the one check that names what it is |
| `o is <alias-of-opaque>` (RFC 0043) | **TRUE** — same type, same answer as the spelling |
| `o is I` — I trait | **FALSE** — the box dispatches nothing; recover first |
| host payload box (RFC 0023) | unchanged — already this law (`is opaque` true, `is T` misses) |
| recovery | `downcast<T>() -> ?T` ONLY (its `TidOf` read untouched) |

### 2.2 The fix: the op bodies, NOT `effective_ty` (six one-line edits)

The law's enforcement point is where the answer is produced — the
`IsType`/`IsTrait` op bodies in BOTH engines. For every cell,
`effective_ty` equals `cell.ty` EXCEPT the box see-through — so the
honest form of the type test is simply the cell's own type:

- `step.rs:247–251` + `threaded.rs:720–727` (`IsType`):
  `let ty = cell.ty;` (with the law comment: "`is` names the box, never
  the payload — `o is X` misses for every payload X, `o is opaque`
  (or an alias) hits; recovery is `downcast<T>` only (its own `TidOf`
  keeps reading the payload — RFC 0014)"). Concretely: drop the
  `effective_ty` call; compare `cell.ty == want`.
- `step.rs:253–270` + `threaded.rs:729+` (`IsTrait`): the scalar-miss
  fallback reads `cell.ty` instead of `effective_ty(cell)` — a box then
  probes `TY_OPAQUE`'s vtable, which has no rows (core registers no
  impls for the primitive; the orphan rule, RFC 0012 §2a, bars user
  impls), so the probe answers false by the ordinary registry scan —
  no special case, the machinery just stops lying about which type is
  probed.
- **`effective_ty` itself: UNTOUCHED.** Its remaining callers are
  exactly the legitimate readers — `TidOf` (downcast's match test,
  §1.5) and `op_call_i` (unreachable for boxes, §1.4/§1.6). The
  `HostBoxed` sentinel in `TidOf` stays.

Consequences, checked against the probes:

- `box1 is Point` → `TY_OPAQUE == TY_POINT` → **false** (probe 1 flips).
- `box2 is Point` → **false** (now by the law, not by luck).
- `box1 is opaque` → **true** (unchanged — the typeck fold never
  reaches the VM).
- `box1 is BoxTy` → `IsType{want: TY_OPAQUE}` → `TY_OPAQUE == TY_OPAQUE`
  → **true** — the §1.4 alias inconsistency REPAIRS ITSELF with zero
  checker code. This is the argument that settles the fold question.
- `box1 is Hashable` → **false** (no `TY_OPAQUE` vtable rows).
- `box3 is str` → **false** (the demo row flips).
- Host boxes: byte-identical behavior (they already answered by
  `cell.ty`).

### 2.3 The fold-to-false question — REJECTED for the checker

"typeck may fold a statically-opaque `is X` to false if that is the
honest lowering" — it is NOT the chosen lowering, for three recorded
reasons:

1. **The alias hazard.** A fold must distinguish `want == TY_OPAQUE`
   (true — and today's fold is keyed on the NAME `opaque`, which is
   exactly how the §1.4 wart was born). Getting it right means
   resolving the RHS and special-casing `TY_OPAQUE` — checker code that
   duplicates what `cell.ty == want` already answers correctly for
   every spelling.
2. **Orphaning the op.** The opaque receiver is the ONLY emitter of
   `Op::IsType` (`§1.2`); folding all of them leaves the op with zero
   emitters and moves the law's enforcement from the one runtime truth
   (`cell.ty`, RFC 0012 §3's own "one runtime truth per value") into a
   second, parallel fold that can drift.
3. **Minimal diff.** Six one-line edits in two files (§2.2) versus a
   checker rewrite plus a new diagnostic surface.

RFC 0012 §3's static-folds clause is satisfied in spirit and phase 1
says so in the RFC edit: the receiver's static type DOES answer, but
the answer is left to the runtime compare so the alias spelling cannot
diverge from the primitive spelling (the exact failure mode §1.4
caught). The existing `is opaque` fold STAYS (it is the pinned TRUE and
it predates this batch).

### 2.4 The trait probe disposition

`o is I` on a box: false, by the ordinary `IsTrait` machinery probing
the box (§2.2). RFC 0014's capability-probe-through-the-box sentence
dies with the see-through law — no separate mechanism, no new op, and
the pedagogy is the RFC's own: capability is a property of the VALUE's
type, and the box's value IS the box until `downcast`. The checker arm
that emits `IsTrait` for opaque receivers (`expr.rs:228–230`) stays as
written — it becomes honest without an edit.

### 2.5 The regression pin's home (driver suite + the demo)

- **The driver suite** (`crates/rut-cli/tests/e2e.rs`) is the pin:
  `case3_opaque` is the directive's case verbatim. Phase 1 moves its
  `:143` row and EXTENDS the case with the full §2.1 table as new
  assertions: `is opaque` true, `is <alias>` true, `is Hashable` false,
  `is Point` false, downcast still recovers — one test that fails if
  ANY cell of the law regresses. (`host_boxes.rs` already pins the host
  tier; untouched.)
- **The demo classic KEEPS teaching it** — the demo teaches TODAY and
  this IS today's law. `demo/src/examples/opaque.rut` stays, its two
  live rows move with the expected tables (`playground.rs` + `index.ts`
  — each gate enforces its own copy, the demo-no-sidecars lesson), and
  its teaching comments flip with it: `:45`'s "`is` sees through the
  box" becomes the honest law (`is` names the box; downcast recovers),
  `:3`'s "KEEPS the runtime type" re-worded to "keeps the runtime type
  FOR downcast", `:90` likewise. The example already teaches the
  recovery in the same file — `is` misses, `downcast` hits — which is
  exactly the law; no new case is needed.
- **The queued demo-stacktrace-case batch — the one coordination
  point.** Both batches edit `demo/src/examples/index.ts` and
  `crates/rut-cli/tests/playground.rs` expected tables. The interaction
  is textual (adjacent rows in shared tables), never semantic:
  SEQUENCE the two phases, don't parallelize them; whichever lands
  second rebases over moved ADJACENT lines. The smoke's §5 scan pin (35
  files under `demo/src`) is untouched by THIS batch — `opaque.rut` is
  edited in place, no file added — so any re-pin belongs to the
  stacktrace batch alone.

### 2.6 The RFC edits (same commit as the code — the law and the tree land together)

- `rfc/0014-any.md`: the `:73–78` see-through bullet reversed (misses
  for every rut T, concrete AND trait; `o is opaque` the one true;
  recovery downcast-only); the summary's "keeping the runtime type"
  (`:29`) scoped to downcast; a revision note dated 2026-09 (the
  opaque-is batch) recording the reversal and the downcast coupling
  (`TidOf` keeps the payload read — the ONE legitimate see-through).
- `rfc/0012-traits-and-dispatch.md` §3: the `:176–178` bullet's
  see-through sentence replaced (opaque receivers answer by the box;
  the concrete-`is X` case deliberately NOT typeck-folded — §2.3's
  alias reason, one sentence); the static-folds clause untouched.
- `rfc/0015-…` §6: optional one-liner (a box's answering type is the
  box). `rfc/0044` untouched (the recovery law is already correct).

### 2.7 The full phase-1 edit list (compressed)

1. `step.rs` + `threaded.rs`: `IsType`/`IsTrait` read `cell.ty`
   (§2.2) + law comments.
2. `binary.rs`: `VERSION 11` + the v11 ledger entry (§3).
3. `e2e.rs` `case3_opaque`: row move + the full-law extension (§2.5).
4. `playground.rs` `EXPECTED["opaque"]` + `index.ts` expected: the two
   rows (§1.8's table, old values disclosed in the commit body).
5. `opaque.rut`: the three teaching comments (§2.5).
6. The RFC edits (§2.6).
7. Gates re-run on the exact tree: `cargo test --workspace`, the demo
   smoke (273 — row VALUES move, row COUNT doesn't), `tsc --noEmit`,
   `npm run build`, wasm32 check.

## 3. The VERSION call: 10 → 11

**The call: `VERSION 11`, landing WITH phase 1's fix — the byte and
the law move in the same commit.** The tree's own ledger
(`binary.rs:409–456`) has three shapes:

| Move | What | Why |
|---|---|---|
| 5 → 6 | the `opt_prim_store` gate | a **rejection addition** — accepting fewer sources |
| 6 → 7 | `downcast` → `?T` | declared-surface change |
| 7 → 8 | StackTrace + func-table `pos` | format-affecting |
| 8 (stayed) | the entry-err widening | a check relaxation — "old artifacts are **bit-identical in behavior**"; bumping "burn[s] every cached v8 artifact for zero protection" |
| 8 → 9 | the orphan rule | rejection addition; "the version byte is the only provenance marker a `.rutc`/bundle carries about the law that produced it" |

This batch is a **fourth shape** — neither a rejection addition (the
checker is untouched; no source is refused) nor a relaxation (old
artifacts are NOT behavior-identical: the same `istype` bytes answer
differently under the new engine). The v8 ride's own no-bump condition
— behavior-identical old artifacts — explicitly FAILS here, and once
it fails, the v9 principle governs: the byte is what says which `is`
law an artifact's answers belong to. Honest disclosures, so nobody
overclaims: the compiler's op stream is byte-identical before/after
(runtime-only fix), so a re-packed bundle differs from the old one only
in the header u32 — the bump's COST is refusing pre-packed v10
`.rutbundle`s (forcing a re-pack under the law their bytes will
actually run with: protection, not waste — the refused bundle's
documented `is` behavior is precisely what died), and its BENEFIT is
that no artifact ever straddles the two answer-laws silently in either
direction. Decode and verify are invariant: no new op, no new type
encoding, no declared-surface row — the bump is policy per the v9
precedent, not format necessity.

## 4. The phase order — confirmed

- **Phase 0 — this survey** (docs only). Done.
- **Phase 1 — the law lands**: the six op-body edits (§2.2), `VERSION
  11` (§3), the pin move + full-law extension (§2.5), the demo rows +
  comments (§2.5), the RFC edits (§2.6), all gates re-run.

The split is right, for the standing reason: **the law is one unit** —
the runtime fix without the pin move is an unexplained checksum break;
the pin move without the fix is a lie; the version byte without both is
a legacy marker for a law that never landed. A separate docs phase for
the RFC edits was considered and REJECTED: the RFC sentences are the
law statement, and a tree whose code and RFC disagree between commits
is worse than either alone — the whole change is one phase's size.

---

## Appendix — the empirical log (phase 0)

Scratch: `/tmp/opencode/batch-opaque-is/p0/` (5 probes; the CLI binary
pre-built at base, zero repo files touched; gates cited from the two
pinned test binaries re-run at base).

- **A1 — the directive's case** (`case.rut`: the verbatim body +
  `struct Point { x: i32; y: i32 }` + `use ink::{ Logger }`, which the
  e2e harness appends but a raw file needs):
  `box1 is Point: true` / `box2 is Point: false`.
- **A2 — the pinned law surface** (`probe2.rut`, trait + alias):
  `box1 is opaque: true`, `point is opaque: false`,
  `box1 is Hashable: true` (see-through probe),
  `point is Hashable: true`,
  `box1 is BoxTy: false` (the alias wart), `p is BoxTy: false`.
- **A3 — the by-value gate + the demo row** (`probe3.rut`):
  `box1 is i32` is a COMPILE error (`` `is` needs a concrete (cell) or
  trait type —`i32` is by-value ``); `box3 is str: true`.
- **A4 — dispatch through the box is compiler-rejected**
  (`probe4.rut`): `` `opaque` has no methods in this build —recover
  with `opaque.downcast<T>(o)` (RFC 0014) `` — `op_call_i`'s
  `effective_ty` use is dead for boxes.
- **A5 — the IR receipt** (`probe_ir.rut` via `rut dump`): the two
  `is Point` tests emit live `istype r12, r6, t18` / `istype r9, r8,
  t18`; `is opaque` emits `const r11, 1` (the typeck fold — the op
  never runs). Proves: no fold today for concrete X, fold today for
  the primitive name.
- **A6 — the pins re-run at base**: `cargo test -p rut-cli --test e2e
  case3` → 3 passed (the `"box1 is Point: true"` row green TODAY);
  `cargo test -p rut-cli --test playground` → 1 passed (both
  `"is str: true"` / `"3 boxes; first is Point: true"` rows green
  TODAY). The rows that move are proven live.
- **Method deviations, disclosed**: the directive's snippet lacks the
  struct + `use` preamble (added in A1, matching `case3_opaque`);
  `rut dump` does not mount the std tree, so A5 uses a Logger-free
  variant; the full gate battery was NOT re-run for phase 0 (docs-only
  phase — the tree is byte-identical to base outside this file, and
  the two gates that carry the moving rows were re-run green
  individually).
