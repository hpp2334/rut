# The host boundary

rut exists to be embedded. The host — a Rust program — owns a VM,
declares its API once in rut source, binds the implementations with
typed Rust closures, and drives everything. The design rule for the
whole boundary: **the type system does the checking, not your bridge
code.** There is no argument re-parsing, no `as number`, no
"trust-me" coercion — a value crossing either direction is checked
against a reified runtime type, and a mismatch is a loud error naming
both sides.

## The embedding model

```rust
let mut session = Session::new();
mount_std_core(&mut session);
session.register_module("app:gfx", lower_decl_module(GFX_DECL, "gfx.d.rut")?);

let mut hosts = HostRegistry::new();
hosts.register::<_, (i32, i32), Opaque<Canvas>, _>(
    "app:gfx::newCanvas",
    |_vm, w: i32, h: i32| Ok(Canvas::new(w, h)),
);

// the join: every declared row must have a binding, and vice versa —
// verified BEFORE any script runs
hosts.verify_against(&session.expected_host_fns())?;

let mut vm = Vm::new(program, &limits, hooks, hosts)?;
vm.call::<_, ()>("main", ())?;      // an entry point
vm.run_ready()?;                    // drive async work to idle
```

The declaration file (a *host package*) spells the surface in rut:

```rut
pub host fn newCanvas(w: i32, h: i32) -> opaque;
pub host fn circle(h: opaque, x: f32, y: f32, r: f32) -> nil;
```

and the load-time join rejects three wiring bugs as panics — an
embedder mistake, never a script diagnostic: a declared row with no
binding, a binding with no declaration, and signature drift. "Mount
what you bind": an embedder that mounts a package declares its rows
and must bind them.

## The crossing set

What may cross is a **compile-time property of the surface**, not a
runtime negotiation:

- **Parameters**: the primitives, `str`, `bytes`, and `opaque`.
- **Returns**: the same set *plus* the answer optionals `?str`,
  `?bytes`, `?opaque` — `Option<String>`, `Option<Vec<u8>>`, and the
  opaque handles mint the nullable box, with `None` as the flat `nil`.
- **Tuples** cross field by field, which makes the error convention —
  `(value, err)` — a first-class entry answer. The two channels stay
  distinct by law: a *returned* `err` is data the host reads and acts
  on; a *panic* is drift and arrives on the trap channel, never filling
  an err field.

Everything else — user structs and classes, `Vec`s, interface-typed values,
closures — stays inside the VM. A declaration that violates the set is
a compile error at the declaration, not a failed call at 2 a.m. A
polymorphic crossing seals its value in an erasure box
(`opaque(v)` at the call, `opaque.downcast<T>` after), checked, never
silent — see [reified types](reified-types.md).

## The currency is typed Rust

On the Rust side the boundary speaks ordinary Rust types. Each type
declares the rut type it binds against and converts checked:

- **Borrow, don't copy.** `&str` and `&[u8]` parameters read the VM's
  buffers zero-copy. The borrow is call-scoped — the type system makes
  smuggling it past the return a compile error.
- **Copy on purpose.** Owned `String`/`Vec<u8>` parameters are the
  explicit "I keep this data" choice.
- **`Opaque<T>`** is the typed view of a host handle: an 8-byte store
  handle the host holds and passes back; the payload stays host-owned
  (`Box<dyn Any>` plus an optional finalize hook).

**Borrow guards** make the zero-copy views safe while script runs: a
borrowed object is flagged, and script-side mutation through a
re-entrant call traps with `borrowed by host` instead of racing. Guards
clear on return. This is sound forever because the heap never moves
objects (see [memory](memory.md)).

## Wrapping host state: the class-in-rut pattern

There is no "host class". Native state crosses as an `opaque` handle
and a rut class wraps it — ordinary source the compiler can see and
optimize around:

```rut
class Canvas {
    h: opaque;
    fn circle(mut self, x: f32, y: f32, r: f32) -> nil {
        canvas_circle(self.h, x, y, r);
    }
    fn hits(self) -> i32 { return canvas_hits(self.h); }
}
```

Every method is exactly one host call — the same crossing a magic host
class would have paid — but the wrapper can also carry impl blocks,
validation, and convenience the native side never needed to know about.
The payoff for keeping the boundary this small: host resources die
deterministically. When the wrapper's last reference goes, the handle's
refcount hits zero and the payload's Rust `Drop` runs *at that point* —
sockets and textures close on script schedule, not at some future
collection.

## Host functions are rows, dispatched by slot

Each declared row gets a stable slot id at compile time; calls compile
to slot dispatch, so names are binding-time labels only — renaming a
Rust-side binding target fails the join instead of silently rebinding.
The surface grammar is small:

```rut
host fn name(params) -> T;        // concrete signature; no generics —
                                  // a generic has no shape to check
host struct Name { fields };      // flat record of crossing fields
```

`builtin` declarations (the engine's own fns and classes —
`str` methods, `Weak`, `Future`) are engine surface: the embedder
cannot spell them, and the engine's contracts are the bracket markers
and the closed classes — no interface machinery behind them. Async rows (`pub host async fn`) are declared like any row and
expand into a small row family the driving loop uses — the embedder
binds them with one closure per future; see
[the async model](async-model.md).

## Re-entrancy, traps, budgets

- A native fn receives a context that may call **back into rut**
  (`vm.call`) — nested calls run under the same budget on a fresh
  frame stack.
- Native code runs **outside the op budget**: the host is trusted to be
  fast, or to hand back a future instead of blocking. A trap raised
  inside a native fn propagates to the host as an `Err(Trap)` with the
  native frame visible in the backtrace.
- Traps are catchable **only at the boundary** — script never catches
  one. That is what keeps panics loud and state uncorrupted.

## What was deliberately left out

No C ABI. There is no `register_struct`, no `size_of`, no pointer
mirroring of record layouts: records are slot arrays internal to the
VM, and a host sees them only through the checked crossing. The few
bytes you give up to the checked boundary buy the property that makes
embedding pleasant — every mismatch is a named, typed, load-time
diagnostic instead of a runtime mystery.

The full details live in the reference: [embedding and native
modules](../reference/embedding.md), [the value boundary and
borrows](../reference/value-boundary.md), [host fns and declaration
files](../reference/host-fns.md), and the withdrawn C-struct
experiment recorded at [repr(C) interop](../reference/repr-c.md).
