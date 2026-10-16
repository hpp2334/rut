# Dependency kinds

Every package's `rut.jsonc` names the packages it relates to, and the
*kind* of each relation decides how — and whether — it mounts, and
whether the declarer may name it. Two tables, one grammar, strict
keys:

- **`deps`** — the transitively mounted dependencies — and the
  declaration table: a pkg may `use` exactly the heads its manifest
  names.
- **`dev-deps`** — mounted only when building/testing the package
  itself (the program root), never in a consumer's world.

A package is either pulled transitively (`deps`) or held for
development (`dev-deps`) — never both. There is no third relation:
the strict-keys law refuses any other top-level key naming the key,
so a retired table (`peer-deps`) or flag (`inline`) refuses as the
unknown key it now is — `manifest: unknown key 'peer-deps'`.

## The grammar

```jsonc
// rut/json/rut.jsonc — the reference shape
{
  "name": "json",

  "deps": {
    "strbuild": { "path": "../strbuild" },
    "pouch":    { "path": "../pouch" },
    "nmapset":  { "path": "../nmapset" }
  }
}
```

- Keys are bare package names — the same `[a-zA-Z0-9_]+` law as `name`;
  a dep-table key outside it is a path-targeted manifest error.
- A `deps` descriptor accepts exactly one source: `path` (a directory,
  relative to this manifest) or `url` (a remote `.rutbundle`,
  http(s)) — both set, or neither, is the one-descriptor-one-source
  manifest error. Any other key is the strict-manifest error naming
  the path (`deps.pouch: unknown key 'feats'`).
- A `dev-deps` descriptor is **`path`-only** — dev rows mount only for
  the declarer's own build walk (directories, never fetches); `url`,
  `optional`, and `lib` refuse as the unknown keys they are.
- The zero-dep spelling `{ path = ".." }` is valid in both tables.
- **The cross-table law**: a name riding `deps` beside `dev-deps` is
  a manifest error naming both rows — *`pouch` appears in both `deps`
  and `dev-deps` — a package is either pulled transitively or held
  for development, never both*.

## Url dependencies — the second source kind

A `deps` descriptor may name a remote `.rutbundle` instead of a
directory:

```jsonc
{
  "deps": {
    "pouch": { "url": "https://example.com/pouch.rutbundle" }
  }
}
```

- **One descriptor, one source**: `path` or `url`, never both, never
  neither — a path-targeted manifest error either way. The url itself
  must be http(s). `dev-deps` stays `path`-only — a dev row is
  directory-time metadata for the declarer's own build, nothing to
  fetch.
- **The manifest row carries no pin.** The old optional `sha256` key
  is **retired** from the descriptor grammar — the strict (directory)
  lane refuses it, naming the fix: `deps.pouch: 'sha256' retired —
  pins live in rut.lock (run `rut install`)`. Pins live in the walk
  root's **`rut.lock`** ([`rut install`](cli.md) writes it), read
  fresh on every load. (Inside a *published* bundle the reader's
  compat lane still parses the old key — inert metadata, never
  enforced — [module bundles](bundles.md).)
- **The layer split.** The *call site* owns HOW bytes arrive —
  transport, caching, offline policy — by implementing the `DepRemote`
  contract (`fetch(url)` — one required method returning a boxed future
  of bytes — plus optional `lookup`/`write`; the standard
  `rut_native::HttpRemote` is cache-first). The *walk* owns WHAT the
  bytes are declared to be: the pin is **lock law** — the closure's
  url rows are collected at the root, gated against `rut.lock` before
  any fetch (an unpinned url is refused before the wire), and the
  fetched bytes are verified against the lock's pin at the mount door
  on **every** load — fresh fetch, cache hit, vendored map, test
  fixture. A check the call site performs would be a check the call
  site could skip; only the mount point holds both the pin and the
  bytes, so only the mount point can enforce it.
- **Url deps are leaves.** The `deps` walk never follows them: one
  fetch brings the whole closure, because **bundles stay closed** —
  every dep a bundle's manifest declares must be satisfied by an
  in-archive group, and a bundle that does not satisfy one is refused
  at the mount door. A url row inside an already-packed bundle is
  inert metadata; loading a bundle never fetches.
