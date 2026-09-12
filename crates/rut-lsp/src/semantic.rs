//! Semantic tokens + document symbols — the M1 classifier (RFC 0041 §2,
//! rut-lsp). Pure rut-side, no LSP types: token-level classes come from the
//! lexed stream (keywords by text — RFC 0002 §4/§5; literals; f-string
//! tiling with fully-lexed holes — RFC 0030 §1.1), identifier classes from
//! a flat walk of the AST arena. Names are interner ids whose nodes carry
//! whole-construct spans, so name positions are **recovered**: a bounded
//! scan for the matching `Ident` token inside the node's span (the parser
//! itself matched by text, so this is reliable; a miss skips the name —
//! never a wrong color). Value-position paths have no resolution in M1 —
//! they classify by capitalization convention, like a TextMate grammar
//! would, and full resolution lands with M2 modules.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::{FPart, Tok, Token};

// ---- the legend ----

/// Semantic token types, in legend order. **Standard types only** — stock
/// themes color them in every LSP editor without scope mappings (the VS
/// Code extension still ships `semanticTokenScopes` as belt & braces).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenType {
    Keyword,
    Number,
    String,
    Type,
    Function,
    Method,
    Variable,
    Parameter,
    Property,
    Enum,
    EnumMember,
    Class,
    Interface,
    Operator,
}

pub const ALL: [TokenType; 14] = [
    TokenType::Keyword,
    TokenType::Number,
    TokenType::String,
    TokenType::Type,
    TokenType::Function,
    TokenType::Method,
    TokenType::Variable,
    TokenType::Parameter,
    TokenType::Property,
    TokenType::Enum,
    TokenType::EnumMember,
    TokenType::Class,
    TokenType::Interface,
    TokenType::Operator,
];

impl TokenType {
    pub fn name(self) -> &'static str {
        match self {
            TokenType::Keyword => "keyword",
            TokenType::Number => "number",
            TokenType::String => "string",
            TokenType::Type => "type",
            TokenType::Function => "function",
            TokenType::Method => "method",
            TokenType::Variable => "variable",
            TokenType::Parameter => "parameter",
            TokenType::Property => "property",
            TokenType::Enum => "enum",
            TokenType::EnumMember => "enumMember",
            TokenType::Class => "class",
            TokenType::Interface => "interface",
            TokenType::Operator => "operator",
        }
    }
    pub fn index(self) -> u32 {
        ALL.iter().position(|t| *t == self).unwrap() as u32
    }
    pub fn legend() -> Vec<&'static str> {
        ALL.iter().map(|t| t.name()).collect()
    }
}

// ---- word classes (RFC 0002 §4 surface facts) ----

/// The highlighting keyword set: the grammar's reserved words
/// (`rut_parser::is_reserved_kw`) plus the contextual/statement words the
/// parser matches by text (`break`, `continue`, `as`, `super`, `default`)
/// and the receiver `self`. `true`/`false` lex as `Tok::Bool` and are
/// classified there (keyword constants).
pub fn is_keyword(s: &str) -> bool {
    matches!(s, "self" | "break" | "continue" | "as" | "super" | "default")
        || rut_parser::is_reserved_kw(s)
}

pub fn is_primitive_ty(s: &str) -> bool {
    // canonical table lives with the grammar (shared with `host primitive`)
    rut_parser::is_primitive_ty(s)
}

/// A word the token layer already owns by text (keywords, `Self`) — never
/// a name position. Primitives (`unit`, `string`, …) are contextual type
/// names, NOT reserved: a field or param may legally carry one, and the
/// AST pass then overrides the token layer's `type` class.
fn owned_by_tokens(s: &str) -> bool {
    is_keyword(s) || s == "Self"
}

fn is_cap(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_uppercase())
}

