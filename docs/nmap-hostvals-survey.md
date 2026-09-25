# nmap-hostvals survey — phase 0

Batch: **nmap-hostvals** (the host owns the table AND the values): the
wrapper's values move host-side into ONE store with TWO entry kinds
(HostOpaque = a Rust payload, RutOpaque = a rut cell held host-side),
`char` finishes dying (the RFC 0004 v1.1 surface cut left the machinery
alive in the IR, the VM, and the boundary), and `nmapset.rut`'s last
loop dies. Phase 0 = the census, the decisive probes, the baselines,
the micro-calls. Docs only — zero code changes.

THE USER DIRECTIVES (verbatim, this batch): *"But what I say is host
part impl, I recommend use HashMap"* — the real-HashMap core (P4);
*"And value can cross with opaque. If slow, we should optimize the
opaque perf"* — values cross; *"If we want to introduce it, I recommend
rename to HostOpaque and replace our current host part opaque
mechanism"* — FULL REPR CHANGE (the arena anchor cell dies); *"And
remove 'char' entirely, we removed it before, but not clean"* — the
exorcism is P1; *"And we may need to change vm since the rc and release
requirement"* — the VM rc work is in scope by directive.

THE LANDED STATE this survey censuses (base `b8ced31`, i.e. post
nmapset-hostops `a445096..b8ced31`): VERSION 11
(`crates/rut-core/src/binary.rs:470` — `pub const VERSION: u32 = 11;`;
MAGIC `RUTC` at `binary.rs:413`); the wrapper is 340 lines
(`rut/nmapset/nmapset.rut`) over 49 bound crossings
(`rut/nmap_host/nmap.d.rut`), the fused h-family
(`nmap.d.rut:157-191`) carrying `HashMap`/`HashSet`, the sidecar the
`vals/vlen/vcap` triple with `vstore`'s doubling copy loop
(`nmapset.rut:200-212`); every opaque still anchors through the arena
(`CellData::HostBoxed` + `OpaqueRef`). The bench baselines below were
measured on exactly this tree.

---

## 1. The char liveness census

The surface died at RFC 0004 v1.1; the machinery below is what survives.
Verdicts: **LIVE** = emitted by a compiler path or reachable from a
compilable program (the emitter cited); **DEAD** = decode/branch that no
new binary can reach (no minter); **MACHINERY** = live code whose char
arm is unreachable-from-surface (the resolver rejects the name, so no
program can produce a char-typed thing) but which the compiler/VM still
executes for OTHER inputs.

### 1.1 rut-core

