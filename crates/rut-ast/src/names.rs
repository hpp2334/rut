//! The tree's name inventory — every recorded name position, one flat
//! walk. Every name-bearing node records its name token's own span, so
//! the walk emits `(span, ident)` directly: the consumers above the
//! grammar (a references scan, a rename lane) enumerate positions from
//! the tree, never from the token stream.

use rut_core::IdentId;
use rut_lexer::span::Span;

use crate::{ElseBranch, ExprRef, ItemRef, MethodRef, ParamRef, PatRef, StmtRef, TyRef};

/// every name position in the tree, in walk order (items in source
/// order, each decl's own name first). The spans are the recorded
/// name spans — byte-exact declaring/using idents.
pub fn name_positions(ast: &crate::Ast) -> Vec<(Span, IdentId)> {
    let mut out = Vec::new();
    for item in ast.root().items(ast) {
        item_names(ast, *item, &mut out);
    }
    out
}

fn put(span: Span, id: IdentId, out: &mut Vec<(Span, IdentId)>) {
    // the synthesized forms carry a degenerate extent (the newtype's
    // `inner` — no token exists); an empty span is not a position
    if span.hi > span.lo {
        out.push((span, id));
    }
}

fn item_names(ast: &crate::Ast, item: ItemRef, out: &mut Vec<(Span, IdentId)>) {
    macro_rules! decl {
        ($t:ty, $h:expr) => {{
            let d = $h.get(ast);
            put(d.name.span, d.name.ident, out);
        }};
    }
    match item {
        ItemRef::Fn(h) => {
            decl!(crate::FnNode, h);
            let d = h.get(ast);
            param_names(ast, &d.params, out);
            if let Some(r) = d.ret {
                ty_names(ast, r, out);
            }
            block_names(ast, d.body, out);
        }
        ItemRef::Alias(h) => {
            decl!(crate::AliasNode, h);
            ty_names(ast, h.get(ast).target, out);
        }
        ItemRef::SurfaceFn(h) => {
            decl!(crate::SurfaceFnNode, h);
            let d = h.get(ast);
            param_names(ast, &d.params, out);
            if let Some(r) = d.ret {
                ty_names(ast, r, out);
            }
        }
        ItemRef::ModuleLet(h) => {
            decl!(crate::ModuleLetNode, h);
            let d = h.get(ast);
            if let Some(t) = d.ty {
                ty_names(ast, t, out);
            }
            expr_names(ast, d.init, out);
        }
        ItemRef::Enum(h) => {
            decl!(crate::EnumNode, h);
            for m in &h.get(ast).members {
                if let crate::MemberRef::Member(mh) = m {
                    decl!(crate::EnumMemberNode, *mh);
                }
            }
        }
        ItemRef::Struct(h) => {
            decl!(crate::StructNode, h);
            member_names(ast, &h.get(ast).fields, &h.get(ast).methods, out);
        }
        ItemRef::Class(h) => {
            decl!(crate::ClassNode, h);
            let d = h.get(ast);
            for (_, b) in &d.requires {
                ty_names(ast, *b, out);
            }
            member_names(ast, &d.fields, &d.methods, out);
        }
        ItemRef::Interface(h) => {
            decl!(crate::InterfaceNode, h);
            method_names(ast, &h.get(ast).methods, out);
        }
        ItemRef::Impl(h) => {
            let d = h.get(ast);
            ty_names(ast, d.target, out);
            for (_, b) in &d.bounds {
                ty_names(ast, *b, out);
            }
            method_names(ast, &d.methods, out);
        }
        ItemRef::SurfaceStruct(h) => {
            decl!(crate::SurfaceStructNode, h);
            field_names(ast, &h.get(ast).fields, out);
        }
        ItemRef::BuiltinTy(h) => {
            decl!(crate::BuiltinTyNode, h);
            method_names(ast, &h.get(ast).members, out);
        }
        ItemRef::BuiltinPrimitive(h) => {
            decl!(crate::BuiltinPrimitiveNode, h);
            method_names(ast, &h.get(ast).members, out);
        }
        ItemRef::ModDecl(h) => decl!(crate::ModDeclNode, h),
        ItemRef::Use(h) => {
            let d = h.get(ast);
            for n in &d.path {
                put(n.span, n.ident, out);
            }
            for n in &d.names {
                put(n.span, n.ident, out);
            }
        }
        // the root module is walked from [`name_positions`]; an Error
        // slot carries no names
        ItemRef::Module(_) | ItemRef::Error(_) => {}
    }
}

fn member_names(
    ast: &crate::Ast,
    fields: &[crate::FieldRef],
    methods: &[MethodRef],
    out: &mut Vec<(Span, IdentId)>,
) {
    field_names(ast, fields, out);
    method_names(ast, methods, out);
}

fn field_names(ast: &crate::Ast, fields: &[crate::FieldRef], out: &mut Vec<(Span, IdentId)>) {
    for f in fields {
        let crate::FieldRef::Field(h) = f else {
            continue; // the recovery slot carries no name
        };
        let d = h.get(ast);
        put(d.name.span, d.name.ident, out);
        ty_names(ast, d.ty, out);
        if let Some(init) = d.init {
            expr_names(ast, init, out);
        }
    }
}

