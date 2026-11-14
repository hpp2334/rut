//! The AST pass — identifier classes from a walk of the slot tree;
//! every name-bearing node records its name token's span, so the walk
//! emits `(recorded span, class)` directly (the only guard left: texts
//! the token layer owns by word shape — keywords, `Self` — do not
//! emit; the token pass's class stands).

use rut_ast::{
    Ast, ElseBranch, EnumNode, ExprRef, FieldDeclNode, FieldRef, FnNode, Handle, ItemRef,
    MethodDeclNode, MethodRef, ModDeclNode, ModInlineNode, ParamRef, PatRef, StmtRef, TyRef,
    UseNode,
};
use rut_lexer::span::Span;

use super::legend::{is_cap, is_primitive_ty, owned_by_tokens, TokenType};

/// One recorded name position — emit unless the text is one the token
/// layer already owns by word shape (keywords, `Self`): the token
/// pass's class stands there. A name position is the name TOKEN's own
/// span; the synthesized forms carry a degenerate extent instead (the
/// newtype's `inner` field — no token exists for it), so an empty span
/// does not emit either.
fn emit(name: &str, span: Span, ty: TokenType, out: &mut Vec<(Span, TokenType)>) {
    if span.hi == span.lo || owned_by_tokens(name) {
        return; // not a token position / the token layer's class stands
    }
    out.push((span, ty));
}

/// The AST walk — a recursion over the slot tree from the root (the
/// parser's nesting budgets keep the depth shallow). Only
/// name-bearing nodes act; Error slots are skipped everywhere.
pub(crate) fn classify_ast(ast: &Ast, out: &mut Vec<(Span, TokenType)>) {
    for item in ast.root().items(ast) {
        classify_item(ast, *item, out);
    }
}

fn classify_item(ast: &Ast, item: ItemRef, out: &mut Vec<(Span, TokenType)>) {
    match item {
        ItemRef::Fn(h) => {
            let d = h.get(ast);
            classify_fn(ast, d, out);
        }
        ItemRef::Alias(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Type, out);
            classify_ty(ast, d.target, out);
        }
        ItemRef::SurfaceFn(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Function, out);
            classify_params(ast, &d.params, out);
            if let Some(r) = d.ret {
                classify_ty(ast, r, out);
            }
        }
        ItemRef::ModuleLet(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Variable, out);
            if let Some(t) = d.ty {
                classify_ty(ast, t, out);
            }
            classify_expr(ast, d.init, out);
        }
        ItemRef::Enum(h) => {
            let d = h.get(ast);
            classify_enum(ast, d, out);
        }
        ItemRef::Struct(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Class, out);
            classify_members(ast, &d.fields, &d.methods, out);
        }
        ItemRef::Class(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Class, out);
            classify_members(ast, &d.fields, &d.methods, out);
        }
        ItemRef::Interface(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Interface, out);
            classify_methods(ast, &d.methods, out);
        }
        ItemRef::Impl(h) => {
            let d = h.get(ast);
            classify_ty(ast, d.target, out);
            for (_, b) in &d.bounds {
                classify_ty(ast, *b, out);
            }
            classify_methods(ast, &d.methods, out);
        }
        ItemRef::SurfaceStruct(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Class, out);
            classify_fields(ast, &d.fields, out);
        }
        ItemRef::BuiltinTy(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Class, out);
            classify_methods(ast, &d.members, out);
        }
        ItemRef::BuiltinPrimitive(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Class, out);
            classify_methods(ast, &d.members, out);
        }
        // `builtin impl i32 { .. }` declares no new name — the primitive
        // is already in scope; only its methods classify
        ItemRef::BuiltinImpl(h) => {
            let d = h.get(ast);
            classify_methods(ast, &d.methods, out);
        }
        // mod decls (file and inline) ARE namespaces — their names
        // classify here (the declared head reads like the `layout` in
        // `layout::mk`, which the path law colors too). The inline
        // form's nested items classify through the same recursion.
        ItemRef::ModDecl(h) => {
            let d: &ModDeclNode = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Namespace, out);
        }
        ItemRef::ModInline(h) => {
            let d: &ModInlineNode = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Namespace, out);
            for nested in h.items(ast) {
                classify_item(ast, *nested, out);
            }
        }
        ItemRef::Use(h) => {
            let d: &UseNode = h.get(ast);
            classify_use(ast, d, out);
        }
        // the root module is walked from `classify_ast`; an Error slot
        // carries no names
        ItemRef::Module(_) | ItemRef::Error(_) => {}
    }
}

