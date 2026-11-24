//! L4 — types are values. A display type is a [`Ty`]: its verbatim
//! spelling beside the [`Shape`] read off the tree. Nothing re-parses
//! a type from its own spelling — the substitution, the structural
//! match, and the lookup heads all walk the shape; the text is the
//! rendering (a `Display`), never an input.
//!
//! Two construction sites: [`Ty::of`] renders a tree node by the
//! `ty_src` rules (path segments join `::`, generics join `, `, the
//! opt prefix spells `?`, unions join ` | `), and [`Ty::of_annotation`]
//! carries the WRITTEN annotation's verbatim span join beside its tree
//! shape (the bindings' declared rows — the verbatim law). A call-site
//! generic argument's `text` is the verbatim source slice (the
//! inference's own law), so as-written types render exactly as today.

use rut_ast::{
    AnyHandle, Ast, SelfTyNode, TyArrayNode, TyFnNode, TyOptNode, TyPathNode, TyRef, TyTupleNode,
    TyUnionNode,
};

use crate::world::File;

/// the decl layer's shared type-form vocabulary — the view
/// classification's dial (`TyView::form`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TyForm {
    Class,
    Struct,
    /// `interface Name<..> { fn sig(..); }` — an observed-capability
    /// declaration (signatures only; satisfaction is structural, so a
    /// type qualifies by having the members, never by registering)
    Interface,
    Enum,
    /// `builtin Name<..>` — an engine builtin's member contract (core
    /// only; members are compiler-lowered)
    Builtin,
    /// `builtin primitive <name>` — a boot primitive's surface statement
    /// (`str`/`bytes`/`opaque`); members are compiler-lowered
    Primitive,
    /// `host struct Name { fields }` — a flat host-constructed record
    HostStruct,
    /// `type X = A;` — a transparent alias
    Alias,
}

/// a display type: its verbatim spelling beside the shape read off
/// the tree. Never re-parsed from `text`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Ty {
    pub text: String,
    pub shape: Shape,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Shape {
    /// `i32`, `str`, `pouch::Vec`, `Vec<T>` — the segments and the last segment's arguments
    Named {
        segs: Vec<String>,
        args: Vec<Ty>,
    },
    Opt(Box<Ty>),
    Array(Box<Ty>),
    Tuple(Vec<Ty>),
    Union(Vec<Ty>),
    Selfy,
    /// a spelling kept for display but not decomposed (fn types, a damaged slot)
    Opaque,
}

impl Ty {
    /// from the tree, rendered by today's `ty_src` rules — path
    /// segments join `::`, the last segment's generics join `, `, the
    /// opt prefix spells `?`, arrays `[t]`, tuples `(a, b)`, unions
    /// `a | b`, `Self` its own word; a fn type or a damaged slot
    /// renders empty (an `Opaque` shape)
    pub(crate) fn of(ast: &Ast, t: TyRef) -> Ty {
        Ty {
            text: render(ast, t),
            shape: shape_of(ast, t),
        }
    }

    /// the WRITTEN annotation — the verbatim span join beside the
    /// shape read off the tree (the node under the span's first byte;
    /// a damaged slot is `Opaque`). The bindings' and module lets'
    /// declared rows — the verbatim law.
    pub(crate) fn of_annotation(file: &File, sp: rut_lexer::span::Span) -> Ty {
        let ast = file.doc.ast();
        let shape = ast
            .node_at(sp.lo)
            .and_then(ty_ref_of_any)
            .map(|t| shape_of(ast, t))
            .unwrap_or(Shape::Opaque);
        Ty {
            text: file.doc.text(sp).to_string(),
            shape,
        }
    }

    /// a computed name spelled as a type — the literals' primitives,
    /// `self`'s enclosing type, a declaring view's bare name. One
    /// segment, no arguments.
    pub(crate) fn named(name: &str) -> Ty {
        Ty {
            text: name.to_string(),
            shape: Shape::Named {
                segs: vec![name.to_string()],
                args: Vec::new(),
            },
        }
    }

