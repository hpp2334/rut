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
    binding_ty, constructor_of, enclosing_type, expr_ty, iface_decl, impl_method_fn, member,
    member_at, member_dial_allows, named_recv, recv_ty_name, used_ifaces, MemberHit, MemberTarget,
    RecvNode,
};
pub(crate) use tiers::{
    ambiguous_scope, bare, bound_has, file_scope, functions, module_let, one_name_refused,
    qualified_module, qualified_scope, type_decl, type_via_imports, use_bound_scope, use_exports,
    CandRow, Candidate, ExportRow, FileScope, MemberRow, UseExports, Via,
};
pub(crate) use tiers::{members, members_for, prefer_mounted, surface_names};
pub(crate) use views::{FnView, GlobUse, LetView, MemberView, ModView, TyView, UseView};

use crate::semantic::Mode;
use crate::world::File;

/// body-over-decl — THE single preference: a package that indexes
/// BOTH its `.d.rut` surface (`mode == Decl`) and its `mod.rut` body
/// answers from the body; the surface hits ride only when they are all
/// there is. One precedence above the body law: a MOUNTED dep outranks
/// the ambient prelude — the prelude is the zero-fetch fast path for
/// the ONE embedded pkg, a fallback, never the answer when the real
/// thing is mounted (its `src_path` marks it). The generic payload
/// keeps one law for every shape the layers collect (spans, views).
pub(crate) fn prefer_bodies<'a, T: Copy>(hits: &[(&'a File, T)]) -> Vec<(&'a File, T)> {
    let mounted: Vec<(&File, T)> = hits
        .iter()
        .copied()
        .filter(|(f, _)| f.stamp.src_path.is_none())
        .collect();
    let pool: &[(&File, T)] = if mounted.is_empty() { hits } else { &mounted };
    let bodies: Vec<(&File, T)> = pool
        .iter()
        .copied()
        .filter(|(f, _)| f.stamp.mode != Mode::Decl)
        .collect();
    if bodies.is_empty() {
        pool.to_vec()
    } else {
        bodies
    }
}
