# RFC 0037: Reflection — `Reflectable`, `Deserializable`, `std:reflect`

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0005 (Vec, `Array<T, N>`), RFC 0012 (`dyn I`,
  `requires`, vtables, no-recovery), RFC 0013 (generics — extended §3),
  RFC 0014 (`Opaque`, `downcast`), RFC 0015 (descriptors,
  `is_a`, boxing), RFC 0006/0009/0010, RFC 0022/0028 (builtin-impl
  registry), RFC 0025/0026, RFC 0030 (grammar — `where`), RFC 0031,
  RFC 0033
- **Part:** G — Reflection & serialization

## Summary

Reflection is an **interface**, not a keyword privilege. `Reflectable`
is the mechanism protocol; auto-implementations (dataclass, enum) and
builtin-impl registry entries (`Option`/`Result`/`Vec`/`Array<T, N>`)
fill it for the data world; classes opt in by hand with a **curated**
view. Libraries layer contracts on top (`interface Serializable
requires Reflectable {}`) and take **interface-typed consumers**
(`stringify(v: dyn Serializable)`) or **bounded producers**
(`deserialize<T>(..) where T requires Deserializable`). Nothing in the
language names a builtin; no strings are matched; nothing changes
under `--release` stripping. The proving example is user-defined JSON
in `examples/json/` — serialization stays **userland** (RFC 0001 "Why
not JavaScript" #2: an engine that must ship `JSON` is an engine with
conformance debt; rut's reflection suffices, so it doesn't).

## 1. The protocols

```rut
// std/reflect.d.rut
export interface Reflectable {            // the mechanism protocol
    fn reflect(self): TypeInfo;           // exact descriptor handle
    fn arity(self): i32;                  // children of THIS value
    fn child(self, i: i32): Option<Opaque>;  // i-th child, boxed
}
export interface Deserializable requires Reflectable { }
```

| type | `Reflectable` | `Deserializable` | stringify | deserialize |
|---|---|---|---|---|
| `dataclass` | compiler auto-impl | auto | needs opt-in (`implements Serializable`) | ✓ |
| user `enum` | compiler auto-impl | auto | opt-in | ✓ |
| `Option`/`Result`/`Vec`/`Array<T,N>` | builtin-impl registry, every instantiation | registry | as *fields* only | ✓ (Array<T, N> mint OQ-15 — v1 Vec) |
| `class`, no impl | — | — | compile error at call | compile error at call |
| `class`, manual impl | hand-written (curated) | **impossible** | ✓ (positional view) | **compile error** |

- **Auto-impls** are ordinary vtable fills (RFC 0015 §6): dataclass —
  arity = field count, child(i) = field i (boxed cell handle — shared,
  §2); enum — arity =
  the current variant's payloads. The `implements` list gains the
  entry implicitly; re-declaring one is a duplicate-impl error, and
  auto-impls satisfy `requires` edges of contract layers written on
  top (`dataclass User implements Serializable {}` costs zero methods).
- **Registry impls** follow the `string: Hashable` pattern (RFC 0022
  §2, RFC 0028) for the builtin generics, all instantiations.
- **`Deserializable` is auto-only**: hand-writing `implements
  Deserializable` is a compile error (the `Any` admission precedent,
  RFC 0014) — reflective construction is descriptor-backed, and
  classes construct through constructors (RFC 0010 §1); reflection never
  calls a constructor. Not vacuous, so it *can* be a bound.
- Manual class views are **curated**: private fields stay hidden
  because the impl does not expose them — privacy is what the impl
  says, not a walker-side law. They serialize **positionally**
  (`[...]`); they cannot deserialize.

## 2. `std:reflect` — the surface

