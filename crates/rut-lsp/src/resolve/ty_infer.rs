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
//! Every answer is a [`Ty`](super::ty::Ty) — the verbatim spelling
//! beside the shape read off the tree (L4); substitution and the
//! structural match walk the shape, never the text.
//!
//! The Future family types here too, text-level rules mirroring the
//! engine's own laws (real typing stays rut-lir's lane): a call to an
//! `async` fn answers `Future<R>` (`R` = written ret, `nil` when
//! unwritten — the sugar law), an `async { }` block mints
//! `Future<T>` (T from the body's valued returns, the weave's founds
//! law), and `await e` unwraps a head that is EXACTLY `Future` (the
//! `compile_await` law — no `?` peel, an optional future is not
//! awaitable). Call sites price the ret through a first-order
//! substitution (spelled generics + declared-vs-inferred argument
//! matching over the shape); residual type variables are silence,
//! never `: T` text.

use crate::doc::is_cap;
use crate::doc::IntSuffix;
use rut_ast::{
    Ast, BinOp, Binding, BlockNode, ElseBranch, ExprRef, Handle, Lit, PathSeg, SelfNode, StmtRef,
    TyRef,
};

use crate::resolve::prefer_mounted;
use crate::world::ty::{bind_var, Shape, Ty, TyForm};
use crate::world::views::FnView;
use crate::world::File;

/// the type an expression suggests, as display text. `home` is the
/// document whose bindings resolve the walk's names (the doc the expr
/// lives in); `idxs` the file window the member/module-let lanes read.
/// one qualified-lane hit — the module-namespace lane's answer keyed
/// by ladder: a VALUE ladder query answers the callee fn view (`m::mk`
/// prices the call through its written ret); a TYPE ladder query
/// answers the declaring view's bare name (`m::Counter` names the
/// type the member chain resolves)
pub(crate) enum QualHit<'a> {
    Fn(crate::world::views::FnView<'a>),
    Ty(Ty),
}

pub(crate) fn expr_ty<'a>(
    home: &File,
    idxs: &[&'a File],
    e: ExprRef,
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<Ty> {
    let ast = home.doc.ast();
    match e {
        // the literal defaults are the engine's own law
        ExprRef::Lit(h) => match &h.get(ast).lit {
            Lit::Str(_) | Lit::RawStr(_) => Some(Ty::named("str")),
            Lit::Bool(_) => Some(Ty::named("bool")),
            Lit::Int(_, sfx) => Some(Ty::named(sfx.map(int_suffix).unwrap_or("i32"))),
            Lit::Float(_, sfx) => Some(Ty::named(sfx.map(float_suffix).unwrap_or("f32"))),
            Lit::Nil => None,
        },
        ExprRef::FStr(_) => Some(Ty::named("str")),
        // `self` — the receiver: the enclosing type (the walk's own
        // law), HOME-scoped — the only file whose spans index the
        // token's offsets; the `Self` type spelling has no value type
        ExprRef::Self_(h) => {
            let d: &SelfNode = h.get(ast);
            if d.is_ty {
                return None;
            }
            super::enclosing_type(std::iter::once(home), d.span.lo).map(|t| Ty::named(t.name()))
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
                        .map(|s| Ty::named(ast.name(s.name.ident)));
                }
                return None;
            }
            let ty = expr_ty(home, idxs, d.recv, qual)?;
            member_ret(
                home,
                idxs,
                ast,
                &ty.head(),
                ast.name(name),
                &d.generics,
                &d.args,
                qual,
            )
        }
        ExprRef::Field(h) => {
            let d = h.get(ast);
            let ty = expr_ty(home, idxs, d.recv, qual)?;
            let out = field_ty(idxs, &ty.head(), ast.name(d.name.ident))?;
            // a QUALIFIED field type (`c: m::Counter`): the type
            // ladder's qualified lane names the declaring view — the
            // bare name the member chain then resolves
            if let Shape::Named { segs, .. } = &out.shape {
                if segs.len() >= 2 {
                    if let Some(QualHit::Ty(name)) = qual(segs, true) {
                        return Some(name);
                    }
                }
            }
            Some(out)
        }
        ExprRef::Index(h) => {
            let d = h.get(ast);
            expr_ty(home, idxs, d.recv, qual).and_then(|t| t.first_arg().cloned())
        }
        ExprRef::Struct(h) => Some(Ty::of(ast, h.get(ast).ty)),
        ExprRef::Tuple(h) => {
            let elems = &h.get(ast).elems;
            let parts: Option<Vec<Ty>> = elems
                .iter()
                .map(|&e| expr_ty(home, idxs, e, qual))
                .collect();
            parts.map(|p| Ty {
                text: format!(
                    "({})",
                    p.iter()
                        .map(|t| t.text.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                shape: Shape::Tuple(p),
            })
        }
        ExprRef::ArrayLit(h) => h
            .get(ast)
            .elems
            .first()
            .and_then(|&e| expr_ty(home, idxs, e, qual))
            .map(|t| Ty {
                text: format!("[{}]", t.text),
                shape: Shape::Array(Box::new(t)),
            }),
        ExprRef::ArrayRepeat(h) => {
            let d = h.get(ast);
            expr_ty(home, idxs, d.value, qual).map(|t| Ty {
                text: format!("[{}]", t.text),
                shape: Shape::Array(Box::new(t)),
            })
        }
        ExprRef::Cast(h) => Some(Ty::of(ast, h.get(ast).ty)),
        ExprRef::Is(_) => Some(Ty::named("bool")),
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
                | BinOp::Or => Some(Ty::named("bool")),
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
            // an `Opt` spells `?..` — never exactly `Future`
            if t.opt_inner().is_some() {
                return None;
            }
            if t.head() != "Future" {
                return None;
            }
            t.first_arg().cloned()
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
            Some(future_ty(t.unwrap_or_else(|| Ty::named("nil"))))
        }
        _ => None,
    }
}