/// Token-level classification — no AST needed. FStr is tiled separately
/// (`classify_token`); structural punctuation stays unclassified (themes
/// already paint it, and it keeps token streams small).
pub fn token_type(tok: &Tok) -> Option<TokenType> {
    match tok {
        Tok::Bool(_) => Some(TokenType::Keyword),
        Tok::Int(..) | Tok::Float(..) => Some(TokenType::Number),
        Tok::Str(_) | Tok::RawStr(_) | Tok::Char(_) => Some(TokenType::String),
        Tok::Ident(s) => {
            if s == "Self" || is_primitive_ty(s) {
                Some(TokenType::Type)
            } else if is_keyword(s) {
                Some(TokenType::Keyword)
            } else {
                None
            }
        }
        Tok::Eof => None,
        Tok::LParen
        | Tok::RParen
        | Tok::LBrace
        | Tok::RBrace
        | Tok::LBracket
        | Tok::RBracket
        | Tok::Comma
        | Tok::Semi
        | Tok::Colon
        | Tok::Dot => None,
        // arrows, `?` `@` `~`, and the whole arithmetic/bitwise/wrapping
        // families (RFC 0004 §3)
        _ => Some(TokenType::Operator),
    }
}

// ---- classification ----

/// Classify a whole document: token-level classes, then AST-level name
/// classes (which win at equal positions). Output is sorted by start and
/// guaranteed non-overlapping — LSP's semantic-token contract.
pub fn classify(toks: &[Token], ast: &Ast) -> Vec<(Span, TokenType)> {
    let mut out: Vec<(Span, TokenType)> = Vec::new();
    for t in toks {
        classify_token(t, &mut out);
    }
    // name recovery runs over a FLAT token stream — f-string holes are
    // fully lexed with real spans (RFC 0030 §1.1) but nested inside the
    // FStr token; the AST's hole expressions recover their names against
    // the spliced view
    let flat = flatten_holes(toks);
    classify_ast(&flat, ast, &mut out);
    finalize(out)
}

/// Splice f-string holes into the stream (recursively — an f-string may
/// appear inside a hole). Literal chunks produce no tokens, so string
/// text is never mistaken for a name.
fn flatten_holes(toks: &[Token]) -> Vec<Token> {
    let mut flat = Vec::with_capacity(toks.len());
    for t in toks {
        flatten_into(&mut flat, t);
    }
    flat
}

fn flatten_into(flat: &mut Vec<Token>, t: &Token) {
    if let Tok::FStr(f) = &t.tok {
        for part in &f.parts {
            if let FPart::Hole(hole) = part {
                for ht in hole {
                    flatten_into(flat, ht);
                }
            }
        }
    } else {
        flat.push(t.clone());
    }
}

/// One token; f-strings tile exactly — string for the literal gaps, then
/// recursion into each hole's fully-lexed token stream (real spans inside
/// the literal's span, RFC 0030 §1.1).
fn classify_token(t: &Token, out: &mut Vec<(Span, TokenType)>) {
    if let Tok::FStr(f) = &t.tok {
        let mut cursor = t.span.lo;
        for part in &f.parts {
            if let FPart::Hole(hole) = part {
                if let (Some(first), Some(last)) = (hole.first(), hole.last()) {
                    if first.span.lo > cursor {
                        out.push((Span::new(cursor, first.span.lo), TokenType::String));
                    }
                    for ht in hole {
                        classify_token(ht, out);
                    }
                    cursor = last.span.hi;
                }
            }
        }
        if t.span.hi > cursor {
            out.push((Span::new(cursor, t.span.hi), TokenType::String));
        }
    } else if let Some(ty) = token_type(&t.tok) {
        out.push((t.span, ty));
    }
}

/// The AST walk — a flat iteration over the arena (every node is present;
/// no recursion needed). Only name-bearing nodes act.
fn classify_ast(toks: &[Token], ast: &Ast, out: &mut Vec<(Span, TokenType)>) {
    for node in &ast.nodes {
        let span = node.span;
        if span.hi <= span.lo {
            continue; // error-recovery placeholder
        }
        match &node.kind {
            Kind::Item(item) => classify_item(toks, ast, span, item, out),
            Kind::Member(m) => classify_member(toks, ast, span, m, out),
            Kind::Stmt(s) => classify_stmt(toks, ast, span, s, out),
            Kind::Type(t) => classify_type(toks, ast, span, t, out),
            Kind::Pat(p) => classify_pat(toks, ast, span, p, out),
            Kind::Expr(e) => classify_expr(toks, ast, span, e, out),
            Kind::Arm(_) => {}
        }
    }
}

