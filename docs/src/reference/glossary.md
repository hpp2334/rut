# Glossary

The book's vocabulary for the [compilation model](compilation-model.md).
Where a standard toolchain word's intuition is ~90% right, rut uses the
community's word and documents only the delta — there are exactly two.

## The vocabulary

| term | meaning |
|---|---|
| deterministic reference mode | whole-program compilation over a declared closure — byte-identical everywhere; the contract every other lane is measured against, not a fallback ([The compilation model](compilation-model.md)) |
| object file | a package's compiled binary (`mod.rutc`): bodies + surface — the linking truth ([Module binary and verification](module-binary.md)) |
| archive | the published `.rutbundle` — objects and their symbol tables, shaded to the closure; uber-jar / Go `.a` semantics; the sole publication unit ([Module bundles](bundles.md)) |
| declaration entry | the generated surface text (`mod.d.rut`) a compiled group carries for humans and tooling — the editing truth; generated at pack, never hand-maintained |
| symbol table | the archive's scope ledger (`rut.scopes`): one row per linked module, spelling the content identity the loader relocates through ([Module bundles](bundles.md)) |
| templates | instantiation-independent bodies: exported generic decls compiled once against `#<param>` placeholder rows and carried in the binary, so any consumer-spelled shape can materialize at the consumer |
| instantiate | the consumer-side materialization of the generics the consumer spelled — template substitution, ledger lookups, repr refolds; the only consumer-side codegen |
| link | scope resolution + relocation + merge — arithmetic only, never type-aware, never code-generating |
| load + relocate | mounting a decoded archive into a run: its ids rebase onto the loader's own numbering (dynamic relocation, PIE-style) |
| pre-instantiated bodies | the ledger rows the pack closure already compiled — ≈ COMDAT-folded duplicates |
| auto-linked core / prelude | `core` — offered and linked automatically unless overridden, like Go's runtime ([core and the swappable packages](stdlib.md)) |
| host bindings | host packages and their rows — the FFI surface ([Embedding and native modules](embedding.md)) |
| compilation session | the run's internal compile state (`RutRun`) — an implementation detail, never user-facing vocabulary |

Already community-standard, unchanged: **pkg**, **manifest**,
**`[deps]`**, **`rut.lock`**, **install**, **fetch**,
**url + sha256 pins**, **pack** ([Dependency kinds](dependency-kinds.md),
[The rut CLI](cli.md)).

## Delta 1 — instantiation independence

The contract in plain words: the same generic substitution, spelled by
any consumer, in any run, against any mount of the owner, yields
the same body — one row program-wide, identity by owner. Consumer
spelling picks shapes; it never changes bodies.

This is separate compilation for generics: a package publishes without
knowing the shapes its consumers will spell, and still every shape
links to exactly one body — a ledger hit when the owner's own closure
already compiled it, otherwise a substitution into the template the
binary carries ([Module bundles](bundles.md)). There is no standard
toolchain word for the property — the groundings are C++ template
instantiation and COMDAT folding — so rut describes it instead of
branding it: **instantiation independence, the separate-compilation
contract**.

## Delta 2 — surface validation at compile

A consumer compiles against an archive without its source: the
consumer's **compile** stage reads the object's **surface** — exported
names, signatures, types — the way rustc reads a dependency's metadata.
`use` heads, member sets, and types validate at consumer compile time;
a missing member is a compile diagnostic, never a link or load surprise
([The compiler pipeline](compiler.md)).

## Internal vocabulary

Engine internals — seeds, reseeding, the graph walk, dev tables — may
appear in source comments and trace output. They are not part of the
model, and the book avoids them.
