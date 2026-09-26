# the rut-fmt survey (phase 0)

**Batch:** rut-fmt · **Phase:** 0 (docs only) · **Base:** `9cb3e24` (the weak batch record) · **VERSION:** 13 (untouched — the formatter is pure frontend, RFC 0030 §7's promise) · **Date:** 2026-09-26

The user's rulings (Sep 26 2026), the law this batch implements:

1. **Formatter brain = AST reprint + comment gap-scan** (the survey's recommendation accepted): the printer drives off the typed AST with canonical layout; comments are recovered from source byte gaps between lexed token spans. A token-stream formatter (which merely re-indents inherited layout) is rejected.
2. **Style is configurable through `rut.toml [style]`** — not fixed Heyt defaults, and not a separate config file: the style law lives in the package manifest beside `[deps]`/`[peer-deps]`/`[dev-deps]` (RFC 0041 §5's model — knobs live in package manifests, never a hidden global).
3. **VM-verified pins too**: reformatting must never change program behavior, and because the AST carries no paren nodes, the guard is the VM itself — probe programs run through the full pipeline before and after formatting must produce identical results.

---

## §1 the contract (RFC 0030 §7, verbatim constraints)

RFC 0030 §7 pins the formatter's contract on the frontend:

> - The lexer and parser are pure functions (`&str -> (Vec<Token>, Vec<Diag>)`, `&[Token] -> (Ast, Vec<Diag>)`) — fuzzable, usable in an LSP or formatter without a VM.
> - Round-trip invariant: `parse(pretty(ast)) == ast` (formatter).
> - Comments and blank lines are dropped from the AST (doc comments kept on declarations); formatting fidelity is the formatter's job, not the AST's.

RFC 0001 M6 lists the formatter as the one M6 tooling piece that does not exist yet: the LSP (semantic tokens, hovers, completions, inlay hints, references, signature help), the CLI runner (`rut run/pack/dump`, RFC 0041 §2), and `.rutbundle` packing (RFC 0038) are all landed.

**The comment situation at this tree**: the lexer skips `//` and `/* */` entirely in its run loop (`lexer.rs:105-113`), and again inside f-string holes (`lexer.rs:515-525`). No `Tok` variant, no trivia side-channel, no doc-comment field anywhere in the AST (`ast.rs` has no `Doc`; "doc comments kept on declarations" is the RFC's plan, not the tree's state). A naive `pretty(ast)` would slaughter every comment in the corpus. **§3's gap scan is the recovery.**

## §2 the fidelity census — what printing must recreate

The AST carries two intentional losses the printer must reconstruct:

- **Source parens** — `ExprKind` has no paren node (`ast.rs:551ff`). The parser's operator stack is precedence-reducing (`expr.rs:233-296`), so the AST tree shape IS the parse grouping; the printer re-emits exactly the parsed shape and re-parens a child only when flat precedence would collide. The table (from `expr.rs:247-259` + the `is`/`as` arms):

  | level | ops | spelling |
  |---|---|---|
  | 3 | `or` | `\|\|` |
  | 4 | `and` | `&&` |
  | 5 | eq | `==` `!=` |
  | 6 | comparison + **`is`** (non-associative — `last_level == 6` breaks the chain, `expr.rs:281-291`) | `<` `>` `<=` `>=` `is` |
  | 7 | `\|` `^` | bitwise |
  | 8 | `&` | |
  | 9 | shifts | `<<` `>>` |
  | 10 | additive | `+` `-` |
  | 11 | multiplicative | `*` `/` `%` |
  | 12 | **`as`** (tighter than `*`, `expr.rs:296-316`) | `as` |
  | 13/14 | unary / postfix | `-` `!` `~`, calls/index/field/`try ?` |

  Paren rules: a child at LOWER precedence than the parent binop gets parens; a child at the SAME precedence on the right-hand side of a left-associative parent gets parens (`a - (b - c)`). The AST shape is the parsed truth — `is`-non-associativity and assignment chains print whatever the parse built; the corpus + VM pins referee.
  - Note: a one-element paren `(1)` parses as a GROUPING not a tuple (`expr.rs:568-584`); the AST carries no record and both spellings reparse the same shape — print bare, no parens (idempotence holds because `fmt(fmt(x))` sees the same shape).
- **Literal re-derivation** — `Lit::Float` is f64 bits (`ast.rs:599`, the RFC 0007 §2 law: every float literal survives as bits); print shortest-round-trip (`format!("{}", f64::from_bits(..))`), suffix from the `FloatSuffix`. `Lit::Str` is the DECODED content — re-quote (re-escape the escapes the lexer's `lex_plain_string_body` consumed). `Lit::RawStr(String)` — re-quote canonically bare `r"..."`, falling back to hash-quoted when content has `"`; identical raw lexing is idempotent. `Lit::Int` re-prints with the suffix from `IntSuffix`.

Tail corollary: **`fmt(fmt(x)) == fmt(x)` and both reparse to the same shape** — that IS the RFC's invariant realized, modulo the paren shape the tree never had.

## §3 the comment vacuum, and the recovery

- Comments are dropped at the lexer's run loop (`lexer.rs:105-113`, `//`-run and `/* */` with unterminated-block diag at `:122`), and again inside f-string holes (`lexer.rs:515-525`, hole comments terminate at `}` — a different skip rule).
- The hole sub-lexer lexes hole content into SEPARATE tokens (`FStrTok`/`FPart`, `token.rs:47-53`; AST side `FPartAst::Hole(NodeHandle<AnyExpr>)` with real spans).
- **The recovery**: a `Comment` lives in a byte gap BETWEEN two lexed token spans (or before the first / after the last token). Token spans index the normalized source (`lexer.rs:12` CRLF rule) — the formatter runs the SAME normalization first, then scans gaps; it never lands inside a string/hole interior because those are token interiors.
- Blank-line preservation needs the same arithmetic: the source line delta between the visiting element's span and the next (comments count as content lines). Corpus-conventions census: blank lines between items are universal; multiple blanks occur in copy history (normalize to <=1, deterministic).
- **Attachment rule** (v1): comment runs are flushed whenever the printer's next emission starts strictly after their source position (`event-flush` at node boundaries + operator/punctuation emission points inside expressions). Same-line vs own-line: a comment starting on the line where previous token content ended stays trailing `// …`; an own-line comment indents to the enclosing block.

## §4 the manifest surface: a real grammar change, not a knob

`parse_manifest` (`crates/rut-driver/src/session.rs:419-500`) REJECTS unknown sections — `[style]` today refuses to load (`unknown section '[style]'`). The landing extends:

- `Section::Style` variant + `Manifest { style: BTreeMap<String, String> }` (string-typed values, validated by the CONSUMER — the manifest layer stays schema-free like the deps tables).
- **Keys, v1**: `indent_width` (validated 1..=8) and `max_width` (validated >= 20); absence = the defaults (4/100), which are the corpus's own conventions. Unknown keys ignored (the forward-compatibility rule the top-level keys already hold, RFC 0041 §5's wording).
- Style resolution lives in the fmt crate consuming `Manifest.style`; the CLI passes a resolved `Style` (the crate stays manifest-free — same I/O-free layering as the Session).

## §5 the corpus sits in the four trees

The format corpus reuses the parser's enumerator exactly (rut-parser corpus.rs:13-38): `examples/**`, `demo/src/examples/**`, `rut/**` (the stdlib), `benches/workloads/**` — 59 running or load-bearing `.rut` files with zero diags. Layout census: 4-space indent throughout (no tabs anywhere), trailing commas in when-arms, struct literals and multi-line containers (`opaque.rut`, `classes.rut`, `digest.rs`'s json section), and ~60 long lines (>100 cols, max 179 — mostly unbreakable trailing comments and a few dense struct literals in `digest.rut`'s json fns). The corpus is therefore the round-trip conformance suite, NOT byte-frozen: the corpus files carry hand-aligned comment columns and one-liner bodies the formatter will normalize (e.g. `fn id(self) -> i32 { return self.n; }` reflows to multi-line). Only unit snapshots pin exact bytes; the corpus asserts idempotence + reparse-clean + VM equivalence, not parity with the hand-written original.

## §6 what v1 prints

Working grammar full coverage v1: items (`use`, `type`, `let`, `enum`, `struct`, `class` (incl. `requires`), `trait`, `impl`/trait-impl, `fn`/`async`/`entry`, `host`/`builtin` surface fns, `host struct`, `builtin Name<..>`, `builtin trait`, `builtin primitive`, `builtin impl <prim>`), statements (let/init/destructure? — `let` has `destructure: Option<Vec<IdentId>>`, if/while/for-of/for-c, when-stmt, return, break/continue, when-expr/arms, expr-stmt), type grammar (`?T` prefix (postfix `T?` is gone — diagnosed), `[T]`, tuple `(A, B)` / grouping only, `fn(..) -> ..`, `requires` bounds, union).

**Test plan** (P1): unit pins (comment cases: leading, trailing, mid-block, f-string holes, trailing EOF comment); corpus suite (idempotence + reparse-clean over all four trees), CLI pins (in-place rewrite + `--check` refused, exit 1); decl-mode (`rut/core/core.d.rut` — `builtin` surface, methods with no bodies; `examples/03-plugin/server/server.d.rut` two-line surface file). **VM-verified pins** (user ruling 3): `tests/semantic_equiv.rs` runs the full pipeline (compile → link → `Vm::call`) on probes original vs formatted — identical Logger lines / returns; this is the honest compensation for paren-stripping (the AST has no paren node, so ONLY the VM can prove the re-paren table preserved meaning).

**Gates**: `cargo test --workspace`, wasm32-unknown-unknown check (workspace unchanged — the crate has no wasm ABI), the corpora untouched by fmt in tests (only special unit dirs rewrite; NOTHING outside the tooling delta tree is rewritten by this batch).

## §7 phase order

P0 (this survey, docs) → P1 (the landing: rut-fmt + CLI + tests, one commit) → P2 (the report `docs/fmt-report.md` + the RFC 0030 §7 / 0041 §5 amendments + the docs sweep). Scratch receipts under `/tmp/opencode/batch-rut-fmt/p{N}/`.
