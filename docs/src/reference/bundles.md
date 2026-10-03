# Module bundles

`rut pack mod/` reads a module directory and emits **one file** —
`mod.rutbundle` — a zip archive carrying the module in one of two root
kinds, paired with the format version: a **lib** pkg packs **compiled**
(v9 — the root's `.rutc` binary, its scope ledger, and each dependency
as a compiled `.rutc` group or a source file set), and a `type = "host"`
pkg packs as a **decl** root (v10 — its `.d.rut` surface rides as
source; single-package). A bundle is the same
contract as the directory it was packed from, in one file: the loader
mounts it exactly like the directory, and the two compile to identical
programs.

## Layout rules

- **Entry names are paths relative to the module root** — `rut.jsonc`,
  `<pkg>.rutc`, `<pkg>.d.rut`, `<dep>/…` group entries for deps. No
  directories otherwise, no metadata entries. Unknown extra entries are
  ignored by loaders (forward compatibility) — except in a v10 decl
  bundle, where a group entry, a root `.rutc`, or a ledger is a
  contradiction and refuses (see [the v10 layout](#the-v10-layout)).
- The manifest's `name` is the package name the bundle answers to — a
  bare `[a-zA-Z0-9_]+` name, the same law as directory manifests.
- Includes and splices never escape the archive: there is no outside.

## The v9 layout — a compiled root

```text
rut.jsonc                byte-for-byte; "format": "rutbundle", "format_version": 9
rut.scopes               the pack-time scope ledger (scope = "spec" rows)
<pkg>.rutc               the root's compiled binary — bodies + surface
<pkg>.rut                the root's generic-bearing source, when its surface
                           exports generics (the riding law, below)
<pkg>.d.rut              the root's surface text (humans, LSP), when it declares one
<dep>/rut.jsonc          per dep group, mixed kinds:
<dep>/<dep>.rutc           linkable dep → a compiled group (its rut.jsonc rides too);
                           generic exports ride compiled — the binary carries
                           the closure's instantiations in its ledger
<dep>/<dep>.rut            the group's generic-bearing source, when its surface
<dep>/group-*.rut …        exports generics (the entry + the peer-deps group
                           files, verbatim — the riding law, below)
<dep>/<dep>.d.rut          its surface text, when it declares one
<leaf>/rut.jsonc           splice-needed dep (the `inline` flag) or a host
<leaf>/<leaf>.rut           pkg → a source group: the source file set
<leaf>/<leaf>.rut …         (entry, libs, peer groups)
```

- The **`.rutc` binaries are the linking truth**: bodies plus the
  surface on the wire ([module binary and
  verification](module-binary.md)). A consumer binds a compiled group's
  exports from the binary alone — nothing is rebuilt from `.d.rut`
  text. The `.d.rut` files ride for humans and tooling; the loader
  never reads them.
- The **scope ledger** names the pack-time scope of every linked module
  in the closure (engine mounts included). A decoded program's ids
  spell its pack-time scopes; the loader resolves each through the
  ledger to the module to ensure and rebases the ids onto its own
  numbering — pack-time and load-time numbering never have to agree. A
  reference with no ledger row is refused.
- **Mixed groups, one classifier**: the same splice law that rules the
  graph (the `inline` flag — [modules and
  visibility](modules-and-visibility.md)) decides each package's kind.
  Refuse, never guess: the packer calls the graph's own classifier,
  there is no second implementation.
- **Generics ride compiled — and their source rides beside them** (the
  generic-source riding law): instantiation is owned by the declaring
  package, and the pack walk seeds each generic dep with the
  instantiations its consumers spell — the binary's instantiation
  ledger names every `(owner, decl, arguments)` row. A compiled pkg
  whose surface exports an OPEN generic surface (generic fns, generic
  type exports, generic methods, generic-target impls) ALSO rides the
  source that serves consumer-spelled shapes: the entry lib, each
  `entry.libs` file, and each `peer-deps` group file, verbatim,
  beside the binary. The entry lib's presence is the loader's dispatch
  marker; non-generic pkgs (`http`, `ink`, `strbuild`) stay
  source-free. At the consumer's link, a request the ledger lacks
  lowers the ridden text in the consumer's session and compiles the
  monomorphized body with owner = the pkg's spec — one row
  program-wide, indistinguishable from a pack-time one, nothing
  persisted. A bundle with no riding source refuses such a request
  loudly (`this bundle predates generic-source riding — re-pack it`),
  never mislinks. `--strip` refuses the combination: the ridden text
  would recompile clean-named beside mangled binaries.
- **The root must be linkable** — else `pack: <pkg> is inline — its
  source is its interface and it cannot be published compiled; share
  the directory instead`. A root that has nothing to compile is not
  this refusal: a `type = "host"` pkg packs as a v10 decl root, below.

## The v10 layout — a host root

A `type = "host"` pkg's root IS its declaration surface, so a host
bundle is the manifest plus that surface, riding as source —
single-package, no ledger, no groups, nothing to decode:

```text
rut.jsonc                byte-for-byte; "format": "rutbundle", "format_version": 10
<pkg>.d.rut              the declaration surface — the root itself
```

The pairing is **total**, both directions refused at the version gate:

- **v9 ⇔ compiled.** A host manifest at v9 refuses — a v9 bundle's
  root must be a `.rutc` a host pkg cannot have.
- **v10 ⇔ decl.** A lib manifest at v10 refuses — lib roots stay v9,
  compiled.
- A v10 bundle carrying a root `.rutc` (`a host bundle's root is its
  surface, not a compiled unit`), a `rut.scopes` ledger, or any group
  entry (`a host bundle is single-package`) refuses: contradictions,
  never guesses.
- Writers emit 10 **only** for decl roots — every lib bundle stays
  byte-identical v9, and old readers refuse a v10 bundle loudly at the
  version gate.

`rut pack <host-dir>` works (the surface verifies by parsing and
lowering once — the exact lane a mount runs — then rides as source),
`--strip` refuses on a host root (`no symbols to strip` — no programs,
no sidecar), and `rut run <host.rutbundle>` refuses with intent: a
host bundle carries a declaration surface — nothing to run; bind its
rows from the embedder ([host fns](host-fns.md)). The mount is the
same lane a host group rides inside a v9 bundle: the surface lowers
into the pkg's host rows.

## The manifest — `rut.jsonc`

First entry in the zip; JSONC (the directory grammar: `//` and `/* */`
comments and trailing commas are legal); the same manifest grammar the
directory form uses, byte-for-byte the directory's manifest — which is
what makes a bundle-shaped directory pack unchanged:

```jsonc
{
  // the bundle LAYOUT version — independent of the module-binary
  // version; a host root declares 10 instead (see the v10 layout)
  "format": "rutbundle",
  "format_version": 9,
  // the package this bundle answers to
  "name": "plugin",
  // directory-time; the compiled form rides plugin.rutc instead
  "entry": { "lib": "./plugin.rut" },

  // the whole dep closure rides the archive
  "deps": {
    "server": { "path": "../server" }
  }
}
```

Directory loading **ignores** `format`/`format_version` — a directory is
not a bundle — which is why the keys are safe to write into every module
manifest today: `examples/03-plugin/plugin` is a working directory that
also packs unchanged.

The rest of the manifest grammar — deps tables, `inline`,
`style` — is defined in [Project structure and
rut.jsonc](project-structure.md); the dependency semantics are
[Dependency kinds](dependency-kinds.md).

## Url deps — bundles by reference

A consumer may pin a bundle by url instead of vendoring it:

```jsonc
{
  "deps": {
    "server": { "url": "https://example.com/server.rutbundle", "sha256": "<64-hex>" }
  }
}
```

The mount is the bundle mount, one gate earlier: the embedder's
remote produces the bytes (`DepRemote` — the standard `HttpRemote` is
cache-first), then the loader runs the same container/layout/name checks and —
before all of them — the **pin law**: the bytes must hash to the
manifest row's `sha256`, on every load, or the load refuses naming the
dep, the url, and both hashes. The url dep mounts as a **leaf**: its
bundle brings the whole closure (every declared dep satisfied by an
in-archive group — the closure law is unchanged, and a url row inside
a packed bundle is inert metadata; loading a bundle never fetches).

Because the manifest rides byte-for-byte, url rows carry into a
consumer's own `rut pack` output, satisfied by the rode-along groups:
the archive's groups re-encode from the consumer session's units (one
`.rutc` path), source groups copy their file set under the new prefix,
and the same manifest + pins still pack byte-identically. v1 refuses,
loudly and named, the cases that would emit a bundle the loader must
reject: an archive source group still declaring `dev-deps`
directories (vendor the dep), a compiled group the consumer's closure
never uses (it cannot ride unpackaged), and a declared dep that did
not ride.

## The layout ledger

`format_version` versions the zip layout and manifest keys themselves.
Loaders refuse a version they do not know **before reading anything
else** — refuse, never guess; the same gate makes an older loader refuse
a newer bundle.

| version | layout | status today |
|---|---|---|
| 1 | one module, sources only | refused — re-pack the directory |
| 2 | + the whole `deps` graph as `<pkg>/` groups | refused — re-pack the directory |
| 3 | + each package's `peer-deps` `lib` group files | refused — re-pack the directory |
| 4 | + each package's `entry.libs` files | refused — re-pack the directory |
| 5 | compiled: `.rutc` (v17) + `.d.rut` per linkable pkg — the manifest TOML | refused — re-pack the directory (the pre-JSON shape) |
| 6 | decl: a `type = "host"` root — the manifest TOML | refused — re-pack the directory (the pre-JSON shape) |
| 7 | compiled, the manifest JSON (`rut.json`): `.rutc` (v17) + `.d.rut` per linkable pkg — generic exports included, their instantiations seeded into the binaries, and a generic-owning pkg's source riding beside its binary — mixed source groups for `inline` deps and host pkgs | refused — re-pack the directory (the pre-JSONC wire) |
| 8 | decl, the manifest JSON (`rut.json`): a `type = "host"` root — the manifest + its `.d.rut` surface, single-package | refused — re-pack the directory (the pre-JSONC wire) |
| 9 | compiled, the manifest JSONC (`rut.jsonc`): the v7 layout under the JSONC cutover — the manifest file renamed, comments + trailing commas legal | **the lib root's layout** |
| 10 | decl, the manifest JSONC (`rut.jsonc`): a `type = "host"` root — the manifest + its `.d.rut` surface, single-package | **the host root's layout** |

The 8→9 bump rides the manifest's own RENAME (`rut.json` →
`rut.jsonc`): the entry name is layout, so the number moves with it —
an old reader must never silently misparse a file whose name it does
not know. Each historical extension existed because the added files
were *part of
the package*: a bundle that dropped peer groups or multi-lib files would
load base-only — semantically wrong. v7 replaced the source contract
with the compiled one — one exception, the riding law above: a
generic-owning compiled pkg's source rides beside its binary so
consumer-spelled shapes stay servable. Source sharing as the general
contract stays a directory (`rut run <dir>`), as it always was outside
bundles.

Peer-gated packages publish inside v9 two ways, per their group-kind
law: a NON-generic compiled declarer's peer groups compile into its
binary at pack time (the rows ride the binary; the group files do not
travel), and a splice-needed declarer publishes as a source group, its
group files riding beside the entry — the loader's peer gate appends by
presence exactly as in a directory world ([Dependency
kinds](dependency-kinds.md)). A generic-owning compiled declarer rides
its group files too (the riding law), and the on-demand recompile
splices exactly the rows whose peers are in the consumer's closure.

**The ledger namespaces per archive.** Each archive's scope rows shift
into a fresh numeric range of the mounting session (boot passes
through), and its binaries rebase with that same map **before**
mounting — so two independently packed bundles can share one session
without their pack-time numberings ever arguing. First-mount-wins
still decides which body a shared group name keeps, and a group's
ledger row must still be the scope its own binary carries (refuse,
never guess — that check is per archive). The CDN lane below is the
reason this exists: per-package bundles are packed independently, each
numbering its own closure.

## The `bundle` module

The container codec, the `rut.jsonc` grammar, the source file-set
collector, and the reader (both root kinds) live in the driver's
`rut_driver::bundle` module — **filesystem-free**. Every read goes
through a one-method `Source` trait: `rut_driver::bundle::FsSource` is
the real filesystem (the CLI, native hosts); an in-memory path→bytes
map serves tests and wasm hosts. The packer itself needs the compiler
and lives beside it (`rut_driver::pack_dir`) — it returns the bundle
bytes; writing the output file stays with the caller. Mounting a bundle
into a session stays in the loader ([Loading](loading.md)), over
`rut_driver::bundle::Bundle` and its parsed `rut_driver::bundle::Layout`
— the compiled root and its ledger + groups, or the decl root and its
surface.

## Deterministic packing

Same directory + same toolchain ⇒ byte-identical `.rutbundle`:

- fixed entry order — `rut.jsonc` first, then the scope ledger, the
  root's binary, then dep groups in name order (deduplicated,
  recursive);
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
| `rut.jsonc` present, `"format": "rutbundle"` | refuse — not a rut bundle; a `rut.json` entry instead → "predates wire 9 — re-pack the directory" |
| `format_version` exactly 9 or 10 | refuse — "reads bundle format_version 9 (compiled) and 10 (decl) only"; ≤8 adds "the pre-JSONC wire is retired" |
| v9 with a `type = "host"` root / v10 with a lib root | refuse — the pairing is total |
| v10 carrying a root `.rutc`, a `rut.scopes` ledger, or a group entry | refuse — a host bundle is its surface, single-package |
| `rut.scopes` present, every row a bare package name (v9) | refuse — not a v9 compiled bundle |
| `<pkg>.rutc` decode: version, tables, surface ids | refuse — names the entry and the cause |
| a compiled group's ledger row agrees with its binary's own scope | refuse — corrupt or doctored |
| a riding source entry that fails to read or decode (UTF-8) | refuse — names the entry; a bad archive never reaches the session |
| every declared dep satisfied by a group (v9) | refuse — names the missing group |

