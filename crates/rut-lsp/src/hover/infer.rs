//! Display-side type inference over initializer expressions — the
//! scoping ruling's engine: surface only what the AST + these
//! heuristics derive, never wire rut-lir in. `init_ty`'s three shapes
//! (`Type.new(..)`, struct literals, string literals) extend to what
//! the survey §4.1 names: field reads, unique fn-call results, chained
//! method calls via ret types, indexing, casts, and the literal
//! defaults (RFC 0007 §1: unsuffixed ints are `i32`, unsuffixed floats
//! take the `f32` default). Names resolve through the binding pass.
//! A miss is `None` — never wrong text.

use rut_ast::ast::*;
use rut_lexer::token::IntSuffix;

use super::bindings::{elem_ty, resolve, Binding};
use super::types::{head_of_ty, ty_src, DefIndex, FnDef, LetDef};

/// the type an expression suggests, as display text
pub(crate) fn expr_ty(
    ast: &Ast,
    binds: &[Binding],
    idxs: &[&DefIndex],
    e: NodeHandle<AnyExpr>,
) -> Option<String> {
    let pos = ast.span(e.id()).lo;
    match ast.expr(e) {
        // the literal defaults are the engine's own law (RFC 0007 §1)
        ExprKind::Lit(Lit::Str(_) | Lit::RawStr(_)) => Some("str".to_string()),
        ExprKind::Lit(Lit::Bool(_)) => Some("bool".to_string()),
        ExprKind::Lit(Lit::Int(_, sfx)) => Some(sfx.map(int_suffix).unwrap_or("i32").to_string()),
        ExprKind::Lit(Lit::Float(_, sfx)) => Some(sfx.map(float_suffix).unwrap_or("f32").to_string()),
        ExprKind::FStr { .. } => Some("str".to_string()),
        ExprKind::Path { segs } => path_ty(ast, binds, idxs, pos, segs),
        ExprKind::Call { callee, .. } => call_ty(idxs, ast, *callee),
        ExprKind::Method { recv, name, .. } => {
            if ast.name(*name) == "new" {
                // `Type.new(..)` — the original init_ty shape
                if let ExprKind::Path { segs } = ast.expr(*recv) {
                    return segs.first().map(|s| ast.name(s.name).to_string());
                }
                return None;
            }
            let ty = expr_ty(ast, binds, idxs, *recv)?;
            member_ret(idxs, &head_of_ty(&ty), ast.name(*name))
        }
        ExprKind::Field { recv, name } => {
            let ty = expr_ty(ast, binds, idxs, *recv)?;
            field_ty(idxs, &head_of_ty(&ty), ast.name(*name))
        }
        ExprKind::Index { recv, .. } => expr_ty(ast, binds, idxs, *recv).and_then(|t| elem_ty(&t)),
        ExprKind::Struct { ty, .. } => Some(ty_src(ast, *ty)),
        ExprKind::Tuple { elems } => {
            let parts: Option<Vec<String>> = elems.iter().map(|&e| expr_ty(ast, binds, idxs, e)).collect();
            parts.map(|p| format!("({})", p.join(", ")))
        }
        ExprKind::ArrayLit { elems } => elems
            .first()
            .and_then(|&e| expr_ty(ast, binds, idxs, e))
            .map(|t| format!("[{t}]")),
        ExprKind::ArrayRepeat { value, .. } => {
            expr_ty(ast, binds, idxs, *value).map(|t| format!("[{t}]"))
        }
        ExprKind::Cast { ty, .. } => Some(ty_src(ast, *ty)),
        ExprKind::Is { .. } => Some("bool".to_string()),
        ExprKind::Binary { op, lhs, rhs } => match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::And | BinOp::Or => {
                Some("bool".to_string())
            }
            // same-type arithmetic; the left operand decides
            _ => expr_ty(ast, binds, idxs, *lhs).or_else(|| expr_ty(ast, binds, idxs, *rhs)),
        },
        ExprKind::Try { expr } => expr_ty(ast, binds, idxs, *expr),
        _ => None,
    }
}

