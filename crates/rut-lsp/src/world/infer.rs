//! Display-side type inference — the resolver family's inference half:
//! surface only what the AST + these heuristics derive, never wire
//! rut-lir in. `init_ty`'s three shapes (`Type.new(..)`, struct
//! literals, string literals) extend to what the survey §4.1 names:
//! field reads, unique fn-call results, chained method calls via ret
//! types, indexing, casts, and the literal defaults (unsuffixed ints
//! are `i32`, unsuffixed floats take the `f32` default). Names resolve
//! through `Doc::bindings()` (read internally — callers never run the
//! pass); an un-annotated binding's type consults its recorded init.
//! A miss is `None` — never wrong text.

use crate::semantic::IntSuffix;
use rut_ast::{Ast, BinOp, Binding, ExprRef, Lit, PathSeg, SelfNode, TyRef};

use super::collection::MountedFile;
use super::types::{head_of_ty, ty_src, TyForm};

/// the type an expression suggests, as display text. `home` is the
/// document whose bindings resolve the walk's names (the doc the expr
/// lives in); `idxs` the file window the member/module-let lanes read.
pub(crate) fn expr_ty(home: &MountedFile, idxs: &[&MountedFile], e: ExprRef) -> Option<String> {
    let ast = home.doc.ast();
    match e {
        // the literal defaults are the engine's own law
        ExprRef::Lit(h) => match &h.get(ast).lit {
            Lit::Str(_) | Lit::RawStr(_) => Some("str".to_string()),
            Lit::Bool(_) => Some("bool".to_string()),
            Lit::Int(_, sfx) => Some(sfx.map(int_suffix).unwrap_or("i32").to_string()),
            Lit::Float(_, sfx) => Some(sfx.map(float_suffix).unwrap_or("f32").to_string()),
            Lit::Nil => None,
        },
        ExprRef::FStr(_) => Some("str".to_string()),
        // `self` — the receiver: the enclosing type (the walk's own
        // law), HOME-scoped — the only file whose spans index the
        // token's offsets; the `Self` type spelling has no value type
        ExprRef::Self_(h) => {
            let d: &SelfNode = h.get(ast);
            if d.is_ty {
                return None;
            }
            super::collection::enclosing_type(std::iter::once(home), d.span.lo)
                .map(|t| t.name().to_string())
        }
        ExprRef::Path(h) => {
            let segs = &h.get(ast).segs;
            path_ty(home, idxs, h.span(ast).lo, segs)
        }
        ExprRef::Call(h) => {
            let d = h.get(ast);
            call_ty(home, idxs, ast, d.callee)
        }
        ExprRef::Method(h) => {
            let d = h.get(ast);
            let name = d.member?.ident;
            if ast.name(name) == "new" {
                // `Type.new(..)` — the original init_ty shape
                if let ExprRef::Path(ph) = d.recv {
                    return ph
                        .get(ast)
                        .segs
                        .first()
                        .map(|s| ast.name(s.name.ident).to_string());
                }
                return None;
            }
            let ty = expr_ty(home, idxs, d.recv)?;
            member_ret(idxs, &head_of_ty(&ty), ast.name(name))
        }
        ExprRef::Field(h) => {
            let d = h.get(ast);
            let ty = expr_ty(home, idxs, d.recv)?;
            field_ty(idxs, &head_of_ty(&ty), ast.name(d.name.ident))
        }
        ExprRef::Index(h) => {
            let d = h.get(ast);
            expr_ty(home, idxs, d.recv).and_then(|t| elem_ty(&t))
        }
        ExprRef::Struct(h) => Some(ty_src(ast, h.get(ast).ty)),
        ExprRef::Tuple(h) => {
            let elems = &h.get(ast).elems;
            let parts: Option<Vec<String>> =
                elems.iter().map(|&e| expr_ty(home, idxs, e)).collect();
            parts.map(|p| format!("({})", p.join(", ")))
        }
        ExprRef::ArrayLit(h) => h
            .get(ast)
            .elems
            .first()
            .and_then(|&e| expr_ty(home, idxs, e))
            .map(|t| format!("[{t}]")),
        ExprRef::ArrayRepeat(h) => {
            let d = h.get(ast);
            expr_ty(home, idxs, d.value).map(|t| format!("[{t}]"))
        }
        ExprRef::Cast(h) => Some(ty_src(ast, h.get(ast).ty)),
        ExprRef::Is(_) => Some("bool".to_string()),
        ExprRef::Binary(h) => {
            let d = h.get(ast);
            match d.op {
                BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => Some("bool".to_string()),
                // same-type arithmetic; the left operand decides
                _ => expr_ty(home, idxs, d.lhs).or_else(|| expr_ty(home, idxs, d.rhs)),
            }
        }
        _ => None,
    }
}

