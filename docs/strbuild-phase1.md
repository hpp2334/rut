# strbuild phase 1 — the landing record

Batch: **strbuild**. Phase 1 lands the 10th std pkg, migrates json's
writer onto it, wires the mount/presence/RFC/LSP surfaces, and pins
the units. The survey's phase-1 calls (docs/strbuild-survey.md) are
the contract; this record carries the one place reality answered
differently than the survey predicted — with the numbers.

## What landed

- **The pkg**: `rut/strbuild/strbuild.rut` + `rut/strbuild/rut.toml` —
  `pub class StringBuilder { out: StrBuf; }` + the six-member impl
  exactly as censused (survey §2): `new`, `with_cap(cap)`,
  `append(value: str)`, `append_code(cp: u32)`, `len() -> i32`,
  `build() -> str`. Deps NONE (`StrBuf` is ambient, RFC 0028
  builtin-surface); `inline = true` load-bearing (a class-method
  module cannot be linked). `clear`/`reserve`/`capacity`/
  `append_char`/`append_i64` stay ABSENT on record. No engine change
  of any kind; VERSION stays 11 (zero new encoded vocabulary).
- **The migration**: `rut/json/json.rut` — every builder call site
  rides StringBuilder per the survey §4 mapping: the field
  (`out: StringBuilder`), the three mints (`with_cap(1024)` /
  `with_cap(s.len()*2+2)` / `with_cap(rl)`), 31 `push` -> `append`,
  2 `push_code` -> `append_code`, `len` unchanged, 3 `finish` ->
  `build`. `rut/json/rut.toml` gains the REQUIRED dep
  `[deps] strbuild` (the writer cannot exist without the cell; the
  peer groups are untouched — they never touch the builder).