```rut
export interface ReflectEngine { }        // module capability: implement
                                            // (≥1 per module) to use the
                                            // structural symbols
export interface Reflectable { ..§1.. }
export interface Deserializable requires Reflectable { }
export enum TypeKind { Leaf, Record, Sum, Seq, Shared }
export enum LeafKind  { Bool, Int, Float, String, Bytes, Class, Iface }
export host fn reflect<T>(): TypeInfo;    // static T (incl. interface T)
export host fn type_of(a: Opaque): TypeInfo;  // content descriptor

export host class TypeInfo {
    fn kind(self): TypeKind;              // structural role
    fn leaf(self): LeafKind;              // kind() == Leaf
    fn name(self): string;
    fn type_id(self): u32;                // == type_id<T>() for static T
    fn implements(self, i: TypeInfo): bool;  // is_a (RFC 0015 §6) — the
                                            // NESTED-node gate
    fn fields(self): Vec<FieldInfo>;      // Record: decl order (wire names)
    fn variants(self): Vec<SumVariant>;   // Sum: decl order
    fn elem(self): TypeInfo;              // Seq: Vec<T> / Array<T, N>
    // dynamic re-entry — children return as Opaque (no recovery,
    // RFC 0012 §3), so the native dispatches arity/child through the
    // box's EXACT-type vtable (RFC 0015 §6 — the calli path), reaching
    // auto and manual slots uniformly. Same slots as the interface
    // methods; rut code cannot spell this itself:
    fn arity(self, a: Opaque): i32;      // Seq .len() · Sum: current
                                            // variant's payloads
    fn child(self, a: Opaque, i: i32): Option<Opaque>;
    fn variant(self, a: Opaque): i32;    // Sum: current variant index
    // mint — Deserializable territory; each member re-checks every
    // box against the reified signature (Option out, never a trap):
    fn construct(self, vals: Vec<Opaque>): Option<Opaque>;             // Record
    fn construct_variant(self, i: i32, vals: Vec<Opaque>): Option<Opaque>; // Sum
    fn make_vec(self, vals: Vec<Opaque>): Option<Opaque>;              // Seq→Vec<T>
}
export host class FieldInfo {
    fn name(self): string;                // the wire name
    fn ty(self): TypeInfo;
    fn default(self): Option<Opaque>;    // folded at compile time initializer —
}                                          // THE parse-time default
export host class SumVariant {
    fn name(self): string;
    fn payloads(self): Vec<TypeInfo>;     // [] C-like · [T] Some/Ok/Err
}
```

No builtin is named anywhere — `Option`/`Result` appear only as
sum-shaped descriptors (`Some/None` is a `{0,1}` sum, `Ok/Err` a
`{1,1}` sum, a C-like enum an all-payloadless sum). There is no
`Shared` node anymore: `Rc` is gone, and composite children box their
**cell handle** (RFC 0016 §1) — a `child` of a record field aliases the
parent's field, so mutation through the original is observable in the
box; walkers treat them as their own Record/Sum nodes via the box's
descriptor. `TypeInfo` is a descriptor **handle**, not a
first-class type value (RFC 0015 OQ-1 stays closed).

## 3. Rules

1. **Engine admission** (resolver, RFC 0031): the structural symbols
   (`reflect<T>`, `type_of`, `TypeInfo`, `FieldInfo`, `SumVariant`)
   resolve only in modules declaring ≥1 `implements ReflectEngine`;
   violation is a compile error naming the fix. std:reflect and the
   host are exempt. *Calling* `stringify`/`deserialize` needs no
   engine — the arg type / bound carries the contract. The `is`
   keyword (RFC 0012 §3) is likewise ungated: it answers the
   capability bit, while `TypeInfo.implements(i)` — descriptor
   *walking* — stays behind the admission; walking and probing are
   different powers.
2. **Walkability = implements the protocol** — auto, registry, or
   manual. Entries may demand a contract (`dyn Serializable`) or a
   capability (`where T requires Deserializable`). Nested nodes are
   gated by the descriptor `implements(Reflectable)` query; misses are
   **values-shaped errors**, never traps.
3. **Boxing widens** (normative; cross-noted RFC 0015 §3): `Opaque`
   of an int stores i64 sign/zero-extended; a float, f64 — the slot
   discipline of RFC 0015 §5. A `Leaf` branch + `downcast<i64>` /
   `downcast<f64>` / `downcast<bool>` / `downcast<string>` is total.
4. **No string identity**: field/variant names are descriptor data
   (layout tables, RFC 0015 §6), not symbols — `--strip-native-names`
   / `--release` never touches them. Sum policy dispatches on
   **payload structure**, never on variant-name matching.
5. **Admission-only `where` bounds on user generic fns** (RFC 0013
   extension): a trailing `where T requires I` gates instantiation
   admission against the requires graph at the call site — compile
   error naming the interface. It grants **no method calls on bare
   `T`** (host-decl param-bound semantics, RFC 0025 §1); bodies may
   still widen a `T`-typed *value* to `dyn I` — the bound proves the
   widening valid.