/// a binding row's type, declared-or-inferred: the WRITTEN annotation
/// verbatim (`Doc::text` join), else the recorded init through the
/// resolver over `idxs` (the window the caller rides — the old pass
/// inferred against the whole chain). `None` when nothing derives —
/// never wrong text.
pub(crate) fn binding_ty(home: &MountedFile, idxs: &[&MountedFile], b: &Binding) -> Option<String> {
    if let Some(sp) = b.ty_span {
        return Some(home.doc.text(sp).to_string());
    }
    b.init.and_then(|init| expr_ty(home, idxs, init))
}

/// a `::` path names a module-qualified member: an enum-member value
/// (`Color::Red` — the head IS the value's type), a namespace/type
/// head's fn or const (a call site resolves through `call_ty`), or a
/// plain local read (the lone-seg tail). Value access after `.` rides
/// `ExprRef::Field` — those resolve as field-of-field reads through
/// the bindings, never here
fn path_ty(
    home: &MountedFile,
    idxs: &[&MountedFile],
    pos: u32,
    segs: &[PathSeg],
) -> Option<String> {
    let ast = home.doc.ast();
    let head = segs.first()?.name.ident;
    let text = ast.name(head);
    if segs.len() >= 2 {
        // a RESOLVED head leads the walk — a binding head before `::`
        // is not a name-resolution shape (locals are not namespaces),
        // so this answers only when the head really binds a type text
        let head_ty = rut_ast::resolve_binding(home.doc.bindings(), pos, head)
            .and_then(|b| binding_ty(home, idxs, b))
            .or_else(|| {
                // a capitalized head is the type itself (`Color::Red`)
                is_cap(text).then(|| text.to_string())
            });
        if let Some(start) = head_ty {
            let mut cur = start;
            for seg in &segs[1..] {
                cur = field_ty(idxs, &head_of_ty(&cur), ast.name(seg.name.ident))?;
            }
            return Some(cur);
        }
        return None;
    }
    let own = rut_ast::resolve_binding(home.doc.bindings(), pos, head)
        .and_then(|b| binding_ty(home, idxs, b));
    own.or_else(|| module_let_ty(idxs.iter().copied(), text))
}

/// `f(..)` — a free fn's declared ret, unique match only (ambiguity is
/// a miss, never a wrong type); the constructor-call form — a single
/// CAPITALIZED path segment the index knows as a class/struct — is the
/// type itself (`let log = Logger("hello");` types `log` as `Logger`,
/// the positional shape of the `Type::new(..)` arm below)
fn call_ty(
    home: &MountedFile,
    idxs: &[&MountedFile],
    ast: &Ast,
    callee: ExprRef,
) -> Option<String> {
    if let ExprRef::Path(h) = callee {
        let segs = &h.get(ast).segs;
        if segs.len() == 1 {
            let name = ast.name(segs[0].name.ident);
            let hits: Vec<_> = idxs
                .iter()
                .flat_map(|f| f.fns())
                .filter(|f| f.name() == name && f.owner().is_none())
                .collect();
            if let [one] = hits.as_slice() {
                return one.ret();
            }
            // `Logger(..)` — the constructor call. Only an INDEXED
            // class/struct answers: a capitalized name with no type
            // head behind it (an enum, a plain unknown) stays a miss —
            // never wrong text. The free-fn match keeps precedence.
            // Generic args render as the OWNING doc's own source text
            // (the verbatim law): `HashMap<i32, i32>()` types the
            // binding `HashMap<i32, i32>`
            if is_cap(name) {
                if let Some(ty) = super::collection::find_ty(idxs, name) {
                    if matches!(
                        ty.form(),
                        TyForm::Class | TyForm::Struct | TyForm::HostStruct
                    ) {
                        return Some(if segs[0].generics.is_empty() {
                            name.to_string()
                        } else {
                            generic_name(home, ast, name, &segs[0].generics)
                        });
                    }
                }
            }
            return None;
        }
        // `Type::new(..)` — the two-seg shape IS the head's type; the
        // args ride whichever seg the tree recorded them on (the head,
        // in the `HashMap<i32, str>::new(..)` spelling the parser's
        // PathGen staging builds)
        if segs.len() == 2 && ast.name(segs[1].name.ident) == "new" {
            let name = ast.name(segs[0].name.ident);
            return Some(
                segs.iter()
                    .find(|s| !s.generics.is_empty())
                    .map(|s| generic_name(home, ast, name, &s.generics))
                    .unwrap_or_else(|| name.to_string()),
            );
        }
        return None;
    }
    None
}