    /// the lookup head — what member lookup keys on: `?Vec<i32>` →
    /// `Vec`, `m::Counter` → `m::Counter` (today's `head_of_ty`: the
    /// `?` peeled, the path joined, the generic arguments cut).
    /// Display keeps the full text; only lookups cut it down. A
    /// `String`, not a `&str`: the multi-segment join has no borrowed
    /// home in the shape.
    pub(crate) fn head(&self) -> String {
        match &self.shape {
            Shape::Opt(inner) => inner.head(),
            Shape::Named { segs, .. } => segs.join("::"),
            _ => self.text.clone(),
        }
    }

    /// the head as its own value — the lookup key carried as a `Ty`
    /// (single segment, so the qualified lane cannot fire on it): the
    /// module-let and inferred-expr receiver lanes key the HEAD only,
    /// never the qualified walk.
    pub(crate) fn headed(&self) -> Ty {
        Ty::named(&self.head())
    }

    /// `?T` → `T`; the await lane's strict head refuses to peel (the
    /// engine rejects awaiting an optional future)
    pub(crate) fn opt_inner(&self) -> Option<&Ty> {
        match &self.shape {
            Shape::Opt(inner) => Some(inner),
            _ => None,
        }
    }

    /// `Future<X>` → `X`, `[X]` → `X` — the element type of a sequence
    /// (the survey's for-of rule) and the await unwrap's first
    /// argument. The `?` peel rides along; `None` when the shape
    /// spells no element
    pub(crate) fn first_arg(&self) -> Option<&Ty> {
        match &self.shape {
            Shape::Opt(inner) => inner.first_arg(),
            Shape::Array(inner) => Some(inner),
            Shape::Named { args, .. } => args.first(),
            _ => None,
        }
    }

    /// replace each bound variable — a whole `Named` leaf (a bare
    /// single-segment name) or a bare segment — with its bound value;
    /// re-renders generics joined `, ` and paths joined `::`. One
    /// pass: a substituted value is embedded verbatim, never
    /// re-substituted (today's `apply_subst` law)
    pub(crate) fn subst(&self, map: &[(String, Ty)]) -> Ty {
        if map.is_empty() {
            return self.clone();
        }
        let out = match &self.shape {
            Shape::Named { segs, args } => {
                // the whole leaf is a bare bound variable — swap it
                if segs.len() == 1 && args.is_empty() {
                    if let Some((_, v)) = map.iter().find(|(v, _)| v == &segs[0]) {
                        return v.clone();
                    }
                }
                let segs: Vec<String> = segs
                    .iter()
                    .map(|s| match map.iter().find(|(v, _)| v == s) {
                        Some((_, v)) => v.text.clone(),
                        None => s.clone(),
                    })
                    .collect();
                let args: Vec<Ty> = args.iter().map(|a| a.subst(map)).collect();
                Ty {
                    text: render_named(&segs, &args),
                    shape: Shape::Named { segs, args },
                }
            }
            Shape::Opt(inner) => {
                let inner = inner.subst(map);
                Ty {
                    text: format!("?{}", inner.text),
                    shape: Shape::Opt(Box::new(inner)),
                }
            }
            Shape::Array(inner) => {
                let inner = inner.subst(map);
                Ty {
                    text: format!("[{}]", inner.text),
                    shape: Shape::Array(Box::new(inner)),
                }
            }
            Shape::Tuple(elems) => {
                let elems: Vec<Ty> = elems.iter().map(|e| e.subst(map)).collect();
                Ty {
                    text: render_joined(&elems, "(", ")", ", "),
                    shape: Shape::Tuple(elems),
                }
            }
            Shape::Union(elems) => {
                let elems: Vec<Ty> = elems.iter().map(|e| e.subst(map)).collect();
                Ty {
                    text: render_joined(&elems, "", "", " | "),
                    shape: Shape::Union(elems),
                }
            }
            // `Self` spells itself; an opaque slot keeps its spelling
            Shape::Selfy | Shape::Opaque => return self.clone(),
        };
        if out.text == self.text {
            // nothing substituted — the spelling stays verbatim
            return self.clone();
        }
        out
    }

