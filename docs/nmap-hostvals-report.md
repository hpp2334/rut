# nmap-hostvals — the batch report

The record of the nmap-hostvals batch: six phases plus this record, one
shared tree, base `b8ced31` (post nmapset-hostops), all on `master`.
Companion to `docs/nmap-hostvals-survey.md` (phase 0 — the census, the
probes, the baselines, the §0.8 micro-calls; its §-numbers are the ones
the phases cite). The laws live in the RFC amendments this record
appends: RFC 0023 #3 (the any-arm law + the Opaque store repr law),
RFC 0014/0025/0026 (the repr change), RFC 0016 §3 (the finalize arm of
the release walk), RFC 0004 (the char exorcism's completion); the
movers live in `benches/README.md`'s nmap-hostvals section; the
user-facing summary lives in `examples/README.md` (the keyed-collections
section).

The one-sentence arc: the host table became a REAL `HashMap`, the
values moved INTO its entries across ONE `any` crossing per op, the
arena anchor cell and the wrapper's `[?V]` sidecar both died, `char`
finished dying (VERSION 12), and no bench row regressed — most rows got
dramatically faster and lighter.

---

## 1. The story, in one section

**The directives (verbatim, this batch):**

1. nmapset-hostops (landed): *"move the impl to hostm rut part is just
   a wrapper at all"*
2. *"But what I say is host part impl, I recommend use HashMap"* — the
   host table becomes a REAL HashMap.
3. *"And value can cross with opaque. If slow, we should optimize the
   opaque perf"* — values cross too; if the crossing costs, the
   crossing-fastpath track absorbs it later (§5 — it never cost).
4. *"If we want to introduce it, I recommend rename to HostOpaque and
   replace our current host part opaque mechanism"* — ONE unified
   type, **FULL REPR CHANGE** (locked by Q&A): the opaque value itself
   becomes a HostOpaque handle; the arena anchor cell dies.
5. *"Also note that avoid copy and allocation if possible"* — the
   copy/alloc laws below are gates.
6. *"And we may need to change vm since the rc and release
   requirement"* — the VM rc/release work is in scope by directive.
7. *"And remove 'char' entirely, we removed it before, but not clean"*
   — RFC 0004 v1.1 killed the surface; the enumerated machinery
   survives in the IR, the VM, and the boundary. The exorcism is
   phase 1.
8. *"We should distingue with RutOpaque and HostOpaque"* — two kinds,
   two identity worlds; the store's entries are the split.
9. *"HostOpaque can use Box Any if needed"* — host payloads are
   per-map, never per-op; the no-Box law lives on the VALUE path.

**The locked calls (Q&A receipts, at planning):** the stdmap core
deletes the slot/sentinel/grow/drain/cap lanes (25 crossings) and
re-bases `map_val_*` on handles, with the hasher the pinned mapset
FNV-1a/mix64 bits through an identity hasher (the map buckets on the
same words the wrapper's `k.hash()` produces); ONE combined batch (the
nmapset-hostops three-phase shape was not repeated); release-at-death =
the VM finalizer hook (payload opt-in, invoked at the release walk) —
NOT the Disposal route; HostOpaque = FULL REPR CHANGE (`OpaqueBox<T>` +
`OpaqueRef` + the RFC-0014 anchor cell REPLACED at the representation
level); RutOpaque vs HostOpaque distinguished, and HostOpaque may stay
`Box<dyn HostPayload>`-shaped — TypeId lives ONLY on the Host path
(the borrow gate), the value path stays Box-free/alloc-free by law;
VERSION 12 lands at phase 1 (the char wire moves; the repr rides the
same bump).

**Phase 0 — the survey (`ab9bd9e`, docs only).** The char liveness
census (every enumerated site, tiered LIVE/DEAD/MACHINERY; the
decisive receipt: `scalar.rs convert()` has NO char arm — the VM never
had char semantics, slots carried codepoints as bits all along; tag 11
LIVE ON THE WIRE through opcode 47's `Conv` operands, making the
VERSION 12 bump load-bearing); the repr inventory (every
OpaqueBox/OpaqueRef/anchor site mapped to its migration shape); the
store-home receipts (slots never discriminate refs at runtime — the
bytecode does; recommendation: the slab); the any-arm receipts (no
any-arg arm exists; a `register!` host returning `Value` COMPILES and
traps on every call — the write direction genuinely missing;
`(slot, TyKind)` is zero new plumbing via `FuncDef.regs`; the
`TY_ANY = u32::MAX` sentinel collision); the finalizer site
(`release_cell`'s children-first walk); the bench V-kind table with
the per-row baselines and gate roles; the suite-fate census; the
errata (§7 here); all ten §0.8 micro-calls resolved with receipts.
Gates: 790/0, wasm32 exit 0.

**Phase 1 — the char exorcism (`8352b50`, VERSION 11 → 12).**
Codepoints ride u32 end-to-end: `s.code()`/`s.code_at(i)` lower to ONE
new op each (`StrCodeAt`, answering the u32 directly — the old
`StrCharAt` + `Conv{Char→U32}` pair dies; the survey proved the
register was ALWAYS a raw codepoint), `str.from_code(n)` and the utf8
walkers drop their `Conv` intermediates for the new `Nat::StrFromCode`
(nat tag 21, appended). Then the enumerated deletion: `PrimTy::Char`
(tag 11), `ConstVal::Char` (tag 3, decode-only dead since the literal
removal), `ArrKind::Char`/`[char]`/`?char`, `Value::Char`,
`Slot::ch`/`as_char`, `alloc_char` retuned to the u32 codepoint (repr
unchanged — a 1-codepoint STR cell; char never had a heap repr of its
own), all six boundary char impls. Opcode 48 RETIRED, never
re-meaninged (`StrCodeAt` takes NEW number 91; old binaries fail by
name, and the VERSION byte rejects them first); the boot-table row at
id 12 STAYS as a reserved `Nil` shell (wire-stable fixed ids never
reorder). The lexer's literal diagnostic verbatim; the type-position
pin test lands (`removed_char_type_positions_diagnose_without_the_kind`
— a DEAD fn is never lowered, so the pin reaches its param through a
call; discovered while landing the pin). One surprise, disclosed: the
re-spell broke `last_reg` (the old pair's accidental "allocate dst
LAST" discipline); the playground str-views pin caught it. Fuel
receipts: the utf8 rows win (json-decode −324,633, json-roundtrip
−518,652, strbuild −648); **fasta's fuel came back BIT-IDENTICAL** —
its loop rides `StrBufPushCode`/`Concat`, never the re-spelled walkers
(the plan's "fasta" mention was a prediction, not a receipt — §4);
every non-utf8 row fuel- AND heap-identical. Gates: 791/0, wasm32
exit 0, 8-row bench gate green, expected.json untouched.

**Phase 2 — the Opaque store repr (`2a4716a`, behavior FROZEN).**
`HostOpaqueStore` — a VM-side slab (chunked like the cell arena,
entries never move, free list) on the `Arena` — of two-kind entries
`OpaqueEntry { Host(HostOpaque), Rut(RutOpaque) }` + rc + borrows.
`HostOpaque { payload: Box<dyn Any>, type_name, finalize }`; RutOpaque
re-seats the RFC 0014 box (`slot`, `val_ty` rides the entry).
`CellData::HostBoxed` and `CellData::OpaqueBox` DELETED from the enum;
`OpaqueBox<T>` deleted; the typed view is `Opaque<T>`. Addressing is a
TAG BIT (bit 0) on the slot word — entries 8-aligned, nil stays
all-zero, the tagged word SELF-IDENTIFIES so `Heap::retain`/release
route to the entry's rc with one branch and the ENTIRE generic rc web
keeps its type-free discipline with zero call-site churn. Borrow
guards (BORROW_MUT) re-homed on the entries — same law, new home;
`from_handle`'s checked-borrow law byte-for-byte today's (the
two-kind law, once an error string, now an exhaustive match). §0.8(k)
disclosed honestly: the payload is `Box<dyn Any>` + an optional
monomorphized finalize thunk, not `Box<dyn HostPayload>` — the suites
pin `Opaque::alloc(vm, 42i64)` mints and the orphan rule forbids the
test-side impls; the observable law is identical ("payloads opt in").
The finalize hook lands at store-entry death (rc-0 in the release
walk): refund, free, `finalize(heap)` BEFORE the Box's own Drop (RFC
0016 §3 ordering preserved by construction), a Rut entry's held slot
released through the same walk. The debug leak check is the BALANCE
form (inserts == deaths + teardown_freed) — the strict "store empty"
form is unreachable: this VM's trap semantics leave torn frames
unreleased today (behavior-frozen). `pool.rs` DELETED (its per-op
churn rationale is exactly what "one payload per map/logger" removes),
six replacement store tests. Fuel BYTE-IDENTICAL on both spot-checked
rows — the frozen proof. Gates: 791/0 (exactly the phase-1 count; −6
pool tests +6 store tests cancel), wasm32 exit 0, checksums immovable.

**Phase 3 — the any arms (`6571549`, additive).** The boundary hands
the arg as `HostVal { slot, ty }` — the raw slot TOGETHER WITH the
CALL SITE's static type (rut types live in the VM, never Rust; `ty`
is the arg register's own `FuncDef.regs` word, read through
`vm.prog.types`, never cloned — the copy/alloc laws hold on the put
path). `HostCode` grew the site-types slice; bindings without any
params pass `&[]` and never pay. The any-answer: `Option<ValSlot>`
(`ValSlot { Empty, Bits(Slot), Ref(Slot) }`, exported from rut-vm,
P5's storage repr) — Bits moves the 8 bytes as-is (the untagged-slot
discipline, §0.8 h), Ref takes the ArrGet shape (retain first, release
the displaced — the register borrows its own rc), None is the flat-nil
miss, Empty TRAPS loudly (§0.8 g); the write-back branches on the DST
register's own declared type — the trust law documented at the decl
site (the host answers the caller's V). The decl spelling: `any` is
host-decl-only — the grammar arm plus the NEW boot id 18 (`TY_VAL`,
deliberately NOT the VM's `u32::MAX` sentinel; the name table stayed
TAIL-ONLY, so no sym was minted — a VERSION bump was forbidden).
13 new tests (7 rut-vm any-lane units + 6 rut-driver end-to-end):
retain/release balance, view-arg identity, zero cells on the answer
path, the narrow-V word, the Empty trap, the grammar row, rut source
refusing the name. Everything ADDITIVE — nmapset-int's fuel
byte-identical to the survey baseline. Gates: 804/0 (791 + exactly
13), wasm32 exit 0.

**Phase 4 — the stdmap core (`8340c74`).** `NativeTable` is the plan's
struct verbatim: `{ kind: KeyKind, map: HashMap<KeyVal, Entry,
MapBuild>, next_handle: i32 }`, `Entry { handle: i32, val: ValSlot }`;
`KeyVal`'s `Hash` writes ONE u64 (`hash_payload` — mix64 for the
integer/bool bits, FNV-1a 64 over the octets; the pinned mapset
constants) through `IdHasher` — the map never re-hashes. The probing
machinery deletes wholesale (hashes/keys/states/vals/handles vecs, the
reloc queue, needs_grow's load law, grow, tombstones, both probe
walks, the sentinel, the slot-addressed val column);
`size_of::<NativeTable>()` 168 → 40, tripwire-pinned. The h-family
re-mounts on the entry API — the packed `(handle << 1) | newly` answer
UNCHANGED bit for bit (HashSet keeps reading bit 0); growth is std's,
internal, invisible (the handle is a birth, not an address — nothing
ever relocates). `map_val_*` re-based on live birth handles, exact-arm
tag checks, never reinterpret. THE ONE HONEST PRICE, disclosed: std
HashMap has no borrowed probe, so the s/sv lanes materialize ONE owned
`KeyVal::Str` per crossing (was: zero-copy until a fresh insert) —
absorbed, every row got faster (§3). Decls: 24 of the planned 25 leave
BOTH `.d.rut` copies (byte-identical) — `map_cap` was re-tuned to a
reserve read-back because the untouched wrapper still pre-sized its
sidecar through it (the plan's own P5 paragraph schedules its death
exactly there; it died at P5). The plan's "49 → 42" arithmetic was
internally inconsistent with its own KEPT(24) list; this phase landed
49 → 25. Suite fates per §0.8(d): `nmap_valcolumn.rs` (4 tests) and
`nmap_typelanes.rs` (8) retire, their laws migrated to unit tests;
nmap.rs units 34 → 20; rut-cli/tests/nmap.rs 9 → 6 onto the h-family;
sweep-found pin retunes beyond the instruction's list (nmapset.rs +
opt_prim_store.rs mount lists, std_surface.rs, the e2e-wasm.js:218
completion pin the phase-0 census missed). Gates: 775/0/3, 94 suites
(the −29 enumerated dying tests, exact), wasm32 exit 0, 7 rows × 3
runtimes green, the e2e lane PASS (85 corpus files / 0 false
diagnostics / 22 smoke assertions through a rebuilt rut-lsp.wasm).

**Phase 5 — the value migration (`365e6c2`).** The values move INTO
the entries: six valued crossings `map_hv{put,get,remove}` + the `_sv`
twins, the key crossing as `any` TOO (see §4's discrepancy 1). THE
PUT: one crossing carries key AND value — a prim V stores
`ValSlot::Bits` (the 8 bytes move as-is, f64 riding the slot's `f`
field; zero cells, zero copy), a reference V retains its ORIGIN slot
in-crossing and stores `ValSlot::Ref` (a str VIEW value stores the
view's own cell — aliasing IS the view; `Value::Str`'s owned decode
never used); replace releases the old value in-crossing; the packed
answer bit-identical to the h-family's. THE GET: one crossing
answering `Option<ValSlot>` — None crosses as the flat nil, an Empty
entry TRAPS loudly naming the placeholder. THE REMOVE: releases the
held cell in-crossing, answers the dead key's handle or −1. The
mixed-kind admission trap fires BEFORE the value retain (a trapped put
cannot leak a retain it has not taken). `impl HostPayload for
NativeTable` fills the P2 hook — the map's real release walk releases
every held Ref at entry death, the map clears. The wrapper collapses
to `HashMap { t: opaque }` pure delegation (340 → 292 lines): the
`[?V]` sidecar, `vlen`/`vcap`, `vstore` and the file's LAST loop all
DELETE; `map_cap` retires with the sidecar it pre-sized (nothing
rut-side pre-sizes anymore); every method is ONE crossing. The
aliasing law survives by construction — the four nmapset.rs scenario
pins (checksums 132 / 85 / 7 / 41) pass on UNCHANGED sources.
Two discrepancies and two surprises, all disclosed in §4: the 18
typed-key hv lanes became 6 any-key lanes (rut has no generic trait
methods), and the `?V` write-back needed engine arms — RFC 0044's ?T
repr forces an unavoidable ONE-box mint per get (the plan's "no box
mint" law dies for the ?T repr; the mint moved sides, the measured
totals improved; measured INSIDE the winning numbers, not pending
optimization). Gates: 785/0/3 (P4 base 775 + exactly the 10 new pins),
wasm32 exit 0, FULL bench gate — ALL 28 rows × {rut, node, qjs} =
85 result rows, 0 mismatches, expected.json UNTOUCHED.

**Phase 6 — the record (this commit).** This report; the RFC
amendments (0023 #3, 0014/0025/0026, 0016 §3, 0004); the
`benches/README.md` perf-log section; the `examples/README.md`
keyed-collections rewrite (the sidecar becomes history); the errata
disclosed against the nmapset-hostops record (§7); the menu (§8).
Docs only, zero code changes — the engine is byte-identical to
`365e6c2`.

## 2. The two-kind law and the copy/alloc laws as landed

**Two kinds, two identity worlds** (directive 8): a rut `opaque` value
addresses ONE store entry directly — no arena anchor cell, one
allocation and one indirection fewer per opaque.

- **`OpaqueEntry::Host(HostOpaque)`** — a HOST payload in the Rust
  TypeId world: `payload: Box<dyn Any>` + `type_name` + the optional
  finalize thunk. TypeId exists ONLY here (the borrow gate):
  `Opaque<T>`'s checked borrow is `payload.is::<T>()` /
  `downcast_ref::<T>()` via the Any vtable — today's exact law, new
  home. One payload per map/logger, NEVER per-op (directive 9 — the
  Box lives here by the user's relaxed call; `pool.rs` died with the
  per-op churn rationale).
- **`OpaqueEntry::Rut(RutOpaque)`** — a RUT value held host-side: the
  cell slot (+ `val_ty`). Identity IS the cell; the rut-side downcast
  reads the cell's runtime kind, NEVER TypeId. Box-free by law.

The any arms consult the call site's static `TyKind` (§0.8 m) — never
Rust types, on either side. `HostOpaque`'s death runs `finalize(heap)`
at the release walk, then the Box's own Drop (RFC 0016 §3, preserved
by construction).

**The copy/alloc laws — planned as gates, landed as receipts:**

- put(prim V): inline bits — zero cells, zero copy (the 8 slot bytes
  move as-is; f64 rides the `f` field).
- put(ref V): retain the arg register's OWN slot — no wrap box, no
  String copy; a str VIEW arg stores the view's cell (aliasing IS the
  view).
- get: the answer writes the V register from bits/slot — no box mint,
  no copy, no rc op **on the plan's assumed plain-V shape**. The LANDED
  wrapper shape is `get -> ?V`, and RFC 0044's ?T repr is a one-slot
  box: the write-back mints exactly ONE box per get (a transient opt
  value for a `?prim` V — byte-for-byte the cost P4's sidecar read
  already paid; the ?V box aliasing the STORED cell for a `?ref` V —
  where P4 boxed per put, P5 boxes per get; the mint moved sides, the
  measured totals improved). Disclosed, measured inside the winning
  numbers.
- remove/replace: the old slot releases in-crossing — rc ops symmetric
  with array-slot semantics, accounting balanced (a double release
  would panic the debug rc walk; a leak shows in the accounting — the
  rut-cli valued suite pins both directions).
- no per-op cell allocation on the put paths: the prim rows' VM heap
  DROPPED by the sidecar and did not regain it — the plan's measured
  pin, over-delivered: nmapset-int 1,720,767 → **311 B**, BELOW the
  hashset no-V floor (343 B, itself byte-identical through the phase).

## 3. The movers (the honest tables, both trees)

Protocol both phases: base commit re-run in a scratch worktree, after =
the landing, same session, through `rut-bench-probe` (P4: 3 probe iters;
P5: 3 probe runs × 3 fresh-VM iters, min exec); checksums × {rut, node,
qjs} against expected.json (UNTOUCHED all batch). Full numbers in
`benches/README.md`'s nmap-hostvals section.

**P4 vs P3 (`8340c74` vs `6571549`)** — the wrapper untouched; every
row moved DOWN on every axis anyway (the box charge 168 → 40, the
reserve-shaped sidecar ladder, less memset/clone surface):

| row | checksum | fuel | heap B | exec ms |
|---|---|---|---|---|
| nmapset-int | 734932704 = | 21,571,682 → 21,243,927 (−1.5%) | 1,966,663 → 1,720,767 (−12.5%) | 68.8 → 62.4 (−9.4%) |
| nmapset-str | 1264308351 = | 9,910,968 → 9,747,053 (−1.7%) | 984,086 → 861,070 (−12.5%) | 53.7 → 49.3 (−8.2%) |
| nmap-hashset | 21500055 = | 12,716,782 → 12,716,782 (0, BIT-IDENTICAL) | 615 → 343 (−272) | 45.8 → 39.9 (−12.9%) |
| nmap-knucleotide | 2198604 = | 37,419,557 → 37,091,862 (−0.9%) | 2,229,052 → 1,983,156 (−11.0%) | 163.3 → 150.4 (−7.9%) |
| kmer-view | 2198604 = | 35,019,405 → 34,691,710 (−0.9%) | 2,228,884 → 1,982,988 (−11.0%) | 139.2 → 131.0 (−5.9%) |
| strview | 1264308351 = | 11,094,312 → 10,930,397 (−1.5%) | 2,032,285 → 1,909,269 (−6.1%) | 39.1 → 38.3 (−1.9%) |
| refvals | 140052990000 = | 55,633,341 → 54,977,831 (−1.2%) | 26,843,812 → 26,188,180 (−2.4%) | 253.8 → 240.1 (−5.4%) |

The P4 mechanism, disclosed: `HashMap::with_capacity(8)` reserves 14,
not the old power-of-two 8, so the wrapper's `with_capacity` pre-sized
its sidecar at 14 and the doubling ladder landed 14 → 28 → 56 → … —
fewer `vstore` copy-loop iterations per churn (the errata'd "ZERO
loops" loop, §7). The CONTROL is nmap-hashset: NO V, no sidecar, fuel
BIT-IDENTICAL — the h-family's answers unchanged, proven by the one
row that can see nothing else.

**P5 vs P4 (`365e6c2` vs `8340c74`)** — the values cross inside the
crossing; the sidecar is gone:

| row | fuel | VM heap | exec ms |
|---|---|---|---|
| nmapset-int | 21,243,927 → 12,600,090 (−40.7%) | 1,720,767 → 311 B (−99.98%) | 62.680 → 40.742 (−35.0%) |
| nmapset-str | 9,747,053 → 6,133,436 (−37.1%) | 861,070 → 772 B (−99.9%) | 49.258 → 42.250 (−14.2%) |
| nmap-hashset | 12,716,782 → 12,716,782 (0.0%) | 343 → 343 B (identical) | 40.562 → 42.421 — the receipt, §5 |
| nmap-knucleotide | 37,091,862 → 23,608,704 (−36.4%) | 1,983,156 → 263,526 B | 147.810 → 122.352 (−17.2%) |
| kmer-view | 34,691,710 → 23,008,616 (−33.7%) | 1,982,988 → 263,134 B | 128.626 → 101.329 (−21.2%) |
| strview | 10,930,397 → 7,616,781 (−30.3%) | 1,909,269 → 1,048,973 B | 37.458 → 29.416 (−21.5%) |
| refvals | 54,977,831 → 35,470,157 (−35.5%) | 26,188,180 → 12,000,492 B | 248.998 → 160.192 (−35.7%) |
| json-roundtrip | 31,457,155 → 31,457,155 (0.0%) | 3,423,706 → 3,423,706 B | 110.192 → 110.221 (+0.03%) |

The §0.8(n) receipt, measured: nmapset-int's VM heap drops BELOW the
hashset floor and does not regain it — the full-8-byte-slot storage
(the default §0.8(n) chose) over-delivers; array-style width packing
stays menu-only (§8). refvals' record-mint churn halves (26.2 →
12.0 MB) with the alias law checksum-load-bearing and green.
json-roundtrip byte-identical fuel AND heap: the pkg's own map rows
already rode the h-family; the wrapper's V traffic is off its hot path
— the record-V gate passed by standing still.

## 4. Survey claims corrected on receipt

- **The lexer's reserved `any` — the "zero parser work" miss.** The
  survey's §4 sized the decl spelling as "+1 arm in `crossing_ty` …
  the parser is not the gate", and §9(a) confirmed "zero parser work".
  WRONG: the LEXER has reserved `any` in every mode since RFC 0012
  ("rut has no `any`; use a trait type or `opaque`"). P3 taught the
  lexer a decl-mode (`lex_mode(src, decl)`; `lex(src)` unchanged, LSP
  callers untouched; the parser passes `mode == Mode::Decl`). In `.rut`
  source the name stays reserved — the RFC 0012 diagnostic verbatim,
  test-pinned; `any` is host-decl-only.
- **The checker's one-liner undersized the answer side.** The survey's
  sizing named "the checker's any-ARG coercion"; landing needed one
  more arm to be honest end-to-end: a `-> any` row has no register
  type of its own, so the extern-call path types the dst register with
  the CONTEXT's expected type (`expected.unwrap_or(TY_VAL)`) and
  returns it as the expression's static type. Disclosed as
  sized-by-receipt, not by the survey's one-liner.
- **Ret-for-Value traps — the survey's finding held exactly.** §4's
  receipt (a `register!` host returning `Value` COMPILES and then
  traps on every call via the default `into_slot` — the write
  direction genuinely missing) was confirmed verbatim on landing; the
  any-answer lane is the different shape that serves the direction,
  with the P5 ?V write-back extending it (the next item).
- **The ?V write-back's unavoidable one-box mint.** Nothing in the
  survey (or the plan) priced the ?T repr into the answer path: a
  `?prim` register MUST mint the opt value (retaining raw prim bits as
  a cell pointer would be a wild write), a `?ref` register needs the
  answer INSIDE the ?T box (answering the bare cell SEGFAULTS the
  first field read — the probe pinned it before anything landed), and
  the Some/None tag cannot ride the raw word (a stored zero and the
  miss are the same 8 bytes; nmapset-int stores zeros — the checksum
  itself forced the `Vm::host_val_out` tag carry). The plan's "no box
  mint" law dies for the ?T repr's unavoidable ONE box, measured
  inside the winning numbers (§3's P5 table).
- **The P5 discrepancy pair — 18 typed-key hv lanes became 6 any-key
  lanes.** The plan sketches `k.hvput(self.t, v)` as KeyLane grown by
  hvput/hvget/hvremove at the same 11 impls; rut has no generic trait
  methods ("generic method calls are not supported in this build",
  rut-lir call.rs) and `any` is host-decl-only, so a per-key trait
  method CANNOT carry V, and typed hvput_i rows would be unreachable
  dead decls. The lane SUFFIX moved host-side: the key crosses as
  `any` and the call-site TyKind classifies it against the same
  closed set the h-family admits (floats refused — no stable equality
  contract). **Decl surface 25 → 30, not the plan's 42** (map_cap
  deleted, 6 hv rows added; the plan's 42 continued its own
  inconsistent 49→42 arithmetic already noted at P4 — landed: 49 → 25
  at P4, 25 → 30 at P5).
- **Tag 11 riding Conv operands — the load-bearing receipt, and the
  fasta prediction that missed.** The survey's §1.1 finding that wire
  tag 11 is LIVE through opcode 47's operands (every codepoint-reading
  binary carries a `Conv{Char↔U32}`) is what made VERSION 12
  load-bearing; the re-spell stopped every char-Conv emission BEFORE
  the deletion, so the tag withdrew with zero live users. The miss:
  the survey/plan predicted a fuel win on fasta too — its fuel came
  back BIT-IDENTICAL (its loop rides `StrBufPushCode`/`Concat`, never
  the re-spelled walkers; disclosed at P1).
