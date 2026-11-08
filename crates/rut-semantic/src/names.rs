//! The AST pass — identifier classes from a walk of the slot tree;
//! name positions are recovered by matching `Ident` tokens inside each
//! node's span (the classifier stays token-driven by nature: f-string
//! holes tile through the flattened stream, and the recovery rules
//! below are keyed on token order).

use rut_ast::{
    Ast, ElseBranch, EnumNode, ExprRef, FieldDeclNode, FieldRef, FnNode, Handle, ItemRef,
    MethodDeclNode, MethodRef, ModDeclNode, ModInlineNode, ParamRef, PatRef, StmtRef, TyRef,
    UseNode,
};
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use super::legend::{is_cap, is_keyword, is_primitive_ty, owned_by_tokens, TokenType};
use super::recover::{push_name, toks_in};

/// The AST walk — a recursion over the slot tree from the root (the
/// parser's nesting budgets keep the depth shallow). Only
/// name-bearing nodes act; Error slots are skipped everywhere.
pub(crate) fn classify_ast(toks: &[Token], ast: &Ast, out: &mut Vec<(Span, TokenType)>) {
    for item in ast.root().items(ast) {
        classify_item(toks, ast, *item, out);
    }
}

fn classify_item(toks: &[Token], ast: &Ast, item: ItemRef, out: &mut Vec<(Span, TokenType)>) {
    match item {
        ItemRef::Fn(h) => {
            let d = h.get(ast);
            classify_fn(toks, ast, d, out);
        }
        ItemRef::Alias(h) => {
            let d = h.get(ast);
            push_name(toks, d.span, ast.name(d.name), TokenType::Type, out, false);
            classify_ty(toks, ast, d.target, out);
        }
        ItemRef::SurfaceFn(h) => {
            let d = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.name),
                TokenType::Function,
                out,
                false,
            );
            classify_params(toks, ast, &d.params, out);
            if let Some(r) = d.ret {
                classify_ty(toks, ast, r, out);
            }
        }
        ItemRef::ModuleLet(h) => {
            let d = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.name),
                TokenType::Variable,
                out,
                false,
            );
            if let Some(t) = d.ty {
                classify_ty(toks, ast, t, out);
            }
            classify_expr(toks, ast, d.init, out);
        }
        ItemRef::Enum(h) => {
            let d = h.get(ast);
            classify_enum(toks, ast, d, out);
        }
        ItemRef::Struct(h) => {
            let d = h.get(ast);
            push_name(toks, d.span, ast.name(d.name), TokenType::Class, out, false);
            classify_members(toks, ast, &d.fields, &d.methods, out);
        }
        ItemRef::Class(h) => {
            let d = h.get(ast);
            push_name(toks, d.span, ast.name(d.name), TokenType::Class, out, false);
            classify_members(toks, ast, &d.fields, &d.methods, out);
        }
        ItemRef::Interface(h) => {
            let d = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.name),
                TokenType::Interface,
                out,
                false,
            );
            classify_methods(toks, ast, &d.methods, out);
        }
        ItemRef::Impl(h) => {
            let d = h.get(ast);
            classify_ty(toks, ast, d.target, out);
            for (_, b) in &d.bounds {
                classify_ty(toks, ast, *b, out);
            }
            classify_methods(toks, ast, &d.methods, out);
        }
        ItemRef::SurfaceStruct(h) => {
            let d = h.get(ast);
            push_name(toks, d.span, ast.name(d.name), TokenType::Class, out, false);
            classify_fields(toks, ast, &d.fields, out);
        }
        ItemRef::BuiltinTy(h) => {
            let d = h.get(ast);
            push_name(toks, d.span, ast.name(d.name), TokenType::Class, out, false);
            classify_methods(toks, ast, &d.members, out);
        }
        ItemRef::BuiltinPrimitive(h) => {
            let d = h.get(ast);
            push_name(toks, d.span, ast.name(d.name), TokenType::Class, out, false);
            classify_methods(toks, ast, &d.members, out);
        }
        // `builtin impl i32 { .. }` declares no new name — the primitive
        // is already in scope; only its methods classify
        ItemRef::BuiltinImpl(h) => {
            let d = h.get(ast);
            classify_methods(toks, ast, &d.methods, out);
        }
        // mod decls (file and inline) ARE namespaces — their names
        // classify here (the declared head reads like the `layout` in
        // `layout::mk`, which the path law colors too). The inline
        // form's nested items classify through the same recursion.
        ItemRef::ModDecl(h) => {
            let d: &ModDeclNode = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.name),
                TokenType::Namespace,
                out,
                false,
            );
        }
        ItemRef::ModInline(h) => {
            let d: &ModInlineNode = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.name),
                TokenType::Namespace,
                out,
                false,
            );
            for nested in h.items(ast) {
                classify_item(toks, ast, *nested, out);
            }
        }
        ItemRef::Use(h) => {
            let d: &UseNode = h.get(ast);
            classify_use(toks, ast, d, out);
        }
        // the root module is walked from `classify_ast`; an Error slot
        // carries no names
        ItemRef::Module(_) | ItemRef::Error(_) => {}
    }
}