    /// one first-order structural match: heads equal, generic arity
    /// equal, non-var text equal (today's `unify`). A bare variable
    /// binds whatever the arg inferred; later occurrences of a bound
    /// variable don't re-check — the honesty rides the residual rule
    pub(crate) fn unify(&self, inferred: &Ty, vars: &[String], map: &mut Vec<(String, Ty)>) {
        let d = peel_opt(self);
        let i = peel_opt(inferred);
        if let Shape::Named { segs, args } = &d.shape {
            if segs.len() == 1 && args.is_empty() && vars.iter().any(|v| v == &segs[0]) {
                bind_var(map, segs[0].clone(), i.clone());
                return;
            }
        }
        if let (Shape::Array(di), Shape::Array(ii)) = (&d.shape, &i.shape) {
            di.unify(ii, vars, map);
            return;
        }
        let (
            Shape::Named {
                segs: ds,
                args: dargs,
                ..
            },
            Shape::Named {
                segs: isegs,
                args: iargs,
                ..
            },
        ) = (&d.shape, &i.shape)
        else {
            return;
        };
        if ds.is_empty() || ds.join("::") != isegs.join("::") {
            return;
        }
        if !dargs.is_empty() && !iargs.is_empty() && dargs.len() == iargs.len() {
            for (a, b) in dargs.iter().zip(iargs.iter()) {
                a.unify(b, vars, map);
            }
        }
    }

