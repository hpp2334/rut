//! Building the index — one pass over a document's module items,
//! recovering verbatim signatures and doc comments from the source.
//! The decl layer (the lsp-features survey §3.2): fields, enum
//! members, module lets, and use-imported names all carry their name
//! spans — token recovery over the lexed stream (`bindings::ident_span`;
//! names are `IdentId`s, no parser changes) — so decl-site hover and
//! phase 2's definition lookup are span-exact.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};

use super::bindings::ident_span;
use super::infer;
use super::types::{ty_head, ty_src, DefIndex, FnDef, ImplDef, LetDef, MemberSrc, TyDef, TyForm, UseDef};

/// `line` of a byte offset, 1-based
fn line_of(src: &str, lo: u32) -> u32 {
    1 + src[..lo as usize].matches('\n').count() as u32
}

/// doc comment lines directly above `lo`: consecutive `//` lines with no
/// blank line between them and the decl (comments are not tokens — the
/// lexer skips them, so this scans the source itself)
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

fn member(
    src: &str,
    toks: &[Token],
    name: &str,
    sig: String,
    ty: Option<String>,
    span: Span,
) -> MemberSrc {
    MemberSrc {
        name: name.to_string(),
        src: sig,
        ty,
        name_span: ident_span(toks, span, name),
        doc: doc_before(src, span.lo),
        line: line_of(src, span.lo),
    }
}