- **The lock is the pin lane.** A url the lock has no row for refuses
  at the mount door, naming the fix: ``dep `pouch` (https://…) is not
  pinned — rut.lock has no row for it: run `rut install` ``; fetched
  bytes that break the lock's pin refuse too — ``dep `pouch` — sha256
  pin mismatch for <url>: rut.lock pins <sha>, the fetched bytes hash
  to <sha>``. Reproducibility is the pair's law: **same manifest +
  same `rut.lock` ⇒ identical resolved world** — the lock pins are
  what make packs reproducible, and `--locked` verifies the pair
  before anything runs.
- The CLI lane: `rut run`/`rut pack` fetch through the cache-first
  standard remote (`<project>/.rut/cache`, `$RUT_CACHE_DIR` overrides;
  a hit never touches the network), and a pin refusal evicts the
  poisoned cache entry so the next run heals. `rut install <dir>`
  resolves + fetches + writes the lock (`--locked` verifies without
  writing — the CI lane), and `rut fetch <dir>` re-warms the cache
  without running anything ([The rut CLI](cli.md)).

### The std tree on jsDelivr

**Which kind, when.** The toolchain's standard packages (`core`,
`ink`, `pouch`, `json`, …) are consumed as **url rows pinned by the
project's `rut.lock`** from this CDN — that is the recommended import
for everything the toolchain ships. A `path` row is for **your own
local packages**: a sibling directory in the same project (the modules
tutorial's `greet` app mounting `../pkg` is the shape). The tag
advances with format changes (`std-v11` today) and is never re-pointed,
so a lock pin at a tag stays honest forever.

The std tree ships as committed per-package bundles —
`https://cdn.jsdelivr.net/gh/hpp2334/rut@<tag>/dist/std/<pkg>.rutbundle`.
The manifests here keep placeholder tags; the worked manifests in the
quick-start, the tutorial, and the examples carry the real urls, and
their committed `rut.lock`s carry the real pins (a pin at an immutable
tag cannot rot). Tags advance (`std-vNN`) and are never re-pointed —
jsDelivr caches aggressively; the artifacts are committed at
`dist/std/`, packed by `scripts/pack-std::cjs` (whose `--check` gate
is a pure byte-equality repack — CI never touches the network).

**What a url row can deliver** is the engine's instantiation law, read
from the CDN side: a compiled bundle carries the generic instantiations
its own pack closure spelled **in its ledger**, and — the delivery law
(v22) — its exported generic decls ride **as templates inside the
binary**:

- **host surfaces and concrete-class libs deliver** — a `type = "host"`
  bundle is a declaration surface (no generics), and a lib of concrete
  classes (`http`, `ink`, `strbuild`) crosses on its surface's inherent
  rows with its whole closure riding inside;
- **generic owners deliver too** — `pouch`, `flow`, `nmapset`, `json`,
  `futures` carry their generic bodies as templates in the binaries,
  so a consumer's `Vec<Todo>` or `decodeJson<Vec<Todo>>` compiles **at
  the consumer's link**: a shape the ledger lacks monomorphizes from
  the template, the body registers under the declaring pkg's spec (one
  row program-wide, identity by owner), and nothing persists (`.rutc`
  caches stay pack-time). The law binds both sides: a pkg that owns an
  open generic surface but whose binary carries no template bodies
  refuses at the mount (corrupt or doctored — re-pack the directory).
  And a bundle is **closed**: every dep its manifest
  declares rides as an in-archive group, so a bundle-mounted json
  carries `strbuild`, `pouch`, and `nmapset` inside — the ordinary
  closure law, nothing to supply at the consumer's side.

## Semantics

| table | who supplies it | transitive? | missing behavior |
|---|---|---|---|
| `deps` | the declarer's own graph | yes — walked recursively | mount error |
| `dev-deps` | the pkg's own self-build | root only — never walked for a dep | n/a (they exist to be there) |

