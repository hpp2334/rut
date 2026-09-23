# The orphan rule — phase 0 survey: the 76-impl census (zero orphans), the origin-map design, the §2a diagnostic, the honest coherence ledger, the VERSION 9 call

*Phase 0 of the orphan-rule batch (docs only — no code). Base `963ed33`,
binary `VERSION 8` (`crates/rut-core/src/binary.rs:431`), bundles v3,
9 std pkgs embedded (`core`, `calc`, `nmap_host`, `nmapset`, `pouch`,
`ink`, `rt`, `bench_cross`, `json`). Scratch:
`/tmp/opencode/batch-orphan-rule/p0/` (the census dump
`impl-lines.txt` and the two live probes). Everything below cites the
tree as it stands; the design sections propose, they do not describe.
Phases 1–2 implement; this document is their contract.*

---

## 0. The law (the user's ruling — verbatim, not negotiable)

For every `impl Trait for Type { .. }` — **at least one of `Type` (the
self type) or `Trait` must be DEFINED IN THE CURRENT PKG.** Both
foreign = a compile error with a **dedicated diagnostic** (house style:
one diag, naming its RFC — 0012, where the amendment lands).

The locked readings, embedded verbatim:

- **"In current pkg" = declared by a source of this pkg**, tracked
  through the MOUNT/SPLICE model (consumer units splice pkg sources
  in — the compiler must know each definition's ORIGIN pkg despite
  splicing). Locality is a **pkg-level** property; `Module.spec` *is*
  the exact pkg name (`crates/rut-driver/src/session.rs:55–57`), and
  the mount model is one module per pkg (entry source + appended peer
  groups, RFC 0045 §3), so pkg identity and module spec coincide today
  — the design keeps them distinguishable anyway (§2.4).
- **Builtin types (`[T]`, the primitives, `?T`, `opaque`) are in NO
  pkg.** A builtin self type with a FOREIGN trait is an orphan error;
  only a LOCAL trait may be implemented for a builtin (json impling
  `JsonSerialize` for `[T]`: trait local — legal). Note the asymmetry,
  which is RFC 0012's own: builtin **traits** (`Iterator`, `Index`,
  `Disposal`) are *core's decls* (RFC 0012 §2: "both are core decls
  like every prelude name") — their origin pkg is `core`, never "no
  pkg". Builtin **types** have no origin pkg at all.
