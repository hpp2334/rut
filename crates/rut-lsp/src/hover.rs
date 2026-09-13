//! Hover 鈥?a definition index over the parsed document, plus lookup and
//! markdown rendering. One rule for method lookup (mirrors the
//! compiler's): **methods on type `T` = T's own surface (class body,
//! `host class`, `host primitive`) 鈭?interface methods from impls targeting
//! `T`.** Heuristic, like the classifier: a miss is an empty hover,
//! never wrong text. Signatures render as verbatim source slices 鈥?no
//! pretty-printer, truthful to what was written.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use crate::semantic::is_keyword;

// ---- the index ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TyForm {
    Class,
    Dataclass,
    Trait,
    Enum,
    HostClass,
    /// `host primitive string { .. }` 鈥?a primitive's native surface
    HostPrimitive,
}

impl TyForm {
    fn keyword(self) -> &'static str {
        match self {
            TyForm::Class => "class",
            TyForm::Dataclass => "dataclass",
            TyForm::Trait => "trait",
            TyForm::Enum => "enum",
            TyForm::HostClass => "host class",
            TyForm::HostPrimitive => "host primitive",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemberSrc {
    pub name: String,
    /// verbatim source of the declaration (signature for methods)
    pub src: String,
    pub doc: Vec<String>,
    /// 1-based line of the declaration
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct TyDef {
    pub name: String,
    pub form: TyForm,
    pub generics: Vec<String>,
    pub fields: Vec<MemberSrc>,
    pub methods: Vec<MemberSrc>,
    pub doc: Vec<String>,
    pub span: Span,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    /// verbatim signature source
    pub src: String,
    pub doc: Vec<String>,
    /// `Circle` for an inherent method, `impl Drawable for Circle` for a
    /// trait-impl method, `None` for a free fn
    pub owner: Option<String>,
    pub span: Span,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct ImplDef {
    pub trait_name: String,
    pub target_name: String,
}

#[derive(Debug, Default)]
pub struct DefIndex {
    pub types: Vec<TyDef>,
    pub fns: Vec<FnDef>,
    pub impls: Vec<ImplDef>,
    /// label for provenance lines 鈥?the file's path, or `std:core`
    pub origin: String,
}

impl DefIndex {
    pub fn ty(&self, name: &str) -> Option<&TyDef> {
        self.types.iter().find(|t| t.name == name)
    }
}

// ---- building ----

/// `line` of a byte offset, 1-based
fn line_of(src: &str, lo: u32) -> u32 {
    1 + src[..lo as usize].matches('\n').count() as u32
}

/// doc comment lines directly above `lo`: consecutive `//` lines with no
/// blank line between them and the decl (comments are not tokens 鈥?the
/// lexer skips them 鈥?so this scans the source itself)
fn doc_before(src: &str, lo: u32) -> Vec<String> {
    let mut out = Vec::new();
    // walk whole lines upward from the line containing `lo`
    let mut end = src[..lo as usize].trim_end().len();
    loop {
        let start = src[..end].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = src[start..end].trim();
        if let Some(rest) = line.strip_prefix("//") {
            out.push(rest.trim_start().to_string());
        } else {
            break;
        }
        if start == 0 {
            break;
        }
        end = start - 1;
    }
    out.reverse();
    out
}

/// verbatim signature: source up to (not including) the body; bodiless
/// decls keep their span minus the trailing `;`
fn sig_src(src: &str, ast: &Ast, span: Span, body: Option<NodeHandle<BlockNode>>) -> String {
    let hi = match body {
        Some(b) => ast.span(b.id()).lo,
        None => span.hi,
    };
    src[span.lo as usize..hi as usize]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

fn member(src: &str, name: &str, sig: String, span: Span) -> MemberSrc {
    MemberSrc {
        name: name.to_string(),
        src: sig,
        doc: doc_before(src, span.lo),
        line: line_of(src, span.lo),
    }
}

fn members_of(src: &str, ast: &Ast, methods: &[NodeHandle<MethodDeclNode>]) -> Vec<MemberSrc> {
    methods
        .iter()
        .map(|m| {
            let d = ast.method_decl(*m);
            let sp = ast.span(m.id());
            // `pub(..)`/`suspend` sit before `fn` — outside the decl span;
            // restore them from the flags (truthful to intent)
            let mut pre = String::new();
            if let Some(v) = d.vis {
                pre.push_str(&member_vis_str(v));
                pre.push(' ');
            }
            if d.is_suspend {
                pre.push_str("suspend ");
            }
            let sig = format!("{pre}{}", sig_src(src, ast, sp, d.body));
            member(src, ast.name(d.name), sig, sp)
        })
        .collect()
}

/// the `pub`-form spelling of a member visibility
fn member_vis_str(v: Vis) -> String {
    match v {
        Vis::Pub => "pub".to_string(),
        Vis::Mod => "pub(mod)".to_string(),
        Vis::Super => "pub(super)".to_string(),
        Vis::Self_ => "pub(self)".to_string(),
    }
}

/// strip leading member modifiers (`pub`, `pub(..)`, `static`) from a
/// source slice — they are re-rendered from the decl's flags
fn strip_member_mods(mut s: &str) -> &str {
    loop {
        let t = s.trim_start();
        let word_end = t.find(char::is_whitespace).unwrap_or(t.len());
        let rest = &t[word_end..];
        match &t[..word_end] {
            "pub" => {
                let r = rest.trim_start();
                if let Some(inner) = r.strip_prefix('(') {
                    if let Some(i) = inner.find(')') {
                        s = &inner[i + 1..];
                        continue;
                    }
                }
                s = rest;
            }
            "static" => s = rest,
            _ => return t,
        }
    }
}

/// field member list: the decl slice carries the modifiers (the field
/// span starts before them) — strip, then re-render from the flags
fn field_members(src: &str, ast: &Ast, fields: &[NodeHandle<FieldDeclNode>]) -> Vec<MemberSrc> {
    fields
        .iter()
        .map(|f| {
            let d = ast.field_decl(*f);
            let sp = ast.span(f.id());
            let mut vis = String::new();
            if let Some(v) = d.vis {
                vis.push_str(&member_vis_str(v));
                vis.push(' ');
            }
            if d.is_static {
                vis.push_str("static ");
            }
            let body = strip_member_mods(
                src[sp.lo as usize..sp.hi as usize]
                    .trim_end()
                    .trim_end_matches(';'),
            )
            .to_string();
            let decl = format!("{vis}{body}");
            member(src, ast.name(d.name), decl, sp)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn ty_def(
    src: &str,
    _ast: &Ast,
    name: &str,
    form: TyForm,
    generics: Vec<String>,
    fields: Vec<MemberSrc>,
    methods: Vec<MemberSrc>,
    span: Span,
) -> TyDef {
    TyDef {
        name: name.to_string(),
        form,
        generics,
        fields,
        methods,
        doc: doc_before(src, span.lo),
        span,
        line: line_of(src, span.lo),
    }
}

fn generics_of(ast: &Ast, gs: &[IdentId]) -> Vec<String> {
    gs.iter().map(|&g| ast.name(g).to_string()).collect()
}

/// Build the index for one document.
pub fn index(src: &str, ast: &Ast) -> DefIndex {
    let mut idx = DefIndex::default();
    for h in ast.module_items(ast.root) {
        let span = ast.span(h.id());
        match ast.item(*h) {
            ItemKind::Class { name, generics, fields, methods, .. } => {
                let fs = field_members(src, ast, fields);
                let ms = members_of(src, ast, methods);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::Class,
                    generics_of(ast, generics),
                    fs,
                    ms,
                    span,
                ));
            }
            ItemKind::Dataclass { name, generics, fields, methods, .. } => {
                let fs = field_members(src, ast, fields);
                let ms = members_of(src, ast, methods);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::Dataclass,
                    generics_of(ast, generics),
                    fs,
                    ms,
                    span,
                ));
            }
            ItemKind::Trait { name, generics, methods, .. } => {
                let ms = members_of(src, ast, methods);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::Trait,
                    generics_of(ast, generics),
                    Vec::new(),
                    ms,
                    span,
                ));
            }
            ItemKind::Enum { name, members, .. } => {
                // member list renders from names 鈥?spans would need token
                // recovery; names are the hover's content
                let ms = members
                    .iter()
                    .map(|(m, _)| MemberSrc {
                        name: ast.name(*m).to_string(),
                        src: ast.name(*m).to_string(),
                        doc: Vec::new(),
                        line: line_of(src, span.lo),
                    })
                    .collect();
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::Enum,
                    Vec::new(),
                    ms,
                    Vec::new(),
                    span,
                ));
            }
            ItemKind::Impl { trait_ref, target, methods, .. } => {
                let owner = format!(
                    "impl {} for {}",
                    ty_head(ast, *trait_ref),
                    ty_head(ast, *target)
                );
                idx.impls.push(ImplDef {
                    trait_name: ty_head(ast, *trait_ref),
                    target_name: ty_head(ast, *target),
                });
                for m in methods {
                    let d = ast.method_decl(*m);
                    let sp = ast.span(m.id());
                    idx.fns.push(FnDef {
                        name: ast.name(d.name).to_string(),
                        src: sig_src(src, ast, sp, d.body),
                        doc: doc_before(src, sp.lo),
                        owner: Some(owner.clone()),
                        span: sp,
                        line: line_of(src, sp.lo),
                    });
                }
            }
            ItemKind::Fn(d) => {
                idx.fns.push(FnDef {
                    name: ast.name(d.name).to_string(),
                    src: sig_src(src, ast, span, Some(d.body)),
                    doc: doc_before(src, span.lo),
                    owner: None,
                    span,
                    line: line_of(src, span.lo),
                });
            }
            ItemKind::SurfaceFn { name, .. } => {
                idx.fns.push(FnDef {
                    name: ast.name(*name).to_string(),
                    src: sig_src(src, ast, span, None),
                    doc: doc_before(src, span.lo),
                    owner: None,
                    span,
                    line: line_of(src, span.lo),
                });
            }
            ItemKind::SurfaceClass { name, members, .. } => {
                let ms = members_of(src, ast, members);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::HostClass,
                    Vec::new(),
                    Vec::new(),
                    ms,
                    span,
                ));
            }
            ItemKind::ModuleLet { .. } | ItemKind::Import { .. } | ItemKind::Module { .. } => {}
        }
    }
    idx
}

