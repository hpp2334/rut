# strbuild survey — phase 0

Batch: **strbuild** (`rut/strbuild`, the 10th std pkg — the engine's
builder grows its std-pkg face). Phase 0 delivers the docs base every
later phase argues from: the census of the engine `StrBuf` machinery as
landed (json-perf phase 2), the member contract named from real need,
the pkg shape, the rut/json migration mapping, and the PRE-REGISTERED
bench design the optimization pass must measure against. Docs only —
zero code changes, zero engine changes, no bench row landed yet.

THE USER DIRECTIVES (law, verbatim intent):

1. "add rut/strbuild, it contains `class StringBuilder { pub fn
   append(value: str), ... }` and so that rut/json can use it."
2. The materialization method is `build() -> str` (NOT finish).
3. "also bench and optimize the perf for it."

THE LANDED STATE this survey censuses (base `5985617`): VERSION 11;
json's writer rides `StrBuf` directly (`rut/json/json.rut` + the two
group files); std mount order json = 9th; the LSP embeds 9 pkgs
(`std_surface`). Scratch probes live under
`/tmp/opencode/batch-strbuild/p0/` (§9) — nothing from scratch lands.

---

## 1. The census — the engine `StrBuf` as landed

### 1.1 The five natives (exact signatures + semantics)

All in `crates/rut-vm/src/interp/native/buf.rs` (dispatch
`native/mod.rs:23`), heap bodies in `crates/rut-vm/src/heap/mod.rs`
(`alloc_str_buf` :255, `str_buf_append` :270, `str_buf_bytes` :294),
block store in `crates/rut-vm/src/heap/blocks.rs`:

| nat (id) | rut spelling | signature | semantics as landed |
|---|---|---|---|
| `StrBufNew` (16) | `StrBuf(cap)` / `StrBuf()` | `(cap: i32) -> StrBuf` | one empty UTF-8 buffer pre-sized to `cap` octets; negative cap traps `Invalid`; the hint is class-rounded by the block store (the charge is what the cell actually holds) |
| `StrBufPush` (17) | `b.push(s)` | `(recv: StrBuf, s: str) -> nil` | append a str/view's octets IN PLACE; traps on non-builder recv or non-string piece (both unreachable through the checker's typing); O(\|s\| amortized |
| `StrBufPushCode` (18) | `b.push_code(c)` | `(recv: StrBuf, c: u32) -> nil` | append one codepoint; invalid scalars (surrogates) mint U+FFFD — the `str.from_code` rule, decided at the nat, not the pkg |
| `StrBufLen` (19) | `b.len()` | `(recv: StrBuf) -> i32` | the CODEPOINT count, tracked at append time — O(1), never a scan |
| `StrBufFinish` (20) | `b.finish()` | `(recv: StrBuf) -> str` | the ONE materialization: a fresh immutable `str` cell with a copy of the octets; the builder KEEPS its buffer — finish twice answers the same text (probed, §9 probe1) |

Encoding facts (the VERSION law's subject, §6): `TyKind::StrBuf` is
type id **17** / kind tag **15** (`rut-core/src/types.rs:264`,
`binary.rs:651/814`); the nat ids are 16–20 (`binary.rs:999`);
`NativeTy::StrBuf` is a declared-surface row in `Surface::core()`
(`binary.rs:283`).

### 1.2 The block-store law underneath (growth + non-movement)

- Size classes (`blocks.rs:34`): 16→2048 in 24 steps (8-aligned,
  coarse above 512), `SMALL_MAX = 2048`; above that, dedicated boxed
  blocks. A block is 1:1 with its owning cell (the cell is the RC
  unit); an 8-byte header carries the CLASS-rounded cap.
- **Growth is geometric with factor 2**, class-rounded:
  `grow()` targets `max(want, 2 * current cap)` (`blocks.rs:171`) —
  appending touches the allocator O(log n) times, then moves
  `old_len` bytes (`copy_nonoverlapping`) and frees the old block.
  Measured (§9 probe2, `RUT_BLOCKS_STATS=1`): a 4-round churn of
  ~30M builder ops produced **68 grows** total (~17 per ~6.3 MB doc
  grown from a 64-byte hint = log₂(6.3 MB / 64 B)). The growth law is
  NOT a hot path; per-append dispatch is (§5.4).
- **Blocks never move** (RFC 0016 OQ-1): raw `&[u8]` into the store
  stays valid for the cell's life — the property the shared-cell law
  (§1.4) rests on.
- `own()` on a builder deep-copies the buffer (`heap/mod.rs:609`) —
  value semantics for the reflection escape only; ordinary binding is
  a share (RFC 0044).

### 1.3 How rut code reaches it today — the declared surface

`StrBuf` is **AMBIENT** (RFC 0028 revised, builtin-surface): it rides
`Surface::core()` inside `mount_std_core`, which every host mounts
unconditionally (`rut-driver/src/lib.rs:505,536`). No `use`, no
manifest row, no dep edge. The mirror for humans and the LSP is
`rut/core/core.d.rut:73` — `builtin class StrBuf { push / push_code /
len / finish }` with the class-aliasing comment. The checker's guards
(`rut-lir`): `StrBuf` takes an `i32` cap or nothing (`call.rs:319`),
no generics (`check/resolve.rs:212`), no impl blocks (`check/collect.rs:467`),
and a fixed member contract at `call.rs:1144` with the exact
"no method" diagnostic naming `push(s)`/`push_code(c)`/`len()`/`finish()`.
Engine tests pin it: `crates/rut-driver/tests/scan_builder_surface.rs`
(content across many appends, pre-sizing, assignment-shares-the-cell).

Probed live (§9 probe1): a bare single file over `mount_std` — no
`use` — constructs `StrBuf(16)`, pushes, push_codes, lens, and
finishes. The ambient claim is not a reading of the table; it runs.

### 1.4 json's writer — every call site + what each costs

`rut/json/json.rut`, three distinct builder lives:

**(a) The writer's accumulator** — field `out: StrBuf` on
`JsonWriter` (:114), minted `StrBuf(1024)` in `new()` (:134):

| line | site | op | cost per hit |
|---|---|---|---|
| 144 | `finish()` | `out.finish()` | 1 nat call + full-octet copy + fresh str cell |
| 164 | `fail()` — FAILURE PATH ONLY | `out.len()` | 1 nat call, O(1) |
| 194 | `value_comma()` | `out.push(",")` | 1 nat call |
| 210 / 222 / 231 / 240 | begin/end object/array | `out.push(...)` × 4 | 1 nat call each |
| 249 | `key()` comma | `out.push(",")` | 1 nat call |
| 254 / 255 | `key()` | `out.push(self.quote(k))`, `out.push(":")` | 1 nat call each (+ quote's own below) |
| 263 | `write_str()` | `out.push(self.quote(s))` | 1 nat call (+ quote) |
| 270 | `write_raw()` | `out.push(t)` | 1 nat call — the numbers/bools fast lane |

**(b) `quote_slow()`** — a LOCAL builder per escaped string (:318),
minted `StrBuf(s.len() * 2 + 2)`:
12 push sites (:319, 320, 326, 328, 330, 332, 334, 336, 338, 346, 349,
353) + 1 finish (:354). The clean prefix appends as ONE piece; the
per-escape-loop body pushes 1-char slices / `\u00xx` f-strings. **This
is the per-string-mint churn shape** — a fresh builder cell + block +
finish copy for every string that contains an escape. The writer's hot
path hits it once per escaped string value or key.

**(c) `unescape()`** — the decode-side builder (:907), minted
`StrBuf(rl)` (the escaped payload's own length — pre-sizing done
right): 9 push sites (:912, 922–929) + **2 push_code sites** (:955 the
surrogate-pair join, :965 the BMP escape) + 1 finish (:974). The
`push_code` pair is the engine's ONLY non-ASCII-appending consumer
today.

Tally: **3 mints, 31 pushes, 2 push_codes, 1 len, 3 finishes** across
the pkg. Cost shape: every push/pay call is one `Op::CallNat` (recv +
args in pooled regs, type checks inside the nat); heap charge lands
only on the grow path (`str_buf_append` charges the capacity delta);
fuel is charged at loop back-edges and allocations (`interp/mod.rs:3`)
— so the writer's fuel is dominated by the surrounding rut loops, not
by the nats themselves.

### 1.5 The class-law share/copy shape — what must not regress

RFC 0044: bindings share cells. Concretely, the chain that makes the
writer cheap:

1. `JsonWriter` instances are cells; `mut self` params share the
   instance — no field-by-field copy anywhere.
2. `out: StrBuf` is a slot to the builder CELL; every
   `self.out.push(...)` appends IN PLACE through the shared cell —
   zero buffer copies on the hot path. Assignment shares (probed,
   §9 probe1 claim 4: appends through an alias visible through the
   original, and the built `str` immune to later appends).
3. Copies happen at exactly two engineered points: `finish`'s one
   materialization (per document / per quoted string), and `grow`'s
   prefix move (O(log n) times per document, measured 68 over §9's
   whole churn).

This is the shape the json-perf phase-2 wins were banked on. The
migration (§4) and the pkg (§3) must preserve it bit-for-bit: the pkg
is a FACE over the same cell, not a wrapper that could re-introduce a
copy.

---

## 2. The member contract — named from real need

The directive fixes two names. The rest are named ONLY where a call
site justifies them (json's own, or a plausible general builder's —
the same standard the json survey used). Everything else is ABSENT on
record, not forgotten.

| member | engine nat | verdict | justification |
|---|---|---|---|
| `append(value: str)` | `StrBufPush` | **IN** (directive) | the writer's 31 push sites are the call-site proof; the general builder's primary verb |
| `build() -> str` | `StrBufFinish` | **IN** (directive; NOT finish) | json's 3 finish sites; the ONE materialization, non-destructive (finish-twice law kept — §1.1) |
| `append_code(cp: u32)` | `StrBufPushCode` | **IN** | json's unescape :955/:965 — a live call site the pkg must carry; a general builder needs codepoint append (the `from_code` U+FFFD rule rides the nat, unchanged) |
| `len() -> i32` | `StrBufLen` | **IN** | json's `fail()` :164 needs the output offset; a general builder's progress queries; O(1) tracked |
| `with_cap(cap: i32) -> StringBuilder` | `StrBufNew` | **IN** | all three json mints pre-size (1024, `s.len()*2+2`, `rl`) — pre-sizing is proven need, and the pkg constructor is the only place to spell it |
| `new() -> StringBuilder` | `StrBufNew` (cap 0) | **IN** | class-construction law (RFC 0010): `StringBuilder.new()` is the mint every consumer writes; grows from small (the `StrBuf(0)` path) |
| `clear()` | — | ABSENT | no nat exists, no json call site, no plausible builder need spelled in this repo (a fresh builder is one `new()` away); adding it would be a NEW nat → VERSION territory (§6) for zero proven need |
| `capacity()` / `reserve()` / `ensure(n)` | — | ABSENT | no nat, no call site; the constructor hint covers every pre-sizing use json has |
| `append_char(ch: str)` (1-char sugar) | — | ABSENT | `append` over a 1-char str is the same nat cost; quote_slow's 1-char pushes (:349) prove the slice form is livable |
| `append_i64/f64/bool(...)` | — | ABSENT | json renders scalars through f-strings into `write_raw` (:273–283); the conversion lives at the caller, no nat for it |
| `build_or_clear` / destructive build | — | ABSENT | the finish-keeps-buffer law is landed engine semantics; changing it is out of scope by §0's no-engine-changes gate |

The class surface is therefore exactly: `new`, `with_cap`, `append`,
`append_code`, `len`, `build`. Six members, every one backed by a
line number or the directive; the engine learns nothing new.

Directive-sketch → real syntax (probed, §9 probe1's first two
compile attempts): rut methods live in an `impl` block, not the class
body (RFC 0012 — "type bodies declare fields only"), and mutating
methods take the `mut self` receiver. The directive's
`class StringBuilder { pub fn append(value: str), ... }` is sketch
notation for:

```rut
class StringBuilder {
    out: StrBuf;          // the engine cell — the whole pkg is this field
}

