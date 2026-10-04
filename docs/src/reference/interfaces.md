# Interfaces and dispatch

An `interface` is a member set: signatures only, no bodies. A type
satisfies an interface by **having the members** — satisfaction is
**structural**, checked once at the positions that need it. There is
nothing to register, nothing to declare, and no `impl` lane for an
interface: `impl I for T` does not parse. Dispatch follows two positive
rules, fixed at compile time per call site.

## Interfaces

```rut
pub interface Shape {
    fn area(self) -> f64;         // bodiless signatures; async legal
}

pub interface Decoder<E> {
    fn decode(mut r: JsonReader) -> (Self, ?E);   // Self + generics legal
}
```

- **Signatures only, no bodies.** No fields, no properties, and **no
  default implementations, ever** — one member kind, one dispatch
  candidate per call. Anything that reads like a property is a method.
- **`Self` and generics are legal.** `Self` in a member's signatures
  accepts the satisfying concrete type (`-> Self` returns,
  `(?Self, ?E)` decodes into it); a generic interface gives each
  type-argument list its own member set (`Decoder<E>` ≠
  `Decoder<F>`). Interface generic parameters take **no bounds** — an
  interface is a flat requirement, not a constraint stack.
- Members carry **no `pub`** — they are as visible as the interface.
- **No `requires` list on the interface.** Bound-taking code spells
  `T requires Shape` at the use site; the interface itself stays a
  bare member set.
- **No embedding.** Interfaces are flat member sets — an interface
  cannot embed another. Compose by declaring the members: an interface
  that needs two capabilities spells both member sets.
- **No object-type keyword anywhere**: an interface name in *type
  position* is the bare name — `s: Shape`, `Vec<Shape>`, generic
  arguments included.
- **No top interface.** The erased-storage type is the concrete
  primitive `opaque` (see [opaque — erasure and downcast](opaque.md)),
  reached by an explicit call, never by widening.
- **Intersection types (`A & B`) are never supported** — not deferred:
  heterogeneous needs compose an interface that declares both member
  sets.
- Interfaces may be declared in `.d.rut` declaration files —
  signatures-only fits the declaration surface; the engine's own
  prelude carries none (the engine's contracts are the bracket
  markers and the closed classes below).

## Satisfaction — the member-set law

Satisfaction is **structural**: a type satisfies `I` exactly when its
**public inherent members** cover `I`'s member set. Any `pub` form
counts — a class method, a struct's method on its inherent impl, a
newtype wrapper's member. Checking, never searching: there is no
registry, no registration, no impl block for an interface anywhere in
the language.

- **The check runs at the boundary** — exactly the positions where a
  value of some type `T` is used *as* the interface:
  - **interface-typed parameters** — `dump(p)` against
    `fn dump(x: Serializable)` checks `Point`'s member set once;
  - **typed lets and assignments** — `let s: Shape = Point { .. };`
    is a satisfaction assertion;
  - **returns** — a function whose return type is the interface
    checks each returned concrete;
  - **boxing sites** — pushing into a `Vec<Shape>` checks before the
    box is built.
  Everywhere else the compiler never asks the question.
- **Members must be the type's own.** The member set is the public
  *inherent* surface — the type's module's impl block. A third
  party's inherent block is refused; a used type's members live where
  the type was declared.
- **Signatures must match**: receiver form (`self` vs `mut self` vs
  none), parameter types, return type, and `async` spelling — exactly,
  wherever both sides are locally visible. `Self` in the interface's
  signatures accepts the satisfying concrete type.
- **Failure names all three**: passing a `Point` without `area` to a
  `Shape` parameter diagnoses
  "`Point` does not satisfy `Shape`: no member `area`".
- **Who can never satisfy**: primitives, the composites (`[T]`,
  tuples), `?T`, and foreign closed classes have no inherent members,
  so no member set — they satisfy nothing. The route in is the
  **wrapper** (below).
- **The empty member set is vacuously true**: an interface with no
  members is satisfied by every type — there is no marker idiom. If a
  declaration needs to mean something, it spells a member.
- **Shape-equal is satisfied.** Two types whose public members have
  the same shapes both satisfy — including types whose author never
  thought of the interface. Accidental satisfaction is real and
  accepted: the mitigation is convention (distinct member shapes,
  spelled receivers), not machinery. No registration exists that
  could gate it.