fn classify_fn(toks: &[Token], ast: &Ast, d: &FnNode, out: &mut Vec<(Span, TokenType)>) {
    push_name(
        toks,
        d.span,
        ast.name(d.name),
        TokenType::Function,
        out,
        false,
    );
    for (_, b) in &d.bounds {
        classify_ty(toks, ast, *b, out);
    }
    classify_params(toks, ast, &d.params, out);
    if let Some(r) = d.ret {
        classify_ty(toks, ast, r, out);
    }
    classify_block(toks, ast, d.body, out);
}

/// `pub? enum Name { A, B }` — the enum's own name plus its members:
/// the first plain ident is the name; the declared members follow,
/// matched by text in order
fn classify_enum(toks: &[Token], ast: &Ast, d: &EnumNode, out: &mut Vec<(Span, TokenType)>) {
    let span = d.span;
    let mut name_done = false;
    for t in toks_in(toks, span) {
        if let Tok::Ident(s) = &t.tok {
            if is_keyword(s) {
                continue;
            }
            if !name_done && s == ast.name(d.name) {
                out.push((t.span, TokenType::Enum));
                name_done = true;
            } else if d.members.iter().any(
                |m| matches!(m, rut_ast::MemberRef::Member(h) if ast.name(h.get(ast).name) == s),
            ) {
                out.push((t.span, TokenType::EnumMember));
            }
        }
    }
}

/// use names classify best-effort by the capitalization law until real
/// resolution (M2): the path segments read as their heads (lowercase =
/// a module → namespace, capitalized = a type), the imported names as
/// the leaf members (capitalized = a type, lowercase = a function)
fn classify_use(toks: &[Token], ast: &Ast, d: &UseNode, out: &mut Vec<(Span, TokenType)>) {
    // the ordered queue — path segments first, then the imports; the
    // walk matches `Ident` texts against the head in order (same shape
    // as `classify_path_segs` / the Enum arm)
    let mut queue: Vec<(&str, TokenType)> = Vec::with_capacity(d.path.len() + d.names.len());
    for id in &d.path {
        let s = ast.name(*id);
        queue.push((
            s,
            if is_cap(s) {
                TokenType::Type
            } else {
                TokenType::Namespace
            },
        ));
    }
    for id in &d.names {
        let s = ast.name(*id);
        queue.push((
            s,
            if is_cap(s) {
                TokenType::Type
            } else {
                TokenType::Function
            },
        ));
    }
    let mut i = 0;
    for t in toks_in(toks, d.span) {
        if i >= queue.len() {
            break;
        }
        if let Tok::Ident(s) = &t.tok {
            if s == queue[i].0 {
                if owned_by_tokens(s) {
                    i += 1;
                    continue; // the token layer's class stands
                }
                out.push((t.span, queue[i].1));
                i += 1;
            }
        }
    }
}

