# RFC 0045: Dependency Kinds — `[deps]`, `[peer-deps]`, `[dev-deps]`

## Summary

Every package's `rut.toml` names the packages it relates to, and the
*kinds* of that relation decide how — and whether — each one mounts.
Three tables, one grammar:

- **`[deps]`** — today's transitively mounted dependencies. Unchanged.
- **`[peer-deps]`** — **required by default**: not transitively pulled;
  the *consumer* must supply the peer; a missing required peer is a
  loud resolution error naming pkg + peer + the fix. With
  `optional = true` (a per-entry attribute): the *optional* peer — not
  pulled; if the consumer's closure contains the pkg, the integration
  sources mount; absent = the integration is simply not there;
  referencing it = the dedicated missing-peer diagnostic (never a bare
  unresolved-name).
- **`[dev-deps]`** — mounted ONLY when building/testing the pkg itself
  (the program root) — never in a consumer's world.

A pkg may declare the same dep as BOTH optional peer and dev-dep (the
sanctioned both-kinds pairing): integration-if-present for consumers;
always-present while developing the pkg itself.

The motivating case: json's serde-model impls (`impl JsonSerialize for
Vec<T>` written in json) must not force every json consumer to mount
pouch/nmapset. The impls live in *peer groups* — impl-only `.rut`
files that mount only when the peer is present — and the splice graph
dedups by origin so overlapping closures compose. The law the
diagnostics already cite: the grammar errors cite §2, the mount/gate
errors cite §3, the reference-site missing-peer error cites §4.

## 1. The three kinds — semantics

| table | who supplies it | transitive? | missing behavior |
|---|---|---|---|
| `[deps]` | the declarer's own graph | yes — walked recursively | mount error (today's law) |
| `[peer-deps]` (required, the default) | the **consumer's** closure | **never pulled** | loud D1 at mount (§4) |
| `[peer-deps]` with `optional = true` | the consumer's closure, if anywhere | never pulled | inert; referencing the integration = D2 (§4) |
| `[dev-deps]` | the pkg's own self-build | root only — never walked for a dep | n/a (they exist to be there) |

Peers are a *presence* relation, not a *pull* relation: the gate (§3)
looks at what the program's closure already contains and never adds a
package. A pkg is either pulled transitively (`[deps]`) or required of
the consumer / held for development (`[peer-deps]`/`[dev-deps]`) —
never both (§2's cross-table law).

Dev-deps exist for exactly the both-kinds shape: json develops against
real pouch/nmapset (its own tests dispatch through the group impls),
while its consumers mount json light and get the integrations only if
they already carry the peers.

## 2. The grammar

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

- Keys are bare package names — the same `[a-zA-Z0-9_]+` law as
  `name` (RFC 0041 §5); a dep-table key outside it is a line-targeted
  manifest error.
- Descriptor values are inline tables. `[peer-deps]`/`[dev-deps]`
  accept exactly three keys: `path` (string, directory-time),
  `optional` (the one bool the inline-table grammar learns —
  `` `optional` expects `true` or `false` ``), and `lib` (string; the
  peer-gated integration file, §3). Any other key is the `[entry]`
  strictness: `` unknown `[peer-deps]` key `..` ``. `[deps]` keeps
  string-only values and REJECTS the flag: `` `optional` is a
  `[peer-deps]` attribute — `[deps]` has no options ``. A `lib` ending
  in `.d.rut` is a load error: `` `lib` must be a `.rut` source — a
  `.d.rut` decl surface does not gate `` (§3).
- `optional` defaults to **false** — required — per the
  required-by-default law. The zero-dep spell `{ path = ".." }` stays
  valid in all three tables.
- **The cross-table law (D4):** a name riding `[deps]` beside
  `[peer-deps]` or `[dev-deps]` is a manifest error naming both rows.
  `[peer-deps]` + `[dev-deps]` together is the sanctioned pairing.
  The exact texts (verbatim, as landed):

  > `pouch` appears in both `[deps]` and `[peer-deps]` — a package is
  > either pulled transitively or required of the consumer, never both
  > (RFC 0045 §2)

  > `pouch` appears in both `[deps]` and `[dev-deps]` — a package is
  > either pulled transitively or held for development, never both
  > (RFC 0045 §2)

Bundles carry this manifest byte-for-byte (RFC 0038 §2); layout v3 is
what packs peer groups (§3, and RFC 0038's amendment).

## 3. The mount law — four passes, one gate

All loader-owned (`load_dir_session`'s land); the `Session` stays
I/O-free and the graph never learns what a peer is:

1. **The `[deps]` walk — unchanged.** Recursive, name order,
   first-mount-wins, cycle guard, name-mismatch error (RFC 0041 §5).
   While walking, the loader records each mounted pkg's `[peer-deps]`
   into the session's peer registry (it reads every dep's manifest
   anyway — no extra I/O).
2. **The dev pass — root only.** The program root's `[dev-deps]` mount
   exactly like `[deps]` (same walk, same first-mount-wins). A dep's
   dev table is NEVER walked — a consumer's world never contains
   another pkg's dev table. `mount_dir` (the embedder offer) mounts no
   dev-deps and runs no gate: it offers a pkg to someone else's
   program, it is not "building the pkg itself" (its peer declarations
   are still recorded).
3. **The peer gate — ONE post-closure pass.** After the full closure
   exists, for every mounted pkg P and every `[peer-deps]` entry:
   - **required** (default): the peer must resolve in the session,
     else D1 (§4). Never auto-pulled — the consumer supplies.
   - **optional, present** (any reason): *presence-based* group
     assembly — P's group file (the descriptor's `lib`, an impl-only
     `.rut`) is appended to P's `Module.source`, groups in peer-name
     order after the base. The combined text stays ONE source string,
     so every existing consumer of `Module.source` — the graph splice,
     bundles, the wasm mounts — is untouched.
   - **optional, absent**: inert; the group simply never mounts.
   - **paths**: the program root's own peer paths are read and
     name-checked even when dev-deps already supplied presence — a
     broken path is the loud D3 packaging-bug error at the pkg's own
     build. A dep's peer paths are never read: presence is by NAME
     (first-mount-wins already guarantees the consumer's own path
     won), so a broken peer path is inert for an optional peer and
     unreachable for a required one (its absence is D1's business, not
     the path's).
   The gate is post-closure precisely because a peer may mount after
   its declarer alphabetically (consumer `[deps] json, pouch` — json
   loads first, pouch not yet present; a during-walk gate would
   wrongly skip the group). Groups add no new package names, so one
   pass is a fixpoint — no iteration.
4. **Compile — unchanged.** `compile_graph` sees `Module.source`
   texts that now include groups; the splice composes each unit from
   its origin-deduplicated leaf list — the dep-kinds batch's
   companion change (§5 and the survey §2.4).

**Groups are impl-only.** A group file contains `impl` blocks (and
their private helpers); it declares NO new public names — the base
owns the trait and every public surface. This is what makes the
matrix (§4) total: the only ways to "reference" the integration are a
trait-method dispatch or a `use` of the peer's pkg name, and both
paths have dedicated peer-aware diagnostics. A group-owned free fn
would create bare-name misses — the exact thing the matrix bans. A
group appended to a module with no rut source body (a `.d.rut` decl
surface) is a load error: decl surfaces do not gate — and a
conditional *host* surface would drag `expected_host_fns` through the
peer's presence (§5).

**Bundles (RFC 0038).** `pack_dir` emits `format_version = 3` and
packs each pkg's `lib` files beside its entry in its `<pkg>/` group;
the v3 loader resolves groups by name exactly like a directory world,
then runs the same gate over the archive (a declared group the
archive lacks is a load error: refuse, never guess). A peer-deps
manifest under `format_version = 2` is refused — a v2 layout has no
group entries and would silently mount base-only, semantically wrong.
An older loader refuses a v3 bundle by the same unknown-version gate
it already had. See RFC 0038's amendment for the ledger row.

## 4. The missing-peer matrix and the diagnostics

THE MISSING-PEER MATRIX (pinned):

| case | behavior |
|---|---|
| REQUIRED peer absent from the consumer's closure | LOUD resolution error at mount: names the pkg, the peer, and the fix ("add nmapset to [deps]"). Not silent, not auto-pulled. |
| optional peer absent, integration never touched | NOTHING — silent success, that IS the feature (json mounts light, no transitive pull). |
| optional peer absent, integration-only surface referenced | the DEDICATED missing-peer diagnostic (pkg + peer + fix); never a bare unresolved-name. |
| peer PRESENT in the consumer's closure (any reason) | the integration group mounts automatically (presence-based resolution — no extra declaration beyond having the pkg). |
| self-build/dev mode | dev-deps guarantee presence; no missing case exists. |
| a [peer-deps] path that does not resolve | LOUD manifest error at the pkg's own build (a packaging bug); for consumers, a required peer errors, an optional peer is inert (group simply not mounted). |

There is exactly ONE missing-peer diagnostic shape per site, and all
four diagnostics are load/resolution-time errors — never runtime
traps. The exact landed texts:

- **D1 — required peer absent** (mount-time, matrix row 1; the loader
  gate and the bundle gate say the same):

  > pkg `json` requires the peer `nmapset`, and `nmapset` is not in
  > this program's closure — peers are not pulled transitively: add
  > `nmapset = { path = ".." }` to your `rut.toml` `[deps]` (RFC 0045
  > §3)

- **D2 — optional peer's name referenced while absent** (the
  reference-site dedicated diag; matrix row 3). `Session::resolve` is
  the ONE path every reference-site miss flows through: when the
  missed name is a declared OPTIONAL peer of some mounted pkg, the
  registry answers D2 instead of the bare NoModule text — pkg + peer
  + the integration it unlocks + the fix:

  > cannot resolve `pouch` — `json`'s pouch integration is not
  > mounted because the optional peer `pouch` is absent from this
  > program's closure; add `pouch = { path = ".." }` to your
  > `rut.toml` `[deps]` (RFC 0045 §4)

  Declaring pkgs scan in mount (name) order, so the diagnostic is
  deterministic when several pkgs declare the same peer. A REQUIRED
  peer's absence never reaches resolve through the loader (D1 fires
  at mount); in a gate-less hand-mounted world it stays the bare
  miss — D1's business, not D2's. Item-level misses can never be
  peer-gated (groups are impl-only, §3), so "never a bare
  unresolved-name" holds by construction: every peer-related failure
  surfaces on a path that knows the peer.
- **D3 — a packaging bug in the pkg's own peer descriptors** (matrix
  row 6 for the path shapes — read only at the pkg's own build; the
  group-file shape fires at gate time for any mounted pkg). Three
  shapes:

  > pkg `json`'s [peer-deps] entry `pouch` points at `../pouch` —
  > cannot read a manifest there (a packaging bug in json; RFC 0045
  > §3)

  > pkg `json`'s [peer-deps] entry `pouch` points at `../pouch` — the
  > manifest there names it `other` (a packaging bug in json; RFC
  > 0045 §3)

  > pkg `json`'s [peer-deps] entry `pouch` names the group
  > `./serde_pouch.rut` — cannot read it (a packaging bug in json;
  > RFC 0045 §3)

- **D4 — cross-table name collision** (§2), both texts quoted there.

## 5. Interplay

- **Placement/orphan (RFC 0012) — the gate precedes the check.** The
  peer gate runs at LOAD; placement and pair-uniqueness run at
  COMPILE/LINK. The checker never sees a half-mounted world: peer
  absent → the group text was never assembled → there is no `impl
  JsonSerialize for Vec<T>` anywhere to place, and no orphan question
  exists; peer present → the group's impl is a trait impl in json —
  legal by RFC 0012 §2's placement law ("trait impls are legal in any
  module"; the trait is json's own, and json is where the impl lives —
  the peer-gated impl stays trait-local), and it registers under
  whatever unit holds it (the consumer's, if spliced; json's, if
  linked).
- **The duplicate-pair link error is the consumer-side guard**
  (RFC 0012 §5): a consumer who hand-writes `impl JsonSerialize for
  Vec<T>` in a world where json's pouch group mounted gets the
  link-time duplicate — loud, correct: the group already provides it.
  Documented behavior, not a gap.
- **Splice dedup (the seam law).** Peer groups make overlapping
  transitive closures the norm, so the graph composes each inline
  unit from its ordered leaf list, deduped by origin spec (first
  position wins, topological order preserved) — the dep-kinds batch's
  companion change, designed in the same survey (§2.4). In the
  no-collision case the composed text is byte-identical to the old
  per-dep `extra + src` law — every existing program compiles
  identically. See the batch report for the proof.
- **VERSION/verify — no move.** Impl-only groups declare no host fns
  (`expected_host_fns` unchanged — no binding obligations appear or
  vanish for the embedder), add no binary sections (impls ride the
  existing `surface.impls` encoding), and touch nothing `verify`
  reads. A consumer's binary differs by which impls registered, which
  is ordinary program content. Module VERSION stays 8
  (RFC 0033 §2); the bundle format_version ledger is separate
  (RFC 0038's amendment).
- **Dev-deps are invisible to bundles-as-consumers**: a packed json
  carries its peer groups; a consumer packing THEIR app never pulls
  json's dev table (pass 2 is root-only), so dev-only convenience
  pkgs cannot leak into consumer worlds.
- **The LSP** embeds the toolchain pkgs' sources and never parses
  manifests (RFC 0041 §2's tree is invisible to it); peer gating is a
  resolution-time concept it does not model, so group impls
  complete/hover unconditionally. Recorded as accepted — the LSP is
  advisory; the compiler is the law. Gating its completions by a
  peer-aware session is future menu, not law here.

## Open questions

- OQ-1: registry/index and version-range deps — deliberately NOT
  here. `[peer-deps]` is a *presence* relation over mounted packages;
  a registry, version selection, or lockfile layer would change what
  "the consumer supplies" means. The tables are shaped so such a
  layer can land beside them (descriptors are key/value maps) without
  re-litigating the kinds.
- OQ-2: rut/json — the real extraction. The toolchain fixture world
  proves the law hermetically; extracting the actual json pkg from
  `examples/02-digest` is the json lane's property, with the survey's
  constraint recorded: today's `Json` struct rides pouch's `Vec`, and
  the extracted base must be pouch-free (every pouch-typed surface
  lives in the peer group).
- OQ-3: the LSP's peer-aware gating (§5) — worth doing only when the
  LSP parses manifests; until then the gap is honest and bounded
  (advisory surface only).
