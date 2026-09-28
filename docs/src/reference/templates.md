# Templates — f"..." across the boundary

Status: this page specifies the template surface; it is not yet wired
into the engine. The format-literal behavior it extends is live
([literals and inference](literals-and-inference.md)).

A format literal normally renders to a `str` at the call site — it
desugars to a string concatenation, and the structure of the interpolated
values is gone. Hosts that need the **structure** — localization,
structured logging, analytics — must not re-parse strings.

`Template` is the answer: a builtin value type built **only** by format
literals, chosen by expected type.

## One literal, two behaviors

| expected type | behavior |
|---|---|
| `str` | the ordinary desugaring to concatenation — zero new cost on the hot path, output identical to the plain formatting path |
| `Template` | the literal compiles to a **template construction**: literal chunks and boxed values, not pre-rendered text |

```rut
pub host fn label(t: Template) -> nil;      // a host fn taking structure

fn work(name: str, n: i32) -> nil {
    let s = f"hi {name}, n={n}";            // str position: rendered text
    let t: Template = f"hi {name}, n={n}";  // Template position: structure
    label(t);
}
```

## The value

A `Template` is `{ parts: [str], args: [opaque] }` — the literal chunks,
and the interpolated values **boxed with their runtime types** through the
erasure box ([opaque — erasure and downcast](opaque.md)). Construction is
an internal native call; there is no user-spellable constructor and no
other way to mint one.

| API | Meaning |
|---|---|
| `t.str() -> str` | render with rut's own formatting rules — byte-identical to the `str` path |
| `t.parts() -> [str]` | the literal chunks |
| `t.args() -> [opaque]` | the boxed arguments, in order |
| `t.type_id(i) -> u32` | argument `i`'s runtime type id |

Nothing else. Like `opaque`, a template cannot do anything until someone
renders it: recover an argument with `opaque.downcast<T>(a)` — `i64` stays
`i64`, so locale decimal separators and number formats are the renderer's
choice, never baked into a pre-rendered string.

## At the boundary

A `Template` parameter arrives as `{ parts, args }` — the chunks and the
typed args:

```rust
rut_vm::register!(hosts, "app::label", (rut_vm::Template,) -> (),
    |vm: &mut rut_vm::interp::Vm, t: rut_vm::Template| {
        // per-locale formatting: reorder placeholders, re-render numbers
        let _ = (t.parts(), t.args());
        Ok(())
    });
```

## Example: locale-aware rendering

```rut
// app.d.rut
pub host fn label(t: Template) -> nil;

pub fn checkout(item: str, price: f64, n: i32) -> nil {
    label(f"{item}: {n} x {price}");
}
```

The literal in `label`'s argument position compiles to the template —
parts `["", ": ", " x ", ""]`, args boxed in order. The host renders per
locale: reorder the placeholders, format `price` with the locale's
decimal separator, pluralize around the `n` arg — `i64`/`f64` arrive as
typed values, so none of that is baked into a pre-rendered string:

```rust
rut_vm::register!(hosts, "app::label", (rut_vm::Template,) -> (),
    |vm: &mut rut_vm::interp::Vm, t: rut_vm::Template| {
        for (chunk, arg) in t.parts().iter().zip(t.args().iter()) {
            // emit chunk; if the locale grammar needs arg i, recover it:
            // let n: i64 = opaque.downcast-like read on the host side
        }
        Ok(())
    });
```

## Laws

- The expected type chooses the behavior; a literal never renders
  "twice". The `str` path and `Template.str()` are byte-identical by
  construction.
- Arguments are retained by the template's boxes; a template keeps its
  args alive as any cell does ([the Rc heap](rc-heap.md)).
- A `Template` crosses the host boundary, and crosses isolate channels
  like any builtin ([workers and channels](workers-and-channels.md)) — a
  worker may return one where the main VM expects `Template`.