    /// does any of `vars` occur as a whole name in the shape? The
    /// residual check's walk (today's `mentions_var` on text)
    pub(crate) fn mentions_any(&self, vars: &[String]) -> bool {
        match &self.shape {
            Shape::Named { segs, args } => {
                segs.iter().any(|s| vars.iter().any(|v| v == s))
                    || args.iter().any(|a| a.mentions_any(vars))
            }
            Shape::Opt(inner) | Shape::Array(inner) => inner.mentions_any(vars),
            Shape::Tuple(elems) | Shape::Union(elems) => elems.iter().any(|e| e.mentions_any(vars)),
            // `Self` names no variable; an opaque slot spells no tree
            // names (its text is empty by construction)
            Shape::Selfy | Shape::Opaque => false,
        }
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// first binding wins — a variable already bound keeps its first type
pub(crate) fn bind_var(map: &mut Vec<(String, Ty)>, var: String, ty: Ty) {
    if !map.iter().any(|(v, _)| *v == var) {
        map.push((var, ty));
    }
}

/// the `?` peels, all of them — the leading-question marks today's
/// text trim stripped
fn peel_opt(t: &Ty) -> &Ty {
    let mut cur = t;
    while let Shape::Opt(inner) = &cur.shape {
        cur = inner;
    }
    cur
}

/// a path's rendering — segments join `::`, the arguments join `, `
fn render_named(segs: &[String], args: &[Ty]) -> String {
    let mut s = segs.join("::");
    if !args.is_empty() {
        s.push_str(&format!(
            "<{}>",
            args.iter()
                .map(|a| a.text.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    s
}

fn render_joined(elems: &[Ty], open: &str, close: &str, sep: &str) -> String {
    let mut s = String::from(open);
    s.push_str(
        &elems
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join(sep),
    );
    s.push_str(close);
    s
}

/// display text of a type node — the alias hover's `type X = <target>`;
/// path segments join with `::` (the name-resolution law's spelling —
/// a `.` would read value access)
fn render(ast: &Ast, t: TyRef) -> String {
    match t {
        TyRef::Path(h) => {
            let d = h.get(ast);
            let segs: Vec<String> = d
                .segs
                .iter()
                .map(|sg| ast.name(sg.name.ident).to_string())
                .collect();
            let args: Vec<Ty> = d
                .segs
                .last()
                .map(|last| last.generics.iter().map(|&g| Ty::of(ast, g)).collect())
                .unwrap_or_default();
            render_named(&segs, &args)
        }
        // the prefix spelling renders with its payload: `?i32`, `??str`
        // (matches symbols.rs's `ty_text` — the memo all three renderers
        // must carry)
        TyRef::Opt(h) => format!("?{}", render(ast, h.get(ast).inner)),
        TyRef::Array(h) => format!("[{}]", render(ast, h.get(ast).elem)),
        // a written tuple annotation renders as written — `-> (opaque,
        // str)` used to render empty, which leaked `: `-style hints and
        // hovers (the inlay phase's find)
        TyRef::Tuple(h) => render_joined(
            &h.get(ast)
                .elems
                .iter()
                .map(|&e| Ty::of(ast, e))
                .collect::<Vec<_>>(),
            "(",
            ")",
            ", ",
        ),
        TyRef::Union(h) => render_joined(
            &h.get(ast)
                .elems
                .iter()
                .map(|&e| Ty::of(ast, e))
                .collect::<Vec<_>>(),
            "",
            "",
            " | ",
        ),
        // the enclosing type's own spelling (mirrors symbols.rs's
        // `ty_text` — the memo all three renderers carry): a residual
        // render shows the written word, never silent empty. Member
        // lookups key on it and miss — no type is NAMED `Self`
        TyRef::Self_(_) => "Self".to_string(),
        // a fn type or a damaged slot spells nothing
        _ => String::new(),
    }
}

/// the shape read off the tree — the segments, the wrappers, the
/// `Self` word; a fn type or a damaged slot stays `Opaque` (kept for
/// display, never decomposed)
fn shape_of(ast: &Ast, t: TyRef) -> Shape {
    match t {
        TyRef::Path(h) => {
            let d = h.get(ast);
            let segs = d
                .segs
                .iter()
                .map(|sg| ast.name(sg.name.ident).to_string())
                .collect();
            let args = d
                .segs
                .last()
                .map(|last| last.generics.iter().map(|&g| Ty::of(ast, g)).collect())
                .unwrap_or_default();
            Shape::Named { segs, args }
        }
        TyRef::Opt(h) => Shape::Opt(Box::new(Ty::of(ast, h.get(ast).inner))),
        TyRef::Array(h) => Shape::Array(Box::new(Ty::of(ast, h.get(ast).elem))),
        TyRef::Tuple(h) => Shape::Tuple(
            h.get(ast)
                .elems
                .iter()
                .map(|&e| Ty::of(ast, e))
                .collect::<Vec<_>>(),
        ),
        TyRef::Union(h) => Shape::Union(
            h.get(ast)
                .elems
                .iter()
                .map(|&e| Ty::of(ast, e))
                .collect::<Vec<_>>(),
        ),
        TyRef::Self_(_) => Shape::Selfy,
        TyRef::Fn(_) | TyRef::Error(_) => Shape::Opaque,
    }
}

/// the type node under an `AnyHandle` — the annotation shape's probe
/// (`node_at` answers the deepest node at a byte; the downcast ladder
/// reads whether it is a type)
fn ty_ref_of_any(h: AnyHandle) -> Option<TyRef> {
    if let Some(h) = h.downcast::<TyPathNode>() {
        return Some(TyRef::Path(h));
    }
    if let Some(h) = h.downcast::<SelfTyNode>() {
        return Some(TyRef::Self_(h));
    }
    if let Some(h) = h.downcast::<TyFnNode>() {
        return Some(TyRef::Fn(h));
    }
    if let Some(h) = h.downcast::<TyOptNode>() {
        return Some(TyRef::Opt(h));
    }
    if let Some(h) = h.downcast::<TyArrayNode>() {
        return Some(TyRef::Array(h));
    }
    if let Some(h) = h.downcast::<TyTupleNode>() {
        return Some(TyRef::Tuple(h));
    }
    if let Some(h) = h.downcast::<TyUnionNode>() {
        return Some(TyRef::Union(h));
    }
    None
}
