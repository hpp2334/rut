# Dependency kinds

Every package's `rut.jsonc` names the packages it relates to, and the
*kind* of each relation decides how — and whether — it mounts. Three
tables, one grammar:

- **`deps`** — the transitively mounted dependencies.
- **`peer-deps`** — **required by default**: not transitively pulled;
  the *consumer* must supply the peer. With `optional = true`, presence
  in the consumer's closure mounts the integration; absence is inert.
- **`dev-deps`** — mounted only when building/testing the package
  itself (the program root), never in a consumer's world.

Peers are a *presence* relation, not a *pull* relation: the gate looks at
what the program's closure already contains and never adds a package. A
package is either pulled transitively (`deps`) or required of the
consumer / held for development (`peer-deps`/`dev-deps`) — never
both.

## The grammar

```jsonc
// rut/json/rut.jsonc — the reference shape
{
  "name": "json",
  "entry": { "lib": "./json.rut" },
  "inline": true,

  "deps": {
    "strbuild": { "path": "../strbuild" }
  },

  "peer-deps": {
    "pouch":   { "path": "../pouch",   "optional": true, "lib": "./group-pouch.rut" },
    "nmapset": { "path": "../nmapset", "optional": true, "lib": "./group-nmapset.rut" }
  },

  "dev-deps": {
    "pouch":   { "path": "../pouch" },
    "nmapset": { "path": "../nmapset" }
  }
}
```

- Keys are bare package names — the same `[a-zA-Z0-9_]+` law as `name`;
  a dep-table key outside it is a path-targeted manifest error.
- `peer-deps`/`dev-deps` descriptors accept exactly three keys:
  `path` (string, directory-time), `optional` (the one bool the inline
  grammar learns), and `lib` (string; the peer-gated integration file).
  Anything else is the strict-manifest error. `deps` keeps
  string-valued descriptors and **rejects** `optional` — it has no
  options.
- `optional` defaults to `false` — required by default. The zero-dep
  spelling `{ path = ".." }` is valid in all three tables.
- A `lib` ending in `.d.rut` is a load error: the integration must be a
  `.rut` source — a declaration surface does not gate.
- **The cross-table law**: a name riding `deps` beside `peer-deps`
  or `dev-deps` is a manifest error naming both rows — *`pouch`
  appears in both `deps` and `peer-deps` — a package is either
  pulled transitively or required of the consumer, never both*. Peer +
  dev together is the sanctioned pairing: integration-if-present for
  consumers, always-present while developing the package itself.

## Url dependencies — the second source kind

A `deps` descriptor may name a remote `.rutbundle` instead of a
directory:

```jsonc
{
  "deps": {
    "pouch": { "url": "https://example.com/pouch.rutbundle", "sha256": "<64-hex>" }
  }
}
```

- **One descriptor, one source**: `path` or `url`, never both, never
  neither — a path-targeted manifest error either way. `sha256` is
  legal only beside `url` and must be exactly 64 hex digits (stored
  lowercase; the comparison is byte-exact). The url itself must be
  http(s). `peer-deps`/`dev-deps` stay `path`-only — a peer's path
  is directory-time metadata for the declarer's own build, nothing to
  fetch.
- **The layer split.** The *call site* owns HOW bytes arrive —
  transport, caching, offline policy — by implementing the `DepRemote`
  contract (`fetch(url)` — one required method returning a boxed future
  of bytes — plus optional `lookup`/`write`; the standard
  `HttpRemote` is cache-first). The *loader*
  owns WHAT the bytes are declared to be: the `sha256` pin is manifest
  law, verified at the mount door on **every** load — fresh fetch,
  cache hit, vendored map, test fixture. A check the call site
  performs would be a check the call site could skip; only the mount
  point holds both the pin and the bytes, so only the mount point can
  enforce it.
- **Url deps are leaves.** The `deps` walk never follows them: one
  fetch brings the whole closure, because **bundles stay closed** —
  every dep a bundle's manifest declares must be satisfied by an
  in-archive group, and a bundle that does not satisfy one is refused
  at the mount door. A url row inside an already-packed bundle is
  inert metadata; loading a bundle never fetches.
- **Pin your deps.** An unpinned url compiles from whatever the cache
  or the network served — a pinned one refuses to load anything else,
  naming the dep, the url, and both hashes. Pins are what make packs
  reproducible: same manifest + same pins ⇒ byte-identical bundle.
- The CLI lane: `rut run`/`rut pack` fetch through the cache-first
  standard remote (`<project>/.rut/cache`, `$RUT_CACHE_DIR` overrides;
  a hit never touches the network), and a pin
  refusal evicts the poisoned cache entry so the next run heals.
  `rut fetch <dir>` warms the cache without running anything
  ([The rut CLI](cli.md)).