fn classify_members(
    toks: &[Token],
    ast: &Ast,
    fields: &[FieldRef],
    methods: &[MethodRef],
    out: &mut Vec<(Span, TokenType)>,
) {
    classify_fields(toks, ast, fields, out);
    classify_methods(toks, ast, methods, out);
}

fn classify_fields(
    toks: &[Token],
    ast: &Ast,
    fields: &[FieldRef],
    out: &mut Vec<(Span, TokenType)>,
) {
    for f in fields {
        let FieldRef::Field(h) = f else {
            continue; // the recovery slot carries no name
        };
        let d: &FieldDeclNode = h.get(ast);
        push_name(
            toks,
            d.span,
            ast.name(d.name),
            TokenType::Property,
            out,
            false,
        );
        classify_ty(toks, ast, d.ty, out);
        if let Some(init) = d.init {
            classify_expr(toks, ast, init, out);
        }
    }
}

fn classify_methods(
    toks: &[Token],
    ast: &Ast,
    methods: &[MethodRef],
    out: &mut Vec<(Span, TokenType)>,
) {
    for m in methods {
        let MethodRef::Method(h) = m else {
            continue;
        };
        let d: &MethodDeclNode = h.get(ast);
        push_name(
            toks,
            d.span,
            ast.name(d.name),
            TokenType::Method,
            out,
            false,
        );
        for (_, b) in &d.bounds {
            classify_ty(toks, ast, *b, out);
        }
        classify_params(toks, ast, &d.params, out);
        if let Some(r) = d.ret {
            classify_ty(toks, ast, r, out);
        }
        if let Some(body) = d.body {
            classify_block(toks, ast, body, out);
        }
    }
}

/// params of a fn/method/lambda — `self` is token-classified as a
/// keyword everywhere it appears
fn classify_params(
    toks: &[Token],
    ast: &Ast,
    params: &[ParamRef],
    out: &mut Vec<(Span, TokenType)>,
) {
    for p in params {
        let ParamRef::Param(h) = p else {
            continue;
        };
        let d = h.get(ast);
        push_name(
            toks,
            d.span,
            ast.name(d.name),
            TokenType::Parameter,
            out,
            false,
        );
        if let Some(t) = d.ty {
            classify_ty(toks, ast, t, out);
        }
    }
}

fn classify_stmt(toks: &[Token], ast: &Ast, s: StmtRef, out: &mut Vec<(Span, TokenType)>) {
    match s {
        StmtRef::Let(h) => {
            let d = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.name),
                TokenType::Variable,
                out,
                false,
            );
            if let Some(t) = d.ty {
                classify_ty(toks, ast, t, out);
            }
            classify_expr(toks, ast, d.init, out);
        }
        StmtRef::ForOf(h) => {
            let d = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.var),
                TokenType::Variable,
                out,
                false,
            );
            classify_expr(toks, ast, d.iter, out);
            classify_block(toks, ast, d.body, out);
        }
        StmtRef::ForC(h) => {
            let d = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.var),
                TokenType::Variable,
                out,
                false,
            );
            classify_expr(toks, ast, d.init, out);
            classify_expr(toks, ast, d.cond, out);
            classify_expr(toks, ast, d.update, out);
            classify_block(toks, ast, d.body, out);
        }
        StmtRef::If(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.cond, out);
            classify_block(toks, ast, d.then, out);
            match d.els {
                Some(ElseBranch::If(h)) => classify_stmt(toks, ast, StmtRef::If(h), out),
                Some(ElseBranch::Block(b)) => classify_block(toks, ast, b, out),
                None => {}
            }
        }
        StmtRef::While(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.cond, out);
            classify_block(toks, ast, d.body, out);
        }
        StmtRef::When(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.scrut, out);
            for a in &d.arms {
                let arm = a.get(ast);
                for p in &arm.pats {
                    classify_pat(toks, ast, *p, out);
                }
                classify_expr(toks, ast, arm.body, out);
            }
        }
        StmtRef::Return(h) => {
            if let Some(v) = h.get(ast).value {
                classify_expr(toks, ast, v, out);
            }
        }
        StmtRef::Expr(e) => classify_expr(toks, ast, e, out),
        StmtRef::Break(_) | StmtRef::Continue(_) | StmtRef::Error(_) => {}
    }
}

