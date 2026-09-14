//! Building the index — one pass over a document's module items,
//! recovering verbatim signatures and doc comments from the source.

use rut_ast::ast::*;
use rut_lexer::span::Span;

use super::types::{ty_head, DefIndex, FnDef, ImplDef, MemberSrc, TyDef, TyForm};

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
            ItemKind::BuiltinTy { name, generics, members, .. } => {
                let ms = members_of(src, ast, members);
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
            ItemKind::SurfaceDataclass { name, fields, .. } => {
                let fs = field_members(src, ast, fields);
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
            ItemKind::BuiltinIface { name, generics, methods, .. } => {
                let ms = members_of(src, ast, methods);
                idx.types.push(ty_def(
                    src,
                    ast,
                    ast.name(*name),
                    TyForm::BuiltinIface,
                    generics_of(ast, generics),
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
