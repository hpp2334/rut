//! The decl layer's shared vocabulary — the mount stamp, the type
//! forms, and the type-text helpers the views and the queries join
//! with. The row types (`TyDef`/`FnDef`/`LetDef`/`UseDef`/`ModDef`/
//! `MemberSrc`) are GONE: every decl fact derives from a view over
//! `(Doc, AnyHandle)` at read (see [`super::collection`]) — verbatim
//! slices are span×src joins on the owning file, and the
//! `ambient`/`is_pub`/`static` verdicts ride the modifier chains.

use crate::semantic::Mode;
use rut_ast::{Ast, TyRef};

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

/// Where one mounted file sits in the session's world — the lanes'
/// identity stamps, ONE record built in ONE expression at the mount
/// site and handed up front (identity is never patched on after the
/// fact). A plain record: many construction sites, no invariant.
#[derive(Debug, Default, Clone)]
pub struct Mount {
    /// label for provenance lines — the file's path, or `core`; the
    /// defs store's replace-by-origin key
    pub origin: String,
    /// the true repo-relative path of an EMBEDDED source (the
    /// prelude's `include_str!` origin: `rut/core/src/core.d.rut`) — the
    /// definition layer jumps there instead of the provenance label,
    /// so a workspace that is the rut repo lands in the real file
    pub src_path: Option<String>,
    /// the module's explicit manifest name (`pouch` — what a `use`
    /// path spells). Named files answer the use graph FIRST: a
    /// `Some` here is the whole match; `None` (open documents) falls
    /// back to the origin path-segment / file-stem derivation,
    /// byte-for-byte the old behavior
    pub module: Option<String>,
    /// which of the package's file modules THIS file is — the full
    /// mod path (`""` = the pkg root's `mod.rut`, `"layout/grid"` =
    /// that child). The (module, mod_path) pair is the whole key the
    /// mod-aware queries walk; `None` is the flat single-file shape.
    /// Stamped by the faces that mount a package's mod tree (the dep
    /// walk, the bundle loader); never derived from text.
    pub mod_path: Option<String>,
    /// which parse mode this file was BUILT in — `Some(Decl)` marks a
    /// `.d.rut` surface (a dep's `entry.type`, a host pkg's whole
    /// file), `Some(Impl)` the body (`mod.rut`); `None` = unknown
    /// (open documents, any builder that does not stamp). The
    /// definition layer prefers body hits over decl-surface hits when
    /// a package indexes both — never derived from text.
    pub mode: Option<Mode>,
}

impl Mount {
    /// an open document — origin only; mode stays `None` (the doc
    /// lanes' exact shape)
    pub fn doc(uri: &str) -> Mount {
        Mount {
            origin: uri.to_string(),
            ..Mount::default()
        }
    }

    /// a mounted dep file — the flat shape (`mod_path` `None`) or one
    /// file module of a mod-carrying pkg (`Some(path)`, `""` = the
    /// pkg root); the parse mode derives from the uri's filename
    /// convention
    pub fn dep(name: &str, uri: &str, mod_path: Option<&str>) -> Mount {
        Mount {
            origin: uri.to_string(),
            module: Some(name.to_string()),
            mod_path: mod_path.map(str::to_string),
            mode: Some(crate::semantic::mode_of(uri)),
            ..Mount::default()
        }
    }
}

/// head segment text of a type node (`Circle`, `Vec` in `Vec<T>`)
pub(crate) fn ty_head(ast: &Ast, t: TyRef) -> String {
    match t {
        TyRef::Path(h) => h
            .get(ast)
            .segs
            .first()
            .map(|s| ast.name(s.name.ident).to_string())
            .unwrap_or_default(),
        // `?Circle` binds like `Circle` for member lookup —
        // without this arm a nullable let-binding infers "" and its
        // receiver hover misses
        TyRef::Opt(h) => ty_head(ast, h.get(ast).inner),
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

/// display text of a type node — the alias hover's `type X = <target>`;
/// path segments join with `::` (the name-resolution law's spelling —
/// a `.` would read value access)
pub(crate) fn ty_src(ast: &Ast, t: TyRef) -> String {
    match t {
        TyRef::Path(h) => {
            let d = h.get(ast);
            let mut s = d
                .segs
                .iter()
                .map(|sg| ast.name(sg.name.ident).to_string())
                .collect::<Vec<_>>()
                .join("::");
            if let Some(last) = d.segs.last() {
                if !last.generics.is_empty() {
                    s.push_str(&format!(
                        "<{}>",
                        last.generics
                            .iter()
                            .map(|&g| ty_src(ast, g))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            s
        }
        // the prefix spelling renders with its payload: `?i32`, `??str`
        // (matches symbols.rs's `ty_text` — the memo all three renderers
        // must carry)
        TyRef::Opt(h) => format!("?{}", ty_src(ast, h.get(ast).inner)),
        TyRef::Array(h) => format!("[{}]", ty_src(ast, h.get(ast).elem)),
        // a written tuple annotation renders as written — `-> (opaque,
        // str)` used to render empty, which leaked `: `-style hints and
        // hovers (the inlay phase's find)
        TyRef::Tuple(h) => format!(
            "({})",
            h.get(ast)
                .elems
                .iter()
                .map(|&e| ty_src(ast, e))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TyRef::Union(h) => h
            .get(ast)
            .elems
            .iter()
            .map(|&e| ty_src(ast, e))
            .collect::<Vec<_>>()
            .join(" | "),
        // the enclosing type's own spelling (mirrors symbols.rs's
        // `ty_text` — the memo all three renderers carry): a residual
        // render shows the written word, never silent empty. Member
        // lookups key on it and miss — no type is NAMED `Self`
        TyRef::Self_(_) => "Self".to_string(),
        _ => String::new(),
    }
}
