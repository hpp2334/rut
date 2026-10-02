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
use ink::{ Logger };

enum Color { Red, Green, Blue }              // 0, 1, 2
enum Direction { Up = 1, Down, Left, Right } // 1, 2, 3, 4

entry fn main() {
    let log = Logger.new("t");
    log.info(f"{Color.Blue} {Direction.Right}");
}
```

```text
Blue Right
```

- An enum is a distinct named type over fixed-width integer constants.
  Members are the enum's values: implicit numbering continues from the
  last value (starting at 0); an explicit initializer (a possibly
  negative integer literal) resets the counter.
- No data payloads and no computed members — ever. Methods are an
  `impl` block away (see [Impl blocks](#impl-blocks)).

## Using members

Members are named through the enum and compare as equal singletons:

```rut
use ink::{ Logger };

enum Light { Red, Yellow, Green }

entry fn main() {
    let log = Logger.new("t");
    let l = Light.Yellow;
    log.info(f"{l == Light.Yellow}");   // true — members are immortal singleton cells
}
```

```text
true
```

- An enum value renders as its member name in format strings:
  `f"{Color.Red}"` is `"Red"`.
- Enums are ordinary shared values: binding shares the singleton.

## Exhaustive `when`

`when` over an enum must be **exhaustive**: cover every member (then
`else` is optional) or add an explicit `else` arm — a partial `when` is
a compile error. See [Control flow and when](control-flow.md).

```rut
use ink::{ Logger };

enum Light { Green, Yellow, Red }

fn go() { let log = Logger.new("t"); log.info("go"); }
fn brake() { let log = Logger.new("t"); log.info("brake"); }
fn stop() { let log = Logger.new("t"); log.info("stop"); }

entry fn main() {
    let l = Light.Yellow;
    when (l) {
        Light.Green  -> { go(); },
        Light.Yellow -> { brake(); },
        Light.Red    -> { stop(); },   // all members: no else needed
    }
}
```

```text
brake
```

## Impl blocks

Enums take `impl` blocks — the same three forms a struct or class
takes: non-self methods (called on the enum's name), `self` methods
(called on a value), and trait impls.

```rut
use ink::{ Logger };

enum Dial { Low, High }

impl Dial {
    fn default() -> Self { return Dial.Low; }
    fn flipped(self) -> Dial {
        return when (self) {
            Dial.Low  -> Dial.High,
            Dial.High -> Dial.Low,
        };
    }
}

entry fn main() {
    let log = Logger.new("t");
    let d = Dial.default().flipped();
    log.info(when (d) {
        Dial.Low  -> "low",
        Dial.High -> "high",
    });
}
```

```text
high
```

- A non-self method is a namespaced function: `Dial.default()`.
  `Self` spells the enum, so `-> Self` is the return type and
  `Dial.Low` mints the member.
- A `self` method dispatches on the value: `d.flipped()` — the
  receiver crosses as the singleton cell it already is.
- Method visibility is real: `pub fn` exports cross-module, plain
  `fn` is module-private (see
  [Modules and visibility](modules-and-visibility.md)).

Trait impls make enum values iterable — `for (let v of c)` rides the
same desugar as a class's (see [The iteration protocol](traits.md)):

```rut
use core::{ Iterable };
use ink::{ Logger };

enum Light { Green, Yellow, Red }

impl Iterable<Light> for Light {
    fn iterate(self, emit: fn(Light) -> bool) {
        let mut cur = self;
        for (let i = 0; i < 3; i += 1) {
            if (!emit(cur)) { return; }
            cur = when (cur) {
                Light.Green  -> Light.Yellow,
                Light.Yellow -> Light.Red,
                Light.Red    -> Light.Green,
            };
        }
    }
}

entry fn main() {
    let log = Logger.new("t");
    for (let l of Light.Red) {
        log.info(f"{l}");
    }
}
```

```text
Red
Green
Yellow
```

One limit stays: `Disposal` is for structs and classes only — the
engine disposes a record's cell, and an enum member is immortal.

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
