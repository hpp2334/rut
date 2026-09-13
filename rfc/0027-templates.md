# RFC 0027: Templates — `f"..."` Across the Boundary

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0007 §2 (format literals), RFC 0014 (`Opaque`),
  RFC 0023 (Value boundary)
- **Supersedes:** RFC 0005 §6 (pre-restructure)
- **Part:** E — Host & FFI

## Summary

Problem: `f"..."` normally renders to a `str` at the call site
(RFC 0007 §2 — `concat("a=", str(a))`), which destroys structure. Hosts
that need the structure — **localization**, structured logging, analytics —
must not re-parse strings.

Design: a builtin **`Template`** value type, built **only** by format
literals, chosen by expected type:

- `f"hi {name}, n={n}"` in a `str`-expected position behaves exactly as
  RFC 0007 §2 (desugars to `concat` — zero new cost on the hot path).
- The **same literal** in a `Template`-expected position (host fn
  parameter annotated `Template`, or an explicit `let t: Template =
  f"..."`) compiles to the construction sequence — vec pushes +
  `Opaque.new(..)` boxing, or one internal-native `tmpl` call (RFC 0032 §1.1
  R2; no `tmpl` op): a `Template { parts: Vec<str>,
  args: Vec<Opaque> }` — literal chunks and **boxed values with their
  runtime types** (`Opaque`, RFC 0014), not pre-rendered text.
- `Template` API: `t.str() -> str` renders with rut's own `str()` rules
  (identical output to the `str` path); `t.parts()`, `t.args()`,
  `t.type_id(i)` for hosts/stdlibs doing per-arg formatting. Nothing else
  — like `Opaque`, a template can't do anything until someone renders it.
- At the FFI, a `Template` parameter arrives as `Tmpl { parts: &[StrRef],
  args: &[Value] }` — the host formats per-locale, reorders placeholders,
  or logs structured fields, with **typed** args (`i64` stays `i64`, so
  locale decimal separators are the host's choice, not baked into a
  string).

```rust
.fn_("label", |ctx, t: Tmpl| {                    // host side
    let s = ctx.localize(t.parts(), t.args())?;   // ICU-style formatting
    Ok(Value::Unit)
})
```

Serialization note: `Template` is a storage/carrier type — it crosses
isolate channels like any builtin (RFC 0021 §2), and a worker can return
one where the main VM expects `Template`.
