# RFC 0029: Declaration Files & the DeclIr — `.d.rut`, `.d.ir`, Publishing

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0003 (modules), RFC 0025 (host classes — read after),
  RFC 0022 (embedding)
- **Part:** F — Toolchain & artifacts

## Summary

rut separates **surface** from **implementation** at the artifact level.
Three file kinds, one rule each:

| Kind | Contents | Role |
|---|---|---|
| `.rut` | implementation source | what authors write; **may not declare `host`/`extern`** — a compile error: "belongs in a `.d.rut`" |
| `.d.rut` | declarations only — the surface | publishable, human-readable, hand-writable; **the only place `host fn/class/primitive` and `extern fn/class` may appear** |
| `.d.ir` | compiled **DeclIr** of a `.d.rut` — the declaration surface | cache; version-locked (see §4) |

Plus the runtime artifact `.rutc` — the compiled module binary (RFC 0033)
with bodies. A published package ships `.rutc` (implementation) +
`.d.rut` (surface). Consumers typecheck against the surface and link
against the binary — they never need package source.

The `.d.rut` mechanism serves both linkage kinds (RFC 0025):

| keyword | implementation lives in | link check compares against |
|---|---|---|
| `host fn` / `host class` | the **embedding Rust** — a registered `NativeModule` | the ClassTable / fn-table reflection (RFC 0026 §1) |
| `extern fn` / `extern class` | **another rut compilation unit** — a published `.rutc` binary | the package binary's export/slot table (§6) |

## 1. Why declaration files exist

- **Publishing without source.** Package authors likely want to hide
  variable names and logic. Publishing compiled binaries (`.rutc`) does that:
  bodies are compiled; local names are compiled away (§6).
- **Cheap third-party typechecking.** To compile module `M` importing
  `"pkg:mod"`, the compiler needs only `pkg:mod`'s *surface* — names,
  signatures, bounds, slots. Bodies are needed at link/run, not compile
  (§5).
- **Host surfaces in-language.** The embedder's Rust modules
  (`app:gfx`, `plugin:my_map`, `std:collection`) declare themselves the
  same way — the tur `index.d.ts` role, but type-checked in rut itself
  (RFC 0025).

## 2. What a `.d.rut` may declare

A `.d.rut` is parsed in **declaration mode** (RFC 0030 §3): declarations
only, and — beyond RFC 0003's module scope — every declaration must be
*complete as a surface*. Allowed:

- `import` / `pub` — visibility applies exactly as in RFC 0003 §2
  (non-exported decls are known inside the file, nameable nowhere else);
- `let` — with load-time expression initializers (RFC 0033 §3);
- `enum` — a member list *is* the whole definition;
- `trait` — method signatures (+ `requires`) *are* the whole
  definition (`std:collection`'s `Hashable` lives this way,
  RFC 0028);
- `dataclass` — **fields only** (with load-time expression field initializers). Field
  names and types are the published surface (RFC 0015 §4), so a published
  value type is sound. No method bodies, no impl blocks in v1 (OQ-2);
- `host fn` / `host class` / `extern fn` / `extern class` — signatures
  only, with admission-only param bounds (RFC 0025);
- `host primitive` — **the native member surface of a primitive type**:
  `pub host primitive str { fn len(self) -> i32; }`. Members are
  bodiless and statically bound (RFC 0032 §1.1 R2 — named things on
  builtins are natives, never ops); the decl is the declarative form of
  the host's builtin member table. This is how primitives grow methods
  **without a wrapper-class fiction** — `str` stays the one name for
  the type and the impl target (the `std:string` `String` builder is a
  separate class, not a wrapper name; the slot
  table is per-primitive, same shape as a `host class`). `primitive` is
  a contextual keyword, `.d.rut`-only after the linkage keyword — it
  stays a legal identifier everywhere else. Builtin containers
  (`Vec`, `Option`, …) remain `host class` decls: they are class-shaped
  (generic); primitives are not.

Forbidden — the parser errors "implementation in a declaration file":
`fn` with a body, `class` with a body, field/method bodies of any kind,
statements. And symmetrically, a `.rut` file that spells `host`/`extern`
errors "declaration keyword in an implementation file — belongs in a
`.d.rut`".

## 3. Compiling a `.d.rut` — the DeclIr

A `.d.rut` compiles to a **DeclIr** (serialized as `.d.ir`):

```rust
struct DeclIr {                       // .d.ir — no bodies, no code
    version: CompilerVersion,         // exact rutc version — §4
    module: String,                   // specifier this surface answers to
    exports: Vec<Symbol>,             // name, visibility, kind
    types: Vec<RutType>,              // full resolved types incl. bounds
    slots: Vec<(String, SlotId, Sig)>,// host/extern members, decl order
    digest: DeclDigest,               // hash over everything above
}
```

