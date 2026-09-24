# The opaque-is law — the batch report

The record of the opaque-is-law batch: two phases, one shared tree,
base `2bee5f6`, all on `master`. Companion to
`docs/opaque-is-survey.md` (phase 0 — the census, the design; its
§-numbers are the ones the landed code, tests, and RFCs cite). The law
lives in the RFC 0014 amendment (cross-landed in RFC 0012 §3 and RFC
0015 §6, same commit as the code); the user-facing summary lives in
[`examples/README.md`](../examples/README.md) (the orphan-rule
section's opaque neighbor) and the demo classic
`demo/src/examples/opaque.rut` (still teaching, corrected rows). This
report is the record.

## 1. The story, in one section

The directive's case — `box1 is Point` on `opaque(Point { x: 1, y: 2 })`
— answered **TRUE** at base. Not hypothetically: it was a pinned,
green driver row (`e2e.rs` `case3_opaque` asserting
`"box1 is Point: true"`), with two more wrong rows pinned in the demo
(`playground.rs` `EXPECTED["opaque"]` and its twin in
`demo/src/examples/index.ts`). The runtime's `is` path inspected the
box's PAYLOAD: `effective_ty`'s `OpaqueBox` arm returned `val_ty`, so
`Op::IsType` compared the payload's TypeId and `Op::IsTrait` scanned
the payload's vtable (`box1 is Hashable: true`, probed live at base).
The batch landed the law in two commits:

1. **The census** (phase 0, `b696d41` — the survey). Empirically, at
   base, with five scratch probes: the case runs true/false; the
   checker path mapped (the single `Op::IsType` emitter,
   `expr.rs:261` — an opaque receiver on a concrete RHS emits UNFOLDED,
   while `is opaque` folds TRUE by NAME, breeding the alias wart:
   `box1 is BoxTy` answered false for the same type); the runtime path
   mapped (both engines share `effective_ty`); and the
   **design-governing find — the see-through is LOAD-BEARING**:
   `downcast<T>` is lowered through the very same read (`TidOf` →
   `effective_ty`, `call.rs:400`), so the fix could not touch
   `effective_ty`. Six one-line op-body edits pinned instead; VERSION
   called 10 → 11; the wrong answer's pins enumerated for the move.

2. **The law** (phase 1, `8e7ab51`). The six edits landed — the
   `IsType`/`IsTrait` op bodies in BOTH engines read `cell.ty`; the
   box answers BY the box. `effective_ty`/`TidOf` untouched and
   downcast proven recovering live (`recovered 1 2`); the alias wart
   repaired itself with zero checker code (`box1 is BoxTy: true` now);
   the full law table pinned as one test; the disclosed moves landed
   (§5); VERSION 11 with the ledger entry; the RFC amendments landed
   IN the same commit (the law and the tree land together — a tree
   whose code and RFC disagree between commits is worse than either).

3. **The record** (phase 2, this commit). The batch report; the RFC
   verification (§7); the README verification + the one missing link
   (§7.4). Docs only.

## 2. The load-bearing design — the decision record

The batch's one non-negotiable: **`effective_ty` and `TidOf` are
untouched, BECAUSE downcast recovers through them.**
`opaque.downcast<T>(o)` lowers through `emit_opaque_downcast`
(`call.rs:395–431`): `Op::TidOf` on the box → `Eq` against `want` →
the alias handoff or nil — and `TidOf` reads **`effective_ty`**, whose
`OpaqueBox` arm is precisely what makes the match hit. Touch that arm
and every recovery in the language collapses to nil while every gate
goes red. The census therefore split the see-through into its
consumers, and the fix took only the two that lie:

| consumer | reads | verdict as landed |
|---|---|---|
| `Op::TidOf` (downcast's engine) | payload tid (`val_ty`) | **legitimate — the recovery path itself; UNTOUCHED** |
| `Op::IsType` (`is X`) | now `cell.ty` | **the fix** |
| `Op::IsTrait` (`is I` on a box) | now `cell.ty` → the box's vtable | **the fix** |
| `op_call_i` dispatch | still `effective_ty` | unreachable for boxes (the checker rejects methods on `opaque`, probed at base) — defense only, left alone |

The fold-to-false question — "typeck may fold a statically-opaque
`is X` to false" — was REJECTED, and the rejection is the second half
of the decision record: a fold must special-case `TY_OPAQUE` or it
re-breeds the alias wart (the base wart was born from exactly a
name-keyed fold), and folding every opaque-receiver `is` would leave
`Op::IsType` with zero emitters, moving the law's enforcement from the
one runtime truth (`cell.ty`) into a second, parallel fold that can
drift. The runtime compare answers every spelling correctly for free —
and repairs the wart as a side effect. `is opaque` STAYS the typeck
fold TRUE (pinned, predates the batch).

## 3. The six edits, as landed

All in phase 1, surgical, nothing beyond them:

1. `crates/rut-vm/src/interp/step.rs` `Op::IsType`:
   `effective_ty(cell)` → `cell.ty`, with the law comment.
2. `crates/rut-vm/src/interp/step.rs` `Op::IsTrait`: the scalar-miss
   fallback → `cell_of(..).ty` — a box probes TY_OPAQUE's vtable,
   which has no impl rows, so `o is I` answers false by the ordinary
   registry scan, no special case.
3. `crates/rut-vm/src/interp/threaded.rs` `op_istype`: the same read
   (the `Machine` impl covers the threaded dispatch AND the wasm lane).
4. `crates/rut-vm/src/interp/threaded.rs` `op_istrait`: the same
   fallback.
5. `crates/rut-core/src/binary.rs`: `VERSION 11` + the v11 ledger
   entry (§6), the two driver version-pin tests moved with it.
6. The RFC amendments (§7) — the law and the tree in one commit.

Plus the pins (§4–§5); `host_boxes.rs` untouched and green — host
boxes already answered the law by `cell.ty` (RFC 0023).

## 4. The law table, as pinned

One test pins every cell — `crates/rut-cli/tests/e2e.rs`
`case3_opaque`, the directive's case verbatim extended; it fails if
ANY cell regresses:

| expression on an opaque box | pinned answer |
|---|---|
| `box1 is Point` (the payload's own type) | **false** |
| `box2 is Point` (a foreign payload type) | **false** — now by the law, not by luck |
| `box1 is opaque` | **true** — the one check that names what it is |
| `box1 is BoxTy` (the RFC 0043 alias spelling) | **true** — same type, same answer; the base alias wart repaired |
| `box1 is Hashable` (trait probe; the impl EXISTS) | **false** — the empty TY_OPAQUE vtable; the pin is strong because the impl is real |
| `opaque.downcast<Point>(box1)` | **recovers** — `recovered 1 2` |
| `opaque.downcast<Point>(box2)` | **nil** — `miss is nil: true` |

The host tier (RFC 0023) is unchanged by construction — it already
answered by `cell.ty` — and stays pinned by `host_boxes.rs`.

## 5. The disclosed moves

Every pinned row that relied on the wrong semantics, old values
verbatim (the survey §1.8 table, plus the copy it missed):

| pin | old row | landed as |
|---|---|---|
| `e2e.rs` `case3_opaque` | `"box1 is Point: true"` | `"box1 is Point: false"` (row moved; case extended per §4) |
| `playground.rs` `EXPECTED["opaque"]` | `"is str: true"` | `"is str: false"` |
| same table | `"3 boxes; first is Point: true"` | `"3 boxes; first is Point: false"` |
| `demo/src/examples/index.ts` expected | the same two rows | the same two answers (the demo smoke enforces its own copy) |
| `demo/src/cases.ts` inline "opaque + downcast" case | `"box1 is Point: true"` | `"box1 is Point: false"` — **the third expected-table copy the survey's §1.8 table missed** (the case3 twin the demo smoke verifies), disclosed in phase 1's commit body |
| `demo/src/examples/opaque.rut` `:3/:45/:90` | "`is` sees through the box" / "KEEPS the runtime type" teaching comments | re-worded to the law (`:3` "keeps the runtime type FOR downcast; `is` names the BOX, never the payload"; `:46`/`:91` likewise) — the classic KEEPS teaching with corrected rows, both expected-table copies moved together (the demo-no-sidecars lesson) |

The demo-no-sidecars lesson held: each copy is enforced by its own
gate (cargo for `playground.rs`, the demo smoke for `index.ts` +
`cases.ts`), so none could silently rot. Semantic changes with NO
pinned row, disclosed so nobody rediscovers them: the trait-probe flip
(`box is I` true → false — no test in the tree pinned one; grep-swept
at base; now pinned false going forward by §4's `Hashable` row) and
the alias repair (`box1 is BoxTy` false → true, an unpinned spelling
fixed in the open).

## 6. The VERSION ledger: 10 → 11

**The call: `VERSION 11`, landed WITH the fix — the byte and the law
in the same commit.** The tree's ledger shapes:

| Move | What | Shape |
|---|---|---|
| 5 → 6 | the `opt_prim_store` gate | rejection addition |
| 6 → 7 | `downcast` → `?T` | declared-surface change |
| 7 → 8 | StackTrace + func-table `pos` | format-affecting |
| 8 (stayed) | the entry-err widening | no-bump: old artifacts behavior-identical |
| 8 → 9 | the orphan rule | POLICY bump — "the version byte is the only provenance marker a `.rutc`/bundle carries about the law that produced it" |
| **10 → 11** | **the opaque-is law** | **POLICY bump, the v9 precedent — a fourth shape: neither rejection addition nor relaxation** |

**Why the op stream is byte-identical yet the answer changed:** the
fix is runtime-only — the compiler emits exactly the same `istype`
bytes before and after (the opaque receiver was never folded; that is
the §1.2 finding). The answer lives in the ENGINE's reading of the
bytes, not in the bytes: the same `istype` op answers true under the
v10 engine and false under v11. That is precisely why the v8 no-bump
shield — old artifacts are behavior-identical, bumping burns caches
for zero protection — FAILS here, and why the v9 principle governs
once it fails: the byte is what says which `is` law an artifact's
answers belong to. Honest costs and benefits, on record: a re-packed
bundle differs from its v10 twin only in the header u32; the bump's
COST is refusing pre-packed v10 `.rutbundle`s — a re-pack under the
law their bytes will actually run with, protection not waste — and its
BENEFIT is that no artifact straddles the two answer-laws silently in
either direction. Decode and verify are invariant: no new op, no new
type encoding, no declared-surface row.

## 7. The RFC verification (phase 2)

Phase 1 amended 0012/0014/0015 in its own commit; phase 2 verified
each reads complete and cross-linked, amending only what was missing.
No re-litigating.

### 7.1 RFC 0014 (`rfc/0014-any.md`) — COMPLETE

The revision note (Revised 2026-09, the opaque-is law) records the
reversal, the runtime-only mechanism, the VERSION 10 → 11 call, the
`TidOf` exception, and the non-fold reason. The Summary's erasure
sentence is scoped — "keeping the runtime type **for downcast only**
(`is` names the box, never the payload)". The `o is X` bullet is
reversed in full: concrete `T` AND trait `I` both miss, `o is opaque`
(including alias spellings) is the one true, recovery downcast-only,
the earlier "see through the box" reading explicitly retired, with the
cross-links to RFC 0012 §3 and RFC 0015 §6 in place. Verified — no
edit needed in phase 2.

### 7.2 RFC 0012 (`rfc/0012-traits-and-dispatch.md` §3) — COMPLETE

The keyword's bullet now delegates with the new law: "On an `Opaque`
receiver, `o is T` / `o is I` answer **by the box** (RFC 0014, the
2026-09 opaque-is law): both miss for every payload type — recovery is
`downcast<T>` only" — and carries the delegation's own reason sentence
(the concrete case deliberately NOT typeck-folded; the runtime compare
answers every spelling from the one runtime truth; a name-keyed fold
is exactly how the alias wart was born). The §3 static-folds clause is
untouched (why `is opaque` may stay a fold). Verified — no edit needed
in phase 2.

### 7.3 RFC 0015 (`rfc/0015-reified-types-and-layout.md` §6) — COMPLETE

The §6 touchpoint landed as designed: "A box's answering type is the
BOX: `is` reads the box cell's own type (`TY_OPAQUE` — no impl rows,
so every `o is X`, concrete or trait, misses); only `downcast`'s
`tidof` keeps reading the payload (the 2026-09 opaque-is law,
RFC 0014)." Cross-linked. Verified — no edit needed in phase 2.

### 7.4 The READMEs — one missing link amended

- **`examples/README.md`, the orphan-rule section's opaque neighbor**
  (added at the orphan-rule close-out, `139ee7d`): it stated the
  recovery half ("its cross-type law is RFC 0014's downcast, nil on a
  miss") but not the is-law. **Amended** (this commit, the batch's
  only phase-2 tree change beyond this report): the neighbor now
  states the new law AND the reason it follows from the section's own
  subject — a foreign trait can never be implemented for `opaque`, so
  a capability probe on a box finds nothing (`o is I` is `false`; `is`
  names the box, never the payload — RFC 0014). The orphan rule is
  why the trait probe answers false; the sentence now says so.
- **`demo/README.md`**: verified — it says nothing about `is`/opaque
  at all (the teaching lives in the classic itself,
  `demo/src/examples/opaque.rut`, corrected in phase 1), so nothing
  stale to fix.

A repo-wide sweep for stale see-through prose (`grep "sees through"`)
returns only the survey's census quotes and RFC 0014's own
"this reading is retired" note — the disclosures, not live claims.

## 8. The gates, as landed per phase

- **Phase 1** (on `8e7ab51`, recorded in its body): `cargo test
  --workspace` 770 passed / 0 failed; wasm32 check exit 0; benches —
  all 27 pinned checksums bit-identical across rut/qjs/node rows,
  `expected.json` untouched; corpus through the REBUILT wasm — 84
  files, 0 false diagnostics; demo smoke 273 passed / 0 failed (the
  corrected opaque rows verified through the rebuilt wasm); tsc clean;
  `npm run build` green; vsix re-issued same-version 0.2.2.
- **Phase 2** (this commit, docs only — the gates re-run green on the
  exact tree as the untouched-tree proof): `cargo test --workspace`
  770 passed / 0 failed; `cargo check --workspace --target
  wasm32-unknown-unknown` exit 0; benches re-run exit 0, the 27 pinned
  checksums bit-identical, `benches/expected.json` untouched (the
  paranoia pin); the workspace tree clean of everything but this
  phase's staged docs + the parallel session's foreign items (see §9).

## 9. The menu going forward

- **The trait-probe flip shipped with no tree pin — noted, now
  closed.** At base no test pinned `box is I` on an opaque (grep-swept,
  phase 0), so the true→false flip was a disclosed semantic change, not
  a checksum move. Phase 1's extension pins it false going forward —
  and the pin is strong: `Hashable` IS implemented for `Point`, so the
  false comes from the law, not from a missing impl.
- **The name-keyed `is opaque` fold could be re-keyed on the resolved
  type**, killing the wart CLASS at typeck rather than repairing the
  one wart at runtime. Rejected this batch (§2 — a fold orphans
  `Op::IsType` and risks drift from the one runtime truth); banked as
  the standing note. The runtime compare answers every spelling today.
- **`op_call_i`'s `effective_ty` read stays as defense** — unreachable
  for boxes (the checker rejects methods on `opaque`), harmless
  elsewhere, left alone deliberately.
- **RFC 0014's standing open questions** (OQ-1 content
  equality/hashing for opaque identity use; OQ-2 derived-atom caching)
  are untouched by this batch and stay open.
- **The queued demo-stacktrace-case batch** shares the expected tables
  this batch moved (`demo/src/examples/index.ts`,
  `crates/rut-cli/tests/playground.rs`); the interaction is textual
  adjacency only (survey §2.5) — whichever lands second rebases over
  moved adjacent lines. This batch landed first; the tables as of
  `8e7ab51` are the base.