fn classify_fn(ast: &Ast, d: &FnNode, out: &mut Vec<(Span, TokenType)>) {
    emit(ast.name(d.name), d.name_span, TokenType::Function, out);
    for (_, b) in &d.bounds {
        classify_ty(ast, *b, out);
    }
    classify_params(ast, &d.params, out);
    if let Some(r) = d.ret {
        classify_ty(ast, r, out);
    }
    classify_block(ast, d.body, out);
}

/// `pub? enum Name { A, B }` — the enum's own name plus its members,
/// each at its recorded span
fn classify_enum(ast: &Ast, d: &EnumNode, out: &mut Vec<(Span, TokenType)>) {
    emit(ast.name(d.name), d.name_span, TokenType::Enum, out);
    for m in &d.members {
        let rut_ast::MemberRef::Member(h) = m else {
            continue; // the recovery slot carries no name
        };
        let md = h.get(ast);
        emit(ast.name(md.name), md.name_span, TokenType::EnumMember, out);
    }
}

/// use names classify best-effort by the capitalization law until real
/// resolution (M2): the path segments read as their heads (lowercase =
/// a module → namespace, capitalized = a type), the imported names as
/// the leaf members (capitalized = a type, lowercase = a function)
fn classify_use(ast: &Ast, d: &UseNode, out: &mut Vec<(Span, TokenType)>) {
    for (id, sp) in d.path.iter().zip(&d.path_spans) {
        let s = ast.name(*id);
        let ty = if is_cap(s) {
            TokenType::Type
        } else {
            TokenType::Namespace
        };
        emit(s, *sp, ty, out);
    }
    for (id, sp) in d.names.iter().zip(&d.name_spans) {
        let s = ast.name(*id);
        let ty = if is_cap(s) {
            TokenType::Type
        } else {
            TokenType::Function
        };
        emit(s, *sp, ty, out);
    }
}

fn classify_members(
    ast: &Ast,
    fields: &[FieldRef],
    methods: &[MethodRef],
    out: &mut Vec<(Span, TokenType)>,
) {
    classify_fields(ast, fields, out);
    classify_methods(ast, methods, out);
}

fn classify_fields(ast: &Ast, fields: &[FieldRef], out: &mut Vec<(Span, TokenType)>) {
    for f in fields {
        let FieldRef::Field(h) = f else {
            continue; // the recovery slot carries no name
        };
        let d: &FieldDeclNode = h.get(ast);
        emit(ast.name(d.name), d.name_span, TokenType::Property, out);
        classify_ty(ast, d.ty, out);
        if let Some(init) = d.init {
            classify_expr(ast, init, out);
        }
    }
}

fn classify_methods(ast: &Ast, methods: &[MethodRef], out: &mut Vec<(Span, TokenType)>) {
    for m in methods {
        let MethodRef::Method(h) = m else {
            continue;
        };
        let d: &MethodDeclNode = h.get(ast);
        emit(ast.name(d.name), d.name_span, TokenType::Method, out);
        for (_, b) in &d.bounds {
            classify_ty(ast, *b, out);
        }
        classify_params(ast, &d.params, out);
        if let Some(r) = d.ret {
            classify_ty(ast, r, out);
        }
        if let Some(body) = d.body {
            classify_block(ast, body, out);
        }
    }
}

/// params of a fn/method/lambda — `self` is token-classified as a
/// keyword everywhere it appears
fn classify_params(ast: &Ast, params: &[ParamRef], out: &mut Vec<(Span, TokenType)>) {
    for p in params {
        let ParamRef::Param(h) = p else {
            continue;
        };
        let d = h.get(ast);
        emit(ast.name(d.name), d.name_span, TokenType::Parameter, out);
        if let Some(t) = d.ty {
            classify_ty(ast, t, out);
        }
    }
}

