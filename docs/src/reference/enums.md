# Enums

`enum` — simple named integer sets. This is the entire feature: rut has
**no data-carrying enums**. Heterogeneous data goes through traits (see
[Traits and dispatch](traits.md)); absence goes through `?T` (see
[Builtin generic types](builtin-generic-types.md)).

## Syntax

```text
enum := 'pub'? 'enum' Ident '{' member (',' member)* ','? '}'
member := Ident ('=' int)?
```

```rut
enum Color { Red, Green, Blue }              // 0, 1, 2
enum Direction { Up = 1, Down, Left, Right } // 1, 2, 3, 4
```

- An enum is a distinct named type over fixed-width integer constants.
  Members are the enum's values: implicit numbering continues from the
  last value (starting at 0); an explicit initializer (a possibly
  negative integer literal) resets the counter.
- No data payloads, no methods, no computed members — ever.

## Using members

Members are named through the enum and compare as equal singletons:

```rut
let l = Light.Yellow;
l == Light.Yellow;   // true — members are immortal singleton cells
```

- An enum value renders as its member name in format strings:
  `f"{Color.Red}"` is `"Red"`.
- Enums are ordinary shared values: binding shares the singleton.

## Exhaustive `when`

`when` over an enum must be **exhaustive**: cover every member (then
`else` is optional) or add an explicit `else` arm — a partial `when` is
a compile error. See [Control flow and when](control-flow.md).

```rut
when (l) {
    Light.Green  -> { go(); },
    Light.Yellow -> { brake(); },
    Light.Red    -> { stop(); },   // all members: no else needed
}
```

## Boundaries

- Where another language would use a union of literals
  (`"left" | "right"`), rut uses an enum; where it would use a union of
  *shapes*, rut uses a trait-typed value (see
  [Traits and dispatch](traits.md)).
- The `|` spelling exists only for bound-only union aliases and
  `requires` bounds (see [Type aliases and union bounds](type-aliases.md))
  — a compile-time admission gate, never a runtime union value.
- There is no enum↔int cast: `as` is the numeric cast only. Enum values
  cross the host boundary as their runtime identity plus the integer
  value.
