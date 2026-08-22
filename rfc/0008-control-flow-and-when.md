# RFC 0008: Control Flow & `when` — Exhaustive Pattern Expressions

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0006 (enums), RFC 0007 (literals)
- **Supersedes:** RFC 0002 §1.2 (pre-restructure)
- **Part:** B — Language surface

## Summary

rut has the classic structured statements — `if`/`else`, `while`, `for..of`,
indexed `for` — and **one** match construct: `when`, an exhaustive pattern
*expression*. `switch`/`case`/`default` do not exist. See
**`examples/basic/when.rut`** — `when` as an expression (with `else`
required for non-enum scrutinees) and as a statement (void arms).

## 1. Statements

- `if (cond) { .. } else if (..) { .. } else { .. }` — braces required.
- `while (cond) { .. }`.
- `for (let x of expr) { .. }` iterates vecs, fixed arrays, slices and
  strings (chars).
- `for (let i = 0; i < n; i += 1) { .. }` — indexed form.
- `return expr?;` — `expr` required unless the fn returns `void`.
- No `break`/`continue` labels in v1 (plain `break`/`continue` exist for
  loops); no `do..while`.

## 2. `when`

- `when (x) { pattern -> body, ... }` is an **expression**: the first
  matching arm's body produces its value. No fallthrough; exactly one arm
  runs.
- Patterns in v1: enum members, literals (`i32`/`f64`/`bool`/`char`/
  `string`), comma-separated alternatives, and the `else` wildcard.
  Ranges and destructuring are OQ-2.
- **Exhaustiveness is always enforced.** Enum-typed scrutinee: cover every
  member (then `else` is optional) or add `else` — a partial `when` is a
  compile error. Any other type: `else` is mandatory (integers can't be
  enumerated).
- All arms must agree on one type — that is the `when`'s type. Used as a
  statement, that type must be `void`.
- Duplicate patterns and arms made unreachable by earlier ones are compile
  errors.
- Arms use `->` like nothing else except `select` (RFC 0019); expression
  arms are comma-separated, block arms are not.

## Open questions

- OQ-1: destructuring (`const [a, b] = pair`) — omitted from v1; revisit
  with real code.
- OQ-2: `when` pattern set: ranges (`1..9 ->`), destructuring, and
  guards (`x if cond ->`) — none in v1; enum members, literals, and `else`
  only.
