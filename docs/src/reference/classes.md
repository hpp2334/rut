# Classes and constructors

`class` — the sealed record: module-private fields by default,
construction gated behind class methods, no inheritance.

## Declaration and construction

```rut
use ink::{ Logger };

class Rect {
    w: f32;
    h: f32;
}

impl Rect {
    pub fn new(w: f32, h: f32) -> Self {   // construction VALIDATES — it is
        if (w <= 0 || h <= 0) {            // just a function
            panic("Rect: negative extents");
        }
        return Self { w: w, h: h };
    }

    pub fn from_square(s: f32) -> Self {   // named constructors are siblings
        return Rect.new(s, s);
    }

    pub fn area(self) -> f32 { return self.w * self.h; }
}

entry fn main() {
    let log = Logger.new("t");
    let r = Rect.from_square(3);
    log.info(f"area={r.area()}");
}
```

```text
area=9
```

- **Classes construct through their own class methods — nothing else is
  constructible.** No outside literal exists:

  ```rut
  let r = Rect.new(3, 4);            // the one construction surface
  let v = Version.parse("1.2");      // ?Version — nil on failure
  // Rect { w: 1, h: 1 };            // ERROR: classes have no outside literal
  ```

  `new` is not special syntax — just the conventional primary-constructor
  name (`from`, `parse`, `open`, `default` are its siblings); it is an
  ordinary identifier. Try-construction returns the nullable: a class
  method `fn parse(s: str) -> ?Version` answers `nil` on failure.

- **The `Self { field: expr, .. }` literal is the class-private
  construction** — legal anywhere inside the class body (class methods
  and instance methods alike); the class name spells it inside the body
  too (`Rect { .. }`). The literal must initialize every field without
  an initializer; field initializers run for omitted fields. Private
  fields are settable in the literal — inside the class body only.
  That privacy *is* the seal: outside code can build a class value only
  by calling a class method that chooses to build one.

- **No implicit default construction.** A class whose fields all have
  initializers still needs an explicit `fn new() -> Self { return
  Self {}; }` if outsiders should build it. A class with no accessible
  constructing class method is **sealed** — constructible only inside
  its own body.

- **No parameter properties**: parameters are parameters — the
  `Self { field: name }` literal makes the param→field mapping explicit.

- **`async` class methods are allowed** — same function, `await` in the
  body. No partially constructed instance ever exists across an
  `await`: the `Self { .. }` literal is an ordinary expression, and
  locals live in the coroutine frame.

## Newtypes: the one-field wrapper

`class Name(Wrapped);` — the positional one-field decl. It IS an
ordinary class: exactly one field, `inner`, of the wrapped type. The
positional spelling additionally provides the constructor — the call
form `Name(value)`:

```rut
class JsonI64(i64);              // the wrapper family — one per wrapped
class JsonI32(i32);              // type, each with its own body
class JsonF64(f64);
impl JsonI64 {
    pub fn encode(mut self, w: JsonWriter) -> nil { w.int(self.inner); }
}

dump(JsonI64(64));               // manufacture at the boundary
```

- **The wrapped type is unrestricted** — primitives, composites,
  tuples, fn types, another module's class: `class Opt(?i64);`,
  `class Pair((i32, str));`, `class Cb(fn(i32) -> str);` are all legal.
  The wrapped position is a FIELD type, never an impl target — that is
  the point. A wrapper is how a primitive or a third-party type gains a
  capability satisfaction can't reach: declare the wrapper, implement
  on it, hand the wrapper across the boundary.
- **No auto-insertion, ever.** `dump(64)` is an error forever; the
  capability exists exactly where the constructor is spelled. And no
  implicit forwarding: the wrapper exposes exactly its own members —
  `inner` (the ordinary field, ordinary visibility) and its `impl`
  methods.
- **The naming law**: wrappers are named for what they wrap (`JsonI64`,
  never `JsonExt`). Per-type wrappers with type-specific bodies when
  the bodies differ; a generic wrapper covers the uniform cases:
  `class DebugWrap<T>(T);`.
- **The generic instantiation law**: a binder is determined by the
  wrapped argument when it appears there (`class Tail<T>([T]);` →
  `Tail([1, 2])` is a `Tail<i32>`), or by explicit type arguments at
  the constructor (`Converter<i32>("64")` — `class Converter<T>(str);`
  cannot infer, the binder is not in `str`). If any binder is
  undetermined, the call spells ALL binders — all-or-nothing:
  `class Pair<A, B>(i32);` accepts `Pair<i32, str>(7)`, never
  `Pair<i32>(7)`.
- **The third-party adapter** (the orphan case, dissolved):
  `class TheirJson(TheirType);` plus an inherent `encode` hands THEIR
  type the capability — consumers manufacture `TheirJson(x)` from their
  own modules; the newtype flag rides the class's surface row like the
  class itself does.
- A braced class — even one with a single `inner` field — has no call
  construction. The positional spelling is what provides the
  constructor; everything else stays sealed.

At runtime the construction mints a real cell: the wrapper is a value
like every class.

## Methods: explicit `self`

- **The receiver is explicit.** An instance method spells its receiver
  as the first parameter — `fn add(self, x: i32, y: i32)` — and the
  body reads fields through `self`. A
  method that mutates declares `mut self` and requires a `let mut`
  receiver (see [Modules and visibility](modules-and-visibility.md)).
- A method **without** a `self` parameter is a **class method** —
  invoked on the class itself (`Rect.new(..)`, `Self.new(..)` inside
  the body). Presence or absence of `self` is the whole distinction.
  Class methods are ordinary functions: they validate, default,
  cache, register, or hand out singletons.
- **A computed property is a method** (`c.count()`), and a settable one
  takes an argument (`c.set_count(n)`). One member kind, one call
  convention.
- **Static fields** are declared `static name: T = init;` in the class
  body, with the same visibility forms as fields.

## Member visibility

Members follow the same visibility forms as declarations (see
[Modules and visibility](modules-and-visibility.md)): an unannotated
field or method is **module-private**; `pub`, `pub(mod)`, `pub(super)`,
`pub(self)` expose it to the form's audience. The construction surface
is therefore explicit: `pub fn new(..)` builds; unannotated members
stay the class's own business within its module.

## Reference semantics and equality

A class value is a **heap cell handle** like every non-primitive (see
[By-reference and nullable](by-reference-and-nullable.md)):
assignment shares, mutation is visible through aliases. `==` is cell
identity; field-wise comparison is an interface you write for it.

## No inheritance

- **No `extends` for classes** — no base-class constructors
  (`super(..)`), no method overriding, no `super.m()`, no `protected`.
- Code sharing is composition (hold a helper object or struct in a
  field) or free functions; polymorphism is only the structural kind —
  a type used through an interface whose members it has (see
  [Interfaces and dispatch](interfaces.md)).
- Layout stays trivial: fields at fixed offsets — identical inside every
  cell payload (see [Reified types and layout](reified-types.md)) — no
  prefix layout, no fat pointers, and every object has exactly one
  concrete class forever. That keeps `is` a single descriptor check.