### The std tree on jsDelivr

**Which kind, when.** The toolchain's standard packages (`core`,
`ink`, `pouch`, `json`, …) are consumed as **pinned url rows** from
this CDN — that is the recommended import for everything the toolchain
ships. A `path` row is for **your own local packages**: a sibling
directory in the same project (the modules tutorial's `greet` app
mounting `../pkg` is the shape). The tag advances with format changes
(`std-v5` today) and is never re-pointed, so a pin at a tag stays
honest forever.

The std tree ships as committed per-package bundles —
`https://cdn.jsdelivr.net/gh/hpp2334/rut@<tag>/dist/std/<pkg>.rutbundle`,
pinned by sha256 (the shapes here keep placeholder tags; the worked
manifests in the quick-start, the tutorial, and the examples carry the
real pins — a pin at an immutable tag cannot rot). Tags advance
(`std-vNN`) and are never re-pointed — jsDelivr caches aggressively;
the artifacts are committed at `dist/std/`, packed by
`scripts/pack-std.cjs` (whose `--check` gate is a pure byte-equality
repack — CI never touches the network).

**What a url row can deliver** is the engine's instantiation law, read
from the CDN side: a compiled bundle carries the generic instantiations
its own pack closure spelled **in its ledger**, and — since the
generic-source riding law — a compiled pkg whose surface exports
generics **also rides the source that serves consumer-spelled shapes**:

- **host surfaces and concrete-class libs deliver** — a `type = "host"`
  bundle is a declaration surface (no generics), and a lib of concrete
  classes (`http`, `ink`, `strbuild`) crosses on its surface's inherent
  rows with its whole closure riding inside;
- **generic owners deliver too** — `pouch`, `nmapset`, `json`,
  `futures` ride their entry + group source beside the binaries, so
  a consumer's `Vec<Todo>` or `decodeJson<Vec<Todo>>` compiles **at the
  consumer's link**: the ridden text lowers in the consumer's session,
  the monomorphized bodies register under the declaring pkg's spec (one
  row program-wide, identity by owner), and nothing persists (`.rutc`
  caches stay pack-time). A bundle that predates the riding — no source
  beside the binary — refuses a consumer-spelled shape loudly and says
  so: re-pack it. A bundle-mounted json also names its pack-time dev
  closure in its ledger, so the consumer's closure must contain those
  names (`pouch`, `nmapset` beside `json` — the six-pin law).

## Semantics

| table | who supplies it | transitive? | missing behavior |
|---|---|---|---|
| `deps` | the declarer's own graph | yes — walked recursively | mount error |
| `peer-deps` (required, the default) | the **consumer's** closure | never pulled | loud resolution error at mount |
| `peer-deps` with `optional = true` | the consumer's closure, if anywhere | never pulled | inert; referencing the integration is the dedicated missing-peer diagnostic |
| `dev-deps` | the pkg's own self-build | root only — never walked for a dep | n/a (they exist to be there) |

## The mount law — four passes, one gate

All loader-owned; the session stays I/O-free and the compile graph never
learns what a peer is.

1. **The `deps` walk — unchanged.** Recursive, name order,
   first-mount-wins, cycle guard, name-mismatch error. While walking,
   the loader records each mounted package's `peer-deps` into the
   session's peer registry (it reads every dep's manifest anyway).
2. **The dev pass — root only.** The program root's `dev-deps` mount
   exactly like `deps`. A dep's dev table is **never** walked — a
   consumer's world never contains another package's dev table. Embedder
   mounting (`mount_dir`) offers a package to someone else's program: it
   mounts no dev-deps and runs no gate (its peer declarations are still
   recorded).
3. **The peer gate — one post-closure pass.** After the full closure
   exists, for every mounted package and every `peer-deps` entry:
   - **required**: the peer must resolve in the session, else the D1
     error. Never auto-pulled.
   - **optional, present** (any reason): the declarer's group file (the
     descriptor's `lib`, an impl-only `.rut`) is appended to its
     `Module.source` — groups in peer-name order after the base. The
     combined text stays one source string, so every consumer of module
     sources (the splice, bundles, wasm mounts) is untouched.
   - **optional, absent**: inert — the group simply never mounts.
   - **paths**: the program root's own peer paths are read and
     name-checked even when dev-deps already supplied presence — a
     broken path is the loud D3 packaging-bug error at the package's own
     build. A dep's peer paths are never read: presence is by name, so a
     broken peer path is inert for an optional peer and unreachable for
     a required one.
   The gate is post-closure because a peer may mount after its declarer
   alphabetically; groups add no package names, so one pass is a
   fixpoint.
