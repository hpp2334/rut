# RFC 0030: Frontend — Lexer, Parser, AST, Diagnostics

- **Status:** Draft
- **Date:** 2026-08-22
- **Author:** hpp2334
- **Depends on:** RFC 0002 (lexical structure), RFC 0003 (modules),
  RFC 0029 (declaration files), RFC 0001 (M0)
- **Supersedes:** RFC 0006 §3–8 (pre-restructure)
- **Covers:** the frontend half of the pipeline — source text → tokens →
  AST → diagnostics. No type knowledge; the parser is syntactic only.
- **Part:** F — Toolchain & artifacts

## Summary

A hand-written, dependency-free frontend: a mode-stack **lexer** (handles
`f"..."` interpolation holes and `r"..."` raw strings), a
**recursive-descent parser with precedence climbing** for expressions, and
a **spanned AST** shared by the resolver (RFC 0031). One diagnostics model
from the first illegal byte to the last semantic error: `Diag { span, msg,
labels }` — pretty-rendered, byte offsets, no line/column lossage.

```
source (UTF-8) ─► Lexer ─► Token[] ─► Parser ─► Ast ─► (RFC 0031 resolve/typecheck)
                         └─ Diag[] ◄──┘
```

Lexical rules (source model, identifiers, `$`, reserved words) are the
language contract in RFC 0002; this file is the machinery.

## 1. Tokens

```rust
enum Tok {
    // literals
    Int(u64, IntSuffix), Float(u64 /*bits*/, FloatSuffix),
    Str(Vec<u8>),            // decoded UTF-8, escapes resolved
    RawStr(Vec<u8>),         // no escape processing
    FStr(FStrTok),           // §2 — parts + lexed holes
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

- **Wrapping-arith assignment** (`&+=`, `&-=` …, RFC 0004 §3) lexes as
  dedicated `Amp` + `PlusEq`-style digraph tokens (`AmpPlusEq` etc. exist
  in the real enum; the sketch abbreviates). Assignment operators are one
  token each — no maximal-munch ambiguity.
- **Maximal munch** with an explicit longest-match table; `/` vs `//` vs
  `/*` resolved by one lookahead.
- Numeric literals: per RFC 0007 §1 (`0x`/`0b`/`0o`, `_` separators,
  suffixes; unsuffixed integer → `i32`, unsuffixed float → `f64`).

### 1.1 Format strings — lexer mode stack

`f"..."` is the one construct the lexer must understand structurally,
because placeholders contain **expressions**, not strings (RFC 0007 §2):

```rust
struct FStrTok { parts: Vec<FPart> }      // interleaved chunks and holes
enum FPart {
    Lit(Vec<u8>),             // decoded like a plain string; {{ }} -> { }
    Hole(Vec<Token>),         // a FULLY LEXED token stream, `}`-terminated
}
```

- the lexer enters `Hole` mode after `{` (not `{{`), **balances braces**,
  and lexes normally until depth 0;
- a hole may contain **any expression except a string literal** — a quote
  inside a hole is a lex error with a "bind it to a name first" note;
- holes are re-lexed, not stored as text: the parser receives ready tokens,
  so placeholder expressions get real spans inside the literal's span.

Raw strings `r"..."`: everything until the closing `"` is literal; `\"`
inside a raw string does NOT close it. `rf"..."` is RFC 0007 OQ-3
(rejected in v1; the parser emits a targeted error).

## 2. Grammar summary

The authoritative surface syntax is the RFC series Part B + `examples/`;

```
module     := (import | export? decl)*
import     := 'import' '{' name (',' name)* '}' 'from' Str ';'
decl       := constdecl | enumdecl | dataclassdecl | classdecl | interfacedecl | fndecl
constdecl  := 'const' Ident ':' Type '=' expr ';'
enumdecl   := 'enum' Ident '{' Ident (',' Ident)* ','? '}'
dataclass  := 'dataclass' Ident genericparams? ('implements' IfaceList)? '{' (field | meth)* '}'
class      := 'class' Ident genericparams? ('implements' IfaceList)? '{' member* '}'
member     := 'private'? ('static' field | ('suspend')? 'factory' | field | meth | dispose)
meth       := 'fn' Ident '(' 'self'? params ')' (':' Type)? block
                                              // 'self' first param => instance
                                              // method; no 'self' => class
                                              // method (RFC 0010 §2 — there
                                              // is no 'static fn')
dispose    := 'dispose' '(' 'self' ')' ':' 'void' block
interface  := 'interface' Ident genericparams? ('requires' IfaceList)? '{' methsig* '}'
methsig    := 'fn' Ident '(' 'self' ',' params ')' (':' Type)? ';'
                                              // interface methods are always
                                              // instance methods (RFC 0012 §2)
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
expr       := assignment | lambda | when-expr | …         // §3 precedence
lambda     := '(' params ')' (':' Type)? '=>' (expr | block)
```

