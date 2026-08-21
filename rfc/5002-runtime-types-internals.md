# RFC 5002: rut — Runtime Types & Dispatch Internals (implementation)

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Implements:** RFC 0002 §10 (reified types) — slot representation, Rc-cell
  & vtable layout, interface call opcode, type test. Language-facing
  contract lives in RFC 0002. Numbering: 5xxx = implementation RFCs.

## 1. Slots

Interpreter registers hold untagged 8-byte slots — the bytecode is typed, so
hot paths carry no tags:

```rust
#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,                       // all int widths, sign/zero-extended
    pub f: f64,                       // f32 payloads widened
    pub b: bool,
    pub c: char,
    pub r: Option<NonNull<Header>>,   // Rc cells, arrays, strings, bytes,
}                                     // interface values, Option/Result of refs
```

Dataclass and bare class values don't fit a slot — they are **inline byte
sequences** spanning consecutive slots (or a stack-frame region), laid out
by the compile-time field table; `Array<Point>` elements are contiguous
inline values with no per-element headers (RFC 0004 §6). The bytecode is
typed, so value copies (`StructCopy { dst, src, size }`) are plain memcpys
with ref-field retain/release emitted by the compiler.

The tagged `Value` enum exists only at the host FFI boundary (RFC 0005 §3).

## 2. Rc cell & vtable layout

```rust
#[repr(C)]
struct RutClass {                    // heap object: an Rc<T> CELL (RFC 0002 §5.3)
    h: Header,                       // rc + type id (RFC 5004 §1)
    vt: *const VTable,               // exact type's vtable — class OR
                                     // dataclass (RFC 0002 §5.1) — set
                                     // at `Rc(v)`
    // class fields follow inline at fixed offsets — the SAME repr-C block
    // as the bare value (RFC 0002 §10.2): boxing adds the prefix, it never
    // re-lays-out. Hosts read the block through StructRef (RFC 0005 §4).
    // No base prefix — no inheritance (RFC 0002 §5.4).
}

#[repr(C)]
struct VTable {
    ty: TypeId,                      // exact runtime type (points into RutType)
    dispose: Option<unsafe fn(*mut RutClass)>,  // RFC 0002 §5.3 destructor
    slots: [CodePtr],                // interface method slots, global ids (§6)
}
```

Interface method ids are assigned **globally per interface instantiation**
at compile time (`Equal<Point>` ≠ `Equal<string>`, RFC 0002 §6); a
class's — or a dataclass's (RFC 0002 §5.1) — vtable fills every slot of
every interface instantiation it declares `implements` (the cell is
minted by `Rc(v)` or by implicit boxing at an
interface widening). A call through an interface is two loads and an
indirect jump:

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

## 3. Type test & upcast

```rust
impl TypeTable {
    /// `is<T>(x)` builtin and host-boundary argument checks. The exact
    /// type lives in the vtable; the descriptor lists the implemented
    /// interfaces — a flat scan, no inheritance chain to walk (RFC 0002 §5.4).
    fn is_a(&self, exact: TypeId, want: TypeId) -> bool {
        if exact == want { return true; }
        self.desc(exact).implements.iter().any(|&i| i == want)
    }
}
```

`is<T>` monomorphizes with `want` as a compile-time constant, so the check
is: load the object's vtable `TypeId`, compare, then (rarely) scan the
descriptor's flat `implements` list. `upcast<T>` needs **no runtime code at
all** — an interface value already *is* the object ref whose header reaches
the vtable — it erases to a plain `MovRef` (often to nothing).

## 4. Type stability & optimization rules

In a **no-JIT** VM, compile-time type knowledge is the only knowledge:
there is no speculation and no deopt to recover what the type checker
doesn't prove. The IR therefore tracks a per-SSA-value **type lattice**:

```
exact concrete  >  interface (satisfies I)  >  opaque (unknown)
```

Rules that bound the cost of the two lattice-lowering features:

1. **Interface values** (RFC 0002 §6): one indirect call per use; fields
   inaccessible; callee unknown (no inlining without evidence). Cost is
   per-call, never per-field — the vtable makes it a single load+jump.
2. **`Opaque` + `downcast`** (RFC 0002 §3.1): erasure is a hole in the
   lattice, but a *scoped* one:
   - the `downcast` check is the refinement — the `is_some()` branch
     re-enters the **exact** lattice position (strictly more information
     than an interface value carries), so downstream code optimizes as if
     nothing was erased;
   - boxes are immutable ⇒ downcast is pure ⇒ **CSE** repeated checks,
     **LICM** invariant ones, cache results forever;
   - a chain of downcasts over the same cell **folds to a `TypeId` switch**
     — the compiler emits one jump table, not N checked boxes.
3. **What the compiler must NOT assume**: the type inside a box at a given
   program point. No speculative devirtualization through `Opaque` (that
   is JIT behavior; rut has no deopt to fall back on).
4. **Guardrails are language-level**: primitives in columnar user stores
   never box (typed arrays); hot shared state uses direct `Rc<State<T>>`
   cells; lints flag `downcast` in loop bodies and `Opaque` crossing
   non-storage function boundaries. The intended shape of a rut program:
   exact types on the hot path, interfaces where polymorphism is real,
   `Opaque` only inside heterogeneous storage.
