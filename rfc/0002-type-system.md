# RFC 0002: rut — Type System

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0001 (pillar decisions)
- **Revised:** grammar re-targeted from Rust-like to TypeScript-like
  (`interface` / `class` / simple `enum`); Option/Result are builtin types
  (no user data-enums); no sugar operators. Later: factories (`constructor`
  renamed to `factory` — construction is a function, `suspend factory`
  allowed), dataclass **and** class are **repr C** (§10.2), layout &
  identity builtins added (§3.2), `box<T>` rejected outright (§2). Latest:
  dataclasses gained **methods + `implements`** (remaining limits: no
  `private` fields, no statics, no `factory`, no `dispose` — §5.1),
  interface **`requires`** shipped (`extends` rejected — §6, OQ-4), and
  `Rc<T>` over dataclasses allowed (§5.3, OQ-17).

## Summary

rut is fully statically typed with **no dynamic typing** — no `any`, no
implicit unions. Types are inferred where possible, annotated where useful,
monomorphized where generic, and — unlike TypeScript — **reified at runtime**:
every value carries a type identity the VM and the host can inspect and check.

The grammar is TypeScript-shaped: `let`/`const`, `fn` (not `function`),
arrow closures, `interface`, `class`, `implements`, simple `enum`, and
exhaustive `when` pattern expressions (no `switch`/`case`). The one
controlled dynamic mechanism is **interface references** (vtable dispatch);
everything else is static.

## 1. Grammar at a glance

See **`examples/basic/grammar-tour.rut`** — dataclass, interface, class
factory type-calls, `Rc<T>`, vtable dispatch, exhaustive `when`, and
format literals in one file. rut source lives in `examples/` (see its
README); RFCs keep only prose and VM-side sketches.

**One default copy regime: by value.** Primitives and **both** user data
types — `dataclass` and `class` — copy on assignment, passing, and return
(shallow copy: ref-typed *fields* copy the handle). References are
**explicit**: `Rc<T>` boxes a class value in a refcounted heap cell, and an
`Rc<T>` handle is ref-copied (`Rc(v)` constructs one). Construction is a
**type-call** everywhere: classes via their `factory` (`Circle(1, 2, 3)`;
`await`-able when the factory is `suspend`), builtin containers/wrappers the
same way (`Array<f32>(n)`, `bytes(n)`, `Rc(v)`, `Rc<Circle>(v)`, `Weak(v)`,
`Opaque(v)`, `Channel<T>()`), dataclasses via literals (`Point { x: 1, y: 2 }`).
There is **no `new` keyword** and **no `factory` on dataclasses** — that
split *is* the dataclass/class distinction.

**No inheritance in v1**: there is no `extends` for classes (and no `super`,
no `override`). Classes are standalone; code sharing happens through
composition (hold a helper object) and free functions; polymorphism happens
through `implements`. Interfaces may not extend interfaces either (§6).

`when` is the only match construct — exhaustive pattern expressions (§1.2);
`switch`/`case`/`default` do not exist. `for..of` iterates arrays and
strings (chars). `enum` is a simple named-int
constant set (§7) — there are **no data-carrying enums** in v1.

**No output builtins**: there is no `console`, no `print`, no implicit
stdout anywhere in the language or the VM. Logging is an imported stdlib
class — `Logger` from `std:log` (RFC 0005 §8) — and where logs go is the
host's decision (embedding default: silent).

### Naming conventions (enforced)

- **Types are PascalCase** — user types and parameterized builtins:
  `Array<T>`, `Option<T>`, `Result<T,E>`, `Rc<T>`, `Weak<T>`, `Opaque`,
  `Future<T>`, `Task<T>`, `Sender<T>`, `Receiver<T>`, `Point`, `Color`,
  `Drawable`.
- Scalars and simple buffers stay lowercase, C-style: `i32`, `u8`, `f32`,
  `bool`, `char`, `string`, `bytes`.
- **Construction is a type-call**: the type name in call position
  constructs — `Circle(1, 2, 3)` (user class `factory`; `await Circle(..)`
  when the factory is `suspend`), `Rc(c)`,
  `Rc<Circle>(c)`, `Weak(b)`, `Opaque(v)`, `Array<f32>(1024)`, `bytes(64)`,
  `Channel<Job>()`. Lowercase types keep lowercase calls (`bytes(64)`).
  Named variants of multi-case builtins stay statics: `Option.some`,
  `Option.none`, `Result.ok`, `Result.err`.
- **Functions and methods are lowercase snake_case** — `unwrap_or(d)`,
  `is<T>(x)`, `upcast<T>(x)`, `downcast<T>(o)`, `select_all(futs)`,
  `spawn_worker(..)`.
- **A trailing `$` marks dispatch-inverted members** (tur convention,
  extended uniformly): anything a *runtime* fires or owns, rather than you
  calling it — event props (`on_click$: Mutation<ClickEvent, void>` or a
  closure prop), lifecycle hooks (`on_mount$`, `before_destroy$`),
  subscription updates (`on_update$`), watch controls (`start$`, `stop$`),
  engine atoms (`viewport_size$`), and `Mutation` handles generally
  (`set_tab$: Mutation<Tab, void>`). Never on functions *you* call, widget
  builders (`click`, not `click$`), or types. `$` is an ordinary identifier
  character (RFC 0006 §2), so the suffix is advisory — the compiler gives
  it no meaning, and the style rule is what keeps it meaningful.