fn method_names(ast: &crate::Ast, methods: &[MethodRef], out: &mut Vec<(Span, IdentId)>) {
    for m in methods {
        let MethodRef::Method(h) = m else {
            continue;
        };
        let d = h.get(ast);
        put(d.name.span, d.name.ident, out);
        for (_, b) in &d.bounds {
            ty_names(ast, *b, out);
        }
        param_names(ast, &d.params, out);
        if let Some(r) = d.ret {
            ty_names(ast, r, out);
        }
        if let Some(body) = d.body {
            block_names(ast, body, out);
        }
    }
}

fn param_names(ast: &crate::Ast, params: &[ParamRef], out: &mut Vec<(Span, IdentId)>) {
    for p in params {
        let ParamRef::Param(h) = p else {
            continue; // `self` — a keyword, never a name position
        };
        let d = h.get(ast);
        put(d.name.span, d.name.ident, out);
        if let Some(t) = d.ty {
            ty_names(ast, t, out);
        }
    }
}

fn block_names(
    ast: &crate::Ast,
    b: crate::Handle<crate::BlockNode>,
    out: &mut Vec<(Span, IdentId)>,
) {
    let d = b.get(ast);
    for s in &d.stmts {
        stmt_names(ast, *s, out);
    }
    if let Some(t) = d.tail {
        expr_names(ast, t, out);
    }
}

fn stmt_names(ast: &crate::Ast, s: StmtRef, out: &mut Vec<(Span, IdentId)>) {
    match s {
        StmtRef::Let(h) => {
            let d = h.get(ast);
            put(d.name.span, d.name.ident, out);
            if let Some(ns) = &d.destructure {
                for n in ns {
                    put(n.span, n.ident, out);
                }
            }
            if let Some(t) = d.ty {
                ty_names(ast, t, out);
            }
            expr_names(ast, d.init, out);
        }
        StmtRef::ForOf(h) => {
            let d = h.get(ast);
            put(d.var.span, d.var.ident, out);
            expr_names(ast, d.iter, out);
            block_names(ast, d.body, out);
        }
        StmtRef::ForC(h) => {
            let d = h.get(ast);
            put(d.var.span, d.var.ident, out);
            expr_names(ast, d.init, out);
            expr_names(ast, d.cond, out);
            expr_names(ast, d.update, out);
            block_names(ast, d.body, out);
        }
        StmtRef::If(h) => {
            let d = h.get(ast);
            expr_names(ast, d.cond, out);
            block_names(ast, d.then, out);
            match d.els {
                Some(ElseBranch::If(ih)) => stmt_names(ast, StmtRef::If(ih), out),
                Some(ElseBranch::Block(b)) => block_names(ast, b, out),
                None => {}
            }
        }
        StmtRef::While(h) => {
            let d = h.get(ast);
            expr_names(ast, d.cond, out);
            block_names(ast, d.body, out);
        }
        StmtRef::When(h) => {
            let d = h.get(ast);
            expr_names(ast, d.scrut, out);
            for a in &d.arms {
                let arm = a.get(ast);
                for p in &arm.pats {
                    pat_names(ast, *p, out);
                }
                expr_names(ast, arm.body, out);
            }
        }
        StmtRef::Return(h) => {
            if let Some(v) = h.get(ast).value {
                expr_names(ast, v, out);
            }
        }
        StmtRef::Expr(e) => expr_names(ast, e, out),
        StmtRef::Break(_) | StmtRef::Continue(_) | StmtRef::Error(_) => {}
    }
}

fn pat_names(ast: &crate::Ast, p: PatRef, out: &mut Vec<(Span, IdentId)>) {
    match p {
        // path segments in order — the last names the member, the rest
        // the type it rides on
        PatRef::Path(h) => {
            for seg in &h.get(ast).segs {
                put(seg.name.span, seg.name.ident, out);
                for g in &seg.generics {
                    ty_names(ast, *g, out);
                }
            }
        }
        PatRef::Lit(h) => expr_names(ast, h.get(ast).expr, out),
        PatRef::Wild(_) | PatRef::Else(_) | PatRef::Error(_) => {}
    }
}

fn ty_names(ast: &crate::Ast, t: TyRef, out: &mut Vec<(Span, IdentId)>) {
    match t {
        TyRef::Path(h) => {
            for seg in &h.get(ast).segs {
                put(seg.name.span, seg.name.ident, out);
                for g in &seg.generics {
                    ty_names(ast, *g, out);
                }
            }
        }
        TyRef::Self_(_) => {}
        TyRef::Fn(h) => {
            let d = h.get(ast);
            for p in &d.params {
                ty_names(ast, *p, out);
            }
            ty_names(ast, d.ret, out);
        }
        TyRef::Opt(h) => ty_names(ast, h.get(ast).inner, out),
        TyRef::Array(h) => ty_names(ast, h.get(ast).elem, out),
        TyRef::Tuple(h) => {
            for e in &h.get(ast).elems {
                ty_names(ast, *e, out);
            }
        }
        TyRef::Union(h) => {
            for e in &h.get(ast).elems {
                ty_names(ast, *e, out);
            }
        }
        TyRef::Error(_) => {}
    }
}