fn classify_item(
    toks: &[Token],
    ast: &Ast,
    span: Span,
    item: &ItemKind,
    out: &mut Vec<(Span, TokenType)>,
) {
    match item {
        ItemKind::Fn(d) => push_name(toks, span, ast.name(d.name), TokenType::Function, out, false),
        ItemKind::SurfaceFn { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Function, out, false)
        }
        ItemKind::ModuleLet { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Variable, out, false)
        }
        ItemKind::Enum { name, members, .. } => {
            // first plain ident is the name; the declared members follow,
            // matched by text in order (discriminants are Int tokens)
            let mut name_done = false;
            for t in toks_in(toks, span) {
                if let Tok::Ident(s) = &t.tok {
                    if is_keyword(s) {
                        continue;
                    }
                    if !name_done && s == ast.name(*name) {
                        out.push((t.span, TokenType::Enum));
                        name_done = true;
                    } else if members.iter().any(|(m, _)| ast.name(*m) == s) {
                        out.push((t.span, TokenType::EnumMember));
                    }
                }
            }
        }
        ItemKind::Dataclass { name, .. } | ItemKind::Class { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Class, out, false)
        }
        ItemKind::Trait { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Interface, out, false)
        }
        ItemKind::SurfaceClass { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Class, out, false)
        }
        // methods classify via their own Member nodes; import names need
        // resolution (M2) — left unclassified
        ItemKind::Impl { .. } | ItemKind::Import { .. } | ItemKind::Module { .. } => {}
    }
}

fn classify_member(
    toks: &[Token],
    ast: &Ast,
    span: Span,
    m: &MemberKind,
    out: &mut Vec<(Span, TokenType)>,
) {
    match m {
        MemberKind::FieldDecl(d) => {
            push_name(toks, span, ast.name(d.name), TokenType::Property, out, false)
        }
        MemberKind::MethodDecl(d) => {
            push_name(toks, span, ast.name(d.name), TokenType::Method, out, false)
        }
        MemberKind::Param(d) => {
            push_name(toks, span, ast.name(d.name), TokenType::Parameter, out, false)
        }
        // `self` is token-classified as a keyword everywhere it appears
        MemberKind::SelfParam(_) => {}
    }
}

fn classify_stmt(
    toks: &[Token],
    ast: &Ast,
    span: Span,
    s: &StmtKind,
    out: &mut Vec<(Span, TokenType)>,
) {
    match s {
        StmtKind::LetStmt { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Variable, out, false)
        }
        StmtKind::ForOf { var, .. } | StmtKind::ForC { var, .. } => {
            push_name(toks, span, ast.name(*var), TokenType::Variable, out, false)
        }
        _ => {}
    }
}

fn classify_type(
    toks: &[Token],
    ast: &Ast,
    span: Span,
    t: &TypeKind,
    out: &mut Vec<(Span, TokenType)>,
) {
    match t {
        // path segments appear in source order; generic args are their own
        // Type nodes and classify themselves
        TypeKind::TyPath { segs, .. } => {
            classify_path_segs(toks, ast, span, segs, out, TypeRule::Type);
        }
        TypeKind::TyFn { .. } | TypeKind::TyConst(_) => {}
    }
}