- **Generic impls: locality is of the HEAD.** `impl JsonSerialize for
  Vec<T>` is local exactly when `Vec`'s declaration is; the type
  PARAMETERS (`T`) never satisfy locality. A `?T`/`[T]` head peels to
  its element: the element is a type parameter, so the head's locality
  is "builtin — no pkg" (§2.6's classification).
- **The origin survives splicing.** An orphan written in a consumer
  unit against two foreign pkgs errors — the consumer unit is where
  spliced text compiles, and that is precisely where origins must be
  known (§2's whole problem).
- **The peer gate precedes the orphan check** (the RFC 0012 amendment's
  own ordering law, recorded Sep 2026: "the gate precedes the orphan
  check") — the checker never sees a half-mounted world.
- **The two rut-json checker arms landed** (`TyKind::Opt` impl targets,
  `collect.rs:510–529`; type-param static receivers, the `Self`-obj
  match in `collect_impl_trait`, `collect.rs:783–809`) — the orphan
  check classifies `?T` heads through the same template shapes, not
  around them.

Today's written law is RFC 0012 §2: "There is no orphan rule beyond
placement" — trait impls are legal in any module, the only constraint
being one impl per `(trait, type)` pair (§5, a link error). The
orphan rule is therefore an **amendment to RFC 0012** (§3 of this
document lands it as §2a), narrowing "any module" to "any module of
the trait's pkg or the type's pkg".

---

## 1. The impl census (repo-wide)

### 1.1 Method and scope

Every `impl` block in every tracked `.rut` source, found by
`grep -rn '^\s*impl\b' --include='*.rut'` (dump:
`/tmp/opencode/batch-orphan-rule/p0/impl-lines.txt`), then each block
classified by resolving **both sides of the head** to their declaring
file/pkg — verified by grepping each trait/type's declaration, not by
trusting the comment prose. Counted: **76 impl blocks in 28 files**:
**27 inherent** (`impl T { .. }`) and **49 trait impls** (`impl I for
T { .. }`). The LSP corpus (the `corpus.rs` roots: `examples/`,
`demo/src/examples/`, `rut/`, `benches/workloads/`) is **70 files
today** — see the disclosure in §1.6.

### 1.2 The trait impls — 49, classified

| Site | Impl | Trait's pkg | Type's pkg | Verdict |
|---|---|---|---|---|
| `rut/nmapset/nmapset.rut:106–170` (×11) | `impl KeyLane for i8/i16/i32/i64/u8/u16/u32/u64/bool/str/bytes` | nmapset (`:100`) | builtin prims | trait local — legal |
| `rut/json/json.rut:1327–1383` (×8) | `impl JsonSerialize/JsonDeserialize for i64/f64/bool/str` | json (`:1212`, `:1216`) | builtin prims | trait local — legal |
| `rut/json/json.rut:1397,1408` (×2) | `impl .. for ?T` | json | builtin (`?T` template) | trait local — legal |
| `rut/json/json.rut:1427,1439` (×2) | `impl .. for [T]` | json | builtin (`[T]` template) | trait local — legal |
| `rut/json/group-pouch.rut:16,28` (×2) | `impl JsonSerialize/JsonDeserialize for Vec<T>` | json (group text = json's source) | pouch (`pouch.rut:23`) | trait local — legal; the peer-gated motivating case |
| `rut/json/group-nmapset.rut:48–137` (×6) | `impl KeyText for str/i64/bool/u64/f64/bytes` | json (KeyText declared *in the group*, `:43` — pkg-private, impl-only law intact) | builtin prims | trait local — legal |
| `rut/json/group-nmapset.rut:147–260` (×5) | `impl JsonDeserialize for HashMap<K,V>/HashSet<T>/PrimMapI64<K>/PrimMapU64<K>/PrimMapF64<K>` | json | nmapset (`:183`, `:341`, `:390–496`) | trait local — legal |
| `demo/src/examples/type-aliases.rut:24,28` (×2) | `impl Labeled for Ridge/Trench` | same file (`trait Labeled` `:17`) | same file | trait local — legal |
| `examples/02-digest/digest.rut:944` | `impl JsonSerialize for Json` | json | digest (same file, `struct Json` `:666`) | **type local — legal;** the consumer-side motivating case |
| `examples/04-custom-async/custom_async.rut:122` | `impl Task<T> for CustomTask<T>` | core (builtin trait) | same file (`struct CustomTask<T>` `:82`) | type local — legal |
| `benches/workloads/json-roundtrip/main.rut:137,155,192` (×3) | `impl JsonSerialize/JsonDeserialize for DocMeta/DocRow` | json | same file (`:125–126`) | type local — legal |
| `crates/rut-driver/tests/data/peers/json{,_required,_broken}/json.rut` (×3) | `impl JsonSerialize for str` | the fixture pkg's own trait | builtin prim | trait local — legal |
| `…/peers/json/serde_pouch.rut:8`, `…/json_broken/serde_pouch.rut:6` (×2) | `impl JsonSerialize for Vec<T>` | fixture json's trait | fixture pouch's type | trait local — legal |
| `…/peers/json/serde_nmapset.rut:5` | `impl JsonSerialize for Map<K,V>` | fixture json's trait | fixture nmapset's type | trait local — legal |

Tally: **44 trait-local, 5 type-local, 0 orphans.** 29 of the 49 have
builtin/primitive self types — every one behind a local trait, the
sanctioned direction. The generic heads present (`Vec<T>`,
`HashMap<K,V>`, `HashSet<T>`, `PrimMap*<K>`, `CustomTask<T>`, `?T`,
`[T]`) all classify by their HEAD; no parameter ever carries a verdict.

### 1.3 The inherent impls — 27, all in-owner

`impl Session/Tile/Node/Counter/Rect/Version` (demo classics),
`impl Vec<T>` (pouch `:30`; fixture pouch `:10`),
`impl HashMap<K,V>/HashSet<T>/PrimMapI64<K>/PrimMapU64<K>/PrimMapF64<K>` (nmapset
`:188–544`), `impl Logger` (ink `:14`), `impl JsonWriter/JsonReader`
(json `:113,339`), `impl Circle` (vscode fixture), `impl
Crate/Slot/Map<K,V>` (peers fixtures), `impl
AuditLog/CustomTask<T>/CustomLaunched<T>` (custom-async), `impl
TodoList/Moderator/Widget/Store` (todolist, plugin, t1/store). Every
one names a type declared in the **same file** — the inherent
placement law (RFC 0012 §4: the type's module only) holds repo-wide
today. Inherent blocks are not this rule's subject (the law is about
`impl Trait for Type`), but they are where the splice hole (§1.5,
probe A) bites, so they are censused anyway.

### 1.4 The result: ZERO orphans — no migration plan

**Every impl in the repository conforms to the orphan rule as it
stands.** The codebase already follows the discipline — the std pkgs
by construction (container impls live in the trait's pkg; RFC 0028's
json amendment and RFC 0045's groups are trait-local by law), the
examples and workloads by the "orphan-legal direction" prose that the
rut-json batch seeded. Phase 1 therefore lands the check against a
**clean corpus: no migration, no grandfathering, no allowlist**. The
standing obligation, restated for the record: *if* any orphan had been
found, its migration plan would be disclosed here and land WITH phase
1, before the rule turns it into an error. Zero found — the obligation
is discharged by this census, not deferred.

### 1.5 Two verified probes — the holes the rule closes

Both probes live in scratch (`/tmp/opencode/batch-orphan-rule/p0/`)
and ran against the base tree via `rut run` (the CLI's loose-file
path: `mount_std` + the four-pkg convenience mounts
`crates/rut-cli/src/main.rs:117–126` + `assemble_peers`
(`main.rs:130`, `loader.rs:688`) + `compile_module_in`
(`rut-driver/src/lib.rs:594–619`) → `compile_graph`, which splices).

- **Probe A — the inherent-splice hole (verified: compiles and runs,
  exit 0).** A consumer file with `use pouch::{ Vec };` and an
  **inherent** `impl Vec<T> { fn probe_hi(self) -> i64 { return 7; } }`
  compiles clean and the method dispatches. Why: pouch's text is
  *spliced* into the consumer's unit (`Vec` is a generic export →
  `Unit::Inline`, `graph.rs:307–336`), so the checker's `find_data`
  (`collect.rs:434`) finds the spliced declaration and reads `is_local
  = true` — the "type's module only" law is blind at splice
  granularity, while it holds at link granularity (a USED type is a
  trait-impl target only, `collect.rs:554–559`). This is not the
  orphan rule's subject, but the census records it: today the
  discipline is manual, and the origin map (§2.3) is the data a future
  tightening would need. Phase 1 keeps the rule scoped to trait impls
  — the inherent law's fix, if wanted, is a separate disclosed change.
- **Probe B — the consumer-side orphan (verified: compiles clean,
  exit 0).** A consumer file with `use json::{ .. }` +
  `use nmapset::{ HashSet };` and `impl JsonSerialize for HashSet<T>`
  compiles with **zero diags** today. Neither side is the consumer's;
  no group provides the pair, so not even the duplicate check stirs.
  The impl registers in the consumer's unit and would answer dispatch
  and widening program-wide. **This is the exact program the rule
  rejects** — the §3 diagnostic's first rendered example is this
  probe. (A pair a group *does* provide — `impl JsonSerialize for
  Vec<T>` — already collides with the group's impl as an in-unit
  duplicate; §4.)

### 1.6 Disclosures

- The corpus is **70** `.rut` files today (examples 13, playground
  classics 17, stdlib 11, bench workloads 29); the plan's "69" predates
  `benches/workloads/json-roundtrip/main.rut` (the rut-json batch
  phase 2). The corpus gate itself is count-agnostic
  (`corpus.rs:48`, `assert!(files.len() >= 50)`).
- The census greps source files only. Engine-side surfaces that never
  spell `impl` in rut source — core's `builtin impl` numeric methods
  (RFC 0032 §1.1), `native_impls` — are out of the rule's reach by
  construction (they are the primitives' inherent surface, core's own).

---

## 2. The origin-tracking design

### 2.1 Where origin data exists today

- **The splice graph carries origins — as leaf specs.**
  `Unit::Inline { leaves: Vec<(String, String)>, bound }`
  (`crates/rut-driver/src/graph.rs:59–70`): the ordered
  `(origin spec, own source)` leaves, post-order, **deduped by origin
  spec** (first position wins). Each recursion level pushes its own
  `own_leaf = (spec, src)` (`graph.rs:275`), so a leaf's `String` is
  the mounted pkg's exact name.
- **The composition concatenates the texts.** The accepted leaves join
  into `extra`, the unit's own source follows the final `"\n"`, and
  the WHOLE string compiles as one unit under the **consumer's** spec
  (`graph.rs:235–290`, `compile_program_resolved(.., spec, ..)` at
  `rut-driver/src/lib.rs:66–73`). The origin data dies exactly here:
  the AST's spans are offsets into the combined text, and nothing
  records which range came from which leaf.
- **Linked deps bind surfaces, not text.** `bound:
  Vec<(ScopeId, Surface)>` — the exporter's scope + surface, but not
  its spec; the consumer registers `extern_types`/`extern_trait_decls`
  (`check/mod.rs:195,208`) name-by-name with no origin attached.
- **Peer groups ride the declaring pkg's source.** The peer gate
  appends group text via `session.append_source(&pkg, &text)`
  (`loader.rs:639–642`, `session.rs:351`) — a LOAD-time pass (RFC 0045
  §3), *before* any compilation. Group text is therefore part of
  json's `source`, and its leaves' origin spec is `json` — pkg-level
  by construction, exactly the law's granularity. The `PeerDecl`
  registry (`session.rs:153–158`) and `mount_dir`'s peer recording
  (`loader.rs:646–675`) are pkg-aware but never reach the checker.

### 2.2 What the checker can say today

`collect.rs`'s `collect_impl` (pass 2, `:412–565`) classifies an impl
target through four doors — `find_data` (declared in THIS unit →
"is_local"), `extern_native_types` (the core builtin classes), 
`extern_types` (a USED type of a linked dep → "is_used"), 
`sym::primitive_ty` (builtin prim) — plus the two template shapes
`TyArray`/`TyOpt` (`:491–529`). Trait refs resolve through
`resolve_trait_ref` (`check/resolve.rs:10–60`): a local `find_trait`,
an `extern_trait` (a used module's exported trait), or a core native
trait. Every door answers a **name-scope** question ("declared in this
unit? bound from a surface?"), none answers **"which pkg declared
it"** — after a splice, pouch's `Vec` and the consumer's own types are
indistinguishable. That is the whole gap; it is also why probe A
passes today.

### 2.3 The origin map (the design)

**Data shape.** One new driver type:

```rust
/// One spliced leaf's byte range in the combined unit text, and the
/// pkg whose source it is. `lo` is inclusive, `hi` exclusive — the
/// leaf's own text exactly, seams excluded.
pub struct OriginLeaf { pub lo: u32, pub hi: u32, pub spec: String }
```

**Where it is built.** In `graph.rs`'s composition loop, which already
knows each accepted leaf's text as it appends: record
`extra.len() as u32` before every `extra.push_str(&src)`, and the own
leaf closes the map (`extra.len() + 1 .. extra.len() + 1 +
src.len()`; when `extra` is empty the combined text is `src` alone —
own leaf `(0, len)`). Decl units compile their own source only, so
their map is the single own leaf. Total new work: one `Vec` push per
leaf; no text is re-laid-out, and the T13 byte-identity guarantee
(the combined text is unchanged whenever nothing dedups) is untouched
— the map is pure metadata.

**How it threads.** `compile_program_resolved` gains one parameter
(`origins: &[OriginLeaf]`; `compile_program` and every other caller
passes `&[]`), and `Ctx` gains two fields:

```rust
pub origins: Vec<OriginLeaf>,   // empty for a no-splice unit
pub own_spec: String,           // = module_name, already a parameter
```

both set after construction in the `allow_uses` style
(`rut-driver/src/lib.rs:98`, the field at `check/mod.rs:230`), plus
one lookup, used by every origin question:

```rust
/// The pkg whose source this byte offset was parsed from. The unit's
/// own spec when the offset is the own source (or the map is empty).
pub fn origin_of(&self, lo: u32) -> &str
```

(binary search over `origins`, last-wins on seams — a seam byte
belongs to the *following* leaf, which makes the own-source fallback
exact; `own_spec` when the map is empty or the offset exceeds it.)

**Fallback = the whole-unit law.** A unit compiled with no map (the
CLI/demo/wasm single-file paths, every `compile_program` caller, all
tests) has every definition's origin = its own spec. The check then
reduces to "both names must resolve in this unit or its bound
surfaces" — which for single-file programs is trivially satisfied by
any impl that compiles at all today. **The check is inert where
splicing does not happen; it fires only on genuinely cross-pkg
heads.** This is what keeps the corpus (§3.4) and the fixture suites
green without edits.

### 2.4 Linked deps: the exporter's spec rides the binding

For a LINKED dep the impl was already checked — in the dep's own unit,
under the dep's spec, where every law ran with full locality. The
consumer never re-checks foreign surfaces; it binds them. What the
consumer's own orphan check needs is the ORIGIN of each bound name:

- The graph's `bound` tuples grow the exporter's spec —
  `Vec<(ScopeId, Surface)>` → `Vec<(ScopeId, Surface, String)>` —
  trivially available at every construction site (`graph.rs:238,263–267`,
  the `bound_scopes` dedup unchanged).
- `compile_program_resolved`'s registration of used names records the
  origin beside the name: `extern_types`/`extern_trait_decls` gain a
  parallel `HashMap<IdentId, String>` (or the `ExternTrait` struct
  grows `origin: String` — phase 1 picks the lower-churn shape; the
  survey commits only to the data being present per name).
- Core's native traits (`extern_traits`, `check/mod.rs:204`) arrive
  through core's own surface, so their origin is `"core"` — which is
  the RFC 0012 §2 truth (builtin traits are core decls), and what the
  classification table below wants.

Decl-file surfaces (`.d.rut`, RFC 0029) ride the same path: the
surface's spec is the publishing pkg's.

### 2.5 The name classification (the check's only vocabulary)

Every name on either side of the head resolves to one of:

| Class | Test (all existing) | Origin |
|---|---|---|
| local decl | `find_data` / `find_trait` hit | `origin_of(decl_node.span)` — own spec or a spliced leaf's |
| used decl | `extern_types` / `extern_trait_decls` hit | the exporter spec (§2.4) |
| native trait | `extern_traits` hit (`Iterator`/`Index`/`Disposal`) | `"core"` |
| builtin type | `sym::primitive_ty`, `TyArray`, `TyOpt`, `NativeTy::Opaque` | **none — in no pkg** |
| closed builtin | `NativeTy::StackTrace` | unreachable — impls already rejected (`collect.rs:461–464`) |
| unresolved | none of the above | existing diagnostics fire; the orphan check never runs |

Two consequences worth stating. **The spliced case is the local case:**
a spliced declaration's `find_data`/`find_trait` hit carries a span
inside a leaf's range, so `origin_of` returns the *declaring* pkg, not
the consumer's — the origin survives splicing by construction, and
probe B's shape becomes expressible. **The `?T`/`[T]` heads classify
through the template arms:** the parser hands the checker
`TyKind::TyOpt { inner }` / `TyArray { elem }`; the element resolves
to a bare type parameter (`ty_generic_idents`, `collect.rs:877–888`)
— parameters never satisfy locality — so the head's origin is "no
pkg". A `?T` head under a foreign trait is an orphan; under a local
trait (json's twelve) it is legal. No new template machinery: the
rut-json arms are exactly the shapes the check reads.

### 2.6 Where the check sits, and the law it applies

**The pass: rut-lir's check/collect, pass 2** — the resolve+collect
pass of RFC 0031 §1, `collect_impl_trait`'s entry, immediately after
`resolve_trait_ref` succeeds (`collect.rs:666–668`) and *before* the
duplicate-pair check (`:675`). Not the driver's resolver (that is
module mounting, load-time), not body typeck (`lir.rs`) — every other
impl law (placement `:544–560`, duplicates `:675–678`, coverage
`:739+`) lives here, the impl head is fully resolved on both sides at
exactly this point, spans are native, and one home for the impl laws
keeps the diagnostics' phase-order stable (parse → collect → bodies).

**The law, in three lines:**

```
let trait_origin = classify(trait side);   // §2.5 table
let type_origin  = classify(head side);    // params never satisfy; ?T/[T] peel to "no pkg"
if trait_origin != OWN && type_origin != OWN { orphan diag; return; }   // OWN = this unit's spec
```

precisely: the block is an orphan iff `trait_origin != own_spec &&
type_origin != own_spec` where a builtin type's origin is a value
distinct from every spec (so it never equals `own_spec`, and a builtin
head is saved only by a local trait). Ordering laws already in place:
the peer gate ran at LOAD, so a mounted group's impls are in the text
with origin = the declaring pkg before any of this runs; the
"duplicate" check runs after, so a block that is both orphan and
duplicate reports the orphan (placement precedes registration — the
mirror of the RFC 0012 amendment's own ordering sentence). One diag
per offending block; a block whose trait name does not resolve keeps
today's unresolved-name diag only.

**What does NOT change:** the splice dedup and order (T13 byte-identity
holds), the linked path's compile-once-per-unit economics, the peer
gate's position, the wasm/demo single-unit paths (no map → inert), and
the LSP's parse-only pipeline (§3.4). Phase 1's test additions: the
two probes as negative fixtures (probe B's exact text must produce the
§3 diag; probe A's inherent shape must KEEP compiling — pinning the
rule's scope), a spliced-legal positive (the digest shape), and the
`?T`-head matrix (foreign trait → orphan; local trait → legal).

### 2.7 The spliced-orphan walkthrough (probe B under the rule)

The consumer unit splices nmapset's and json's leaves (both are
generic exporters). The orphan block's span lands inside the
consumer's own leaf. `JsonSerialize` → `find_trait` hit, span in
json's leaf → origin `json`. `HashSet<T>` → `find_data` hit, span in
nmapset's leaf → origin `nmapset`. Neither equals `"main"` (the
consumer's spec) → the §3 diagnostic fires, naming both pkgs, and the
impl never registers. The same block written *in* json (a group) or
*nmapset* is legal — the origin, not the unit, decides.

---

## 3. The diagnostic

### 3.1 The exact text

One diag, span = the impl block's item span (`self.ast.span(node)` —
the same span the duplicate-pair diag uses, `collect.rs:676`; house
style: plain message, no labels, no notes). The template, with every
origin rendered ("in this pkg" / "`X`'s" / "a builtin — in no pkg"):

```
orphan impl: neither `{Trait}` nor `{Type}` is defined in this pkg — `{Trait}` is {trait_origin}'s, `{Type}` is {type_origin}'s; an `impl Trait for Type` needs at least one of the pair declared in its own pkg (RFC 0012 §2a)
```

Rendered on the two real shapes:

- probe B (pkg × pkg):
  `orphan impl: neither `JsonSerialize` nor `HashSet` is defined in this pkg — `JsonSerialize` is json's, `HashSet` is nmapset's; an `impl Trait for Type` needs at least one of the pair declared in its own pkg (RFC 0012 §2a)`
- builtin head (the law's builtin clause — json's twelve render
  legally and never reach this):
  `orphan impl: neither `Draw` nor `[T]` is defined in this pkg — `Draw` is art's, `[T]` is a builtin, in no pkg; only a trait of this pkg may be implemented for a builtin (RFC 0012 §2a)`
- `?T` renders as its written head: `.. `?T` is a builtin, in no pkg ..`.

The RFC cite is **§2a** — the number this survey assigns the amendment
(§3.3), keeping the diag's cite greppable exactly like every
`(RFC 0012 §2)` diag today.

### 3.2 Position and surface

`Diag { span: impl item, msg, labels: [], notes: [] }`
(`rut-lexer/src/diag.rs:7–12`). The CLI renders it through
`render_diags` (caret on the `impl` head line, `diag.rs:32–59`); the
demo/wasm surface returns it structured; the LSP maps it through
`to_diagnostic` (`rut-lsp/src/analysis.rs:100–128`: severity ERROR,
source `rut`, range = the item span) with zero new plumbing.

### 3.3 Where the amendment lands

RFC 0012 gains **"Amendment (Sep 2026): the orphan rule — one of the
pair is local"**, numbered **§2a**, sitting beside the peer-gated
groups amendment (Sep 2026) it must agree with. It records: the law
(§0 above, verbatim rulings included), the builtin and generic-head
readings, the origin-survives-splicing sentence, the ordering sentence
(the peer gate precedes; placement precedes registration), the §2
placement bullet's narrowing ("trait impls are legal in any module"
→ "any module of the trait's pkg or the type's pkg"), and the §4
table's trait-impl row gaining the same footnote. Phase 1 lands the
RFC edit with the check — one commit-flavored unit of law.

### 3.4 The LSP story — honest

The LSP's per-document pipeline is **lex + parse + classify**
(`analysis.rs:42–57`): it never runs rut-lir's check today, so no
check diag — this one included — surfaces in the editor yet. The
corpus gates (`corpus.rs`, mirrored from rut-parser's) assert parse
cleanliness over the 70 corpus files and are untouched by this batch.
The surfacing story is therefore two-halved, stated without
overclaiming: **(a) today**, the diagnostic exists wherever the
compiler runs (CLI, tests, demo compile panes) and the LSP is simply
not a check-diag consumer — no regression is possible; **(b) when the
LSP grows semantic diagnostics** (the lsp-features roadmap's owning
call), this diag flows through `to_diagnostic` unchanged, and the
zero-false-diag invariant is protected by the census, not by hope:
**no in-repo file trips the rule** (§1: 44 trait-local + 5 type-local,
zero orphans), and a check that fires on nothing cannot conjure a red
squiggle. The corpus re-count and the two probes' negatives join the
phase 1 gate so the invariant is *tested*, not merely observed.

---

## 4. The coherence story — pre and post, no overclaiming

**Pre (today).** Pair uniqueness is already enforced two ways: within
a unit, `collect_impl_trait`'s duplicate check (`collect.rs:675–678`);
across units, the link's registry merge (RFC 0012 §5, RFC 0038 §4).
Splice dedup (first origin wins) guarantees a pkg's text compiles once
per program, so a pair provided by a pkg cannot be silently
re-registered by that pkg. What is NOT enforced: a consumer may write
a cross-pkg pair no one else provides (probe B — accepted), and an
inherent block may land on a spliced foreign class (probe A).

**Post.** The collision mechanics are **unchanged** — same two checks,
same errors, no new coherence engine, no global pair table at compile
time. The rule adds exactly one gate ahead of registration, and its
yield is three things, each stated at its true size:

1. **Discipline.** The `(trait, type)` pair set becomes auditable per
   pkg: a pkg's impls are either its own trait speaking about anyone,
   or anyone's trait speaking about its own types. "Who could possibly
   implement `JsonSerialize` for `Vec<T>`" has one answer (json) and a
   grep to prove it — today's answer is "any unit in the closure", which
   is probe B.
2. **The open-world property.** Today, adding a pkg to a closure can
   silently change behavior at a distance: probe B's impl participates
   in widening and dispatch the moment its unit compiles, and a future
   pkg that mounts between consumer and provider cannot be ruled out.
   Post-rule, an impl exists only inside a pkg that owns a side, so a
   consumer's pair set is bounded by the pkgs it named — adding a
   dependency adds the pairs its pkgs declare, never pairs a stranger
   invented.
3. **The shape of future separate compilation.** If pkgs ever compile
   to artifacts consumed without re-splicing (the `.d.ir`/bundle
   direction RFC 0041 points at), pair-uniqueness must become
   decidable from origins, not from "who happened to be spliced into
   this unit". The origin map is the smallest data that makes the
   question well-formed; the rule is the first consumer that makes the
   data real. This is a prerequisite built early, not a claim that
   separate compilation lands here.

**What the rule does NOT do:** it fixes no soundness bug (nothing in
today's dispatch miscompiles; probe B's impl is *wrong-shaped*, not
*wrong-code*), it does not make dispatch faster or smaller (vtables and
static binds are identical), and it does not subsume placement (the
inherent-splice hole, probe A, remains open and out of scope — a
disclosed candidate for a future rule reusing the same map). Duplicates
colliding as duplicate definitions after splicing remains the
consumer-side guard for group-provided pairs, exactly as the peer
amendment recorded it.

---

## 5. The VERSION call: 8 → 9

**The call: `VERSION 9`, landing WITH phase 1's check — the byte and
the law move in the same commit.** The ledger, from the tree's own
records:

| Move | What | Why it moved |
|---|---|---|
| 5 → 6 | the `opt_prim_store` gate (rut-json survey `:521`) | a **rejection addition** — sources the old compiler accepted, the new one refuses |
| 6 → 7 | `opaque.downcast` → `?T` (same cite) | a declared-surface change |
| 7 → 8 | `StackTrace` builtin class + the func table's `pos` field (err-channel report `:159`) | format-affecting, both halves |
| 8 (stayed) | the entry-err widening (the v8 note, `binary.rs:420–430`) | a **check relaxation** — "a CHECK relaxation, not a format change"; old artifacts bit-identical, old compilers refuse the new sources |

The orphan rule is a **rejection addition**: a source that compiles
under 8 is refused under 9. That is the 5 → 6 precedent exactly, and
it is the honest complement of the v8 note's own principle — the
relaxation stayed because accepting-more cannot strand an artifact;
rejecting-more can strand a *source*, and the version byte is the only
provenance marker a `.rutc`/bundle carries about the law that produced
it. Disclosed caveats, so nobody overclaims later: codegen is
**invariant** under the rule (it never changes an emitted program,
only which sources may reach emit), so no decode/verify change rides
this — the bump is policy per the precedent, not format necessity, and
a v8 artifact remains behaviorally correct forever. The plan's
coordination note resolves as stated: err-channel's StackTrace took 8;
this takes 9.

---

## 6. The phase order — confirmed, with one sharpening

- **Phase 0 — this survey** (docs only). Done.
- **Phase 1 — the rule lands**: the `OriginLeaf` map + threading
  (§2.3–2.4), the §2.6 check + §3.1 diagnostic, the RFC 0012 §2a
  amendment, `VERSION 9` (§5), the negative fixtures (both probes) and
  positive (the digest shape) + the `?T` matrix, the corpus gate
  re-run. Gates stay green: the census (§1) is the proof the check
  fires on nothing in-repo.
- **Phase 2 — the record**: the batch report; the prose pass that
  attaches the §2a cite where the tree says "orphan-legal" today
  (`docs/dep-kinds-survey.md`, `docs/rut-json-survey.md`,
  `rfc/0028-standard-library.md`, `examples/02-digest/README.md` +
  `digest.rut`, `benches/workloads/json-roundtrip/main.rut`); the
  corpus re-count.

The split is right, for one reason above all: **the law is one unit**.
Origin data without the check is untestable except through the check's
own tests; the check without the RFC amendment is an unexplained
rejection; the version byte without either is a lie. Splitting phase 1
into "map first, rule second" would land a batch whose middle state
has data nobody reads. Phase 2 stays last so the record describes the
law as landed, not as proposed — the rut-json batch's phase 3 lesson,
applied at this batch's smaller scale.

---

*Scratch inventory: `impl-lines.txt` (the 76-block dump),
`probe-inherent.rut` (exit 0 — the splice hole),
`probe-orphan.rut` (exit 0 today; the §3.1 diag's first rendering
under the rule). The repo tree is untouched by this phase.*
