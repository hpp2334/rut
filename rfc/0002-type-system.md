# RFC 0002: rut — Type System

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0001 (pillar decisions)
- **Revised:** grammar re-targeted from Rust-like to TypeScript-like
  (`interface` / `class` / simple `enum`); Option/Result are builtin types
  (no user data-enums); no sugar operators.

## Summary

rut is fully statically typed with **no dynamic typing** — no `any`, no
implicit unions. Types are inferred where possible, annotated where useful,
monomorphized where generic, and — unlike TypeScript — **reified at runtime**:
every value carries a type identity the VM and the host can inspect and check.

The grammar is TypeScript-shaped: `let`/`const`, `function`, arrow closures,
`interface`, `class` with single `extends`, `implements`, simple `enum`,
`switch`. The one controlled dynamic mechanism is **interface references**
(vtable dispatch); everything else is static.

## 1. Grammar at a glance

```rut
import { sleep } from "std:time";
import { type Rect } from "std:geom";

enum Color { Red, Green, Blue }

interface Drawable {
    draw(g: Canvas): void;              // interfaces declare METHODS only (§6)
}

class Shape {
    x: f32 = 0;
    y: f32 = 0;

    constructor(x: f32, y: f32) {
        this.x = x; this.y = y;
    }

    moveBy(dx: f32, dy: f32): void {    // overridable; calls devirtualize (§5.4)
        this.x += dx; this.y += dy;
    }
}

class Circle extends Shape implements Drawable {
    private r: f32;                     // private: class-only
    protected tag: string;              // protected: class + subclasses

    constructor(x: f32, y: f32, r: f32) {
        super(x, y);                    // base constructor call (required first)
        this.r = r;
        this.tag = "circle";
    }

    override moveBy(dx: f32, dy: f32): void {   // `override` is mandatory
        super.moveBy(dx / 2, dy / 2);
    }

    draw(g: Canvas): void {
        g.circle(this.x, this.y, this.r);
    }

    area(): f64 { return this.r * this.r * 3.141592653589793; }
}

function blitAll(g: Canvas, items: array<Drawable>): void {
    for (const d of items) {            // interface-typed loop variable
        d.draw(g);                      // vtable dispatch (§6)
    }
}

function colorName(c: Color): string {
    switch (c) {                        // exhaustiveness-checked
        case Color.Red:   return "red";
        case Color.Green: return "green";
        case Color.Blue:  return "blue";
    }
}

export function main(): void {
    const c = new Circle(1, 2, 3);
    const g = newCanvas();
    blitAll(g, [c]);                    // array<Drawable> from one Circle
    console.log(`${colorName(Color.Red)} area=${c.area()}`);
}
```

No implicit fallthrough in `switch` (`break`/`return` required per case).
`for..of` iterates arrays and strings (chars). `enum` is a simple named-int
constant set (§7) — there are **no data-carrying enums and no pattern
matching** in v1.

## 2. Primitive types

| Group | Types | Notes |
|-------|-------|-------|
| unsigned int | `u8 u16 u32 u64` | fixed width |
| signed int | `i8 i16 i32 i64` | two's complement |
| float | `f32 f64` | IEEE 754 |
| misc | `bool`, `char` (Unicode scalar, 4 bytes) | |
| heap: text | `string` | immutable, UTF-8, length-prefixed; template literals `` `a=${x}` `` |
| heap: binary | `bytes` | mutable, growable byte buffer |
| heap: seq | `array<T>` | mutable, growable, **unboxed** homogeneous storage |

- `array<f32>` is a flat `Vec<f32>` behind a header — no per-element boxing,
  no per-element refcount traffic (RFC 0004 §6).
- **Reference semantics** (like JS): class instances, arrays, strings, bytes
  are handles; assignment/parameter passing copies the handle (refcount++),
  never the payload. Primitives (`i32`, `f32`, `bool`, `char`) copy by value.
  There is no `&`/`*` syntax anywhere.
- No `null`, no `undefined`. Absence is `Option<T>` (§3).

## 3. Builtin generic types