- **Slot ids** are assigned here, in declaration order (RFC 0025) — the
  table every consumer's `callnat { slot }` ops index.
- The **decl digest** covers names, kinds, full types under the crossing
  rule (RFC 0026 §3), bounds, and slots. Link compares digests — a pure
  data compare, nothing runs (RFC 0026 §1 for `host`; §6 here for
  `extern`).
- Compiling a `.d.rut` runs the same resolve/typecheck passes as `.rut`
  (RFC 0031) minus code generation: bad imports, unknown types, cyclic
  `requires` — all surface errors surface here, with zero Rust linked.

## 4. `.d.ir` is a cache, not a contract

`.d.ir` files are **unstable across compiler versions**: the format may
change freely, any release may regenerate them. The header's exact version
must match the running rutc — on mismatch, rutc silently regenerates from
the sibling `.d.rut` (or refetches it). Nothing may *link* or *ship* a
`.d.ir` as a compatibility surface; the stable artifacts are `.d.rut`
(text) and `.rutc` (binary, versioned per RFC 0033 §1 policy). This keeps
the DeclIr format cheap to evolve — it can change quickly, cross
version, without deprecation cycles.

**Naming note:** the `.d.ir` payload is the **DeclIr** — *declaration
IR*, literally what the extension spells. "Symbol table" is a reserved
term in this series: it means the binary-side function-symbol + pc→span
tables inside `.rutc` (and their `.rutc.map` sidecar) that stack traces
symbolicate against (RFC 0036). A DeclIr has no function symbols and
never participates in trace restoration.

## 5. Import resolution during compilation

When module `M` imports `"pkg:mod"` and rutc needs its surface to typecheck
`M`, resolution is (first hit wins):

1. **source**: `pkg/mod.rut` present on the source path → compile it
   normally (development mode; its exported surface *is* the DeclIr);
2. **bundle**: a mounted `.rutbundle` answering to `pkg:mod` → the
   surface is its bundled `.d.ir` (or, regenerated from the bundled
   `.d.rut` when version-stale) — RFC 0038 §5; explicit mount outranks
   stray caches, never dev source;
3. **cache**: `pkg/mod.d.ir` with a matching compiler version → load the
   DeclIr directly — no parse, the fast path;
4. **decl**: `pkg/mod.d.rut` → compile to a DeclIr (and write the
   `.d.ir` cache next to it);
5. **host registry**: for `host`-linked modules the embedder ships the
   `.d.rut` alongside the registered implementation (RFC 0022 §1).

Bodies are resolved only at link/run: registered Rust for `host` decls;
the published `pkg/mod.rutc` binary for `extern` decls. Compiling `M`
against a package whose implementation is absent is legal and complete —
`rutc check M.rut` passes; only `vm.load` requires the bodies.

## 6. Publishing a package

An author publishes:

- `pkg/mod.rutc` — the compiled binary with bodies (RFC 0033). Local names
  are compiled away during compilation; what remains in diagnostics data
  can be dropped entirely — `--strip-native-names` (or `--release`) leaves
  member-name strings out of the binary (RFC 0033 §1).
- `pkg/mod.d.rut` — the surface, hand-written or generated
  (`rutc decl pkg/mod.rut` emits it from the module's exported surface);
  hand-editable afterwards.
- optionally `pkg/mod.d.ir` — pointless to ship (consumers regenerate,
  §4), but harmless.

Everything above may instead ship as **one file**: `rutc pack` zips binary +
surface + DeclIr + the `.rutc.map` sidecar under a versioned `rut.toml`
manifest — a `.rutbundle` (RFC 0038). Same contract, one file; the
resolver (§5) treats a mounted bundle's entries as the loose artifacts.

For an `extern class` in a consumer to link, the published `.rutc` must
carry an export/slot table whose decl digest equals the one the consumer
compiled against (§3) — drift is a load error naming both modules, never
a runtime surprise. This is the same two-checkpoint contract as RFC 0026
§1 with "linked binary" substituted for "ClassTable reflection".

**Honesty note:** compilation is not encryption — published code is
recoverable with effort, like any compiled artifact. What publishing
guarantees is *API discipline*: consumers depend only on the declared
surface, names/logic do not appear in diagnostics, and the digest makes
silent drift impossible.

## Open questions

- OQ-1: should `rutc decl` (surface generation) also emit doc comments
  into `.d.rut`, and is a generated-then-edited file re-checkable against
  the binary (surface drift lint)?
- OQ-2: `dataclass` in `.d.rut` with impl blocks/method bodies — the
  impl table would live in the `.rutc` while the field block ships in the
  surface; defer until a package actually needs it.
- OQ-3: multi-module packages — one `.d.rut` per module, or a package-level
  surface file re-exporting the tree (RFC 0003 OQ-2 dependency)?