| site | receipt | verdict |
|---|---|---|
| `PrimTy::Char` variant | `types.rs:16` | MACHINERY — the enum arm every table below matches |
| `is_int()` excludes Char | `types.rs:21` | MACHINERY |
| `name()` → `"char"` | `types.rs:34` | MACHINERY (diag text) |
| wire tag 11 (`to_u8`/`from_u8`) | `types.rs:43/:50`, `binary.rs:611/:779` | **LIVE ON THE WIRE** — opcode 47 (`Conv`) encodes `from`/`to` as `PrimTy::to_u8` bytes (`binary.rs:893`), and every `s.code()`/`s.code_at(i)`/`from_code`/`encode`/`decode` program carries a `Conv{Char→U32}` or `Conv{U32→Char}` (emitters §1.2) — tag 11 rides live `.rutc` streams today |
| `width()` → 4 | `types.rs:62` | MACHINERY (packed-array law) |
| boot type-table entry `push(sym::CHAR, PrimTy::Char)` | `types.rs:300` | DEAD-from-surface — reachable only if a resolver produced `TY_CHAR`, which §1.4 shows cannot happen |
| `TY_CHAR: TypeId = 12` | `types.rs:253` | MACHINERY — imported by `boundary.rs:15`, `lir` emitters |
| `ConstVal::Char` (tag 3) | `binary.rs:62`, encode `:551-554`, decode `:724` | **DEAD** — zero mint sites in the tree (`grep "ConstVal::Char("` finds only `interp/mod.rs:45`'s DECODE); the literal removal means the parser never produces one; tag 3 dies with the wire bump |
| `Op::StrCharAt` (opcode 48) | `ops.rs:294`, encode `binary.rs:894`, decode `:941` | **LIVE** — emitted by five sites (§1.2) |
| `REMOVED_CORE` diagnostic entry | `binary.rs:331` | **LIVE** — the type-position pin (§1.4) |
| `sym::CHAR` ident | `sym.rs:187/:314` | MACHINERY (the diagnostic matches by this symbol) |
| `sym::primitive_ty` does NOT map CHAR | `sym.rs:271-287` | the resolver's prim table never produced `char` even before the removed-core check — the boot-table entry at `types.rs:300` and this table were always disjoint paths |

### 1.2 rut-lir — the emitters (all LIVE, all re-spelled at P1)

| site | what it emits |
|---|---|
| `utf8.rs:90-93` `str_char_at` helper | mints a `TY_CHAR` reg + `Op::StrCharAt` |
| `utf8.rs:125-126`, `:151-152` (`emit_string_encode` passes 1/2) | `StrCharAt` + `to_u32(ch, PrimTy::Char)` = `Conv{Char→U32}` per codepoint |
| `utf8.rs:286-287` (`emit_bytes_decode`) | `Conv{U32→Char}` then `CallNat::Str` (the 1-cp str mint) |
| `call.rs:507-508` (`str.from_code(n)`) | `Conv{U32→Char}` + `CallNat::Str` |
| `call.rs:936-943` (`s.code()`) | `StrCharAt` + `Conv{Char→U32}` — the plan's named pair |
| `call.rs:986-989` (`s.code_at(i)`) | same pair, indexed |
| `slice.rs:139-144` (`s[i]` on str) | `StrCharAt` + `CallNat::Str` (the 1-cp str view) |
| `stmt.rs:296` (`for`-of retype `it == TY_CHAR → TY_STR`) | DEAD CHECK — no expressible `TY_CHAR` iterator exists; the guard can never fire |
| `peephole.rs:716` (dst set), `:879-882` (def/use), `:1002-1005` (read rewrite) | LIVE plumbing — StrCharAt rides the generic peephole machinery; P1 deletes the three arm sites with the op |

The P1 re-spell removes the intermediates: `StrCharAt` itself already
answers a **u32 codepoint in a slot** (the VM §1.3 stores `char as u32
as i64` — `value.rs:103`), so `s.code()` becomes ONE op answering u32
directly, `from_code` builds the str from u32, and the utf8 walkers drop
five `Conv` pairs per codepoint loop. A small fuel WIN on
json-decode/json-roundtrip/fasta/strbuild — disclosed, never a gate.

### 1.3 rut-vm

| site | receipt | verdict |
|---|---|---|
| `ArrKind::Char` | `cell.rs:131`; `ArrKind::of` `:151`; width `:168`; read `:219`; `prim_payload` `:257`; write `:286`; `Opt(Char)` writes `:312`, `:411` | MACHINERY — reachable only via `[char]`/`?char`, uncompilable since the resolver rejects the name (§1.4); dies with `PrimTy::Char` |
| `Value::Char` | `value.rs:13`; eq `:31`; debug `:49`; kind_name `:66` | MACHINERY — produced only by boundary decode paths no tree host reaches |
| `Slot::ch` / `Slot::as_char` | `value.rs:102-104` (`Slot { i: v as u32 as i64 }`), `:108-110` | **LIVE** — `StrCharAt`'s own store path (`step.rs:342+` ends in `Slot::ch`) — note the slot ALREADY carries the codepoint as raw bits; the Rust `char` is a momentary interpreter-side value |
| const-pool decode | `interp/mod.rs:45` (`ConstVal::Char(v) => Slot::ch(*v)`) | DEAD (no tag-3 consts can exist in new binaries) |
| `value_in` char arm | `interp/mod.rs:525` (`(Value::Char(c), PrimTy::Char) => Slot::ch(*c)`) | MACHINERY — fires only if an embedder `call`s with a `char` arg against a TY_CHAR param; no tree surface declares one |
| `Op::StrCharAt` interpreter | `step.rs:342-365` | **LIVE** — mints `*b as char` / `chars().nth(i)`, stores `Slot::ch` |
| `Op::StrCharAt` verifier | `verify.rs:451` | LIVE plumbing |
| `Conv` char arms | `scalar.rs:134-186` `convert()` — **NO char arm exists**; `Char→U32` rides the `(false,false)` int branch as a raw bit trunc | the decisive receipt: the VM's Conv NEVER had char semantics — slots carry codepoint bits end-to-end; P1's re-spell deletes zero VM conversion code |
| host decode helper | `util.rs:10` (`PrimTy::Char => Value::Char(v.as_char())`) | MACHINERY (`slot_to_value` arm; needs a TY_CHAR-typed slot) |
| `expect_kind` char arm | `boundary.rs:72` (`PrimTy::Char if rust == "char"`) | MACHINERY — reached only from `read_checked`/`from_slot` twins |
| `Ret for char` | `boundary.rs:186-197` | MACHINERY — `from_slot`/`into_slot` complete but no tree host uses `char` |
| `CallArg for char` | `boundary.rs:414-417` (`Value::Char`) | MACHINERY — embedder-only |
| `HostParam for char` | `boundary.rs:628-639` | MACHINERY — the fast-lane test exercises it directly (`boundary.rs:934`, `Slot::ch('λ')`) — **P1 retunes this test** |
| `Nat::Str` char render | `native/str.rs:15-17` (`PrimTy::Char` → `alloc_char`) | **LIVE** — the `from_code`/f-string `{c}` mint path |
| `alloc_char` | `heap/mod.rs:161-172` | **LIVE** — and its repr is the punchline: it mints a 1-codepoint **STR cell** (`TY_STR` + `CellData::Str`), never a char cell — char has had no heap repr of its own all along |

### 1.4 The decisive probe — type-position spellability (§0.8 l)

Scratch: `/tmp/opencode/batch-nmap-hostvals/char_let.rut`,
`char_param.rut`, `char_let_init.rut` (never committed), driven by
`cargo run --release -p rut-cli -- run <file>`:

| program | result |
|---|---|
| `let x: char;` | parse error `expected =, found ';'` — the annotation GRAMMAR accepted `char`, the program died on the missing initializer before type-check (rut `let` requires one) |
| `let x: char = 65;` | **`error: char was removed — codepoints are u32: s.code() reads one, str.from_code(n) builds one (RFC 0004 v1.1)`** at the annotation |
| `fn f(c: char) -> u32` | the same dedicated diagnostic at the param |

**The pin:** `char` does NOT type-check in any type position — the
resolver consults `removed_core(name)` BEFORE the primitive table
(`rut-lir/src/check/resolve.rs:172-175`; the map built at
`check/mod.rs:363`, the fn-name twin `call.rs:121`) and the diagnostic
lives in `REMOVED_CORE` (`binary.rs:331`). The parser's grammar accepts
any ident in type-annotation position, but no `char`-typed TyKind is
ever produced: `sym::primitive_ty` has no CHAR arm (`sym.rs:271-287`)
and the removed-core check precedes everything else. P1 therefore lands
the **compile-error pin** variant (a test asserting the diagnostic
verbatim — the probe above IS the expected output); no new "char is
gone" error needs writing, and the lexer's literal diagnostic stays
verbatim (`lexer.rs:207-221` — consumed-and-diagnosed, pushes nothing).
The boot-table entry (`types.rs:300`) and `PrimTy::Char` itself become
deletable once the emitters stop naming the type.

**Where VERSION lives:** `crates/rut-core/src/binary.rs:470` —
`pub const VERSION: u32 = 11` (RFC 0033 §1; checked against MAGIC at
decode, `binary.rs:659-664`). P1 bumps it to 12; the same constant is
the only bump site (crate versions are untouched by wire bumps).

### 1.5 The retune surface (the deletion's blast radius)

`T_STRCHARAT`: `rut-vm-threaded/src/api.rs:179` (op→token),
`native/mod.rs:151` (`h_strcharat`), `:235` (table install),
`wasm/mod.rs:114` (dispatch). Driver disasm: `rut-driver/src/lib.rs:793`
(`strcharat r{dst}, r{s}, r{idx}`). Opcode 48 is REPLACED, never
re-meaninged — old binaries fail on VERSION before the opcode matters.

---

## 2. The repr inventory — every OpaqueBox/OpaqueRef/anchor site

Today's repr: a rut `opaque` value is an 8-byte slot holding
`*const CellVal`; the cell is `CellData::HostBoxed { payload: Box<dyn
Any>, type_name, borrows: Cell<u32> }` (host payload) or
`CellData::OpaqueBox { val: Slot, val_ty }` (a rut value — the RFC 0014
anchor cell); the host holds `OpaqueRef` (an arena+acct+ptr triple that
retains on clone, releases on drop — `arena.rs:351-397`). Phase 2
deletes BOTH variants: the slot addresses a store entry directly
(`OpaqueEntry::Host(HostOpaque)` / `::Rut(RutOpaque { slot })`).

| site | receipt | P2 migration shape |
|---|---|---|
| `OpaqueBox<T>` (alloc/from_handle/with/with_mut/handle) | `heap/hostbox.rs:16-121`; `BORROW_MUT` `:12` | the typed view re-mounts on the store entry: `Opaque<T>` = `is::<T>()`/`downcast_ref` against the entry's `HostOpaque` payload — the checked-borrow law verbatim, new home |
| `CellData::HostBoxed` | `cell.rs:529-532` (variant + the Box-to-keep-CellVal-small note) | DELETED — replaced by `OpaqueEntry::Host` (the Box rides the entry, not a cell) |
| `CellData::OpaqueBox` (the rut-value anchor) | `cell.rs:524-525`; release child `arena.rs:286-289` | DELETED — `OpaqueEntry::Rut(RutOpaque { slot })`; identity IS the held cell, downcast reads the cell's runtime kind |
| `Ret for OpaqueRef` / `Ret for OpaqueBox<T>` | `boundary.rs:239-259` (TRANSFER into_slot, mem::forget), `:261-275` | re-based on the store (handle = store identity) |
| `CallArg for OpaqueRef` / `OpaqueBox<T>` | `boundary.rs:439-442`, `:444-447` | re-based |
| `HostParam for OpaqueRef` / `OpaqueBox<T>` | `boundary.rs:641-660` (nil check stays), `:664-682` (from_handle token stays) | re-based; the nil check becomes the entry's absence test |
| `value_in` Opaque decode | `interp/mod.rs:549-557` (checks BOTH box kinds, retains) | validates `OpaqueEntry` instead of `CellData` |
| `slot_to_value` Opaque arm | `util.rs:24-28` (`heap.opaque_handle(p)`) | mints the store handle |
| exports | `lib.rs:11` (`pub use heap::{OpaqueBox, OpaqueRef, ..}`) | the public names survive P2 re-based (embedders hold them across calls — `host_boxes.rs`'s whole subject) |
| `alloc_host_box` / `alloc_opaque` / `opaque_handle(_take)` | `heap/mod.rs:400`, `:390`, `:494/:504` | the mint path becomes store-entry insertion; `alloc_opaque` (the rut `opaque(v)` box) becomes `RutOpaque` |
| `alloc_opaque_str` (the logger's own mint) | `interp/mod.rs:689` | the logger sink becomes a HostOpaque payload (one per logger) |
| `pool.rs` (the HostBoxed block pool) | `heap/pool.rs` (retire runs the payload dtor in place, `POOL_MAX`/`HIGH_WATER`) | DIES with HostBoxed — the entry's `Box<dyn HostPayload>` is plain Rust malloc; the pool's churn rationale (per-op mints) is exactly what "one payload per map/logger, never per-op" removes |
| the `from_handle` rut-value error | `hostbox.rs:36-41` — "the box holds a rut value, not a host payload — recover it with `downcast<T>` (RFC 0014)" | the two-kind law ALREADY spelled: this error is `OpaqueEntry::Rut` answering a Host decode — the store makes the match exhaustive instead of an error string |
| `rut-std/src/nmap.rs` | 49 `register!` sites; `OpaqueBox<NativeTable>` params in ~40 of them; 48 `with`/`with_mut` borrows; `map_new` mints `OpaqueBox::alloc(vm, NativeTable::new(cap))` `:865-867` | `NativeTable` becomes the HostOpaque payload (one per map); `b.with(_mut)` → the entry borrow; the h/val lanes otherwise untouched |
| `rut-std/src/logger.rs:30` | `vm.alloc_opaque_str(name)` — the logger state is a str-backed opaque | the sink HostOpaque payload |
| tests: `rut-cli/tests/host_boxes.rs` (14 Opaque hits — mint/borrow/Drop/rc laws), `boundary.rs` (4), `fastlane.rs` (4), `reentrancy.rs` (7 — the with_mut-across-nested-call law), `e2e.rs` (`case3_opaque` :126, the RFC 0014 is-law), `rut-driver/tests/host_pkgs.rs` (13), `key_payload.rs` (15 — `Vm::opaque_key_payload` reads the box's payload kind), `rut-lsp/src/std_surface.rs` (pins decls, not repr) | | migrate MECHANICALLY at P2 (behavior-frozen): every pinned law (is-opaque, downcast-miss, Drop-at-rc-0, borrow exclusion, key-payload classification) is repr-independent by design |

---

## 3. Store-home receipts (§0.8 j — default: the slab)

**How slots discriminate refs today: they don't — the bytecode does.**
`Slot` is an untagged 8-byte union (`heap/value.rs:88-97`: `i64 / f64 /
bool / char / *const CellVal`, with the null pointer as "no cell").
Which interpretation applies is STATIC program data: `FuncDef.regs:
Vec<TypeId>` (`binary.rs:33`) types every register; the verifier checks
reads against it (`verify.rs:17-24`, "the verifier guarantees registers
hold their declared types", RFC 0015 §5); the host write-back
pre-freezes the ref verdict into `HostSlot.ret_is_ref` (`interp/host.rs:59-67`,
"the bool packs into the struct's padding hole"). At runtime a ref is
distinguished from a prim ONLY by the code path reading it.

**Where a store index would ride:** the same way — an integer in `.i`
under a ref-kinded static type (the `opaque` TyKind). The machinery
that must agree: `expect_kind`'s Opaque arm (`boundary.rs:75`),
`HostParam::read`'s nil check (`:650-654` — today `slot.r.is_null()`;
a slab index needs an index-space answer to "absent", e.g. the +1 bias
the handle laws use), `value_in`'s retain (`:556`), `slot_to_value`
(`util.rs:24-28`), and `call_host`'s `ret_is_ref` release
(`interp/mod.rs:610-619` — today `heap.release(old)` on the arena cell;
after, the store's rc or nothing, since the register BORROWS the entry
while the slot holds only its index).

**What the two homes cost:**

- **The slab** (a VM-side `Vec<OpaqueEntry>` + free list, entries
  addressed by the slot's index, rc + borrow guards ON the entry): the
  maximal de-anchoring the plan names. One allocation and one
  indirection fewer per opaque (the plan's repr law); `OpaqueRef`'s
  arena+acct back-pointers (3 words, `arena.rs:351-370`) collapse to
  (store-gen, index); borrow guards move from the cell's `Cell<u32>`
  (`cell.rs:532`) onto the entry; death runs the finalize hook + ValSlot
  release walk in ONE place. Cost: every `slot.r` deref in the opaque
  paths retunes to an index read (the §2 table, ~10 sites), and the
  arena's accounting (`HeapAcct`) no longer sees host payloads — the
  store self-reports or charges on insert (a P2 decision, disclosed).
- **A lean CellData variant** (e.g. `CellData::StoreRef { entry: u32 }`):
  keeps arena rc, accounting, identity, and the release walk for free;
  the anchor survives as a 16-byte cell — cheaper than today's
  HostBoxed (no Box, no type_name, no borrows word) but still ONE cell
  and one arena round-trip per opaque, which is precisely the
  indirection the plan's repr law deletes.

**Recommendation: the slab** (the default, confirmed). Receipts: (a)
the anchor's only runtime services are rc + borrow-guard + death — all
three move to the entry wholesale; (b) opaque churn is per-MAP not
per-op after P4/P5 (49 crossings, one NativeTable payload each), so the
arena free-list recycling the cell enjoyed (`Arena::drop` walk,
`arena.rs:161-202`) buys nothing; (c) the two-kind store needs a tag
ANYWAY (the `[?V]` repr moved host-side, `ValSlot`), and a CellData
variant would still need the entry enum inside it — the cell would be
a shell around the same shape; (d) `OpaqueRef`'s Drop-runs-release
contract (`arena.rs:387-393`) is the one behavior the slab must
re-implement (the handle's own rc), and the store's generation counter
answers it with fewer moving parts than an arena reference triple.

---

## 4. The any-arm receipts (§0.8 a, g, h + the P3 surface)

**No any-value ARG arm exists.** The arg repertoires: `CallArg`
(embedder `call`, `boundary.rs:376-447`) covers int widths, f32/f64,
bool, char, String/&str, Vec/&[u8], OpaqueRef, OpaqueBox — no `Value`
impl; `HostParam` (host-fn params, `:523-750`) — same, no `Value`, no
tagged-slot capture. The only `Value` crossing is the READ direction:
`impl Ret for Value` (`boundary.rs:366-371`, `from_slot` →
`slot_to_value`, the positional decode).

**Can a `register!` host fn return `Value`? It compiles and then traps
on every call.** `Value: Ret` is satisfied (the impl exists), so
`register!(hosts, "f", () -> Value, ..)` type-checks; but the adapter's
write-back calls `out.into_slot(vm)` (`boundary.rs:789/:821`) and
`Value` has NO `into_slot` — the trait default fires
(`boundary.rs:37-43`: "`Value` does not cross as a host-fn argument or
return"). A register!-host returning `Value` is a compile-ok,
runtime-always-trap TODAY. P3's any-answer is a DIFFERENT shape (the
V-typed register write), but this trap is the receipt that the write
direction is genuinely missing, not merely unexercised.

**How the host-call path exposes the call site's static TyKind: it
already does.** `call_host` (`interp/mod.rs:572-622`) slices the
program's argv table (`prog.funcs[f].argv[argv_off..argv_off+argc]`) to
snapshot arg registers — the SAME `FuncDef` carries `regs: Vec<TypeId>`
(`binary.rs:33`), so `(slot, TyKind)` for argument i is
`(snapshot[i], prog.funcs[f].regs[args[i]])` with zero new plumbing.
The binding's own row types ride `HostSig` (`interp/host.rs:31-53`,
derived from the Rust shape, verified against the `.d.rut` at the join
— `verify_against`, RFC 0025). For `v: any` the ROW's param type is the
new any id; the CALL SITE's static V comes from `regs` — exactly the
plan's "(Slot, TyKind) where the TyKind comes from the CALL SITE's
static type".

**Does the .d.rut grammar admit `any`? The grammar is a whitelist, and
the parser is not the gate.** `.d.rut` surfaces parse in `Mode::Decl`
(`decl.rs:69`) and the type arrives as TEXT (`ty_text`, `decl.rs:44-53`
— any single-segment path parses); admission is
`crossing_ty(name)` (`decl.rs:22-39`: nil, bool, str, bytes, f32, f64,
i8..i64, u8..u64, opaque — 16 spellings, no `char`, correctly). Sizing
the `any` change: ONE arm in `crossing_ty` + ONE boot TypeId + the
checker's any-arg coercion + the adapter's tagged capture. **Collision
receipt:** `TY_ANY` is TAKEN inside rut-vm as the untyped SENTINEL
`u32::MAX` (`interp/mod.rs:189`; guards at `:238`, `:763`, `:775`;
`ArrKind::of`'s sentinel fallback `cell.rs:138-141`) — the new boot id
must be a normal small integer under a distinct name (the sentinel
checks read `!= TY_ANY`, so a boot `any` would invert them); P3 names
it deliberately (e.g. `TY_VAL`) and leaves the sentinel alone.

**The answer-direction register write for narrow numerics — already
the ArrGet law.** `call_host` writes the returned slot straight into
the dst register (`interp/mod.rs:610-619`), releasing the old only when
`ret_is_ref`; `into_slot` for i32 is `Slot::int(self as i64)`
(`boundary.rs:117-119`) — the SAME 8 bytes an i64 answer produces. A
narrow `i32` V register holds the word; no width conversion exists to
skip. `Empty` trapping loudly (§0.8 g) and f64 riding the `f` field
(§0.8 f) are P3 additions on this exact path.

---

## 5. The finalizer site (§0.8 i; RFC 0016 §3)

- **The release walk:** `release_ref_slot` (`arena.rs:208-236`) → at
  rc-0 (after the `on_drop` pin check `:219-223`) → `release_cell`
  (`arena.rs:304-349`): children collected FIRST
  (`collect_ref_children` `:247-292` — record fields via the per-type
  `ReleasePlan.record_refs`, array elements via `is_ref`, the
  OpaqueBox val `:286-289`, StrView/ArrView parents, closure captures),
  then the payload freed, accounting refunded (`acct.used -= bytes`
  `:349`), `drop_in_place`, the cell onto the free list, children
  released recursively (the nested-release precedent — a store entry's
  `ValSlot::Ref` release joins this pattern, and an entry holding a
  rut record recurses through the SAME walk).
- **Where the HostBoxed arm dies:** TWO arms — `release_cell:340-347`
  (rc-0 death: the payload parks via `pool::retire`, whose
  `drop_in_place` IS today's "destructor", `pool.rs` retire) and
  `Arena::drop:180-188` (teardown, same retire). P2 replaces both with
  the store-entry death: `entry.finalize(heap)` (the `HostPayload`
  hook, no-op default) then the ValSlot release walk, then the Box's
  own Drop. The RFC 0016 §3 ordering ("host opaques run their Rust Drop
  at the same point fields are released", rfc/0016 §3 items 1-4) is
  preserved by construction — finalize runs BEFORE the held cells
  release, mirroring "dispose first, fields second".
- **HeapAcct balance:** `used`/`peak`/`limit` (`heap/mod.rs:33-55`),
  refunded per cell death (`arena.rs:348-349`). After the slab lands,
  host payload bytes leave `HeapAcct` — the store must charge/refund on
  entry insert/death (or disclose the exclusion) so the RFC 0040 budget
  and the bench `heap_peak_bytes` receipt stay honest.
- **The VM-exit leak check: none exists today** (grep for
  leak/residue/outstanding in arena/heap/interp: zero). The natural
  site: `Arena::drop` (`arena.rs:161`) already walks every live cell —
  the debug check joins it (`debug_assert!(acct.used == 0)` + the
  store's live-entry walk asserting every `ValSlot::Ref` is reachable),
  plus the store's own Drop walk for entries the arena never saw. A
  `#[cfg(debug_assertions)]` gate keeps release builds clean.

---

## 6. The bench V-kind table + baselines (this tree, `--iters 1`, rut engine)

| row | V kind | gate role | fuel | heap_peak_bytes |
|---|---|---|---|---|
| nmapset-int | prim (`HashMap<i32, i32>`) | the prim-V inlining gate — heap must DROP at P5 (sidecar cells → bits) | 21,571,682 | 1,966,663 |
| nmapset-str | prim V, str K (`HashMap<str, i32>`) | the str-key/hash lane (K side) — V inlining rides the same law | 9,910,968 | 984,086 |
| nmap-hashset | NO V (`HashSet<i32>`) | the h-family control (untouched by P3-P5; 615 B heap = the no-V floor) | 12,716,782 | 615 |
| nmap-knucleotide | prim (`HashMap<str, i32>`) | str-key + counting, sv-adjacent | 37,419,557 | 2,229,052 |
| kmer-view | prim (`HashMap<str, i32>`, view keys) | **the sv/view-arg gate** — a view arg must store the view's cell (aliasing = the view) | 35,019,405 | 2,228,884 |
| strview | prim (view keys) | the view-alias parity twin (checksum = knucleotide's family) | 11,094,312 | 2,032,285 |
| refvals | **RECORD** (`HashMap<i64, Pt>`) | **the alias gate** — `get -> ?V` write-through is checksum-load-bearing; `Ref` storage must preserve cell identity | 55,633,341 | 26,843,812 |
| json-decode | record-V inside (`HashMap` decode rows) | the decode path + the char-exorcism fuel mover (utf8 walkers) | 111,322,915 | 34,377,027 |
| json-roundtrip | record-V inside | **the record-V gate** (the plan's name) + the utf8 mover | 31,975,807 | 3,423,706 |
| fasta | (no map) | the char-exorcism fuel mover (codepoint-heavy) | 200,029 | 16,806 |
| strbuild | (no map) | the char-exorcism fuel mover (`Nat::Str` char mints, `alloc_char` churn — 22 MB heap is 1-cp str cells) | 162,285,165 | 22,028,108 |

Receipts under `/tmp/opencode/batch-nmap-hostvals/bench-*.json`.
The prim-row law's shape is visible NOW: nmap-hashset (no V) = 615 B
vs nmapset-int (prim V sidecar) = 1.97 MB — the sidecar's ?V cells ARE
the delta P5 deletes; refvals' 26.8 MB is the record-mint churn the
`Ref` (retain-the-arg's-own-cell) law must keep honest while deleting
the wrapper-side store indirection. Checksums: expected.json UNTOUCHED
all batch.

---

## 7. The suite-fate census (§0.8 d)

| suite | what it spells (receipts) | fate |
|---|---|---|
| `rut-driver/tests/nmapset.rs` | the class surface: the union-bound compile refusal + `with_capacity` through `map_cap` + the `get`-staleness ALIASING pins (its header :2-11) | **KEPT** — the aliasing pins are P5's gate ("nmapset.rs's aliasing pins UNCHANGED"); the refusal law and the pins are repr-independent |
| `rut-driver/tests/nmap_typelanes.rs` | the 15 typed lanes + Opaque lanes + sentinel/grow/drain vocabulary (its SRC const spells `map_entry_i..map_remove_y`, `map_take_reloc`, `map_needs_grow` :14-17) | **RETires at P4** (its subject — the 25 slot/sentinel/grow/drain lanes — is deleted). Migrating laws: "lanes agree with the fused family" and "hash bits = mix64/FNV-1a pinned" → `nmap.rs` unit tests (several already exist: the sv≡s, legacy-agreement, packed-answer pins from hostops P1) |
| `rut-driver/tests/nmap_valcolumn.rs` | the RAW val column: `map_val_set_u/get_u` over `map_entry_i` SLOT indices, the 9999 out-of-range trap, the grow sweep (`:34-137`) + a checksum leg through `HashMap<i64,i64>` | **RETIRES at P4** (§0.8 d) — `map_val_*` re-bases on HANDLES; the slot-addressing laws die. Its parity law (raw column ≡ `HashMap<i64, i64>`) migrates to an `nmap.rs` unit test over the handle-rebased lanes; the checksum leg is already the `refvals`-family's job |
| `rut-driver/tests/opt_prim_store.rs` | the RFC 0044 §5 `[?prim]` element store over `pouch` (NOT nmap) — the store elision, deref fold, VERSION-6 rejection | **UNAFFECTED** — censused because the `[?V]` sidecar and `ValSlot`'s Bits/Ref split mirror its repr laws; the suite keeps passing untouched (the store stays; only the wrapper's USE of it dies) |
| `rut-driver/tests/nmap_viewkeys.rs`, `nmap_primmap.rs` | the range-method suite (`HashMap<str, i32>` + put_range, pin 2572351) / the sidecar twin (retired row's class surface) | KEPT — range methods stay on the class (the sv twins survive P4); `nmap_primmap.rs` keeps its `HashMap<K, i64/u64/f64>` pins (P5 touches the impl, not the surface) |
| `rut-lsp/src/std_surface.rs` | pins: HashMap/HashSet present, PrimMap* absent (:125-131), NO alias rows (:134-147), **`map_entry` present in nmap_host's index (:148-153)** | the `map_entry` pin RETUNES at P4 (the round-1 OpaqueRef lane dies — pin a surviving decl, e.g. `map_hput_i` or `map_new`); the rest hold. The `integrations/vscode-extension/test/e2e-wasm.js:215` PrimMap negative pin holds automatically |

---

## 8. The errata (for P6 to disclose)

**The nmapset-hostops phase-2 "ZERO loops" overclaim.**
`docs/nmapset-hostops-report.md:53-54` (commit `54fceca`, whose subject
also says "ONE-CROSSING wrapper (zero loops)") states `nmapset.rut`
"596 → 310 lines, ZERO loops" — but the same paragraph's own `vstore`
description ("fresh birth appends with the doubling") is the loop: the
grow branch copies the sidecar element-by-element,
`rut/nmapset/nmapset.rut:201-211` (unchanged since `54fceca` — `git log
-- rut/nmapset/nmapset.rut`):

```rut
if (self.vlen == self.vcap) {
    let next_cap = self.vcap * 2;
    let mut next: [?V] = [nil; next_cap];
    for (let i = 0; i < self.vlen; i += 1) {   // ← the surviving loop
        next[i] = self.vals[i];
    }
```

"Zero loops" was true of the METHOD BODIES (every map op is one
crossing + a store), false of the FILE. The doubling copy loop and the
sidecar itself die at THIS batch's P5 (the value crosses inside the
crossing; `with_capacity`'s `map_cap` pre-size goes with it). P6's
report discloses the errata against the phase-2 record verbatim.

---

## 9. The §0.8 decisions (each confirmed or overturned, with receipts)

- **(a) the decl spelling `any` (host-decl-only)** — CONFIRMED. The
  `.d.rut` grammar is a text whitelist (`decl.rs:22-39`), so `any` is
  +1 arm with zero parser work; the resolver never sees decl-mode
  types, so no surface leakage. NEW RECEIPT: the name must avoid the
  VM's `TY_ANY = u32::MAX` sentinel (`interp/mod.rs:189`) — P3 picks a
  distinct boot id.
- **(b) h-family KEPT (handle-only escape hatch)** — CONFIRMED. The 18
  lanes (`nmap.d.rut:157-191`) are the only value-free crossings; P5's
  wrapper keeps them spelled (the plan's wrapper sketch calls
  `hvput/hvget/hvremove` — they BECOME the primary surface; "kept"
  means the decls stay after P4's 25-decl cull).
- **(c) `map_val_*` handle-rebased, exact-arm match** — CONFIRMED. The
  four decls (`nmap.d.rut:119-132`) re-point at handles; the
  slot-index semantics die with `nmap_valcolumn.rs` (§7). Trap-on-kind-
  mismatch rides `ValSlot`'s tag (Empty/Bits/Ref), never reinterpret.
- **(d) suite fates** — CONFIRMED with one addition: `nmap_valcolumn.rs`
  retires, `nmap_typelanes.rs` retires (both §7), `opt_prim_store.rs`
  UNAFFECTED (censused, untouched), `std_surface.rs` needs ONE pin
  retune (`map_entry` :148-153) beyond the name-based pins the old
  survey called automatic.
- **(e) VERSION 12 at P1** — CONFIRMED. `binary.rs:470`; the wire moves
  (tags 3/11 die, opcode 48 replaced) and the store repr rides the same
  bump at P2 with no second bump.
- **(f) f64 rides the slot's `f` field** — CONFIRMED. The untagged
  discipline (`value.rs:88-97`; `Slot::float` writes all 8 bytes) and
  `into_slot(float)` (`boundary.rs:155-157`) already do exactly this.
- **(g) `ValSlot::Empty` traps loudly on hvget** — CONFIRMED. The
  h-family's fresh-birth+valued-read misuse is a caller bug; the
  `Value`-return trap precedent (`boundary.rs:37-43`) shows the house
  style for loud impossible crossings.
- **(h) numeric answers follow the untagged-slot discipline** —
  CONFIRMED. `call_host`'s register write does no conversion
  (`interp/mod.rs:610-619`); i32 V = the same word (§4).
- **(i) the release-context law + the VM-exit leak check** — CONFIRMED,
  sites pinned (§5): in-crossing via `&mut Vm` (`heap.retain/release`,
  `heap/mod.rs:449/:489`), at death via the entry finalize walk joining
  `release_cell`'s pattern, the leak check in `Arena::drop` (none exists
  today — added debug-only).
- **(j) the store's home** — CONFIRMED: the slab (§3's receipts; the
  lean-CellData variant keeps the anchor the plan deletes).
- **(k) HostOpaque payload = `Box<dyn HostPayload>`, one per
  map/logger** — CONFIRMED. The churn rationale for `pool.rs`'s
  existence (per-op mints) is exactly what dies; the Box stays on the
  Host path only; `ValSlot`/`RutOpaque` stay Box-free by law (the
  8-byte slot moves, §4).
- **(l) the char census receipts** — CONFIRMED: type positions DIAGNOSE
  today (the probe, §1.4) — P1 lands the pin test, no new error;
  opcode 48 REPLACED; the `Conv` table has NO char arms to delete
  (`scalar.rs:134-186` — the surprising zero-cost finding); the
  threaded handler retunes (`T_STRCHARAT`, §1.5); tag 11 is LIVE ON THE
  WIRE through opcode 47's operands (§1.1) — the bump is load-bearing.
- **(m) the two-kind law** — CONFIRMED. The split is already spelled in
  the repr: `from_handle`'s error names the rut-value box
  (`hostbox.rs:36-41`), `value_in` matches BOTH cell kinds
  (`interp/mod.rs:551`), RFC 0014's amendment history names
  `RutOpaque`. TypeId stays Host-path-only; RutOpaque consults the
  cell's runtime kind; the any-arms consult `FuncDef.regs` (§4).
- **(n) prim value storage width** — CONFIRMED DEFAULT: the full 8-byte
  slot (one repr for all prims; the copy/alloc laws' "the 8 bytes move
  as-is"). The P5 heap receipt decides the packing menu:
  nmapset-int's 1.97 MB (sidecar cells) must drop toward the hashset
  615 B floor; array-style packing (`ArrKind::width`, `cell.rs:164-172`)
  stays the alternative if the drop under-delivers.

---

## 10. The calls

1. **P1 (char exorcism)**: the u32 re-spell of the five emitter sites
   (§1.2) + the deletions (§1.1/§1.3 MACHINERY/DEAD arms) + opcode 48
   replaced + VERSION 12 + the type-position pin test (the probe's
   diagnostic verbatim) + the `boundary.rs:934` fast-lane test retune.
2. **P2 (the store)**: the slab + two-kind entries (§3) + the §2
   migration table + the finalize hook at the §5 sites + the
   accounting disclosure.
3. **P3 (any arms)**: `(Slot, TyKind)` from `FuncDef.regs` (§4), the
   boot `TY_VAL`-shaped id (not the u32::MAX sentinel), the
   `crossing_ty` arm, the answer write per `call_host`'s existing law.
4. **P4/P5**: per the plan, with the §7 retirements and the §6
   baselines as the measured gates (checksums immovable; the prim-row
   heap drop is the receipt).