/// head segment text of a type node (`Circle`, `Vec` in `Vec<T>`)
fn ty_head(ast: &Ast, h: NodeHandle<AnyTy>) -> String {
    match ast.ty(h) {
        TypeKind::TyPath { segs, .. } => segs
            .first()
            .map(|s| ast.name(s.name).to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

// ---- lookup ----

fn contains(sp: Span, pos: u32) -> bool {
    sp.lo <= pos && pos < sp.hi
}

/// The hover result: markdown body + the span it decorates.
pub struct HoverOut {
    pub markdown: String,
    pub span: Span,
}

/// `idxs` in priority order — the open document first, then the std
/// surface, then the rest of the workspace.
pub fn hover(
    idxs: &[&DefIndex],
    src: &str,
    toks: &[Token],
    ast: &Ast,
    pos: u32,
) -> Option<HoverOut> {
    let t = tok_at(toks, pos)?;
    let Tok::Ident(name) = &t.tok else { return None };
    if is_keyword(name) {
        if name == "self" {
            let (i, ty) = enclosing_type(idxs, pos)?;
            return Some(HoverOut { markdown: render_ty(i, ty), span: t.span });
        }
        return None;
    }

    // member position: `recv.member`
    if let Some(recv) = member_context(toks, t) {
        if let Some(out) = member_hover(idxs, src, ast, pos, name, recv) {
            return Some(HoverOut { markdown: out, span: t.span });
        }
        // unresolved receiver falls through to the name search below
    }

    // type position: capitalized or primitive names
    if is_cap(name) || rut_parser::is_primitive_ty(name) {
        if let Some((i, ty)) = find_ty(idxs, name) {
            return Some(HoverOut { markdown: render_ty(i, ty), span: t.span });
        }
    }

    // bare name: unique fn match (free fns, methods); ambiguity lists
    let hits: Vec<(&DefIndex, &FnDef)> = idxs
        .iter()
        .flat_map(|i| i.fns.iter().filter(|f| &f.name == name).map(move |f| (*i, f)))
        .collect();
    match hits.as_slice() {
        [] => None,
        [(_, one)] => Some(HoverOut { markdown: render_fn_hits(&hits, one), span: t.span }),
        _ => Some(HoverOut { markdown: render_candidates(&hits), span: t.span }),
    }
}

/// token under (or immediately before) `pos`
fn tok_at<'t>(toks: &'t [Token], pos: u32) -> Option<&'t Token> {
    toks.iter().rev().find(|t| t.span.lo <= pos && pos <= t.span.hi)
}

/// when the hovered token follows a dot: the receiver's source text —
/// an identifier, or the `string` primitive for string literals
fn member_context<'t>(toks: &'t [Token], t: &Token) -> Option<String> {
    let i = toks.iter().position(|x| x.span.lo == t.span.lo)?;
    if i < 2 || toks[i - 1].tok != Tok::Dot {
        return None;
    }
    match &toks[i - 2].tok {
        Tok::Ident(s) => Some(s.clone()),
        Tok::Str(_) => Some("string".to_string()),
        _ => None,
    }
}

