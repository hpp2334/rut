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
//!
//! The Future family types here too, text-level rules mirroring the
//! engine's own laws (real typing stays rut-lir's lane): a call to an
//! `async` fn answers `Future<R>` (`R` = written ret, `nil` when
//! unwritten — the sugar law), an `async { }` block mints
//! `Future<T>` (T from the body's valued returns, the weave's founds
//! law), and `await e` unwraps a head that is EXACTLY `Future` (the
//! `compile_await` law — no `?` peel, an optional future is not
//! awaitable). Call sites price the ret through a first-order
//! text-level substitution (spelled generics + declared-vs-inferred
//! argument matching); residual type variables are silence, never
//! `: T` text.

use crate::semantic::IntSuffix;
use rut_ast::{
    Ast, BinOp, Binding, BlockNode, ElseBranch, ExprRef, Handle, Lit, PathSeg, SelfNode, StmtRef,
    TyRef,
};

use super::collection::{FnView, MountedFile};
use super::types::{head_of_ty, ty_src, TyForm};

/// the type an expression suggests, as display text. `home` is the
/// document whose bindings resolve the walk's names (the doc the expr
/// lives in); `idxs` the file window the member/module-let lanes read.
/// one qualified-lane hit — the module-namespace lane's answer keyed
/// by ladder: a VALUE ladder query answers the callee fn view (`m::mk`
/// prices the call through its written ret); a TYPE ladder query
/// answers the declaring view's bare name (`m::Counter` names the
/// type the member chain resolves)
pub(crate) enum QualHit<'a> {
    Fn(crate::world::collection::FnView<'a>),
    Ty(String),
}

pub(crate) fn expr_ty<'a>(
    home: &MountedFile,
    idxs: &[&'a MountedFile],
    e: ExprRef,
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<String> {
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
            path_ty(home, idxs, h.span(ast).lo, segs, qual)
        }
        ExprRef::Call(h) => {
            let d = h.get(ast);
            call_ty(home, idxs, ast, d.callee, &d.args, qual)
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
            let ty = expr_ty(home, idxs, d.recv, qual)?;
            member_ret(
                home,
                idxs,
                ast,
                &head_of_ty(&ty),
                ast.name(name),
                &d.generics,
                &d.args,
                qual,
            )
        }
        ExprRef::Field(h) => {
            let d = h.get(ast);
            let ty = expr_ty(home, idxs, d.recv, qual)?;
            let text = field_ty(idxs, &head_of_ty(&ty), ast.name(d.name.ident))?;
            // a QUALIFIED field type (`c: m::Counter`): the type
            // ladder's qualified lane names the declaring view — the
            // bare name the member chain then resolves
            if text.contains("::") {
                let mut segs: Vec<String> =
                    text.split("::").map(|s| s.trim().to_string()).collect();
                if let Some(last) = segs.last_mut() {
                    if let Some(i) = last.find('<') {
                        *last = last[..i].to_string();
                    }
                }
                if let Some(QualHit::Ty(name)) = qual(&segs, true) {
                    return Some(name);
                }
            }
            Some(text)
        }
        ExprRef::Index(h) => {
            let d = h.get(ast);
            expr_ty(home, idxs, d.recv, qual).and_then(|t| elem_ty(&t))
        }
        ExprRef::Struct(h) => Some(ty_src(ast, h.get(ast).ty)),
        ExprRef::Tuple(h) => {
            let elems = &h.get(ast).elems;
            let parts: Option<Vec<String>> = elems
                .iter()
                .map(|&e| expr_ty(home, idxs, e, qual))
                .collect();
            parts.map(|p| format!("({})", p.join(", ")))
        }
        ExprRef::ArrayLit(h) => h
            .get(ast)
            .elems
            .first()
            .and_then(|&e| expr_ty(home, idxs, e, qual))
            .map(|t| format!("[{t}]")),
        ExprRef::ArrayRepeat(h) => {
            let d = h.get(ast);
            expr_ty(home, idxs, d.value, qual).map(|t| format!("[{t}]"))
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
                _ => expr_ty(home, idxs, d.lhs, qual).or_else(|| expr_ty(home, idxs, d.rhs, qual)),
            }
        }
        // `await e` = unwrap: e's type head EXACTLY `Future` → the
        // first generic argument (the engine's `compile_await` law).
        // STRICT: the head is read WITHOUT the `?` peel — the engine
        // rejects awaiting an optional future, so `?Future<..>` stays
        // a miss; a non-Future is a miss too (never wrong text)
        ExprRef::Await(h) => {
            let t = expr_ty(home, idxs, h.get(ast).expr, qual)?;
            if head_strict(&t) != "Future" {
                return None;
            }
            first_generic_arg(&t)
        }
        // `async { .. }` mints `Future<T>` — the engine's
        // `compile_async_block` law. T is the body's answer: the first
        // valued `return` (the weave records each return's found type
        // and unifies them), else `nil`. A TAIL expression has NO
        // engine counterpart — the weave compiles and DISCARDS it (no
        // answer-field write), so a tail-only block answers
        // `Future<nil>` exactly like the engine
        ExprRef::AsyncBlock(h) => {
            let t = first_return(ast, h.get(ast).body).and_then(|e| expr_ty(home, idxs, e, qual));
            Some(format!(
                "Future<{}>",
                t.unwrap_or_else(|| "nil".to_string())
            ))
        }
        _ => None,
    }
}