fn classify_pat(toks: &[Token], ast: &Ast, span: Span, p: &PatKind, out: &mut Vec<(Span, TokenType)>) {
    match p {
        // path segments in order — the last names the member, the rest the
        // type it rides on (`Color.Red`, `Option<T>.Some`); constructor
        // args are binding names (variables)
        PatKind::PatPath { segs } => {
            classify_path_segs(toks, ast, span, segs, out, TypeRule::EnumLast);
        }
        PatKind::PatCtor { segs, args } => {
            classify_path_segs(toks, ast, span, segs, out, TypeRule::EnumLast);
            let names: Vec<&str> = args.iter().filter_map(|a| a.map(|id| ast.name(id))).collect();
            let mut i = 0;
            for tk in toks_in(toks, span) {
                if i >= names.len() {
                    break;
                }
                if let Tok::Ident(s) = &tk.tok {
                    if s == names[i] && !owned_by_tokens(s) {
                        out.push((tk.span, TokenType::Variable));
                        i += 1;
                    }
                }
            }
        }
        PatKind::PatLit(..) | PatKind::PatWild | PatKind::PatElse => {}
    }
}

fn classify_expr(
    toks: &[Token],
    ast: &Ast,
    span: Span,
    e: &ExprKind,
    out: &mut Vec<(Span, TokenType)>,
) {
    match e {
        // `d.draw(g)`, `Vec.from(..)` — the name trails its receiver,
        // so recover the LAST match
        ExprKind::Method { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Method, out, true)
        }
        ExprKind::Field { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Property, out, true)
        }
        // dataclass literal labels (`Point { x: 1 }`) lead their values
        ExprKind::Struct { fields, .. } => {
            for (fid, _) in fields {
                push_name(toks, span, ast.name(*fid), TokenType::Property, out, false);
            }
        }
        // dotted value paths (`Color.Red`, `self.center.x`) — capitalization
        // convention until resolution exists (M2)
        ExprKind::Path { segs } => {
            classify_path_segs(toks, ast, span, segs, out, TypeRule::Value);
        }
        // a bare call `name(args)` colors its callee as a function (free
        // fns and fn-typed locals read alike); conversions `i32(x)` keep
        // their type color via the primitive guard below
        ExprKind::Call { callee, .. } => {
            if let ExprKind::Path { segs } = ast.expr(*callee) {
                if segs.len() == 1 {
                    let name = ast.name(segs[0].name);
                    if !owned_by_tokens(name) && !is_primitive_ty(name) {
                        push_name(toks, span, name, TokenType::Function, out, false);
                    }
                }
            }
        }
        _ => {}
    }
}

/// How the segments of a path classify.
#[derive(Clone, Copy, PartialEq)]
enum TypeRule {
    /// every segment is a type (type positions)
    Type,
    /// last segment names an enum member (patterns)
    EnumLast,
    /// value position: capitalized prefix = type, capitalized last =
    /// enum member, else property; a lone lowercase seg is a variable
    Value,
}

/// Match path segments against the `Ident` tokens inside `span`, in order.
/// Segments the token layer already owns by text (keywords, `Self`,
/// primitives) advance without emitting — their token class stands.
fn classify_path_segs(
    toks: &[Token],
    ast: &Ast,
    span: Span,
    segs: &[PathSeg],
    out: &mut Vec<(Span, TokenType)>,
    rule: TypeRule,
) {
    let n = segs.len();
    let mut i = 0;
    for tk in toks_in(toks, span) {
        if i >= n {
            break;
        }
        if let Tok::Ident(s) = &tk.tok {
            if s == ast.name(segs[i].name) {
                if owned_by_tokens(s) {
                    i += 1;
                    continue;
                }
                let ty = match rule {
                    TypeRule::Type => TokenType::Type,
                    TypeRule::EnumLast => {
                        if i + 1 == n { TokenType::EnumMember } else { TokenType::Type }
                    }
                    TypeRule::Value => {
                        // primitives stay types even in call position —
                        // the conversion family `i32(x)` / `f64(x)` reads
                        // as a type operation, not a variable
                        if is_primitive_ty(s) {
                            TokenType::Type
                        } else if i + 1 == n {
                            if is_cap(s) { TokenType::EnumMember }
                            else if n == 1 { TokenType::Variable }
                            else { TokenType::Property }
                        } else if is_cap(s) {
                            TokenType::Type
                        } else {
                            TokenType::Variable
                        }
                    }
                };
                out.push((tk.span, ty));
                i += 1;
            }
        }
    }
}