fn classify_stmt(ast: &Ast, s: StmtRef, out: &mut Vec<(Span, TokenType)>) {
    match s {
        StmtRef::Let(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Variable, out);
            if let Some(t) = d.ty {
                classify_ty(ast, t, out);
            }
            classify_expr(ast, d.init, out);
        }
        StmtRef::ForOf(h) => {
            let d = h.get(ast);
            emit(ast.name(d.var), d.name_span, TokenType::Variable, out);
            classify_expr(ast, d.iter, out);
            classify_block(ast, d.body, out);
        }
        StmtRef::ForC(h) => {
            let d = h.get(ast);
            emit(ast.name(d.var), d.name_span, TokenType::Variable, out);
            classify_expr(ast, d.init, out);
            classify_expr(ast, d.cond, out);
            classify_expr(ast, d.update, out);
            classify_block(ast, d.body, out);
        }
        StmtRef::If(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.cond, out);
            classify_block(ast, d.then, out);
            match d.els {
                Some(ElseBranch::If(h)) => classify_stmt(ast, StmtRef::If(h), out),
                Some(ElseBranch::Block(b)) => classify_block(ast, b, out),
                None => {}
            }
        }
        StmtRef::While(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.cond, out);
            classify_block(ast, d.body, out);
        }
        StmtRef::When(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.scrut, out);
            for a in &d.arms {
                let arm = a.get(ast);
                for p in &arm.pats {
                    classify_pat(ast, *p, out);
                }
                classify_expr(ast, arm.body, out);
            }
        }
        StmtRef::Return(h) => {
            if let Some(v) = h.get(ast).value {
                classify_expr(ast, v, out);
            }
        }
        StmtRef::Expr(e) => classify_expr(ast, e, out),
        StmtRef::Break(_) | StmtRef::Continue(_) | StmtRef::Error(_) => {}
    }
}

fn classify_block(ast: &Ast, b: Handle<rut_ast::BlockNode>, out: &mut Vec<(Span, TokenType)>) {
    let d = b.get(ast);
    for s in &d.stmts {
        classify_stmt(ast, *s, out);
    }
    if let Some(t) = d.tail {
        classify_expr(ast, t, out);
    }
}

fn classify_ty(ast: &Ast, t: TyRef, out: &mut Vec<(Span, TokenType)>) {
    match t {
        // path segments appear in source order; generic args are their
        // own type slots and classify themselves
        TyRef::Path(h) => {
            let d = h.get(ast);
            classify_path_segs(ast, &d.segs, out, TypeRule::Type);
        }
        TyRef::Fn(h) => {
            let d = h.get(ast);
            for p in &d.params {
                classify_ty(ast, *p, out);
            }
            classify_ty(ast, d.ret, out);
        }
        TyRef::Const(h) => classify_expr(ast, h.get(ast).expr, out),
        TyRef::Opt(h) => classify_ty(ast, h.get(ast).inner, out),
        TyRef::Array(h) => classify_ty(ast, h.get(ast).elem, out),
        TyRef::Tuple(h) => {
            for e in &h.get(ast).elems {
                classify_ty(ast, *e, out);
            }
        }
        TyRef::Union(h) => {
            for e in &h.get(ast).elems {
                classify_ty(ast, *e, out);
            }
        }
        TyRef::Error(_) => {}
    }
}

fn classify_pat(ast: &Ast, p: PatRef, out: &mut Vec<(Span, TokenType)>) {
    match p {
        // path segments in order — the last names the member, the rest the
        // type it rides on (`Color.Red`, `Option<T>.Some`); constructor
        // args are binding names (variables)
        PatRef::Path(h) => {
            let d = h.get(ast);
            classify_path_segs(ast, &d.segs, out, TypeRule::EnumLast);
        }
        PatRef::Ctor(h) => {
            let d = h.get(ast);
            classify_path_segs(ast, &d.segs, out, TypeRule::EnumLast);
            for a in &d.args {
                let Some(b) = a else {
                    continue; // `_` — no binding name
                };
                emit(ast.name(b.name), b.name_span, TokenType::Variable, out);
            }
        }
        PatRef::Lit(h) => classify_expr(ast, h.get(ast).expr, out),
        PatRef::Wild(_) | PatRef::Else(_) | PatRef::Error(_) => {}
    }
}

