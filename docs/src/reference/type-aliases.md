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
use ink::{ Logger };

type Meters = i64;
type Km = Meters;             // chains expand: Km is Meters is i64
type Row = [i32];

let trip: Km = 1500;          // Km is Meters is i64 — all one type

entry fn main() {
    let log = Logger("t");
    let plain: Meters = trip;     // no conversion: the alias IS the target
    log.info(f"{trip} {plain}");
}
```

```text
1500 1500
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
  alias, enum, struct, class, or interface in the same module — the
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
  goes through interfaces — see
  [Interfaces and dispatch](interfaces.md) — or
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
use ink::{ Logger };

interface Labeled { fn label(self) -> str; }
struct Ridge { depth: i32; }
struct Trench { depth: i32; }

impl Ridge { fn label(self) -> str { return "ridge"; } }
impl Trench { fn label(self) -> str { return "trench"; } }

fn name<T requires Labeled>(x: T) -> str {
    let w: Labeled = x;      // the widening compiles BECAUSE the bound holds
    return w.label();
}

fn kind<T requires Ridge | Trench>(x: T) -> str { return "geo"; }

entry fn main() {
    let log = Logger("t");
    log.info(f"{name(Ridge { depth: 3 })} {kind(Trench { depth: 1 })}");
}
```

```text
ridge geo
```

- Members may be concrete type names (satisfied by exact type identity
  at instantiation), aliases (expanded before detection), or a **type
  union** of those. **Type names only in a union**: an interface member
  inside a union spelling parses but diagnoses at admission — an
  interface bound must stand alone.
- **Enforcement at every substitution-completing site**: free-fn calls,
  direct and instance method instantiation, and impl-method enqueue. A
  failing instantiation diagnoses with the bound spelled out:
  "`bool` does not satisfy `T` requires `i32 | str` — no matching type
  in the bound". An interface bound fails with the member set:
  "`T`'s instantiation does not satisfy `Labeled`: no member `label`".
- **Interface-typed values satisfy nothing**: instantiating `T` at an
  interface type fails any bound — only a concrete type with the
  members admits.
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
- **Class generics take bounds; struct/interface/surface generics do not**:

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
