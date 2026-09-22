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
    /// `builtin primitive <name>` — a boot primitive's surface statement
    /// (`str`/`bytes`/`opaque`); members are compiler-lowered
    Primitive,
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
            TyForm::Primitive => "primitive",
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
    /// the declared type as written — fields: the type; methods: the
    /// return (`None` = unwritten, i.e. `nil`-returning). Inferred-type
    /// hovers key on this instead of re-parsing `src`
    pub ty: Option<String>,
    /// byte span of the name identifier — token recovery (names are
    /// `IdentId`s: the AST carries no name spans, no parser changes).
    /// The field / enum-member decl-site hover and phase 2's
    /// definition lookup key on it; `None` only when recovery fails
    /// (a degenerate parse)
    pub name_span: Option<Span>,
    pub doc: Vec<String>,
    /// 1-based line of the declaration
    pub line: u32,
}

/// a module-scope `let` — a real definition (the survey §3.2 decl
/// layer: `ModuleLet` and `Use` were skipped by the old index)
#[derive(Debug, Clone)]
pub struct LetDef {
    pub name: String,
    pub name_span: Option<Span>,
    /// declared annotation as written, else the initializer-derived
    /// type (the same display-side heuristic the binding pass uses)
    pub ty: Option<String>,
    /// verbatim decl slice, through the initializer
    pub src: String,
    pub doc: Vec<String>,
    pub span: Span,
    pub line: u32,
}

/// a `use pkg::{ Name, .. }` import edge — the name's span is a
/// definition target (phase 2: ctrl+click on `Vec` in the use jumps
/// to the pouch decl)
#[derive(Debug, Clone)]
pub struct UseDef {
    pub name: String,
    pub name_span: Option<Span>,
    pub pkg: String,
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
    /// the declared return as written (`None` = unwritten);
    /// inferred-type hovers read it for call initializers
    pub ret: Option<String>,
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
    /// module-scope lets (the decl layer)
    pub lets: Vec<LetDef>,
    /// use-imported names (the decl layer)
    pub uses: Vec<UseDef>,
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
        // `?Circle` binds like `Circle` for member lookup (RFC 0044) —
        // without this arm a nullable let-binding infers "" and its
        // receiver hover misses
        TypeKind::TyOpt { inner } => ty_head(ast, *inner),
        _ => String::new(),
    }
}

/// the lookup head of a rendered type text — what member lookup keys
/// on: `?Circle` → `Circle` (the RFC 0044 bind-like law), `Vec<i32>` →
/// `Vec`. Display keeps the full text; only lookups cut it down
pub(crate) fn head_of_ty(text: &str) -> String {
    let t = text.trim_start_matches('?');
    match t.find('<') {
        Some(i) if t.ends_with('>') => t[..i].to_string(),
        _ => t.to_string(),
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
        // the prefix spelling renders with its payload: `?i32`, `??str`
        // (matches symbols.rs's `ty_text` — the memo all three renderers
        // must carry)
        TypeKind::TyOpt { inner } => format!("?{}", ty_src(ast, *inner)),
        TypeKind::TyArray { elem } => format!("[{}]", ty_src(ast, *elem)),
        TypeKind::TyUnion { elems } => elems
            .iter()
            .map(|&e| ty_src(ast, e))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => String::new(),
    }
}