fn is_cap(s: &str) -> bool {
    s.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
}

/// resolve the receiver's type name
fn recv_type(idxs: &[&DefIndex], src: &str, ast: &Ast, pos: u32, recv: &str) -> Option<String> {
    match recv {
        "self" | "Self" => enclosing_type(idxs, pos).map(|(_, t)| t.name.clone()),
        _ if is_cap(recv) => Some(recv.to_string()),
        _ if rut_parser::is_primitive_ty(recv) => Some(recv.to_string()),
        _ => infer_local(src, ast, pos, recv),
    }
}

/// the type whose body contains `pos` — a class/dataclass/interface body, or
/// the target of the enclosing impl (via the method's owner)
fn enclosing_type<'a>(idxs: &'a [&'a DefIndex], pos: u32) -> Option<(&'a DefIndex, &'a TyDef)> {
    let mut best: Option<(u32, &'a DefIndex, &'a TyDef)> = None;
    for i in idxs.iter().copied() {
        for t in &i.types {
            if t.form == TyForm::Enum || !contains(t.span, pos) {
                continue;
            }
            if best.map(|(l, _, _)| t.span.hi - t.span.lo < l).unwrap_or(true) {
                best = Some((t.span.hi - t.span.lo, i, t));
            }
        }
        for f in &i.fns {
            if let Some(target) = f.owner.as_deref().and_then(|o| o.rsplit(" for ").next()) {
                if let Some(t) = i.ty(target) {
                    if contains(f.span, pos)
                        && best.map(|(l, _, _)| f.span.hi - f.span.lo < l).unwrap_or(true)
                    {
                        best = Some((f.span.hi - f.span.lo, i, t));
                    }
                }
            }
        }
    }
    best.map(|(_, i, t)| (i, t))
}

