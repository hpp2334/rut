# Module bundles

`rut pack mod/` reads a module directory and emits **one file** —
`mod.rutbundle` — a zip archive carrying the module in one of two root
kinds, routed by the manifest's `type` key: a **lib** pkg packs
**compiled** (the root's `mod.rutc` binary, its scope ledger, and each
dependency as a group under `deps/<name>@<fp16>/`), and a
`type = "host"` pkg packs as a **decl** root (its `.d.rut` surface
rides as source; single-package). A bundle is the same contract as the
directory it was packed from, in one file: the loader mounts it
exactly like the directory, and the two compile to identical programs.

## Layout rules

- **The archive root carries only the root's own files** — `rut.jsonc`,
  `rut.scopes`, `mod.rutc`, the root's surface/source/rows. Every dep
  group rides one level down, at `deps/<name>@<fp16>/…` keyed by the
  group's **content identity**. No metadata entries; nothing else at
  the root. Unknown extra entries are ignored by loaders (forward
  compatibility) — except in a decl bundle, where a group entry, a
  root `.rutc`, or a ledger is a contradiction and refuses (see [the
  decl root](#the-decl-root-host)).
- **Entry names are canonical** — `mod.rutc`, `mod.d.rut`, `mod.rut`,
  `rut.mods` at both levels. The archive is name-free: whatever a
  directory grammar spelled (`<pkg>.rutc`, a custom `entry.type`
  surface), the wire spelling is the canonical one.
- The manifest's `name` is the package name the bundle answers to — a
  bare `[a-zA-Z0-9_]+` name, the same law as directory manifests. A
  group's manifest `name` must equal its dirname's name half (the
  identity's `name@`).
- No entry reaches outside the archive: there is no outside — a
  bundle's sources compile from the archive's own bytes alone.

## The compiled root (lib)

```text
rut.jsonc                byte-for-byte; "format": "rutbundle", "format_version": 11
rut.scopes               the pack-time scope ledger — one `<scope> = "<identity>"`
                           row per linked module, ascending by scope; every row
                           spells name@<16 hex> (the group identities; the loader
                           maps the self-row and engine rows to their bare mounts)
mod.rutc                 the root's compiled binary — bodies + surface
mod.d.rut                the root's surface text (humans, LSP), when it declares
                           one — the canonical name, whatever the directory
                           grammar spelled
rut.mods                 the root's module rows, when it has mod children
deps/<name>@<fp16>/      one directory per dep group, keyed by the group's
                           CONTENT IDENTITY — fp16 is the first 16 hex chars of
                           the sha256 over the group's own rode source set
                           (rut.jsonc + mod.rut + mod.d.rut + rut.mods, fixed
                           order, absent files skipped); same source ⇒ same
                           dirname, two identities of one name coexist side by
                           side
deps/<name>@<fp16>/rut.jsonc   the group's manifest, byte-for-byte
deps/<name>@<fp16>/mod.rutc    a compiled group (the ordinary case: a source
                           body the graph linked) — bodies + surface, the
                           linking truth; the binary carries the closure's
                           instantiations in its ledger AND its generic
                           bodies as templates (v22)
deps/<name>@<fp16>/mod.d.rut   the group's surface text, when it declares one
deps/<name>@<fp16>/rut.mods    its module rows, when it has mod children
deps/<name>@<fp16>/…           a host pkg (or a declared dep whose unit did
                           not link) → a declaration file set: no binary, the
                           source is the interface
```

- The **`.rutc` binaries are the linking truth**: bodies plus the
  surface on the wire ([module binary and
  verification](module-binary.md)). A consumer binds a compiled group's
  exports from the binary alone — nothing is rebuilt from `.d.rut`
  text. The `.d.rut` files ride for humans and tooling; the loader
  never reads them.
- The **scope ledger** names the pack-time scope of every linked module
  in the closure (engine mounts included), and every row spells the
  owner's content identity. A decoded program's ids spell its pack-time
  scopes; the loader resolves each through the ledger to the module to
  ensure and rebases the ids onto its own numbering — pack-time and
  load-time numbering never have to agree. A reference with no ledger
  row is refused.
- **The module rows are the tree's wire form** (`rut.mods` —
  `deps/<ident>/rut.mods` for a group): one JSON object, mod path →
  source text, `""` = the root module, one row per mounted child. A
  package with no mod children writes no rows entry — a flat package
  packs exactly as before, byte-stable. The section is **additive**:
  the reader accepts both envelopes, so a bundle packed before file
  modules keeps loading — that is a loading law, not a transition; a
  NEW directory whose manifest spells the repealed
  `entry.lib`/`entry.libs` keys refuses loudly at the manifest walk.
- **Group kinds are the packer's one decision**: a source body rides
  compiled iff the graph linked it; a host body (or a declared dep
  whose unit did not link) rides its declaration file set. Refuse,
  never guess: the packer calls the graph's own classifier, there is
  no second implementation.
- **Generics ride compiled — the delivery law (v22)**: instantiation
  is owned by the declaring package, and the pack walk seeds each
  generic dep with the instantiations its consumers spell — the
  binary's instantiation ledger names every `(owner, decl, arguments)`
  row the closure compiled. A compiled pkg whose surface exports an
  OPEN generic surface (generic fns, generic type exports, generic
  methods) ALSO carries the generic BODIES as templates inside the
  binary (v22 — the wire's templates section): every exported generic
  decl compiles once against `#<param>` placeholder rows at pack time,
  and a consumer-spelled shape the ledger misses monomorphizes from
  the template at link — one row program-wide, owner-anchored,
  indistinguishable from a pack-time one, nothing persisted. **A
  published bundle never carries source**: the ledger is the fast
  path, the templates the only other lane, and a request missing both
  is a torn or doctored bundle — the mount refuses loudly (`pouch`
  owns an open generic surface but its binary carries no template
  bodies — corrupt or doctored, re-pack the directory), never
  mislinks. `--strip` is legal with a generic surface
  unconditionally: a successful pack's templates cover by
  construction.
- **The root must compile** — a root that produces no compiled program
  is refused: `pack: <pkg> produced no compiled program — a host pkg
  (a `.d.rut` surface with no body) cannot be a bundle root; pack a
  source pkg instead`.

## The decl root (host)

A `type = "host"` pkg's root IS its declaration surface, so a host
bundle is the manifest plus that surface, riding as source —
single-package, no ledger, no groups, nothing to decode:

```text
rut.jsonc                byte-for-byte; "format": "rutbundle", "format_version": 11
mod.d.rut                the declaration surface — the ONE canonical name,
                           whatever the directory grammar spelled
rut.mods                 the surface's mod tree, when it has mod children
```

The root file rides at the canonical `mod.d.rut` name — the directory
keeps its own spelling (`entry.type` or the `mod.d.rut` default root),
and the archive is name-free. There is no override lane at the reader:
the surface entry is `mod.d.rut`, full stop.

The mod tree rides the same additive rows envelope a lib body tree
uses (see [layout rules](#layout-rules)): the tree mounts at pack
time (a declared child with no `NAME/mod.d.rut` refuses the pack),
travels as path-keyed source rows, and remounts at load — a row under
`net/` registers path-qualified exactly as it does from a directory. A
FLAT host pkg writes no rows entry: its bundle stays the two-entry
shape, byte-identical.

`rut pack <host-dir>` works (the surface verifies by parsing and
lowering once — the exact lane a mount runs — then rides as source),
`--strip` refuses on a host root (`no symbols to strip` — no programs,
no sidecar), and `rut run <host.rutbundle>` refuses with intent: a
host bundle carries a declaration surface — nothing to run; bind its
rows from the embedder ([host fns](host-fns.md)). The mount is the
same lane a host group rides inside a compiled bundle: the surface
lowers into the pkg's host rows.

## The manifest — `rut.jsonc`

First entry in the zip; JSONC (the directory grammar: `//` and `/* */`
comments and trailing commas are legal); the same manifest grammar the
directory form uses, byte-for-byte the directory's manifest — which is
what makes a bundle-shaped directory pack unchanged:

```jsonc
{
  // the bundle wire — 11, always; the manifest's `type` routes the
  // root kind (the module-binary version is a different constant)
  "format": "rutbundle",
  "format_version": 11,
  // the package this bundle answers to
  "name": "plugin",

  // the whole dep closure rides the archive, as groups under
  // deps/<name>@<fp16>/ — this manifest is the closure's map
  "deps": {
    "server": { "path": "../server" }
  }
}
```

Directory loading **ignores** `format`/`format_version` — a directory is
not a bundle — which is why the keys are safe to write into every module
manifest today: `examples/03-plugin/plugin` is a working directory that
also packs unchanged.

The rest of the manifest grammar — the dep tables, `dev-deps`,
`style`, and the strict-keys law — is defined in [Project structure
and rut.jsonc](project-structure.md); the dependency semantics are
[Dependency kinds](dependency-kinds.md).

## Url deps — bundles by reference

A consumer may mount a bundle by url instead of vendoring it:

```jsonc
{
  "deps": {
    "server": { "url": "https://example.com/server.rutbundle" }
  }
}
```

The pin lives beside the manifest, in the walk root's `rut.lock`
(`rut install` writes it — [the rut CLI](cli.md)): the manifest row
names the url, the lock pins its bytes.

The mount is the bundle mount, one gate earlier: the embedder's
remote produces the bytes (`DepRemote` — the standard
`rut_native::HttpRemote` is cache-first), then the loader runs the same container/layout/name checks and —
before all of them — the **pin law**: the url must have a lock row
(an unpinned url refuses, naming `rut install`), and the bytes must
hash to the lock row's `sha256`, on every load, or the load refuses
naming the dep, the url, and both hashes. The url dep mounts as a
**leaf**: its bundle brings the whole closure (every declared dep
satisfied by an in-archive group — the closure law is unchanged, and a
url row inside a packed bundle is inert metadata; loading a bundle
never fetches).

Because the manifest rides byte-for-byte, url rows carry into a
consumer's own `rut pack` output, satisfied by the rode-along groups:
the archive's groups re-encode from the consumer session's units (one
`.rutc` path) or copy their file sets under the new identity prefixes,
and the same manifest + lock still pack byte-identically. The pack
refuses, loudly and named, the cases that would emit a bundle the
loader must reject: an archive source group still declaring `dev-deps`
directories (vendor the dep), a compiled group the consumer's closure
never uses (it cannot ride unpackaged), and a declared dep that did
not ride.

## The wire version

`format_version` is 11 — the one bundle wire, both root kinds; the
manifest's `type` routes lib-compiled vs host-decl. A loader refuses
any other value **before reading anything else** — refuse, never
guess: `this toolchain reads bundle format_version 11 only (found {v})
— re-pack the directory`.

Every file in the two layouts is part of the package: a bundle that
dropped a declared child module would load partial — semantically
wrong, so the closure law is checked at the mount. Compilation is the
delivery contract, no exceptions — the generic bodies ride INSIDE the
binary as templates (the delivery law above), so consumer-spelled
shapes stay servable and a published bundle never carries source.
Source sharing as the general contract stays a directory
(`rut run <dir>`), as it always was outside bundles.

**The groups mirror the manifest.** The archive root is the root's own
files; every dep rides at `deps/<name>@<fp16>/` — the dirname IS the
content identity, fingerprinted over the group's own rode source set,
and the reader recomputes the fingerprint and refuses a mismatch (a
doctored or stale dirname). Same source ⇒ same dirname, so a repack of
an unchanged group lands at the same directory and the two identities
of one name coexist side by side (the packer exact-dedups one
identity's file set — two groups stage to one identity only when their
bytes are equal).

**The ledger namespaces per archive.** Each archive's scope rows shift
into a fresh numeric range of the consuming run's world (boot passes
through), and its binaries rebase with that same map **before**
mounting — so two independently packed bundles can share one world
without their pack-time numberings ever arguing. First-mount-wins
still decides which body a shared group name keeps, and a group's
ledger row must still be the scope its own binary carries (refuse,
never guess — that check is per archive). The CDN lane below is the
reason this exists: per-package bundles are packed independently, each
numbering its own closure.

## The `bundle` module

The container codec, the `rut.jsonc` grammar, the source file-set
collector, and the reader (both root kinds) live in the driver's
`rut_driver::bundle` module — **pure and in-memory**: the walk adds
every filesystem read. That half lives in `rut-native`, whose
one-method `Source` trait is the one filesystem door:
`rut_native::FsSource::at(dir)` is the real filesystem (the CLI,
native hosts); an in-memory path→bytes map serves tests and wasm
hosts. The packer needs the walk and lives beside it
(`rut_native::pack_dir`) — it returns the bundle bytes; writing the
output file stays with the caller. Loading a bundle into a run goes
through `rut_native::load_bundle_session(path)` (the file read) or
`rut_driver::Pkg::from_bundle(&bytes)` (the pure parse) over the
parsed `rut_driver::bundle::Bundle` and its `rut_driver::bundle::Layout`
— the compiled root and its ledger + groups, or the decl root and its
surface.

## Deterministic packing

Same directory + same toolchain ⇒ byte-identical `.rutbundle`:

- fixed entry order — `rut.jsonc` first, then the scope ledger, the
  root's binary, then the root's surface/source/rows, then dep groups
  in identity order (one group per distinct identity);
- fixed (zeroed) timestamps;
- STORE (no compression) — determinism over size.

This is the module binary's determinism law extended one level up
([Module binary and verification](module-binary.md)): bundles are
content-cacheable, and two builds of the same module diff to nothing.

## Stripping — the private sidecar

`rut pack <dir> --strip` emits the bundle as above **plus a private
symbol table** at the sibling path `<out-without-ext>.rutsym`. Every
renameable name in the `.rutc` binaries mangles to `%N` and every
function's span/position tables leave — the sidecar carries exactly
what left, so supplying it at load restores real names and line/col
for stack-trace symbolication. The sidecar is never an entry inside
the archive. It needs a fully-compiled closure (a source group that
could bind a compiled group's surface is a pointed refusal); without
`--strip`, packing is unchanged. The whole law lives in [Symbol
stripping and `.rutsym` sidecars](symbol-stripping.md). `run` accepts
`--symbols <file.rutsym>` against a compiled bundle
([The rut CLI](cli.md)).

## Checks on load

Every check runs at load time, before compilation and linking — a bad
bundle never executes. The order is the law: container, then manifest
and version, then every group's decode, then the mount.

| check | rule on mismatch |
|---|---|
| zip entry CRC-32 | refuse — corrupt bundle (names the entry) |
| `rut.jsonc` present, `"format": "rutbundle"` | refuse — not a rut bundle |
| `format_version` exactly 11 | refuse — `this toolchain reads bundle format_version 11 only (found {v}) — re-pack the directory` |
| a host root carrying a root `mod.rutc` | refuse — a host bundle's root is its surface, not a compiled unit |
| a host root carrying a `rut.scopes` ledger or any group entry | refuse — a host bundle is its surface, single-package |
| a compiled root without its `rut.scopes` ledger, or a ledger row that is not a content identity (`name@<16 hex>`) | refuse — a corrupt compiled bundle |
| an entry under a top-level prefix other than `deps/` | refuse — the archive root carries only the root's own files; dep groups ride at `deps/<name>@<fp16>/` |
| a `deps/` group directory whose name is not a content identity | refuse — expected `<name>@<16 hex>` |
| a group's manifest `name` disagreeing with its dirname's name half | refuse — names both |
| a group dirname's fingerprint disagreeing with the hash of its rode source set | refuse — the dirname and the content disagree |
| `mod.rutc` decode (root and groups): version, tables, surface ids | refuse — names the entry and the cause |
| a compiled group's ledger row agrees with its binary's own scope | refuse — corrupt or doctored |
| a riding source entry that fails to read or decode (UTF-8) | refuse — names the entry; a bad archive never reaches the session |
| every declared dep satisfied by a group (compiled roots) | refuse — names the missing group |

## The std tree on a CDN — the per-package delivery

The std tree (`rut/`) ships as **committed** per-package bundles at
`dist/std/<pkg>.rutbundle` — one bundle per package a `deps` row can
spell (16 today). jsDelivr's gh lane serves repo files at a ref, so the
artifacts ARE the delivery and the publish act is a git tag:

```jsonc
{
  "deps": {
    "http": {
      "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@<tag>/dist/std/http.rutbundle"
    }
  }
}
```

`<tag>` is a placeholder — real urls live in the worked manifests, and
the real pins live in each project's committed `rut.lock` (the
in-repo examples' locks are the reference; anti-rot: docs never rot
pins). The laws:

- **Committed artifacts, immutable tags.** `scripts/pack-std::cjs`
  packs all 16 (deterministic — same input ⇒ byte-identical, so
  freshness is a pure equality gate: `--check` repacks and
  byte-compiles, no network; the retired `--pins` flag is gone — the
  pins live in `rut.lock` now, written by `rut install`, and no
  manifest tool rewrites them anymore). Tags advance
  (`std-vNN`) and are never re-pointed — jsDelivr caches aggressively.
- **What a compiled bundle serves.** Instantiation is owner-anchored:
  a compiled owner carries the generic instantiations its own pack
  closure spelled in its ledger — and its generic BODIES as templates
  inside the binary (the delivery law): at the consumer's link a
  request the ledger lacks monomorphizes from the template under the
  declaring pkg's spec (one row program-wide, nothing persisted). So
  the CDN lane delivers **host surfaces** (decl bundles),
  **concrete-class libs** (`http`, `ink`, `strbuild` — methods cross
  on the surface's inherent rows), **and the generic owners**
  (`pouch`, `flow`, `nmapset`, `json`, `futures` — `Vec<Todo>`,
  `Flow<i32>`, `decodeJson<T>` compile at the link from the
  templates). The law binds this lane too: a pkg that owns an open
  generic surface but whose binary carries no template bodies refuses
  at the mount (corrupt or doctored — re-pack the directory). And
  every bundle is **closed** —
  a bundle-mounted json carries its `strbuild` + `pouch` + `nmapset`
  groups inside, so its wrappers' `use` heads resolve wherever it
  travels (the ordinary closure law).
- **Duplicate mounts are safe.** Two archives may carry the same group
  (`json`'s and `http`'s closures both carry `strbuild`): the same
  identity mounts once (exact dedup), two identities of one name
  coexist side by side, and the per-archive ledger namespacing keeps
  every binary's ids resolving through its own archive's rows. A bare
  `use` head under several identities of one name is the loud
  ambiguity ([Dependency kinds](dependency-kinds.md)) — pin one of
  them with your own `deps` row.

## Loading

A bundle is one more resolution source alongside the directory form —
same name resolution, same compile, same identity-keyed groups:

```rust
// from a file
let loaded = rut_native::load_bundle_session(Path::new("vendor/plugin.rutbundle"))?;
// from bytes (embedders, tests, wasm) — the pure container parse
let loaded = rut_driver::Pkg::from_bundle(&bytes)?;
```

After the offer, the package name resolves exactly as the directory form
does — with one difference: a compiled group's body is the decoded
program itself. The run assigns it a fresh scope, rebases its packed
ids through the ledger, and pushes it; source groups offer as sources
and compile exactly as a directory world. Then the normal
pipeline takes over ([Loading and the embed loop](loading.md)). Loose
directories and loose `.rut` files stay valid share layouts; the bundle
packages them, nothing requires it.

## CLI

```sh
rut pack <dir> [-o <dir>.rutbundle] [--strip]   # default output: a sibling of the dir
rut run <dir | mod.rutbundle> [--symbols <file.rutsym>]
```

The packed form and its directory compile to identical programs — pinned
by tests (the linked binaries are equal).
