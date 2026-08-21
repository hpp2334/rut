# RFC 0006: Syntax — Lexer, AST, Parser

- **Status:** Draft
- **Date:** 2026-08-21
- **Author:** hpp2334
- **Depends on:** RFC 0002 (grammar), RFC 0001 (M0)
- **Covers:** the frontend half of the pipeline — source text → tokens → AST →
  diagnostics. No type knowledge; the parser is syntactic only.

## Summary

A hand-written, dependency-free frontend: a mode-stack **lexer** (handles
`f"..."` interpolation holes and `r"..."` raw strings), a **recursive-descent
parser with precedence climbing** for expressions, and a **spanned AST**
shared by the resolver (RFC 0007 §2). One diagnostics model from the first
illegal byte to the last semantic error: `Diag { span, msg, labels }` —
pretty-rendered, byte offsets, no line/column lossage.

```
source (UTF-8) ─► Lexer ─► Token[] ─► Parser ─► Ast ─► (RFC 0007 resolve/typecheck)
                        └─ Diag[] ◄──┘
```

## 1. Source model

- Files are UTF-8; both LF and CRLF accepted (CRLF normalized to LF in
  spans); a BOM is skipped. Extension: `.rut`.
- Lines are counted lazily from spans for diagnostics; there is no
  column-arithmetic bug class because spans are byte ranges
  (`Span { lo: u32, hi: u32 }`) rendered against the original text.
- Comments: `// line`, `/* block (nesting NOT allowed) */`, and `/** doc */`
  attached to the following declaration (kept in the AST for future tooling;
  not semantic).

## 2. Tokens

```rust
enum Tok {
    // literals
    Int(u64, IntSuffix), Float(u64 /*bits*/, FloatSuffix),
    Str(Vec<u8>),            // decoded UTF-8, escapes resolved
    RawStr(Vec<u8>),         // no escape processing
    FStr(FStrTok),           // §2.1 — parts + lexed holes
    Char(char), True, False,
    Ident(String),           // includes keywords after reservation check
    // punctuation & operators — exhaustive, single flat enum
    LParen RParen LBrace RBrace LBracket RBracket,
    Comma Semi Colon Dot DotDot Arrow FatArrow,   // .  ..  ->  =>
    Plus Minus Star Slash Percent,                // + - * / %
    AmpAmp PipePipe Bang,                         // && || !
    Eq EqEq NotEq Lt Gt LtEq GtEq,                // = == != < > <= >=
    PlusEq MinusEq StarEq SlashEq PercentEq,      // += -= *= /= %=
    Amp Pipe Caret Tilde, Shl Shr,                // & | ^ ~ << >>  (bit ops)
    AmpEq PipeEq CaretEq ShlEq ShrEq,             // &= |= ^= <<= >>=
    AmpAmpEq PipePipeEq,                          // &&= ||= (short-circuit assign)
    Question, At,
    Eof,
}
struct Token { tok: Tok, span: Span }
```

- **Identifiers**: `(letter | _ | $)(letter | digit | _ | $)*` — `$` is a
  fully valid identifier character, anywhere in the name (`on_mount$`,
  `viewport_size$`, `$temp`, `$x1`, even `$` alone). There is no `$`
  punctuation token — the lexer only ever produces `$` inside identifiers,
  and f-string holes use `{ }` (RFC 0002 §4.1), so `$`-identifiers create
  no ambiguity anywhere. The **trailing** `$` remains the sole *conventional*
  position (RFC 0002 — reactive-handle marker): greppability is enforced by
  style, not by the lexer.

- **Wrapping-arith assignment** (`&+=`, `&-=` …, RFC 0002 §8) lexes as
  `Amp` + `PlusEq`-style digraphs: dedicated tokens `AmpPlusEq` etc. exist
  in the real enum; the sketch above abbreviates. Assignment operators are
  one token each — no maximal-munch ambiguity.
- **Maximal munch** with an explicit longest-match table; `/` vs `//` vs
  `/*` resolved by one lookahead.
- **No off-side rule**: statements end with `;` (required), blocks brace.
- Numeric literals: decimal and `0x`/`0b`/`0o`, `_` separators, suffixes
  `u8..u64 i8..i64 f32 f64` (`3.14159f32`, `0xFF_u32`); unsuffixed integer →
  `i32`, unsuffixed float → `f64` (RFC 0002 §4).

### 2.1 Format strings — lexer mode stack

`f"..."` is the one construct the lexer must understand structurally,
because placeholders contain **expressions**, not strings (RFC 0002 §4.1):

```rust
struct FStrTok {
    parts: Vec<FPart>,        // interleaved literal chunks and holes, in order
}
enum FPart {
    Lit(Vec<u8>),             // decoded like a plain string; {{ }} -> { }
    Hole(Vec<Token>),         // a FULLY LEXED token stream, `}`-terminated
}
```

Lexing rules for a hole:

- the lexer enters `Hole` mode after `{` (not `{{`), **balances braces**
  (nesting for `f(a{b})`-style calls is not needed — braces inside a hole
  only appear in nested `f"..."`, which v1 forbids — but `{`/`}` from
  struct-ish syntax is still balanced to find the true closing `}`), and
  lexes normally until depth 0.
