//! The index data — one entry per type, fn, or impl declaration:
//! verbatim signature slices, doc lines, and spans for rendering.

use rut_ast::ast::*;
use rut_lexer::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TyForm {
    Class,
    Dataclass,
    Trait,
    Enum,
    /// `builtin Name<..>` — an engine builtin's member contract (core
    /// only; members are compiler-lowered)
    Builtin,
    /// `builtin trait Name<..>` — an engine-woven contract (Index,
    /// Iterator, Disposal); users implement it with ordinary impl blocks
    BuiltinTrait,
    /// `host struct Name { fields }` — a flat host-constructed record
    HostDataclass,
    /// `type X = A;` — a transparent alias (RFC 0043)
    Alias,
}

impl TyForm {
    pub(crate) fn keyword(self) -> &'static str {
        match self {
            TyForm::Class => "class",
            TyForm::Dataclass => "struct",
            TyForm::Trait => "trait",
            TyForm::Enum => "enum",
            TyForm::Builtin => "builtin",
            TyForm::BuiltinTrait => "builtin trait",
            TyForm::HostDataclass => "host struct",
            TyForm::Alias => "type",
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
    /// `type X = A;` (RFC 0043) — the target as written; `None` otherwise
    pub alias_target: Option<String>,
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
    /// label for provenance lines — the file's path, or `core`
    pub origin: String,
}

impl DefIndex {
    pub fn ty(&self, name: &str) -> Option<&TyDef> {
        self.types.iter().find(|t| t.name == name)
    }
}

/// head segment text of a type node (`Circle`, `Vec` in `Vec<T>`)
pub(crate) fn ty_head(ast: &Ast, h: NodeHandle<AnyTy>) -> String {
    match ast.ty(h) {
        TypeKind::TyPath { segs, .. } => segs
            .first()
            .map(|s| ast.name(s.name).to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// display text of a type node — the alias hover's `type X = <target>`
pub(crate) fn ty_src(ast: &Ast, h: NodeHandle<AnyTy>) -> String {
    match ast.ty(h) {
        TypeKind::TyPath { segs } => {
            let mut s = segs
                .iter()
                .map(|sg| ast.name(sg.name).to_string())
                .collect::<Vec<_>>()
                .join(".");
            if let Some(last) = segs.last() {
                if !last.generics.is_empty() {
                    s.push_str(&format!(
                        "<{}>",
                        last.generics.iter().map(|&g| ty_src(ast, g)).collect::<Vec<_>>().join(", ")
                    ));
                }
            }
            s
        }
        TypeKind::TyUnion { elems } => elems
            .iter()
            .map(|&e| ty_src(ast, e))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => String::new(),
    }
}