/// the constructor-call type text for `name<args>` — each argument is
/// the OWNING doc's verbatim slice (`Doc::text` over the arg's span,
/// trimmed), joined with `, `; nested args (`Vec<str>`) ride their own
/// spans by construction. No type stringifier — a span×src join
fn generic_name(home: &MountedFile, ast: &Ast, name: &str, args: &[TyRef]) -> String {
    let rendered = args
        .iter()
        .map(|&a| home.doc.text(a.span(ast)).trim().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}<{rendered}>")
}

/// a member's declared type on `ty_name`: own-surface methods and
/// fields, then inherent impl-block methods — per file, the same
/// order the old window walk kept. Interface members stay out — the
/// use-gate is the member door's call to make with the full answer;
/// inference stays conservative
fn member_ret(idxs: &[&MountedFile], ty_name: &str, member: &str) -> Option<String> {
    for i in idxs.iter() {
        if let Some(t) = i.ty(ty_name) {
            if let Some(m) = t.methods().into_iter().find(|m| m.name() == member) {
                return m.ty();
            }
        }
        for f in i.fns() {
            if f.name() == member && f.owner().as_deref() == Some(ty_name) {
                return f.ret();
            }
        }
    }
    None
}

/// a field's written type — the first file that declares it
fn field_ty(idxs: &[&MountedFile], ty_name: &str, field: &str) -> Option<String> {
    idxs.iter().copied().find_map(|i| {
        i.ty(ty_name)?
            .fields()
            .into_iter()
            .find(|f| f.name() == field)
            .and_then(|f| f.ty())
    })
}

/// a module let's type head — the decl layer's contribution to
/// receiver inference (`let TOTAL = ..;` at module scope, read below)
pub(crate) fn module_let_ty<'a, I>(idxs: I, name: &str) -> Option<String>
where
    I: IntoIterator<Item = &'a MountedFile>,
{
    let hits: Vec<Option<String>> = idxs
        .into_iter()
        .flat_map(|f| f.lets())
        .filter(|l| l.name() == name)
        .map(|l| l.ty())
        .collect();
    match hits.as_slice() {
        [one] => one.as_deref().map(head_of_ty),
        _ => None,
    }
}

/// the element type of a sequence type text — the survey's for-of rule:
/// `[T]` → `T`, `Vec<T>` → `T` (first generic argument). `None` when the
/// text isn't a sequence spelling (a bare head iterates nothing we can
/// name — a miss, never wrong members)
pub(crate) fn elem_ty(text: &str) -> Option<String> {
    let t = text.trim_start_matches('?');
    if t.starts_with('[') && t.ends_with(']') {
        return Some(t[1..t.len() - 1].trim().to_string());
    }
    let open = t.find('<')?;
    if !t.ends_with('>') {
        return None;
    }
    let inner = &t[open + 1..t.len() - 1];
    let mut depth = 0i32;
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return Some(inner[..i].trim().to_string()),
            _ => {}
        }
    }
    Some(inner.trim().to_string())
}

pub(crate) fn is_cap(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_uppercase())
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

fn float_suffix(sfx: crate::semantic::FloatSuffix) -> &'static str {
    match sfx {
        crate::semantic::FloatSuffix::F32 => "f32",
        crate::semantic::FloatSuffix::F64 => "f64",
    }
}
