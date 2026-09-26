# The Weak Batch — Report

RFC 0017 v1 (`Weak<T>`), three phases: the survey (`docs/weak-survey.md`,
base `8d59215`), the landing (`85a14d0`, VERSION 13), this record. The
design decisions D1–D3 were fixed in the brief and restated as law in
the survey; everything here is what the LANDING taught that the survey
did not know.

## 1. The surface, as landed

```rut
let w = Weak(node);        // construction is a type-call (the opaque(v) law)
let back = w.upgrade();    // ?Node — the live referent, or nil
```

- `Weak<T>` is the engine's **first generic builtin class**
  (`TyKind::Weak { elem }`, the `Array { elem }` precedent — `mk_weak`
  beside `mk_opt`, no boot row). `StackTrace`/`StrBuf` were the
  non-generic template; `Array` was the only prior generic TyKind, and
  it has no builtin class surface — Weak is the first to be both.
- Ambient (no `use`), the `builtin class` row in `core.d.rut`; the LSP
  embeds that file verbatim (`include_str!`), so the surface move rode
  the one file.
- `upgrade()` on `Weak<?U>` answers `??U` (D2) — MakeOpt wraps the box;
  there is no unguarded unwrap anywhere on the path.

## 2. The one structural find: OPS, not Nats

The survey's plan was `Nat::WeakNew`/`Nat::WeakUpgrade` beside the StrBuf
natives. Probing the mint path killed it: both ops need the instantiated
TypeId at RUNTIME (`WeakNew` must stamp the box's `CellVal.ty` with the
`Weak{elem}` id; `WeakUpgrade` must mint the `?elem` opt box), and
`CallNat` carries no type operand. `MakeOpt { dst, src, ty }` was the
exact precedent already in the wire: **`Op::WeakNew { dst, src, ty }` /
`Op::WeakUpgrade { recv, dst, ty }`**, opcodes 92/93. No Nat entries;
the bump to VERSION 13 is driven by the TyKind + the two opcodes.

## 3. The consume law (the landing's real find)

The first end-to-end run failed five pins in one shape: a referent
reassigned away STILL upgraded non-nil. The op stream showed why: the
type-call's argument is a path load — a `MovRef` **+1 temporary** — and
the temporary's reference lives until the frame ends. A weak that
merely reads its argument never observes the binding's death mid-frame;
every in-frame death was invisible until frame teardown.

The fix is a semantic law, now stated on the op: **`WeakNew` CONSUMES
its incoming reference** — releases it and nulls the register (frame
teardown would otherwise release it again). The weak observes the
BINDING's lifetime, not the temporary's. It is the engine's one
consuming op; everything else shares.

## 4. The call-boundary test method

The consume law makes deaths observable mid-frame, but the FIRST-draft
tests then failed the mirror-image way: upgrades used as probes pin the
referent through their answer boxes, and if-condition reads pin their
operands — all lawfully, until frame end. Three drafts fought this
before the shape settled:

- **sever and probe across a CALL BOUNDARY** — the callee's temps died
  at its own ret, so the kill is decisive and the probe is clean;
- **build in a callee, sever in main** — the cycle test's `build()`
  returns the fused sole handle; `parent = nil` in main drops the last
  reference, and the drain loops the cascade (parent cleanup → field
  release kills the child → child cleanup) exactly as RFC 0016 §3
  promises;
- **the cycle test's on_drop attaches to the `?Node` bindings
  themselves** — a first draft wrapped them into fresh boxes, and the
  cleanups fired on the WRAPPERS' deaths (at build's ret, LIFO) —
  the upgrade then legitimately answered a still-alive parent. The
  binding IS the cell (RFC 0044); attach to it.

The passing cycle pin is the batch's centerpiece: the child's cleanup
UPGRADES its weak back-pointer from inside the cleanup — after the
parent's box died — and answers nil, because the nulling preceded any
user code. No resurrection is possible; the ordering is total.

## 5. D1 as landed: store entries are weak-referenceable

Both `release_ref_slot` arms carry the hook, so `opaque(v)` boxes (Rut
entries) and `alloc_host_box` payloads (Host entries — the
`weak_host` pkg fixture binds one) are weak-referenceable from script.
The keying is the full slot word, the same keying `drop_fns` already
used for both shapes; the entry arm nulls BEFORE the pin check and
before `release_entry`'s finalize. The host-side Weak VIEW (a host
holding a weak to a rut cell) remains RFC 0017 §1's future — this is
the script-side half only.

## 6. Accounting & the heap receipt

WeakBoxes charge `CELL_OVERHEAD + 8` at mint and refund at death; the
`weak_lists` map is uncharged engine bookkeeping (the `drop_fns`
precedent). The churn pin runs 50 add/upgrade/drop rounds and asserts
`heap_usage() == 0` after the run — cells, boxes, opt boxes and the
lists themselves all come back. An OOM at the WeakNew mint traps
`OutOfMemory` before any registration (the charge precedes the write,
RFC 0040 §1).

## 7. The menu (recorded, not built)

- **§3's shutdown leak report** (D3): type-grouped survivors at
  `Vm::drop` — `Arena::drop` already walks every cell, so the
  histogram is cheap; allocation SITES need debug-build plumbing and
  stay the bigger half.
- **Host-queryable live counts per type** (`Heap::live_cell_counts()`)
  for dev UIs — the same chunk walk.
- **Fuzz targets** (no crash; destructor counts vs allocations minus
  survivors).
- `alive()` sugar (`upgrade() != nil` spells it), retained-by graph
  dumps (RFC 0017 OQ-1), the collector hook (OQ-2 — the `weak_lists`
  map is the only state a collector would need; v1 assumes nothing).

## 8. Honest limits

- `Weak` does not cross the host boundary (no `Value::Weak`) — hosts
  that need lifetimes hold `OpaqueRef`; a crossing attempt diagnoses
  with the standard "cannot cross" family.
- `Weak(fn)` refuses by law — fns are RFC 0016 §1's one non-cell
  non-prim, and the admission says so with the same message as
  primitives. (The survey's first draft listed closure-typed as legal;
  the landing corrects the record.)
- The demo corpus (`weak-cache.rut`, `node-cycle.rut`) still tells the
  strong-refs story and runs green — the Weak rewrite is `demo/*`, the
  foreign lane's.

## 9. Gates (on the landing tree)

cargo test --workspace 97 suites / 827 tests, 0 failed; wasm32
check exit 0; LSP 115+ green on the rebuilt artifact (`bin/rut-lsp.wasm`
md5 f5976d2e1832489faa686d42768f236c; vsix 0.2.5 — both gitignored, the
md5s are the receipt). Commits: survey `e5285d5`, landing `85a14d0`,
this record — docs only, staged by explicit path.