6. Accessors are total; `Option`/`i32` results signal mismatch. No
   `field_set`: a composite child's `Opaque` **aliases** its source cell
   (RFC 0014) — a mutator would reach the original value; v1 reflection
   is read-only.

## 4. The walker (acceptance trace)

Every call below exists in §2 — this trace is the API's test:

```rut
export fn stringify(v: dyn Serializable): Result<string, string> {
    return write_val(v.reflect(), v);   // vtable reflect(); v descends
}                                       // to Opaque (erased storage)

export fn deserialize<T>(v: string): Result<T, JsonError>
        where T requires Deserializable {
    let t = reflect<T>();             // guaranteed descriptor-backed
    let tree = parse_tree(v)?;        // by the bound — no runtime
    let built = build(t, tree)?;      // pre-check
    let opt = downcast<T>(built);   // monomorphized TypeId compare (RFC 0014)
    if (opt.is_none()) { return Err(construction_failed(t)); }
    return Ok(opt.value);             // refined to exact T in this branch
}
```

`write_val`: Record → `fields()` + `child` (`f"{quote(f.name())}:{..}"`,
joined); Seq → `arity`/`child` loop (`[...]`, Vec and `Array<T, N>`
alike); Sum → all-payloadless: variant name; `{0,1}`: `null`/recurse
payload; else `Err` ("unwrap first"); Leaf → `downcast` scalars
(boxing-widened), `Leaf+Class` → `implements(Reflectable)` query →
positional walk or `Err`. `build`: Record →
`member(j, f.name())`; absent → `f.default()`, else error naming the
field; `construct(vals)`. Sums → `construct_variant(i, [])` /
`(payloaded_i, [v])`. Seq → `make_vec`. Leaves → `Opaque(v)`.

## 5. The example

`examples/json/json.rut` (the engine) + `app.rut` (the consumer) —
user-defined JSON end to end: opt-in `implements Serializable`,
manual `class Tree implements Serializable` (curated, positional),
`deserialize<Vec<Address>>` (registry instantiation), wire-dataclass
renames by hand. **Dependency note:** recursive-descent parsing needs
string byte access and byte↔string helpers — **RFC 0007 OQ-1** (the
byte-index + helpers proposal). serde is that OQ's forcing case; the
example names its assumed helpers in a header comment.

## Open questions

- OQ-1: decorators/attributes — considered (TypeId identity, protocol
  downcasts, lazy class recipes) and **rejected for v1** ("too
  complex"); `@` stays unclaimed (RFC 0030 OQ-2). Wire renames are
  separate wire dataclasses; wire contracts are `Serializable`.
- OQ-2 (resolved): wire contracts — `Serializable` (opt-in) +
  `Deserializable` (capability); serialize exposes structure outward
  (consent matters), deserialize only builds valid public values
  (capability suffices).
- OQ-3: exact-builtin identity — the structural `{0,1}` pattern, or
  `type_id()` vs `reflect<Option<E>>().type_id()`.
- OQ-4: `field_set` / mutators (deserialization-into-reuse).
- OQ-5 (resolved): `Shared` (Rc) — moot: `Rc` was removed (RFC 0016 §1);
  composite children are shared cell handles boxed as `Opaque`, walked
  as their own Record/Sum nodes (§2).
- OQ-6: content-driven walking via `type_of` (v1: declared types).
- OQ-7: `child` copies per access — lazy views if profiling demands.
- OQ-8: engine admission per-module vs per-declaration.
- OQ-9: width folding under boxing-widens (u8/i16/i32/i64/f32/f64 →
  i64/f64); `char`'s Leaf classification.
- OQ-10: auto-impl for user wrappers of reflectable types.
- OQ-11: bound + widening interplay (v1: admission only).
- OQ-12: unify `where` bounds with host/extern inline param bounds.
- OQ-13: stringify over `dyn Reflectable` sans opt-in (debug
  printers)? v1 no — contract is the discipline; debuggers use
  std:debug (RFC 0036).
- OQ-14: `dyn Slice<T>`-typed fields — Seq view over lending cells
  (RFC 0023 §2 borrows)? v1: unsupported `Leaf+Iface`.
- OQ-15: `Array<T, N>` mint — `construct` for const-generic shapes;
  v1 deserialize targets `Vec<T>`.