## The std tree on a CDN — the per-package delivery

The std tree (`rut/`) ships as **committed** per-package bundles at
`dist/std/<pkg>.rutbundle` — one bundle per package a `deps` row can
spell (15 today). jsDelivr's gh lane serves repo files at a ref, so the
artifacts ARE the delivery and the publish act is a git tag:

```jsonc
{
  "deps": {
    "http": {
      "url": "https://cdn.jsdelivr.net/gh/hpp2334/rut@<tag>/dist/std/http.rutbundle",
      "sha256": "<64-hex>"
    }
  }
}
```

`<tag>`/`<sha256>` are placeholders — real pins live in the examples
(anti-rot: docs never rot pins). The laws:

- **Committed artifacts, immutable tags.** `scripts/pack-std.cjs`
  packs all 15 (deterministic — same input ⇒ byte-identical, so
  freshness is a pure equality gate: `--check` repacks and
  byte-compiles, no network); `--pins` rewrites the example manifests'
  url/sha256 rows; `--tag` prints the publish commands. Tags advance
  (`std-vNN`) and are never re-pointed — jsDelivr caches aggressively.
- **What a compiled bundle serves.** Instantiation is owner-anchored:
  a compiled owner carries the generic instantiations its own pack
  closure spelled in its ledger — and, the generic-source riding law,
  a compiled pkg whose surface exports generics also rides the source
  that serves consumer-spelled shapes: at the consumer's link a
  request the ledger lacks lowers the ridden text in the consumer's
  session and compiles the monomorphized body under the declaring
  pkg's spec (one row program-wide, nothing persisted). So the CDN
  lane delivers **host surfaces** (v10 decl bundles), **concrete-class
  libs** (`http`, `ink`, `strbuild` — methods cross on the surface's
  inherent rows), **and the generic owners** (`pouch`, `nmapset`,
  `json`, `futures` — `Vec<Todo>`, `Map<K,V>`, `decodeJson<T>`
  compile at the link from the ridden source). A bundle that predates
  the riding refuses a consumer-spelled shape loudly and says so
  (re-pack it), and a bundle-mounted json names its pack-time dev
  closure in its ledger, so the consumer's closure must contain those
  names (`pouch`, `nmapset` beside `json` — the six-pin law).
- **Duplicate mounts are safe.** Two archives may carry the same group
  (`json`'s and `http`'s closures both carry `strbuild`):
  first-mount-wins mounts one body, and the per-archive ledger
  namespacing keeps every binary's ids resolving through its own
  archive's rows. Pin sets stay minimal-top anyway — a closed bundle
  carries the rest.

## Loading

A bundle is one more resolution source alongside the directory form —
same name resolution, same compile, same splice rules:

```rust
// from a file
let (session, root) = rut_driver::load_path_session(Path::new("vendor/plugin.rutbundle"))?;
// from bytes (embedders, tests, wasm)
let (session, root) = rut_driver::load_bundle_bytes(&bytes, Path::new("mem"))?;
```

After mounting, the package name resolves exactly as the directory form
does — with one difference: a compiled group's body is the decoded
program itself. The graph assigns it a fresh scope, rebases its packed
ids through the ledger, and pushes it; source groups mount as sources
and compile (or splice) exactly as a directory world. Then the normal
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
