# Module bundles

`rut pack mod/` reads a module directory and emits **one file** —
`mod.rutbundle` — a zip archive carrying the manifest and the module's
sources. A bundle is the same contract as the directory it was packed
from, in one file: the loader mounts it exactly like the directory, and
the two compile to identical programs.

## Layout rules

- **Entry names are source paths relative to the module root**, flat —
  `rut.toml`, `entry.rut`, `util.rut`, `<pkg>/…` group entries for deps.
  No directories otherwise, no metadata entries. Unknown extra entries
  are ignored by loaders (forward compatibility).
- The manifest's `name` is the package name the bundle answers to — a
  bare `[a-zA-Z0-9_]+` name, the same law as directory manifests.
- Includes and splices never escape the archive: there is no outside.

## The manifest — `rut.toml`

First entry in the zip; TOML; the same subset the directory form uses,
which is what makes a bundle-shaped directory pack unchanged:

```toml
format = "rutbundle"
format_version = 4          # the bundle LAYOUT version — independent of
                            #   the module-binary version
name = "plugin"             # the package this bundle answers to
entry.lib = "./plugin.rut"  # the entry source, relative to the root
entry.libs = ["./store.rut"]  # optional: extra body files (multi-lib)

[deps]                      # the whole dep graph rides the archive
server = { path = "../server" }
```

Directory loading **ignores** `format`/`format_version` — a directory is
not a bundle — which is why the keys are safe to write into every module
manifest today: `examples/03-plugin/plugin` is a working directory that
also packs unchanged.

The rest of the manifest grammar — deps tables, `host_scope`, `inline`,
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
| 1 | one module, sources only | loads (the one-module special case) |
| 2 | + the whole `[deps]` graph as `<pkg>/` groups (manifest + entry each, recursively, deduplicated) | loads; no longer packs |
| 3 | + each package's `[peer-deps]` `lib` group files beside its entry | loads; no longer packs |
| 4 | + each package's `entry.libs` files beside its entry | **the packer's output** |

Each extension exists because the added files are *part of the package*:
a bundle that dropped peer groups or multi-lib files would load
base-only — semantically wrong. Groups resolve **by name** (a dep's
`path` key is directory-time metadata only), and a v4 load splices
`entry.libs` exactly as the directory loader does: base first, then the
list in manifest order — the array *is* the splice order
([Dependency kinds](dependency-kinds.md)).

A manifest that declares `[peer-deps]` or `entry.libs` under a layout
older than the one that introduced them is refused — those layouts have
no group entries and would silently mount base-only.

## Deterministic packing

Same directory + same toolchain ⇒ byte-identical `.rutbundle`:

- fixed entry order — `rut.toml` first, then the entry source and every
  transitive relative include in include order, then dep groups
  (name order, deduplicated, recursive);
- fixed (zeroed) timestamps;
- STORE (no compression) — determinism over size.

This is the module binary's determinism law extended one level up
([Module binary and verification](module-binary.md)): bundles are
content-cacheable, and two builds of the same module diff to nothing.

## Checks on load

Every check runs at load time, before compilation and linking — a bad
bundle never executes:

| check | rule on mismatch |
|---|---|
| `rut.toml` present, `format = "rutbundle"` | refuse — not a rut bundle |
| `format_version` known | refuse — unknown bundle layout |
| zip entry CRC-32 | refuse — corrupt bundle (names the entry) |
| entry name / UTF-8 / STORE method | refuse — corrupt or unsupported |
| `entry.lib` present; includes + declared groups/libs resolvable in the zip | refuse — load error naming the entry |
| a declared peer group missing from the archive | refuse — refuse, never guess |

## Loading

A bundle is one more resolution source alongside the directory form —
same name resolution, same compile, same splice rules:

```rust
// from a file
let (session, root) = rut_driver::load_path_session(Path::new("vendor/plugin.rutbundle"))?;
// from bytes (embedders, tests, wasm)
let (session, root) = rut_driver::load_bundle_bytes(&bytes, Path::new("mem"))?;
```

After mounting, the package name resolves through the bundled sources
exactly as the directory form does: the entry source is expanded
(relative includes inlined, include-once), group and libs files splice
in their manifest order, and the unit compiles under the manifest's
`name` — then the normal pipeline takes over ([Loading and the embed
loop](loading.md)). Loose directories and loose `.rut` files stay valid
publish layouts; the bundle packages them, nothing requires it.

## CLI

```sh
rut pack <dir> [-o <dir>.rutbundle]   # default output: a sibling of the dir
rut run <dir | mod.rutbundle>         # mount, compile, execute main
```

The packed form and its directory compile to identical programs — pinned
by tests (the linked binaries are equal).

## What a bundle is not

A bundle is a **source** contract: it carries `rut.toml` + `.rut`
sources and compiles on load. A compiled-artifact payload (`.rutc`
binaries plus declaration surfaces inside the zip) is not part of any
layout version yet — when it lands it will be a new ledger row with its
own consistency rules, and the source layouts above remain unchanged.
