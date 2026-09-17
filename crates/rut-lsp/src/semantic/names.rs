//! The AST pass — identifier classes from a flat walk of the arena;
//! name positions are recovered by matching `Ident` tokens inside each
//! node's span.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use super::legend::{is_cap, is_keyword, is_primitive_ty, owned_by_tokens, TokenType};
use super::recover::{push_name, toks_in};

/// The AST walk — a flat iteration over the arena (every node is present;
/// no recursion needed). Only name-bearing nodes act.
pub(crate) fn classify_ast(toks: &[Token], ast: &Ast, out: &mut Vec<(Span, TokenType)>) {
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
            push_name(toks, span, ast.name(*name), TokenType::Trait, out, false)
        }
        ItemKind::BuiltinTrait { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Trait, out, false)
        }
        ItemKind::BuiltinTy { name, .. } | ItemKind::SurfaceDataclass { name, .. } => {
            push_name(toks, span, ast.name(*name), TokenType::Class, out, false)
        }
        // methods classify via their own Member nodes; use names need
        // resolution (M2) — left unclassified
        ItemKind::Impl { .. } | ItemKind::Use { .. } | ItemKind::Module { .. } => {}
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
        // `*T` / `(A, B)` — the puncts carry no classification; the
        // element types classify themselves
        TypeKind::TyPtr { .. } | TypeKind::TyTuple { .. } => {}
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
        // struct literal labels (`Point { x: 1 }`) lead their values
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
        // fns and fn-typed locals read alike); casts `x as i32` keep
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
                        // the cast family `x as i32` / `x as f64` reads
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
