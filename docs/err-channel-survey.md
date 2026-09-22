# err-channel — phase 0 survey: the MakeRecord census, the IR feasibility call, StackTrace, the entry-err contract, the bench

*Phase 0 of the err-channel batch (docs only). Base `4ceb17e`, VERSION 7
(downcast → `?T` landed). Scratch artifacts: `/tmp/opencode/batch-err-channel/p0/`
(the census tool, the probe program, the full IR dumps). Every number below
is reproducible from those artifacts.*

---

## 1. The MakeRecord census

### 1.1 The five source-level mint sites (definitive)

All record minting funnels through one op, `Op::MakeRecord { dst, ty,
argv_off, argc }` (`crates/rut-core/src/ops.rs:195`, opcode 49 in
`binary.rs`). It is emitted from exactly five lowering sites:

| site | lowers | tuple-shaped? |
|---|---|---|
| `rut-lir/src/lir/intrinsic.rs:380` | `checked_add`/`checked_sub`/`checked_mul` → the `(value, ok)` pair (the v1.1 convention, RFC 0005 §10) | yes |
| `rut-lir/src/lir/expr.rs:99` | the `(a, b, ..)` tuple literal — which includes every literally-written `(T, err)` result | yes (n-field) |
| `rut-lir/src/lir/lit.rs:250` | struct/class literals (`Point { .. }`) | no |
| `rut-lir/src/lir/lit.rs:275` | `zero_value` of a record-typed slot | no |
| `rut-lir/src/lir/lit.rs:341` | used-struct literals (RFC 0035 §1) | no |

Tuples ARE `TyKind::Data` (numeric fields) — there is no separate tuple
op; the `(T, err)` economy and the struct economy share the same mint.

### 1.2 The op's cost anatomy (why the mint taxes hot paths)