/// a binding row's type, declared-or-inferred: the WRITTEN annotation
/// verbatim (`Doc::text` join), else the recorded init through the
/// resolver over `idxs` (the window the caller rides — the old pass
/// inferred against the whole chain). `None` when nothing derives —
/// never wrong text.
pub(crate) fn binding_ty<'a>(
    home: &MountedFile,
    idxs: &[&'a MountedFile],
    b: &Binding,
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<String> {
    if let Some(sp) = b.ty_span {
        return Some(home.doc.text(sp).to_string());
    }
    b.init.and_then(|init| expr_ty(home, idxs, init, qual))
}

/// a `::` path names a module-qualified member: an enum-member value
/// (`Color::Red` — the head IS the value's type), a namespace/type
/// head's fn or const (a call site resolves through `call_ty`), or a
/// plain local read (the lone-seg tail). Value access after `.` rides
/// `ExprRef::Field` — those resolve as field-of-field reads through
/// the bindings, never here
fn path_ty<'a>(
    home: &MountedFile,
    idxs: &[&'a MountedFile],
    pos: u32,
    segs: &[PathSeg],
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<String> {
    let ast = home.doc.ast();
    let head = segs.first()?.name.ident;
    let text = ast.name(head);
    if segs.len() >= 2 {
        // a RESOLVED head leads the walk — a binding head before `::`
        // is not a name-resolution shape (locals are not namespaces),
        // so this answers only when the head really binds a type text
        let head_ty = rut_ast::resolve_binding(home.doc.bindings(), pos, head)
            .and_then(|b| binding_ty(home, idxs, b, qual))
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
        .and_then(|b| binding_ty(home, idxs, b, qual));
    own.or_else(|| module_let_ty(idxs.iter().copied(), text))
}

/// `f(..)` — a free fn's declared ret: a bare callee resolves through
/// the hook first (the file scope — what the queried file binds), the
/// window's unique match only otherwise (ambiguity is a miss, never a
/// wrong type); the constructor-call form — a single
/// CAPITALIZED path segment the index knows as a class/struct — is the
/// type itself (`let log = Logger("hello");` types `log` as `Logger`,
/// the positional shape of the `Type::new(..)` arm below)
fn call_ty<'a>(
    home: &MountedFile,
    idxs: &[&'a MountedFile],
    ast: &Ast,
    callee: ExprRef,
    args: &[ExprRef],
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<String> {
    if let ExprRef::Path(h) = callee {
        let segs = &h.get(ast).segs;
        if segs.len() == 1 {
            let name = ast.name(segs[0].name.ident);
            // the ONE home first: what the QUERIED FILE binds by this
            // name (its own decl, a use leaf, a glob, the ambient rows)
            // — the flat unique-match below is the hook-less callers'
            // window only
            if let Some(QualHit::Fn(fv)) = qual(&[name.to_string()], false) {
                return call_ret(home, idxs, ast, &fv, &segs[0].generics, args, qual);
            }
            let hits: Vec<_> = idxs
                .iter()
                .flat_map(|f| f.fns())
                .filter(|f| f.name() == name && f.owner().is_none())
                .collect();
            if let [one] = hits.as_slice() {
                return call_ret(home, idxs, ast, one, &segs[0].generics, args, qual);
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
        // `m::mk(..)` — a module-qualified callee: the namespace lane
        // resolves it (the ONE home — [`Chain::qualified_scope`]), the
        // written ret prices the call (the async/Self handling rides
        // [`call_ret`])
        let seg_names: Vec<String> = segs
            .iter()
            .map(|s| ast.name(s.name.ident).to_string())
            .collect();
        if let Some(QualHit::Fn(fv)) = qual(&seg_names, false) {
            return call_ret(
                home,
                idxs,
                ast,
                &fv,
                &segs.last().expect("path has segments").generics,
                args,
                qual,
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

/// a member call's ret on `ty_name` — the impl-fn lane over the
/// window: [`find_member_fn`]'s answer priced by [`call_ret`] (the
/// substitution, then the async wrap). `None` when no impl fn
/// answers — an own-surface member is FINAL with no recorded fn
/// (never a same-named impl fn from another file leaks through: the
/// member law's gate)
#[allow(clippy::too_many_arguments)] // the arity IS the resolver's shape — plumbing, not an API to nest
fn member_ret<'a>(
    home: &MountedFile,
    idxs: &[&'a MountedFile],
    ast: &Ast,
    ty_name: &str,
    member: &str,
    written: &[TyRef],
    args: &[ExprRef],
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<String> {
    find_member_fn(idxs, ty_name, member)
        .and_then(|f| call_ret(home, idxs, ast, &f, written, args, qual))
}

/// a member's impl fn on `ty_name` — the callee's method half over
/// the inference window (the chain's `impl_method_fn` law, spelled
/// over idxs): per file, a same-named own-surface member answers
/// FINAL with no fn, then the impl fn whose owner is the target
fn find_member_fn<'a>(idxs: &[&'a MountedFile], ty_name: &str, member: &str) -> Option<FnView<'a>> {
    for i in idxs.iter() {
        if let Some(t) = i.ty(ty_name) {
            if t.methods().iter().any(|m| m.name() == member) {
                return None;
            }
        }
        for f in i.fns() {
            if f.name() == member && f.owner().as_deref() == Some(ty_name) {
                return Some(f);
            }
        }
    }
    None
}

/// a callee fn view's ret at a call site — the written ret priced for
/// display. First-order text-level substitution over the call site:
///
/// (a) the spelled call-site generics zip the decl's parameters
///     (`make<i32>()` names `T := i32`);
/// (b) each declared param whose type mentions a variable
///     structurally matches the arg's inferred text — heads equal,
///     generic arity equal, non-var text equal (`Future<T>` vs
///     `Future<str>` → `T := str`; `[Future<T>]` vs `[Future<str>]` →
///     `T := str`; `?X` peels like `[X]`).
///
/// The ret substitutes under the map, then the RESIDUAL rule: a
/// variable still standing is silence — never `: T` text. Last the
/// async wrap: a call to an `async` fn answers `Future<R>` with `R`
/// the written ret, `nil` unwritten (the engine's sugar law); a plain
/// callee answers `R` (unwritten = a miss, nothing derives)
fn call_ret<'a>(
    home: &MountedFile,
    idxs: &[&'a MountedFile],
    ast: &Ast,
    f: &FnView,
    written: &[TyRef],
    args: &[ExprRef],
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<String> {
    let vars = f.generics();
    let mut map: Vec<(String, String)> = Vec::new();
    for (v, w) in vars.iter().zip(written.iter()) {
        // the call site's own spelling — the HOME doc's verbatim slice
        let text = home.doc.text(w.span(ast)).trim().to_string();
        if !text.is_empty() {
            bind_var(&mut map, v.clone(), text);
        }
    }
    // argument-driven matching: the receiver slot rides out (the args
    // never include `self`)
    let params = f.param_tys();
    let aligned = params.iter().skip(usize::from(f.takes_self()));
    for (p, a) in aligned.zip(args.iter()) {
        let Some(declared) = p else {
            continue;
        };
        if !mentions_var(declared, &vars) {
            continue;
        }
        let Some(inferred) = expr_ty(home, idxs, *a, qual) else {
            continue;
        };
        unify(declared, &inferred, &vars, &mut map);
    }
    let Some(ret) = f.ret() else {
        // unwritten = `nil` (the engine's law) — only an async callee
        // answers it
        return f.is_async().then(|| "Future<nil>".to_string());
    };
    let priced = apply_subst(&ret, &map);
    if mentions_var(&priced, &vars) {
        return None;
    }
    Some(if f.is_async() {
        format!("Future<{priced}>")
    } else {
        priced
    })
}

// ---- the text-level substitution ----

/// first binding wins — a variable already bound keeps its first text
fn bind_var(map: &mut Vec<(String, String)>, var: String, text: String) {
    if !map.iter().any(|(v, _)| *v == var) {
        map.push((var, text));
    }
}

/// the maximal identifier runs of a type text — the boundary scan the
/// substitution's application and its residual check share
fn idents(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut run: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '_' {
            if run.is_none() {
                run = Some(i);
            }
        } else if let Some(s) = run.take() {
            out.push(&text[s..i]);
        }
    }
    if let Some(s) = run {
        out.push(&text[s..]);
    }
    out
}

/// does any of `vars` occur as a whole identifier in `text`?
fn mentions_var(text: &str, vars: &[String]) -> bool {
    idents(text).iter().any(|w| vars.iter().any(|v| v == w))
}

/// ident-boundary substitution: each whole-identifier occurrence of a
/// bound variable swaps to its text (`Vec<T>` with `T := i32` →
/// `Vec<i32>`); a variable inside a longer ident never fires
fn apply_subst(text: &str, map: &[(String, String)]) -> String {
    if map.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut run: Option<usize> = None;
    for (i, c) in text.char_indices().chain([(text.len(), '\0')]) {
        if c.is_alphanumeric() || c == '_' {
            if run.is_none() {
                run = Some(i);
            }
            continue;
        }
        if let Some(s) = run.take() {
            let word = &text[s..i];
            out.push_str(
                map.iter()
                    .find(|(v, _)| v == word)
                    .map(|(_, t)| t.as_str())
                    .unwrap_or(word),
            );
        }
        if i < text.len() {
            out.push(c);
        }
    }
    out
}

/// one first-order structural match: heads equal, generic arity
/// equal, non-var text equal. A bare variable binds whatever the arg
/// inferred; later occurrences of a bound variable don't re-check —
/// the honesty rides the residual rule
fn unify(declared: &str, inferred: &str, vars: &[String], map: &mut Vec<(String, String)>) {
    let d = declared.trim_start_matches('?').trim();
    let i = inferred.trim_start_matches('?').trim();
    if vars.iter().any(|v| v == d) {
        bind_var(map, d.to_string(), i.to_string());
        return;
    }
    if d.starts_with('[') && d.ends_with(']') && i.starts_with('[') && i.ends_with(']') {
        unify(&d[1..d.len() - 1], &i[1..i.len() - 1], vars, map);
        return;
    }
    let (dh, ih) = (head_strict(d), head_strict(i));
    if dh.is_empty() || dh != ih {
        return;
    }
    if let (Some(ds), Some(is)) = (generic_args(d), generic_args(i)) {
        if ds.len() == is.len() {
            for (a, b) in ds.iter().zip(is.iter()) {
                unify(a, b, vars, map);
            }
        }
    }
}

/// an async body's first valued `return`, source order — the engine's
/// weave records EVERY return's found type and unifies them into the
/// answer (display answers with the first; disagreeing returns are
/// already the check lane's diagnosis). A lambda's returns are the
/// lambda's own — the scan stops at its body
fn first_return(ast: &Ast, body: Handle<BlockNode>) -> Option<ExprRef> {
    body.get(ast)
        .stmts
        .iter()
        .find_map(|&s| stmt_return(ast, s))
}

fn stmt_return(ast: &Ast, s: StmtRef) -> Option<ExprRef> {
    match s {
        StmtRef::Return(h) => h.get(ast).value,
        StmtRef::If(h) => {
            let d = h.get(ast);
            first_return(ast, d.then).or_else(|| match d.els {
                Some(ElseBranch::Block(b)) => first_return(ast, b),
                Some(ElseBranch::If(i)) => stmt_return(ast, StmtRef::If(i)),
                None => None,
            })
        }
        StmtRef::While(h) => first_return(ast, h.get(ast).body),
        StmtRef::ForOf(h) => first_return(ast, h.get(ast).body),
        StmtRef::ForC(h) => first_return(ast, h.get(ast).body),
        StmtRef::When(h) => h
            .get(ast)
            .arms
            .iter()
            .find_map(|a| expr_return(ast, a.get(ast).body)),
        StmtRef::Expr(e) => expr_return(ast, e),
        _ => None,
    }
}

fn expr_return(ast: &Ast, e: ExprRef) -> Option<ExprRef> {
    match e {
        ExprRef::Block(b) => first_return(ast, b),
        ExprRef::Lambda(_) => None,
        _ => rut_ast::expr_children(ast, e)
            .into_iter()
            .find_map(|c| expr_return(ast, c)),
    }
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
    first_generic_arg(t)
}

/// the type text's head WITHOUT the `?` peel — the strict head the
/// await unwrap keys on (`head_of_ty` strips by design for member
/// lookup; the engine rejects awaiting an optional future)
fn head_strict(text: &str) -> &str {
    match text.find('<') {
        Some(i) if text.ends_with('>') => &text[..i],
        _ => text,
    }
}

/// a type text's `<..>` arguments, split at depth-0 commas —
/// `Future<Vec<str>, i32>` → [`Vec<str>`, `i32`]; `None` when the
/// text spells no generic region
fn generic_args(text: &str) -> Option<Vec<String>> {
    let open = text.find('<')?;
    if !text.ends_with('>') {
        return None;
    }
    let inner = &text[open + 1..text.len() - 1];
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(inner[start..].trim().to_string());
    Some(out)
}

/// the first generic argument of a `<..>` spelling — the depth-aware
/// cut (`Future<Vec<str>>` → `Vec<str>`); `None` when the text spells
/// no generic region
fn first_generic_arg(text: &str) -> Option<String> {
    generic_args(text)?.into_iter().next()
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
