# Reflection

Reflection is an **interface**, not a keyword privilege. `Reflectable` is
the mechanism protocol; compiler auto-fills (structs, enums) and builtin
fills (records, `Vec`, `[T]`) cover the member set for the data world;
classes join by spelling the members by hand with a **curated** view.
Libraries layer contracts on top and take interface-typed consumers or
bounded producers. Nothing in the language names a builtin; no strings
are matched; nothing changes under `--release` stripping. Userland serde
is the proving application.

Status: this page specifies the reflection surface; the protocols are
not yet wired into the engine. The surfaces it builds on — structural
interfaces ([interfaces](interfaces.md)),
[opaque](opaque.md), reified descriptors
([reified types](reified-types.md)) — are live.

## The protocols

```rut
// reflect/reflect.d.rut
pub interface Reflectable {                  // the mechanism protocol
    fn reflect(self) -> opaque;              // exact descriptor handle
    fn arity(self) -> i32;                   // children of THIS value
    fn child(self, i: i32) -> ?opaque;       // i-th child, boxed
}
pub interface Deserializable {
    fn construct(fields: [opaque]) -> ?Self; // the descriptor-backed mint
}
```

| type | `Reflectable` | `Deserializable` | stringify | deserialize |
|---|---|---|---|---|
| `struct` | compiler auto-fill | auto | spelling the member set costs zero lines — the auto-fill covers it | yes |
| user `enum` | compiler auto-fill | auto | as above | yes (`[T; N]` minting excepted — deserialize targets `Vec<T>`) |
| `Option`/`Result`/`Vec`/`[T]` | builtin fill, every instantiation | builtin | as *fields* only | yes |
| `class`, no members | — | — | compile error at the call | compile error at the call |
| `class`, members spelled by hand | hand-written (curated) | spellable by hand — reflective construction still routes through the descriptor mint | yes (positional view) | yes |

- **Auto-fills** are ordinary member fills: a struct's member set walks
  its fields (arity = field count, child `i` = field `i`, boxed); an
  enum's walks the current variant's payloads. Satisfaction is
  structural, so the filled members are the type's members — nothing
  registers, nothing duplicates. Auto-fills satisfy `requires` edges of
  contract layers written on top — a `User` satisfies a
  `Stringify<T requires Reflectable>` bound for free.
- **`Deserializable` is auto-filled for the data world**: the member is
  the construct capability, and reflective construction is
  descriptor-backed. Classes construct through their own class methods —
  reflection never calls one, so a class joins by spelling the member as
  a curated mint (or not at all). Under structural satisfaction there is
  no registration gate that could make the member auto-only; what keeps
  the law honest is that the member's only sane body routes through the
  descriptor mint.
- **Manual class views are curated**: private fields stay hidden because
  the members do not expose them — privacy is what the members say. They
  serialize **positionally**.

## The engine surface

```rut
pub interface ReflectEngine {
    fn reflect_engine(self) -> i32;          // the admission member (below)
}
pub enum TypeKind { Leaf, Record, Sum, Seq }
pub enum LeafKind  { Bool, Int, Float, String, Class, Iface }

builtin fn reflect<T>() -> opaque;          // static T (incl. interface T):
                                            // the descriptor, folded at
                                            // compile time
pub host fn type_of(a: opaque) -> opaque;   // content descriptor, boxed
```

An interface with no members would be satisfied by everything — the
empty member set is vacuously true — so the admission contract spells
one member. A `TypeInfo` **is** an `opaque` handle; the rut surface
wraps it:

```rut
pub class TypeInfo { d: opaque; .. }        // methods forward to the fns below
pub class FieldInfo { d: opaque; .. }       // name / ty / default
pub class SumVariant { d: opaque; .. }      // name / payload ordinals
```

The fn surface the wrapper forwards to:

| fn | meaning |
|---|---|
| `type_kind(t: opaque) -> i32` | `TypeKind` ordinal |
| `type_leaf(t: opaque) -> i32` | `LeafKind` ordinal |
| `type_name(t: opaque) -> str` | the type's name |
| `type_id(t: opaque) -> u32` | equals `type_id<T>()` for static `T` |
| `type_is_a(t: opaque, i: opaque) -> bool` | the nested-node gate (descriptor walking) |
| `type_fields_len(t: opaque) -> i32` / `type_field(t, i) -> opaque` | `Record`: fields in declaration order |
| `type_variants_len(t: opaque) -> i32` / `type_variant(t, i) -> opaque` | `Sum`: variants in declaration order |
| `type_elem(t: opaque) -> opaque` | `Seq`: the element descriptor of `Vec<T>` / `[T]` |
| `type_arity(t, a) -> i32` | dynamic re-entry: `Seq` length; `Sum` = current variant's payload count |
| `type_child(t, a, i) -> ?opaque` | dynamic re-entry: the `i`-th child, boxed |
| `type_variant_of(t, a) -> i32` | `Sum`: the current variant index |
| `type_construct(t, vals…) -> ?opaque` | `Record` mint; re-checks every box — mismatch is `nil`, never a trap |
| `type_construct_variant(t, i, vals…) -> ?opaque` | `Sum` mint |
| `type_make_vec(t, vals…) -> ?opaque` | `Seq` → `Vec<T>` mint |

