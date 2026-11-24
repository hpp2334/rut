//! The decl layer's shared vocabulary — the type forms and the
//! type-text helpers the views and the queries join with. The row
//! types (`TyDef`/`FnDef`/`LetDef`/`UseDef`/`ModDef`/`MemberSrc`) are
//! GONE: every decl fact derives from a view over
//! `(Doc, AnyHandle)` at read (see [`super::decls`]) — verbatim
//! slices are span×src joins on the owning file, and the
//! `ambient`/`is_pub`/`static` verdicts ride the modifier chains.

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
