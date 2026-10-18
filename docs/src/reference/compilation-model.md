# The compilation model

rut compiles deterministically: a declared closure is a pure input, and
whole-program compilation is the **deterministic reference** — the same
closure in ⇒ the byte-identical program out, on any machine, any day. A
package **compiles** to an object (code + surface + templates); objects
and their symbol tables form an **archive** — the `.rutbundle`, the
sole publication unit. A consumer compiles its own source against the
archive's surface; the **instantiate** stage materializes the generics
the consumer spelled; the **linker** resolves and relocates —
arithmetic only. A staged build is correct exactly when it equals the
reference build.

## The pipeline

```text
publish:  compile (pkg → object + surface + templates)
          → emit declaration entry (generated mod.d.rut)
          → archive (shade the closure)

consume:  compile (app; validates `use` against the archive surface)
          → instantiate (ledger hit | template substitution + repr refold)
          → link (resolve + relocate — arithmetic only)
          → load (mount, install host rows, rebase, verify, run)
```

| stage | side | what happens | more |
|---|---|---|---|
| compile | both | source → object: lex/parse, resolve, typecheck, monomorphize, optimize, encode | [The compiler pipeline](compiler.md) |
| declaration entry | publish | the group's surface text, generated at pack — never hand-maintained | [Module bundles](bundles.md) |
| archive | publish | the object set + symbol tables, shaded to the closure, one file | [Module bundles](bundles.md) |
| instantiate | consume | the only consumer-side codegen: template substitution, ledger lookups, repr refolds — one row program-wide per shape | [The compiler pipeline](compiler.md) |
| link | consume | scope resolution + relocation + merge — arithmetic only, never type-aware, never code-generating | [Loading and the embed loop](loading.md) |
| load | consume | mount, install host rows, rebase the ids onto the loader's numbering, verify, run | [Loading and the embed loop](loading.md) |

The stages are the model's wiring, not the API's surface: embedders
compose intent-level verbs — publish is `rut pack`
([The rut CLI](cli.md)); consume is the run chain,
`RutRun::new().pkg(..).entrypoint(..).compile()`
([Embedding and native modules](embedding.md)).

## The reference, and the equivalence contract

Two statements are the mode's contract:

- **The reference is deterministic.** Whole-program compilation is a
  pure function of the declared closure: byte-identical everywhere,
  independent of caches, mounts, or how a consumer happens to spell
  its generics. Same source + same dependency versions + same compiler
  version ⇒ byte-identical output — this is what makes bundles
  diffable and content-addressable
  ([The compiler pipeline](compiler.md)).
- **Staged equals reference.** A staged build — the archive-mounted
  lane — is correct exactly when it produces what the reference build
  would have for the same closure. The equivalence is pinned, not
  assumed: goldens compile one closure through both lanes and compare
  the results — the same run behavior, and identical bytes where the
  lanes make bytes comparable.

The reference is also the widest dialect: it serves any source closure.
A staged build *derives from* the reference and is licensed by the
equivalence — it never produces more than the reference would.

## Instantiation is lazy

Only consumer-spelled shapes materialize. A compiled owner carries two
things relevant here:

- its **ledger** — the `(owner, decl, arguments)` rows its own pack
  closure spelled (the pre-instantiated bodies), and
- its **templates** — every exported generic decl compiled once
  against `#<param>` placeholder rows, riding inside the binary
  ([Module bundles](bundles.md)).

At the consumer's instantiate stage, a request the ledger satisfies is
a lookup; a shape the ledger misses monomorphizes from the template —
one row program-wide, owner-anchored, indistinguishable from a
pack-time one, nothing persisted. A shape nobody spells compiles
nowhere: that laziness is what keeps a consumer's one-off `Vec<Todo>`
out of every other consumer's program.

## Two truths: linking and editing

Every published group carries both truths, and neither contaminates
the other:

- The **object** (`mod.rutc`) is the *linking truth*: bodies plus the
  surface on the wire. A consumer binds a compiled group's exports
  from the binary alone
  ([Module binary and verification](module-binary.md)).
- The **declaration entry** (`mod.d.rut`) is the *editing truth*: the
  group's surface text, **generated** at pack time from the validated
  surface — never hand-maintained, so it cannot drift from what
  links.

This is the TypeScript `.d.ts` pattern: ship the declaration beside
the compiled code, generate it from the thing that links, and let the
runtime ignore it. The loader never reads declaration entries of
compiled groups; they ride for humans and tooling.

## The editor's ladder

Editor-facing data degrades gracefully — and nothing editor-facing is
ever load-required:

1. **Declaration entry (full)** — names, types, structure: the
   editor's primary view of a dependency. Go-to-def on a dependency's
   member lands on the declaration — the declaration is where the name
   is published; the body is the owner's private code.
2. **Object surface (names + types)** — the binary's own surface
   section, always present, is the floor: tooling can read names and
   signatures from the decoded object alone
   ([Module binary and verification](module-binary.md)).

Stripping removes names for runtime privacy and carries a private
sidecar to restore them ([Symbol stripping and `.rutsym`
sidecars](symbol-stripping.md)); the surface rows the linker and the
instantiation machinery consult survive by construction, so the
ladder's floor holds on stripped bundles too.

## Where to read more

- [The compiler pipeline](compiler.md) — the compile stage, stage by
  stage.
- [Module bundles](bundles.md) — the archive: layout, identities,
  checks, the CDN lane.
- [Module binary and verification](module-binary.md) — the object on
  the wire.
- [Loading and the embed loop](loading.md) — the consume tail: offer,
  link, verify, boot.