### 1.1 Module structure — declarations only (C++-style)

Module scope contains **declarations only**; every statement lives inside a
function. There are no top-level statements and no load-time code. See
**`examples/basic/module-structure.rut`**.

- Allowed at module scope: `import`/`export`, `const`, `enum`, `dataclass`,
  `interface`, `class`, `fn`, and extern declarations (`extern fn`,
  `extern class` — signature-only, for native surfaces; RFC 0005 §5).
  Anything else — top-level `let`, calls, any
  statement — is a compile error.
- Visibility applies uniformly to every declaration, extern included: only
  `export`ed names enter a module's export table. A non-exported decl is
  *known* inside its module (callable, type-checkable) but *nameable*
  nowhere else — which is how native libraries hide implementation
  surfaces behind exported ones (RFC 0005 §5).
- **Imports have no `type` marker** (`import { Canvas, newCanvas } from
  "app:gfx"` — not `import { type Canvas }`). rut is fully statically
  typed: the compiler resolves every imported name and knows from usage
  whether it lands in type position (`Canvas` in an annotation) or value
  position (`newCanvas()`), so there is nothing for the user to annotate.
  (Unreferenced imports are a lint, not an error.)
- `const` initializers at module scope must be **const-expressions**:
  literals, enum members, builtin operators over const-expressions, dataclass
  literals whose fields are const-expressions, and builtin zero allocations
  `Array<T>(n)` / `bytes(n)` with const `n`. Calls to user functions are not
  const-expressions (OQ-11).
- There is no mutable module state. Program state is constructed in `main`
  or lives in class `static` fields (§5.2), which follow the same
  const-expression initializer rule and are materialized at load — **no user
  code runs at load**.
- **Loading a module executes nothing.** The embedder loads, then explicitly
  calls an entry function (conventionally `main`, sync or suspend; worker
  entry points receive transferred args — RFC 0003 §5). Consequences: no
  import side-effect ordering, no load-order bugs, deterministic and cheap
  loads — unlike JS/TS modules.

### 1.2 `when` — exhaustive pattern expressions

See **`examples/basic/when.rut`** — `when` as an expression (with `else`
required for non-enum scrutinees) and as a statement (void arms).

- `when (x) { pattern -> body, ... }` is an **expression**: the first
  matching arm's body produces its value. No fallthrough; exactly one arm
  runs.
- Patterns in v1: enum members, literals (`i32`/`f64`/`bool`/`char`/
  `string`), comma-separated alternatives, and the `else` wildcard.
  Ranges and destructuring are OQ-13.
- **Exhaustiveness is always enforced.** Enum-typed scrutinee: cover every
  member (then `else` is optional) or add `else` — a partial `when` is a
  compile error. Any other type: `else` is mandatory (integers can't be
  enumerated).
- All arms must agree on one type — that is the `when`'s type. Used as a
  statement, that type must be `void`.
- Duplicate patterns and arms made unreachable by earlier ones are compile
  errors.

### 1.3 Export visibility — `export`, `export(mod)`, `export(super)`, `export(self)`

Modules form a **tree per package** (files in directories; the package root
is the root module). Every module-scope declaration carries a visibility:

| Form | Meaning |
|---|---|
| `export fn ..` | **public** — importable from anywhere (other packages, the host) |
| `export(mod) fn ..` | visible everywhere inside this **package's module tree** |
| `export(super) fn ..` | visible to the **parent module** only |
| `export(self) fn ..` | module-private — **the default** for unannotated declarations |

See **`examples/basic/module-visibility.rut`**.

- Unannotated = `export(self)`: safe-by-default privacy; nothing leaks
  unless it says `export`.
- Applies to all module-scope declarations: `const`, `enum`, `dataclass`,
  `interface`, `class`, `fn` (on `class`, it means the *type name* is
  visible; class members use `private` as in §5.2).
- The entry point must be plain `export fn main` — the host imports it from
  outside the package.
- Visibility is checked at compile time; it has no runtime representation.
  (Class-member `private` is likewise compile-time only.)
- Deeper module-system semantics (specifiers, paths, package layout) are
  RFC 0001 OQ-4 / RFC 0005 territory.

## 2. Primitive types

| Group | Types | Notes |
|-------|-------|-------|
| unsigned int | `u8 u16 u32 u64` | fixed width |
| signed int | `i8 i16 i32 i64` | two's complement |
| float | `f32 f64` | IEEE 754 |
| misc | `bool`, `char` (Unicode scalar, 4 bytes) | |
| heap: text | `string` | immutable, UTF-8, length-prefixed; format literals `f"a={x}"` (§4.1), raw literals `r"..."` |
| heap: binary | `bytes` | mutable, growable byte buffer |
| heap: seq | `Array<T>` | mutable, growable, **unboxed** homogeneous storage |
| user: value | `dataclass D { .. }` | open record — inline value, copied on assignment, methods & interface impls allowed (§5.1); **repr C** (§10.2) |
| user: value | `class C { .. }` | sealed record — also a value type (§5.2), **repr C** (§10.2); `Rc<C>` for refs (§5.3) |