/// cheap local inference for a lowercase receiver: the enclosing
/// fn/method's params (`fn f(c: Circle)`), then `let` bindings before
/// `pos` (`let p = Circle.new(..)`, `let p: Point = ..`, `let s = ".."`)
fn infer_local(_src: &str, ast: &Ast, pos: u32, name: &str) -> Option<String> {
    // locate the enclosing fn/method body — smallest containing span
    let mut body: Option<(u32, NodeHandle<AnyExpr>)> = None;
    let mut see = |sp: Span, b: Option<NodeHandle<BlockNode>>| {
        if let Some(b) = b {
            if contains(sp, pos) && body.map(|(l, _)| sp.hi - sp.lo < l).unwrap_or(true) {
                body = Some((sp.hi - sp.lo, b.into()));
            }
        }
    };
    for h in ast.module_items(ast.root) {
        let sp = ast.span(h.id());
        match ast.item(*h) {
            ItemKind::Fn(d) => see(sp, Some(d.body)),
            ItemKind::Class { methods, .. }
            | ItemKind::Dataclass { methods, .. }
            | ItemKind::Trait { methods, .. }
            | ItemKind::Impl { methods, .. } => {
                for m in methods {
                    see(ast.span(m.id()), ast.method_decl(*m).body);
                }
            }
            _ => {}
        }
    }
    let (_, block) = body?;

    // params of that fn/method
    for h in ast.module_items(ast.root) {
        let sp = ast.span(h.id());
        if !contains(sp, pos) {
            continue;
        }
        match ast.item(*h) {
            ItemKind::Fn(d) => {
                if contains(sp, pos) {
                    if let Some(t) = param_ty(ast, &d.params, name) {
                        return Some(t);
                    }
                }
            }
            ItemKind::Class { methods, .. }
            | ItemKind::Dataclass { methods, .. }
            | ItemKind::Trait { methods, .. }
            | ItemKind::Impl { methods, .. } => {
                for m in methods {
                    let msp = ast.span(m.id());
                    if contains(msp, pos) {
                        let d = ast.method_decl(*m);
                        if let Some(t) = param_ty(ast, &d.params, name) {
                            return Some(t);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // lets before pos within the body (flat scan; heuristic by design)
    let mut found: Option<String> = None;
    walk_lets(ast, block, &mut |stmt| {
        let sp = ast.span(stmt.id());
        if sp.lo < pos {
            if let Kind::Stmt(StmtKind::LetStmt { name: n, ty, init, .. }) = ast.kind(stmt.id()) {
                if ast.name(*n) == name && found.is_none() {
                    if let Some(t) = ty {
                        found = Some(ty_head(ast, *t));
                    } else if let Some(inferred) = init_ty(ast, *init) {
                        found = Some(inferred);
                    }
                }
            }
        }
    });
    found
}

fn param_ty(ast: &Ast, params: &[NodeHandle<AnyParam>], name: &str) -> Option<String> {
    for p in params {
        if let MemberKind::Param(d) = ast.param(*p) {
            if ast.name(d.name) == name {
                if let Some(ty) = d.ty {
                    return Some(ty_head(ast, ty));
                }
            }
        }
    }
    None
}

/// the type an initializer suggests: `Circle.new(..)`, `Circle { .. }`,
/// string literals
fn init_ty(ast: &Ast, init: NodeHandle<AnyExpr>) -> Option<String> {
    match ast.expr(init) {
        ExprKind::Method { recv, name, .. } if ast.name(*name) == "new" => {
            if let ExprKind::Path { segs } = ast.expr(*recv) {
                segs.first().map(|s| ast.name(s.name).to_string())
            } else {
                None
            }
        }
        ExprKind::Struct { ty, .. } => Some(ty_head(ast, *ty)),
        ExprKind::Lit(Lit::Str(_)) => Some("string".to_string()),
        _ => None,
    }
}

/// visit every LetStmt in the block tree under `node`
fn walk_lets(ast: &Ast, node: NodeHandle<AnyExpr>, f: &mut dyn FnMut(NodeHandle<AnyStmt>)) {
    if let ExprKind::Block { stmts } = ast.expr(node) {
        for s in stmts {
            if let Kind::Stmt(StmtKind::LetStmt { .. }) = ast.kind(s.id()) {
                f(*s);
            }
            stmt_exprs(ast, *s, &mut |e| walk_lets(ast, e, f));
        }
    }
}

/// expressions held by a statement (conditions, inits, bodies)
fn stmt_exprs(ast: &Ast, s: NodeHandle<AnyStmt>, f: &mut dyn FnMut(NodeHandle<AnyExpr>)) {
    match ast.kind(s.id()) {
        Kind::Stmt(StmtKind::LetStmt { init, .. }) => f(*init),
        Kind::Stmt(StmtKind::If { cond, then, els }) => {
            f(*cond);
            f(NodeHandle::<AnyExpr>::from(*then));
            if let Some(ElseBranch::Block(b)) = els {
                f(NodeHandle::<AnyExpr>::from(*b));
            }
        }
        Kind::Stmt(StmtKind::While { cond, body, .. }) => {
            f(*cond);
            f(NodeHandle::<AnyExpr>::from(*body));
        }
        Kind::Stmt(StmtKind::ForOf { iter, body, .. }) => {
            f(*iter);
            f(NodeHandle::<AnyExpr>::from(*body));
        }
        Kind::Stmt(StmtKind::ForC { init, cond, update, body, .. }) => {
            f(*init);
            f(*cond);
            f(*update);
            f(NodeHandle::<AnyExpr>::from(*body));
        }
        Kind::Stmt(StmtKind::Return { value }) => {
            if let Some(v) = value {
                f(*v);
            }
        }
        Kind::Stmt(StmtKind::WhenStmt { scrut, arms }) => {
            f(*scrut);
            for a in arms {
                if let ArmKind::WhenArm { body, .. } = ast.arm(*a) {
                    f(*body);
                }
            }
        }
        Kind::Stmt(StmtKind::ExprStmt(e)) => f(*e),
        _ => {}
    }
}

// ---- member / type resolution ----

fn find_ty<'a>(idxs: &'a [&DefIndex], name: &str) -> Option<(&'a DefIndex, &'a TyDef)> {
    idxs.iter().find_map(|i| i.ty(name).map(|t| (*i, t)))
}

fn member_hover(
    idxs: &[&DefIndex],
    src: &str,
    ast: &Ast,
    pos: u32,
    member: &str,
    recv: String,
) -> Option<String> {
    let ty_name = recv_type(idxs, src, ast, pos, &recv)?;
    let (ti, ty) = find_ty(idxs, &ty_name)?;
    // own surface first
    if let Some(m) = ty.methods.iter().find(|m| m.name == member) {
        return Some(render_member(ti, ty, m, None));
    }
    if let Some(f) = ty.fields.iter().find(|f| f.name == member) {
        return Some(render_member(ti, ty, f, None));
    }
    if ty.form == TyForm::Enum {
        // `Color.Red` — the member IS an enum member; show the enum
        if ty.fields.iter().any(|f| f.name == member) {
            return Some(render_ty(ti, ty));
        }
        return None;
    }
    // interface methods from impls targeting this type (the unified rule)
    for i in idxs {
        for im in &i.impls {
            if im.target_name == ty_name {
                if let Some(t) = i.ty(&im.trait_name) {
                    if let Some(m) = t.methods.iter().find(|m| m.name == member) {
                        return Some(render_member(
                            i,
                            t,
                            m,
                            Some(format!("impl {} for {}", im.trait_name, im.target_name)),
                        ));
                    }
                }
            }
        }
    }
    None
}

// ---- rendering ----

fn code_block(body: &str) -> String {
    format!("```rut\n{body}\n```")
}

fn provenance(origin: &str, line: u32) -> String {
    format!("— {origin}:{line}")
}

fn render_ty(i: &DefIndex, ty: &TyDef) -> String {
    let mut out = String::new();
    let gens = if ty.generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", ty.generics.join(", "))
    };
    match ty.form {
        TyForm::Enum => {
            let members: Vec<&str> = ty.fields.iter().map(|f| f.name.as_str()).collect();
            out.push_str(&code_block(&format!(
                "enum {}{} {{ {} }}",
                ty.name,
                gens,
                members.join(", ")
            )));
        }
        TyForm::Trait | TyForm::HostClass | TyForm::HostPrimitive => {
            let mut body = String::new();
            for m in &ty.methods {
                body.push_str("    ");
                body.push_str(&m.src);
                body.push('\n');
            }
            out.push_str(&code_block(&format!(
                "{} {}{} {{\n{}}}",
                ty.form.keyword(),
                ty.name,
                gens,
                body
            )));
        }
        TyForm::Class | TyForm::Dataclass => {
            let mut body = String::new();
            for f in &ty.fields {
                body.push_str("    ");
                body.push_str(&f.src);
                body.push('\n');
            }
            out.push_str(&code_block(&format!(
                "{} {}{} {{\n{}}}",
                ty.form.keyword(),
                ty.name,
                gens,
                body
            )));
            if !ty.methods.is_empty() {
                out.push_str(&format!(
                    "\n{} method{} — hover one for its signature",
                    ty.methods.len(),
                    if ty.methods.len() == 1 { "" } else { "s" }
                ));
            }
        }
    }
    if !i.origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(&i.origin, ty.line)));
    }
    for d in &ty.doc {
        out.push_str(&format!("\n\n{}", d));
    }
    out
}

fn render_member(i: &DefIndex, ty: &TyDef, m: &MemberSrc, via: Option<String>) -> String {
    let mut out = code_block(&m.src);
    match via {
        Some(v) => out.push_str(&format!("\nfrom `{v}`")),
        None if ty.form == TyForm::Trait => {
            out.push_str(&format!("\ndeclared in `{}`", ty.name));
        }
        None => out.push_str(&format!("\nin `{}`", ty.name)),
    }
    if !i.origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(&i.origin, m.line)));
    }
    for d in &m.doc {
        out.push_str(&format!("\n\n{}", d));
    }
    out
}

