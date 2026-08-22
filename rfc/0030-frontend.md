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
`f"..."` interpolation holes and `r"..."` raw strings), an **iterative
parser** — explicit-stack descent with iterative precedence climbing, zero
native recursion — producing a **flat, arena-allocated AST (ESTree-style)**
shared by the resolver (RFC 0031). One diagnostics model
from the first illegal byte to the last semantic error: `Diag { span, msg,
labels }` — pretty-rendered, byte offsets, no line/column lossage.

The parser is built to a four-line contract (mechanics in §4–§5):

- **C1 — flat AST**: nodes are records in one arena, children are
  `NodeId`s — never `Box<Expr>` — built bottom-up; drop/clone are flat
  `Vec` ops, so no input can overflow the host stack through the tree.
- **C2 — no native recursion**: an explicit frame stack for structure,
  iterative Pratt (operator + operand stacks) for expressions; host
  stack usage is constant regardless of input.
- **C3 — depth budget, not stack exhaustion**: frame depth and lexer
  bracket depth are budgeted; exceeding one is a normal `Diag`, never a
  host crash — untrusted source cannot take down the embedding process
  at parse time (RFC 0035 §3).
- **C4 — lookahead discipline**: local decisions peek ≤ 4 tokens; the
  two decisions needing more (lambda, generic call) use read-only
  balancing *scans*. The cursor is monotone non-decreasing for the whole
  parse, recovery included — no checkpoint/rollback, no guess branches.

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
decl       := letdecl | enumdecl | dataclassdecl | classdecl | interfacedecl | fndecl
letdecl    := 'let' Ident ':' Type '=' expr ';'   // module binding — load-time
enumdecl   := 'enum' Ident '{' Ident (',' Ident)* ','? '}'
dataclass  := 'dataclass' Ident genericparams? ('implements' IfaceList)? '{' (field | meth)* '}'
class      := 'class' Ident genericparams? ('implements' IfaceList)? '{' member* '}'
member     := 'private'? ('static' field | ('suspend')? 'constructor' | field | meth)
meth       := 'fn' Ident '(' 'mut'? 'self'? params ')' (':' Type)? block
                                              // 'self' first param => instance
                                              // method; no 'self' => class
                                              // method (RFC 0010 §2 — there
                                              // is no 'static fn')
dispose    := 'dispose' '(' 'self' ')' ':' 'void' block
interface  := 'interface' Ident genericparams? ('requires' IfaceList)? '{' methsig* '}'
methsig    := 'fn' Ident '(' 'self' ',' params ')' (':' Type)? ';'
                                              // interface methods are always
                                              // instance methods (RFC 0012 §2)
fndecl     := modifiers? ('suspend')? 'fn' Ident genericparams? '(' params ')' (':' Type)? whereclause? block
whereclause := 'where' Ident 'requires' IfaceList  // admission-only bounds
                                               // (RFC 0013 §2, RFC 0037 §3)
block      := '{' stmt* '}'
stmt       := 'let' 'mut'? Ident (':' Type)? '=' expr ';'
             | 'if' '(' expr ')' block ('else' 'if' … | 'else' block)?
             | 'while' '(' expr ')' block
             | 'for' '(' 'let' Ident 'of' expr ')' block
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
extmember   := 'constructor' '(' params ')' ':' Type ';'
             | 'fn' Ident '(' 'self' ',' params ')' (':' Type)? ';'
                                            // host/extern class methods are
                                            // instance methods — `self`
                                            // spelled, no body (RFC 0025 §2)