fn expr_names(ast: &crate::Ast, e: ExprRef, out: &mut Vec<(Span, IdentId)>) {
    match e {
        // the member trails its receiver
        ExprRef::Method(h) => {
            let d = h.get(ast);
            if let Some(m) = d.member {
                put(m.span, m.ident, out);
            }
            for g in &d.generics {
                ty_names(ast, *g, out);
            }
            expr_names(ast, d.recv, out);
            for a in &d.args {
                expr_names(ast, *a, out);
            }
        }
        ExprRef::Field(h) => {
            let d = h.get(ast);
            put(d.name.span, d.name.ident, out);
            expr_names(ast, d.recv, out);
        }
        // struct literal labels lead their values
        ExprRef::Struct(h) => {
            let d = h.get(ast);
            for fi in &d.fields {
                put(fi.name.span, fi.name.ident, out);
                expr_names(ast, fi.value, out);
            }
            ty_names(ast, d.ty, out);
        }
        // `::` value paths and plain reads — every segment is a name
        ExprRef::Path(h) => {
            for seg in &h.get(ast).segs {
                put(seg.name.span, seg.name.ident, out);
                for g in &seg.generics {
                    ty_names(ast, *g, out);
                }
            }
        }
        ExprRef::Call(h) => {
            let d = h.get(ast);
            expr_names(ast, d.callee, out);
            for a in &d.args {
                expr_names(ast, *a, out);
            }
        }
        ExprRef::Block(h) => block_names(ast, h, out),
        ExprRef::Lambda(h) => {
            let d = h.get(ast);
            param_names(ast, &d.params, out);
            if let Some(r) = d.ret {
                ty_names(ast, r, out);
            }
            expr_names(ast, d.body, out);
        }
        // f-string holes are real expression slots — they enumerate in
        // place (the hole's own recorded spans tile inside the literal)
        ExprRef::FStr(h) => {
            for part in &h.get(ast).parts {
                if let crate::FPartAst::Hole(e) = part {
                    expr_names(ast, *e, out);
                }
            }
        }
        ExprRef::Index(h) => {
            let d = h.get(ast);
            expr_names(ast, d.recv, out);
            expr_names(ast, d.idx, out);
        }
        ExprRef::Unary(h) => expr_names(ast, h.get(ast).expr, out),
        ExprRef::Binary(h) => {
            let d = h.get(ast);
            expr_names(ast, d.lhs, out);
            expr_names(ast, d.rhs, out);
        }
        ExprRef::Assign(h) => {
            let d = h.get(ast);
            expr_names(ast, d.target, out);
            expr_names(ast, d.value, out);
        }
        ExprRef::Tuple(h) => {
            for e in &h.get(ast).elems {
                expr_names(ast, *e, out);
            }
        }
        ExprRef::ArrayLit(h) => {
            for e in &h.get(ast).elems {
                expr_names(ast, *e, out);
            }
        }
        ExprRef::ArrayRepeat(h) => {
            let d = h.get(ast);
            expr_names(ast, d.value, out);
            expr_names(ast, d.count, out);
        }
        ExprRef::WhenExpr(h) => {
            let d = h.get(ast);
            expr_names(ast, d.scrut, out);
            for a in &d.arms {
                let arm = a.get(ast);
                for p in &arm.pats {
                    pat_names(ast, *p, out);
                }
                expr_names(ast, arm.body, out);
            }
        }
        ExprRef::If(h) => {
            let d = h.get(ast);
            expr_names(ast, d.cond, out);
            block_names(ast, d.then, out);
            match d.els {
                Some(ElseBranch::If(ih)) => {
                    // `else if` links hold if STATEMENTS: flattened
                    let mut link = Some(ih);
                    while let Some(i) = link {
                        let nd = i.get(ast);
                        expr_names(ast, nd.cond, out);
                        block_names(ast, nd.then, out);
                        match nd.els {
                            Some(ElseBranch::If(next)) => link = Some(next),
                            Some(ElseBranch::Block(b)) => {
                                block_names(ast, b, out);
                                break;
                            }
                            None => break,
                        }
                    }
                }
                Some(ElseBranch::Block(b)) => block_names(ast, b, out),
                None => {}
            }
        }
        ExprRef::Await(h) => expr_names(ast, h.get(ast).expr, out),
        ExprRef::Assert(h) => expr_names(ast, h.get(ast).expr, out),
        ExprRef::AsyncBlock(h) => block_names(ast, h.get(ast).body, out),
        ExprRef::Is(h) => {
            let d = h.get(ast);
            expr_names(ast, d.expr, out);
            ty_names(ast, d.ty, out);
        }
        ExprRef::Cast(h) => {
            let d = h.get(ast);
            expr_names(ast, d.expr, out);
            ty_names(ast, d.ty, out);
        }
        // the receiver spellings and the literals carry no name slots
        ExprRef::Self_(_) | ExprRef::Lit(_) | ExprRef::Error(_) => {}
    }
}
