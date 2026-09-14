//! Document symbols — the outline tree, lsp-free so it stays testable
//! without a protocol dependency.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::Token;

use super::recover::find_name;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymKind {
    Function,
    Variable,
    Enum,
    EnumMember,
    Class,
    Field,
    Method,
    Interface,
    Module,
}

/// An outline entry — lsp-free so the classifier stays testable without a
/// protocol dependency.
#[derive(Clone, Debug)]
pub struct RawSymbol {
    pub name: String,
    pub kind: SymKind,
    pub range: Span,
    /// the name's own span (recovered); falls back to `range`
    pub selection: Span,
    pub children: Vec<RawSymbol>,
}

pub fn symbols(toks: &[Token], ast: &Ast) -> Vec<RawSymbol> {
    ast.module_items(ast.root)
        .iter()
        .filter_map(|h| item_symbol(toks, ast, *h))
        .collect()
}

fn item_symbol(toks: &[Token], ast: &Ast, h: NodeHandle<AnyItem>) -> Option<RawSymbol> {
    let span = ast.span(h.id());
    let sym = |name: &str, kind, selection: Option<Span>, children| RawSymbol {
        name: name.to_string(),
        kind,
        range: span,
        selection: selection.unwrap_or(span),
        children,
    };
    match ast.item(h) {
        ItemKind::Fn(d) => Some(sym(
            ast.name(d.name),
            SymKind::Function,
            find_name(toks, span, ast.name(d.name), false),
            vec![],
        )),
        ItemKind::ModuleLet { name, .. } => Some(sym(
            ast.name(*name),
            SymKind::Variable,
            find_name(toks, span, ast.name(*name), false),
            vec![],
        )),
        ItemKind::Enum { name, members, .. } => {
            let children = members
                .iter()
                .map(|(m, _)| {
                    let text = ast.name(*m);
                    sym(text, SymKind::EnumMember, find_name(toks, span, text, false), vec![])
                })
                .collect();
            Some(sym(ast.name(*name), SymKind::Enum, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::Dataclass { name, fields, methods, .. }
        | ItemKind::Class { name, fields, methods, .. } => {
            let children = member_symbols(toks, ast, fields, methods);
            Some(sym(ast.name(*name), SymKind::Class, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::Trait { name, methods, .. } => {
            let children = methods
                .iter()
                .map(|m| method_symbol(toks, ast, *m))
                .collect();
            Some(sym(ast.name(*name), SymKind::Interface, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::Impl { trait_ref, target, methods, .. } => {
            let children = methods
                .iter()
                .map(|m| method_symbol(toks, ast, *m))
                .collect();
            let name = format!("impl {} for {}", ty_text(ast, *trait_ref), ty_text(ast, *target));
            Some(sym(&name, SymKind::Module, None, children))
        }
        ItemKind::SurfaceFn { name, .. } => Some(sym(
            ast.name(*name),
            SymKind::Function,
            find_name(toks, span, ast.name(*name), false),
            vec![],
        )),
        ItemKind::BuiltinTy { name, members, .. } => {
            let children = members.iter().map(|m| method_symbol(toks, ast, *m)).collect();
            Some(sym(ast.name(*name), SymKind::Class, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::BuiltinIface { name, methods, .. } => {
            let children = methods
                .iter()
                .map(|m| method_symbol(toks, ast, *m))
                .collect();
            Some(sym(ast.name(*name), SymKind::Interface, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::SurfaceDataclass { name, fields, .. } => {
            let children = member_symbols(toks, ast, fields, &[]);
            Some(sym(ast.name(*name), SymKind::Class, find_name(toks, span, ast.name(*name), false), children))
        }
        ItemKind::Import { .. } | ItemKind::Module { .. } => None,
    }
}

fn member_symbols(
    toks: &[Token],
    ast: &Ast,
    fields: &[NodeHandle<FieldDeclNode>],
    methods: &[NodeHandle<MethodDeclNode>],
) -> Vec<RawSymbol> {
    let mut out = Vec::new();
    for f in fields {
        let span = ast.span(f.id());
        let d = ast.field_decl(*f);
        out.push(RawSymbol {
            name: ast.name(d.name).to_string(),
            kind: SymKind::Field,
            range: span,
            selection: find_name(toks, span, ast.name(d.name), false).unwrap_or(span),
            children: vec![],
        });
    }
    for m in methods {
        out.push(method_symbol(toks, ast, *m));
    }
    out
}

fn method_symbol(toks: &[Token], ast: &Ast, m: NodeHandle<MethodDeclNode>) -> RawSymbol {
    let span = ast.span(m.id());
    let d = ast.method_decl(m);
    RawSymbol {
        name: ast.name(d.name).to_string(),
        kind: SymKind::Method,
        range: span,
        selection: find_name(toks, span, ast.name(d.name), false).unwrap_or(span),
        children: vec![],
    }
}

/// Best-effort type text for impl headers (`impl Drawable for Circle`).
fn ty_text(ast: &Ast, h: NodeHandle<AnyTy>) -> String {
    match ast.ty(h) {
        TypeKind::TyPath { segs, is_dyn } => {
            let names: Vec<&str> = segs.iter().map(|s| ast.name(s.name)).collect();
            if *is_dyn {
                names.join(".")
            } else {
                names.join(".")
            }
        }
        TypeKind::TyFn { .. } => "fn(..)".to_string(),
        TypeKind::TyPtr { .. } => "*T".to_string(),
        TypeKind::TyTuple { .. } => "(..)".to_string(),
        TypeKind::TyConst(_) => "const".to_string(),
    }
}
