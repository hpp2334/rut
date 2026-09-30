# Module bundles

`rut pack mod/` reads a module directory and emits **one file** —
`mod.rutbundle` — a zip archive carrying the module **compiled**: the
root's `.rutc` binary, its scope ledger, and each dependency as a
compiled `.rutc` group or a source file set. A bundle is the same
contract as the directory it was packed from, in one file: the loader
mounts it exactly like the directory, and the two compile to identical
programs.

## Layout rules

- **Entry names are paths relative to the module root** — `rut.toml`,
  `<pkg>.rutc`, `<pkg>.d.rut`, `<dep>/…` group entries for deps. No
  directories otherwise, no metadata entries. Unknown extra entries are
  ignored by loaders (forward compatibility).
- The manifest's `name` is the package name the bundle answers to — a
  bare `[a-zA-Z0-9_]+` name, the same law as directory manifests.
- Includes and splices never escape the archive: there is no outside.

## The v5 layout

```text
rut.toml                 byte-for-byte; format = "rutbundle", format_version = 5
rut.scopes               the pack-time scope ledger (scope = "spec" rows)
<pkg>.rutc               the root's compiled binary — bodies + surface
<pkg>.d.rut              the root's surface text (humans, LSP), when it declares one
<dep>/rut.toml           per dep group, mixed kinds:
<dep>/<dep>.rutc           linkable dep → a compiled group (its rut.toml rides too);
                           generic exports ride compiled — the binary carries
                           the closure's instantiations in its ledger
<dep>/<dep>.d.rut          its surface text, when it declares one
<leaf>/rut.toml            splice-needed dep (the `inline` flag) or a host
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
- **Generics ride compiled**: instantiation is owned by the declaring
  package, and the pack walk seeds each generic dep with the
  instantiations its consumers spell — the binary's instantiation
  ledger names every `(owner, decl, arguments)` row, and a consumer
  session resolves its requests against it. A request the binary does
  not carry refuses (`re-pack with the consumer in the closure`), never
  mislinks.
- **The root must be linkable** — else `pack: <pkg> is inline — its
  source is its interface and it cannot be published compiled; share
  the directory instead`.

## The manifest — `rut.toml`

First entry in the zip; TOML; the same subset the directory form uses,
byte-for-byte the directory's manifest — which is what makes a
bundle-shaped directory pack unchanged:

```toml
format = "rutbundle"
format_version = 5          # the bundle LAYOUT version — independent of
                            #   the module-binary version
name = "plugin"             # the package this bundle answers to
entry.lib = "./plugin.rut"  # directory-time; the compiled form rides
                            #   plugin.rutc instead

[deps]                      # the whole dep closure rides the archive
server = { path = "../server" }
```

Directory loading **ignores** `format`/`format_version` — a directory is
not a bundle — which is why the keys are safe to write into every module
manifest today: `examples/03-plugin/plugin` is a working directory that
also packs unchanged.

The rest of the manifest grammar — deps tables, `inline`,
`[style]` — is defined in [Project structure and
rut.toml](project-structure.md); the dependency semantics are
[Dependency kinds](dependency-kinds.md).

## The layout ledger

`format_version` versions the zip layout and manifest keys themselves.
Loaders refuse a version they do not know **before reading anything
else** — refuse, never guess; the same gate makes an older loader refuse
a newer bundle.

| version | layout | status today |
|---|---|---|
| 1 | one module, sources only | refused — re-pack the directory |
| 2 | + the whole `[deps]` graph as `<pkg>/` groups | refused — re-pack the directory |
| 3 | + each package's `[peer-deps]` `lib` group files | refused — re-pack the directory |
| 4 | + each package's `entry.libs` files | refused — re-pack the directory |
| 5 | compiled: `.rutc` (v17) + `.d.rut` per linkable pkg — generic exports included, their instantiations seeded into the binaries — mixed source groups for `inline` deps and host pkgs | **the packer's output** — this toolchain reads 5 only |

Each historical extension existed because the added files were *part of
the package*: a bundle that dropped peer groups or multi-lib files would
load base-only — semantically wrong. v5 replaced the source contract
with the compiled one: source sharing is a directory (`rut run <dir>`),
as it always was outside bundles.

A compiled group that declares `[peer-deps]` `lib` files is refused at
pack (and a hand-doctored archive at load): appending source into a
compiled package is impossible. Peer-gated packages publish inside v5
the splice-needed way — as source groups, their group files riding
beside the entry — and the loader's peer gate appends by presence
exactly as in a directory world ([Dependency
kinds](dependency-kinds.md)).

## The `rut-bundle` crate

The container codec, the `rut.toml` grammar, the source file-set
collector, and the v5 reader live in the `rut-bundle` crate — std-only
and **filesystem-free**. Every read goes through a one-method `Source`
trait: `rut_bundle::FsSource` is the real filesystem (the CLI, native
hosts); an in-memory path→bytes map serves tests and wasm hosts. The
packer itself needs the compiler and lives in `rut-driver`
(`rut_driver::pack_dir`) — it returns the bundle bytes; writing the
output file stays with the caller. Mounting a bundle into a session
stays in `rut-driver` ([Loading](loading.md)), over
`rut_bundle::Bundle` and its parsed `rut_bundle::Layout` — the
compiled root, the ledger, and each group's kind.

## Deterministic packing

Same directory + same toolchain ⇒ byte-identical `.rutbundle`:

- fixed entry order — `rut.toml` first, then the scope ledger, the
  root's binary, then dep groups in name order (deduplicated,
  recursive);
- fixed (zeroed) timestamps;
- STORE (no compression) — determinism over size.

This is the module binary's determinism law extended one level up
([Module binary and verification](module-binary.md)): bundles are
content-cacheable, and two builds of the same module diff to nothing.

## Checks on load

Every check runs at load time, before compilation and linking — a bad
bundle never executes. The order is the law: container, then manifest
and version, then every group's decode, then the mount.

| check | rule on mismatch |
|---|---|
| zip entry CRC-32 | refuse — corrupt bundle (names the entry) |
| `rut.toml` present, `format = "rutbundle"` | refuse — not a rut bundle |
| `format_version` exactly 5 | refuse — "reads bundle format_version 5 only" |
| `rut.scopes` present, every row a bare package name | refuse — not a v5 compiled bundle |
| `<pkg>.rutc` decode: version, tables, surface ids | refuse — names the entry and the cause |
| a compiled group's ledger row agrees with its binary's own scope | refuse — corrupt or doctored |
| a compiled group declaring `[peer-deps]` `lib` files | refuse — appending into a compiled pkg is impossible |
| every declared dep satisfied by a group | refuse — names the missing group |

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
rut pack <dir> [-o <dir>.rutbundle]   # default output: a sibling of the dir
rut run <dir | mod.rutbundle>         # mount, compile, execute main
```

The packed form and its directory compile to identical programs — pinned
by tests (the linked binaries are equal).