extparams   := '<' (Ident ('requires' Iface)? ','?)+ '>'   // bounds: equires form    // bounds: surface decls only (RFC 0013 §2)
```

Mode is chosen by file extension. The parser in declaration mode rejects
any `block` with *"implementation in a declaration file"*; in
implementation mode it rejects `host`/`extern` with *"declaration keyword
in an implementation file — belongs in a `.d.rut`"* (RFC 0029 §2).

## 4. The parser — explicit-stack descent, iterative precedence climbing

No native recursion (C2): one loop over the token slice drives two
mechanisms. A **frame stack** handles structure — one frame kind per
grammar rule (`Module`, `Item`, `Block`, `TypeArgList`, `Pattern`, `Arm`,
…), each holding the children it has collected so far and the token it is
waiting for; a rule that needs a child pushes a frame, a frame that
completes pops and hands its `NodeId` to the parent. An embedded **Pratt
engine** handles expressions — an operator stack plus an operand stack of
`NodeId`s; binary/assign steps push an operator, reductions pop two
operands and push one node, bottom-up (C1). No grammar-generator, no
left-recursion handling. Binding powers (loosest → tightest):

| prec | associativity | operators |
|---|---|---|
| 1 | – | `=` `+=` `-=` `&+=` … (assignment, right; target must be a path/index) |
| 2 | – | `=>` lambdas — `(params)` or single-ident form; decided by the §4.2 scan *before* the Pratt loop starts, not by binding powers |
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
without special cases.

### 4.1 Lookahead audit — every local decision peeks ≤ 4 tokens

rut's grammar makes this cheap: statements are keyword-led, `{` never
starts an expression (no block-expressions; dataclass literals start
with `Ident`), there are no tuples (RFC 0009), imports are flat, paths
are `.`-dotted.

| decision | mechanism |
|---|---|
| item dispatch | peek 1 (`import` `let` `enum` `dataclass` `class` `interface` `fn`) |
| stmt vs expr-stmt | peek 1 (leading keyword: `let` `if` `while` `for` `return` `when`) |
| `for`-of vs `for`-c | peek 4: `for ( let Ident <of or =>` |
| instance vs class method | peek 4: `fn Ident ( <mut? self? …>` |
| dataclass literal vs path expr | peek 2: `Ident {` ⇒ Struct literal (classes have no instance literal, RFC 0009) |
| `when`-arm body form | peek 1 after `->` (`{` ⇒ block arm, else expr arm) |
| postfix loop step | peek 1 (`.` `(` `[` `?`) |
| assignment target | no lookahead — parse the expression, then validate (path/index) |
| declaration-mode rejections | keyword-led, peek 1 (RFC 0029 §2) |

The two decisions that cannot be answered in 4 tokens are scans (§4.2).

### 4.2 The two scans — peek far, commit once, never guess

- **Lambda vs parenthesized expression** (`(a, b): T => …` vs
  `(a + b) * 2`): scan read-only to the matching `)`; it is a lambda iff
  the tokens form a valid parameter list **and** `=>` follows (`: Type`
  may sit between). If not a lambda, a comma inside the parens errors
  *at the comma* — there are no tuples (RFC 0009) — with a note: "if you
  meant a lambda, add `=>`". (This also corrects the original design's
  claim that one token of lookahead disambiguates call vs lambda — you
  must see past the `)`.)
- **Generic call vs `<` comparison** (`Vec<f32>(n)` vs `a < b > (c)`):
  scan read-only from the `<`, tracking angle depth — a `Shr`/`Shl`
  consumed where a closer/opener is expected counts as two (span
  arithmetic), so `Vec<Vec<i32>>` needs no re-lexing and no glued
  tokens. Generic arguments commit iff depth returns to 0 on a `>`
  **and the very next token is `(`** — TypeScript's rule. `a < b > (c)`
  is then not expressible unparenthesized: write `(a < b) > (c)`. When
  `f<a>(b)` commits but `f` is not generic, the resolver reports it with
  a "wrap the left side in parens" note (RFC 0031).

Scans never mutate parser state: they look, answer yes/no, and the
parser then moves forward once. Peeking at arbitrary distance is
allowed; *guessing* — checkpoint, parse, roll back — is forbidden.

### 4.3 Monotone cursor, depth budget

The cursor is non-decreasing for the whole parse, recovery included;
debug builds assert it. The token-slice checkpoint/restore of the
original design (backtracking one token sequence to resolve
`Vec<f32>(n)`) is **deleted** — the API does not exist, so rollback is
unrepresentable. Depth is bounded the same way: frame-stack depth and
the lexer's bracket depth carry budgets, and exceeding one is a normal
`Diag` ("nesting too deep", span at the offending opener), never a host
stack overflow (C3) — malformed or hostile source cannot crash the
embedding process at parse time (RFC 0035 §3).

### 4.4 Holes and `when` arms

An `f"..."` hole's tokens (§1.1) parse as a frame whose input is the
hole's sub-slice — the same loop, no nested parse call. `when` in
expression position parses arms as `pattern -> expr ,` — the comma is
required between expression arms and forbidden after block arms
(RFC 0008 §2).

## 5. AST — flat arena, ESTree-style

C1 in full: nodes are plain records in one arena; children are `NodeId`s
into it, never `Box<Expr>`. Construction is bottom-up — a node is pushed
after its children exist (the Pratt reductions and frame pops of §4 do
exactly that) — so drop/clone are flat `Vec` ops: no input can overflow
the host stack through the tree itself. Spans on every node; no `String`
keys — names are `IdentId`s into an interner, resolved later (RFC 0031
§1). Sketch (abbreviated — the full enum is mechanical):

```rust
struct Ast { nodes: Vec<Node>, idents: Vec<IdentData>, root: NodeId }
struct NodeId(u32);                       // index into nodes — nothing else
struct IdentId(u32);                      // interner index
struct Node { span: Span, kind: NodeKind }

enum NodeKind {
    // Items (§2) — children as NodeId / Vec<NodeId>:
    Import  { names: Vec<IdentId>, from: LitId },
    Let     { vis, name: IdentId, ty: Option<NodeId>, init: NodeId },  // module-level
    Enum    { vis, name: IdentId, members: Vec<IdentId> },
    Dataclass{ vis, name, generics: Vec<IdentId>, implements: Vec<NodeId>,
               fields: Vec<NodeId>, methods: Vec<NodeId> },            // RFC 0009
    Class   { vis, name, generics, implements: Vec<NodeId>,
              members: Vec<NodeId> },
    Interface{ vis, name, generics, requires: Vec<NodeId>,  // RFC 0012 §2
               methods: Vec<NodeId> },
    Fn      { vis, is_suspend, name, generics,
              params: Vec<NodeId>, ret: Option<NodeId>, body: NodeId },
    SurfaceFn  { vis, linkage, name, generics, params: Vec<NodeId>, ret: NodeId },
    SurfaceClass{ vis, linkage, name, params: Vec<NodeId>,
                  members: Vec<NodeId> },  // linkage = Host | Extern —
                                            // ONLY in .d.rut (RFC 0029 §2)
    // Members & statements — same discipline:
    FieldDecl{ is_mut, name: IdentId, ty: NodeId, init: Option<NodeId> },
    Constructor{ is_suspend, params: Vec<NodeId>, ret: NodeId, body: NodeId },
    MethodDecl{ has_self, is_mut, sig: NodeId, body: NodeId },
              // is_mut = `mut self`: may assign fields; interface members
              // are always dynamic (RFC 0012). No is_static: absence of
              // `self` IS the class-method case (RFC 0010 §2)
    LetStmt { is_mut, name: IdentId, ty: Option<NodeId>, init: NodeId },
    If{..}, While{..}, ForOf{..}, ForC{..}, Return{..}, When{..},
    ExprStmt(NodeId),
    // Expressions (§4):
    Lit(Lit), Path(Vec<IdentId>),
    Call    { callee: NodeId, generics: Vec<NodeId>, args: Vec<NodeId> },
    Method  { recv: NodeId, name: IdentId, generics: Vec<NodeId>, args: Vec<NodeId> },
    Field   { recv: NodeId, name: IdentId },             // field access
    Index   { recv: NodeId, idx: NodeId },
    Unary{..}, Binary{ op: Tok, lhs: NodeId, rhs: NodeId }, Assign{..},
    Lambda  { params: Vec<NodeId>, ret: Option<NodeId>, body: NodeId },
    When    { scrut: NodeId, arms: Vec<NodeId> },
    Try     { expr: NodeId },             // postfix `?`
    FStr    { parts: Vec<FPartAst> },     // holes are NodeIds here
    Struct  { ty: Vec<IdentId>, fields: Vec<(IdentId, NodeId)> },  // dataclass
                                          // literal — classes have none
    Array   { elems: Vec<NodeId> },       // fixed-array literal: [e1..en] : Array<T, n>
    Cast    { ty: NodeId, expr: NodeId }, // i32(x) etc. — a Call on a type
}                                         // name, resolved to Cast (RFC 0031 §1)
```

Stable `NodeId`s are what the rest of the pipeline wants: the resolver
keys its side tables by `NodeId` (RFC 0031 §1 walks the arena), diag
labels reach spans through them (§6), and symbolication / LSP / formatter
hooks (§7, RFC 0036) hold stable, copyable references — no reparenting,
no `Box` juggling.

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
  one. Resync only ever skips *forward* — the §4.3 monotone-cursor
  invariant survives errors. Compilation stops before IR if any diag exists.
- Suggestions: `did you mean \`when\`?` for `switch`/`match`; "bind it to a
  name first" for string literals in `f"..."` holes; "classes have no
  instance literal — use \`Circle(..)\`" for `Circle { .. }` on a class.

## 7. Testing & tooling hooks

- The lexer and parser are pure functions (`&str -> (Vec<Token>, Vec<Diag>)`,
  `&[Token] -> (Ast, Vec<Diag>)`) — fuzzable, usable in an LSP or formatter
  without a VM.
- Parser invariants, tested: the cursor is monotone (debug assert; the
  checkpoint API no longer exists, so rollback is unrepresentable) and the
  depth budgets fire as Diags — the corpus includes 100k-deep `((((`,
  `[[[[`, `{{{{`, unary chains, and nested generic args, each yielding one
  clean "nesting too deep" diag; fuzz targets assert no host stack overflow
  and no rollback.
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
  default, RFC 0015 §4, so `@repr` is NOT planned). Decorators were
  later designed for reflection policy (TypeId identity, protocol
  downcasts, lazy class recipes) and **rejected for v1** — RFC 0037
  OQ-1 records the history; reflection shipped on interfaces instead.
- OQ-3: the depth budget value (C3). Proposed: a single NEST_MAX = 1024
  shared by parser frame depth and lexer bracket depth — deep enough for
  generated code, shallow enough that exceeding it is pathological.
- OQ-4: a lint for generic-scan near-misses (§4.2): source shaped like
  `a < b > (c)` parses as comparisons; the lint would suggest parens
  when the trailing `(c)` makes the comparison-chain reading suspicious.