`op_make_record` (`rut-vm/src/interp/ops.rs:300`): one heap allocation
(`alloc_record_zeroed`), a retain loop over the type's ref-field list,
the field writes, and the dst register's old-value release. For a
`(i64, bool)` pair that is an allocation + two field writes + an rc
release for a value that often dies a few ops later — the ~+175 ns/get
the refcolumn experiment attributed to the read-back spelling (benches
README, refval round 1; the downcast package's dominant unpriced term),
of which phase 1 of that batch bought back ~88 ns by deleting the
`(V, bool)` mint at downcast (VERSION 7). The remaining mint tax lives
in `checked_*` and in every user-written `(T, err)` return.

### 1.3 What SROA already banks — and the hole it leaves

`rut-lir/src/lir/sroa.rs` scalar-replaces a `MakeRecord` whose register
is read **only by `GetF` of non-ref fields**, with explicit
disqualifiers: anything that observes identity (call argument, `RefEq`,
`is`, `TidOf`, `opaque(..)`, a store, **a return**), a ref-typed field
read, or a redefined field register. It runs per function after LIR
emission (`lir/mod.rs:473`), to a fixed point; the inliner has already
run at the checker level, so hot callee/caller pairs are ONE function
body by then.

**The empirical result** (census tool over the post-optimization IR of
39 units: all 27 bench workloads, the `rut/` pkgs, the examples): across
**668 surviving mints**, the `getf-only` class — the one SROA exists for
— is **empty in real code**. Every record literal that is genuinely
local has already been scalar-replaced. What survives is precisely the
hole in SROA's disqualification list: the **return channel**.

### 1.4 The surviving mints, classed

Post-SROA survivors, by field-count and use shape (the census follows
actual use chains — single-use `mov`/`movref` chains to their terminal
`ret`/`getf`/call; multi-write join slots resolved by their post-join
reads):

| class | count | share | reading |
|---|---|---|---|
| pair (2-field), return-family | **459** | **68.7%** | returned directly (105) or through the ret-slot handoff (354) |
| 3+-field record, return-family | ~100 | ~15% | same shapes for aggregate results |
| 3+-field record, stored | 107 | 16.0% | container nodes / Vec cells — genuinely escape (e.g. binary-trees' nodes, pouch `Vec` internals) |
| boxed `?T` payload | 9 | 1.3% | `MakeOpt` box constructions |

Two units own 90% of everything: **digest.rut (338 mints)** and the
**json-decode workload (262 mints)** — both are `(T, str)`-returning
parser programs. That is the honest shape of the survey: the `(T, err)`
convention is not a corner of the corpus, it is the dominant surviving
mint economy, and it is exactly the class phase 1 targets.

Classification caveat, disclosed: the IR is not SSA — hot join slots
are reused temps (`json_dec`'s `r54` is an arrget target, an eq operand,
and a pair handoff), so per-mint classes inside "return-family" are
resolved by chain-walking with sampled manual verification (the probe
trace below is a full manual read). The class boundaries are honest;
the ±few% inside the return family is not the design driver — the 69%
pair share and the empty `getf-only` class are.

### 1.5 The hot shape, traced

`/tmp/opencode/batch-err-channel/p0/probe.rut` compiles to one inlined
function whose every tuple rides the same three-step sequence:

```text
10 makerecord r13, t17, [r5, r12]   ; the callee's (i64, bool) mint
11 movref    r3, r13                ; hand the record to the ret slot
13 getf      r14, r3, f0            ; caller destructures field 0
14 getf      r16, r3, f1            ;   ... and field 1
```

This is the pattern in `checked_*` returns, in digest's parser returns,
in every `let (a, b) = f(..)`. The mint, the handoff, and the two field
loads exist only to move two already-computed registers across the
return boundary. **Answer to the census's driving question: the
overwhelming majority of surviving mints — ~69% by count, ~83% counting
the 3+-field return-family records — die to return-position fusion.**
The stored minority (container nodes) is real escape and correctly
stays a heap allocation.

---

## 2. IR feasibility — the call: a lowering pass, not a pair-return IR shape

### 2.1 What typed bytecode (RFC 0032) supports today

- Registers are typed (`u16` indices, static types in the function
  signature, load-verified, RFC 0032 header); **prim values already ride
  raw in slots** — no cell, no boxing. The PrimMap u64 column is the
  host-side existence proof (`rut/nmapset/nmapset.rut`: "VALUES now — a
  raw u64 column relocated with the keys inside the [host map]"); the
  VM-side proof is `Op::ConstRaw`/`mov` on typed prim registers and the
  slot ABI itself (RFC 0015 §5).
- Returns are **single-register**: `Op::Ret { val: Option<Reg> }`
  (`ops.rs`), calls take one `dst`. There is **no multi-value return**
  anywhere in LIR, the binary format (v2, RFC 0033), or the verifier.
- `Op` is **pinned at 24 bytes** (RFC 0032 §1.0: measured +13% intloop
  cost at 16, aliasing rationale, `Op::Pad` holds the pin). Binary
  format v2 and the three interpreters (`step`, `run`, `threaded`) all
  assume the current op set.

### 2.2 The two candidate shapes

**A. Return-position destructure elision (a lowering pass).** After
inlining, the callee's mint → ret-slot `movref` → caller `getf` chain is
one function's op stream. Extend the SROA machinery (it already has the
def/use maps, the jump-target guards, the register-remap and pool
re-intern) with a pass that treats *mint → single-use mov/movref into a
slot whose only reads are `getf`* as consumable: the components pass
through their existing registers, the mint and the reads die. Prim pairs
unbox entirely (no allocation, no rc traffic); a ref-typed component
costs one `MovRef` retain at the handoff instead of the record cell +
its ref-field retains. No op change, no format change, no verify change,
no interpreter change. The pipeline slot is ready-made: `lir/mod.rs:473`
already runs SROA → peephole after emission.

**B. A pair-return IR shape** (`ret2`, `call { dst0, dst1 }`, or
tuple-registers): touches the 24-byte op pin (the pinned `Op::Pad`
rationale), binary format v2 (RFC 0033), the verifier, all three
interpreters, and wasm32 — for a benefit pass A already delivers
post-inline, where every hot pair lives anyway.

### 2.3 The feasibility call

**A.** The reasons, in order of weight: (1) the census shows the pairs
live post-inline — one function, one op stream, so no cross-function
contract is needed; (2) the 24-byte op pin and format v2 stability are
pinned assets this batch has no reason to spend; (3) the PrimMap u64
column proves the engine's slot discipline already carries prims raw —
pass A merely stops re-boxing what the slots never needed boxed. The
fusion design follows: **a lowering pass in `rut-lir` extending the SROA
family, not an IR change.** Phase 1's gates (checksums bit-identical,
fuel re-pins disclosed) are exactly the gates a pure-lowering pass can
meet.

---

## 3. The StackTrace surface — `builtin class`, decided against the fields

### 3.1 The ruling, restated against RFC 0036's current text

RFC 0036 (and RFC 0028 §`debug`) spell the capture as a **host fn**
`capture_stack_trace() -> Opaque` with a rut-side wrapper class in the
`debug` package's rut source (`pub class StackTrace { h: Opaque; .. }`).
The user ruling supersedes: **`capture_stacktrace() -> StackTrace` is a
`builtin class`** — the engine itself implements it, compiler-lowered,
declared in the toolchain's decl file only. That is RFC 0025's
`builtin class` row exactly: "the engine itself — compiler-lowered;
nothing to bind; the decl is a pure signature contract." A `builtin
primitive` stays wrong for this (primitives are value types —
`str`/`bytes`/`opaque`; a trace is a stateful engine snapshot with
methods, not a value spelling). The consequences of `builtin class`
over the RFC 0036 wrapper: no Opaque-box indirection, no `debug` host
bodies to register (wasm32 gets it for free — there are no host fns to
bind), member dispatch through the engine's member contract, and a
VERSION bump per the surface-change precedent (7 → 8, phase 2's work).

Naming note, disclosed: RFC 0036/0028 spell `capture_stack_trace`; the
batch ruling (and this design) spell `capture_stacktrace`. Phase 4's
RFC 0036 amendment must reconcile to one spelling — recommend the
ruling's.

### 3.2 The member contract

```rut
builtin class StackTrace {
    fn len(self) -> i32;                 // frame count (innermost first)
    fn name(self, i: i32) -> str;        // frame i's function name
    fn line(self, i: i32) -> i32;        // call-site line (0 when stripped)
    fn col(self, i: i32) -> i32;         // call-site column (0 when stripped)
    fn render(self) -> str;              // the RFC 0036 symbolication string
}
```

Decided **against fields** (e.g. `frames: Vec<Frame>`): a frame array
would mint rut-side data (a Vec of records per access — re-entering the
exact tuple-mint economy phase 1 is deleting), fix the representation
into the surface, and pay for symbolication eagerly. Members keep the
representation engine-side: **capture stores raw frames only; every
member symbolicates lazily, per index, on demand** through the loaded
binaries' `SymbolTable` (RFC 0036 §4: `FuncSym.name` +
pc→`(line, col)` span intervals), degrading to `0`/pc-only text when
stripped — RFC 0036's three restoration paths (in-VM, `.rutc.map`
sidecar, recompile-by-determinism) apply unchanged. `render()` reuses
the same walk as `vm.symbolicate`'s formatter. Out-of-range `i` traps
(the index is a bug, not data — the loud-boundary convention).

### 3.3 The frame data at the VM boundary

Capture is RFC 0036 §2 verbatim — `RawTrace { frames }` of
`Rut { module, func, pc, state }` / `Native { module, slot }` markers:
a walk of `vm.frames`, no name lookup, no source access, innermost
first, native boundary markers included (RFC 0034's `callnat` markers),
async resume states preserved. Target-independent by construction — the
frame walk reads VM structures, so wasm32 needs no separate path (phase
2 pins it explicitly). `Trap` already carries its unwind capture; the
new surface is the *opt-in rut-side* snapshot.

### 3.4 The cost model

- **Capture: depth-proportional, pay-per-capture.** Nothing is captured
  unless rut code calls `capture_stacktrace()`. The walk is O(depth)
  raw-frame pushes; no symbolication at capture.
- **Members: pay-per-access, lazy.** `name(i)`/`line(i)`/`col(i)` cost
  one binary-search into the loaded `SymbolTable` at access time;
  `len()` is O(1); `render()` is the only whole-trace formatting.
- **Opt-in at raise sites.** The `?`/err propagation path stays
  zero-cost (RFC 0036 §1's law). The intended shape: rich errs attach a
  trace only where the producer decides the cost is worth it
  (`trace: ?StackTrace` in err data — nil the default), exactly the
  RFC 0028 §`debug` sketch (`trace: Option<StackTrace>` "paid only when
  asked for").

---

## 4. The entry-err contract — the original rule, no carve-out

### 4.1 Grammar/typing: what already holds, and the one gap

The ORIGINAL crossing rule (RFC 0023 §2, enforced at
`rut-lir/src/check/mod.rs:628` via `crosses_boundary`,
`rut-core/src/types.rs:422`): an `entry fn`'s parameters and return are
built from primitives, `str`, `bytes`, `nil`, and `Opaque` — plus
tuples, which **already cross field-by-field**: `crosses_boundary`
recurses into `TyKind::Data`, so `(bytes, str)` entry returns type-check
TODAY (digest.rut's `entry fn hex_dec(s: str) -> (bytes, str)` and five
more compile on the current tree — the empirical proof, no engine change
needed for the err-as-primitive convention).

The user ruling's constraint: **no err-specific carve-out.** `err` is
not a type — it never was (RFC 0005 §10: the second element is
user-chosen; empty `str` / `false` / `nil` = success). Under the
original rule an entry's err component must itself be a crossable:
`str` errs are host-decodable directly (the advice shape —
`("", "")` = success), `bytes` errs carry binary detail, and **rich errs
ride `opaque(v)`** (RFC 0014 — the boundary currency) as the tuple's
first component. Nothing new is added to the crossing set for err's
sake.

**The one gap, measured:** `-> (?T, err)` does not type-check today —
`crosses_boundary` returns false for `TyKind::Opt` regardless of
element (verified empirically: `entry fn probe_opt_err(b: bool) ->
(?i64, str)` is a compile error on base `4ceb17e`, while the same
signature with a prim first element compiles). RFC 0023 §1's own text
already promises the intent ("optionals cross as the v1.1 tuples they
were always spelled as"), the implementation just predates it. Phase 3
closes the gap the minimal way: `Opt { elem }` crosses iff `elem`
crosses, and decodes at the boundary as **nil-flattening** — the null
slot becomes `Value::Nil`, the some-slot becomes the payload's Value.
No `Value::Opt` variant is needed (nil IS absence — the v1.1
convention; the err channel disambiguates a legitimately-absent value
from a failure: empty err + nil = "not found", non-empty err = "failed").
`slot_to_value` (`rut-vm/src/interp/util.rs:6`) gains the one
`TyKind::Opt` arm; `Value::Tuple(Vec<Value>)` already exists for the
pair.

### 4.2 The envelope shapes

- **The driver result (`vm.call`).** Today `Result<Value, Trap>`, with
  `Value::Tuple` for pair returns (field-by-field recursion in
  `slot_to_value`). A `-> (?T, err)` entry surfaces as
  `Value::Tuple([Nil|payload, Str(err)])` — the host decodes positionally,
  per the convention: `err` empty = the value channel is live.
- **The raw-ABI JSON (err beside trap).** The `rut-wasm` run envelope
  (`run_envelope`, `crates/rut-wasm/src/lib.rs:230`) is
  `{"output":[..],"trap":..,"fuelUsed":..,"heapBytes":..,"parked":..}`.
  Phase 3 adds **`"err"` beside `"trap"`** — the two channels stay
  distinct by law: `trap` carries panics (bugs, wiring drift —
  `OutOfFuel`, type drift, the loud channel, unchanged), `err` carries a
  returned failure (data the host reads and acts on). A soft-fail run
  reads `"trap": null, "err": "<the returned err>"`; the envelope's
  fuel/heap reporting is unchanged, so the json-decode fuel-disclosure
  discipline keeps applying verbatim.
- **The web pump's keep-running semantics.** The pump
  (`drain_queue`, `examples/05-todolist-web/src/state.rs:140`) runs one
  `vm.call("on_event", ..)` per queued event and today propagates a trap
  with `r?` — a failing turn kills the drain. Phase 3's soft-fail: the
  call returns the pair, the pump **decodes the err, reports it, and
  keeps draining** — the container survives, the page stays alive, the
  queue proceeds. Panics still abort the pump loudly (the drift
  detector, unchanged: a trapped turn is a BUG, a returned err is DATA).
  The twin tests phase 3 gates on: a failing turn leaves the page alive
  and the container usable. The wasm lane's `rut_web_*` exports keep the
  `0 | -1` + `rut_web_last_error` envelope, with the err text riding the
  existing error slot (no ABI growth).

### 4.3 Adoption shape (phase 3's scope call, pre-stated)

digest.rut is the teaching adoption: its six entry fns already return
`(bytes|opaque|str, str)` — under the phase 3 contract the demo runner
surfaces the err field and the entries' str errs become the driver's
soft channel with zero rut-side rewrites. The todolist store's str
answer lines are the second candidate; adopt-or-record-why-not is
phase 3's call, per the plan.

---

## 5. The bench method — pre-registered

### 5.1 The proving row: a `checked_*` microbench

The census's best tuple-heavy candidate for a per-op proof is the
**`checked_*` family itself**: compiler-intrinsic, zero source noise,
one mint per call, the exact `(value, ok)` pair phase 1 fuses — and a
rut-only row is precedented (`u64loop` has no `.js` twin; i64/u64
arithmetic below 2⁵³ keeps even a JS twin exact where wanted).

New workload `checkedadd` (name TBN in phase 1), shape:

- A hot loop over i64 pairs calling `a.checked_add(b)`, destructure
  `let (v, ok) = ...`, accumulate `v` into an i64 checksum (wrapping)
  and count `ok == false` arms separately — both terms in the checksum
  so the fusion cannot silently drop the bool lane.
- Inputs chosen so the overflow-arm count is deterministic and small
  but nonzero (the ok-lane must be exercised, not predicted-branch-only).
- Scale sized so the row's runtime is dominated by the pair economy
  (target ≥10M checked ops), printed once as `CHECKSUM <n>` per the
  suite's discipline; every term an exact integer.
- Pinned in `expected.json` BEFORE phase 1's engine change lands; the
  baseline time/fuel/heap recorded on the base commit.

### 5.2 What proves the fusion

1. **Checksums immovable**: the new row's checksum is bit-identical
   before/after (pure lowering — the plan's gate), as are ALL existing
   rows' checksums (predicted; any drift is a bug, not a re-pin — the
   only sanctioned `expected.json` edits are new-row additions).
2. **The per-op delta**: the new row's time falls by the mint tax
   (predicted band: ~40–80 ns/op today — allocation + 2 field writes +
   rc release + `movref` + 2 `getf`s, per §1.2's anatomy and the refval
   round-1/2 calibration of ~88–175 ns for the same package); after
   fusion the residual is the bare `waddi` + overflow-check ops.
   Fuel falls by a fixed op-count per checked call site (the
   `makerecord` + `movref` + 2 `getf` + join ops) — counted from the IR
   diff, disclosed as ops, and cross-checked against the fuel delta
   (the 0.77 ns/op crossing-nop calibration reconciles the two meters).
3. **Fuel/heap deltas disclosed, old values verbatim** in the commit
   body (the json-decode precedent): any row whose fuel/heap pin moves
   names the old value in-source-adjacent disclosure. Predicted
   pin movements: **none** — no existing bench row's hot loop
   destructures tuples (census §1.4: the nmapset/refvals pair mints are
   `Pt` values escaping INTO `put()` calls — real escape, correctly not
   fused; alloc/binary-trees mints are SROA-local or stored nodes).
   Predicted time movement of existing rows: within noise. If a row
   moves beyond noise, the cause is measured and disclosed before
   phase 1 closes.
4. **The digest row (optional, phase 1's call)**: digest.rut is an
   example, not a bench row; adding it as a workload would give the
   `(T, err)` economy its own before/after line (~600 mints/rep
   class). Pre-registered as optional because it mints opaque boxes per
   value (its time mixes box traffic with pair traffic) — the
   checkedadd row isolates the pair economy cleanly, and digest's
   improvement is a disclosed observation, not the proof.

### 5.3 Reconciliation discipline

Phase 1's verdict table: per-row (old checksum / new checksum / old
fuel / new fuel / old heap / new heap / time band), predicted-vs-
measured for the rows named above, and the per-op proof line for
checkedadd. Anything unexplained at phase-1 close is a defect logged in
the phase report, not a silent accept.

---

## 6. Phase-0 exits, mapped to phases 1–4

| finding | feeds |
|---|---|
| SROA banks all local literals; the survivors are the return channel (§1.3–1.5) | phase 1's fusion targets the ret-slot chain, nothing else |
| Single-reg ret/call + pinned 24-byte op + prims-raw-in-slots (§2.1) | phase 1's shape: a `rut-lir` lowering pass, no IR/format change |
| `builtin class` member contract, RawTrace/SymbolTable reuse, lazy per-index symbolication (§3) | phase 2's engine builtin + the VERSION 7→8 bump |
| tuples already cross field-by-field; `(?T, err)` is one `crosses_boundary` arm + nil-flattening decode; envelopes named (§4) | phase 3's entry-err work, engine-minimal |
| the checkedadd row + immovable-checksum discipline (§5) | phase 1's proving bench, phase 4's verdict table |

**Risks, stated now:** (1) the lowering pass must respect the SROA
guards on ref-field pairs (`(bytes, str)` components cost one retain
each at the handoff — never a borrow across the join); (2) the
threaded interpreter's op stream changes only by deletion, but the
fusible windows span branch joins — the pass must stay within
single-def, post-join-visible chains or decline (the census tool's
precision rules are the prototype); (3) `(?T, err)` entries widen the
host ABI by one decode arm — the nil-flattening must be pinned by twin
tests so rut-nil and VM-nil stay indistinguishable.

*No code was changed in this phase. The gates: docs-only commit,
workspace untouched and green, tree clean.*