fn members_of(src: &str, ast: &Ast, toks: &[Token], methods: &[NodeHandle<MethodDeclNode>]) -> Vec<MemberSrc> {
    methods
        .iter()
        .map(|m| {
            let d = ast.method_decl(*m);
            let sp = ast.span(m.id());
            // `pub(..)`/`async` sit before `fn` —outside the decl span;
            // restore them from the flags (truthful to intent)
            let mut pre = String::new();
            if let Some(v) = d.vis {
                pre.push_str(&member_vis_str(v));
                pre.push(' ');
            }
            if d.is_async {
                pre.push_str("async ");
            }
            let sig = format!("{pre}{}", sig_src(src, ast, sp, d.body));
            let name = ast.name(d.name);
            member(src, toks, name, sig, d.ret.map(|r| ty_src(ast, r)), sp)
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
/// span starts before them) —strip, then re-render from the flags
fn field_members(src: &str, ast: &Ast, toks: &[Token], fields: &[NodeHandle<FieldDeclNode>]) -> Vec<MemberSrc> {
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
            let name = ast.name(d.name);
            member(src, toks, name, decl, Some(ty_src(ast, d.ty)), sp)
        })
        .collect()
}

/// enum member list with token-recovered name spans — the recovery
/// build.rs's old comment said this needed. The member idents are the
/// non-keyword Ident run inside the decl's braces; generic parameters
/// (`enum E<T>`) interleave but never match an expected member name,
/// so the ordered match skips them honestly (a miss leaves `None`)
fn enum_members(
    src: &str,
    ast: &Ast,
    toks: &[Token],
    span: Span,
    members: &[(IdentId, Option<i64>)],
) -> Vec<MemberSrc> {
    let expected: Vec<&str> = members.iter().map(|(m, _)| ast.name(*m)).collect();
    let mut spans: Vec<Option<Span>> = vec![None; expected.len()];
    let mut k = 0usize;
    for t in toks {
        if t.span.lo < span.lo || t.span.hi > span.hi {
            continue;
        }
        let Tok::Ident(text) = &t.tok else { continue };
        if rut_parser::is_reserved_kw(text) {
            continue;
        }
        if k < expected.len() && text == expected[k] {
            spans[k] = Some(t.span);
            k += 1;
        }
    }
    expected
        .iter()
        .zip(spans)
        .map(|(name, name_span)| MemberSrc {
            name: name.to_string(),
            src: name.to_string(),
            ty: None,
            name_span,
            doc: Vec::new(),
            line: line_of(src, span.lo),
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
        alias_target: None,
        doc: doc_before(src, span.lo),
        span,
        line: line_of(src, span.lo),
    }
}

fn generics_of(ast: &Ast, gs: &[IdentId]) -> Vec<String> {
    gs.iter().map(|&g| ast.name(g).to_string()).collect()
}

/// Build the index for one document. `toks` is the same lex the parser
/// consumed — the decl layer's name-span recovery rides it (no re-lex,
/// no parser changes).
pub fn index(src: &str, ast: &Ast, toks: &[Token]) -> DefIndex {
    let mut idx = DefIndex::default();
    let mut pending_lets: Vec<(IdentId, Option<NodeHandle<AnyTy>>, NodeHandle<AnyExpr>, Span)> = Vec::new();
    for h in ast.module_items(ast.root) {
        let span = ast.span(h.id());
        match ast.item(*h) {
            ItemKind::Class { name, generics, fields, methods, .. } => {
                let fs = field_members(src, ast, toks, fields);
                let ms = members_of(src, ast, toks, methods);
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
                let fs = field_members(src, ast, toks, fields);
                let ms = members_of(src, ast, toks, methods);
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
                let ms = members_of(src, ast, toks, methods);
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
                // member spans recovered by token scan inside the enum's
                // span (the recovery the old comment deferred)
                let ms = enum_members(src, ast, toks, span, members);
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
                let trait_head = trait_ref
                    .map(|tr| ty_head(ast, tr))
                    .unwrap_or_default();
                let owner = if trait_ref.is_some() {
                    format!("impl {} for {}", trait_head, ty_head(ast, *target))
                } else {
                    format!("impl {}", ty_head(ast, *target))
                };
                idx.impls.push(ImplDef {
                    trait_name: trait_head,
                    target_name: ty_head(ast, *target),
                });
                for m in methods {
                    let d = ast.method_decl(*m);
                    let sp = ast.span(m.id());
                    idx.fns.push(FnDef {
                        name: ast.name(d.name).to_string(),
                        src: sig_src(src, ast, sp, d.body),
                        ret: d.ret.map(|r| ty_src(ast, r)),
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
                    ret: d.ret.map(|r| ty_src(ast, r)),
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
                    ret: None,
                    doc: doc_before(src, span.lo),
                    owner: None,
                    span,
                    line: line_of(src, span.lo),
                });
            }
            ItemKind::BuiltinTy { name, generics, members, .. } => {
                let ms = members_of(src, ast, toks, members);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::Builtin,
                    generics_of(ast, generics),
                    Vec::new(),
                    ms,
                    span,
                ));
            }
            ItemKind::BuiltinPrimitive { name, members } => {
                let ms = members_of(src, ast, toks, members);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::Primitive,
                    Vec::new(),
                    Vec::new(),
                    ms,
                    span,
                ));
            }
            ItemKind::SurfaceDataclass { name, fields, .. } => {
                let fs = field_members(src, ast, toks, fields);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::HostDataclass,
                    Vec::new(),
                    fs,
                    Vec::new(),
                    span,
                ));
            }
            ItemKind::BuiltinTrait { name, generics, methods, .. } => {
                let ms = members_of(src, ast, toks, methods);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::BuiltinTrait,
                    generics_of(ast, generics),
                    Vec::new(),
                    ms,
                    span,
                ));
            }
            ItemKind::BuiltinImpl { .. } => {
                // core's `builtin impl i32 { .. }` — no new type; the
                // methods surface through core's own hover data
            }
            ItemKind::Alias(d) => {
                // `type X = A;` / `type X = A | B;` (RFC 0043) — the
                // target renders as written
                idx.types.push(TyDef {
                    name: ast.name(d.name).to_string(),
                    form: TyForm::Alias,
                    generics: Vec::new(),
                    fields: Vec::new(),
                    methods: Vec::new(),
                    doc: doc_before(src, span.lo),
                    span,
                    line: line_of(src, span.lo),
                    alias_target: Some(ty_src(ast, d.target)),
                });
            }
            ItemKind::ModuleLet { name, ty, init, .. } => {
                // processed after the main walk: the initializer's
                // inference may name THIS document's types
                pending_lets.push((*name, *ty, *init, span));
            }
            ItemKind::Use { pkg, names } => {
                // `use pouch::{ Vec, Vec2 };` / `use pouch::Vec;` — the
                // pkg ident first, then the imported names in order
                let pkg_text = ast.name(*pkg).to_string();
                let expected: Vec<&str> = names.iter().map(|n| ast.name(*n)).collect();
                let mut spans: Vec<Option<Span>> = vec![None; expected.len()];
                let (mut seen_pkg, mut k) = (false, 0usize);
                for t in toks {
                    if t.span.lo < span.lo || t.span.hi > span.hi {
                        continue;
                    }
                    let Tok::Ident(text) = &t.tok else { continue };
                    if rut_parser::is_reserved_kw(text) {
                        continue;
                    }
                    if !seen_pkg {
                        seen_pkg = *text == pkg_text;
                        continue;
                    }
                    if k < expected.len() && text == expected[k] {
                        spans[k] = Some(t.span);
                        k += 1;
                    }
                }
                for (name, name_span) in expected.into_iter().zip(spans) {
                    idx.uses.push(UseDef {
                        name: name.to_string(),
                        name_span,
                        pkg: pkg_text.clone(),
                    });
                }
            }
            ItemKind::Module { .. } => {}
        }
    }
    // module lets — with the index's types/fns in place so a
    // `let c = Circle.new(..)` at module scope infers its type
    let lets = std::mem::take(&mut pending_lets);
    for (name, ty, init, span) in lets {
        let text = ast.name(name).to_string();
        let ty_text = ty.map(|t| ty_src(ast, t)).or_else(|| {
            let done: [&DefIndex; 1] = [&idx];
            infer::expr_ty(ast, &[], &done, init)
        });
        // statement spans end at the next token (the parser's
        // convention) — cut the verbatim slice at the decl's own `;`
        let hi = toks
            .iter()
            .find(|t| t.tok == Tok::Semi && t.span.lo >= span.lo && t.span.hi <= span.hi)
            .map(|t| t.span.hi)
            .unwrap_or(span.hi);
        idx.lets.push(LetDef {
            name: text,
            name_span: ident_span(toks, span, ast.name(name)),
            ty: ty_text,
            src: src[span.lo as usize..hi as usize]
                .trim_end()
                .to_string(),
            doc: doc_before(src, span.lo),
            span,
            line: line_of(src, span.lo),
        });
    }
    idx
}