fn classify_block(
    toks: &[Token],
    ast: &Ast,
    b: Handle<rut_ast::BlockNode>,
    out: &mut Vec<(Span, TokenType)>,
) {
    let d = b.get(ast);
    for s in &d.stmts {
        classify_stmt(toks, ast, *s, out);
    }
    if let Some(t) = d.tail {
        classify_expr(toks, ast, t, out);
    }
}

fn classify_ty(toks: &[Token], ast: &Ast, t: TyRef, out: &mut Vec<(Span, TokenType)>) {
    match t {
        // path segments appear in source order; generic args are their
        // own type slots and classify themselves
        TyRef::Path(h) => {
            let d = h.get(ast);
            classify_path_segs(toks, ast, d.span, &d.segs, out, TypeRule::Type);
        }
        TyRef::Fn(h) => {
            let d = h.get(ast);
            for p in &d.params {
                classify_ty(toks, ast, *p, out);
            }
            classify_ty(toks, ast, d.ret, out);
        }
        TyRef::Const(h) => classify_expr(toks, ast, h.get(ast).expr, out),
        TyRef::Opt(h) => classify_ty(toks, ast, h.get(ast).inner, out),
        TyRef::Array(h) => classify_ty(toks, ast, h.get(ast).elem, out),
        TyRef::Tuple(h) => {
            for e in &h.get(ast).elems {
                classify_ty(toks, ast, *e, out);
            }
        }
        TyRef::Union(h) => {
            for e in &h.get(ast).elems {
                classify_ty(toks, ast, *e, out);
            }
        }
        TyRef::Error(_) => {}
    }
}