fn render_fn_hits(hits: &[(&DefIndex, &FnDef)], one: &FnDef) -> String {
    let i = hits.iter().find(|(_, f)| f.span == one.span).map(|(i, _)| *i);
    let mut out = code_block(&one.src);
    if let Some(o) = &one.owner {
        out.push_str(&format!("\nin `{o}`"));
    }
    if let Some(i) = i {
        if !i.origin.is_empty() {
            out.push_str(&format!("\n{}", provenance(&i.origin, one.line)));
        }
    }
    for d in &one.doc {
        out.push_str(&format!("\n\n{}", d));
    }
    out
}

fn render_candidates(many: &[(&DefIndex, &FnDef)]) -> String {
    let mut out = String::from("multiple definitions:");
    for (i, f) in many {
        let where_ = match &f.owner {
            Some(o) => format!(" ({o})"),
            None => String::new(),
        };
        out.push_str(&format!(
            "\n- `{}`{} — {}:{}",
            f.src.replace('\n', " "),
            where_,
            i.origin,
            f.line
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;

    fn hover_at(src: &str, needle: &str) -> Option<String> {
        hover_nth(src, needle, 0)
    }

    /// hover the `n`-th (0-based) occurrence of `needle` — tests target
    /// use sites, which usually aren't the declaration
    fn hover_nth(src: &str, needle: &str, n: usize) -> Option<String> {
        let src2 = rut_lexer::lexer::normalize(src);
        let (toks, _) = rut_lexer::lexer::lex(&src2);
        let (ast, _) = rut_parser::parse(&src2, rut_parser::Mode::Impl);
        let idx = index(&src2, &ast);
        let idxs = [&idx];
        let pos = find_ident_pos(&toks, needle, n)?;
        hover(&idxs, &src2, &toks, &ast, pos).map(|h| h.markdown)
    }

    fn find_ident_pos(toks: &[Token], name: &str, n: usize) -> Option<u32> {
        toks.iter()
            .filter(|t| matches!(&t.tok, Tok::Ident(s) if s == name))
            .nth(n)
            .map(|t| t.span.lo)
    }

    #[test]
    fn method_hover_shows_signature_doc_and_owner() {
        let src = "\
// a 2D point
dataclass Point {
    x: f64;
    y: f64;
}

// euclidean length
fn length(p: Point) -> f64 {
    return p.x;
}
";
        // 2nd occurrence: the use site `p.x`, not the field decl
        let md = hover_nth(src, "x", 1).unwrap();
        assert!(md.contains("```rut"), "code block: {md}");
        assert!(md.contains("x: f64"), "field decl: {md}");
        assert!(md.contains("in `Point`"), "owner: {md}");
    }

    #[test]
    fn class_hover_shows_struct_definition() {
        let src = "\
// a circle
class Circle {
    pub r: f64;
    x: f64;
    y: f64;
}
";
        let md = hover_at(src, "Circle").unwrap();
        assert!(md.contains("class Circle {"), "{md}");
        assert!(md.contains("pub r: f64"), "{md}");
        assert!(md.contains("x: f64"), "{md}");
        assert!(md.contains("a circle"), "doc: {md}");
    }

    #[test]
    fn method_via_inference_and_self() {
        let src = "\
class Circle {
    r: f64;
    fn area(self) -> f64 { return 3.14; }
    fn grow(self, k: f64) -> unit { self.r = self.r * k; }
}
fn use_it() -> f64 {
    let c = Circle.new(1.0);
    return c.area();
}
";
        // 2nd occurrence: the call `c.area()`, receiver inferred from
        // `let c = Circle.new(..)`
        let md = hover_nth(src, "area", 1).unwrap();
        assert!(md.contains("fn area(self) -> f64"), "{md}");
        assert!(md.contains("in `Circle`"), "{md}");
    }

    #[test]
    fn trait_method_via_impl() {
        let src = "\
interface Drawable {
    fn draw(self) -> unit;
}
class Circle {
    r: f64;
}
impl Drawable for Circle {
    fn draw(self) -> unit { }
}
fn render(d: Circle) -> unit {
    d.draw();
}
";
        // 3rd occurrence: the call `d.draw()` — param-typed receiver,
        // resolved through the impl (the unified rule)
        let md = hover_nth(src, "draw", 2).unwrap();
        assert!(md.contains("fn draw(self) -> unit"), "{md}");
        assert!(md.contains("from `impl Drawable for Circle`"), "{md}");
    }

    #[test]
    fn free_fn_unique_match() {
        let src = "\
// euclidean length
fn length(p: i32) -> f64 { return 1.0; }
fn main() -> unit { let x = length(3); }
";
        let md = hover_at(src, "length").unwrap();
        assert!(md.contains("fn length(p: i32) -> f64"), "{md}");
        assert!(md.contains("euclidean length"), "doc: {md}");
    }

    #[test]
    fn ambiguity_lists_candidates() {
        // two workspace files each defining `helper`
        let mk = |origin: &str| {
            let f = rut_lexer::lexer::normalize("fn helper() -> unit { }\n");
            let (fa, _) = rut_parser::parse(&f, rut_parser::Mode::Impl);
            let mut i = index(&f, &fa);
            i.origin = origin.to_string();
            i
        };
        let a = mk("a.rut");
        let b = mk("b.rut");

        let src = "fn go() -> unit { let h = helper(); }\n";
        let s = rut_lexer::lexer::normalize(src);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let mut doc = index(&s, &ast);
        doc.origin = "main.rut".to_string();
        let idxs = [&doc, &a, &b];
        let pos = find_ident_pos(&toks, "helper", 0).unwrap();
        let md = hover(&idxs, &s, &toks, &ast, pos).unwrap().markdown;
        assert!(md.contains("multiple definitions"), "{md}");
        assert!(md.contains("a.rut") && md.contains("b.rut"), "{md}");
    }

    #[test]
    fn host_fn_surface_favors_own_methods() {
        // std-style surface index ahead of the doc
        let surf_src = "pub host fn string_len(s: string) -> i32;\n";
        let s2 = rut_lexer::lexer::normalize(surf_src);
        let (sast, _) = rut_parser::parse(&s2, rut_parser::Mode::Decl);
        let mut surf = index(&s2, &sast);
        surf.origin = "std:core".to_string();

        let doc = "fn main() -> i32 { return string_len(\"abc\"); }\n";
        let d2 = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&d2);
        let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
        let mut di = index(&d2, &ast);
        di.origin = "main.rut".to_string();
        let idxs = [&di, &surf];
        let pos = find_ident_pos(&toks, "string_len", 0).unwrap();
        let h = hover(&idxs, &d2, &toks, &ast, pos).unwrap();
        assert!(h.markdown.contains("fn string_len"), "{}", h.markdown);
        assert!(h.markdown.contains("std:core"), "{}", h.markdown);
    }

    #[test]
    fn miss_is_none() {
        let src = "fn main() -> unit { let z = unknown_thing; }\n";
        assert!(hover_at(src, "unknown_thing").is_none());
    }
}
