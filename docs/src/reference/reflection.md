# Reflection

Reflection is a **trait**, not a keyword privilege. `Reflectable` is the
mechanism protocol; compiler auto-implementations (dataclasses, enums) and
builtin-impl registry entries (enums, records, `Vec`, `[T]`) fill it for
the data world; classes opt in by hand with a **curated** view. Libraries
layer contracts on top and take trait-object-typed consumers or bounded
producers. Nothing in the language names a builtin; no strings are
matched; nothing changes under `--release` stripping. Userland serde is
the proving application.

Status: this page specifies the reflection surface; the protocols are
not yet wired into the engine. The surfaces it builds on — traits,
[opaque](opaque.md), reified descriptors
([reified types](reified-types.md)) — are live.

## The protocols

```rut
// reflect/reflect.d.rut
pub trait Reflectable {                     // the mechanism protocol
    fn reflect(self) -> opaque;             // exact descriptor handle
    fn arity(self) -> i32;                  // children of THIS value
    fn child(self, i: i32) -> ?opaque;      // i-th child, boxed
}
pub trait Deserializable requires Reflectable { }
```

| type | `Reflectable` | `Deserializable` | stringify | deserialize |
|---|---|---|---|---|
| `dataclass` | compiler auto-impl | auto | opt-in (`impl Serializable for T {}`) | yes |
| user `enum` | compiler auto-impl | auto | opt-in | yes |
| `Option`/`Result`/`Vec`/`[T]` | builtin-impl registry, every instantiation | registry | as *fields* only | yes (`[T; N]` minting excepted — deserialize targets `Vec<T>`) |
| `class`, no impl | — | — | compile error at the call | compile error at the call |
| `class`, manual impl | hand-written (curated) | **impossible** | yes (positional view) | **compile error** |

- **Auto-impls** are ordinary vtable fills: a dataclass walks its fields
  (arity = field count, child `i` = field `i`, boxed); an enum walks the
  current variant's payloads. The descriptor's impl list gains the entry
  implicitly; re-declaring one is a duplicate-impl error. Auto-impls
  satisfy `requires` edges of contract layers written on top —
  `impl Serializable for User {}` costs zero methods.
- **`Deserializable` is auto-only**: hand-writing `impl Deserializable
  for T` is a compile error. Reflective construction is
  descriptor-backed, and classes construct through their own class
  methods — reflection never calls one. The trait is not vacuous, so it
  can still be a bound.
- **Manual class views are curated**: private fields stay hidden because
  the impl does not expose them — privacy is what the impl says. They
  serialize **positionally** and cannot deserialize.

## The engine surface

```rut
pub trait ReflectEngine { }                 // module capability (below)
pub enum TypeKind { Leaf, Record, Sum, Seq }
pub enum LeafKind  { Bool, Int, Float, String, Class, Trait }

builtin fn reflect<T>() -> opaque;          // static T (incl. trait T):
                                            // the descriptor, folded at
                                            // compile time
pub host fn type_of(a: opaque) -> opaque;   // content descriptor, boxed
```

A `TypeInfo` **is** an `opaque` handle; the rut surface wraps it:

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
   `TypeInfo`, `FieldInfo`, `SumVariant`) resolve only in modules
   declaring at least one `impl ReflectEngine for T`; the violation is a
   compile error naming the fix. Calling `stringify`/`deserialize` needs
   no engine — the argument type or the bound carries the contract. The
   `is` keyword is likewise ungated: it answers the capability bit, while
   descriptor *walking* stays behind admission — probing and walking are
   different powers.
2. **Walkability = implements the protocol** (auto, registry, or manual).
   Entries may demand a contract (`Serializable`) or a capability (an
   inline `requires Deserializable` bound on a generic — admission-only;
   it grants no method calls on bare `T`). Nested nodes are gated by the
   descriptor `is_a(Reflectable)` query; misses are value-shaped errors,
   never traps.

## The walker

Every call below exists in the tables above — this trace is the API's
test:

```rut
pub fn stringify(v: Serializable) -> (str, err) {
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

The in-tree `json` package does **not** ride reflection — its traits are
direct, nominal, and container impls live in the package itself, which
measures decisively faster for schema-driven decode
([core and the swappable packages](stdlib.md)). Reflection is the right
tool for generic tooling: debug walkers, schema printers, generic editors,
and userland serde where flexibility outranks raw throughput.
