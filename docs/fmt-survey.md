# the rut-fmt survey (phase 0)

**Batch:** rut-fmt · **Phase:** 0 · **Base:** `9cb3e24` (the weak batch record) · **VERSION:** 13 untouched · **Date:** 2026-09-26

The user's rulings, the law this batch implements:

1. **Formatter brain = AST reprint + comment gap-scan** (my §A recommendation accepted): the printer drives off the typed AST with canonical layout; comments are recovered from source byte gaps between lexed token spans. A token-stream formatter (which merely re-indents inherited layout) is rejected.
2. **Style is configurable through `rut.toml [style]`** — not fixed, not a separate config file: the style law lives in the package manifest beside `[deps]`/`[peer-deps]`/`[dev-deps]` (RFC 0041 §5's model — knobs live in package manifests, never a hidden global).
3. **VM-verified pins too**: reformatting must never change program behavior, and because the AST carries no paren nodes, the guard is the VM itself — probe programs run through the full pipeline before and after formatting must produce identical results.

---

## §1 the contract (RFC 0030 §7, verbatim constraints)

RFC 0030 §7 pins the formatter's coordinates:

> - The lexer and parser are pure functions (`&str -> (Vec<Token>, Vec<Diag>)`, `&[Token] -> (Ast, Vec<Diag>)`) — fuzzable, usable in an LSP or formatter without a VM.
> - Round-trip invariant: `parse(pretty(ast)) == ast` (formatter).
> - Comments and blank lines are dropped from the AST (doc comments kept on declarations); formatting fidelity is the formatter's job, not the AST's.

RFC 0001 M6 lists the formatter as the one M6 tooling piece that does not exist yet: the LSP (semantic tokens, hovers, completions, inlay hints, references, signature help), the CLI runner (`rut run/pack/dump`, RFC 0041 §2), and `.rutbundle` packing (RFC 0038) are all landed.

**The comment situation at this tree**: the lexer skips `//` and `/* */` entirely in its run loop (`lexer.rs:105-124`), and again inside f-string holes (`lexer.rs:515-525` — hole comments terminate at `}`, a different skip rule). No `Tok` variant, no trivia side-channel, no doc-comment field anywhere in the AST (`ast.rs` has no `Doc`; "doc comments kept on declarations" is the RFC's plan, not the tree's state). A naive `pretty(ast)` would slaughter every comment in the corpus. **§3's gap scan is the recovery.**

## §2 the fidelity census — what printing must recreate

The AST carries two intentional losses the printer must reconstruct:

- **Source parens** — `ExprKind` has no paren node. The parser's operator stack is precedence-reducing (`expr.rs:233-316`), so the AST tree shape IS the parse grouping; the printer re-emits exactly the parsed shape and re-parens a child only where flat precedence would invert the parse's grouping. The level table (from the op loop, `expr.rs:247-316`):

  | level | ops |
  |---|---|
  | 3 | `\|\|` (Or) |
  | 4 | `&&` (And) |
  | 5 | `==` `!=` |
  | 6 | comparisons `<` `>` `<=` `>=` **plus `is`** — non-associative: the last-level-6 guard breaks `x is T is U` chains (`expr.rs:281-291`) |
  | 7 | `\|` `^` |
  | 8 | `&` |
  | 9 | `<<` `>>` |
  | 10 | `+` `-` |
  | 11 | `*` `/` `%` |
  | 12 | **`as`** — tighter than `*` ("Rust placement"); left-associative; the RHS is a naming-position type whose next non-numeric word is RESERVED for the select-arm bind (`fut as name`, RFC 0019 §3) |
  | 13/14 | unary `- ! ~` / postfix call-index-field-`try ?` — children always bare |

  Paren rules: a child at LOWER level than its parent binop gets parens; a child at the SAME level on the right-hand side of a left-associative parent gets parens (`a - (b - c)` keeps its form; `a + b * c`'s shapes stay parens-free). Assign never occurs as an operand's child (no C-style value expressions). The AST tree shape is the truth the parser built — the printed source must REPARSE to that same shape, and the VM pins referee (no AST captures the paren-ness, so equal trees prove the output is shape-exact).
  - One-element parens: expressions — a `(a)` is a GROUPING, dropped: print bare, same AST both ways (`expr.rs:568-584`); types — `(T)` unfolds, same law (`ty.rs:283-300`).

- **Literal re-derivation** — RFC 0007 §2 lives as bits and content:
  - `Lit::Float(u64 /*f64 bits*/, Option<FloatSuffix>)` — print shortest-round-trip from `f64::from_bits` (suffix `f32` via the f32-twin — the f32 can never be lossless from f64 bits: the f32→f64→f32 roundtrip IS exact, the digits differ, only the value matches — the corpus files use `1.5`-scale values whose strings recover exactly);
  - `Lit::Int(u64, Option<IntSuffix>)` — decimal, the radix is lost (`0x22` → `34`: same value, idempotent re-lex ✓ the AST carries no radix);
  - `Lit::Str(String)` — DECODED content, re-quoted with the EXACT inverse of `lex_escape` (`lexer.rs:171-212`): the escape set is exactly `\n \t \r \b \f \\ \" \' \`$$\0$$ \u{XXXX}` — a string containing a literal `"`, backslash or control char must re-escape identical so re-lexing decodes the same content;
  - `Lit::RawStr(String)` — no escape processing; no hash-quoted form in the lexer (`lex_raw_string_body` requires the closing `"` to be the FIRST in content, `lexer.rs:401-416`) — reprint `r"..."` always with re-lex-identical bytes; a raw body containing `"` is GRAMMAR-IMPOSSIBLE (the raw lexer closes at the first `"` — its content can never contain one);
  - `Lit::Bool` / `nil` — as written.

## §3 the comment recovery — the gap scan (byte-exact, no lexer change)

- Comments: the lexer's run loop skips `//`-runs (`lexer.rs:105-108`) and `/* */` blocks (`:110-124`, the unterminated-block diag at `:122`); in f-string holes the skip differs (hole comments terminate at `}`, `:515-525`). No comment bytes ever REACH a token span — so a byte scan biased at the gaps [the previous token's `hi`, the next token's `lo` → `lo:u32`)] can never land inside a string/hole interior: they are token interiors.
- **The scan**: for every consecutive token pair (i, i+1): scan `src[tok_i.span.hi .. tok_{i+1}.span.lo]` bytes for `//` and `/*`; build `CommentRun { lo, hi, trailing }`: a line comment ends before `\n` (its trailing blanks TRIMMED — the pad re-adds one); block comments end after `*/`. **`trailing` classification: `line(r.lo) == line(tok_i.span.hi)`** — the run attached to the preceding token's line (a `Stmt // explain` pairs this exactly). Before the first token / after the last token also runs; the HEAD is always own-line (no preceding token: `line(...)`'s line-0 was file-header comments). Multi-line block comments survive verbatim, with per-line indent trim (each inner line prints at the CURRENT indent; blank/comment-only lines preserved so re-fmt is inert).
- **Trailing/own-line drain (positional; queue consumed in order)**:
  - `end_elem(el_span)` — drain LINE-comment runs with `line(run.lo) == line(el_span.hi)` (this element's trailing comments — the run must be a line comment to trail): emit on the just-printed line with one pad space, consume, `passed = run.hi`; then newline.
  - `begin_el(next_span, sep)` — drain own-line runs with `run.lo < next.lo`: blank-before if `line(run.lo) - line(passed) >= 2`; print the run at the enclosing indent; `passed = run.hi` (the blank-after derives from the same arithmetic toward `next.lo`, at most one). `sep = false` inside expressions (no blanks — only comment runs; the run terminates the current line because an own-line run is its own line).
- **Idempotence** — all comment/blank decisions re-derive from the formatted source's spans; a formatted file re-parses to the same AST with the same runs on new lines and re-attaches to the same printed elements. Corollaries pinned by tests: no comment is ever deleted; a trailing comment stays trailing; own-line comment groups stay ordered.

## §4 the style block — a real manifest grammar change

`parse_manifest` (`rut-driver/src/session.rs:419-545`) currently REJECTS unknown sections — `[style]` today refuses to load the pkg ( `'unknown section'` at `:440` is an error, never a skip). The landing extends:

- **`Section::Style`** + `Manifest { style: BTreeMap<String, String> }` — rows, `key = "value"` string-mode, schema-free: THE CONSUMER (the fmt crate) validates keys and values into typed `Style`, so unknown keys ride (the forward-compatibility rule §5's top-level keys already holds) and only a formatter run can refuse.
- **Keys, v1**: `indent_width` (validated 1..=8) and `max_width` (validated >= 20). Absence = the defaults (4/100) that match the corpus census.
- **Style resolution** lives in the CLI: `rut fmt` resolves enclosing pkg manifests; the fmt crate takes a plain `Style` — the crate stays manifest-free and I/O-free (the Session's layering law; the same shape the `Entry` struct already rides). **Style resolution rule**: the package whose entry dir clean-match closest to the input path (nearest ancestor); no ancestor manifest → defaults.
- **`[style]` in data**: nothing at the VM/IR level reads it — configs ride parse_manifest's tables, never the compiler's IR — the manifest column of the compiler paper (RFCs 0029-0038) stays clean.

## §5 the layout census (the corpus's own standing style)

The format corpus reuses rut-parser's four-tree enumerator exactly (`corpus.rs:17-38`): `examples/**`, `demo/src/examples/**`, `rut/**` (the stdlib), `benches/workloads/**` — 59 files (plus `*.d.rut` files which the census MISSES; the fmt enumerator must also take `.d.rut` — an extra fn walk in the survey's own plan — the two implementation surfaces differ only in the extension filter), all loaded with zero diags. Layout census across the corpus:

- **Indent 4 spaces, zero tabs** (`Corpus: 0 tab-indented lines counted`) — the house indent is 4, and the default `[style]`'s `indent_width = 4`;
- ** Brace placement': either same line or next line; corpus puts freshly-formatted layout into `{`-on-same-line — **`{` stays on the line it belongs to** (the class/struct/fn-with-body's own opener: `pub fn name(...) -> ret {`; `}` closes at the DEFINING width's column;
- Trailing commas: when-arms end `,` always (`RFC 0008's when-arms carry a trailing comma on EVERY arm including the last, the RFC's own example shows); multi-line containers `P { ... }` with trailing commas — `break_list` breaks and emits trailing commas, flat won't (corpus census: 217 comma-final lines, spanning both styles; the fmt normalizes: multi-line → trailing comma `a,\n b,\n`, single-line → none);
- Blank-line conventions: one between items/decls; runs collapse — the fmt normalizes to ≤1; the census counts ~60 long lines (>100 cols, max 179 — unbreakable trailing comments).

`fmt(fmt(x)) == fmt(x)` (idempotence) and both reparses zero-diag — the corpus remains the conformance suite, NOT byte-frozen (only unit snapshots pin exact bytes; §7 the runner file for the corpus's scan).

## §6 the plan shape

Working surface v1: **items** (`use`, `type`, `let`, `enum`, `struct`, `class` incl. `requires`, `trait`, `impl`/trait-impl, `fn`/`async`/`entry`, `host`/`builtin` surface fns, `host struct`, `builtin Name<..>`, `builtin trait`, `builtin primitive`, `builtin impl <prim>` — spellings recovered from `core.d.rut` + `item.rs:1470-1640`), **statements** (let/destructure, if/while/for-of/for-c, when-stmt, return/break/continue, expr-stmt), **expressions** (all of `ast.rs:551+`: lit/path/call/method/field/index/unary/binary/assign/lambda/try/fstr/struct/tuple/array-lit/repeat/when-expr/await/select/is/cast), **patterns** (lit/path/ctor/wild/else), **types** (path/fn/opt/array/tuple/const/union). Decl-mode (`.d.rut`) support: MethodDecl bodies None → `;`, surface decls — the surface file `examples/03-plugin/server/server.d.rut` (2 lines) and `rut/core/core.d.rut` (302) are the conformance decl inputs.

**Test lanes** (P1): unit pins in-crate (comment cases: leading, trailing, mid-block, f-string, trailing EOF; literal requoting, precedence flat/broken) — `crates/rut-fmt/tests/`; corpus suite (`corpus_fmt.rs`, four trees, idempotence + reparse-clean — mirrors the parser's own corpus driver); CLI pins (in-place rewrite, `--check` refuses + exit 1, decl-mode via `server.d.rut`); **VM-verified equivalence** (`sem_equiv.rs` — full pipeline, original vs formatted: identical Logger lines/return values per probe) — the honest compensation for the paren-stripping gap (the AST holds no paren node, so only the VM can prove the re-paren table preserved meaning).

**Gates**: `cargo test --workspace`, wasm32-unknown-unknown check (workspace unaffected — the crate has no wasm ABI), corpora byte-UNTOUCHED by tests (fmt writes nowhere in tests; `rut fmt` the TOOL rewrites, tests only compare) — demo (the foreign lane) never touched.

## §7 phase order

P0: this survey (docs only, landed `b0b7ebb`). P1: the landing (rut-fmt + CLI + tests, one commit). P2: the report `docs/fmt-report.md` + the RFC 0030 §7 / RFC 0041 §5 amendments + the docs sweep (usage lines).
