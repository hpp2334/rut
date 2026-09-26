# RFC 0038: Module Bundles — `.rutbundle`

- **Status:** Draft
- **Date:** 2026-08-23
- **Revised:** 2026-09-14 — v1 payload is `rut.toml` + rut **sources**
  (compiled on load); the `.rutc`/`.d.ir`/map payload moves to a later
  `format_version` (OQ-6), since module-binary linking (RFC 0033/0035)
  is not shipped yet
- **Author:** hpp2334
- **Depends on:** RFC 0029 (decl files, DeclIr), RFC 0033 (module binary),
  RFC 0035 (loading), RFC 0036 (symbol table & map sidecar)
- **Part:** F — Toolchain & artifacts

## Summary

`rut pack mod/` reads a module directory and emits **one file** —
`mod.rutbundle`, a zip archive carrying the manifest and the module's
sources together:

| Entry | What | Source RFC |
|---|---|---|
| `rut.toml` | manifest — format, layout version, package name | §2 here |
| `*.rut` | the entry source and every transitive relative include | 0035 §1 |

A v1 bundle is the **source** contract: the loader mounts it exactly like
the directory it was packed from — includes are inlined from archive
entries instead of the filesystem, and compilation runs as usual. The
compiled-artifact payload (`.rutc` + `.d.rut` + `.d.ir` + map, the
original RFC 0029 §6 trio zipped) is **format_version 2** (OQ-6) — it
needs module-binary linking, which does not exist yet; this RFC defines
only v1.

## 1. Layout rules

- One module per bundle in v1: the manifest's `name` names the package
  (e.g. `"imaging"` — a bare `[a-zA-Z0-9_]+` name, RFC 0041 §5) the bundle
  answers to, and `[deps]` is refused —
  a consumer directory with dependencies is a *project*, not a module
  (package trees = OQ-3, rides RFC 0029 OQ-3).
- Entry names are the source paths relative to the module root
  (`rut.toml`, `entry.rut`, `util.rut`, …), flat — no directories, no
  `META-INF/`, nothing else. Unknown extra entries are ignored by loaders
  (forward compatibility).
- Includes that escape the bundle root (`../`) are refused at load —
  an archive has no outside.

## 2. The manifest — `rut.toml`

First entry in the zip; TOML; tiny by design — and the same subset the
directory form uses, so a bundle-shaped directory packs unchanged:

```toml
format = "rutbundle"
format_version = 1          # bundle LAYOUT version — independent of rutc
name = "imaging"            # the package name this bundle answers to
entry.lib = "./imaging.rut" # the entry source, relative to the root
```

`format_version` versions the zip layout and manifest keys themselves —
loaders refuse a `format_version` they do not know **before reading
anything else**. Directory loading ignores `format`/`format_version`
(a directory is not a bundle), which is what makes the keys safe to write
into every module manifest today. **No content hashes in v1** — per-entry
hashes and signatures are OQ-2.

## 3. Deterministic packing

Same directory + same rutc ⇒ byte-identical `.rutbundle`: fixed entry
order (`rut.toml` first, then the entry source and every transitive
relative include in include order), fixed (zeroed) timestamps, STORE (no
compression) in v1. This is RFC 0033 §1's deterministic-serialization
rule extended one level up, so bundles are content-cacheable and two
builds of the same module diff to nothing. Compression settings
(DEFLATE-level pinning for large payloads) = OQ-4.

## 4. Consistency — checks on load

| Check | Rule on mismatch |
|---|---|
| `rut.toml` present in the zip | refuse — not a rut bundle |
| `format` = `"rutbundle"` | refuse — not a rut bundle |
| `format_version` | refuse — unknown bundle layout |
| entry CRC-32 (zip) | refuse — corrupt bundle (names the entry) |
| entry name / UTF-8 / STORE method | refuse — corrupt or unsupported bundle |
| `entry.lib` present, includes resolvable in the zip | refuse — load error naming the entry |
| `[deps]`, `entry.type`/`entry.ir` payloads | refuse — v1 is one source module |

Every check runs at **load time**, before compilation and link
(RFC 0035 §1) — a bad bundle never executes. When the compiled-artifact
payload arrives (OQ-6), the §4 version rows of the original draft
(`rut_version` vs entry headers, `.d.ir` regeneration, `.rutc` refusal)
apply to it.

## 5. Loading

A bundle is one more resolution source for `load_module` (RFC 0035 §1),
alongside the directory form:

```rust
let (session, root) = rut_driver::load_path_session("vendor/imaging.rutbundle")?;
// — or, for in-memory bundles (embedders, tests):
let (session, root) = rut_driver::load_bundle_bytes(&bytes, Path::new("mem"))?;
```

- After mounting, the package name resolves through the bundled sources
  exactly as the directory form does: the entry source is expanded
  (relative includes inlined, include-once) into one compilation unit and
  compiled under the manifest's `name`.
- Loose artifacts and directories remain directly loadable exactly as in
  RFC 0029 §5 — the bundle only packages them; nothing about it is
  required.

CLI: `rut pack <dir> [-o <dir>.rutbundle]` (default output is a sibling
of the directory), `rut run <dir | mod.rutbundle>` — both mount, compile,
and execute `main` like `rut run mod.rut` does for the single-file form.