The tables decide mounting; what a mounted head lets its consumer
*name* is the declaration-scoped law of
[Nameability](#nameability) below.

## The mount law — three passes

All loader-owned; the session stays I/O-free.

1. **The `deps` walk.** Recursive, name order, first-mount-wins,
   cycle guard, name-mismatch error. A dep mounted from a url bundle
   brings its whole closure with it: the archive's groups mount under
   their content identities (`name@<16 hex>`) — first-mount-wins
   becomes per-identity exact dedup ([Module
   bundles](bundles.md), [Coexistence](#coexistence)).
2. **The dev pass — root only.** The program root's `dev-deps` mount
   exactly like `deps`. A dep's dev table is **never** walked — a
   consumer's world never contains another package's dev table.
   Offering a package to someone else's program
   (`rut_native::dir_pkgs`, or `.pkg(..)` by hand) is the same law:
   it walks no dev-deps.
3. **Compile.** Every mounted module links — sources compile in
   dependency post-order, and the walk's yield is the whole program.
   There is no presence-gated append and no splice lane: nothing
   changes shape after the walk.

## Nameability

The passes above decide what **mounts**; a separate, declaration-scoped
law decides what a source may **name**. Mounting stays transitive —
A's `deps` still pull B into the closure, and A still compiles against
B's mounted rows — but nameability belongs to the consumer's own
manifest: a package may `use` exactly the heads it declares itself,

```text
deps ∪ (dev-deps, only when the package is the program root) ∪ core
```

— and there is no ambient tier. `core` rides unconditionally (the
graph threads the prelude into every unit, and the resolver passes the
`core` head without a row); every other head must be the consumer's
own declaration. Over-declaring stays legal — the law bounds uses by
declarations, never declarations by uses — so a row naming a head the
source never spells is inert.

The dedicated diagnostic is **D5**, and it fires only when a head is
MOUNTED but UNDECLARED by the consumer — an unmounted head keeps its
existing shape (`NoModule` for a name nothing offers). D5 names the
mount edge — who pulled the head in — and the fix, in two renderings:

- **The walked edge** (the consumer mounts through the dep walk, an
  archive group, or a dev row — there is a first-mount `mounted_by`
  edge to name): the puller and the jsonc row to add:

  ```text
  cannot resolve `b` — `b` is mounted in this program (pulled by `a`) but `app` does not declare it; add "b": { "path": ".." } to `app`'s `rut.jsonc` `deps`
  ```

- **The host edge** (a hand offer — no walk edge exists): the
  "offered by the host" clause, and a fix that names the
  manifest-builder doors, never a file:

  ```text
  cannot resolve `b` — `b` is mounted in this program (offered by the host) but `c` does not declare it; declare it in the pkg's manifest — `Manifest::lib("c", &["b"])` or a `"deps"` row through `parse_manifest(..)`
  ```

The closure check reads every mounted source's uses at every depth, so
the law is per-package, not root-only: a dep naming a head it never
declared is the same D5 with that dep as the consumer. dev-deps follow
the same root-only law as their mount pass — the program root may name
its dev-deps; a dep naming the root's dev-dep is D5.

**This is a breaking law change.** Mounting was always transitive, and
nameability used to ride along with it: a program could `use` any head
the closure happened to contain. Previously-compiling programs that
rode a transitive head now refuse with D5 and the fix row — add the
head to the consumer's own manifest.

Embedders and hand-mounted worlds are lawed by the same declaration
table, carried structurally on the `Pkg` — `parse_manifest` for an
embedded jsonc, `Manifest::lib(name, &[heads])` for an ad-hoc surface
([embedding](embedding.md)). The wasm playground stamps the same
shape, defaulting the program's declaration host-side to the offered
lib names.

## Coexistence

A dep group mounted from a bundle carries its **content identity** —
`name@<16 hex>`, the first 16 hex chars of the sha256 over the group's
own rode source set (`rut.jsonc`, `mod.rut`, `mod.d.rut`, `rut.mods`;
fixed order, absent files skipped). Same source ⇒ same identity, and
the same identity mounts once (exact dedup); two identities of one
name mount side by side — two builds of `util` in one program without
colliding. `@` sits outside the `[a-zA-Z0-9_]+` name alphabet, so an
identity can never collide with a bare package name.

A bare `use` head stays exact while one identity spells the name;
under several, it refuses naming them all:

```text
cannot resolve `util` — 2 identities of that name are mounted (`util@0123…`, `util@4567…`); a bare use head is ambiguous under coexistence — mount one of them under its bare name or bind through the bundle's scope ledger
```

— mount one of them under its bare name (your own `deps` row) or bind
through the bundle's scope ledger ([Module bundles](bundles.md)).

## Interplay

- **Dev-deps are invisible to bundles-as-consumers.** A packed package
  carries its closure; a consumer packing *their* app never pulls the
  package's dev table (pass 2 is root-only), so dev-only convenience
  packages cannot leak into consumer worlds
  ([Module bundles](bundles.md)).
- **Structural satisfaction is unaffected**: a consumer's own types
  satisfy json's interfaces by having the members, whether the
  interface came from a path row or a packed bundle.
- **The LSP is advisory.** The language server embeds the toolchain
  packages' sources, never parses manifests, indexes every dep it can
  reach (any depth, the root's dev-deps too), and completes heads the
  compiler's D5 would refuse for an undeclaring consumer — it runs no
  declaration-scoped filter and raises no D5. Recorded as accepted:
  the LSP is advisory; the compiler is the law.

## Consuming a compiled package

```jsonc
// an app that wants json's whole surface
{
  "deps": {
    "json": { "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/json.rutbundle" }
  }
}
```

One row is the whole story: the bundle is closed, and json's own
`deps` — `strbuild`, `pouch`, `nmapset` — ride inside it as groups, so
every wrapper family resolves wherever json travels. The same shape
works with `path` rows into your own tree; the walk pulls the
declared table transitively either way.