- **The wiring**: the CLI presence list gains
  `("strbuild", "rut/strbuild")` 10th after json
  (crates/rut-cli/src/main.rs); the probe's mount lane gains the
  matching `mount_dir` row (benches/probe/src/main.rs); RFC 0028 is
  amended — `strbuild` joins the swappable set as the tenth package
  (the §-level amendment, the ninth-package precedent's shape); the
  LSP std_surface embeds the pkg (`STRBUILD` include + the indexes
  row, crates/rut-lsp/src/std_surface.rs, 9 -> 10 with the surface
  test retuned and a six-member completion pin added).

## THE DEVIATION — the survey's ZERO-ops-delta prediction is FALSE

The survey predicted the migration would be fuel bit-identical
("same CallNat stream"), mitigated by `inline = true` specializing
`with_cap` away at the consumer. **Measured: the prediction fails,
structurally.** Two mechanisms the survey's model missed:

1. **The instance-cell hop.** `out: StringBuilder` inserts ONE record
   cell between the writer and the engine cell (writer -> StringBuilder
   instance -> StrBuf — three cells, not the survey's two). Every
   member call pays one extra `getf` through the instance.
2. **The inlined frame's arg re-bind.** The checker inlines `append`'s
   body at the site (P1.3, receiver-call), but the native `CallNat`
   ABI re-binds the forwarded argument: one extra `movref` per
   argument-carrying call.
3. **Static mints do not ride P1.3.** `StringBuilder.with_cap(c)` is a
   static call — it emits a real `call` + callee frame
   (`callnat StrBufNew`, `makerecord`, `ret`): +3 executed ops per
   mint. The survey's "inline=true specializes it away" is false for
   static calls (verified in IR: the `call f1` stays).

`rut dump` receipts (scratch A/B, /tmp/opencode/batch-strbuild/p1/):
direct `self.out.push(t)` lowers `getf; movref; callnat StrBufPush`;
the migrated `self.out.append(t)` lowers `getf; movref; getf; movref;
callnat StrBufPush` — **+2 ops per append, exactly** (300k-append A/B:
4,500,017 -> 5,100,023 fuel = +2.000/append).

### The numbers (rut-bench-probe, this machine)

| row | base fuel | migrated fuel | delta |
|---|---|---|---|
| json-roundtrip | 31,543,783 | **31,975,807** | **+432,024 (+1.370%)** |
| json-decode | 111,322,915 | 111,322,915 | **0 (bit-identical)** |

json-decode's exact zero is the mechanism's proof: the generated doc
has no escaped strings, so `unescape` (and the encode side) never run
— the delta lives precisely on the builder member-call path, and the
decode reader pays nothing it does not spend.

Exec (paired 5x, back-to-back): base min/median 110.88/111.08 ms ->
migrated 114.12/115.99 ms: **~+3-4%**, outside the ±1% gate — the
honest disclosure the survey itself wired ("inside the ±1% gate or
honestly disclosed"). The −72.9%/−49.1% json-perf wins stand on the
era's own numbers (those pins are that batch's record); the new era's pin is **fuel 31,975,807, heap 3,423,706 B** (+64 B — the
instance cells alive at peak), exec ~115 ms on this host. Where the cost lives and how to take it back:
survey §5.4 candidate 1 (per-append dispatch) — an engine-side fold
that reuses the same nat ids would close most of the gap; until then
the pkg face is worth one `getf`+`movref` per append, disclosed.

### What did NOT move — the immovables held

- **Checksum `1960875332163557684`** (json-roundtrip): IMMOVABLE, held
  exactly (the fold is over round-tripped VALUES; the op stream feeds
  the same nats with the same bytes).
- json-decode checksum `4502015958359127277`: held.
- **expected.json: untouched.** The full bench suite (all 27 rows x
  rut/node/qjs) agrees with expected.json and cross-runtime, exit 0 —
  every row bit-identical on content.
- `docs/json-perf-report.md` / `benches/README.md`'s 31,543,783 pins
  are that batch's historical record — not rewritten; this document is
  the migrated era's pin.

## The unit pins (crates/rut-driver/tests/strbuild_pkg.rs + data/strbuildpkg)

- append/len/build round trips: ASCII, astral (`len` counts 1 per
  astral codepoint), and the empty builder (`build()` answers `""`).
- The codepoint rule: `append_code` astral = 1 codepoint / 4 octets; a
  surrogate (`0xD800`) mints U+FFFD at the nat — the `str.from_code`
  rule, decided below the pkg.
- The growth law: 2^17 appends over a `with_cap(8)` hint — ~16
  class-rounded geometric grows — answer byte-exact content (head,
  middle, tail spot-checks).
- The share/copy law (RFC 0044): an alias (and a
  `fn sink(mut b: StringBuilder)` parameter) appends through the ONE
  instance cell — visible through the original; `build()` twice
  answers the same text; the built `str` is immune to later appends.
- with_cap, the truth the engine admits: honored as the mint
  allocation, **advisory as behavior** — cap 0, explicit 0, and 4096
  answer identical content and len; a negative hint is the nat's
  `Invalid` trap, unchanged by the face (`StrBuf(cap): capacity must
  be >= 0`).

## Gates

- `cargo test --workspace`: **779 passed / 0 failed** (770 base + 8
  strbuild pins + 1 retuned std-surface test + the six-member pin).
- `cargo check --workspace --target wasm32-unknown-unknown`: exit 0.
- Full bench suite (`node benches/run.mjs`): all rows checksum-agree
  with expected.json and cross-runtime, exit 0; expected.json
  untouched; `1960875332163557684` immovable.
- LSP lane: `npm run build:wasm` rebuilt the artifact (embeds the
  10-pkg surface; `bin/rut-lsp.wasm` md5
  `b61f653e00b10a048ce991b9eae7b0b2`); `test:grammar` 85 files / 0
  violations; `test:e2e` — **85 corpus files, 0 false diagnostics**
  through the REBUILT artifact, 22 smokes green; vsix re-issued
  0.2.2 -> 0.2.3 (`rut-vscode-0.2.3.vsix` md5
  `ecf522b72cb56162a566f8f6e883d0fe`; both artifacts are gitignored
  build outputs — the md5s are the receipt).
- The parallel deploy lane's files (`demo/package.json`,
  untracked `scripts/`, `.wrangler/`) untouched, not staged; staged by
  explicit path only.