impl StringBuilder {
    pub fn new() -> Self { ... }              // out: StrBuf(0)
    pub fn with_cap(cap: i32) -> Self { ... } // out: StrBuf(cap)
    pub fn append(mut self, value: str) { self.out.append-src(value); }
    pub fn append_code(mut self, cp: u32) { ... }
    pub fn len(self) -> i32 { ... }
    pub fn build(self) -> str { ... }
}
```

(`append-src` above marks the delegation site — the landed spelling is
`self.out.push(value)`; the point is the pkg method is ONE forwarded
call, no copy, no re-buffering.)

---

## 3. The pkg shape

**Files**: `rut/strbuild/strbuild.rut` (the class + impl, pure rut —
zero host fns, the json-pkg shape) + `rut/strbuild/rut.toml`.

**Manifest** (`rut.toml`):

```toml
name = "strbuild"
entry.lib = "./strbuild.rut"
inline = true
```

- **`[deps]`: NONE.** `StrBuf` is ambient (§1.3) — the pkg compiles
  against `mount_std_core` alone, exactly as json has zero host-fn
  deps. No `[peer-deps]`, no `[dev-deps]` (nothing to integrate with;
  pkg tests ride the driver's program tests, json-style).
- **`inline = true` — load-bearing, not stylistic.** The ink manifest
  states the law verbatim: "`inline` keeps its class methods
  resolvable at every consumer's call site (a class-method module
  cannot be linked)". StringBuilder is a class-method pkg; without the
  flag, a linked consumer's `b.append(...)` fails admission. Same flag
  ink, nmapset, and json carry.

**Mount order — 10th, after json.** The graph loaders need nothing
(no consumer declares it until code uses it — RFC 0045's `[deps]` walk
is position-free). The two PRESENCE lists gain one row each (phase-1
code, listed here as the wiring bill, not done now):
- CLI: `rut-cli/src/main.rs:117` — `("strbuild", "rut/strbuild")`
  appended after `("json", "rut/json")`;
- probe: `benches/probe/src/main.rs:145` — the matching `mount_dir`
  line after json's.

The ordering claim (10th) is documentation law — RFC 0028's amendment
gains the paragraph (the ninth-package precedent, `rfc/0028:14,205`).
No engine table orders std pkgs; "the 10th" names the reading order
and the amendment, exactly as "json is the 9th" does today.

**The mut/class-law shape (how a mut builder param works over the
StrBuf cell).** Under RFC 0044 every binding shares cells; `mut` on a
rut binding/param is the checker's license to mutate through the
shared handle, never a copy trigger. So:

- `let b = StringBuilder.new()` then `b.append(s)` does NOT compile —
  mutation needs `let mut b = ...` (probed shape: json's own
  `let mut b = StrBuf(rl)` at :907).
- Passing a builder to `fn sink(mut b: StringBuilder)` shares the ONE
  instance cell whose `out` slot shares the ONE builder cell —
  appends through the parameter land in the caller's document. The
  directive's "StringBuilder params are mut non-nilable" is exactly
  this: **`mut`** (mutation license), **non-nilable** (`StringBuilder`,
  never `?StringBuilder` — a nil builder is a bug, and `?` would put a
  check on every hot append).
- The two-cell chain (instance cell → StrBuf cell) is what json's
  writer already runs (§1.4's field shape); the pkg adds no layer the
  engine can see.

---

## 4. The migration mapping — rut/json onto StringBuilder

The law: checksum `1960875332163557684` IMMOVABLE; the json-perf wins
(exec **−72.9%** phase 1, **−49.1%** phase 2; fuel −49.0%; heap
−24.8% — `docs/json-perf-report.md:95–96`) must not regress: the
migration's delta must be inside the ±1% gate or honestly disclosed.

Because the pkg is a pure rename face over the same nats (§2), the
mapping is mechanical and the predicted ops delta is **ZERO** — same
`Op::CallNat` stream, same nat ids, same block behavior. Fuel is a
pure op count (RFC 0040): bit-identical, or the migration did
something wrong.

| json site (today) | migrated | nat | delta |
|---|---|---|---|
| `out: StrBuf` field :114 | `out: StringBuilder` | — | type row changes; cell shape identical |
| `out: StrBuf(1024)` :134 | `out: StringBuilder.with_cap(1024)` | `StrBufNew` | +1 rut frame (with_cap → mint); inline=true specializes it away at the consumer — predicted bit-identical fuel, MEASURE at landing |
| `self.out.push(x)` × 31 | `self.out.append(x)` | `StrBufPush` | 1:1 |
| `b.push_code(joined / cp)` :955, :965 | `b.append_code(...)` | `StrBufPushCode` | 1:1 |
| `self.out.len()` :164 | `self.out.len()` | `StrBufLen` | 1:1 |
| `out.finish()` / `b.finish()` :144, :354, :974 | `out.build()` / `b.build()` | `StrBufFinish` | 1:1 |
| `let b = StrBuf(s.len()*2+2)` :318 | `let b = StringBuilder.with_cap(s.len()*2+2)` | `StrBufNew` | as :134 |
| `let mut b = StrBuf(rl)` :907 | `let mut b = StringBuilder.with_cap(rl)` | `StrBufNew` | as :134 |

Scope: `json.rut` only — the two group files (`group-pouch.rut`,
`group-nmapset.rut`) never touch the builder (verified: zero StrBuf
references), they only gain the writer through the trait impls.
`expected.json` moves nothing; the `-72.9%/-49.1%` wins are re-proven
by the `json-roundtrip` row landing inside noise.

**The one real risk**: `with_cap` as a rut frame in a non-inlined
consumer would add a call per mint (json mints per escaped string in
`quote_slow`). Mitigation on record: json is `inline = true`, so
`with_cap`'s body splices into every consumer — the mint lowers to
the same single `StrBufNew`. The landing phase must verify the op
stream (fuel pin bit-identical) or disclose the mover, per the house
law.

---

## 5. The bench design — PRE-REGISTERED (the optimization pass argues against this, not against vibes)

### 5.1 The row

- **Name**: `strbuild` — the pkg's own row (the json-roundtrip
  precedent: the pkg gets a row whose workload IS the pkg's reason to
  exist).
- **Shape**: a SINGLE-FILE workload, `benches/workloads/strbuild.rut`
  + `strbuild.js` — no dir, no manifest: the surface is ambient
  (§1.3), output rides `use ink::{ Logger }` (the presence mount,
  alloc.rut's shape). run.mjs auto-discovers by filename
  (`run.mjs:157`).
- **Pin**: `expected.json` gains `"strbuild": "<checksum>"` after the
  three-runtime agreement (the twin gate, §5.3).

### 5.2 The workload (fragment distribution, append count, build frequency)

The workload must expose BOTH costs the directive names — chunk
growth AND build() cost — so it runs two phases per round:

- **Phase A — churn (fragment distribution)**: an LCG (the
  knucleotide constants, seed pinned) draws fragment lengths from
  {1, 3, 5, 7, 9} chars — odd sizes so pieces straddle the 8-byte
  class rounding — appended one by one into ONE builder pre-sized
  SMALL (`with_cap(64)`), n = 2²⁰ appends per round. The 64-byte hint
  forces ~14 documented geometric grows per MB-scale document
  (log₂ law, §1.2) — growth is INSIDE the measurement, not assumed
  away. Every fragment's content is derivable (a repeated letter per
  length class), so the fold verifies content, not just counts.
- **Phase B — build frequency**: per round, ONE `build()` on the big
  doc (the amortized pattern — json's writer shape), plus a
  **per-fragment-mint sub-phase**: for m = 2¹⁶ short strings, mint
  `with_cap(hint)` → append → `build()` — the `quote_slow` shape
  (§1.4b), the hot path the writer actually hits per escaped string.
  Phase B is what makes `build()`'s copy cost visible; Phase A makes
  the append path and growth visible.
- **Rounds**: 4 (the checksum folds every round's total codepoints +
  a content-derived sample fold — LCG-deterministic, so
  three-runtime agreement is the gate).
- **Fold law**: checksum over ROUND-TRIPPED COUNTS + sampled content
  (the json-roundtrip values-not-lexemes precedent); the output rides
  a rep-stable length assert where lexemes would be runtime-shaped.

Predicted scale (from §9 probe2's scratch lane, DISCLOSED as one
release-run figure, not the bench): ~30M builder ops ran 0.80 s host
wall, ~30 MiB peak, 68 grows. The row's tuned scale targets a rut net
in the suite's mid-pack (hundreds of ms), reps 3.

### 5.3 The .js twin

```js
// phase A: parts.push(fragment) per draw, then parts.join("") per round
// phase B: per string: small array push + join, or (m small) direct
//          concatenation — the twin MUST pick the JS-idiomatic build,
//          and the choice is DISCLOSED on the twin's header comment
```

Same LCG, same draw order, same fold (`String(v)` normalization where
f64 would be involved — none is; this row folds integers + string
codes only). The twin rides `Array#join` — the JS engine's own
rope-flattened build — which is exactly the "what does a mature
engine's idiom cost" baseline the suite exists for.