## `impl T { .. }` — the one impl form

Type bodies are fields only; methods live in impl blocks. The inherent
impl is the only impl form:

| | `impl T { .. }` (inherent) |
|---|---|
| Lives in | `.rut`, **T's module only** |
| Valid targets | local `struct`/`class`, or a `builtin class` this module declares |
| `pub(..)` | classes only (structs are all-public) |
| `async` | legal |
| no-`self` methods | legal (constructors) |
| fields / empty body | never / legal |

```rut
pub interface Serializable {
    fn encode(mut self, w: JsonWriter) -> nil;
}

class Point {
    x: i32;
    y: i32;
}

impl Point {
    pub fn encode(mut self, w: JsonWriter) -> nil {
        w.key("x"); w.write_i64(self.x);
        w.key("y"); w.write_i64(self.y);
    }
}
```

- The orphan rule and the one-impl-per-pair link law are **gone** —
  there is nothing to register, so there is nothing to duplicate and
  no placement rule beyond the inherent one (the type's own module).
- **Generic binders are declared after `impl`** — `impl<T> Vec<T> {
  .. }`, not `impl Vec<T> { .. }`. The declared list is the definition
  site: a bare parameter name in the head is a use that must resolve
  against it ("undeclared type parameter `T` — declare it:
  `impl<T> ..`").
- **`Self` in impl signatures** names the impl's target under the
  impl's substitution: `-> Self` returns, `Self { .. }` constructs.
- An inherent impl may not carry an interface's name anywhere — there
  is no `for` clause to spell.

## The wrapper route — manufacturing a member set

Primitives and composites can never satisfy (no inherent members, no
way to add one — `impl i32 { .. }` diagnoses). The route in is a
**newtype class**: a one-field wrapper whose inherent impl carries the
members, manufactured **explicitly at every call site**:

```rut
pub interface Encodable {
    fn encode(self) -> str;
}

class JsonI64(i64);

impl JsonI64 {
    pub fn encode(self) -> str { return str(self.inner); }
}

fn dump(x: Encodable) -> str { return x.encode(); }

entry fn main() {
    let s: str = dump(JsonI64(64));   // spelled manufacture
}
```

- `dump(JsonI64(64))` — ✓ spelled, sound, per-call. The standard
  library's json package spells exactly this shape for real
  (`encodeJson(JsonI64(64))` — see
  [core and the swappable packages](stdlib.md)).
- `dump(64)` — ✗ **forever**. No auto-insertion exists at any layer:
  the constructor call IS the manufacture, and its spelling is the
  audit trail.
- Wrappers compose: a generic wrapper's bound checks at instantiation —
  `class SWrap<T requires Encodable> { v: Vec<T>; }` admits only
  instantiations whose argument satisfies, and the wrapper's own
  members let `SWrap` satisfy in turn (recursive wrappers are legal).
- The standard library's wrapper families (`JsonI64`/`JsonF64`/…,
  `ArrFlow`/`VecFlow`/…) are exactly this shape — see
  [core and the swappable packages](stdlib.md).

## Using an interface — the four forms

```rut
use pouch::{ Vec };
use ink::{ Logger };

pub interface Shape { fn area(self) -> f64; }

class Circle { r: f64; }
class Square { side: f64; }

impl Circle { pub fn area(self) -> f64 { return 3.14159 * self.r * self.r; } }
impl Square { pub fn area(self) -> f64 { return self.side * self.side; } }

fn dump(x: Shape) -> f64 {            // 1. interface-typed parameter
    return x.area();
}

entry fn main() {
    let log = Logger.new("shapes");

    dump(Circle { r: 1.0 });          // 2. the boundary checks the member set

    let s: Shape = Square { side: 2.0 };   // 3. satisfaction assertion

    let mut shapes: Vec<Shape> = Vec.new();  // 4. heterogeneous: itable boxes
    shapes.push(Circle { r: 1.0 });
    shapes.push(Square { side: 3.0 });
    let mut total: f64 = 0.0;
    for (let sh of shapes) { total += sh.area(); }
    log.info(f"total={total}");
}
```

```text
total=12.14159
```

An interface-typed value is a reference to a real cell that still
carries its exact class. Passing a concrete value at an
interface-typed position is implicit and allocates nothing; the
compiler checks the member set at that boundary and moves on — there
is no widening relation to maintain and no registry row to consult.
An interface-typed value **cannot be downcast**: use it through the
interface, or erase explicitly through `opaque`.

## Dispatch — the two-rule law

Every method call compiles under exactly one of two rules; the rule is
a property of the call site, fixed at compile time.

- **Static** — the call site names exactly one concrete type; the call
  binds directly to the member, no table hop:
  - a concrete receiver — `c.area()` on `c: Circle`;
  - an interface-typed local of **single concrete origin** —
    `let s: Shape = Point { .. }; s.area()` binds straight to
    `Point`'s member;
  - an interface-typed **parameter** — the callee specializes per
    concrete argument type (one clone per argument type — finite,
    terminating), so an interface parameter *is* an implicit generic
    bound;
  - a monomorphized generic — inside `fn first<T>(..)`, `T`'s members
    are static per instantiation.
- **Itable** — the receiver is interface-typed with multiple possible
  concrete origins; the call consults the value's itable — the
  per-(concrete type × interface) method table:
  - heterogeneous container elements — `for (s of shapes)` over a
    `Vec<Shape>`;
  - interface-typed field/element loads;
  - branch-merged origins — `let s = if (c) { a } else { b };` where
    the arms carry different concretes into one interface-typed
    binding.

The itable rows are synthesized **at the boxing sites where
satisfaction was proved** — the same boundary check that admitted the
value fills the row. Origin counting is conservative: any merge,
indirection load, or cross-function flow counts as multiple.
Mis-analysis cannot produce wrong code — an uncertain origin costs an
itable hop, never a wrong static bind.

**The use-both gate**: `x.iface_member()` requires **both** names at
the call site's module — the type (by declaration or `use`) *and* the
interface (`use`). A call through an interface no `use` names is an
error — "use `I` to call its methods on `T`" — so a foreign interface
can never silently change what a call site means.

## Type tests — `is`

`expr is Type` → `bool`, at relational precedence, non-associative.
The right-hand side is a naming position: a concrete type or a bare
interface name/instantiation.

- **Concrete RHS — exact-type test**: true when the value's exact
  class or struct *is* `T`. With no inheritance this is a single
  descriptor lookup.
- **Interface RHS — capability probe**: true when the value's
  concrete type has itable fills for that interface — which is exactly
  when the value was satisfaction-boxed somewhere (`k is Serializable`).
- **No flow sensitivity**: `if (x is Shape) { .. }` grants nothing —
  no narrowing, no widening. The keyword answers; it does not admit.
- **Static folds**: when the receiver's static type already answers,
  the result is a compile-time constant, with an always-true/false
  lint. On an exact receiver the member set decides at compile time.
- `is` is total: never traps, yields only `bool`. The same descriptor
  that answers the itable answers the probe — one runtime truth per
  value.

## The iteration protocol — the `[iterable]` marker

A type is iterable when an inherent impl marks a member `[iterable]`:

```rut
use ink::{ Logger };

class CountUp {
    n: i32;
}

impl CountUp {
    pub fn new(n: i32) -> Self { return Self { n: n }; }

    [iterable] pub fn iterate(self, emit: fn(i32) -> bool) {
        for (let i = 1; i <= self.n; i += 1) {
            if (!emit(i)) { return; }
        }
    }
}

entry fn main() {
    let log = Logger.new("t");
    for (let v of CountUp.new(3)) {
        log.info(f"tick {v}");
    }
}
```

```text
tick 1
tick 2
tick 3
```

The contract's shape: `fn <free>(self, emit: fn(E) -> bool)` — the
member's NAME IS FREE (the bracket designates, never the spelling),
and the element type `E` falls out of the marked member's own emit
parameter. `for (v of it) { body }` calls that ONE designated member —
`it.<member>(emit)` — with a synthetic closure: the body runs, then
`emit` returns `true`; `break` returns `false` (stopping the
iteration); `continue` returns `true` immediately; a `return` inside
the body stops the iteration (not the enclosing function). The loop
variable is the closure's parameter — a fresh binding per iteration;
captured enclosing locals are copied by value at the desugar, so
accumulate through a shared cell or a method. The builtin sequences
(`[T]`, `Vec<T>`, `str`, `bytes`) keep their fused index loops and
never reach the marker. An enum value iterates the same way — a
marked member on the enum's impl makes `for (let v of c)` walk
whatever it emits (see [Enums](enums.md)).

## Engine contracts — markers and closed classes

The engine's own contracts stand alone — they are neither interfaces
nor user-visible polymorphism, and they share no machinery with the
member-set law. Two mechanisms, one job each:

**The bracket markers** — `[disposal]`, `[iterable]`, and
`[constructor]` designate an inherent impl member. The parser accepts
any contextual word in the brackets; the checker validates the
engine's CLOSED set, at most one member per contract per class,
inherent-members-only, and each contract's signature. The descriptor
ABI is a designated slot per class — the engine's release path and
the for-of weave read their slot directly, never an itable lookup;
`[constructor]`'s slot is bound at the call site — the call form
`Type(..)` resolves to the designated member:

```rut
impl Db {
    [disposal] fn dispose_db(mut self, cx: DisposalContext) {  // name FREE —
        self.file.close();                                      // the bracket
    }                                                           // designates
}

impl Rows {
    [iterable] fn iterate(self, emit: fn(Row) -> bool) { .. }   // for-of reads
}                                                               // this slot

impl Point {
    [constructor] fn from_xy(x: f32, y: f32) -> Self { .. }     // Point(..)
}                                                               // binds this slot
```

`[disposal]`'s contract: `fn <free>(mut self, cx: DisposalContext)` —
the engine calls it when a value of the type reaches refcount zero
([the Rc heap](rc-heap.md)); the target must be a CONCRETE struct or
class (the row keys the cell's type id). `[iterable]`'s contract is
the iteration protocol above. `[constructor]`'s contract:
`fn <free>(..) -> Self` (or `?Self`) on a class's inherent impl — the
one user-invoked surface: `Type(..)` lowers byte-identically to the
designated member call, the seal rides the method's `pub`, and
newtypes are refused (their positional mint already IS `Name(v)`) —
see [Classes and constructors](classes.md).

**The closed builtin classes** — `Future<T>` and `RunContext` carry
the async protocol. Both are `pub builtin` (the import-gated spelling)
and **closed**: no constructor, no impl lane, no user-callable
members beyond `RunContext`'s two reads — a user type cannot BE a
future or a cx. The walls hold by nominal closure: the member-set law
cannot see engine identity, so closure is the only spelling of
"engine-minted only". The ENGINE weaves without them — async frames,
the minted cx, and the fused `for..of` loops never consult user
scope; source that SPELLS a name resolves it only through
`use core::{ .. }` (a launcher's `f: Future<T>`,
`downcast<Future<..>>`, a `cx` probe in an async body — see
[Async and await](async.md), [host fns and declaration
files](host-fns.md), and [the Rc heap](rc-heap.md)).

## Accepted debts

The model trades two guarantees for its one mechanism, on purpose:

- **Shape-equal satisfaction.** Two types with public members of the
  same shapes both satisfy an interface, whether or not either author
  intended it. There is no opt-in to fence this — the mitigation is
  convention (distinct member shapes, spelled receivers), documented
  here as accepted debt.
- **Wrappers at every primitive boundary.** Because primitives never
  satisfy, every polymorphic hand-off of a primitive spells a wrapper
  construction — `dump(JsonI64(64))`, per call site. The spelling is
  the price of having no auto-insertion; the payback is that a
  wrapper's presence is greppable and its cost is visible.
- **Bare containers never derive.** A `[T]`, a tuple, or a `?T` will
  never grow members; container polymorphism always goes through an
  explicit wrapper family (`JsonArr<T>`, `ArrFlow<T>`, …), never a
  compiler-derived instance.

## Equality

`==` is a builtin operator with no dynamic dispatch: primitives by
value, `str`/`bytes` by content, everything else by cell identity —
see [Rc, dispose, and identity](rc-dispose-identity.md). Field-wise
comparison is an interface-shaped contract implemented per type; it
is not connected to `==`.