`Type` in **value positions** (params, returns, locals, fields, generic
arguments) may spell an interface object type with the `dyn` prefix —
`d: dyn Drawable`, `Vec<dyn Widget>`, `Rc<dyn Any>`, `Rc<dyn Slice<i32>>`
(the builtin slice interface, RFC 0005); a bare interface
name there is a type error with an "insert `dyn`" suggestion (RFC 0012 §2).
`IfaceList` — `implements`, `requires`, and extparam bounds (§3) — stays
**bare**: those positions name an interface, they do not form an interface
value (RFC 0013 §2). `dyn` is a keyword (RFC 0002 §4); `as` remains
reserved and always errors (erasure is the `make_any` call, RFC 0014).

Declarations-only module scope (RFC 0003 §1) is enforced by the parser
itself — a statement at module top level is a syntax error, not a semantic
one.

## 3. Declaration mode (`.d.rut`)

A `.d.rut` file parses under the same grammar **plus** the surface
declarations of RFC 0029 §2, with bodies forbidden:

```
surfacedecl := 'host' 'fn' Ident genericparams? '(' params ')' (':' Type)? ';'
             | 'extern' 'fn' Ident genericparams? '(' params ')' (':' Type)? ';'
             | 'host' 'class' Ident extparams? '{' extmember* '}'
             | 'extern' 'class' Ident extparams? '{' extmember* '}'
extmember   := 'factory' '(' params ')' ':' Type ';'
             | 'fn' Ident '(' 'self' ',' params ')' (':' Type)? ';'
                                            // host/extern class methods are
                                            // instance methods — `self`
                                            // spelled, no body (RFC 0025 §2)
extparams   := '<' (Ident (':' Iface)? ','?)+ '>'    // bounds: surface decls only (RFC 0013 §2)
```

Mode is chosen by file extension. The parser in declaration mode rejects
any `block` with *"implementation in a declaration file"*; in
implementation mode it rejects `host`/`extern` with *"declaration keyword
in an implementation file — belongs in a `.d.rut`"* (RFC 0029 §2).

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
| 13 | left | postfix: `.name` `.name<..>(..)` `(..)` `[..]` `?` (no `as` — RFC 0012 §3) |

Postfixes are a loop, so `p.value.x`, `arr[i].push(x)` chains compose
without special cases. Generic arguments in call position
(`downcast<Point>(o)`, `Vec<f32>(n)`) parse when `<` follows an
identifier **and** a matching `>` + `(` closes — the classic ambiguity
with `<`/`>` comparisons is resolved by backtracking one token sequence
(checkpoint/restore over the token slice; never over text).

`when` in expression position parses arms as `pattern -> expr ,` — the
comma is required between expression arms and forbidden after block arms
(RFC 0008 §2).

## 5. AST

Spans on every node; no `String` keys — names resolve later (RFC 0031 §1).
Sketch (abbreviated — the full enum is mechanical):

