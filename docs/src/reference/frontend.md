# The frontend

The frontend turns source text into a checked-syntax tree: source (UTF-8) →
[lexer](#the-lexer) → tokens → [parser](#the-parser) → AST, with one
diagnostics model flowing out of every stage. It has no type knowledge —
resolution and checking begin in [the compiler pipeline](compiler.md).

The implementation is three crates: `rut-lexer`
(`span`, `token`, `lexer`, `diag`), `rut-ast` (the arena tree and its
dumper), and `rut-parser` (the frame machine and its expression engine).
Everything is `no_std`-clean and wasm-compatible, so the same code runs in
the CLI, the LSP, the formatter, and the browser playground.

## The contract

Four invariants hold for any input, hostile included:

| | invariant | mechanism |
|---|---|---|
| C1 | **Flat AST** — nodes are records in one arena; children are `NodeId`s, never boxed pointers, built bottom-up. Drop/clone are flat `Vec` ops. | [AST](#the-ast) |
| C2 | **No native recursion** — an explicit frame stack handles structure; expressions use an iterative operator/operand engine. Host stack use is constant regardless of input. | [parser](#the-parser) |
| C3 | **Depth budget, not stack exhaustion** — nesting over the budget is a normal diagnostic, never a host crash. | [budgets](#nesting-budgets) |
| C4 | **Lookahead discipline** — local decisions peek at most 4 tokens; the two decisions that need more use read-only balancing scans. The cursor is monotone non-decreasing for the whole parse, recovery included — there is no checkpoint/rollback. | [lookahead](#lookahead) |

The lexer and parser are pure functions —
`lex(src) -> (Vec<Token>, Vec<Diag>)` and `parse(src, mode) -> (Ast, Vec<Diag>)` —
so they are fuzzable and reusable in an LSP or formatter with no VM present.

## The lexer

- One flat `Tok` enum for literals, punctuation, and operators. Keywords are
  ordinary `Ident`s; the parser matches them by interner name. **Reserved
  words are rejected by the lexer** with a `rut does not have X` message
  (`new`, `switch`, `case`, `?.`, `??`, `any`, …).
- Maximal munch with an explicit longest-match table; `/` vs `//` vs `/*`
  resolves with one lookahead. Compound assignment operators are one token
  each (`+=`, `<<=`, `&&=`, …), so there is no munch ambiguity.
- Numeric literals: `0x`/`0b`/`0o`, `_` separators, and per-width suffixes
  (`u8..u64`, `i8..i64`, `f32`/`f64`). Unsuffixed integers default to
  `i32`, unsuffixed floats to `f32`.
- Comments are dropped (doc comments are re-attached by the formatter's
  gap-scan, below).

### Format strings

`f"..."` is the one construct the lexer must understand structurally,
because placeholders hold expressions, not strings:

```rust
struct FStrTok { parts: Vec<FPart> }
enum FPart {
    Lit(String),      // decoded like a plain string; {{ }} -> { }
    Hole(Vec<Token>), // a FULLY LEXED token stream, `}`-terminated
}
```

The lexer enters hole mode after `{` (not `{{`), balances braces, and lexes
normally until depth 0. Holes are re-lexed, not stored as text, so
placeholder expressions carry real spans inside the literal's span. A hole
may contain any expression except a string literal — a quote inside a hole
is a lex error with a *bind it to a name first* note. See
[Templates](templates.md) for the surface semantics.

Raw strings `r"..."` carry everything up to the closing `"` literally;
`\"` inside a raw string does not close it.

### Nesting budgets

Two budgets make C3 a law:

- `NEST_MAX = 1024` — the lexer's bracket depth budget (`rut-lexer/span.rs`).
  Exceeding it is one clean diagnostic at the offending opener.
- `EXPR_MAX = 64` — the parser's expression-nesting budget. The frame
  machine itself has no host-stack cost, but downstream recursive walks
  (AST dump, typecheck) do; the cap keeps the trees they visit shallow.

Both produce the same diagnostic — *nesting too deep* — and neither can
overflow the host stack.

## The parser

One loop over the token slice drives two mechanisms:

- A **frame stack** handles structure — one frame kind per grammar rule
  (`Module`, `Item`, `Block`, `TypeArgList`, `Pattern`, `Arm`, `Impl`, …).
  A rule that needs a child pushes a frame; a completed frame pops and hands
  its `NodeId` to the parent.
- An embedded **Pratt engine** handles expressions — an operator stack plus
  an operand stack of `NodeId`s. Binary and assignment steps push an
  operator; reductions pop operands and push one node, bottom-up.

There is no grammar generator and no left-recursion handling. Binding
powers, loosest to tightest:

| prec | assoc | operators |
|---|---|---|
| 1 | right | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` `<<=` `>>=` `&&=` `\|\|=` (target must be a path or index) |
| 2 | – | `=>` lambda bodies — decided by the scan below, not by binding powers |
| 3 | left | `\|\|` |
| 4 | left | `&&` |
| 5 | left | `==` `!=` |
| 6 | left | `<` `>` `<=` `>=` |
| 7 | left | `\|` `^` |
| 8 | left | `&` |
| 9 | left | `<<` `>>` |
| 10 | left | `+` `-` |
| 11 | left | `*` `/` `%` |
| 12 | left | `as` — numeric cast; the RHS is a type-naming position |
| 13 | right | unary `-` `!` `~` |
| 14 | left | postfix: `.name`, `.name<..>(..)`, `(..)`, `[..]`, `?` |

Postfixes chain in one loop, so `p.value.x` and `arr[i].push(x)` compose
without special cases. `is` (type test) is a keyword; `as` is the only cast.

### Lookahead

rut's grammar makes shallow peeking enough: statements are keyword-led,
`{` never starts an expression, imports are flat, and paths are
`.`-dotted.

| decision | mechanism |
|---|---|
| item dispatch | peek 1 (`use` `let` `enum` `struct` `class` `trait` `impl` `fn`) |
| statement vs expression-statement | peek 1 (leading keyword) |
| `for`-of vs C-style `for` | peek 4: `for ( let Ident <of or =>` |
| instance vs class method | peek 4: `fn Ident ( <mut? self? …>` |
| struct literal vs path expression | peek 2: `Ident {` ⇒ literal (classes have no instance literal) |
| `when`-arm body form | peek 1 after `->` (`{` ⇒ block arm, else expression arm) |
| postfix loop step | peek 1 (`.` `(` `[` `?`) |
| assignment target | no lookahead — parse, then validate |

Two decisions need more than 4 tokens and use **scans** — read-only,
commit-once, never a guess:

- **Lambda vs parenthesized expression.** Scan to the matching `)`; it is a
  lambda iff the tokens form a valid parameter list and `=>` follows (an
  optional `-> Type` may sit between). Otherwise a comma inside the parens
  errors at the comma — rut has no tuples-as-expressions spelling — with a
  note: *if you meant a lambda, add `=>`*.
- **Generic call vs `<` comparison** (`Vec<f32>(n)` vs `a < b > (c)`). Scan
  from the `<` tracking angle depth; a `>>`/`<<` consumed where a
  closer/opener is expected counts as two, so `Vec<Vec<i32>>` needs no
  re-lexing. Generic arguments commit iff depth returns to 0 on a `>` and
  the very next token is `(`. `a < b > (c)` is not expressible
  unparenthesized — write `(a < b) > (c)`.

Scans never mutate parser state. Peeking at arbitrary distance is allowed;
checkpoint/rollback is not — the API does not exist, so the monotone-cursor
invariant (debug-asserted) cannot be violated, even during error recovery.

### Recovery

Within a file the parser resynchronizes at `;`, `}`, or the token after a
balanced block and keeps parsing — a file yields many diagnostics, not one.
Resync only ever skips forward.

## Declaration mode

The mode is chosen by file extension: `Mode::Impl` for `.rut`,
`Mode::Decl` for `.d.rut`. Both share one grammar; declaration mode adds
the surface declarations — `host fn`, `host struct`, and the builtin
rows (`prelude builtin` / `pub builtin`, see
[Host fns and declaration files](host-fns.md)) — and **forbids
bodies**: any block,
impl blocks included, is rejected with *implementation in a declaration
file*. Implementation mode rejects `host`/`builtin` with the mirror
message. The full surface and how bodies bind at load time live in
[Host fns and declaration files](host-fns.md).

Declarations-only module scope is enforced by the parser itself: a
statement at module top level is a syntax error, not a semantic one. The
grammar itself is specified in [Lexical structure](lexical-structure.md)
and [Modules and visibility](modules-and-visibility.md).

## The AST

Nodes are plain records in one arena; children are `NodeId`s into it.
Spans sit on every node; there are no `String` keys — names are `IdentId`s
into an interner that lives in `rut-core`, shared by the AST, the type
table, and the module binary's name table. The interner pre-interns a fixed
well-known table (`rut_core::sym`: `self`, `Self`, `nil`, `main`,
`__iterate`, the builtin members, the primitives, …), so ids below
`well_known_len` mean the same name in every interner instance and special
names compare as `IdentId` equality, never by text.

```rust
struct Ast { nodes: Vec<Node>, root: NodeId }
struct Node { span: Span, kind: NodeKind }

enum NodeKind {
    // items
    Use { pkg: IdentId, names: Vec<IdentId> },
    Let { vis, name: IdentId, ty: Option<NodeId>, init: NodeId },
    Enum { vis, name, members: Vec<IdentId> },
    Struct { vis, name, generics, fields: Vec<NodeId> },  // fields only
    Class  { vis, name, generics, fields: Vec<NodeId> },  // fields only —
                                                           // methods live in impls
    Trait  { vis, name, generics, requires, methods },
    Impl   { trait_ref: Option<NodeId>, target: NodeId },
    Fn     { vis, name, generics, params, ret, body },
    SurfaceFn / SurfaceClass,                             // .d.rut only
    // statements
    LetStmt, If, While, ForOf, ForC, Return, When, ExprStmt,
    // expressions
    Lit(Lit), Path(Vec<IdentId>),
    Call { callee, generics, args },
    Method { recv, name, generics, args },
    Field { recv, name }, Index { recv, idx },
    Unary, Binary { op, lhs, rhs }, Assign,
    Lambda { params, ret, body }, When { scrut, arms },
    Try { expr },                 // postfix ?
    FStr { parts },               // holes are NodeIds
    StructLit { ty, fields }, Array { elems }, Conv { ty, expr },
}
```

Stable `NodeId`s are the rest of the pipeline's currency: the checker keys
side tables by node id, diagnostics reach spans through them, and the LSP,
formatter, and dumper hold copyable references with no reparenting.

What the AST does **not** contain: no `new`, no `switch`/`case`, no `?.`
or `??` — the lexer rejects those words before parsing. There are no paren
nodes either: the parse's grouping *is* the tree.

## Diagnostics

```rust
struct Diag { span: Span, msg: String,
              labels: Vec<(Span, String)>, notes: Vec<String> }
```

One renderer serves every frontend stage: a primary line with a caret
span, secondary labels, and notes, all over byte offsets
(`rut_lexer::diag::render_diags`). Rendering sorts by span so multi-file
output is stable. Suggestions are part of the message set:
*did you mean `when`?* for `switch`/`match`; *bind it to a name first* for
string literals in format-string holes; *classes have no instance literal —
use `Circle(..)`* for `Circle { .. }` on a class.

The compile stops before IR if any diagnostic exists — a module with
diagnostics never produces bytecode. See
[Diagnostics, traces, and symbolication](diagnostics.md) for the runtime
half of the story.

## The formatter

`rut fmt` (crate `rut-fmt`, CLI subcommand) reprints a parsed-clean module
from the AST:

- **Comment gap-scan** — comments live in the byte gaps between token
  spans, so the formatter recovers every `//` and block comment with zero
  lexer/parser changes: trailing runs go on their element's line, own-line
  runs at the enclosing indent.
- **Parens and literals re-derived** — the printer re-parens only where
  flat precedence would invert the tree, re-spells floats shortest
  round-trip from their bits, and re-quotes strings by the exact inverse of
  the lexer's escape set (`0xff` legitimately becomes `255`).
- **Tested invariants** — every corpus file formats, reparses clean, and is
  idempotent (`fmt(fmt(x)) == fmt(x)`); probe programs run through the full
  pipeline before and after formatting and log identical output.
- **Refusal law** — a source with diagnostics is refused: rendered
  diagnostics, exit 1, nothing rewritten. The formatter never guesses.
- **Style is the package's** — the nearest ancestor `rut.toml`'s
  `[style]` block sets `indent_width` (1–8, default 4) and `max_width`
  (≥ 20, default 100); no manifest means defaults. See
  [Project structure and rut.toml](project-structure.md).

## Testing

The corpus is the conformance suite: every `examples/**/*.rut`,
`examples/**/*.d.rut`, and playground classic must parse with zero
diagnostics. Parser invariant tests include 100k-deep `((((`, `[[[[`,
`{{{{`, unary chains, and nested generic arguments — each yields exactly
one *nesting too deep* diagnostic — and fuzz targets assert no host stack
overflow and no cursor rollback. Formatter tests pin the round-trip and
idempotence laws above.