## 6. Relationship to loose publishing

The directory form and loose `.rut` files stay valid publish layouts; a
`.rutbundle` is the same contract as one file — authors publish whichever
they prefer. Determinism (§3) means a bundle and the directory it was
packed from carry byte-identical sources, and a directory and its bundle
compile to identical programs (tested: the linked binaries are equal).

## Open questions

- OQ-1: binary cross-version policy — inherits RFC 0033 OQ-1; v1 refuses.
- OQ-2: per-entry content hashes and/or bundle signatures (provenance) —
  deferred by decision; the manifest is the natural home.
- OQ-3: multi-module / package-tree bundles — **ANSWERED (layout v2)**:
  a bundle embeds the whole `[deps]` graph, each package under its own
  `<pkg>/` group (its `rut.toml` + entry, recursively and
  deduplicated — `pack_dir` writes it deterministically), and the
  loader resolves groups by NAME (the deps' `path` keys are
  directory-time only). Host-pkg deps ride too — an `entry.type`-only
  package's `.d.rut` is its entry. A v1 bundle remains the one-module
  special case. A v2 bundle missing a declared dep group is a load
  error naming it; `format_version = 2` is required to pack one.
- OQ-4: compression — STORE for determinism in v1; pinned DEFLATE for
  large payloads is a layout change (bumps `format_version`).
- OQ-5: does `rut pack` also accept a `.d.rut`-only input (a surface
  bundle for host-decl distribution)? v1 refuses `entry.type` payloads;
  defer until a host package wants it.
- OQ-6: the compiled-artifact payload — `.rutc` + `.d.rut` + `.d.ir` +
  `.rutc.map` under `format_version 2`, with the original draft's §4
  version rows and RFC 0029 §5's fast path. Blocked on module-binary
  linking (RFC 0033/0035).

---

## Amendment (Sep 2026): layout v3 — the peer groups ride the archive

The dep kinds (RFC 0045) extend the OQ-3 answer: a package's
`[peer-deps]` integration files (`lib` keys — impl-only `.rut`
sources mounted by the peer gate) are part of the package, so a
bundle that drops them would load base-only — semantically wrong.
The version ledger:

| version | layout | status |
|---|---|---|
| 1 | one module, sources only | loads (the one-module special case) |
| 2 | the whole `[deps]` graph as `<pkg>/` groups (OQ-3) | loads; no longer packs |
| 3 | + each package's `[peer-deps]` `lib` files beside its entry in its group | **the packer's output** |

- **Packing**: `pack_dir` requires the bundle keys at
  `format_version = 3` (a directory-shaped manifest without them is a
  loud packing error naming the fix) and packs each pkg's `lib` files
  deterministically (name order, same-input ⇒ byte-identical).
  Directory loading still ignores the format keys (§2's law).
- **Loading**: the loader knows 1 | 2 | 3 and still refuses unknown
  versions **before reading anything else** (§2's gate — the same
  gate makes an OLDER v2-era loader refuse a v3 bundle: refuse, never
  guess). A manifest declaring `[peer-deps]` under `format_version`
  1 or 2 is refused: those layouts have no group entries.
- **The v3 load runs the peer gate over the archive** (RFC 0045 §3):
  peer present → the group file is read from the zip and appended (a
  declared group the archive lacks is a load error — refuse, never
  guess); optional peer absent → inert; required peer absent → the
  loud D1 error. The packed form and its directory compile to
  identical programs (§6's law holds with groups — pinned by the
  dep-kinds batch's T12).
- Module binary VERSION is untouched: bundle `format_version` is a
  separate ledger (§2), and impl-only groups add no host fns and no
  binary sections (RFC 0045 §5).

---

## Amendment (Sep 2026): layout v4 — `entry.libs` files ride the
archive

The multi-lib entry (RFC 0041 §5 — a package's body authored as
several `.rut` files, spliced into one module) extends the same
ledger row v3 opened for peer groups: the lib files are part of the
package, so a bundle that drops them would load base-only —
semantically wrong.

| version | layout | status |
|---|---|---|
| 1 | one module, sources only | loads (the one-module special case) |
| 2 | the whole `[deps]` graph as `<pkg>/` groups (OQ-3) | loads; no longer packs |
| 3 | + each package's `[peer-deps]` `lib` files beside its entry | loads; no longer packs |
| 4 | + each package's `entry.libs` files beside its entry | **the packer's output** |

- **Packing**: `pack_dir` requires the bundle keys at
  `format_version = 4` and packs each pkg's `entry.libs` files beside
  its entry in its group, in manifest order (the array IS the splice
  order — same-input ⇒ byte-identical).
- **Loading**: the loader knows 1 | 2 | 3 | 4; a manifest declaring
  `entry.libs` under `format_version` 1-3 is refused (those layouts
  carry one source per pkg) — the same gate peer groups got at v3,
  and the same refusal an older loader gives a v4 bundle: refuse,
  never guess. The v4 load splices the lib files exactly as the
  directory loader does (RFC 0041 §5), so §6's law — the packed form
  and its directory compile to identical programs — holds with
  multi-lib entries (pinned by the rut-driver multi-lib suite).