`FieldInfo`/`SumVariant` accessors over their own handles: `name: str`,
`ty: TypeInfo` box, `default: ?opaque` (the folded load-time initializer —
the parse-time default), payload ordinals (`[]` for C-like enums,
`[T]`-shaped for `Some`/`None`/`Ok`/`Err`).

Laws:

- No builtin is named anywhere. `Option`/`Result` appear only as
  sum-shaped descriptors (`Some/None` is a `{0,1}` sum, `Ok/Err` a
  `{1,1}` sum; a C-like enum is an all-payloadless sum).
- Composite children box the **cell handle** — a child of a record field
  *aliases* the parent's field, so mutation through the original is
  observable in the box. Walkers treat them as their own Record/Sum nodes
  via the box's descriptor.
- **Boxing widens**: an `opaque` of an int stores i64 sign/zero-extended;
  a float, f64. A `Leaf` branch + `downcast<i64>` / `downcast<f64>` /
  `downcast<bool>` / `downcast<str>` is therefore total
  ([opaque](opaque.md)).
- **No string identity**: field/variant names are descriptor data, never
  symbols — stripping native names never touches them. Sum policy
  dispatches on **payload structure**, never variant-name matching.
- Accessors are total; optional/`i32` results signal mismatch. There is
  no field setter: children alias their source cells, so v1 reflection is
  **read-only**.

## Rules

1. **Engine admission**: the structural symbols (`reflect<T>`, `type_of`,
   `TypeInfo`, `FieldInfo`, `SumVariant`) resolve only in modules whose
   types satisfy `ReflectEngine` — at least one local type carries the
   member; the violation is a compile error naming the fix. Calling
   `stringify`/`deserialize` needs no engine — the argument type or the
   bound carries the contract. The `is` keyword is likewise ungated: it
   answers the capability bit, while descriptor *walking* stays behind
   admission — probing and walking are different powers.
2. **Walkability = satisfies the protocol** (auto-fill, builtin fill, or
   hand-spelled members). Callers may demand a contract (`Stringify<T
   requires Reflectable>`) or an admission bound — admission-only; it
   grants no method calls on bare `T`. Nested nodes are gated by the
   descriptor `is_a(Reflectable)` query; misses are value-shaped errors,
   never traps.

## The walker

Every call below exists in the tables above — this trace is the API's
test:

```rut
pub fn stringify(v: Reflectable) -> (str, err) {
    return write_val(v.reflect(), v);    // vtable reflect(); descends to opaque
}

pub fn deserialize<T requires Deserializable>(v: str) -> (?T, err) {
    let t = reflect<T>();                // guaranteed descriptor-backed by
    let (tree, e1) = parse_tree(v);      // the bound — no runtime pre-check
    if (e1 != nil) { return (nil, e1); }
    let (built, e2) = build(t, tree);
    if (e2 != nil) { return (nil, e2); }
    let opt = opaque.downcast<T>(built); // monomorphized type compare -> ?T
    if (opt == nil) { return (nil, construction_failed(t)); }
    return (opt, nil);                   // refined to exact T in this branch
}
```

`write_val`: Record → fields + children joined; Seq → arity/child loop
(`Vec` and `[T]` alike); Sum → all-payloadless renders the variant name, a
`{0,1}` sum renders `null`/the payload, otherwise "unwrap first"; Leaf →
downcast the widened scalars; a Leaf class → `is_a(Reflectable)` query,
then the positional walk or an error. `build`: Record → member lookup,
absent → the field's folded default, then `construct`; Sums →
`construct_variant`; Seq → `make_vec`; Leaves → `opaque(v)`.

## Reflection vs the shipped `json`

The in-tree `json` package does **not** ride reflection — its interfaces
are direct and its wrappers live in the package itself, which
measures decisively faster for schema-driven decode
([core and the swappable packages](stdlib.md)). Reflection is the right
tool for generic tooling: debug walkers, schema printers, generic editors,
and userland serde where flexibility outranks raw throughput.