`Option<T>`, `Result<T, E>` are **builtin (VM-native)** types — they cannot
be user-defined because user enums carry no data (§7). No sugar operators
(`?.`, `??`); the API is explicit methods plus the `?` propagation operator:

```rut
function find(xs: array<string>, key: string): Option<i32> {
    for (let i = 0; i < xs.length; i += 1) {
        if (xs[i] == key) { return Option.some(i); }
    }
    return Option.none();
}

const idx = find(list, "beta");
if (idx.isSome()) {
    console.log(idx.value);            // `.value` traps on None — check first
}
const v = idx.unwrapOr(0);             // Option<T>.unwrapOr(default: T): T
```

| `Option<T>` | `Result<T, E>` |
|---|---|
| `isSome() / isNone(): bool` | `isOk() / isErr(): bool` |
| `value: T` (traps on `None`) | `value: T` (traps on `Err`) |
| `unwrapOr(d: T): T` | `error: E` (traps on `Ok`) |
| `expect(msg: string): T` | `unwrapOr(d: T): T` |

`array<T>` completes the builtin generic set.

## 4. Literals & inference

```rut
const a = 10;            // i32 (default, OQ-1)
const b = 10u8;          // u8 via suffix
let c: u64 = 10;         // u64 via annotation
const d = 1.5;           // f64
const e = 1.5f32;        // f32
const s = "hi";          // string
const t = `hi ${name}`;  // template literal -> concat ops
const ch = 'h';          // char
const arr = [1, 2, 3];   // array<i32>
const zero = new array<f32>(1024);  // zeroed, unboxed
const buf  = new bytes(64);         // zeroed
```

- Inference is bidirectional (literal ↔ expected type); a literal without
  context defaults to `i32` / `f64`.
- **Conversions are always explicit** — `x as u8`, `i32(x)`, `f32(x)`. No
  implicit numeric conversions at all in v1 (dead-simple rule).

## 5. Classes

### 5.1 Shape

```rut
class Counter {
    private n: i32 = 0;             // default 0 shown for clarity
    static instances: i32 = 0;      // static: one slot per class, per VM

    constructor() {
        Counter.instances += 1;
    }

    press(): void {                 // public by default
        this.n &+= 1;               // wrapping add (§8) — or `+=` to trap
    }

    get count(): i32 { return this.n; }   // get accessor (set: symmetrical)
}
```

- Fields with fixed offsets, assigned in declaration order after the base
  prefix (§5.3). Initializers run at the top of the constructor (after
  `super(...)` when present).
- `private` = this class only; `protected` = class + subclasses; default =
  public. Compile-time only — no runtime cost.
- `static` members live in the class's module-static slot table.
- Accessors (`get`/`set`) are methods under the hood and may appear in
  interfaces (§6).

### 5.2 Reference semantics & lifetime

A class instance is a refcounted heap object (RFC 0004). When the last
reference dies, its reserved `dispose()` method (if any) runs, then fields
are released in order — deterministic destruction:

```rut
class TempFile {
    constructor(public path: string) { }   // ctor param property (see OQ-6)

    dispose(): void {                      // reserved: runs at refcount 0
        fs.remove(this.path);
    }
}

function scratch(): void {
    const f = new TempFile("tmp.dat");
    use(f);
}   // rc hits 0 HERE: dispose() runs, then `path` is released
```

### 5.3 Single inheritance

```rut
class Circle extends Shape implements Drawable { ... }
```

- **Single** `extends`; **multiple** `implements`.
- Layout is prefix-based: a `Circle` begins with `Shape`'s fields, then its
  own — so a base-class view is the *same pointer* (no fat pointers, RFC
  0004 stays a plain walk).
- `super(..)` must be the first statement of a derived constructor; `super.m()`
  calls the base implementation directly (statically resolved).
- Overriding a method requires `override`; signatures must match exactly;
  `dispose()` participates too (a derived `dispose` should call
  `super.dispose()` if the base has one — the compiler warns when it detects
  a missing call).

### 5.4 Dispatch: direct by default, vtable at interfaces

The static type decides the opcode:

