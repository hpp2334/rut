# The rut-fmt Batch — Report

RFC 0030 §7's formatter (`rut fmt`), three phases: the survey
(`docs/fmt-survey.md`, base `9cb3e24`, landed `14459bb`), the landing
(`a406d40` — `crates/rut-fmt` + the CLI subcommand + the `[style]`
manifest table; VERSION untouched 13, no LSP rebuild), this record.
The three design rulings were fixed in the brief — AST reprint +
comment gap-scan, `[style]` in the package manifest, VM-verified pins —
and restated as law in the survey; everything here is what the LANDING
taught that the survey did not know.

## 1. The surface, as landed

```sh
rut fmt src/                 # rewrites *.rut / *.d.rut in place
rut fmt src/main.rut --check # writes nothing; exit 1 + the list
```

- `crates/rut-fmt` is a **pure lib**: `format(src, mode, &Style) ->
  Result<String, Vec<String>>` over `rut-lexer`/`rut-parser`/
  `rut-ast` only — no IO, no manifest, no VM (the Session's layering
  law). The CLI owns the walk, the ancestor-manifest style resolution,
  and the refusal policy.
- A source with diagnostics is **refused, never guessed**: rendered
  diags come back as `Err`, the CLI exits 1 and writes nothing. The
  formatter's input is a parsed-clean module, full stop.
- Style resolution: the nearest ancestor `rut.toml`'s `[style]` block,
  parsed and validated by the fmt crate (`style::from_manifest`);
  no manifest → the defaults (4/100 — the corpus's own conventions).

## 2. The one structural find: SPANS OVERSHOOT

The survey planned positional comment drains anchored on node spans.
The landing's first end-to-end run broke in one shape: **every parser
node span ends at the NEXT token's hi** (the parser absorbs at the
current token, so `span.hi` is one token past the element's last byte
— the same convention the LSP's statement spans already accepted). A
drain anchored on `span.hi` read the wrong LINE whenever the next
token sat on a new line, and trailing comments migrated.

The fix is `content_end(span)`: walk the token stream back from the
last token ending at-or-before `span.hi` while a token ends EXACTLY at
`span.hi` and starts after `span.lo` — those ride the overshoot; the
element's own last token ends strictly inside. Anchored there, every
drain law became exact.

**The when-arm is the pathological case**: its node span is built from
the bare `p.span()` — not `lo.to(p.span())` — so an arm's span is its
NEXT SIBLING's first token, covering none of its own bytes. The arms
loop therefore bounds each arm's begin by the PREVIOUS arm's displaced
`span.lo` (which IS this arm's real first token) and anchors the
trailing drain on the printer's cursor (the body's `expr()` just
boosted it). Both facts are pinned by the corpus idempotence test —
without them, pass 2 moves comments that pass 1 placed.

## 3. The drain laws (what the corpus forced)

The comment queue drains positionally at `begin_el`/`end_el` hooks in
three modes (`Begin::Stmt`/`List`/`Expr`):

- **Stmt** (items, statements, fields, methods, arms): trailing runs
  wait for the element they trail; own-line runs blank-before when the
  source gap is ≥ 2 lines; the element itself blanks on the same
  arithmetic. Blank runs collapse to one; re-formatting is inert.
- **List** (`break_list`'s elements): comments drain own-line, never
  blanks — a 2-line source gap between two ARGUMENTS must not invent a
  blank inside a call.
- **Expr** (expression children): no blank rules; a trailing run
  attaches own-line right there, because no later statement-boundary
  drain can match its line.

Three laws the corpus forced that the survey did not know:

1. **A line comment owns the rest of its line.** A trailing comment
   consumed inside a flat arg list must force the list BROKEN —
   otherwise the printer writes `, );` AFTER the `//` and the parser
   reads the comment as swallowing the closer (`expected ), found
   let`). In inline-flat contexts the drain defers entirely.
2. **Tuples take no trailing comma.** `paren_top` pushes an expression
   frame after every comma — `(a, b,)` is a parse error — while calls,
   arrays and struct literals eat the trailing comma. `break_list`
   carries the distinction; the corpus's `(nil, err)` records caught
   it immediately.
3. **Floats keep their dot.** `9.0f32` printed through Rust's shortest
   `{}` comes out `9f32` — which re-lexes as a SUFFIXED INT and
   refuses to compile (`invalid numeric suffix`). The printer forces
   `.0` whenever the digits have no `.` or exponent.

## 4. The idempotence discipline

`fmt(fmt(x)) == fmt(x)` is tested over the whole corpus, and it is the
test that caught the most: every layout decision must be a pure
function of the text it produces. The pin: comment/blank decisions
re-derive from the formatted source's spans; a decision that consults
anything else (the printer's own line count, a pass counter) drifts on
pass 2. The two-entry tail-blank rule (the element's own blank vs the
comment's blank-before) nearly doubled — the rule fires at most once
per boundary and `begin_el` must run it even when the run queue is
exhausted (an early `return` there silently dropped every blank line
between items).

## 5. The re-paren table, refereed by the VM

The AST has no paren nodes; the printer re-parens a child only where
flat precedence would invert the parse's grouping — the child-level
table mirrors the parser's reduce levels (`||` 3 … `as` 12, unary 13;
same-level rhs parens, unary operands parens when the operand is
binary). The corpus's dump-tree equality proves the SHAPE survives;
because no AST captures paren-ness, the VALUE-level proof is the VM's:
eight probes (precedence matrix, unary/cast chains, bool ladders +
`is`, when-arms, f-string holes, containers, comment storms,
postfix/record returns) run compile → link → `Vm::call` on original
and formatted text and require identical `rt:log` lines. Two probes
taught grammar facts worth recording: `~` is not supported in this
build, and the `try ?` postfix was REMOVED (RFC 0005 §10's `(T, err)`
records replaced it) — the survey's probe list had carried both from
an older reading of the language.

## 6. The gates, as landed

- `cargo test --workspace`: **102 suites / 857 tests, zero failures**
  (base was 97/827 — +5 suites, +30 tests, all in the fmt lanes).
- wasm32 check clean (`rut-lsp-wasm`, `rut-wasm`) — the fmt crate has
  no wasm surface and rides no wasm build.
- The corpus is **conformance, not byte-frozen**: formatting the
  stdlib normalizes hand-aligned comment columns and one-liner bodies;
  the `expected.json` checksums are untouched because formatting is a
  tool, never a gate the workloads ride.
- `demo/*`, `AGENTS.md`, the untracked `scripts/` lanes: foreign work,
  untouched. VERSION stays 13; no LSP rebuild (no surface move — the
  vsix stays 0.2.5).

## 7. What v1 deliberately does not do

- **No method-chain breaking**: a single-argument call on a long
  receiver chain can exceed `max_width` (one documented case in the
  corpus's `app.rut`); chain-breaking is a whole sub-problem deferred.
- **No comment alignment**: the corpus's aligned trailing-comment
  columns normalize to one pad space — deterministic, but the diff on
  first format is honest about it.
- **No doc-comment extraction**: §7's "doc comments kept on
  declarations" remains the AST's future work; today every comment
  survives as a comment, none becomes metadata.