- `Array<f32>` is a flat `Vec<f32>` behind a header — no per-element boxing,
  no per-element refcount traffic (RFC 0004 §6). `Array<Point>` (dataclass or
  class element) is likewise flat: values stored inline, no headers, no
  refcounts.
- **One default copy regime: by value.** Primitives and both user data types
  (`dataclass`, `class`) copy on assignment/passing/return — shallow copies;
  ref-typed *fields* copy the handle (a retain). The reference regime is
  **opt-in and explicit**: `Rc<C>` boxes a class value in a refcounted heap
  cell; Rc handles are ref-copied (§5.3). Builtin heap values (arrays,
  strings, bytes) are handles — they were born shared. There is no `&`/`*`
  syntax anywhere.
- No `null`, no `undefined`. Absence is `Option<T>` (§3).
- **No `box<T>`, no loans.** A loan needs an exclusivity proof; rut has no
  compile-time borrow checker and no runtime aliasing control over inline
  values (they are plain copies) — so loans could alias silently. The
  construct is **rejected, not deferred**. `Rc<T>` is the only boxing; the
  only borrows anywhere are host-side, call-scoped, flag-guarded ones at
  the FFI (RFC 0005 §3).

## 3. Builtin generic types

`Option<T>`, `Result<T, E>` are **builtin (VM-native)** types — they cannot
be user-defined because user enums carry no data (§7). No sugar operators
(`?.`, `??`); the API is explicit snake_case methods plus the `?`
propagation operator. See **`examples/basic/option-result.rut`**.

| `Option<T>` | `Result<T, E>` |
|---|---|
| `is_some() / is_none(): bool` | `is_ok() / is_err(): bool` |
| `value: T` (traps on `None`) | `value: T` (traps on `Err`) |
| `unwrap_or(d: T): T` | `error: E` (traps on `Ok`) |
| `expect(msg: string): T` | `unwrap_or(d: T): T` |

`Array<T>` completes the builtin generic set. `Map<K, V>` / `Set<T>` are
deliberately absent from it: containers are **library types**, provided by
`std:collection` as a decl module + Rust bodies (RFC 0005 §5.1, §8) —
the example spans three files: `examples/host/plugin/my_map.rut`
(decl), `examples/host/my-map.rut` (consumer) +
`examples/host/my_map.rs` (implementation).

### 3.1 `Opaque` — explicit erasure with checked recovery

`Opaque` is a builtin heap-cell type that **forgets the static type and
keeps the runtime type** — the controlled replacement for `any`, built on
reification (§10). Erasure is a *value-level operation you spell out*; the
static type system never loosens.

- `Opaque(v): Opaque` (a type-call, §5.2) — box any value. **Value
  semantics apply at boxing time**: a dataclass or bare class value is
  snapshotted (copied into the cell); ref values (`Array`, `string`, `Rc<T>`)
  store the handle. To share a class by identity, box the handle:
  `Opaque(Rc(c))`.
- `downcast<T>(o: Opaque): Option<T>` — checked recovery. `T` concrete:
  exact runtime-type match. `T` an interface: the same check `is<T>` uses.
  `None` on mismatch — never a trap.