- **P4's map_cap prune, corrected on receipt.** The plan's DELETED(25)
  list includes map_cap, but the untouched wrapper could not compile
  without it; P4 deleted 24 of the 25 and retuned map_cap to a reserve
  read-back, which died at P5 with the sidecar it pre-sized — exactly
  the plan's own P5 schedule.
- **The e2e-wasm.js completion pin the phase-0 census missed.** The
  survey's §7 pinned std_surface.rs's map_entry pin as the ONE LSP
  retune; P4's sweep found `e2e-wasm.js:218`'s positive map_entry
  completion pin too (retuned to map_hput_i + the same negative pin).

## 5. The crossing-cost receipt

Directive 3's price question — *"If slow, we should optimize the
opaque perf"* — answered by measurement: **NO row regresses.** The one
adverse sample in the whole batch — nmap-hashset exec +4.6% in P5's
first single-sample read — is noise, profiled: its fuel is
byte-identical (12,716,782), its code untouched by the phase, and the
extended 7×3 measurement gives base median 40.763 ms vs P5 median
40.677 ms (base's own worst sample 44.194) — parity inside the band.
json-roundtrip's +0.03% exec is the same band on a byte-identical op
stream. The crossing-fastpath menu item for this phase stayed CLOSED
on that evidence; the extended parity profile is recorded in §8 as the
baseline for any future revisit. The disclosures above (the ?V box
mints) are already INSIDE the winning numbers, not pending
optimization.

## 6. The gates (on the exact trees, in order)

- Phase 0 (`ab9bd9e`): workspace 790/0 (95 suites, 2 ignored), wasm32
  check exit 0; docs-only.
- Phase 1 (`8352b50`): workspace 791/0 (95 suite results), wasm32 exit
  0; the lexer diagnostic verbatim; the type-position pin landed;
  bench gate — 8 rows (json-decode, json-roundtrip, fasta, strbuild +
  the 4 nmap rows) checksum-green × 3 runtimes, expected.json
  untouched; **VERSION 12**.
- Phase 2 (`2a4716a`): workspace 791/0 — EXACTLY the phase-1 count
  (−6 pool tests +6 store tests cancel), wasm32 exit 0; fuel
  byte-identical on both spot-checked rows (12,716,782 / 21,571,682);
  checksums immovable; zero new compiler warnings; VERSION 12 NO BUMP.
- Phase 3 (`6571549`): workspace 804/0 (96 suites — the phase-2
  baseline 791 + exactly the 13 new tests), wasm32 exit 0;
  nmapset-int fuel byte-identical (21,571,682 — the additive proof);
  VERSION 12 NO BUMP.
- Phase 4 (`8340c74`): workspace 775/0/3 (94 suites — the delta is
  exactly the enumerated dying tests: −14 rut-std nmap units, −3
  rut-cli nmap, −4 nmap_valcolumn, −8 nmap_typelanes, −2 suite files),
  wasm32 exit 0; both `.d.rut` copies content-identical (25 decls);
  bench family rows × {rut, node, qjs} green; the e2e lane PASS (85
  corpus files / 0 false diagnostics / 22 smoke assertions).
- Phase 5 (`365e6c2`): workspace 785/0/3 (94 suites — P4 base 775 +
  exactly the 10 new pins: 4 rut-vm any-lane arms, 3 rut-std valued
  units, 3 rut-cli valued integrations), the nmapset.rs aliasing pins
  UNCHANGED (checksums 132 / 85 / 7 / 41), wasm32 exit 0; FULL bench
  gate — ALL 28 rows × {rut, node, qjs} = 85 result rows, 0
  mismatches, expected.json UNTOUCHED; VERSION 12 NO BUMP.
- Phase 6 (this commit): docs-only paranoia — workspace 785/0/3
  (the phase-5 count exactly, nothing moved); the engine
  byte-identical to `365e6c2`.

## 7. The errata

**The nmapset-hostops phase-2 "ZERO loops" overclaim.** `docs/
nmapset-hostops-report.md` §1 (commit `54fceca`, whose subject also
says "ONE-CROSSING wrapper (zero loops)") stated `nmapset.rut`
"596 → 310 lines, ZERO loops" — but the same paragraph's own `vstore`
description ("fresh birth appends with the doubling") was the loop:
the grow branch copied the sidecar element-by-element. "Zero loops"
was true of the METHOD BODIES, false of the FILE. The stale sentence
now carries a pointer to this section (the report's history kept);
the claim became TRUE only at THIS batch's P5, when `vstore` and its
doubling copy loop deleted with the sidecar (commit `365e6c2`) — the
wrapper's own header comment records the errata by name.

## 8. The menu (recorded, not landed)

- **Handle recycling under churn** — the monotonic-v1 law's cost is
  now one `Empty` entry per distinct key ever inserted (the sidecar's
  nil slot became the map's own placeholder); a host-side free-list
  (handing a removed handle to a later birth) is the measured
  follow-up if a churn-heavy workload ever prices it.
- **The iteration surface** — still absent; json's map ENCODE still
  waits on it.
- **Crossing-fastpath** — CLOSED on this batch's evidence (§5: no
  regressing row); the recorded baseline for any future revisit is
  P5's extended parity profile (7×3, nmap-hashset medians 40.763 vs
  40.677 ms) plus the P5 movers table — a revisit must name a row the
  profile shows regressing.
- **The `?bool` lane** — unchanged, deferred on the `?bool` nil-law
  checker gap (the round-3 repro stands; a bool val rides the entries
  as Bits today, which the value migration made the only spelling).
- **Arena+vtable payloads / inline-in-entry payloads** — the
  HostOpaque Box and the entry-side Box-in-entry variant stay on the
  menu for a measured need; the VALUE path stays Box-free/alloc-free
  by law.
- **The prim width-packing receipt (§0.8 n)** — P5 landed the
  full-slot repr and the heap outcome over-delivered (nmapset-int
  311 B, BELOW the 343 B no-V floor): array-style width packing has
  nothing left to win; it stays menu-only unless a future repr change
  reopens the question.
