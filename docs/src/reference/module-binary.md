# Module binary and verification

The compiled module artifact — a `.rutc` file — is a versioned,
deterministic, hash-stable serialization of a linked program. Encoder and
decoder live in `rut-core/src/binary.rs`; this page is the layout, the
verification contract, and the constant-pool rules. The loader that runs
verification is [Loading and the embed loop](loading.md).

## Program shape

Everything the VM runs is one `Program`:

```rust
pub struct Program {
    pub name: String,               // module / program name
    pub scope: ScopeId,             // stable module scope (0 if unset)
    pub interner: Interner,         // every IdentId in the program
    pub surface: Surface,           // exports, for using modules (on the wire since v16)
    pub inst_types: Vec<InstTy>,    // the instantiation ledger (v17)
    pub inst_fns: Vec<InstFn>,      //   — link unifies rows by key
    pub types: TypeTable,           // type descriptors, dense ids
    pub traits: Vec<TraitDesc>,     // trait tables
    pub trait_slots: Vec<(u32, u32)>,   // global trait-method slots
    pub vtables: Vec<Vec<Option<u32>>>, // per type: slot -> func id
    pub disposal_impls: Vec<Option<u32>>, // per type: dispose fn id
    pub consts: Vec<ConstVal>,      // the constant pool
    pub funcs: Vec<FuncCode>,       // the code ([Typed bytecode](typed-bytecode.md))
    pub exports: Vec<(IdentId, u32)>,   // host-callable names
}
```

The `Surface` is the export record — functions, constants, types, traits,
impl registrations, and the builtin names `core` publishes. It is how a
*using* module binds a *used* module's members at compile time; since v16
it is part of the wire format too — the surface section rides after
`exports`, and decode validates every row against the tables above it.

## Wire layout

The format is little-endian, everywhere — encoded words, hash inputs,
checksums. The first byte of a byte range is the least significant byte
of word 0; this is a law pinned by unit tests, not an accident of the
host.

```
offset  field
0       magic "RUTC"                      # 4 bytes
4       version: u32                      # refuses any other value
8       name: str
        name table: u32 count, then count × str
        types: u32 count, then per type { name: IdentId, kind }
        traits: u32 count, then per trait { name, methods[] }
        trait slots: u32 count, then (trait: u32, method: u32) pairs
        vtables: u32 entries, then per entry { ty, [(slot, func)] }  # sparse
        disposal rows: u32 entries, then (ty, func) pairs            # sparse
        consts: u32 count, then tag + payload (below)
        funcs: u32 count, then per func (below)
        exports: u32 count, then (name: IdentId, func: u32) pairs
        instantiation ledger: type rows, then fn identities   # v17, below
        surface: namespace, funcs, consts, types + scope blocks + type
                 exports, traits, impls (both ABI lists), the reserved
                 inherent-impl table, native rows   # v16, below
```

- **Name table.** Names are interner ids everywhere — type names, field
  names, enum members, trait/method names, function names, exports. The
  binary carries only the interner's *non-well-known tail*; the fixed
  well-known prefix (`self`, `nil`, the builtin members, the primitives,
  …) is implied by the format. Decoding rebuilds the interner, so a
  decoded program is self-contained; at link, each module's tail merges
  into the linked program's table with the same rebase the type ids get.
- **Type kinds** encode as a one-byte tag: nil, primitives, `str`,
  `bytes`, arrays, enums (member + value pairs), records (field table),
  trait objects, `opaque`, fn types, `?T`, trace, string builder, `Weak<T>`.
- **Const tags**: `0` i64, `1` f64, `2` bool, `4` str, `5` type id.
  Retired tags fail decode loudly rather than being reinterpreted.
- **Funcs** serialize in full: name, param/ret types, `is_method`,
  capture count, the typed register table, both operand pools, the op
  stream, the span table, the parallel `(line, col)` position table, and
  the host-fn binding id when the function is a bodyless thunk.
- **Instantiation ledger** (v17): the owner-anchored identity of every
  instantiation the program compiled — type rows `(owner, decl,
  arguments, type id)` first, then fn identities (the kind, the
  substitution values, and the local fn id). Instantiation is owned by
  the declaring package, and link unifies rows sharing a key into ONE
  program-wide row; a consumer resolves its requests against a packaged
  binary's ledger. Type exports carry their generic parameter lists
  beside this (see the surface below).
- **Surface** (v16): the exported surface rides after `exports` — the
  namespace head, funcs (with their async/host rows), consts, the
  carried type descriptors + scope blocks + type exports (generic
  exports carry their parameter names in order, v17), trait decls,
  impl registrations in both ABI lists, a reserved length-prefixed
  inherent-impl table (zero rows until class methods link), and the
  native rows with their ambient bits. Names are ids into the name
  table above; decode rejects any id its tables cannot resolve.