- a hole may contain **any expression except a string literal** (plain, raw,
  or format) — RFC 0002 §4.1; a quote inside a hole is a lex error with a
  "bind it to a name first" note.
- holes are re-lexed, not stored as text: the parser receives ready tokens,
  so placeholder expressions get real spans inside the literal's span.

Raw strings `r"..."`: everything until the closing `"` is literal — no
escapes; `\"` inside a raw string does NOT close it (use a plain string).
`rf"..."` is OQ-14 (rejected in v1; the parser emits a targeted error).

## 3. Grammar summary

The authoritative surface syntax is RFC 0002 + `examples/`; the parser's
grammar (in order):

```
module     := (import | export? decl)*
import     := 'import' '{' name (',' name)* '}' 'from' Str ';'
decl       := constdecl | enumdecl | dataclassdecl | classdecl | interfacedecl | fndecl
constdecl  := 'const' Ident ':' Type '=' expr ';'
enumdecl   := 'enum' Ident '{' Ident (',' Ident)* ','? '}'
dataclass  := 'dataclass' Ident genericparams? ('implements' IfaceList)? '{' (field | 'fn')* '}'
class      := 'class' Ident genericparams? ('implements' IfaceList)? '{' member* '}'
member     := 'private'? ('static' field | 'static' 'fn' | ('suspend')? 'factory' | field | 'fn' | 'dispose')
interface  := 'interface' Ident genericparams? ('requires' IfaceList)? '{' meth* '}'
fndecl     := modifiers? ('suspend')? 'fn' Ident genericparams? '(' params ')' (':' Type)? block
block      := '{' stmt* '}'
stmt       := 'let' Ident (':' Type)? '=' expr ';' | 'const' …
            | 'if' '(' expr ')' block ('else' 'if' … | 'else' block)?
            | 'while' '(' expr ')' block
            | 'for' '(' ('let'|'const') Ident 'of' expr ')' block
            | 'for' '(' 'let' … '=' expr ';' expr ';' expr ')' block
            | 'return' expr? ';' | 'when' '(' expr ')' '{' arm* '}'
            | expr ';'                                   // calls, assignments
arm        := pattern '->' (expr ',' | block)
pattern    := path | literal | pattern (',' pattern)* | 'else'
expr       := assignment | lambda | when-expr | …         // §4 precedence
lambda     := '(' params ')' (':' Type)? '=>' (expr | block)
```

Declarations-only module scope (RFC 0002 §1.1) is enforced by the parser
itself — a statement at module top level is a syntax error, not a semantic
one.

## 4. Expression parsing — precedence climbing

One Pratt loop with a binding-power table; no grammar-generator, no
left-recursion handling. Powers (loosest → tightest):

| prec | associativity | operators |
|---|---|---|
| 1 | – | `=` `+=` `-=` `&+=` … (assignment, right; target must be a path/index) |
| 2 | – | `=>` lambdas (parsed when an expression *starts* with `(` params or a single ident and `=>` follows — one-token lookahead disambiguates call vs lambda) |
| 3 | left | `\|\|` |
| 4 | left | `&&` |
| 5 | left | `==` `!=` |
| 6 | left | `<` `>` `<=` `>=` |
| 7 | left | `\|` `^` |
| 8 | left | `&` |
| 9 | left | `<<` `>>` |
| 10 | left | `+` `-` |
| 11 | left | `*` `/` `%` |
| 12 | right | unary `-` `!` `~` |
| 13 | left | postfix: `.name` `.name<..>(..)` `(..)` `[..]` `?` `as`? (no `as` — RFC 0002 §6.1) |

Postfixes are a loop, so `p.value.x`, `arr[i].push(x)`, and `opt?.`-less
`o.value?` chains compose without special cases. Generic arguments in call
position (`downcast<Point>(o)`, `Array<f32>(n)`) parse when `<` follows an
identifier **and** a matching `>` + `(` closes — the classic ambiguity with
`<`/`>` comparisons is resolved by backtracking one token sequence
(checkpoint/restore over the token slice; never over text).

`when` in expression position parses arms as `pattern -> expr ,` — the
comma is required between expression arms and forbidden after block arms.

## 5. AST

Spans on every node; no `String` keys — names resolve later (RFC 0007 §2).
Sketch (abbreviated — the full enum is mechanical):