fn classify_pat(toks: &[Token], ast: &Ast, p: PatRef, out: &mut Vec<(Span, TokenType)>) {
    match p {
        // path segments in order — the last names the member, the rest the
        // type it rides on (`Color.Red`, `Option<T>.Some`); constructor
        // args are binding names (variables)
        PatRef::Path(h) => {
            let d = h.get(ast);
            classify_path_segs(toks, ast, d.span, &d.segs, out, TypeRule::EnumLast);
        }
        PatRef::Ctor(h) => {
            let d = h.get(ast);
            classify_path_segs(toks, ast, d.span, &d.segs, out, TypeRule::EnumLast);
            let names: Vec<&str> = d
                .args
                .iter()
                .filter_map(|a| a.map(|id| ast.name(id)))
                .collect();
            let mut i = 0;
            for tk in toks_in(toks, d.span) {
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
        PatRef::Lit(h) => classify_expr(toks, ast, h.get(ast).expr, out),
        PatRef::Wild(_) | PatRef::Else(_) | PatRef::Error(_) => {}
    }
}

fn classify_expr(toks: &[Token], ast: &Ast, e: ExprRef, out: &mut Vec<(Span, TokenType)>) {
    match e {
        // `d.draw(g)`, `Vec.from(..)` — the name trails its receiver,
        // so recover the LAST match
        ExprRef::Method(h) => {
            let d = h.get(ast);
            push_name(toks, d.span, ast.name(d.name), TokenType::Method, out, true);
            classify_expr(toks, ast, d.recv, out);
            for a in &d.args {
                classify_expr(toks, ast, *a, out);
            }
            for g in &d.generics {
                classify_ty(toks, ast, *g, out);
            }
        }
        ExprRef::Field(h) => {
            let d = h.get(ast);
            push_name(
                toks,
                d.span,
                ast.name(d.name),
                TokenType::Property,
                out,
                true,
            );
            classify_expr(toks, ast, d.recv, out);
        }
        // struct literal labels (`Point { x: 1 }`) lead their values
        ExprRef::Struct(h) => {
            let d = h.get(ast);
            for (fid, val) in &d.fields {
                push_name(
                    toks,
                    d.span,
                    ast.name(*fid),
                    TokenType::Property,
                    out,
                    false,
                );
                classify_expr(toks, ast, *val, out);
            }
            classify_ty(toks, ast, d.ty, out);
        }
        // `::` value paths (`Color::Red`, `layout::mk`, `i32::abs`) —
        // the name-resolution law: interior segments are module/type
        // heads, the leaf is the member. Value access after `.` rides
        // Field/Method instead, so a `::` path never colors a property
        ExprRef::Path(h) => {
            let d = h.get(ast);
            classify_path_segs(toks, ast, d.span, &d.segs, out, TypeRule::Value);
            for seg in &d.segs {
                for g in &seg.generics {
                    classify_ty(toks, ast, *g, out);
                }
            }
        }
        // a bare call `name(args)` colors its callee as a function (free
        // fns and fn-typed locals read alike); casts `x as i32` keep
        // their type color via the primitive guard below. The callee
        // walks FIRST — its lone-seg path pushes `variable` at the same
        // start, and the call's `function` push must land last to win
        // the equal-start tie (the flat-arena order this replaces)
        ExprRef::Call(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.callee, out);
            for a in &d.args {
                classify_expr(toks, ast, *a, out);
            }
            if let ExprRef::Path(ph) = d.callee {
                let pd = ph.get(ast);
                if pd.segs.len() == 1 {
                    let name = ast.name(pd.segs[0].name);
                    if !owned_by_tokens(name) && !is_primitive_ty(name) {
                        push_name(toks, d.span, name, TokenType::Function, out, false);
                    }
                }
            }
        }
        ExprRef::Block(h) => classify_block(toks, ast, h, out),
        ExprRef::Lambda(h) => {
            let d = h.get(ast);
            classify_params(toks, ast, &d.params, out);
            if let Some(r) = d.ret {
                classify_ty(toks, ast, r, out);
            }
            classify_expr(toks, ast, d.body, out);
        }
        // f-string holes are real expression slots — they classify
        // against the flattened stream (names recover inside literals)
        ExprRef::FStr(h) => {
            let d = h.get(ast);
            for part in &d.parts {
                if let rut_ast::FPartAst::Hole(e) = part {
                    classify_expr(toks, ast, *e, out);
                }
            }
        }
        ExprRef::Index(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.recv, out);
            classify_expr(toks, ast, d.idx, out);
        }
        ExprRef::Unary(h) => classify_expr(toks, ast, h.get(ast).expr, out),
        ExprRef::Binary(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.lhs, out);
            classify_expr(toks, ast, d.rhs, out);
        }
        ExprRef::Assign(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.target, out);
            classify_expr(toks, ast, d.value, out);
        }
        ExprRef::Tuple(h) => {
            for e in &h.get(ast).elems {
                classify_expr(toks, ast, *e, out);
            }
        }
        ExprRef::ArrayLit(h) => {
            for e in &h.get(ast).elems {
                classify_expr(toks, ast, *e, out);
            }
        }
        ExprRef::ArrayRepeat(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.value, out);
            classify_expr(toks, ast, d.count, out);
        }
        ExprRef::WhenExpr(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.scrut, out);
            for a in &d.arms {
                let arm = a.get(ast);
                for p in &arm.pats {
                    classify_pat(toks, ast, *p, out);
                }
                classify_expr(toks, ast, arm.body, out);
            }
        }
        ExprRef::If(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.cond, out);
            classify_block(toks, ast, d.then, out);
            match d.els {
                Some(ElseBranch::If(ih)) => {
                    // `else if` links hold if STATEMENTS: flattened —
                    // each link's condition and block join the walk
                    let mut link = Some(ih);
                    while let Some(i) = link {
                        let nd = i.get(ast);
                        classify_expr(toks, ast, nd.cond, out);
                        classify_block(toks, ast, nd.then, out);
                        match nd.els {
                            Some(ElseBranch::If(next)) => link = Some(next),
                            Some(ElseBranch::Block(b)) => {
                                classify_block(toks, ast, b, out);
                                break;
                            }
                            None => break,
                        }
                    }
                }
                Some(ElseBranch::Block(b)) => classify_block(toks, ast, b, out),
                None => {}
            }
        }
        ExprRef::Await(h) => classify_expr(toks, ast, h.get(ast).expr, out),
        ExprRef::AsyncBlock(h) => classify_block(toks, ast, h.get(ast).body, out),
        ExprRef::Try(h) => classify_expr(toks, ast, h.get(ast).expr, out),
        ExprRef::Is(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.expr, out);
            classify_ty(toks, ast, d.ty, out);
        }
        ExprRef::Cast(h) => {
            let d = h.get(ast);
            classify_expr(toks, ast, d.expr, out);
            classify_ty(toks, ast, d.ty, out);
        }
        // literals and paths' leaves carry no child slots to walk
        ExprRef::Lit(_) | ExprRef::Error(_) => {}
    }
}

