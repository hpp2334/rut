//! The index data — one entry per type, fn, or impl declaration:
//! verbatim signature slices, doc lines, and spans for rendering.

use rut_ast::ast::{AnyTy, Ast, NodeHandle, TypeKind, Vis};
use rut_lexer::span::Span;

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
    /// a dep manifest's `namespace` head (`calc`'s `Math`) — no surface
    /// decl spells it, the mint in `deps::index_dep` records it so the
    /// qualified shape (`Math.sqrt`, `Math.PI`) resolves like the
    /// engine's bound namespace
    Namespace,
}

impl TyForm {
    pub(crate) fn keyword(self) -> &'static str {
        match self {
            TyForm::Class => "class",
            TyForm::Struct => "struct",
            TyForm::Interface => "interface",
            TyForm::Enum => "enum",
            TyForm::Builtin => "builtin",
            TyForm::Primitive => "primitive",
            TyForm::HostStruct => "host struct",
            TyForm::Alias => "type",
            TyForm::Namespace => "namespace",
        }
    }
}

use crate::line_index::LineIndex;

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
    /// the declared visibility — the position-path completion's tier
    /// gate reads it (`pub(pkg)`/`pub(super)` are not `pub`)
    pub vis: Vis,
    pub span: Span,
    pub line: u32,
}

/// a `use pkg::A::B::{ Name, .. }` import edge — the name's span is a
/// definition target (phase 2: ctrl+click on `Vec` in the use jumps
/// to the pouch decl). The full path rides since the file-module phase:
/// `path[0]` is the package, the rest name modules — the use-path
/// completion walk and the segment hovers/definition targets key on
/// them (the flat spelling degenerates to `path = [pkg]`).
#[derive(Debug, Clone)]
pub struct UseDef {
    pub name: String,
    pub name_span: Option<Span>,
    pub pkg: String,
    /// the spelled path, pkg head included (`["pouch", "layout"]`)
    pub path: Vec<String>,
    /// token-recovered spans per path segment, same order — the use
    /// segment hover/definition targets (`None` when recovery failed)
    pub path_spans: Vec<Option<Span>>,
}

#[derive(Debug, Clone)]
pub struct TyDef {
    pub name: String,
    /// byte span of the name identifier — token recovery (names are
    /// `IdentId`s); phase 2's definition target (the declaring ident,
    /// not the whole decl)
    pub name_span: Option<Span>,
    pub form: TyForm,
    /// the declared visibility — the completion/hover tier gate reads
    /// it (`pub(pkg)`/`pub(super)` are not `pub`)
    pub vis: Vis,
    /// `pub` on the declaration — the std surface offers pub rows only
    /// (the hashmap-surface batch: nmapset's internal lane classes
    /// leave the bare-project surface)
    pub is_pub: bool,
    pub generics: Vec<String>,
    pub fields: Vec<MemberSrc>,
    pub methods: Vec<MemberSrc>,
    /// `type X = A;` — the target as written; `None` otherwise
    pub alias_target: Option<String>,
    pub doc: Vec<String>,
    pub span: Span,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    /// byte span of the name identifier — the definition target
    /// (recovery may fail on a degenerate parse; `span` is the fallback)
    pub name_span: Option<Span>,
    /// verbatim signature source
    pub src: String,
    /// the declared return as written (`None` = unwritten);
    /// inferred-type hovers read it for call initializers
    pub ret: Option<String>,
    pub params: Vec<String>,
    pub doc: Vec<String>,
    /// the declared visibility — the position-path completion's tier
    /// gate reads it (free fns; impl methods carry their member vis)
    pub vis: Vis,
    /// the owning inherent impl's target type (`Circle` — inherent
    /// impls own their methods, so an impl method renders like the
    /// type's own surface), `None` for a free fn
    pub owner: Option<String>,
    pub span: Span,
    pub line: u32,
}