4. **Compile — unchanged.** The graph splices each compilation unit from
   its origin-deduplicated leaf list (first position wins, topological
   order preserved); in the no-collision case the composed text is
   byte-identical to the pre-peer law.

**Groups are satisfaction-only.** A group file contains the peer
integration's wrapper families — newtype class decls and their
inherent impls (and private helpers); it declares no host fns and no
manifest rows — the base owns the interfaces and every other public
name, and the wrappers are the group's whole addition. This is what
keeps the diagnostic matrix total: the only ways to reach the
integration are through the wrapper names (which `use` paths route
into the declaring package) or the base package's interfaces, and both
have dedicated peer-aware diagnostics. A group appended to a module
with no rut body (a `.d.rut` surface) is a load error — declaration
surfaces do not gate.

## The missing-peer matrix

| case | behavior |
|---|---|
| required peer absent from the consumer's closure | **D1** — loud at mount: names the package, the peer, and the fix. Not silent, not auto-pulled. |
| optional peer absent, integration never touched | nothing — silent success; that *is* the feature |
| optional peer absent, integration referenced | **D2** — the dedicated missing-peer diagnostic; never a bare unresolved name |
| peer present (any reason) | the integration mounts automatically — presence-based resolution |
| self-build / dev mode | dev-deps guarantee presence; no missing case exists |
| a `peer-deps` path that does not resolve | **D3** — loud manifest error at the pkg's own build; for consumers, required ⇒ D1, optional ⇒ inert |

The four diagnostics, verbatim shapes (all load/resolution-time errors,
never runtime traps):

- **D1** — `pkg \`json\` requires the peer \`nmapset\`, and \`nmapset\`
  is not in this program's closure — peers are not pulled transitively:
  add "nmapset": { "path": ".." } to your \`rut.jsonc\` \`deps\``
- **D2** — `cannot resolve \`pouch\` — \`json\`'s pouch integration is
  not mounted because the optional peer \`pouch\` is absent from this
  program's closure; add "pouch": { "path": ".." } to your
  \`rut.jsonc\` \`deps\``. Declaring packages scan in mount order, so
  the diagnostic is deterministic when several packages declare the same
  peer. Required peers never reach this path — D1 fires at mount.
- **D3** — three shapes, all *a packaging bug in json*: cannot read a
  manifest at the peer path; the manifest there names it `other`; cannot
  read the group file.
- **D4** — the cross-table collision (both texts quoted above).

## Interplay

- **Placement and the gate — the gate precedes the compile.** The peer
  gate runs at load; the group compiles inside the declaring package's
  unit. Peer absent ⇒ the group text was never assembled ⇒ the wrapper
  families are nowhere in the program. Peer present ⇒ the group's
  wrapper classes and their inherent impls compile in the declaring
  package — the inherent impl's one home
  ([Interfaces and dispatch](interfaces.md)).
- **Structural satisfaction is the consumer-side story**: a consumer's
  own types satisfy json's interfaces by having the members, whether
  or not any peer mounted; what the peer gate controls is the
  wrapper families only — a `JsonVec` spelling with `pouch` absent
  produces D2, never a mystery.
- **Binary format untouched.** Group files declare no host fns
  (no binding obligations appear or vanish), add no binary sections, and
  touch nothing the verifier reads — the wrapper classes are ordinary
  program content. A consumer's binary differs only by which wrappers
  mounted.
- **Dev-deps are invisible to bundles-as-consumers.** A packed package
  carries its peer groups; a consumer packing *their* app never pulls
  the package's dev table (pass 2 is root-only), so dev-only
  convenience packages cannot leak into consumer worlds
  ([Module bundles](bundles.md)).
- **The LSP completes group impls unconditionally.** Peer gating is a
  resolution-time concept the language server does not model; it embeds
  the toolchain packages' sources and never parses manifests. Recorded
  as accepted: the LSP is advisory; the compiler is the law.

## Consuming a peer-gated package

```jsonc
// an app that wants json's pouch integration
{
  "deps": {
    "json":  { "path": "vendor/json" },
    // presence is the only requirement
    "pouch": { "path": "vendor/pouch" }
  }
}
```

The group mounts because `pouch` is anywhere in the closure — no extra
declaration beyond having the package. Drop the `pouch` row and `json`
mounts light; the first `use` naming a `JsonVec`-family wrapper
produces D2 instead of a mystery.
