# RFC 0033: Module Image & Verification

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0032 (LIR), RFC 0029 (DeclIr), RFC 0025
  (decl digest), RFC 0001 (M1)
- **Supersedes:** RFC 0007 §5–8 (pre-restructure)
- **Part:** F — Toolchain & artifacts

## 1. The `.rutc` module image

```rust
struct ModuleImage {                 // serialized, versioned, hash-stable
    version: u32, name: String,
    imports: Vec<(specifier, Vec<ImportedName>)>,   // resolved by loader
    types: Vec<RutType>,             // incl. field layouts + vtables
                                       (RFC 0015 §6, repr C — RFC 0024)
    consts: Vec<Const>,              // strings, byte blobs, i64/f64, tids
    funcs: Vec<FuncImage>,           // name, signature (typed regs), code,
                                       // state tables (suspend), host slots
    symbols: SymbolTable,            // function symbols + pc→span — RFC 0036;
}                                     // strippable (below + RFC 0036 §4)
```

- Decl surfaces (`.d.rut`, RFC 0029) compile to **DeclIrs**
  (`.d.ir`), not images — no bodies exist. A module that references
  surface members carries their **native slot table**: member name → slot
  id, signatures, and the surface's **decl digest** (covers slots — a
  re-bound impl or republished package whose table disagrees fails link,
  RFC 0035 §1). Member-name strings live only in diagnostics data;
  `--strip-native-names` (or `--release`) drops them — publishing hides
  names (RFC 0029 §6).
- Deterministic serialization: same AST + same dependency versions →
  byte-identical image (cacheable by content hash; the embedder may ship
  `.rutc` artifacts instead of sources).
- `type_id`s are **module-local indices at rest**; link-time rebase maps
  them into the VM's global type table (RFC 0035 §1). `type_id<T>()`
  constants are re-based with everything else.
- **Symbols & spans (RFC 0036).** `symbols: SymbolTable` carries
  per-function names (`FuncSym.name`) and pc→span interval tables — the
  data stack traces symbolicate against. Strip levels: default keeps
  names + spans; `--strip-native-names` drops name strings; `--release`
  additionally drops spans — traces still capture, degrading to
  `pkg:mod #[3] @ pc 41`. `rutc build --map` serializes the same
  `SymbolTable` as a `.rutc.map` sidecar keyed by the image's content
  hash — a **stable, versioned contract** (the deliberate opposite of
  `.d.ir`, an unstable DeclIr cache that contains no symbols at all,
  RFC 0029 §4) for restoring traces captured against stripped images.

## 2. Verification (load time)

The verifier re-checks, per function: register types vs op signature,
def-before-use, jump targets in-range and to block heads, `brtable`
density, factory `Self { .. }` completeness (every uninitialized field
covered — RFC 0010 §1), suspend state tables closed under resume edges,
native-slot signatures vs the DeclIrs the module compiled against
(RFC 0029 — including generic-instantiation admission: the `implements`
scan closing over `requires`, so a bad `MyMap<Canvas, ..>` is a load
error), and `callnat` slots present in the native table. A failed
verification is a load error reporting the module and function — corrupted
images never execute.

## 3. Const pool & const-expressions

RFC 0003 §1's const-expression rule is enforced here: module `const` and
`static` initializers are **folded at compile time**; the surviving forms
are literals, enum members, operators over consts, dataclass literals,
fixed-array literals, `Vec<T>(n)`/`Vec.from([..])`/`bytes(n)` blobs, and
the layout builtins. A call to a user
function in a const position is a compile error (RFC 0003 OQ-1), not a
deferred-evaluation hack.

`type_id<T>()`, `size_of<T>()`, `align_of<T>()` (RFC 0015 §3) never
execute at runtime: HIR folds them to constants from the type table.
`type_id<T>()` values are `u32`, comparable, and unique per *instantiated*
type within a VM run (`Vec<f32>` ≠ `Vec<f64>`, `Array<i32, 3> ≠
Array<i32, 4>` — const `N` is identity, RFC 0005, `Point` = `Point`
across modules — identity is assigned at link). `size_of<T>()`/`align_of<T>()`
return the repr-C value size/alignment (RFC 0024): for dataclasses and
classes this is the C-layout field block (what `Vec<Point>` strides by,
what `StructCopy` copies, what a host struct mirrors).

## Open questions

- OQ-1: image compatibility across VM versions — `version` field exists;
  policy (refuse, recompile, or migration shims) undecided. (`.d.ir` has
  no such question — it is a cache, never a contract, RFC 0029 §4.)
