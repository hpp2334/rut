//! Document symbols — the outline tree, lsp-free so it stays testable
//! without a protocol dependency.

use rut_ast::{
    Ast, EnumMemberNode, FieldDeclNode, FieldRef, ItemRef, MethodDeclNode, MethodRef, TyRef,
};
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
    /// the name's own span (the node's recorded `name_span`); falls
    /// back to `range`
    pub selection: Span,
    pub children: Vec<RawSymbol>,
}

pub fn symbols(toks: &[Token], ast: &Ast) -> Vec<RawSymbol> {
    ast.root()
        .items(ast)
        .iter()
        .filter_map(|h| item_symbol(toks, ast, *h))
        .collect()
}

fn item_symbol(toks: &[Token], ast: &Ast, item: ItemRef) -> Option<RawSymbol> {
    // the walk surface's own span read (modifiers excluded where the
    // node's own span excludes them — fn/method spans start at `fn`)
    let span = item.span(ast);
    let sym = |name: &str, kind, selection: Option<Span>, children| RawSymbol {
        name: name.to_string(),
        kind,
        range: span,
        selection: selection.unwrap_or(span),
        children,
    };
    match item {
        ItemRef::Fn(h) => {
            let d = h.get(ast);
            Some(sym(
                ast.name(d.name),
                SymKind::Function,
                Some(d.name_span),
                vec![],
            ))
        }
        // `use pkg::A::B::{C, D};` — an import is a document symbol: a
        // module-shaped outline entry named for the package (path[0]).
        // Matters for multi-lib bases: a base file can be the
        // law header + the use set alone, and an outline that ignored
        // imports would show it empty. The FULL path rides the item
        // for classification.
        ItemRef::Use(h) => {
            let d = h.get(ast);
            Some(sym(
                ast.name(d.path[0]),
                SymKind::Module,
                Some(d.name_span),
                vec![],
            ))
        }
        // `pub? mod NAME;` — a file-module declaration: a namespace
        // outline entry (the closest existing kind; the body lives in
        // the mounted file)
        ItemRef::ModDecl(h) => {
            let d = h.get(ast);
            Some(sym(
                ast.name(d.name),
                SymKind::Module,
                Some(d.name_span),
                vec![],
            ))
        }
        // the declaration file's inline module — the same outline
        // entry, its nested declarations riding as children
        ItemRef::ModInline(h) => {
            let d = h.get(ast);
            let children = h
                .items(ast)
                .iter()
                .filter_map(|&i| item_symbol(toks, ast, i))
                .collect();
            Some(sym(
                ast.name(d.name),
                SymKind::Module,
                Some(d.name_span),
                children,
            ))
        }
        ItemRef::Alias(h) => {
            let d = h.get(ast);
            Some(sym(
                ast.name(d.name),
                SymKind::Class,
                Some(d.name_span),
                vec![],
            ))
        }
        ItemRef::ModuleLet(h) => {
            let d = h.get(ast);
            Some(sym(
                ast.name(d.name),
                SymKind::Variable,
                Some(d.name_span),
                vec![],
            ))
        }
        ItemRef::Enum(h) => {
            let d = h.get(ast);
            let children = d
                .members
                .iter()
                .filter_map(|m| member_symbol(ast, span, m))
                .collect();
            Some(sym(
                ast.name(d.name),
                SymKind::Enum,
                Some(d.name_span),
                children,
            ))
        }
        ItemRef::Struct(h) => {
            let d = h.get(ast);
            let children = member_symbols(toks, ast, &d.fields, &d.methods);
            Some(sym(
                ast.name(d.name),
                SymKind::Class,
                Some(d.name_span),
                children,
            ))
        }
        ItemRef::Class(h) => {
            let d = h.get(ast);
            let children = member_symbols(toks, ast, &d.fields, &d.methods);
            Some(sym(
                ast.name(d.name),
                SymKind::Class,
                Some(d.name_span),
                children,
            ))
        }
        ItemRef::Interface(h) => {
            let d = h.get(ast);
            let children = method_symbols(ast, &d.methods);
            Some(sym(
                ast.name(d.name),
                SymKind::Interface,
                Some(d.name_span),
                children,
            ))
        }
        ItemRef::Impl(h) => {
            let d = h.get(ast);
            let children = method_symbols(ast, &d.methods);
            let gen_head = if d.generics.is_empty() {
                String::new()
            } else {
                let gs = d
                    .generics
                    .iter()
                    .map(|&g| ast.name(g).to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("<{}>", gs)
            };
            let name = format!("impl{} {}", gen_head, ty_text(ast, d.target));
            Some(sym(&name, SymKind::Module, None, children))
        }
        ItemRef::SurfaceFn(h) => {
            let d = h.get(ast);
            Some(sym(
                ast.name(d.name),
                SymKind::Function,
                Some(d.name_span),
                vec![],
            ))
        }
        ItemRef::BuiltinTy(h) => {
            let d = h.get(ast);
            let children = method_symbols(ast, &d.members);
            Some(sym(
                ast.name(d.name),
                SymKind::Class,
                Some(d.name_span),
                children,
            ))
        }
        ItemRef::BuiltinPrimitive(h) => {
            let d = h.get(ast);
            let children = method_symbols(ast, &d.members);
            Some(sym(
                ast.name(d.name),
                SymKind::Class,
                Some(d.name_span),
                children,
            ))
        }
        ItemRef::BuiltinImpl(h) => {
            // core's `builtin impl i32 { .. }` — a method group on a
            // primitive; symbolized as `builtin impl <prim>`
            let d = h.get(ast);
            let children = method_symbols(ast, &d.methods);
            Some(sym(
                ast.name(d.prim),
                SymKind::Class,
                Some(d.name_span),
                children,
            ))
        }
        ItemRef::SurfaceStruct(h) => {
            let d = h.get(ast);
            let children = field_symbols(toks, ast, &d.fields);
            Some(sym(
                ast.name(d.name),
                SymKind::Class,
                Some(d.name_span),
                children,
            ))
        }
        // an Error slot renders no outline entry (errors are the panel's
        // business, not the outline's); the root module is walked from
        // `symbols` and never rides an items list
        ItemRef::Error(_) | ItemRef::Module(_) => None,
    }
}

/// one enum member's outline entry — range stays the enum's own span
/// (the member is a line inside it), selection the member's name ident.
/// A member Error slot renders no entry (errors are the panel's
/// business, not the outline's)
fn member_symbol(ast: &Ast, enum_span: Span, m: &rut_ast::MemberRef) -> Option<RawSymbol> {
    let d: &EnumMemberNode = match m {
        rut_ast::MemberRef::Member(h) => h.get(ast),
        rut_ast::MemberRef::Error(_) => return None,
    };
    Some(RawSymbol {
        name: ast.name(d.name).to_string(),
        kind: SymKind::EnumMember,
        range: enum_span,
        selection: d.name_span,
        children: vec![],
    })
}

fn member_symbols(
    toks: &[Token],
    ast: &Ast,
    fields: &[FieldRef],
    methods: &[MethodRef],
) -> Vec<RawSymbol> {
    let mut out = field_symbols(toks, ast, fields);
    out.extend(method_symbols(ast, methods));
    out
}

fn field_symbols(toks: &[Token], ast: &Ast, fields: &[FieldRef]) -> Vec<RawSymbol> {
    let mut out = Vec::new();
    for f in fields {
        let FieldRef::Field(h) = f else {
            continue; // the recovery slot renders no entry
        };
        let d: &FieldDeclNode = h.get(ast);
        out.push(RawSymbol {
            name: ast.name(d.name).to_string(),
            kind: SymKind::Field,
            range: d.span,
            // the field's span covers its modifiers (`pub static ..`);
            // the recorded name ident is the honest selection
            selection: find_name(toks, d.span, ast.name(d.name), false).unwrap_or(d.span),
            children: vec![],
        });
    }
    out
}

fn method_symbols(ast: &Ast, methods: &[MethodRef]) -> Vec<RawSymbol> {
    let mut out = Vec::new();
    for m in methods {
        let MethodRef::Method(h) = m else {
            continue;
        };
        let d: &MethodDeclNode = h.get(ast);
        out.push(RawSymbol {
            name: ast.name(d.name).to_string(),
            kind: SymKind::Method,
            range: d.span,
            selection: d.name_span,
            children: vec![],
        });
    }
    out
}

/// Best-effort type text for impl headers (`impl Circle`).
fn ty_text(ast: &Ast, t: TyRef) -> String {
    match t {
        TyRef::Path(h) => {
            let d = h.get(ast);
            let names: Vec<&str> = d.segs.iter().map(|s| ast.name(s.name)).collect();
            names.join(".")
        }
        TyRef::Fn(_) => "fn(..)".to_string(),
        // the prefix spelling renders with its payload: `?i32`, `??str`
        TyRef::Opt(h) => format!("?{}", ty_text(ast, h.get(ast).inner)),
        TyRef::Array(h) => format!("[{}]", ty_text(ast, h.get(ast).elem)),
        TyRef::Tuple(_) => "(..)".to_string(),
        TyRef::Const(_) => "const".to_string(),
        TyRef::Union(h) => h
            .get(ast)
            .elems
            .iter()
            .map(|&e| ty_text(ast, e))
            .collect::<Vec<_>>()
            .join(" | "),
        TyRef::Error(_) => String::new(),
    }
}