/// How the segments of a path classify.
#[derive(Clone, Copy, PartialEq)]
enum TypeRule {
    /// every segment is a type (type positions)
    Type,
    /// last segment names an enum member (patterns)
    EnumLast,
    /// value position over a `::` chain — the resolution law's
    /// capitalization reading: interior segments are name-resolution
    /// heads (a lowercase head is a module, never a local; a
    /// capitalized one a type), the leaf is the member (capitalized =
    /// enum member/static, lowercase = the head's fn/const; a lone
    /// lowercase seg is a local read). A property never appears here —
    /// `.` spells value access
    Value,
}

/// Match path segments against the `Ident` tokens inside `span`, in order.
/// Segments the token layer already owns by text (keywords, `Self`,
/// primitives) advance without emitting — their token class stands. The
/// `::` separators lex as their own token and are skipped for free: the
/// walk matches `Ident`s only.
fn classify_path_segs(
    toks: &[Token],
    ast: &Ast,
    span: Span,
    segs: &[rut_ast::PathSeg],
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
                    continue; // the token layer's class stands
                }
                let ty = match rule {
                    TypeRule::Type => TokenType::Type,
                    TypeRule::EnumLast => {
                        if i + 1 == n {
                            TokenType::EnumMember
                        } else {
                            TokenType::Type
                        }
                    }
                    TypeRule::Value => value_seg_class(s, i, n),
                };
                out.push((tk.span, ty));
                i += 1;
            }
        }
    }
}

/// One `::` path segment in value position. Interior segments are the
/// resolution heads: capitalized = a type/enum head, lowercase = a
/// module head (the `namespace` class — never a local variable). The
/// leaf is the member reached through the head: capitalized = an enum
/// member/static/const, lowercase = the head's fn (a lone lowercase seg
/// is a local read; call sites confirm with the function color). A
/// multi-seg `::` path never yields `property` or bare-`variable`
/// colors — that would read `.` value access into name resolution.
fn value_seg_class(s: &str, i: usize, n: usize) -> TokenType {
    // primitives stay types at every position — the cast family
    // `x as i32` reads as a type operation, the associated-fn head
    // `i32::abs` as a type's member lane
    if is_primitive_ty(s) {
        return TokenType::Type;
    }
    if i + 1 == n {
        if is_cap(s) {
            TokenType::EnumMember
        } else if n == 1 {
            TokenType::Variable
        } else {
            TokenType::Function
        }
    } else if is_cap(s) {
        TokenType::Type
    } else {
        TokenType::Namespace
    }
}
