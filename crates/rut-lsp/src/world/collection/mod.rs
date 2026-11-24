//! The world's resolver layer — the doors that answer the cross-file
//! questions over [`crate::world::World`]. The subtree: `member` (the
//! member law's resolver family — hover, definition, and completion
//! share it), `tiers` (completion's folded tier doors and THE
//! file-scope resolver), `views` (the typed views over
//! `(Doc, AnyHandle)` — a view's verbatim text is a span×src join on
//! the OWNING file's document, its line number is `doc.line_of`, its
//! `ambient`/vis verdict derives from the node's modifier chain —
//! nothing stored, nothing stale).
//!
//! The doors are free functions over `(w: &World, file: FileId, ..)`
//! with one home each: the FILE SCOPE is [`file_scope`] (tiers) — what
//! the queried file binds by one name, own decls ∪ use-bound ∪
//! ambient, the law the one-name Act 2 spelled (sibling modules may
//! share a name; a name the file never binds answers nothing). The
//! three flat doors re-spell that one home: [`type_decl`] the file
//! scope's first type, [`functions`] the 0/1/many honesty rule,
//! [`module_let`] the unique-or-none rule. The member law lives in
//! `member`, completion's tiers in `tiers`, and the body-over-decl
//! preference once in [`prefer_bodies`].

mod member;
mod tiers;
mod views;

pub(crate) use member::{
    binding_ty, constructor_of, enclosing_type, expr_ty, iface_decl, impl_method_fn,
    impl_method_fn_in, member, member_dial_allows, member_gated, named_recv, qualified_ty_name,
    recv_ty, unique_free_fn, unique_free_fn_in, used_ifaces, MemberHit, MemberTarget,
};
pub(crate) use tiers::{
    ambiguous_scope, bare, bound_has, file_scope, functions, one_name_refused, qualified_module,
    qualified_scope, type_decl, type_via_imports, use_bound_scope, use_exports, CandRow, Candidate,
    ExportRow, FileScope, MemberRow, UseExports, Via,
};
pub(crate) use tiers::{members, members_for, surface_names};
pub(crate) use views::{FnView, GlobUse, LetView, MemberView, ModView, TyView, UseView};

use crate::semantic::Mode;
use crate::world::File;

/// the ambient layer's mounted-over-embed pick: the MOUNTED carriers
/// (real origins) when any exist, else the embedded prelude — the
/// zero-fetch fallback (the old flat `type_decl`'s precedence, kept
/// as the ambient fallback shape). THE one home of the preference;
/// [`prefer_bodies`] stages it first (a mounted carrier outranks the
/// prelude, above the body law), and the inference pass's ctor lane
/// rides it over its own window.
pub(crate) fn prefer_mounted<'a, T: Copy>(pairs: &[(&'a File, T)]) -> Vec<(&'a File, T)> {
    let mounted: Vec<(&File, T)> = pairs
        .iter()
        .copied()
        .filter(|(f, _)| f.stamp.src_path.is_none())
        .collect();
    if mounted.is_empty() {
        pairs.to_vec()
    } else {
        mounted
    }
}

/// body-over-decl — THE single preference: a package that indexes
/// BOTH its `.d.rut` surface (`mode == Decl`) and its `mod.rut` body
/// answers from the body; the surface hits ride only when they are all
/// there is. One precedence above the body law: a MOUNTED dep outranks
/// the ambient prelude — the prelude is the zero-fetch fast path for
/// the ONE embedded pkg, a fallback, never the answer when the real
/// thing is mounted (its `src_path` marks it) — staged first through
/// [`prefer_mounted`]. The generic payload keeps one law for every
/// shape the layers collect (spans, views).
pub(crate) fn prefer_bodies<'a, T: Copy>(hits: &[(&'a File, T)]) -> Vec<(&'a File, T)> {
    let pool = prefer_mounted(hits);
    let bodies: Vec<(&File, T)> = pool
        .iter()
        .copied()
        .filter(|(f, _)| f.stamp.mode != Mode::Decl)
        .collect();
    if bodies.is_empty() {
        pool
    } else {
        bodies
    }
}