fn classify_expr(ast: &Ast, e: ExprRef, out: &mut Vec<(Span, TokenType)>) {
    match e {
        // `d.draw(g)`, `Vec.from(..)` — the name trails its receiver
        ExprRef::Method(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Method, out);
            classify_expr(ast, d.recv, out);
            for a in &d.args {
                classify_expr(ast, *a, out);
            }
            for g in &d.generics {
                classify_ty(ast, *g, out);
            }
        }
        ExprRef::Field(h) => {
            let d = h.get(ast);
            emit(ast.name(d.name), d.name_span, TokenType::Property, out);
            classify_expr(ast, d.recv, out);
        }
        // struct literal labels (`Point { x: 1 }`) lead their values
        ExprRef::Struct(h) => {
            let d = h.get(ast);
            for fi in &d.fields {
                emit(ast.name(fi.name), fi.name_span, TokenType::Property, out);
                classify_expr(ast, fi.value, out);
            }
            classify_ty(ast, d.ty, out);
        }
        // `::` value paths (`Color::Red`, `layout::mk`, `i32::abs`) —
        // the name-resolution law: interior segments are module/type
        // heads, the leaf is the member. Value access after `.` rides
        // Field/Method instead, so a `::` path never colors a property
        ExprRef::Path(h) => {
            let d = h.get(ast);
            classify_path_segs(ast, &d.segs, out, TypeRule::Value);
            for seg in &d.segs {
                for g in &seg.generics {
                    classify_ty(ast, *g, out);
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
            classify_expr(ast, d.callee, out);
            for a in &d.args {
                classify_expr(ast, *a, out);
            }
            if let ExprRef::Path(ph) = d.callee {
                let pd = ph.get(ast);
                if pd.segs.len() == 1 {
                    let name = ast.name(pd.segs[0].name);
                    if !is_primitive_ty(name) {
                        emit(name, pd.segs[0].name_span, TokenType::Function, out);
                    }
                }
            }
        }
        ExprRef::Block(h) => classify_block(ast, h, out),
        ExprRef::Lambda(h) => {
            let d = h.get(ast);
            classify_params(ast, &d.params, out);
            if let Some(r) = d.ret {
                classify_ty(ast, r, out);
            }
            classify_expr(ast, d.body, out);
        }
        // f-string holes are real expression slots — they classify in
        // place (the hole's own recorded spans tile inside the literal)
        ExprRef::FStr(h) => {
            let d = h.get(ast);
            for part in &d.parts {
                if let rut_ast::FPartAst::Hole(e) = part {
                    classify_expr(ast, *e, out);
                }
            }
        }
        ExprRef::Index(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.recv, out);
            classify_expr(ast, d.idx, out);
        }
        ExprRef::Unary(h) => classify_expr(ast, h.get(ast).expr, out),
        ExprRef::Binary(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.lhs, out);
            classify_expr(ast, d.rhs, out);
        }
        ExprRef::Assign(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.target, out);
            classify_expr(ast, d.value, out);
        }
        ExprRef::Tuple(h) => {
            for e in &h.get(ast).elems {
                classify_expr(ast, *e, out);
            }
        }
        ExprRef::ArrayLit(h) => {
            for e in &h.get(ast).elems {
                classify_expr(ast, *e, out);
            }
        }
        ExprRef::ArrayRepeat(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.value, out);
            classify_expr(ast, d.count, out);
        }
        ExprRef::WhenExpr(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.scrut, out);
            for a in &d.arms {
                let arm = a.get(ast);
                for p in &arm.pats {
                    classify_pat(ast, *p, out);
                }
                classify_expr(ast, arm.body, out);
            }
        }
        ExprRef::If(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.cond, out);
            classify_block(ast, d.then, out);
            match d.els {
                Some(ElseBranch::If(ih)) => {
                    // `else if` links hold if STATEMENTS: flattened —
                    // each link's condition and block join the walk
                    let mut link = Some(ih);
                    while let Some(i) = link {
                        let nd = i.get(ast);
                        classify_expr(ast, nd.cond, out);
                        classify_block(ast, nd.then, out);
                        match nd.els {
                            Some(ElseBranch::If(next)) => link = Some(next),
                            Some(ElseBranch::Block(b)) => {
                                classify_block(ast, b, out);
                                break;
                            }
                            None => break,
                        }
                    }
                }
                Some(ElseBranch::Block(b)) => classify_block(ast, b, out),
                None => {}
            }
        }
        ExprRef::Await(h) => classify_expr(ast, h.get(ast).expr, out),
        ExprRef::AsyncBlock(h) => classify_block(ast, h.get(ast).body, out),
        ExprRef::Try(h) => classify_expr(ast, h.get(ast).expr, out),
        ExprRef::Is(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.expr, out);
            classify_ty(ast, d.ty, out);
        }
        ExprRef::Cast(h) => {
            let d = h.get(ast);
            classify_expr(ast, d.expr, out);
            classify_ty(ast, d.ty, out);
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

/// Emit each segment at its recorded `name_span`, in source order.
/// Segments the token layer already owns by text (keywords, `Self`)
/// do not emit — their token class stands.
fn classify_path_segs(
    ast: &Ast,
    segs: &[rut_ast::PathSeg],
    out: &mut Vec<(Span, TokenType)>,
    rule: TypeRule,
) {
    let n = segs.len();
    for (i, seg) in segs.iter().enumerate() {
        let s = ast.name(seg.name);
        if owned_by_tokens(s) {
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
        out.push((seg.name_span, ty));
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