/// a path names a local, a module let, an enum-member value
/// (`Color.Red`), or — the shape the survey's receiver inference is
/// FOR — a dotted field read: the parser spells `wrap.c` as a 2-segment
/// path (`ExprKind::Field` is the tuple-`.0` form only), so `a.b.c`
/// resolves as field-of-field reads through the binding pass
fn path_ty(
    ast: &Ast,
    binds: &[Binding],
    idxs: &[&DefIndex],
    pos: u32,
    segs: &[PathSeg],
) -> Option<String> {
    let head = segs.first()?.name;
    let text = ast.name(head);
    // `self` spells the enclosing type — the same answer recv_type gives
    let self_ty = || -> Option<String> {
        if text == "self" {
            return Some(super::lookup::enclosing_type(idxs, pos)?.1.name.clone());
        }
        None
    };
    if segs.len() >= 2 {
        // a binding leads the path — field reads through its type
        let head_ty = resolve(binds, pos, text)
            .and_then(|b| b.ty.clone())
            .or_else(self_ty);
        if let Some(start) = head_ty {
            let mut cur = start;
            for seg in &segs[1..] {
                cur = field_ty(idxs, &head_of_ty(&cur), ast.name(seg.name))?;
            }
            return Some(cur);
        }
        // `Color.Red` — the head is the type; the member IS the enum
        if is_cap(text) {
            return Some(text.to_string());
        }
        return None;
    }
    resolve(binds, pos, text)
        .and_then(|b| b.ty.clone())
        .or_else(self_ty)
        .or_else(|| module_let_ty(idxs, text))
}

/// `f(..)` — a free fn's declared ret, unique match only (ambiguity is
/// a miss, never a wrong type)
fn call_ty(idxs: &[&DefIndex], ast: &Ast, callee: NodeHandle<AnyExpr>) -> Option<String> {
    if let ExprKind::Path { segs } = ast.expr(callee) {
        if segs.len() == 1 {
            let name = ast.name(segs[0].name);
            let hits: Vec<&FnDef> = idxs
                .iter()
                .flat_map(|i| i.fns.iter().filter(|f| f.name == name && f.owner.is_none()))
                .collect();
            if let [one] = hits.as_slice() {
                return one.ret.clone();
            }
        }
    }
    None
}

/// a member's declared type on `ty_name`: own-surface methods and
/// fields, then inherent impl-block methods. Trait methods stay out —
/// the use-gate is member_hover's call to make with the full answer;
/// inference stays conservative
fn member_ret(idxs: &[&DefIndex], ty_name: &str, member: &str) -> Option<String> {
    for i in idxs {
        if let Some(t) = i.ty(ty_name) {
            if let Some(m) = t.methods.iter().find(|m| m.name == member) {
                return m.ty.clone();
            }
        }
        let owner = format!("impl {ty_name}");
        for f in &i.fns {
            if f.name == member && f.owner.as_deref() == Some(owner.as_str()) {
                return f.ret.clone();
            }
        }
    }
    None
}

fn field_ty(idxs: &[&DefIndex], ty_name: &str, field: &str) -> Option<String> {
    idxs.iter().find_map(|i| {
        i.ty(ty_name)
            .and_then(|t| t.fields.iter().find(|f| f.name == field))
            .and_then(|f| f.ty.clone())
    })
}

/// a module let's type head — the decl layer's contribution to
/// receiver inference (`let TOTAL = ..;` at module scope, read below)
pub(crate) fn module_let_ty(idxs: &[&DefIndex], name: &str) -> Option<String> {
    let hits: Vec<&LetDef> = idxs
        .iter()
        .flat_map(|i| i.lets.iter().filter(|l| l.name == name))
        .collect();
    match hits.as_slice() {
        [one] => one.ty.as_deref().map(head_of_ty),
        _ => None,
    }
}

fn int_suffix(sfx: IntSuffix) -> &'static str {
    match sfx {
        IntSuffix::U8 => "u8",
        IntSuffix::U16 => "u16",
        IntSuffix::U32 => "u32",
        IntSuffix::U64 => "u64",
        IntSuffix::I8 => "i8",
        IntSuffix::I16 => "i16",
        IntSuffix::I32 => "i32",
        IntSuffix::I64 => "i64",
    }
}

fn float_suffix(sfx: rut_lexer::token::FloatSuffix) -> &'static str {
    match sfx {
        rut_lexer::token::FloatSuffix::F32 => "f32",
        rut_lexer::token::FloatSuffix::F64 => "f64",
    }
}

pub(crate) fn is_cap(s: &str) -> bool {
    s.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
}