### 5.4 The predicted fuel/exec profile + optimization candidates, RANKED

Fuel: charged at back-edges + allocations, so the row's fuel ≈ the
append-loop trip counts (phases A+B dominate) + the grow-path
allocations (68-scale events — negligible fuel, real wall time in the
copy). Exec: predicted to be dominated by (1) per-append `CallNat`
dispatch and (2) phase-B's mint+finish churn — NOT by growth.

| rank | candidate | mechanism | predicted yield |
|---|---|---|---|
| 1 | **Per-append overhead** | every `push` pays: recv slot read + recv type `matches!` + piece slot read + piece type `matches!` + `char_len()` + the append. The checker already typed recv and piece — the double check is engine-side re-insurance. Fold to one dispatch (or split nat variants keyed on the verifier's guarantee) | phase A is ~1 nat/append; shaving the checks could plausibly move the row 5–15%. NOTE: any nat-touching change is a §6 VERSION conversation — the fast path must reuse the SAME nat ids or the bump is honest |
| 2 | **build()'s copy shape** | `str_buf_bytes` does `bytes().to_vec()` then `alloc_str_bytes` copies AGAIN — two copies + one intermediate alloc per build. A single-copy mint (allocate the str block, copy once) halves phase-B's materialization work | phase B only: up to ~1–3% of the row; on the json lane, one build per escaped string → the quote_slow path feels it first |
| 3 | **Per-fragment-mint churn** (pkg-level, no engine change) | phase B mints a builder per piece; a pkg-side `append_pieces`/reuse pattern, or json-side quote restructuring, avoids the mint entirely | measured-against-phase-B; could be several % on the row and a real json-quote win — but json's checksum law means the win must show on `json-roundtrip`, inside noise or better |
| 4 | **Chunk size / growth factor** | 2× → 1.5× (less slack, more moves) or class-alignment tweaks | predicted INSIDE NOISE → revert: 68 grows over 30M ops is O(log n) already (§1.2); the pre-registered ledger expects this candidate to die honestly |
| 5 | **Share/copy on hot paths** | already the RFC 0044 law — appends mutate the shared cell; nothing to optimize without breaking the law | zero by design; listed so nobody "optimizes" a copy in |

House method (binding, unchanged): **5×7 paired** interleaved matched
sessions × reps for cross-runtime nets, probe fresh-VM iters in-process,
**±1%** noise gate; inside-noise → REVERT with the analysis kept;
landed-vs-reverted ledger in the report; `json-roundtrip`
`1960875332163557684` re-verified at every engine-touching step.

---

## 6. The calls

### 6.1 VERSION — **NO bump (11 stays)**

The v9/v10 bumps each marked NEW ENCODED VOCABULARY: v10 added the
`StrScan`/`StrStartsWith`/`StrBuf*` nat encodings (16–20), the boot
type (id 17 / tag 15) and the `NativeTy::StrBuf` surface row — "a v9
engine rejects every v10 artifact's new nats/kinds" (`binary.rs:465`).
strbuild adds **zero** encoded vocabulary: no nat ids, no TyKind, no
opcode forms, no surface row. The pkg is ordinary rut source
compiling to existing `Op::CallNat`s over existing ids; a pre-strbuild
v11 artifact decodes byte-identically on a post-strbuild engine, and
vice versa. The decode invariant — the thing VERSION exists to
protect — is untouched by construction. (The opaque-is survey's
standard: state both readings — this is the no-bump reading WITH its
shield intact: old artifacts aren't just "probably fine", they are the
same op stream.)

The LSP re-issue (below) ships new wasm/vsix, but that is a pkg-embed
change, not a binary-format change — the wasm's embedded engine is
still VERSION 11.

### 6.2 The LSP surface — 9 → 10 pkgs, wasm + vsix re-issued

`crates/rut-lsp/src/std_surface.rs` gains (phase-1 code, the wiring
bill):

```rust
pub const STRBUILD: &str = include_str!("../../../rut/strbuild/strbuild.rut");
// + indexes(): ("strbuild", STRBUILD, rut_parser::Mode::Impl, "rut/strbuild/strbuild.rut")
```

- `rut-lsp-wasm` rebuilds through the extension's own lane
  (`npm run build:wasm` — cargo wasm32 release + `copy-wasm.mjs`);
  the wasm's `defs` ride `std_surface::indexes()` (`rut-lsp-wasm/src/lib.rs:61`)
  automatically.
- The vsix re-issues (`npm run package`, version bump — the current
  artifact is `rut-vscode-0.2.2.vsix`); hover/completion/go-to-def on
  StringBuilder verify through the extension's e2e (`test:e2e`).

---

## 7. What phase 0 deliberately does NOT do

- No `rut/strbuild/` files, no manifest, no mount-list edits, no
  std_surface edit — phases 1–3 land those.
- No engine edits of any kind (no nat bodies, no checker, no block
  store) — the §6.1 no-bump call DEPENDS on this.
- No json migration — §4 is the map, not the territory; json's
  sources are untouched this phase.
- No bench row, no expected.json pin, no run.mjs/probe wiring — §5 is
  pre-registration, so the optimization pass cannot be accused of
  designing the course after seeing the runs.
- The demo lane's tree state (`demo/package.json` mod, untracked
  `scripts/`, `.wrangler/`) is a PARALLEL session's — not read as
  signal, not staged, not touched. This phase stages by explicit path
  only: `docs/strbuild-survey.md`.