/// a binding row's type, declared-or-inferred: the WRITTEN annotation
/// (the verbatim span join beside the tree's shape), else the recorded
/// init through the resolver over `idxs` (the window the caller rides
/// — the old pass inferred against the whole chain). `None` when
/// nothing derives — never wrong text.
pub(crate) fn binding_ty<'a>(
    home: &File,
    idxs: &[&'a File],
    b: &Binding,
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<Ty> {
    if let Some(sp) = b.ty_span {
        return Some(Ty::of_annotation(home, sp));
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
    home: &File,
    idxs: &[&'a File],
    pos: u32,
    segs: &[PathSeg],
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<Ty> {
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
                is_cap(text).then(|| Ty::named(text))
            });
        if let Some(start) = head_ty {
            let mut cur = start;
            for seg in &segs[1..] {
                cur = field_ty(idxs, &cur.head(), ast.name(seg.name.ident))?;
            }
            return Some(cur);
        }
        return None;
    }
    let own = rut_ast::resolve_binding(home.doc.bindings(), pos, head)
        .and_then(|b| binding_ty(home, idxs, b, qual));
    own.or_else(|| module_let_ty(idxs.iter().copied(), text).map(|t| t.headed()))
}

/// `f(..)` — a free fn's declared ret: a bare callee resolves through
/// the hook first (the file scope — what the queried file binds), the
/// window's unique match only otherwise (ambiguity is a miss, never a
/// wrong type); the constructor-call form — a single
/// CAPITALIZED path segment the index knows as a class/struct — is the
/// type itself (`let log = Logger("hello");` types `log` as `Logger`,
/// the positional shape of the `Type::new(..)` arm below)
fn call_ty<'a>(
    home: &File,
    idxs: &[&'a File],
    ast: &Ast,
    callee: ExprRef,
    args: &[ExprRef],
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<Ty> {
    if let ExprRef::Path(h) = callee {
        let segs = &h.get(ast).segs;
        if segs.len() == 1 {
            let name = ast.name(segs[0].name.ident);
            // the ONE home first: what the QUERIED FILE binds by this
            // name (its own decl, a use leaf, the ambient rows)
            // — the flat unique-match below is the hook-less callers'
            // window only
            if let Some(QualHit::Fn(fv)) = qual(&[name.to_string()], false) {
                return call_ret(home, idxs, ast, &fv, &segs[0].generics, args, qual);
            }
            // the unique free fn by name over the inference window (the
            // one law beside the member law, the window-shaped adapter):
            // ambiguity is a miss, never a wrong type
            if let Some(fv) = super::unique_free_fn_in(idxs.iter().copied(), name) {
                return call_ret(home, idxs, ast, &fv, &segs[0].generics, args, qual);
            }
            // `Logger(..)` — the constructor call. Only an INDEXED
            // class/struct answers: a capitalized name with no type
            // head behind it (an enum, a plain unknown) stays a miss —
            // never wrong text. The free-fn match keeps precedence.
            // Generic args render as the OWNING doc's own source text
            // (the verbatim law): `HashMap<i32, i32>()` types the
            // binding `HashMap<i32, i32>`
            if is_cap(name) {
                let ty_hits: Vec<(&File, crate::world::views::TyView<'_>)> = idxs
                    .iter()
                    .filter_map(|f| f.ty(name).map(|t| (*f, t)))
                    .collect();
                if let Some((_, ty)) = prefer_mounted(&ty_hits).into_iter().next() {
                    if matches!(
                        ty.form(),
                        TyForm::Class | TyForm::Struct | TyForm::HostStruct
                    ) {
                        return Some(if segs[0].generics.is_empty() {
                            Ty::named(name)
                        } else {
                            written_arg_ty(home, ast, name, &segs[0].generics)
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
                    .map(|s| written_arg_ty(home, ast, name, &s.generics))
                    .unwrap_or_else(|| Ty::named(name)),
            );
        }
        // `m::mk(..)` — a module-qualified callee: the namespace lane
        // resolves it (the ONE home — the collection's
        // `qualified_scope`), the
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

/// the constructor-call type for `name<args>` — the head name beside
/// each argument as the OWNING doc's verbatim slice (`Doc::text` over
/// the arg's span, trimmed) joined `, `; the shape is the tree's, so
/// the substitution can still see through it. Nested args (`Vec<str>`)
/// ride their own spans by construction
fn written_arg_ty(home: &File, ast: &Ast, name: &str, args: &[TyRef]) -> Ty {
    let rendered: Vec<String> = args
        .iter()
        .map(|&a| home.doc.text(a.span(ast)).trim().to_string())
        .collect();
    let shape_args: Vec<Ty> = args.iter().map(|&a| Ty::of(ast, a)).collect();
    let text = format!("{name}<{}>", rendered.join(", "));
    Ty {
        text,
        shape: Shape::Named {
            segs: vec![name.to_string()],
            args: shape_args,
        },
    }
}

/// a member call's ret on `ty_name` — the impl-fn lane over the
/// window: the inherent-impl door's window-shaped adapter
/// ([`super::impl_method_fn_in`]) priced by [`call_ret`]
/// (the substitution, then the async wrap). `None` when no impl fn
/// answers — an own-surface member is FINAL with no recorded fn
/// (never a same-named impl fn from another file leaks through: the
/// member law's gate)
#[allow(clippy::too_many_arguments)] // the arity IS the resolver's shape — plumbing, not an API to nest
fn member_ret<'a>(
    home: &File,
    idxs: &[&'a File],
    ast: &Ast,
    ty_name: &str,
    member: &str,
    written: &[TyRef],
    args: &[ExprRef],
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<Ty> {
    super::impl_method_fn_in(idxs.iter().copied(), ty_name, member)
        .and_then(|f| call_ret(home, idxs, ast, &f, written, args, qual))
}

/// a callee fn view's ret at a call site — the written ret priced for
/// display. First-order substitution over the call site:
///
/// (a) the spelled call-site generics zip the decl's parameters
///     (`make<i32>()` names `T := i32`);
/// (b) each declared param whose type mentions a variable
///     structurally matches the arg's inferred type — heads equal,
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
    home: &File,
    idxs: &[&'a File],
    ast: &Ast,
    f: &FnView,
    written: &[TyRef],
    args: &[ExprRef],
    qual: &impl Fn(&[String], bool) -> Option<QualHit<'a>>,
) -> Option<Ty> {
    let vars = f.generics();
    let mut map: Vec<(String, Ty)> = Vec::new();
    for (v, w) in vars.iter().zip(written.iter()) {
        // the call site's own spelling — the HOME doc's verbatim slice
        let text = home.doc.text(w.span(ast)).trim().to_string();
        if !text.is_empty() {
            let shape = Ty::of(ast, *w).shape;
            bind_var(&mut map, v.clone(), Ty { text, shape });
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
        if !declared.mentions_any(&vars) {
            continue;
        }
        let Some(inferred) = expr_ty(home, idxs, *a, qual) else {
            continue;
        };
        declared.unify(&inferred, &vars, &mut map);
    }
    let Some(ret) = f.ret() else {
        // unwritten = `nil` (the engine's law) — only an async callee
        // answers it
        return f.is_async().then(|| future_ty(Ty::named("nil")));
    };
    let priced = ret.subst(&map);
    if priced.mentions_any(&vars) {
        return None;
    }
    Some(if f.is_async() {
        future_ty(priced)
    } else {
        priced
    })
}

/// the async wrap — a call to an `async` fn (or an async block)
/// answers `Future<R>`, the head's spelling (the engine's sugar law)
fn future_ty(inner: Ty) -> Ty {
    let text = format!("Future<{}>", inner.text);
    Ty {
        text,
        shape: Shape::Named {
            segs: vec!["Future".to_string()],
            args: vec![inner],
        },
    }
}

// ---- the substitution's bookkeeping ----

// `bind_var` lives beside the shape walks it feeds (`world::ty`).

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
fn field_ty(idxs: &[&File], ty_name: &str, field: &str) -> Option<Ty> {
    idxs.iter().copied().find_map(|i| {
        i.ty(ty_name)?
            .fields()
            .into_iter()
            .find(|f| f.name() == field)
            .and_then(|f| f.ty())
    })
}

/// a module let's type — the decl layer's contribution to receiver
/// inference (`let TOTAL = ..;` at module scope, read below). The FULL
/// declared-or-inferred type; the lanes that key a receiver head it
/// with [`Ty::headed`] (today's head-only answer)
pub(crate) fn module_let_ty<'a, I>(idxs: I, name: &str) -> Option<Ty>
where
    I: IntoIterator<Item = &'a File>,
{
    let hits: Vec<Option<Ty>> = idxs
        .into_iter()
        .flat_map(|f| {
            f.lets()
                .into_iter()
                .filter(|l| l.name() == name)
                .map(|l| match l.annotated() {
                    Some(t) => Some(t),
                    None => expr_ty(f, &[f], l.init(), &|_, _| None),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    match hits.as_slice() {
        [one] => one.clone(),
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

fn float_suffix(sfx: crate::doc::FloatSuffix) -> &'static str {
    match sfx {
        crate::doc::FloatSuffix::F32 => "f32",
        crate::doc::FloatSuffix::F64 => "f64",
    }
}
