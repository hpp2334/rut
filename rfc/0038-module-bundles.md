# RFC 0038: Module Bundles — `.rutbundle`

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0029 (decl files, DeclIr), RFC 0033 (module image),
  RFC 0035 (loading), RFC 0036 (symbol table & map sidecar)
- **Part:** F — Toolchain & artifacts

## Summary

`rutc pack mod.rut` loads a module and emits **one file** — `mod.rutbundle`,
a zip archive carrying the image and every surface artifact together:

| Entry | What | Source RFC |
|---|---|---|
| `rut.toml` | manifest — format, versions, module specifier | §2 here |
| `mod.rutc` | module image with bodies | 0033 §1 |
| `mod.d.rut` | declaration surface (text, hand-editable) | 0029 §2 |
| `mod.d.ir` | compiled DeclIr of the surface — **required** | 0029 §3 |
| `mod.rutc.map` | symbol table sidecar | 0036 §2 |

A bundle is not a new contract — it is RFC 0029 §6's publish trio plus the
map sidecar, zipped, with a versioned manifest naming the parts. Consumers
mount it and resolve the specifier it answers to (§5). Consistency between
the parts is checked by **rut's version recorded in each artifact** — no
content hashes in v1 (§4; OQ-2).

## 1. Layout rules

- One module per bundle in v1: `mod` names the artifact stems; the
  manifest's `module` names the specifier (e.g. `"imaging"`) the bundle
  answers to. Package trees = OQ-3 (rides RFC 0029 OQ-3).
- `mod.d.ir` is **always embedded** — a consumer typechecking against the
  bundle takes RFC 0029 §5's fast path (no parse of `.d.rut` at all). If
  its compiler version has gone stale, rutc regenerates it in memory from
  the bundled `.d.rut` (RFC 0029 §4's rule; the bundle is never rewritten).
- `mod.rutc.map` travels by default when the image was built with names/
  spans; `--release` strip policy (RFC 0033 §1, 0036 §4) governs both the
  image's `symbols` and whether the sidecar carries anything at all.
- Entry names inside the zip are exactly the stems above — no directories,
  no `META-INF/`, nothing else. Unknown extra entries are ignored by
  loaders (forward compatibility).

## 2. The manifest — `rut.toml`

First entry in the zip; TOML; tiny by design:

```toml
format = "rutbundle"
format_version = 1          # bundle LAYOUT version — independent of rutc
rut_version = "0.1.4"       # exact toolchain version that packed it
module = "imaging"          # specifier this bundle answers to

[entries]
image = "mod.rutc"
decl = "mod.d.rut"
declir = "mod.d.ir"
map = "mod.rutc.map"        # omitted when stripped (RFC 0036 §4)
```

`format_version` versions the zip layout and manifest keys themselves —
loaders refuse a `format_version` they do not know before reading anything
else. `rut_version` is the cheap pre-check: it must agree with the version
headers inside `.d.ir` (RFC 0029 §3 `version: CompilerVersion`) and
`.rutc` (RFC 0033 §1); disagreement means a repacked/tampered bundle — a
corrupt-bundle load error naming module and entry. **No content hashes in
v1** — the decl-digest link check (RFC 0029 §6) is semantic linkage between
surface and image and is unchanged by bundling; per-entry hashes and
signatures are OQ-2.

## 3. Deterministic packing

Same source + same rutc ⇒ byte-identical `.rutbundle`: fixed entry order
(`rut.toml`, image, `.d.rut`, `.d.ir`, map), fixed timestamps, STORE (no
compression) in v1. This is RFC 0033 §1's deterministic-serialization rule
extended one level up, so bundles are content-cacheable and two builds of
the same module diff to nothing. Compression settings (DEFLATE-level
pinning for large images) = OQ-4.

## 4. Consistency — version policy on load

| Check | Rule on mismatch |
|---|---|
| `rut.toml` `format_version` | refuse — unknown bundle layout |
| `rut.toml` `rut_version` vs entry headers | refuse — corrupt bundle (names module + entry) |
| `.d.ir` `version` vs running rutc (RFC 0029 §4) | regenerate from bundled `.d.rut`, in memory |
| `.rutc` `version` vs running VM (RFC 0033 §1, OQ-1) | refuse in v1 — recompile from source when the consumer has it; otherwise the bundle is simply not loadable by that rutc |

Zip CRCs cover transport corruption; no additional integrity layer. Every
check above runs at **load time**, before verification (RFC 0033 §2) and
link (RFC 0035 §1) — a bad bundle never executes.

## 5. Loading

A mounted bundle is one more resolution source for `load_module`
(RFC 0035 §1), ahead of loose files:

```rust
loader.mount_bundle("vendor/imaging.rutbundle")?;   // reads rut.toml, §4 checks
```

- After mounting, the specifier `imaging` resolves through the bundle:
  **surface** = the bundled `.d.ir` (fast path) or, regenerated, the
  bundled `.d.rut` — RFC 0029 §5's order with the bundle's entries standing
  in for the loose files.
- **Bodies** (`extern` linkage, RFC 0029 §6) come from the bundled
  `mod.rutc`, verified (RFC 0033 §2) and linked like any image; the
  decl-digest compare runs against the bundled DeclIr, so a repacked
  surface that disagrees with the image fails link, not runtime.
- The bundled `mod.rutc.map` serves `vm.symbolicate` fallback for stripped
  images (RFC 0036 §3 path 2) — read from the zip, not the filesystem.
- Loose artifacts (`.d.rut`, `.d.ir`, `.rutc` as plain files) remain
  directly loadable exactly as in RFC 0029 §5 — the bundle only packages
  them; nothing about it is required.

CLI: `rutc pack mod.rut [-o mod.rutbundle] [--release]`,
`rutc bundle list mod.rutbundle` (prints the manifest + entry sizes),
`rutc run app.rutbundle` (mounts, then `vm.call("main", ..)` — RFC 0035 §1).

## 6. Relationship to loose publishing

RFC 0029 §6's loose trio (`.rutc` + `.d.rut` [+ `.d.ir`]) stays a valid
publish layout; a `.rutbundle` is the same contract as one file — authors
publish whichever they prefer, and the consumer sees no difference: the
resolver's order (RFC 0029 §5) is source → mounted bundle → loose cache →
loose `.d.rut` — first hit wins, so a bundle never shadows a developer's
own source tree.
Determinism (§3) means a bundle and the loose trio built by the same rutc
carry byte-identical payloads.

## Open questions

- OQ-1: image cross-version policy — inherits RFC 0033 OQ-1; v1 refuses.
- OQ-2: per-entry content hashes and/or bundle signatures (provenance) —
  deferred by decision; the manifest is the natural home.
- OQ-3: multi-module / package-tree bundles — one `.rutbundle` per module
  in v1; a package-level bundle re-exporting a tree rides RFC 0029 OQ-3.
- OQ-4: compression — STORE for determinism in v1; pinned DEFLATE for
  large images is a layout change (bumps `format_version`).
- OQ-5: does `rutc pack` also accept a `.d.rut`-only input (a surface
  bundle with no image, for host-decl distribution)? Nothing in §5 depends
  on the image being present; defer until a host package wants it.