- **Versioning policy**: the version `u32` must equal the toolchain's
  exactly — there is no migration or best-effort decode. A byte that
  changes observable behavior bumps the version; artifacts from older
  compilers are refused with the standard version error.

## Determinism

Same AST + same dependency versions + same compiler version ⇒
byte-identical bytes. Serialization order is fixed (mount order, manifest
order, pool order), so binaries are cacheable by content hash, two builds
diff to nothing, and a trace captured against one binary can be restored
against a locally rebuilt one ([Diagnostics, traces, and
symbolication](diagnostics.md)).

## Strip levels

The symbolication data rides in every function: the `spans` table
(pc → byte offset) and the parallel `pos` table (pc → line/col). A build
that skips filling `pos` produces a binary whose traces still capture and
render, degrading to pc-only text. Names ride the interner; dropping name
strings is a publishing choice, and stripped traces restore through
recompilation-by-determinism (see [Diagnostics, traces, and
symbolication](diagnostics.md)).

Both strippable halves are format-legal encodings, not special cases:
tail strings are opaque to the format (rewriting them to mangled
`%N` names touches no id, opcode, or table), and `0`/empty `pos` is the
documented stripped encoding — a stripped binary decodes and verifies
identically, no version bump required. The toolchain lane that produces
and restores them — the mangle/keep-set law, the private `.rutsym`
sidecar, and `run --symbols` — is [Symbol stripping and `.rutsym`
sidecars](symbol-stripping.md).

## Type ids at rest and at link

Type ids are program-global in a linked binary only because **link**
already rebased them. At rest (per module, pre-link) ids are module-local;
the boot table — the fixed primitive and builtin prefix — is shared by
every module, and each module's remaining types append after it. Every
type id reachable from a type descriptor, trait signature, constant,
function signature, or op operand is remapped at link, and
`type_id<T>()` constants rebase with everything else.

`type_id<T>()` values are `u32`, comparable, and unique per *instantiated*
type within a VM run: `Vec<f32>` ≠ `Vec<f64>`, `[i32; 3]` ≠ `[i32; 4]`
(the constant length is part of the identity), and `Point` = `Point`
across modules — identity is assigned at link
([Reified types and layout](reified-types.md)).

## Trait and impl tables

- Traits serialize with their method signatures; trait ids merge by name
  at link — a trait imported in one module and declared in another is one
  trait, and every `IsTrait` probe and vtable slot lands on the same
  global table.
- Trait-method slots are a global table of `(trait, method)` pairs;
  vtables are per-type sparse maps slot → function id, merged at link.
  A trait impl registered in any module reaches every call site.
- **Impl registrations** `(trait, target)` merge at link; a duplicate
  pair is a **link error** — per-module compiles cannot see each other,
  so the pair's uniqueness is a link-level law
  ([Traits and dispatch](traits.md)).

## Verification (load time)

Before any code executes, the verifier re-checks every function in the
binary:

| check | rule |
|---|---|
| register range | every register operand < the function's register-file size; `NOREG` legal only where an optional operand is defined |
| pool spans | every `(off, argc)` span lands inside its function's `argv` / `labels` pool |
| type operands | every type id names a row of the type table |
| jump targets | every `jmp`/`br`/`brtable` target is in range |
| call arity | call argument counts match the callee's declared signature |
| enum members | `EnumNew` member indices in range, target actually an enum |
| host thunks | bodyless functions are skipped (nothing to verify) |

A failed verification is a **load error** naming the module and function —
`function name (#i): op @pc: message`. Corrupted binaries never execute.
Verification is structural: it re-establishes what the compiler enforced
at emit time, so a hand-corrupted or stale artifact fails here instead of
misbehaving at runtime.

## The constant pool and load-time expressions

Module-level `let` and `static` initializers are **folded at compile
time**. The surviving forms are: literals, enum members, operators over
constants, record literals (immortal constant-pool cells), fixed-array
literals, `Vec<T>(n)` / `Vec.from([...])` blobs, the layout builtins, and
`Array<T, N>.len()` with const-index bounds checks. A call to a user
function in an initializer is a compile error — there is no deferred
evaluation.

`type_id<T>()` never executes: it folds to a constant from the type
table. `debug` position folding and trace symbolication are covered in
[Diagnostics, traces, and symbolication](diagnostics.md).

## What is not here

A `.d.rut` declaration surface does not produce a binary — it has no
bodies, so it compiles to a signature surface only
([Host fns and declaration files](host-fns.md)). And a `.rutbundle` is
binaries plus their container: the compiled packages ride as `.rutc`
binaries (this format), splice-needed packages and host pkgs as source
groups ([Module bundles](bundles.md)).