- `is<T>(o)` sees through the box (the boxed value's type).
- **That is the whole API** — apart from the three layout accessors of
  §3.2 (`type_id()`, `size()`, `as_bytes()`): no members, no fields, no
  equality (OQ-18), no `f""` formatting. An `Opaque` can't do anything
  until it is recovered — which is exactly how it differs from gradual
  typing.
- Crossing isolates: allowed iff the boxed value's type is crossable
  (RFC 0003 §5.2), checked at runtime via the type descriptor.
- Also the host-FFI escape hatch for host data with no static shape
  (RFC 0005).

**IR contract** (why `Opaque` does not break optimization — RFC 5002 §4):

1. **Boxes are immutable.** `Opaque(v)` snapshots its value; no operation
   mutates a box in place (store-style writers swap whole cells).
2. **`downcast` is therefore pure** in `(cell, T)`: the compiler may CSE
   repeated downcasts of the same cell, hoist invariant checks out of
   loops, and cache results — validity is permanent.
3. **`downcast` is a refinement point**: within the `is_some()` branch the
   value's lattice position is *exact concrete type* — everything
   downstream optimizes as if the value had never been erased.
4. **Downcast chains over one cell fold to a `TypeId` switch** (jump
   table), the erasure analogue of exhaustive `when`.
5. **`Opaque` is a storage type, not a flow type**: lints flag
   downcasts inside loop bodies and `Opaque` parameters/returns on
   non-storage functions. The hot path keeps concrete types.

See **`examples/basic/opaque.rut`**. The motivating user-land pattern — a
tur-style reactive store (`state` / `source` / `derive` / `mutation` /
`watch` / `Store`, with an `Atom` interface unifying the three atom kinds
for `watch`) — is **`examples/gui/dashboard/reactive.rut`**; the app-shaped
consumer is the `examples/gui/dashboard/` project (`state.rut` declares the
graph, `main.rut` runs it end to end).

### 3.2 Layout & type-identity builtins

```rut
const TID_POINT: u32 = type_id<Point>();
const SZ_POINT: u32  = size_of<Point>();     // 8 — two f32s, repr C
const AL_POINT: u32  = align_of<Point>();    // 4
```

- `type_id<T>(): u32` — identity of the *instantiated* type, unique per VM
  run and stable across modules (`Array<f32>` ≠ `Array<f64>`;
  `Point` = `Point` wherever declared). Comparable only — not a first-class
  type value (OQ-8).
- `size_of<T>(): u32`, `align_of<T>(): u32` — the value representation:
  primitives their width/alignment; dataclass/class their **repr C field
  block** (§10.2) — the number `Array<T>` strides by and value copies copy;
  ref types (`Array`, `string`, `Rc<T>`) report handle size, not payload.
  (`u32`, not a word-sized type: rut has no `usize`, and no rut value or
  buffer may exceed 4 GiB in v1.)
- All three are **compile-time constants** — const-expressions (§1.1),
  folded by HIR from the type table, never executed (RFC 0007 §7).
- `Opaque` mirrors them at runtime: `o.type_id(): u32`, `o.size(): u32`,
  `o.as_bytes(): bytes` (a snapshot of the box's repr-C payload). Together
  they enable **layout-aware heterogeneous storage** — group entries by
  `type_id`, preallocate `size_of`-sized slabs, compare payloads byte-wise —
  while recovery still goes through checked `downcast<T>`. Constructing an
  `Opaque` (or any value) *from* raw bytes is deliberately **not**
  provided: it could forge private fields and class invariants.
- Host struct mirroring runs on the same numbers (`register_struct`
  checks size/align/offsets at startup — RFC 0005 §4).

## 4. Literals & inference

See **`examples/basic/literals.rut`** — numeric suffixes and annotations,
plain/raw/format strings, array and dataclass literals, and the lowercase
factory-type calls (`Array<f32>(1024)`, `bytes(64)`).

- Inference is bidirectional (literal ↔ expected type); a literal without
  context defaults to `i32` / `f64`.
- **Conversions are always explicit function calls** — `i32(x)`, `u8(x)`,
  `f32(x)`; there is no `as` operator (§6.1). No implicit numeric conversions
  at all in v1. Narrowing (`i32`→`u8`, `f64`→`i32`) traps when the value
  doesn't fit; lossy intent is spelled out with `u8.wrap(x)` /
  `i32.trunc(x)` style builtins (OQ-12).

### 4.1 String literals: plain, raw, format

Three literal forms — a plain `"..."` string is always inert (no
interpolation ever happens implicitly, unlike JS template literals):

| Form | Example | Meaning |
|---|---|---|
| plain | `"hi\tname"` | escapes processed (`\t \n \r \\ \" \u{...}`) |
| raw | `r"C:\temp\log.txt"` | **no** escape processing; every byte is literal (C++ `R"(...)"` / Rust `r"..."` style, minus the parens) |
| format | `f"hi {name}, n={n}"` | Rust-style placeholders, evaluated at runtime |

**Format literals** `f"..."`:

- `{ expr }` splices an expression's value (Rust `format!` braces, no `$`):
  identifiers, paths, calls, arithmetic — any expression *except* nested
  string literals (bind one first); the lexer balances braces to find the
  closing `}`.
- `{{` and `}}` are literal braces. Escapes work exactly like plain strings.
- The literal desugars at compile time to `str.concat(...)` of the literal
  chunks and one `str(x)` per placeholder — a builtin per-type formatting
  call, no runtime parsing:

```text
f"a={a} b={f(b())}"   ->   concat("a=", str(a), " b=", str(f(b())))
```

- Formattable types and their `str()` output:

| type | rendering |
|---|---|
| ints | decimal, `-` for negatives |
| `f32`/`f64` | shortest round-trip decimal (`3.5`, `0.1`, `1e300`) |
| `bool` | `true` / `false` |
| `char` | the character itself |
| `string` | contents, verbatim |
| `enum` | member name (`Color.Red` → `"Red"`) |

  Everything else (dataclass/class values, arrays, `Option`/`Result`,
  `bytes`) is a **compile error** inside `f"..."` — preventing accidental
  implementation-detail printing. Use `debug.str(x)` for developer output;
  write a `to_string(): string` method on your class and call it explicitly.
- In a **`Template`-expected position** the same literal builds a
  structured value instead — `Template { parts, args }` with the
  placeholder values boxed (`Opaque`), not rendered — for hosts, l10n, and
  structured logging (RFC 0005 §6). `t.str()` renders identically to the
  concat path; the default `string` behavior above is unchanged.
- Raw + format don't combine in v1 (`rf"..."` is OQ-14).

## 5. Dataclasses, classes & `Rc<T>`

Two user data types, **both value types** — and one explicit reference
wrapper. A dataclass has no `factory` — the literal is its only
construction; a class constructs through factories (§5.2). There is
**no `new` keyword** anywhere in rut.

### 5.1 Dataclasses — open value records

See **`examples/basic/dataclasses.rut`** — literal construction everywhere,
copy-on-assignment, field initializers, free functions over data — and
**`examples/basic/interfaces.rut`** for dataclass `implements` +
`requires` in action.

- **Value semantics**: assignment, argument passing, and returning copy the
  whole value (shallow: ref-typed fields copy the handle + retain; nested
  dataclass fields copy inline). No `new`, no factory — `Name { field: expr, .. }`
  is the only construction, available **everywhere** (module-const
  initializers included, §1.1).
- **All fields public, always.** A dataclass is an open data record —
  `private` in a dataclass is a compile error. Privacy needs construction
  control, which is the class's job.
- Dataclass literals must initialize **every** field (any order, by name);
  fields may declare initializers (`x: f32 = 0`), which the literal may then
  omit.
- **Methods and `implements` are allowed** — the dataclass is no longer
  "pure data". Its body may contain fns: inherent methods and interface
  impls, declared and dispatched exactly like class methods (`this`
  included):

  ```rut
  dataclass Point implements Hashable, Equal<Point> {
      x: f32;
      y: f32;
      fn hash(): u64 { .. }
      fn eq(other: Point): bool {
          return other.x == this.x && other.y == this.y;
      }
  }
  ```

  Calls on a concrete `Point` are direct (§6); the interface-ref form
  boxes — below. The limits on a dataclass, exhaustively: **no `private`
  fields** (above), **no `static` members**, **no `factory`** (the
  literal is the only construction — that split *is* the
  dataclass/class distinction), and **no `dispose()`** (a value that is
  copied around has no single death to hook). Everything else
  class-shaped is allowed. Free functions over data remain the default
  idiom; methods are for interface impls and tight helpers.
- **Boxing for interface refs.** A dataclass is still a bare inline
  value; an interface value *is* an Rc cell reference (§5.3). A bare
  dataclass widens to an interface type by **implicit boxing** at the
  widening site — exactly the bare-class rule of §5.3 — and cells
  minted this way (or by `Rc(p)`) carry the dataclass's impl vtable
  (RFC 5002 §2). Layout never changes: methods and impl tables add
  **nothing** to `size_of(D)`; the repr-C field block is copied into
  the cell as-is.
- **Representation: no header, no refcount** (RFC 0004 §1). A dataclass is
  its fields back-to-back, inline wherever it lives: registers/stack for
  locals, inline in class fields and Rc cells, inline in `Array<Point>`
  elements (unboxed and contiguous — a flat buffer of pairs). RC and the
  cycle collector only see a dataclass's ref-typed fields, via the
  compile-time field table.
- `Option<Point>` / `Result<Point, E>` hold the value inline in the payload.
- Copy cost is `size_of(D)` bytes — dataclasses are for small data (points,
  rects, colors, configs). Any size is allowed; the compiler warns past a
  threshold (OQ-16).

### 5.2 Classes — sealed value records with factories

See **`examples/basic/classes.rut`** — factory type-calls, the class-private
`Self { .. }` literal, `suspend factory`, `private factory` sealing, statics.

- **Also a value type.** A bare `Circle` copies on assignment/passing/return
  exactly like a dataclass — inline, no header, no refcount. What a class
  *adds* over a dataclass is **sealing**: private fields, factory-only
  construction, `static` members, and `dispose()` (§5.3). (`implements`
  is no longer class-only — §5.1.)
- **Construction is a type-call**: `Circle(1, 2, 3)` runs the class's
  `factory`. No `new` keyword exists, and there is no outside literal for
  a class — construction always flows through a factory.
- **`factory` is just a function** — implicitly static (no `this`), ordinary
  params, ordinary body, a `return`. It builds the instance with the
  **class-private `Self { field: expr, .. }` literal** (the class name
  spells it inside the body too). The literal must initialize every field
  without an initializer; field initializers run for omitted fields.
  Private fields are settable in the literal — inside the class body only.
- The return type defaults to `Self` and may be declared otherwise, so
  "try" constructors are just factories:
  `factory parse(s: string): Option<Version>`; a dispose class's factory
  may hand out `Rc<Self>` directly (§5.3).
- **`suspend factory` is allowed** — same function, `await` in the body:
  `Circle(..)` then returns `Future<Circle>` and callers write
  `await Circle(..)` (cold future, RFC 0003 §2). No partially constructed
  instance ever exists across an `await` — the `Self { .. }` literal is an
  ordinary expression, and locals live in the coroutine frame.
- **No factory declared + every field has an initializer → default no-arg
  factory** (`Sprite()`). A field without an initializer and no factory
  makes the class unconstructible outside its own body.
- **`private factory` seals** the class: the type-call is legal only inside
  the class body. The named-factory pattern is an opt-in on top —
  `static fn issue(): AuthToken { return AuthToken("..") }` validates,
  caches, or registers, and outside code cannot bypass it.
- `static` members live in the class's module-static slot table. Static
  initializers must be const-expressions (§1.1) and are materialized at
  load — factories and methods are the only places to run logic.
- No `get`/`set` accessor syntax anywhere — a computed property is just a
  method (`c.count()`), and a settable one takes an argument
  (`c.set_count(n)`). One member kind, one call convention, no hidden code
  behind field-access syntax.

### 5.3 `Rc<T>` — explicit references

Want shared, mutable, or long-lived identity? Say so — wrap the value.
See **`examples/basic/rc-and-dispose.rut`** (aliasing vs value copies) and
**`examples/memory/temp-file.rut`** (dispose at rc 0).

- `Rc<T>` is a builtin generic wrapping a class **or dataclass** `T` in a
  heap cell: `Header + vtable + fields inline` (RFC 0004 §1). Creation:
  `Rc(v)` (type inferred) or `Rc<Circle>(v)`. `Rc(p)` over a dataclass is
  how a value record gets shared identity — and what interface impls on
  dataclasses box into (§5.1).
- **Copy = ref-copy.** Assignment/passing/returning an `Rc<T>` copies the
  handle (`rc++`) — JS-like aliasing, but always visible in the type.
- **Access goes through**: `b.x` reads the box's field; `b.x = 1` and
  `b.move_by(..)` mutate the shared box. There is no explicit deref syntax.
- **`dispose()` classes must live behind `Rc`.** If a class declares
  `dispose()`, using it as a bare value is a compile error (a copyable value
  has no single death). Construct then box — `Rc(TempFile("tmp.dat"))` —
  or hand out `Rc` from a factory. When an Rc cell's count hits 0,
  `dispose()` runs, then fields are released in order — deterministic
  destruction.
- **Interfaces need the box.** An interface value *is* an Rc cell reference
  (with its vtable — §10.2). `Rc<Circle>` widens implicitly to `Drawable`;
  a **bare** class value — or, identically, a **bare dataclass** (§5.1) —
  converts to an interface type by **implicit boxing** (allocates the Rc
  cell, vtable from the type's impl table) at the widening site — the one
  place rut heap-allocates without `rc` spelled out. `Array<Circle>` stays
  inline; `Array<Rc<Circle>>` and `Array<Drawable>` store cell pointers.
- **Weak refs**: `Weak(b)` creates a `Weak<C>` that does not keep the cell
  alive (RFC 0004 §4). Rc cells are also what the cycle collector walks
  (RFC 0004 §5).

### 5.4 No inheritance

- **No `extends` for classes** — v1 has no inheritance at all: no base-class
  constructors (`super(..)`), no method overriding (`override`), no `super.m()`
  calls, no `protected`.
- Classes are standalone types. Code sharing is composition (hold a helper
  object or dataclass in a field) or free functions; subtyping is only
  class→interface via `implements` (§6).
- Layout stays trivial: fields at fixed offsets — inline in bare values and
  Rc cells alike, no prefix layout, no fat pointers, and every object has
  exactly one concrete class forever. This keeps `is<T>` a single descriptor
  check (§10.3) and the RC/cycle-collector walk flat (RFC 0004 §5).
- If real code demands it later, inheritance returns as a separate RFC —
  the vtable design (§10.2) already reserves room for it (OQ-5).

### 5.5 Dispatch: direct by default, vtable at interfaces

The static type decides the opcode:

```text
length(p)   // free fn        ->  call length$Point        (direct, always)
c.area()    // c: Circle      ->  call Circle$area         (direct, always)
b.area()    // b: Rc<Circle>  ->  call Circle$area         (direct — T known)
d.draw(g)   // d: Drawable    ->  calliface d, slot=3      (vtable load + call)
```

Calls on concrete dataclass/class/Rc types are always direct — the exact
type is statically known and can never change (no inheritance). Only values
whose static type is an interface dispatch through the vtable. `final` is
meaningless in v1 (nothing can override); OQ-5 tracks whether inheritance
ever returns.

## 6. Interfaces

See **`examples/basic/interfaces.rut`** — multiple `implements`, a composed
`Widget` interface, `requires`, dataclass implementors, interface-typed
arrays.

- Interfaces declare **plain methods only** — no fields, no properties of
  any kind (there is no `get`/`set` syntax in rut at all). Anything that
  reads like a property becomes a method: `x.count()` in the interface,
  implemented as a method on the class. Rationale: fields have no single
  offset rule under multiple `implements`, and an interface slot is always a
  code pointer invoked with an explicit `(...)` — one member kind, no
  call-vs-load ambiguity at the vtable boundary. (`d.x` where `d` is
  interface-typed is a compile error.)
- **Intersection types (`A & B`) are never supported** — not deferred, not
  planned: heterogeneous needs compose an interface that declares both
  method sets (`interface Widget` in the example). This is a design
  principle, not a v1 limitation.
- `implements` is **nominal and declared** — structural ("duck") conformity
  does not satisfy an interface. This keeps runtime type identity exact
  (§10) and casts cheap.
- **`requires` — an admission constraint, not subtyping.** An interface
  may require others: `interface Hashable requires Equal<Self> { .. }`.
  To implement `Hashable`, a type's `implements` list must **also** list
  `Equal<Self>` with `Self` bound to the implementor — `dataclass Point
  implements Hashable, Equal<Point>`; `Equal<SomeOtherType>` does not
  satisfy it. Requirements are transitive (`A requires B`, `B requires C`
  ⇒ `A` needs `C` too), cycles in the requires-graph are a link error,
  and registered builtin impls satisfy requirements like any other impl
  (RFC 0005 §5.1). What `requires` deliberately is **not** (this is why
  interface `extends` was rejected, OQ-4): no member inheritance —
  `Hashable` declares only `hash`, and `eq` is reachable only through
  an `Equal<T>` ref; no subtyping — a `Hashable` ref does not widen to
  an `Equal<T>` ref; **vtables stay flat** — one interface, one vtable,
  §10.3 stays a single scan. The requires-graph is a compile-time walk
  over the implements list, never a runtime dispatch.
- Interfaces are implemented **by classes and dataclasses** (§5.1).
  An interface type is never a value's exact type; every interface
  value points at an Rc cell whose exact class or dataclass it carries
  (RFC 5002 §2). Generic interfaces exist — `Equal<T>` above is the
  canonical example — and each instantiation has its own vtable slots
  (`Equal<Point>` ≠ `Equal<string>`, RFC 5002 §2).
- Interface-typed values are the **only** dynamic dispatch in rut: a
  reference plus a vtable lookup per call. No `dyn`-typed variables, no
  `any`, no dynamic field access, no dynamic `this`.
- Heterogeneous collections are interface-typed arrays:
  `Array<Drawable>` — the replacement for both TS unions and the data-enums
  rut deliberately dropped.

### 6.1 Type tests & upcasts — builtin functions, not keywords

There are **no cast keywords** (`as`, `as?`) and no `is` operator. The two
type-directed operations are prelude builtin generics. See
**`examples/basic/type-tests.rut`**.

- `is<T>(x): bool` — runtime test, true when `x`'s exact class (or
  dataclass — §5.1) implements `T` (or `T` is the exact type itself).
  With no inheritance this is a single descriptor lookup — exact
  `TypeId` compare plus one flat `implements` scan. Machinery in §10.3.
- `upcast<T>(x): T` — explicit widening to an interface.
  **Compile-time checked** (`x`'s class must declare `implements T` —
  otherwise a compile error, never a runtime failure) and **zero runtime
  cost**: an interface value *is* the object ref, so `upcast` erases to a
  plain move (§10.3).
- Implicit widening already happens on assignment/argument passing
  (`blit_all(g, [c])` passes a `Circle` as `Drawable`); `upcast` is the
  explicit form for disambiguation and for making the widening greppable.
- **Interface values cannot be downcast.** An interface value is used
  through its interface methods — if you need `Circle`-specific behavior
  behind a `Drawable`, put that behavior in the interface. Recovery of an
  erased value exists only through `Opaque` + `downcast<T>` (§3.1):
  erasure is explicit, so nothing dynamic ever flows through interface
  types.
- Numeric and enum conversions follow the same principle: named function
  calls, not operators (§4, §7).

## 7. Enums (simple)

`enum Color { Red, Green, Blue }` — 0, 1, 2; `enum Direction { Up = 1, Down,
Left, Right }` — 1, 2, 3, 4 (explicit initializers allowed; members are the
enum's values).

- An enum is a distinct named type over `i32` constants. Members are the
  enum's values; no data payloads, no methods, no computed members.
- `when` over an enum must be exhaustive: every member covered, or an
  explicit `else` arm — anything else is a compile error (§1.2).
- Enums cross the host boundary as their runtime identity + `i32`
  (RFC 0005 §3); the host can register its own enum types.
- Where TS would use a union of literals (`"left" | "right"`), rut uses an
  enum; where TS would use a union of *shapes*, rut uses an interface.
- Enum ↔ `i32` goes through per-enum builtins — `Color.to_int(c): i32` and
  `Color.from_int(i: i32): Option<Color>` (`None` on unknown values) — not a
  cast operator (§6.1).

## 8. Integer semantics

- Overflow in `+ - * <<` **traps** in debug and release by default.
  Wrapping escapes: `&+ &- &* &<<` (compound: `&+= &-= &*= &<<=`);
  saturating: `x.saturating_add(y)`.
- Division by zero traps; `int / int` is integer division.
- Mixed-width arithmetic: both operands must have equal width (cast first).

## 9. Functions & closures

See **`examples/basic/closures-generics.rut`** — arrows (single-expr and
block), and a generic `first<T>` monomorphized to two instantiations.

- Closures capture **by reference** to the enclosing bindings (JS-like),
  which RC keeps safe within a VM. Closures are not transferable across
  isolates in v1 (RFC 0003 §5.2).
- Generics **monomorphize at compile time**: each instantiation emits its own
  typed opcodes (`arr.get<f32>` vs `arr.get<Handle>`). Instantiations whose
  bodies are descriptor-independent share one generic body at load time
  (RFC 0001 §Execution model); the compiler reports instantiation counts.
- Generic parameters are unconstrained in v1 — no `T: Iface` bounds on
  user generics (OQ-7): you cannot call interface methods on a bare `T`.
  Pass values in, or take an interface-typed parameter instead of a
  generic. **The one exception**: generic params of **extern class
  declarations** may carry interface bounds (`MyMap<K: Hashable, V>`,
  RFC 0005 §5) — admission-only syntax: it constrains which
  instantiations compile (closing over `requires`), grants no method
  calls on bare `K`, and adds no IR. A user-class bound would be pure
  forwarding anyway (without dispatch, a body could only pass `K` onward
  to extern positions) — deferred with OQ-7.

## 10. Reified runtime types

Every value's type is available at runtime as a `RutType` descriptor
(kind + width + for composites: field tables / vtable + for generics: the
instantiation descriptors). Slots are untagged 8-byte values — bytecode is
typed, so hot paths carry no tags; dataclass/bare-class values are inline
byte sequences spanning slots. Layouts and opcodes (`StructCopy`,
`CallIface`, the `is_a` type test) are specified with Rust sketches in
RFC 5002.

### 10.1 Where reification is load-bearing

1. **Host boundary** — native fns declare parameter types once; the VM checks
   every call; no bridge-side coercion code (RFC 0005 §3).
2. **Opaque host types** — `extern class Source<T>` carries its
   instantiation identity inside the handle: `store.get(src)` is checked
   statically *and* the handle knows its `T` (fixes tur's erased
   `Source<T>`).
3. **Type tests & upcasts** — `is<T>` / `upcast<T>` builtins (§6.1); host
   fns declaring interface-typed parameters get their arguments checked by
   the same machinery.
4. **Heterogeneous collections** — vtable dispatch (§6) needs the exact type
   reachable from every object header.
5. **Debugging** — `debug.type_of(x)`, stack traces, formatter output.
6. **Serialization** — stdlib walkers traverse `RutType` descriptors.
7. **`Opaque` recovery** — `downcast<T>` (§3.1) checks the boxed cell's
   `TypeId`; `Opaque(v)` stamps it. Erasure without reification would be
   `any`; with reification it is a checked box.

`RutType` descriptors are *not* first-class script values in v1 (OQ-8).

### 10.2 Value layout — repr C

Both `dataclass` and `class` values are **C-layout structs** — one layout
rule, no exceptions:

- Fields in declaration order, natural C alignment, struct size padded to
  its alignment. No hidden header, no tag, no vtable pointer inline.
- `private`, `implements`, `dispose`, `static`, and generic parameters add
  **nothing** to the block — layout depends only on the field list.
- The block is what assignment copies, what `Array<T>` stores inline
  (RFC 0004 §6), what `size_of<T>()`/`align_of<T>()` report (§3.2), and
  what the host mirrors with `#[repr(C)]` structs (RFC 0005 §4) — rut↔host
  struct interop is pointer identity, not field-by-field conversion.
- An `Rc<C>` cell wraps the *same* block behind `Header + vtable pointer`
  (RFC 5002 §2): boxing never re-lays-out fields.
- `enum`, `Option`, `Result` are *not* repr C (tagged layouts, RFC 5004
  §1) and never cross the FFI as structs — pass their payload fields.

## Open questions

- OQ-1 default int width (`i32` proposed).
- OQ-2 `f16`/`bf16` for GPU-facing arrays.
- OQ-3 ~~`map<K,V>` / `set<T>` builtin vs library~~ — **resolved:
  library.** `Map<K, V>` / `Set<T>` live in `std:collection` as a
  decl module + Rust bodies (RFC 0005 §5, §5.1); the VM stays
  container-free. The `examples/host/` trio (decl, consumer, impl)
  shows an embedder doing the same.
- OQ-4 ~~interface `extends` (interface hierarchies)~~ — **resolved:
  rejected**; `requires` shipped instead (§6) — an implementor-side
  admission constraint: flat vtables, no member inheritance, no
  interface-to-interface widening.
- OQ-5 class inheritance (`extends`/`super`/`override`) — removed from v1
  (§5.4); reintroduce only if composition proves insufficient, as a separate
  RFC.
- OQ-6 ~~constructor parameter properties~~ — **obsolete twice over**:
  factories are plain functions (params are params), and the
  `Self { field: name }` literal makes the param→field mapping explicit.
- OQ-7 generic bounds `T: Iface` (would unlock static dispatch on
  bare `T` without interface refs) — still deferred for **user**
  generics; the **admission-only** form shipped scoped to extern decls
  (§9, RFC 0005 §5), which needs no dispatch and adds no IR.
- OQ-8 first-class type values (`type_of(x)` as a manipulable value).
- OQ-9 string indexing: byte-index + helpers proposal stands
  (`for (const c of s)` is the blessed iteration).
- OQ-10 destructuring (`const [a, b] = pair`) — omitted from v1; revisit
  with real code.
- OQ-11 const-expression scope (§1.1): do module-scope `const` /
  `static` initializers ever allow calls to user functions (computed
  constants)? Proposed: no — literal folding and builtin zero-allocs only.
- OQ-12 naming of lossy numeric conversions (§4): `u8.wrap(x)` vs
  `u8.truncate(x)` vs `wrapTo<u8>(x)`.
- OQ-13 `when` pattern set (§1.2): ranges (`1..9 ->`), destructuring, and
  guards (`x if cond ->`) — none in v1; enum members, literals, and `else`
  only.
- OQ-14 `rf"..."` (raw + format combination) and precision/width specifiers
  in format placeholders (`{x:.2}`) — deferred.
- OQ-15 ~~dataclass interface implementation~~ — **resolved: shipped**
  (§5.1). The boxing is the same implicit-boxing rule as bare classes
  (§5.3) — one mechanism, not two.
- OQ-16 dataclass size warning threshold (compiler warns when a dataclass
  grows past N bytes, since copies are `size_of` memcpys).
- OQ-17 ~~`rc<dataclass>`~~ — **resolved: allowed** (§5.3). Interface
  impls on dataclasses (§5.1) made dataclass cells mandatory anyway.
- OQ-18 `Opaque` extras: the `type_id()` accessor **shipped** (§3.2, with
  `size()` and `as_bytes()`); content equality/hashing for identity use in
  collections remains open — `std:collection`'s `Equal<T>` / `Hashable`
  interfaces (RFC 0005 §5.1) are the shipped answer for user types.
- OQ-19 derived-atom caching (deps/version tracking for `derive`-style
  user libraries) — pure recompute-on-read is the v1-simple contract;
  decide whether the stdlib ships an invalidation-based cache.
