# dep-kinds — phase 0 survey: the manifest census, the splice-graph mechanics, the peer/dev mechanism, the dedup decision, the missing-peer matrix, the test plan

*Phase 0 of the dep-kinds batch (docs only — no code). Base `2eaefd8`,
binary `VERSION 8` (`crates/rut-core/src/binary.rs:431`). Scratch:
`/tmp/opencode/batch-dep-kinds/p0/`. Everything below cites the tree as
it stands; the design sections propose, they do not describe.*

---

## 0. The law (the user's ruling, corrected Sep 23 — verbatim)

> - [deps]: today's { path = .. } — transitively mounted for every
>   consumer. Unchanged.
> - [peer-deps]: REQUIRED BY DEFAULT — not transitively pulled; the
>   CONSUMER must supply the peer; a missing required peer is a LOUD
>   resolution error naming pkg + peer + the fix.
> - [peer-deps] with `optional = true` (per-entry attribute): the
>   optional peer — not pulled; if the consumer's closure contains the
>   pkg, the integration sources mount; absent = the integration is
>   simply not there; referencing it = the DEDICATED missing-peer
>   diagnostic (never a bare unresolved-name).
> - [dev-deps]: mounted ONLY when building/testing the pkg itself —
>   never in a consumer's world.
> - A pkg may declare the same dep as BOTH optional peer AND dev (the
>   json shape: integration-if-present for consumers; always-present
>   while developing json itself).

THE MANIFEST GRAMMAR (pinned):

```toml
# rut/json/rut.toml
name = "json"

[peer-deps]
pouch    = { path = "../pouch",    optional = true }
nmapset  = { path = "../nmapset",  optional = true }

[dev-deps]
pouch    = { path = "../pouch" }
nmapset  = { path = "../nmapset" }
```

THE MISSING-PEER MATRIX (pinned):

| case | behavior |
|---|---|
| REQUIRED peer absent from the consumer's closure | LOUD resolution error at mount: names the pkg, the peer, and the fix ("add nmapset to [deps]"). Not silent, not auto-pulled. |
| optional peer absent, integration never touched | NOTHING — silent success, that IS the feature (json mounts light, no transitive pull). |
| optional peer absent, integration-only surface referenced | the DEDICATED missing-peer diagnostic (pkg + peer + fix); never a bare unresolved-name. |
| peer PRESENT in the consumer's closure (any reason) | the integration group mounts automatically (presence-based resolution — no extra declaration beyond having the pkg). |
| self-build/dev mode | dev-deps guarantee presence; no missing case exists. |
| a [peer-deps] path that does not resolve | LOUD manifest error at the pkg's own build (a packaging bug); for consumers, a required peer errors, an optional peer is inert (group simply not mounted). |

