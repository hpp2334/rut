# RFC 0003: Modules & Visibility — Declarations Only

- **Status:** Draft
- **Date:** 2026-08-23
- **Author:** hpp2334
- **Depends on:** RFC 0002 (lexical structure)
- **Supersedes:** RFC 0002 §1.1, §1.3 (pre-restructure)
- **Part:** B — Language surface

## Summary

Module scope contains **declarations only** — C++-style: every statement
lives inside a function, and **loading a module executes nothing**. Visibility
is safe by default (`pub(self)`) with package-scoped forms for libraries.

## 1. Module structure — declarations only

- Allowed at module scope: `import`/`pub`, `let`, `enum`, `dataclass`,
  `trait`, `impl`, `class`, `fn`. Anything else — calls, any
  statement — is a compile error. (`host fn`/`host dataclass` and
  `builtin`/`builtin fn` — signature-only native/engine surfaces —
  exist **only in declaration files** (`.d.rut`), never in `.rut`;
  RFC 0029 §2.)
- **Imports have no `type` marker** (`import { Canvas, newCanvas } from
  "app:gfx"` — not `import { type Canvas }`). rut is fully statically
  typed: the compiler resolves every imported name and knows from usage
  whether it lands in type position (`Canvas` in an annotation) or value
  position (`newCanvas()`), so there is nothing for the user to annotate.
  (Unreferenced imports are a lint, not an error.)
- Module-level `let` initializers must be **load-time expressions**:
  literals, enum members, builtin operators over load-time expressions, dataclass
  literals whose fields are load-time expressions, fixed-array literals
  (`[e1, .., en]` — RFC 0007 §1) whose elements are, and builtin zero
  allocations `Vec<T>(n)` / `Vec.from([..])` with const `n`.
  Calls to user functions are not
  load-time expressions (OQ-1; enforced in the compiler, RFC 0033 §3).
- **Bindings: `let` vs `let mut`.** `let x = e;` binds immutably — no
  reassignment and no field assignment through `x`, and no `mut self`
  method calls on it either: it is a **read-only view of the shared
  object**. `let mut x = e;` may be reassigned and grants write access
  **to the object it holds** — and since every non-primitive is a shared
  cell (RFC 0016 §1), that write is visible to every other handle:
  `mut` is *permission*, never a copy (every assignment path must run
  through a `mut` binding). Methods that assign fields declare
  `mut self`; calling them requires a `let mut` receiver (RFC 0010 §2).
  Parameters may likewise declare `mut name` — the callee's permission
  to mutate **the caller's object** (`fn step(mut p: Point)` moves the
  caller's point; `fn area(p: Point)` promises not to). Diverging —
  mutating a private copy — is `own(p)` at the call site (RFC 0011 §1).
- There is no mutable module state. Program state is constructed in `main`,
  or is held by the embedder and passed back across the boundary — the
  `Opaque` container pattern (RFC 0014, RFC 0023 §2) — **no user code runs
  at load**.
- **Loading a module executes nothing.** The embedder loads, then explicitly
  calls an entry function (conventionally `main`, sync or suspend; worker
  entry points receive transferred args — RFC 0021). Consequences: no
  import side-effect ordering, no load-order bugs, deterministic and cheap
  loads — unlike JS/TS modules.

## 2. Visibility — `pub`, `pub(mod)`, `pub(super)`, `pub(self)`

Modules form a **tree per package** (files in directories; the package root
is the root module). Every declaration carries a visibility:

| Form | Meaning |
|---|---|
| `pub fn ..` | **public** — importable by any rut module (other packages included) |
| `pub(mod) fn ..` | visible everywhere inside this **package's module tree** |
| `pub(super) fn ..` | visible to the **parent module** only |
| `pub(self) fn ..` | module-private — **the default** for unannotated declarations |

- Unannotated = `pub(self)`: safe-by-default privacy; nothing leaks
  unless it says `pub`.
- Applies to all module-scope declarations: `let`, `enum`, `dataclass`,
  `trait`, `impl` (an impl exports with its target type), `class`, `fn`,
  extern declarations included (on `class`, it
  means the *type name* is visible).
- **Class members take the same forms** (RFC 0010 §2): an unannotated
  field or method is module-private, and `pub` (optionally scoped)
  exposes it to the form's audience. There is no `private` keyword —
  the unannotated default *is* the private spelling. Dataclass members
  are always public — no visibility dial (RFC 0009); trait method
  signatures and impl methods are as visible as their trait.
- The conventional entry point is `pub fn main`. The host may also
  call any **`entry fn`** — `entry` is orthogonal to visibility: it
  publishes a function to the *embedder* (RFC 0035 §3), and its
  signature must satisfy the host crossing rule (RFC 0023 §1), checked
  at compile time. `entry` does not combine with `pub` modifiers.
- Visibility is checked at compile time; it has no runtime representation.
- Visibility applies uniformly to every declaration, extern included: only
  `pub` names enter a module's import table, and only `entry` fns
  (plus the conventional `main`) enter the binary's host-entry table. A
  non-exported decl is
  *known* inside its module (callable, type-checkable) but *nameable*
  nowhere else — which is how native libraries hide implementation
  surfaces behind exported ones (RFC 0025 §1).

## Open questions

- OQ-1: load-time expression scope: do module-scope `let` initializers
  ever allow calls to user functions (computed constants)?
  Proposed: no — literal folding and builtin zero-allocs only.
- OQ-2: module/system semantics — URL-like specifiers (`tur:core` today) vs
  paths; how the host intercepts loads (RFC 0001 open question; loader
  policy lives in RFC 0035 §1).