## 8. Gates run at base (docs-only parity)

Docs cannot move pins, but the gates were re-run as the landing
parity check: workspace green — `cargo test --workspace` (770 passed
/ 0 failed), `wasm32-unknown-unknown` check exit 0. The behavioral
probes of §9 ran against the landed release binary at base.

## 9. Scratch inventory (`/tmp/opencode/batch-strbuild/p0/`)

- `probe1.rut` / `probe1b.rut` — the §1.3/§2/§3 claims executed:
  ambient surface (no `use`), the directive's class sketch corrected
  to fields-body + `impl` + `mut self` (RFC 0012), assignment shares
  the builder cell (alias appends visible through the original),
  `finish` twice answers the same text, built `str` immune to later
  appends, `len` counts codepoints (astral = 1). First two compile
  attempts kept as the syntax-correction record (methods-in-body
  rejected; `\U{...}` is not a rut escape — `\u{...}` is).
- `probe2.rut` — the §5.2 shape rehearsal: 4 rounds × 2²⁰ outer
  appends over the {1,3,5,7,9} LCG mix + per-fragment mint/build,
  content-verified. Release lane (base binary, NOT the house
  method): 0.80 s wall, ~30 MiB peak;
  `RUT_BLOCKS_STATS=1`: **68 grows** across ~30M builder ops — the
  §1.2/§5.4 growth-law measurement. (`allocs` in that stat output
  reads 0 — the counter is only wired on the grow path; disclosed so
  the number isn't misread.)
