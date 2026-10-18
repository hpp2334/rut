# Symbol stripping and `.rutsym` sidecars

A publisher can ship a compiled artifact whose function, type, field,
interface, and method names — and source positions — are unreadable, while
keeping a private **symbol table** that, when supplied at load time,
restores the real names and line/col for
[stack-trace symbolication](diagnostics.md). Users holding only the
binary see mangled names and pc-only traces; users holding the sidecar
see the real thing.

The VM changes nothing: symbolication is lazy, per-index, against the
loaded program's interner and position tables — restoring is a matter
of what the loaded program carries
([Diagnostics, traces, and symbolication](diagnostics.md)).

## The model

```sh
rut pack plugins/server --strip -o server.rutbundle
# packed plugins/server -> server.rutbundle (...) + symbol table
# server.rutsym (... bytes, keep PRIVATE)
```

Two files leave the build:

| file | role |
|---|---|
| `server.rutbundle` | the public artifact — every renameable name mangled, every function's symbolication tables gone |
| `server.rutsym` | the private half — mangled → original name rows, and the span/position tables per module |

The sidecar is **never an entry inside the bundle**. It rides beside it,
and whoever holds it may restore; whoever holds only the bundle cannot.
It is also not encrypted or obfuscated — see the limitations below.

## What is renamed

Everything name-shaped a binary carries: function names, type names,
field names, interface and method names, enum members, and the exported
surface rows. Renaming is purely a **tail rewrite**: every name in a
`.rutc` is an interned-id index, and the binary serializes only the
name table's instance-local tail
([Module binary and verification](module-binary.md)) — rewriting those
tail strings touches nothing else. Ids, opcodes, vtables, fn tables,
and type layouts are untouched, so **no format-version bump rides this**:
a stripped binary decodes and verifies identically, and `0`/empty
position tables are the documented stripped encoding.

Mangled names are **`%0`, `%1`, …** — sequential over the sorted union
of renameable names across the whole packed closure (deterministic and
collision-free). `%` can never appear in an identifier or a package
name, so a mangled string cannot collide with any source-derived name
by construction.

Mangling is one closure-wide string→string map applied to every compiled
group in the bundle — never per-group random names. Bundles link across
`.rutc` groups by name text at load (interface merge-by-name, the
instantiation-ledger keys), and a consistent map preserves all
cross-group identity for free.

## The keep-set

Some names are load-bearing and never rename:

| kept | why |
|---|---|
| well-known names (`main`, the builtin members, the primitives) | never in the serialized tail |
| host-fn registration names (`FuncCode::host_id`, `SurfaceFn.host`, host thunks) | the embedder's registry keys — `ink_host::log`, `rt:log`, … |
| export names | hosts call by string (`vm.call("main", …)`) |
| package specs in instantiation rows | load-time unification keys |

`Program::name` (the module's display name in traces) is a plain string
field, not an interned name — it stays as-is. Local variable names never
exist at runtime (registers only), so there is nothing to strip there.

## Positions

The pc → byte-offset span table and the parallel pc → (line, col)
position table leave **together**, per function, and restore together.
Without them, `StackTrace.line(i)`/`col(i)` read `0` and `render()`
degrades to the pc-only form — the behavior is unchanged, only the
diagnostics lose fidelity ([Diagnostics](diagnostics.md)).

## The mixed-closure refusal

`--strip` needs a **fully-compiled** closure. A package riding as a
declaration file set (a host pkg, or a source dep whose unit did not
link) would bind its compiled dependencies' surfaces at load — and
those names are mangled. The packer refuses rather than guesses:

```text
pack: --strip needs a fully-compiled closure: the source group `boxy`
binds compiled `util`'s surface at load, whose names would be mangled —
restructure the closure, or pack without `--strip`
```

Host and declaration leaf groups have no compiled dependencies and pass
trivially. A refused world packs fine without `--strip`.

## CLI

```sh
rut pack <dir> [--strip] [-o out.rutbundle]   # + out.rutsym beside it
rut run <mod.rutbundle> [--symbols <file.rutsym>]
```

- `--strip` writes the bundle as today plus the sibling sidecar
  (`mod.rutbundle` → `mod.rutsym`) and prints both paths.
- `--symbols` is legal only against a compiled `.rutbundle` — the
  source and directory lanes compile fresh and need no map (a
  mismatched input is a usage error). The table restores **before** the
  graph compiles, so linking and every trace see real names. A section
  naming a module the bundle does not carry draws a warning; a table
  from a different build simply matches nothing and errors nothing.

Embedders get the same lane as library calls: the packer's
`pack_dir_opts` returns the sidecar bytes beside the bundle, and
`.symbols(&map)` on the run chain restores into the offered compiled
pkgs.

## Determinism and limitations

Same input directory ⇒ byte-identical bundle ⇒ byte-identical sidecar
(the mangled numbering is sorted-union based, like everything else in
the pack). That determinism is also the honest limitation:

- **String values are data, not names** — string constants in the
  binary stay exactly as the source wrote them. Stripping hides
  identifiers; it does not scrub string literals.
- **The sidecar is the private half, not a secret.** Anyone holding the
  original source can rebuild the same closure deterministically and
  regenerate an identical map. Treat `.rutsym` like a `.pdb`: distribute
  it only to parties who may see names and positions.
- **Stripping and generics compose.** A
  successful pack's templates cover its generic surface by
  construction, so `--strip` on a generic-owning closure is legal
  unconditionally: the consumer-spelled shapes monomorphize from the
  in-binary templates at link, never from source. The keep-set
  therefore preserves every name instantiation independence (the
  separate-compilation contract) consults — the
  exported surface's names (funcs, types, consts, interface and
  inherent methods), the generic parameter names (the consumer's
  substitution env spells `#{param}` against them), the template
  rows' bound names and bounds, the bound-member stubs' member texts,
  the instantiation ledger's owner specs, and the carried type block's
  row names (a mirror construction reads the target row's spelling).
  The mangle stays real where nothing consults: internal helpers,
  lambdas, and pre-surface temporaries rename to `%N` beside the
  sidecar. A consumer binding a stripped compiled group's surface
  later is still a loud unification miss, never a mislink.

## What is not here

`rut dump` on a `.rutc` with `--symbols`, and auto-keeping the shared
surfaces of a mixed closure (so it could strip anyway), are natural
follow-ups; neither ships today.