```rust
struct Ast { module: Mod }
struct Mod { span: Span, items: Vec<Item> }

enum Item {
    Import  { span, names: Vec<Ident>, from: LitStr },
    Const   { span, vis, name: Ident, ty: Option<Ty>, init: Expr },
    Enum    { span, vis, name: Ident, members: Vec<Ident> },
    Dataclass{ span, vis, name, generics: Vec<Ident>, implements: Vec<Ty>,
               fields: Vec<Field>, methods: Vec<FnDecl> },  // RFC 0002 §5.1:
               // impls + inherent methods; no private/static/factory/dispose
    Class   { span, vis, name, generics, implements: Vec<Ty>,
              members: Vec<Member> },
    Interface{ span, vis, name, generics, requires: Vec<Ty>,  // RFC 0002 §6
               methods: Vec<FnSig> },
    Fn      { span, vis, is_async, name, generics,
              params: Vec<Param>, ret: Option<Ty>, body: Block },
}

enum Member { Field(Field), Factory{ span, is_suspend, params: Vec<Param>, ret: Option<Ty>, body: Block },
              Method{ span, is_static, sig: FnSig, body: Block },
              Dispose{ span, body: Block } }

enum Stmt { Let{ span, name, ty, init }, Const{..}, Expr(Expr),
            If{ span, cond, then, els: Option<Block> },
            While{ span, cond, body }, ForOf{ span, pat, iter, body },
            ForC{ span, init, cond, step, body },
            Return{ span, value: Option<Expr> },
            When{ span, scrut, arms: Vec<(Pat, Expr)> } }

enum Expr {
    Lit(Lit), Path(Vec<Ident>),         // segments resolved later
    Call{ span, callee: Box<Expr>, generics: Vec<Ty>, args: Vec<Expr> },
    Method{ span, recv: Box<Expr>, name: Ident, generics, args },
    Field{ span, recv: Box<Expr>, name: Ident },
    Index{ span, recv: Box<Expr>, idx: Box<Expr> },
    Unary{ span, op, expr }, Binary{ span, op, lhs, rhs },
    Assign{ span, target: Box<Expr>, op, value: Box<Expr> },
    Lambda{ span, params, ret, body: LambdaBody },
    When{ span, scrut, arms },          // expression form — same node as Stmt
    Try{ span, expr: Box<Expr> },       // postfix `?`
    FStr{ span, parts: Vec<FPartAst> }, // holes are Exprs here
    Struct{ span, ty: Path, fields: Vec<(Ident, Expr)> },  // dataclass literal
    Array{ span, elems: Vec<Expr> },
    Cast{ span, ty: Ty, expr },         // i32(x) etc. — a Call on type name,
}                                       // resolved to Cast in RFC 0007 §2
```

Note what the AST **does not** contain: no `new` (rejected token), no
`switch`/`case` (rejected token), no `?.`/`??` (rejected tokens) — the lexer
errors on these with a "rut does not have X; use Y" message (§7).

## 6. Rejected & reserved words

- Reserved (parse error with explanation): `new`, `switch`, `case`,
  `default`, `extends`, `super`, `as`, `type` (type alias — future),
  `struct`, `match`, `null`, `undefined`, `any`, `unknown`, `typeof`,
  `instanceof`, `delete`, `in` (only `for..of`), `with`, `var`.
- `this` is contextual (class members only); `Self` is a normal identifier
  bound to the enclosing class inside its body (RFC 0002 §5.2); `fn`,
  `let`, `const`, `if`, `else`, `while`, `for`, `of`, `return`, `when`,
  `else`, `enum`, `class`, `dataclass`, `interface`, `implements`,
  `requires`, `import`, `export`, `from`, `private`, `static`, `suspend`,
  `await`, `factory`, `dispose`, `true`, `false`, `extern` are keywords.

## 7. Diagnostics

```rust
struct Diag { span: Span, msg: String,
              labels: Vec<(Span, String)>, notes: Vec<String>, fatal: bool }
```

- One renderer for lexer/parser/resolver/typecheck (RFC 0007): caret spans,
  a primary message, optional secondary labels, optional notes; unit-tested
  against golden files (`tests/diagnostics/*.txt`).
- **Recovery**: within a file, the parser resyncs at `;`, `}`, or the token
  after a balanced block, and keeps parsing — a file yields many diags, not
  one. Compilation stops before IR if any diag exists.
- Suggestions: `did you mean \`when\`?` for `switch`/`match`; "bind it to a
  name first" for string literals in `f"..."` holes; "classes have no
  instance literal — use \`Circle(..)\`" for `Circle { .. }` on a class.

## 8. Testing & tooling hooks

- The lexer and parser are pure functions (`&str -> (Vec<Token>, Vec<Diag>)`,
  `&[Token] -> (Ast, Vec<Diag>)`) — fuzzable, usable in an LSP or formatter
  without a VM (RFC 0008 §8).
- Round-trip invariant: `parse(pretty(ast)) == ast` (formatter, M6) and
  `examples/**/*.rut` parse with zero diags — the corpus *is* the parser's
  conformance suite.
- Comments and blank lines are dropped from the AST (doc comments kept on
  declarations); formatting fidelity is the formatter's job, not the AST's.

## Open questions

- OQ-1: `for (;;)` C-style loop — keep? The corpus uses `for..of` and
  indexed `for (let i = 0; ..)`; keep both (as sketched) unless the
  formatter fights it.
- OQ-2: trailing commas — allowed in argument lists and dataclass literals
  (as in `enum` above)? Proposed: allowed everywhere a comma list exists.
- OQ-3: attributes (`@inline`, `@repr(align)`) — `At` is lexed but unclaimed;
  park the token until a real need exists (repr C is now the default,
  RFC 0005 §4, so `@repr` is NOT planned).