```rust
struct Ast { module: Mod }
struct Mod { span: Span, items: Vec<Item> }

enum Item {
    Import  { span, names: Vec<Ident>, from: LitStr },
    Const   { span, vis, name: Ident, ty: Option<Ty>, init: Expr },
    Enum    { span, vis, name: Ident, members: Vec<Ident> },
    Dataclass{ span, vis, name, generics: Vec<Ident>, implements: Vec<Ty>,
               fields: Vec<Field>, methods: Vec<FnDecl> },  // RFC 0009
    Class   { span, vis, name, generics, implements: Vec<Ty>,
              members: Vec<Member> },
    Interface{ span, vis, name, generics, requires: Vec<Ty>,  // RFC 0012 §2
               methods: Vec<FnSig> },
    Fn      { span, vis, is_suspend, name, generics,
              params: Vec<Param>, ret: Option<Ty>, body: Block },
    SurfaceFn  { span, vis, linkage, name, generics, params: Vec<Param>, ret: Ty },
    SurfaceClass{ span, vis, linkage, name, params: Vec<(Ident, Option<Ty>)>,
                  members: Vec<FnSig> },   // linkage = Host | Extern (RFC 0029)
}
// linkage: Host | Extern — declared ONLY in .d.rut (RFC 0029 §2)

enum Member { Field(Field), Factory{ span, is_suspend, params, ret, body },
              Method{ span, has_self, sig: FnSig, body: Block },  // instance
              Dispose{ span, body: Block } }  // iff has_self — RFC 0010 §2;
                                              // no is_static: absence of
                                              // `self` IS the class-method case

enum Stmt { Let{..}, Const{..}, Expr(Expr), If{..}, While{..}, ForOf{..},
            ForC{..}, Return{..}, When{..} }

enum Expr {
    Lit(Lit), Path(Vec<Ident>),
    Call{ span, callee: Box<Expr>, generics: Vec<Ty>, args: Vec<Expr> },
    Method{ span, recv: Box<Expr>, name: Ident, generics, args },
    Field{ span, recv: Box<Expr>, name: Ident },
    Index{ span, recv: Box<Expr>, idx: Box<Expr> },
    Unary{..}, Binary{..}, Assign{..},
    Lambda{ span, params, ret, body: LambdaBody },
    When{ span, scrut, arms },
    Try{ span, expr: Box<Expr> },       // postfix `?`
    FStr{ span, parts: Vec<FPartAst> }, // holes are Exprs here
    Struct{ span, ty: Path, fields: Vec<(Ident, Expr)> },  // dataclass literal
    Array{ span, elems: Vec<Expr> },        // fixed-array literal: [e1..en] : Array<T, n>
    Cast{ span, ty: Ty, expr },         // i32(x) etc. — a Call on type name,
}                                       // resolved to Cast in RFC 0031 §1
```

Note what the AST **does not contain**: no `new`, `switch`/`case`,
`?.`/`??` — the lexer errors on these reserved words with a "rut does not
have X; use Y" message (RFC 0002 §4, §6 here).

## 6. Diagnostics

```rust
struct Diag { span: Span, msg: String,
              labels: Vec<(Span, String)>, notes: Vec<String>, fatal: bool }
```

- One renderer for lexer/parser/resolver/typecheck (RFC 0031): caret spans,
  a primary message, optional secondary labels, optional notes; unit-tested
  against golden files (`tests/diagnostics/*.txt`).
- **Recovery**: within a file, the parser resyncs at `;`, `}`, or the token
  after a balanced block, and keeps parsing — a file yields many diags, not
  one. Compilation stops before IR if any diag exists.
- Suggestions: `did you mean \`when\`?` for `switch`/`match`; "bind it to a
  name first" for string literals in `f"..."` holes; "classes have no
  instance literal — use \`Circle(..)\`" for `Circle { .. }` on a class.

## 7. Testing & tooling hooks

- The lexer and parser are pure functions (`&str -> (Vec<Token>, Vec<Diag>)`,
  `&[Token] -> (Ast, Vec<Diag>)`) — fuzzable, usable in an LSP or formatter
  without a VM.
- Round-trip invariant: `parse(pretty(ast)) == ast` (formatter) and
  `examples/**/*.rut` parse with zero diags — the corpus *is* the parser's
  conformance suite (declaration mode: `examples/**/*.d.rut`).
- Comments and blank lines are dropped from the AST (doc comments kept on
  declarations); formatting fidelity is the formatter's job, not the AST's.

## Open questions

- OQ-1: trailing commas — allowed in argument lists and dataclass literals?
  Proposed: allowed everywhere a comma list exists.
- OQ-2: attributes (`@inline`, `@repr(align)`) — `At` is lexed but
  unclaimed; park the token until a real need exists (repr C is the
  default, RFC 0015 §4, so `@repr` is NOT planned).
