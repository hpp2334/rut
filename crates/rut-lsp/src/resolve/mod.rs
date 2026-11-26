//! The resolver layer — the one classification of a caret
//! ([`cursor`]), the one walk from a caret to declarations
//! ([`walk::resolve`]) answering the typed [`Def`], the scope and
//! surface doors the features read through ([`scope`], [`exports`],
//! [`member`]), the package identity ([`pkg`]), the display-side
//! inference ([`ty_infer`]), and the adapters that carry the world's
//! stamped files into rut-lir's pure mod laws ([`mods`]). Every
//! language law has ONE home; the features above (hover, definition,
//! references) classify, resolve, and render — they never walk on
//! their own.

pub(crate) mod cursor;
pub(crate) mod def;
pub(crate) mod exports;
pub(crate) mod member;
pub(crate) mod mods;
pub(crate) mod pkg;
pub(crate) mod scope;
pub(crate) mod ty_infer;
pub(crate) mod walk;

pub(crate) use def::{Def, DefLocation, View};

// the doors' flat spellings — every consumer reads
// `crate::resolve::<door>`; the module files own the laws
pub(crate) use exports::{
    bare, members, members_for, use_exports, ExportRow, MemberRow, UseExports,
};
pub(crate) use member::{
    binding_ty, constructor_of, enclosing_type, expr_ty, iface_decl, impl_method_fn,
    impl_method_fn_in, member, member_dial_allows, member_gated, named_recv, qualified_ty_name,
    recv_ty, unique_free_fn, unique_free_fn_in, MemberHit, MemberTarget,
};
pub(crate) use scope::{
    ambiguous_scope, bound_has, file_scope, functions, module_let_ty, one_name_refused,
    prefer_bodies, prefer_mounted, qualified_module, qualified_scope, type_decl, type_via_imports,
    use_bound_scope, CandRow, Candidate, FileScope,
};
