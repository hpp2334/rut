# Type aliases and union bounds

`type X = A;` — the transparent alias. `type X = A | B;` — the
bound-only union. `fn f<T requires A | B>(..)` — the inline
admission-only bound. All three are compile-time gates: no IR, no
runtime representation.

## Type aliases

```text
typealias := 'pub'? ('(' vis ')')? 'type' Ident '=' Type ';'
```

```rut
type Meters = i64;
type Km = Meters;             // chains expand: Km is Meters is i64
type Row = [i32];

let trip: Km = 1500;          // Km is Meters is i64 — all one type
let plain: Meters = trip;     // no conversion: the alias IS the target
```

- **Transparency by construction**: `X` resolves to the target's type
  identity everywhere — params, fields, returns, lets, bounds. A value
  of the alias and a value of the target are the same type; there is no
  conversion and no wrapper.
- Chains expand through resolution; `type A = B; type B = A;` is a
  "recursive type alias" error. Forward references are legal.
- The alias **target validates at declaration**: `type Foo = NotAType;`
  is an error whether or not `Foo` is used.
- **Non-generic**: the head admits no members. `type Vec2<T> = Vec<T>;`
  is not expressible — a generic alias would be a type constructor, not
  a transparent name.
- **One name = one type**: an alias may not share its name with an
  alias, enum, struct, class, or trait in the same module — the
  duplicate-name check is symmetric and fires in both orders.
- Visibility: `pub`, `pub(mod)`, `pub(super)`, `pub(self)` all parse on
  `type` (see [Modules and visibility](modules-and-visibility.md)).
  Like every name, the alias is usable in a consumer only when the
  consumer's `use` names it.

## Union aliases — bound-only

```text
unionalias := typealias   // with Type spelled `A | B`
bound      := Type ('|' Type)*
```

```rut
type Num = i32 | str;    // legal — here and in requires bounds ONLY
```

- The union exists only in the type **grammar**; the compiler never
  interns a union runtime kind. There is no runtime union value, no
  subtyping, and no `is`/`when` support over unions (heterogeneous data
  goes through traits — see [Traits and dispatch](traits.md) — or
  enums, see [Enums](enums.md)).
- **Value positions reject unions**: a union (or a union-alias name) in
  a param/field/return/let/`is` position is diagnosed as *bound-only*.
- `|` continues a completed type as a union only where unions are legal
  — the alias target and `requires` bounds — so `x as u32 | y` keeps
  spelling the binary operator.
- Union aliases are **module-local**: never exported; a cross-module
  use fails as an unknown type.

## Inline `requires` — the admission-only bound

```text
gparam := Ident ('requires' bound)?
```

```rut
trait Labeled { fn label(self) -> str; }

fn name<T requires Labeled>(x: T) -> str {
    let w: Labeled = x;      // the widening compiles BECAUSE the bound holds
    return w.label();
}

fn kind<T requires Ridge | Trench>(x: T) -> str { .. }   // a union bound
```

- Members may be concrete type names (satisfied by exact type identity
  at instantiation), aliases (expanded before detection), or a **type
  union** of those. **Type names only in a union**: a trait member
  inside a union spelling parses but diagnoses at admission — a trait
  bound must stand alone.
- **Enforcement at every substitution-completing site**: free-fn calls,
  direct and instance method instantiation, and impl-method enqueue. A
  failing instantiation diagnoses with the bound spelled out:
  "`bool` does not satisfy `T` requires `i32 | str` — no matching type
  or impl is registered".
- **Trait objects satisfy nothing**: instantiating `T` at a trait type
  fails any bound — only a concrete type with a registered impl admits.
- **Bounds may reference the item's other generics**:
  `fn hold<T, U requires [T]>(x: U)` — members resolve under the
  call-site substitution.
- **Admission-only — except the union's whole-bound contract.** A
  non-union bound grants NO method calls on bare `T`; what it proves is
  the widening into a bound-member-typed slot. A **union bound**
  carries the whole-bound method contract: a method call on a
  union-bounded value is checked against the WHOLE bound — every member
  must provide the method through its own impls — even when the
  offending member is never instantiated. Dispatch is untouched: each
  instantiation still binds the concrete member's own impl.
- **Provenance** — a value is known to carry a union-bounded type when
  it is: a generic parameter, a `let` whose annotation spells the
  generic, a copy of either, or a direct `self.f` field whose declared
  type spells the generic. Provenance rides inlining and is cleared
  when the name rebinds. Documented holes (the gate under-fires, never
  over-fires): closure bodies compile with fresh provenance;
  destructuring carries none; a shadowing rebind resets it.
- **`where` is removed.** The trailing clause is gone; `where` is an
  ordinary identifier, and a stray clause diagnoses with the inline
  replacement: `fn f<T requires B>(..)`.
- **Class generics take bounds; struct/trait/surface generics do not**:

  ```rut
  pub class HashMap<K requires i8 | .. | str | bytes, V> { .. }
  ```

  The bound records on the class descriptor and admits every
  instantiation: a key type outside the set fails **at the
  instantiation site** (see
  [Builtin generic types](builtin-generic-types.md)). Inside the
  class's method bodies the union contract applies through the class's
  own `requires`.

See also [Functions, closures, and generics](functions-closures-generics.md)
for the bound grammar in context.