// ---- name recovery + helpers ----

/// Tokens fully inside `span`, in source order.
fn toks_in<'a>(toks: &'a [Token], span: Span) -> impl Iterator<Item = &'a Token> {
    toks.iter()
        .skip_while(move |t| t.span.lo < span.lo)
        .take_while(move |t| t.span.hi <= span.hi)
}

/// Find the span of the `Ident` token whose text is `name` inside `span` —
/// `last` for trailing positions (method/field access), else the first.
fn find_name(toks: &[Token], span: Span, name: &str, last: bool) -> Option<Span> {
    let mut found = None;
    for t in toks_in(toks, span) {
        if let Tok::Ident(s) = &t.tok {
            if s == name {
                found = Some(t.span);
                if !last {
                    break;
                }
            }
        }
    }
    found
}

fn push_name(
    toks: &[Token],
    span: Span,
    name: &str,
    ty: TokenType,
    out: &mut Vec<(Span, TokenType)>,
    last: bool,
) {
    if owned_by_tokens(name) {
        return; // not a name position
    }
    if let Some(s) = find_name(toks, span, name, last) {
        out.push((s, ty));
    }
}

/// Sort by start; drop empty and overlapping spans. At equal starts the
/// entry pushed LAST wins (the AST pass runs after the token pass, so
/// identifier classes beat literal classes there); stable sort keeps the
/// push order inside a group.
fn finalize(mut raw: Vec<(Span, TokenType)>) -> Vec<(Span, TokenType)> {
    raw.sort_by_key(|(s, _)| s.lo);
    // walk right-to-left so the group's last entry is taken first
    let mut kept: Vec<(Span, TokenType)> = Vec::with_capacity(raw.len());
    let mut i = raw.len();
    while i > 0 {
        // start of the equal-lo group that ends at i-1
        let lo = raw[i - 1].0.lo;
        let mut j = i - 1;
        while j > 0 && raw[j - 1].0.lo == lo {
            j -= 1;
        }
        let (sp, ty) = raw[i - 1]; // last of the group — AST's entry
        if sp.hi > sp.lo {
            let fits = kept.last().map(|(k, _)| sp.hi <= k.lo).unwrap_or(true);
            if fits {
                kept.push((sp, ty));
            }
        }
        i = j;
    }
    kept.reverse();
    kept
}

// ---- document symbols ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymKind {
    Function,
    Variable,
    Enum,
    EnumMember,
    Class,
    Field,
    Method,
    Interface,
    Module,
}

/// An outline entry — lsp-free so the classifier stays testable without a
/// protocol dependency.
#[derive(Clone, Debug)]
pub struct RawSymbol {
    pub name: String,
    pub kind: SymKind,
    pub range: Span,
    /// the name's own span (recovered); falls back to `range`
    pub selection: Span,
    pub children: Vec<RawSymbol>,
}

pub fn symbols(toks: &[Token], ast: &Ast) -> Vec<RawSymbol> {
    ast.module_items(ast.root)
        .iter()
        .filter_map(|h| item_symbol(toks, ast, *h))
        .collect()
}