```text
c.area()   // c: Circle    ->  call Circle$area        (direct, devirtualized)
d.draw(g)  // d: Drawable  ->  calliface d, slot=3     (vtable load + call)
```

Calls on a concrete class type are always direct — even `override` chains
resolve statically because the exact class is known. Only values whose static
type is an interface dispatch through the vtable. `final` classes/methods are
OQ-5 (they'd only matter to lock a vtable shape).

## 6. Interfaces

```rut
interface Drawable  { draw(g: Canvas): void; }
interface HitTest   { hitTest(x: f32, y: f32): bool; }

class Sprite extends Shape implements Drawable, HitTest { ... }

function pick(items: array<Drawable & HitTest>...)   // NO: no intersections in v1
```

- Interfaces declare **methods only** (including `get`/`set` accessors).
  Fields are excluded on purpose: field access through a multiple-implements
  interface has no single offset rule. Put a getter in the interface instead.
- `implements` is **nominal and declared** — structural ("duck") conformity
  does not satisfy an interface. This keeps runtime type identity exact
  (§10) and casts cheap.
- Interface-typed values are the **only** dynamic dispatch in rut: a
  reference plus a vtable lookup per call. No `dyn`-typed variables, no
  `any`, no dynamic field access, no dynamic `this`.
- Heterogeneous collections are interface-typed arrays:
  `array<Drawable>` — the replacement for both TS unions and the data-enums
  rut deliberately dropped.

### 6.1 Casting & testing

```rut
if (d is Circle) { ... }              // runtime check, exact-class or subtype
const c = d as Circle;                // checked cast; TRAPS on mismatch
const c2 = d as? Circle;              // checked cast; Option<Circle>
```

`is`/`as`/`as?` accept class and interface types. Subtype test = walk the
type descriptor's `extends` chain + `implements` list (§10.2) — no interface
hierarchies in v1 (OQ-4), so the walk is one level deep in practice.

## 7. Enums (simple)

```rut
enum Color { Red, Green, Blue }               // 0, 1, 2
enum Direction { Up = 1, Down, Left, Right }  // 1, 2, 3, 4
enum Flags { None = 0, Bold = 1, Italic = 2 }
```

- An enum is a distinct named type over `i32` constants. Members are the
  enum's values; no data payloads, no methods, no computed members.
- `switch` over an enum must be exhaustive or contain `default` — the
  compiler rejects non-exhaustive switches on enum-typed scrutinees.
- Enums cross the host boundary as their runtime identity + `i32`
  (RFC 0005 §3); the host can register its own enum types.
- Where TS would use a union of literals (`"left" | "right"`), rut uses an
  enum; where TS would use a union of *shapes*, rut uses an interface.

## 8. Integer semantics

- Overflow in `+ - * <<` **traps** in debug and release by default.
  Wrapping escapes: `&+ &- &* &<<`; saturating: `x.saturatingAdd(y)`.
- Division by zero traps; `int / int` is integer division.
- Mixed-width arithmetic: both operands must have equal width (cast first).

## 9. Functions & closures

```rut
function first<T>(xs: array<T>): Option<T> {
    if (xs.length == 0) { return Option.none(); }
    return Option.some(xs[0]);
}

const add = (a: i32, b: i32): i32 => a + b;
const areaOf = (r: f32): f32 => {
    const sq = r * r;
    return sq * 3.14159265358979;
};
xs.map((p: Point) => p.x);
```

- Closures capture **by reference** to the enclosing bindings (JS-like),
  which RC keeps safe within a VM. Closures are not transferable across
  isolates in v1 (RFC 0003 §5.2).
- Generics **monomorphize at compile time**: each instantiation emits its own
  typed opcodes (`arr.get<f32>` vs `arr.get<Handle>`). Instantiations whose
  bodies are descriptor-independent share one generic body at load time
  (RFC 0001 §Execution model); the compiler reports instantiation counts.
- Generic parameters are unconstrained in v1 — no `T extends Iface` bounds
  (OQ-7): you cannot call interface methods on a bare `T`. Pass values in,
  or take an interface-typed parameter instead of a generic.

## 10. Reified runtime types & core code

### 10.1 Value & slot representation

Interpreter registers hold untagged 8-byte slots — bytecode is typed, so hot
paths need no tag checks (RFC 0004 §1):

```rust
#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,                       // all int widths, sign/zero-extended
    pub f: f64,                       // f32 payloads widened
    pub b: bool,
    pub c: char,
    pub r: Option<NonNull<Header>>,   // class/array/string/bytes/Option/Result
}
```

The tagged `Value` enum exists only at the host FFI boundary (RFC 0005 §3).

### 10.2 Class instance & vtable layout

```rust
#[repr(C)]
struct RutClass {                    // heap object: class instance
    h: Header,                       // rc + type id (RFC 0004 §1)
    vt: *const VTable,               // exact class's vtable — set at `new`
    // fields follow: base prefix first, then derived (§5.3)
}

#[repr(C)]
struct VTable {
    ty: TypeId,                      // exact runtime type (points into RutType)
    dispose: Option<unsafe fn(*mut RutClass)>,  // §5.2 reserved destructor
    slots: [CodePtr],                // interface method slots, global ids (§6)
}
```

Interface method ids are assigned **globally per interface** at compile
time; a class's vtable fills every slot of every interface it implements
(inherited `implements` included). A call through an interface is two loads
and an indirect jump:

```rust
// d.draw(g)  where d: Drawable, draw has global slot 3
Op::CallIface { recv, slot: 3, args } => {
    let obj = unsafe { regs[recv].r.unwrap().as_ref() as &RutClass };
    let f = unsafe { (*obj.vt).slots[3] };
    self.call_code(f, recv, args)?;    // `this` passed as receiver register
}
```

Devirtualized direct call for comparison:

```text
Op::Call     { func: "Circle$area", recv, args }   ; c.area(), c: Circle
```

### 10.3 Subtype test

```rust
impl TypeTable {
    /// `d is Circle` / checked casts. Exact type is in the vtable; the
    /// descriptor lists the extends-chain and implemented interfaces, so
    /// this is a one-level walk in practice (§6.1).
    fn is_a(&self, exact: TypeId, want: TypeId) -> bool {
        if exact == want { return true; }
        let d = self.desc(exact);
        d.extends == Some(want)
            || d.implements.iter().any(|&i| i == want)
    }
}
```

### 10.4 Where reification is load-bearing

1. **Host boundary** — native fns declare parameter types once; the VM checks
   every call; no bridge-side coercion code (RFC 0005 §3).
2. **Opaque host types** — `extern class Source<T>` carries its
   instantiation identity inside the handle: `store.get(src)` is checked
   statically *and* the handle knows its `T` (fixes tur's erased
   `Source<T>`).
3. **Checked casts** — `is` / `as` / `as?` (§6.1) via §10.3.
4. **Heterogeneous collections** — vtable dispatch (§6) needs the exact type
   reachable from every object header.
5. **Debugging** — `debug.typeOf(x)`, stack traces, formatter output.
6. **Serialization** — stdlib walkers traverse `RutType` descriptors.

`RutType` descriptors are *not* first-class script values in v1 (OQ-8).

## Open questions

- OQ-1 default int width (`i32` proposed).
- OQ-2 `f16`/`bf16` for GPU-facing arrays.
- OQ-3 `map<K,V>` / `set<T>` builtin vs library.
- OQ-4 interface `extends` (interface hierarchies) — defer; one level keeps
  §10.3 a flat walk.
- OQ-5 `final` classes/methods to freeze vtable shapes.
- OQ-6 constructor parameter properties (`constructor(public path: string)`)
  — sugar used in §5.2's example; keep or drop?
- OQ-7 generic bounds `T extends Iface` (would unlock static dispatch on
  bare `T` without interface refs).
- OQ-8 first-class type values (`typeOf(x)` as a manipulable value).
- OQ-9 string indexing: byte-index + helpers proposal stands
  (`for (const c of s)` is the blessed iteration).
- OQ-10 destructuring (`const [a, b] = pair`) — omitted from v1; revisit
  with real code.