/// an inherent impl block — the target type whose member surface it
/// extends (`impl Circle { .. }` records `Circle`; `impl I for T` is
/// gone, so there is no other half to record)
#[derive(Debug, Clone)]
pub struct ImplDef {
    pub target_name: String,
}

/// a `pub? mod NAME;` declaration — the file-module namespace edge.
/// The namespace itself is the mounted `NAME/mod.rut`'s own index
/// (found through the chain by its mod path); this row carries the
/// edge: the declaring ident span (hover/definition target), the
/// edge's visibility (`mod` vs `pub mod` — the crossing gates read
/// it), and the doc comment.
#[derive(Debug, Clone)]
pub struct ModDef {
    pub name: String,
    /// byte span of the name identifier — the decl-site hover /
    /// definition target (`None` only on a degenerate parse)
    pub name_span: Option<Span>,
    /// the edge's declared visibility (`Vis::Self_` = bare `mod`)
    pub vis: Vis,
    pub doc: Vec<String>,
    pub span: Span,
    pub line: u32,
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
    /// `pub? mod NAME;` declarations — the file-module namespace edges
    /// this file declares (the mounted child lives in its own index)
    pub mods: Vec<ModDef>,
    /// label for provenance lines — the file's path, or `core`
    pub origin: String,
    /// the normalized source this index was built over — cross-file
    /// definition targets convert their byte spans to LSP ranges
    /// against THIS text (the wasm face cannot re-read files)
    pub src: String,
    /// byte offsets ⇄ (line, UTF-16 col) over `src`
    pub lines: LineIndex,
    /// the true repo-relative path of an EMBEDDED source (the std
    /// surface's `include_str!` origin: `rut/pouch/mod.rut`) — the
    /// definition layer jumps there instead of the provenance label,
    /// so a workspace that is the rut repo lands in the real file
    pub src_path: Option<String>,
    /// names whose decl spelled the import-gated builtin linkage
    /// (`pub builtin` — the core disposal pair): completion offers them
    /// only when the document's `use` names them, the same ambient
    /// split the compiler binds by
    pub pub_gated: Vec<String>,
    /// the module's explicit manifest name (`pouch` — what a `use`
    /// path spells). Named indexes answer the use graph FIRST: a
    /// `Some` here is the whole match; `None` (today's workspace
    /// files) falls back to the origin path-segment / file-stem
    /// derivation, byte-for-byte the old behavior
    pub module: Option<String>,
    /// which of the package's file modules THIS index is — the full
    /// mod path (`""` = the pkg root's `mod.rut`, `"layout/grid"` =
    /// that child). The (module, mod_path) pair is the whole key the
    /// mod-aware queries walk: `module` alone stays the bare pkg name
    /// (the `matches_pkg` fallback laws ride unchanged), `None` here
    /// answers the root — today's single-file shapes — so flat
    /// packages resolve exactly as before. Stamped by the faces that
    /// mount a package's mod tree (the dep walk, the bundle loader);
    /// never derived from text.
    pub mod_path: Option<String>,
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
        // `?Circle` binds like `Circle` for member lookup —
        // without this arm a nullable let-binding infers "" and its
        // receiver hover misses
        TypeKind::TyOpt { inner } => ty_head(ast, *inner),
        _ => String::new(),
    }
}

/// the lookup head of a rendered type text — what member lookup keys
/// on: `?Circle` → `Circle` (the bind-like law), `Vec<i32>` →
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
        // a written tuple annotation renders as written — `-> (opaque,
        // str)` used to render empty, which leaked `: `-style hints and
        // hovers (the inlay phase's find)
        TypeKind::TyTuple { elems } => format!(
            "({})",
            elems.iter().map(|&e| ty_src(ast, e)).collect::<Vec<_>>().join(", ")
        ),
        TypeKind::TyUnion { elems } => elems
            .iter()
            .map(|&e| ty_src(ast, e))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => String::new(),
    }
}