fn item_symbol(toks: &[Token], ast: &Ast, h: NodeHandle<AnyItem>) -> Option<RawSymbol> {
    let span = ast.span(h.id());
    let sym = |name: &str, kind, selection: Option<Span>, children| RawSymbol {
        name: name.to_string(),
        kind,
        range: span,
        selection: selection.unwrap_or(span),
        children,
    };
    match ast.item(h) {
        ItemKind::Fn(d) => Some(sym(
            ast.name(d.name),
            SymKind::Function,
            find_name(toks, span, ast.name(d.name), false),
            vec![],
        )),
        ItemKind::ModuleLet { name, .. } => Some(sym(
            ast.name(*name),
            SymKind::Variable,
            find_name(toks, span, ast.name(*name), false),
            vec![],
        )),
        ItemKind::Enum { name, members, .. } => {
            let children = members
                .iter()
                .map(|(m, _)| {
                    let text = ast.name(*m);
                    sym(text, SymKind::EnumMember, find_name(toks, span, text, false), vec![])
                })
                .collect();
            Some(sym(ast.name(*name), SymKind::Enum, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::Dataclass { name, fields, methods, .. }
        | ItemKind::Class { name, fields, methods, .. } => {
            let children = member_symbols(toks, ast, fields, methods);
            Some(sym(ast.name(*name), SymKind::Class, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::Trait { name, methods, .. } => {
            let children = methods
                .iter()
                .map(|m| method_symbol(toks, ast, *m))
                .collect();
            Some(sym(ast.name(*name), SymKind::Interface, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::Impl { trait_ref, target, methods, .. } => {
            let children = methods
                .iter()
                .map(|m| method_symbol(toks, ast, *m))
                .collect();
            let name = format!("impl {} for {}", ty_text(ast, *trait_ref), ty_text(ast, *target));
            Some(sym(&name, SymKind::Module, None, children))
        }
        ItemKind::SurfaceFn { name, .. } => Some(sym(
            ast.name(*name),
            SymKind::Function,
            find_name(toks, span, ast.name(*name), false),
            vec![],
        )),
        ItemKind::SurfaceClass { name, members, .. } => {
            let children = members.iter().map(|m| method_symbol(toks, ast, *m)).collect();
            Some(sym(ast.name(*name), SymKind::Class, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::Import { .. } | ItemKind::Module { .. } => None,
    }
}

fn member_symbols(
    toks: &[Token],
    ast: &Ast,
    fields: &[NodeHandle<FieldDeclNode>],
    methods: &[NodeHandle<MethodDeclNode>],
) -> Vec<RawSymbol> {
    let mut out = Vec::new();
    for f in fields {
        let span = ast.span(f.id());
        let d = ast.field_decl(*f);
        out.push(RawSymbol {
            name: ast.name(d.name).to_string(),
            kind: SymKind::Field,
            range: span,
            selection: find_name(toks, span, ast.name(d.name), false).unwrap_or(span),
            children: vec![],
        });
    }
    for m in methods {
        out.push(method_symbol(toks, ast, *m));
    }
    out
}

fn method_symbol(toks: &[Token], ast: &Ast, m: NodeHandle<MethodDeclNode>) -> RawSymbol {
    let span = ast.span(m.id());
    let d = ast.method_decl(m);
    RawSymbol {
        name: ast.name(d.name).to_string(),
        kind: SymKind::Method,
        range: span,
        selection: find_name(toks, span, ast.name(d.name), false).unwrap_or(span),
        children: vec![],
    }
}

/// Best-effort type text for impl headers (`impl Drawable for Circle`).
fn ty_text(ast: &Ast, h: NodeHandle<AnyTy>) -> String {
    match ast.ty(h) {
        TypeKind::TyPath { segs, is_dyn } => {
            let names: Vec<&str> = segs.iter().map(|s| ast.name(s.name)).collect();
            if *is_dyn {
                format!("dyn {}", names.join("."))
            } else {
                names.join(".")
            }
        }
        TypeKind::TyFn { .. } => "fn(..)".to_string(),
        TypeKind::TyConst(_) => "const".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rut_parser::{parse, Mode};

    fn classify_src(src: &str) -> Vec<(Span, TokenType)> {
        let (toks, _) = rut_lexer::lexer::lex(src);
        let (ast, _) = parse(src, Mode::Impl);
        classify(&toks, &ast)
    }

    fn span_text(src: &str, sp: Span) -> &str {
        &src[sp.lo as usize..sp.hi as usize]
    }

    fn find(src: &str, spans: &[(Span, TokenType)], text: &str) -> Vec<TokenType> {
        spans
            .iter()
            .filter(|(sp, _)| span_text(src, *sp) == text)
            .map(|(_, ty)| *ty)
            .collect()
    }

    #[test]
    fn output_is_sorted_and_disjoint() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/basic/grammar-tour.rut");
        let src = std::fs::read_to_string(&p).unwrap();
        let spans = classify_src(&src);
        let mut prev_hi = 0;
        for (sp, _) in &spans {
            assert!(sp.lo < sp.hi, "empty span at {}", sp.lo);
            assert!(sp.lo >= prev_hi, "overlap at {}", sp.lo);
            assert!(sp.hi as usize <= src.len(), "out of file");
            prev_hi = sp.hi;
        }
    }

    #[test]
    fn decls_and_keywords() {
        let src = "enum Color { Red, Green }\nfn area(r: f32) -> f32 { return r * 2.0f32; }\n";
        let spans = classify_src(src);
        assert_eq!(find(src, &spans, "enum"), vec![TokenType::Keyword]);
        assert_eq!(find(src, &spans, "Color"), vec![TokenType::Enum]);
        assert_eq!(find(src, &spans, "Red"), vec![TokenType::EnumMember]);
        assert_eq!(find(src, &spans, "Green"), vec![TokenType::EnumMember]);
        assert_eq!(find(src, &spans, "area"), vec![TokenType::Function]);
        // the decl is a parameter; body uses are lowercase single-seg paths
        assert_eq!(find(src, &spans, "r"), vec![TokenType::Parameter, TokenType::Variable]);
        assert_eq!(find(src, &spans, "f32"), vec![TokenType::Type, TokenType::Type]);
        assert_eq!(find(src, &spans, "2.0f32"), vec![TokenType::Number]);
        assert_eq!(find(src, &spans, "*"), vec![TokenType::Operator]);
    }

    #[test]
    fn members_fields_and_locals() {
        let src = "class Circle {\n    pub r: f32;\n    fn scale(mut self, k: f32) -> Self { self.r = self.r * k; }\n}\n";
        let spans = classify_src(src);
        assert_eq!(find(src, &spans, "Circle"), vec![TokenType::Class]);
        // the declared field + both `self.r` path tails
        assert_eq!(find(src, &spans, "r"), vec![TokenType::Property, TokenType::Property, TokenType::Property]);
        assert_eq!(find(src, &spans, "scale"), vec![TokenType::Method]);
        assert_eq!(find(src, &spans, "k"), vec![TokenType::Parameter, TokenType::Variable]);
        // `self` stays a keyword everywhere (param + both path prefixes)
        assert_eq!(find(src, &spans, "self"), vec![TokenType::Keyword, TokenType::Keyword, TokenType::Keyword]);
        assert_eq!(find(src, &spans, "Self"), vec![TokenType::Type]);
    }

    #[test]
    fn f_string_tiles_and_lexes_holes() {
        let src = "fn f() -> unit { log.info(f\"{color_name(Color.Red)} area={c.area()}\"); }";
        let spans = classify_src(src);
        // the whole literal tiles with strings + hole tokens, no overlap
        let lit = src.find("f\"").unwrap() as u32;
        let lit_end = src.find("\");").unwrap() as u32 + 1; // past the closing quote
        let inside: Vec<_> = spans
            .iter()
            .filter(|(sp, _)| sp.lo >= lit && sp.hi <= lit_end)
            .collect();
        assert!(!inside.is_empty());
        let mut prev_hi = lit;
        for (sp, _) in &inside {
            assert!(sp.lo >= prev_hi, "gap/overlap inside f-string at {}", sp.lo);
            prev_hi = sp.hi;
        }
        assert_eq!(prev_hi, lit_end);
        // hole contents classify: `Color` type, `Red` enumMember, `area` method
        assert_eq!(find(src, &spans, "Color"), vec![TokenType::Type]);
        assert_eq!(find(src, &spans, "Red"), vec![TokenType::EnumMember]);
        assert_eq!(find(src, &spans, "area"), vec![TokenType::Method]);
    }

    #[test]
    fn types_struct_literals_and_patterns() {
        let src = "enum Color { Red, Green }\n\
                   dataclass Point { x: f64; y: f64; }\n\
                   fn make(c: Color) -> Point {\n\
                       let p = Point { x: 1, y: 2 };\n\
                       return when (c) { Color.Red -> p, _ -> p, };\n\
                   }\n";
        let spans = classify_src(src);
        // decl name, param type, literal type
        assert_eq!(find(src, &spans, "Point"), vec![TokenType::Class, TokenType::Type, TokenType::Type]);
        // field decls + literal labels
        assert_eq!(find(src, &spans, "x"), vec![TokenType::Property, TokenType::Property]);
        // enum decl name, param type, pattern type
        assert_eq!(find(src, &spans, "Color"), vec![TokenType::Enum, TokenType::Type, TokenType::Type]);
        assert_eq!(find(src, &spans, "Red"), vec![TokenType::EnumMember, TokenType::EnumMember]);
        assert_eq!(find(src, &spans, "c"), vec![TokenType::Parameter, TokenType::Variable]);
        assert_eq!(find(src, &spans, "p"), vec![TokenType::Variable, TokenType::Variable, TokenType::Variable]);
    }

    #[test]
    fn recovery_miss_skips_instead_of_guessing() {
        let toks: Vec<Token> = Vec::new();
        let mut out = Vec::new();
        push_name(&toks, Span::new(0, 10), "missing", TokenType::Function, &mut out, false);
        assert!(out.is_empty());
    }

    #[test]
    fn ast_wins_over_token_layer_at_equal_start() {
        // a field named `unit` would be token-classified `type`; the AST
        // pass must replace it with `property`
        let src = "dataclass T { unit: i32; }\n";
        let spans = classify_src(src);
        assert_eq!(find(src, &spans, "unit"), vec![TokenType::Property]);
    }

    #[test]
    fn conversion_calls_keep_their_type_color() {
        // `f64(x)` is the conversion family (RFC 0007) — the primitive
        // stays a type in call position, not a variable
        let src = "fn f(p: Point) -> f64 { return f64(p.x); }\n";
        let spans = classify_src(src);
        assert_eq!(find(src, &spans, "f64"), vec![TokenType::Type, TokenType::Type]);
        assert_eq!(find(src, &spans, "p"), vec![TokenType::Parameter, TokenType::Variable]);
    }

    #[test]
    fn bare_calls_color_their_callee_as_function() {
        // `length(pt)` / `newCanvas()` — the callee gets the function
        // color, overriding the single-seg path's variable class
        let src = "fn go() -> unit { let p = newCanvas(); blit_all(length(p), p); }\n";
        let spans = classify_src(src);
        assert_eq!(find(src, &spans, "newCanvas"), vec![TokenType::Function]);
        assert_eq!(find(src, &spans, "blit_all"), vec![TokenType::Function]);
        assert_eq!(find(src, &spans, "length"), vec![TokenType::Function]);
        assert_eq!(find(src, &spans, "p"), vec![TokenType::Variable, TokenType::Variable, TokenType::Variable]);
    }

    #[test]
    fn symbols_outline() {
        let src = "trait Drawable { fn draw(self, g: Canvas) -> unit; }\nimpl Drawable for Circle { fn draw(self, g: Canvas) -> unit {} }\nenum Color { Red }\npub fn main() -> unit {}\n";
        let (toks, _) = rut_lexer::lexer::lex(src);
        let (ast, _) = parse(src, Mode::Impl);
        let syms = symbols(&toks, &ast);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Drawable", "impl Drawable for Circle", "Color", "main"]);
        assert_eq!(syms[0].kind, SymKind::Interface);
        assert_eq!(syms[0].children.len(), 1);
        assert_eq!(syms[1].kind, SymKind::Module);
        assert_eq!(syms[1].children.len(), 1);
        assert_eq!(syms[2].children.len(), 1);
        assert_eq!(syms[3].kind, SymKind::Function);
        // selectionRange stays inside range
        for s in &syms {
            assert!(s.selection.lo >= s.range.lo && s.selection.hi <= s.range.hi, "{}", s.name);
        }
    }
}