Motivating case: json's serde-model impls (`impl JsonSerialize for
Vec<T>` written in json) must not force every json consumer to mount
pouch/nmapset. This survey is phase 0: census (§1), the ruling (§0),
the mechanism design (§3), the test-matrix design (§4), the phase-order
confirmation (§5).

---

## 1. The census today

### 1.1 The manifest grammar as implemented

`parse_manifest` (`crates/rut-driver/src/session.rs:257`) is a
hand-rolled TOML subset — comments, `key = "string"`, dotted keys,
`[section]`, single-line inline tables — deliberately dependency-free
and wasm-compatible (module doc, session.rs:26–28). The full accepted
surface:

| key | where | type | meaning |
|---|---|---|---|
| `name` | top | string, bare `[a-zA-Z0-9_]+` (session.rs:289–294) | the package's use-path name |
| `entry.type` / `entry.lib` / `entry.ir` | top or `[entry]` | string paths | the declaration surface / body / compiled IR (RFC 0029 §5) |
| `format`, `format_version` | top | string, u64 | bundle layout self-declaration (RFC 0038 §2; directory loads ignore them) |
| `host_scope` | top | string | host-fn registration prefix override (`rt` → `rt:log`, RFC 0022) |
| `inline` | top | bool | force source-inlining into every consumer (`ink`, `nmapset`) |
| `[deps]` | section | `pkg = { path = ".." }`, **string values only** (session.rs:371–391, every value through `parse_string`) | transitively mounted deps (RFC 0041 §5) |

Census facts that constrain the design:

- **Unknown top-level keys are silently ignored**
  (session.rs:308, "forward-compatible"). `[entry]` unknown keys ERROR
  (session.rs:314–319); an unknown `[section]` errors
  (session.rs:270–276). So `[peer-deps]`/`[dev-deps]` are new sections
  the parser must learn — today they fail with
  ``unknown section `[peer-deps]` ``.
- **Inline-table values are strings only** — `optional = true` inside
  `{ .. }` is a load error today ("expected a quoted string"). The
  pinned grammar needs a bool there (§3.1).
- `Manifest` (session.rs:94–114) keeps `deps:
  BTreeMap<String, BTreeMap<String, String>>`; the `Session` itself
  does no I/O — the filesystem walks live in the loader
  (loader.rs), which is exactly why the wasm/in-memory mount paths
  never see a manifest.

### 1.2 The in-tree packages

Eight pkgs under `rut/` (`bench_cross`, `calc`, `core`, `ink`,
`nmap_host`, `nmapset`, `pouch`, `rt`); the LSP embeds all eight
sources by hand (`crates/rut-lsp/src/std_surface.rs:9–12` — it never
parses manifests). Shapes: `entry.type`-only host pkgs (`core`, `calc`,
`nmap_host`, `rt` + `host_scope`, `bench_cross`) vs `entry.lib`
source pkgs (`pouch`; `ink`/`nmapset` with `inline = true`). The
engine mounts `core` + `calc` unconditionally (`mount_std`,
rut-driver/src/lib.rs:516); "pouch and ink are third-party libraries
in the toolchain tree — nothing in the engine knows their names"
(RFC 0041 §5). `[deps]` examples in the tree: `ink → rt`,
`nmapset → nmap_host`.

`rut/json` does not exist yet. Today's json lives as the JSON section
of `examples/02-digest/digest.rut:652ff` (JTag + `struct Json` with
`Vec<opaque>` children). **A constraint the extraction must honor:**
today's `Json` struct rides pouch's `Vec` in its own fields — the
pinned grammar makes pouch an *optional peer* of json, so the
extracted base must be pouch-free (native `[T]`/own growable storage),
with every pouch-typed surface living in the peer group. Recorded as
an input to the json lane, not designed here.

### 1.3 How the CLI/driver resolves and mounts

The CLI (`crates/rut-cli/src/main.rs`) exposes `run <file|dir|
bundle>`, `pack`, `dump`. The mount chain for a directory:

1. `load_dir_session` (loader.rs:44): read + parse `rut.toml`,
   register the root module, then `resolve_deps`
   (loader.rs:363): for each `[deps]` entry in BTreeMap order —
   **first mount wins** (a name already in the session is skipped,
   loader.rs:370–372), the dep's manifest is read, its `name` must
   match its key (else an error naming both, loader.rs:384–390), the
   entry module loads, and the walk recurses with a `visiting` cycle
   guard (loader.rs:377–382). This is RFC 0041 §5's "resolution walks
   the graph recursively, with a cycle guard, and first mount wins".
2. `load_entry_module` (loader.rs:318): `entry.type` alone → a host
   pkg — the `.d.rut` lowers via `lower_decl_module` into bodyless
   `host_funcs`; otherwise the `.rut` body rides `Module.source` as
   one string. **One package is one entry file** — there is no
   include form and no multi-file package (loader.rs:5–7, 35–39;
   RFC 0035 §1: use paths are inter-module). `host_scope`/`inline`
   ride the manifest into the `Module`.
3. `mount_dir` (loader.rs:406) is the programmatic counterpart for
   embedders: same walk into an EXISTING session; the embedder's
   mounts outrank the directory.
4. Bundles (RFC 0038): `pack_dir` (loader.rs:261) packs
   `rut.toml` byte-for-byte + the entry, then the whole dep graph
   under `<pkg>/` groups, recursively, deduplicated by name
   (loader.rs:278–305); `load_bundle_bytes` (loader.rs:106) refuses
   unknown `format_version` **before reading anything else**
   (loader.rs:120–128), then resolves v2 groups by NAME
   (loader.rs:164–192) and requires every declared dep to be
   satisfied (loader.rs:194–201). The `path` keys are directory-time
   only.

### 1.4 The splice graph — and the per-use dedup hole

`compile_graph` (graph.rs:33) walks the root's uses through the
session (`ensure`, graph.rs:76), memoized per spec (`done`,
graph.rs:68). A dep becomes one of two units:

- `Unit::Linked` — compiled under its own scope, registered as a
  program; the consumer binds its `Surface`.
- `Unit::Inline` — its **combined source text** is spliced into the
  consumer. Three triggers (graph.rs:287): the manifest `inline`
  flag; a generic type export (`Vec<T>` cannot link — RFC 0013
  monomorphizes at compile time, the instantiation must happen where
  the body lives); an exported fn with a trait-typed parameter
  (graph.rs:279–283). This is `ink`/`nmapset`'s world and the future
  json's.

The splice loop (graph.rs:219–236) is the finding:

```rust
for dep in &uses {
    match self.ensure(dep, true)? {
        Unit::Inline { source, bound: b } => {
            extra.push_str(&source);      // ← NO cross-use dedup
            ...
```

`uses_of` dedups a module's OWN use list (graph.rs:300–312), but each
inlined dep's `source` is its **pre-combined** text — already
containing its own transitive splices. Two sibling uses sharing a
transitive inline pkg therefore splice that pkg's text twice into one
compilation unit: duplicate type definitions, a compile error.

**The landed finding (t1 batch, `7149d27`)** — verbatim from the
commit: *"the graph splices each use's transitive sources per use with
no cross-use dedup, and store and t1 both ride pouch (t1 also
nmapset), so a consumer importing both would define Vec twice;
spliced together they define everything once."* The example worked
around it by offering store + t1 as ONE inline module `appkit`
(`examples/05-todolist-web/src/mount.rs`, the comment block "WHY
ONE"), collapsing the two sibling uses into one.

Also census here: `core` is pushed into every unit's uses
ambiently-if-mounted (graph.rs:211–214, RFC 0028 builtin-surface);
a stale doc comment claims "relative includes are already merged by
the loader" (graph.rs:9) — there is no include form (loader.rs:6,
36); the loader comment is the true one. Worth fixing in passing
during phase 1 (a comment-only change, listed here so it is not
forgotten).

### 1.5 What VERSION/verify does with pkg presence

- `verify` (rut-vm/src/verify.rs) is **structural per function**:
  register ranges, argv/label spans, jump targets, type operands in
  the table (RFC 0033 §2). It knows nothing about packages. Bodyless
  host fns are skipped (verify.rs:17–19).
- The pkg-presence-sensitive check is the **host-fn contract**
  (RFC 0025, "the load-time contract"): mounting a host pkg
  *declares* fns (`Session::expected_host_fns`, session.rs:203,
  scope-aware via `host_scope`); the embedder *binds* them;
  `Vm::verify_host_fns` panics on declared-unbound / bound-undeclared
  / signature drift before any rut code runs. The law is "mount what
  you bind": an embedder that mounts only `core` binds only `core`.
- `VERSION 8` gates the binary encoding (binary.rs:431, checked at
  decode, binary.rs:623).

Consequence for peer groups (confirmed in §3.6): **impl-only rut
source groups touch none of these** — no host fns declared, no new
binary sections, so no verify change and no VERSION move. A group
shaped as a *host* surface would change `expected_host_fns`
conditionally and drag the embedder's binding obligations through the
peer's presence — one more reason groups are rut-source-only (§3.3).

### 1.6 Where the orphan rule's origin tracking sits

Rut's "orphan rule" is the **placement rule** (RFC 0012 §2): "there is
no orphan rule beyond placement" — inherent impls live in the type's
module (the checker enforces: a USED type is a trait-impl target only,
`crates/rut-lir/src/check/collect.rs:466–471` and the diagnostic at
collect.rs:537; a primitive takes trait impls only, collect.rs:472–482,
524–533); **trait impls are legal in any module** ("`impl ForeignTrait
for ForeignType`"); one impl per `(trait, type)` pair program-wide is
a **link error** (RFC 0012 §5; link merges `surface.impls` and rejects
duplicate pairs — `crates/rut-core/src/link.rs:244ff`; the
cross-module proof is `crates/rut-driver/tests/cross_traits.rs`: trait
in one module, type in another, impl in a third).

The "current pkg" origin: a compilation unit is compiled under its
spec's name (`compile_program_resolved(..., spec, scope, ...)`,
rut-driver/src/lib.rs:66; graph.rs:249–256). For a SPLICED unit the
combined text compiles under the **consumer's** spec — spliced items
and impls are local to that one unit, and since the graph has exactly
one root, spliced impls register once per program. For a LINKED dep
the impl rides that pkg's own program and merges at link. Either way
duplicate detection is: within a unit, the checker's
`duplicate impl for the same (trait, type) pair`
(collect.rs:655–657); across units, the link check.

The **use-both gate** (RFC 0012 §6) is the call-site half:
`x.trait_method()` needs the type named AND the trait used at the
call site's module; an extern impl whose trait no `use` names still
feeds the "use `I` .." diagnostic
(`find_extern_trait_impl_method`, rut-lir/src/lir/call.rs:1516–1531).

This is exactly the shape json's peer-gated impls need:
`impl JsonSerialize for Vec<T>` is a trait impl in json — legal by
placement; it registers where its unit registers; the pair check
guards duplication. The design's obligation (§3.6) is ordering: the
peer gate must PRECEDE these checks so they never see a half-mounted
world.

---

## 2. The mechanism design

### 2.1 Manifest parsing (the three tables; the optional attribute)

`Manifest` grows two tables beside `deps`:

```rust
pub deps:       BTreeMap<String, BTreeMap<String, String>>, // today
pub peer_deps:  BTreeMap<String, BTreeMap<String, String>>, // [peer-deps]
pub dev_deps:   BTreeMap<String, BTreeMap<String, String>>, // [dev-deps]
```

- `Section` gains `PeerDeps`, `DevDeps` (session.rs:335–340). Keys
  are bare package names (the same `valid_spec` law); values are
  inline tables.
- **The one parser extension the grammar needs: bool values in
  inline tables** — `parse_inline_table` (session.rs:371) currently
  forces every value through `parse_string`; it learns
  `parse_bool` for `optional` (line-targeted error otherwise:
  ``line N: `optional` expects `true` or `false` ``). `path` (and
  the group key below) stay strings. `[deps]` tables keep
  string-only values and REJECT `optional`:
  ``line N: `optional` is a `[peer-deps]` attribute — `[deps]` has no options``.
- A descriptor may carry **`lib = "./serde_pouch.rut"`** — the
  peer-gated integration file, relative to the manifest (§3.3). Any
  other key in a peer/dev descriptor is a line-targeted error (the
  `[entry]` strictness, not the top-level lenience).
- Same name in two tables: `[peer-deps]` + `[dev-deps]` is the
  sanctioned both-kinds pairing (the ruling). Anything else
  (`[deps]`+`[peer-deps]`, `[deps]`+`[dev-deps]`) is a manifest
  error naming both rows — a pkg is either pulled transitively or
  required of the consumer / held for development, never both.
- The zero-dep spell `{ path = ".." }` stays valid in all three
  tables; `optional` defaults to **false** (required), per the
  ruling's REQUIRED-BY-DEFAULT.

### 2.2 Resolver semantics (loader-owned; the Session stays I/O-free)

The loader's mount sequence becomes four passes, all in
`load_dir_session`/`resolve_deps`'s land — no graph change:

1. **The `[deps]` walk — unchanged.** Recursive, BTreeMap order,
   first-mount-wins, cycle guard, name-mismatch error
   (loader.rs:363–400). While walking, the loader records each
   mounted pkg's peer declarations (it already reads every dep's
   manifest — no extra I/O).
2. **Dev pass — root only.** If the directory is the program root
   (self-build), its `[dev-deps]` mount exactly like `[deps]`
   (same walk, same first-mount-wins). A dep's dev-deps are NEVER
   walked — a consumer's world never contains them (the ruling).
   `mount_dir` (the embedder path) does NOT mount dev-deps: it offers
   a pkg to someone else's program, it is not "building the pkg
   itself".
3. **The peer gate — one post-closure pass** (the mount-order law;
   see §3.6): after the full closure exists, for every mounted pkg P
   (root included) and every `[peer-deps]` entry (spec, desc):
   - **required** (`optional` absent/false): `spec` must resolve in
     the session, else the LOUD mount error (§3.5, text D1). Never
     auto-pulled — the consumer supplies.
   - **optional**: if `spec` resolves — **presence-based
     auto-mount**: P's group file (desc `lib`) is appended to P's
     `Module.source` (§3.3). If not — nothing; inert; the group
     simply never mounts (matrix rows 2/3/4/6).
   - path validation per matrix row 6: in self-build the loader
     READS each peer path's manifest and checks the name match even
     when dev-deps already supplied presence — a broken path is the
     loud packaging-bug error (text D3). In consumer mode peer paths
     are never read: presence is by NAME (first-mount-wins already
     guarantees the consumer's own path won), so a broken peer path
     is inert for an optional peer and unreachable for a required
     one (its absence is D1's business, not the path's).
4. **Compile** — unchanged entry: `compile_graph` sees
   `Module.source` texts that now include groups. The graph never
   learns what a peer is.

Dev-deps vs deps collision with different paths: loud (§2.1).
Dev-deps of the SAME pkg as its peers (both-kinds): the dev pass
mounts pouch by path; the peer gate then sees pouch present and
mounts the group — the json self-build works with zero special
cases, which is the point of the shape.

### 2.3 The conditional source mechanism — manifest-scoped groups, decided

**Decision: manifest-scoped per-peer integration files** — the `lib`
key on the `[peer-deps]` descriptor — **not source-level headers.**

```toml
[peer-deps]
pouch   = { path = "../pouch",   optional = true, lib = "./serde_pouch.rut" }
nmapset = { path = "../nmapset", optional = true, lib = "./serde_nmapset.rut" }
```

Why this fits the existing mount machinery honestly:

- **The manifest is the only mount contract the in-memory worlds
  have.** Header-scanned files need a directory enumeration; wasm
  hosts, tests, and bundle loaders mount `Module { source: .. }` by
  hand — there is no "directory" to glob. `register_module`
  embedders would silently lose header-gated files. The manifest
  rides every form: directory, `mount_dir`, bundle v3.
- **Bundles pack by manifest** (loader.rs:229–249): group files get
  packed/loaded exactly like entries — no discovery pass, no
  `format_version` ambiguity beyond the one honest bump (§3.6).
- House style: "the rules are small on purpose" (RFC 0041 §5); one
  explicit key beats a convention scanned out of comments. The
  precedent is `entry.type`/`entry.lib` — dotted explicitness.

**The constraint that makes the matrix total: groups are impl-only.**
A group file contains `impl` blocks (and their private helper fns);
it declares NO new public names. The base owns the trait and every
public name (`trait JsonSerialize`, `jparse`/`jencode` entry points).
This is not a limitation — it is the diagnostic guarantee: the only
way to "reference" the integration surface is a trait-method
dispatch or a `use` of the peer's pkg name, and both paths have
dedicated, peer-aware diagnostics (§3.5). A group-owned free fn
would create bare-name misses — the exact thing the matrix bans
("never a bare unresolved-name"). The motivating case is impl-only
by nature: serde-model impls ARE impls.

Assembly (pass 3 above): `Module.source` becomes
`base + "\n" + group(pouch) + "\n" + group(nmapset)` — groups in
peer-name (BTreeMap) order, after the base, so the combined text
stays ONE source string and **every existing consumer of
`Module.source` — the graph splice, bundles, the wasm mounts — is
untouched**. The group's own `use pouch::{Vec};` joins the unit's
use list through the ordinary `uses_of` scan of the combined text
(graph.rs:300): in a spliced world pouch rides the same unit; in a
linked world it becomes a bound surface — both are today's
mechanics, zero new ones.

Group files are `.rut` (impl mode) only; a `.d.rut` lib key is a
load error (decl surfaces don't gate, and a conditional host surface
would drag `expected_host_fns` through the peer's presence — §1.5).

### 2.4 The splice dedup question — decide: dedup by origin at mount

Peer groups make overlapping transitive closures the NORM, not the
exception: a consumer using json (pouch group + nmapset group) beside
any other pouch-riding inline pkg re-creates the t1 collision by
construction. Constraining the grammar to dodge it (the alternative:
"per-use splicing stays, the grammar constrains shape" — i.e. bless
appkit-style single-splice wrappers as THE way to consume
multi-consumer kits) would make the workaround the law and keep a
known duplicate-definition trap armed for every future pkg. **The
design fixes the splice instead: dedup by origin at mount.**

`Unit::Inline` stops carrying pre-combined text and carries the
**leaf list** — the ordered, origin-deduplicated `(spec, source)`
pairs it is made of:

```rust
enum Unit {
    Linked { idx: usize, scope: ScopeId },
    Inline { leaves: Vec<(String, String)> }, // (origin spec, own source),
}                                             // post-order, deduped by spec
```

`ensure(spec)` composes: for each use, extend-own leaves with the
dep's leaves **skipping specs already present** (first position
wins; the order stays topological — deps before users, matching
today's within-chain order), then push its own `(spec,
module.source)` leaf. The unit's own `combined` for COMPILATION is
the leaves' concatenation + its source — byte-identical in shape to
today's `extra + src` for the no-collision case, so all existing
programs compile identically. The memoized `done` map is unchanged.
Cross-use, pouch now appears once; `Vec` is defined once; the t1
collision is structurally impossible.

Correctness: a second splice of the same origin can only duplicate
definitions (items are order-independent — forward references are
legal, collect.rs pass 1) and never contributes a name the first
splice didn't; monomorphization instantiates per call site within
the unit regardless of how many times the template text appears.
Dedup is semantics-preserving and strictly removes an error class.
Bound surfaces keep their existing scope-dedup (graph.rs:225–234).

The t1 example's `appkit` keeps working under dedup (it is one use,
one leaf) — the example is another lane's property and is NOT
touched; phase 2 proves the fix with a fixture that imports two
sibling pouch-riding pkgs separately (§4, T10) — the shape appkit
was built to avoid.

### 2.5 The missing-peer diagnostic — one dedicated diag, exact texts

House style: lowercase, names the things, says the fix, names its
RFC. All four are load/resolution-time errors (never traps), and
there is exactly ONE missing-peer diagnostic shape (RFC 0045 is the
dep-kinds RFC this batch lands):

- **D1 — required peer absent** (mount-time, matrix row 1):

  ```
  pkg `json` requires the peer `nmapset`, and `nmapset` is not in
  this program's closure — peers are not pulled transitively: add
  `nmapset = { path = ".." }` to your `rut.toml` `[deps]` (RFC 0045 §3)
  ```

- **D2 — optional peer's name referenced while absent** (the
  dedicated diag; matrix row 3). The honest reachable path: a
  consumer `use pouch::{Vec}` with pouch absent — today that is the
  generic `ResolveError::NoModule` text. The upgrade: the Session
  carries the peer declarations the loader recorded (§2.2); when a
  missed name IS a declared peer of some mounted pkg, `resolve`
  returns the dedicated error instead of the bare one:

  ```
  cannot resolve `pouch` — `json`'s pouch integration is not
  mounted because the optional peer `pouch` is absent from this
  program's closure; add `pouch = { path = ".." }` to your
  `rut.toml` `[deps]` (RFC 0045 §4)
  ```

  Item-level misses can never be peer-gated (impl-only groups,
  §2.3), so "never a bare unresolved-name" holds by construction:
  every peer-related failure surfaces on a path that knows the
  peer.
- **D3 — peer path broken at self-build** (matrix row 6, packaging
  bug in the pkg itself):

  ```
  pkg `json`'s [peer-deps] entry `pouch` points at `../pouch` —
  cannot read a manifest there (a packaging bug in json; RFC 0045 §3)
  ```

- **D4 — cross-table name collision** (§2.1):

  ```
  `pouch` appears in both `[deps]` and `[peer-deps]` — a package is
  either pulled transitively or required of the consumer, never
  both (RFC 0045 §2)
  ```

All four are `ManifestError`/`ResolveError` variants in today's
shapes (line-targeted where a line exists) — load errors, never
runtime traps, matching session.rs:116–151's law.

### 2.6 Interplay

- **Orphan/placement — the gate precedes the check.** The peer gate
  runs at LOAD (pass 3); placement and pair-uniqueness run at
  COMPILE/LINK. So the checker never sees a half-mounted world: peer
  absent → the group text was never assembled → there is no `impl
  JsonSerialize for Vec<T>` anywhere to place, and no orphan
  question exists; peer present → the target type resolves (the
  group's `use pouch::{Vec}` binds pouch, mounted by definition),
  the impl is a trait impl in json — legal by RFC 0012 §2 placement
  ("trait-local": the trait is json's own, and json is where the
  impl lives), and it registers under whatever unit holds it
  (consumer's, if spliced; json's, if linked — §1.6).
- **The duplicate-pair link error is the consumer-side guard**: a
  consumer who hand-writes `impl JsonSerialize for Vec<T>` in a
  world where json's pouch group mounted gets the link-time
  duplicate (RFC 0012 §5) — loud, correct: the group already
  provides it. Documented behavior, not a gap.
- **Mount order with conditional groups.** Groups append after the
  base, peers in BTreeMap order — deterministic, and AFTER the whole
  `[deps]` closure (pass 3 is post-closure precisely because a peer
  may mount after its declarer alphabetically: consumer
  `[deps] json, pouch` — json loads first, pouch not yet present;
  a during-walk gate would wrongly skip the group). Splice order
  under dedup: first-seen origin wins, uses stay in source order.
- **VERSION/verify when surface presence varies — confirmed: no
  move.** Impl-only groups add no host fns (`expected_host_fns`
  unchanged — no binding obligations appear or vanish for the
  embedder), no new binary sections (impls ride the existing
  `surface.impls` encoding), nothing `verify` reads. A json
  consumer's binary differs by which impls registered, which is
  ordinary program content. `VERSION` stays 8 (RFC 0033 §2 /
  binary.rs:431).
- **Bundles (RFC 0038).** `pack_dir` packs each group file under
  `<pkg>/` beside the entry (collect_pkg_files grows the lib-keyed
  files); the v2 layout gains group entries, and a v2 loader would
  silently mount base-only — semantically wrong. So pkgs with peer
  groups pack as `format_version = 3`, and the version gate
  (loader.rs:120–128) makes old loaders REFUSE, never guess — the
  RFC 0038 §4 law applied to itself. v3's loader resolves groups by
  name exactly like deps and requires each mounted group's entry in
  the zip (a §4-consistency row: missing group file = load error).
- **The LSP.** It embeds sources and indexes names; peer gating is a
  resolution-time concept the LSP does not model. Today's consequence:
  group impls complete/hover unconditionally. Recorded as accepted
  (the LSP is advisory; the compiler is the law) — gating its
  completions by a peer-aware session is future menu, not this
  batch.
- **Dev-deps are invisible to bundles-as-consumers**: a packed json
  carries its peer groups; a consumer packing THEIR app never pulls
  json's dev table (pass 2 is root-only), so dev-only convenience
  pkgs cannot leak into consumer worlds.

---

## 3. The test matrix (phase 1's fixture design)

Fixtures live under `crates/rut-driver/tests/data/` (the
`keypayload`/`fastlane` precedent: hermetic mini-pkgs with their own
manifests, no reliance on `rut/` layout). Hermeticity note: fixtures
define their own tiny `pouch`/`nmapset`-shaped pkgs (a generic class
with one method each) so the suite never couples to the real pkgs'
evolution; names match the law's spelling.

| # | fixture / test | proves |
|---|---|---|
| T1 | `peers_json` (base trait + `serde_pouch.rut`/`serde_nmapset.rut` groups, pinned grammar verbatim) + consumer with only json in `[deps]` | optional peers absent, never touched: compiles green, **silent** — the feature (matrix row 2); no `pouch`/`nmapset` in the session |
| T2 | same consumer + `pouch` in `[deps]` | peer present → group mounts: the group's impl dispatches (`v.json_serialize()` resolves through the trait), orphan/placement green (matrix row 4) |
| T3 | same consumer + BOTH peers | both groups mount, BTreeMap order; both impl sets dispatch |
| T4 | consumer + `use pouch::{Vec}` with pouch absent | D2 — the dedicated diag names json + pouch + the fix; NOT the bare NoModule text (matrix row 3) |
| T5 | json variant with a REQUIRED peer; consumer without it | D1 at mount — names pkg + peer + fix; not auto-pulled (matrix row 1) |
| T6 | self-build of `peers_json` as root (dev-deps present) | dev pass mounts peers; groups mount; json's own tests compile against real group impls — "no missing case exists" (matrix row 5) |
| T7 | json as root with a broken `[peer-deps]` path | D3 loud at self-build (matrix row 6); the same broken path INERT for a consumer (optional: group not mounted, no error) |
| T8 | peer-of-peer: A `[deps]`→json, json peer→pouch, pouch `[deps]`→core-ish base; closure has all | groups chain through one post-closure pass; no fixpoint needed (groups add no new pkg names) |
| T9 | both-kinds-at-once: json self-build with pouch in `[peer-deps]`(optional)+`[dev-deps]` | the sanctioned pairing parses; dev mounts, gate sees presence, group mounts — the ruling's json shape end-to-end |
| T10 | two sibling inline pkgs both riding a shared transitive inline pkg, imported separately by one consumer | the t1 collision shape — compiles green under dedup-by-origin; the `appkit` workaround retires as a NECESSITY (it stays legal) |
| T11 | `[peer-deps]`+`[deps]` same name / `optional = true` inside `[deps]` / unknown descriptor key / `lib` pointing at a `.d.rut` | D4 + the line-targeted manifest errors of §2.1/§2.3 |
| T12 | bundle v3: pack the T2/T3 world, load, run; v2 loader refuses a v3 bundle | RFC 0038 round-trip with groups; refuse-never-guess |
| T13 | regression: every existing suite (643) green untouched; the graph change compiles all existing programs byte-identically in the no-collision case | the dedup is semantics-preserving (§2.4) |

T1–T9 are loader-land tests (driver fixture mounts + `compile_graph`
asserts); T10–T12 need the graph/bundle land. Every row cites the
matrix cell it pins, so the suite IS the ruling, executable.

---

## 4. The phase order — confirmed, with one refinement

The batch plan's split is right, and the survey pins the seam:
**phase 1 = loader/manifest land** — the three tables, `optional`,
the `lib` group key, the four mount passes, D1/D3/D4, group
assembly, fixtures T1–T9 + T11 (all green with today's graph
because every fixture avoids the T10 shape: one consumer per
shared inline pkg). No graph behavior change — the existing suites
cannot move. **phase 2 = graph/bundle land** — dedup-by-origin
(§2.4), D2's `ResolveError` upgrade, bundle v3, T10/T12/T13, and
the stale include-comment fix (§1.4). Reasons: (a) each phase is
independently green — phase 1 lands user-visible law with zero
engine risk; (b) the dedup is peer-independent and lands with its
own proof (T10) instead of riding along; (c) D2 needs the
Session-resident peer registry that phase 1's loader populates —
the phases hand off through data, not through rework. Any report
phase follows the batch's standing convention.

---

## 5. Deviations, limits, menu

- **Docs-only phase**: this survey changes no code; the gates ran on
  the untouched tree (workspace test suite green at `2eaefd8`).
- The ruling, grammar, and matrix are embedded verbatim (§0) and
  are law; where this doc adds structure (the `lib` group key, the
  impl-only constraint, D2's exact mechanism, bundle v3, the
  four-pass loader order) it is design proposal, flagged as such,
  to be ratified by phase 1's implementation and an RFC 0045 text.
- The `appkit` example keeps its workaround (not this batch's
  property); T10 proves the necessity retires.
- The LSP peer-gating gap is recorded, accepted (§2.6).
- rut/json does not exist; its extraction constraints (base must be
  pouch-free) are recorded for the json lane (§1.2), not designed.
- The stale `graph.rs:9` include comment is queued for phase 2
  (comment-only).
