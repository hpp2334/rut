//! The qualified/annotation tiers — the type tier at an annotation
//! boundary, the `::` member lists whose head is a type, and the
//! position-path tier (the intra-package walk + the dep-module lane).

use std::collections::HashSet;

use super::rows::{push_item, CompletionOut};
use super::CompletionKind;
use crate::world::collection::Chain;
use crate::world::mods::{self, DocMods};
use crate::world::types::TyForm;

// ---- the qualified-position tier (intra-package) ----

/// The type tier at an annotation boundary — `let x: ⏐`, `id: u⏐`,
/// `fn f() -> ⏐`: the parser's primitive table (the ONE
/// `PRIMITIVE_TYS` — enumerated, never a duplicated list), the open
/// document's own types, the chain's AMBIENT builtin rows, and the
/// foreign types the document already `use`s. Keywords, fns, and
/// locals stay off: values never ride a type position.
pub(super) fn type_tier_completions(chain: &Chain) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for p in crate::semantic::PRIMITIVE_TYS {
        push_item(
            &mut out,
            CompletionOut::plain(
                (*p).to_string(),
                "primitive".to_string(),
                Vec::new(),
                CompletionKind::Type,
            ),
        );
    }
    let doc = chain.head();
    let imported = crate::world::pkg::import_map(doc);
    for f in chain.iter() {
        let own = std::ptr::eq(f, doc);
        let used_from: HashSet<&str> = imported
            .iter()
            .filter(|(_, pkg)| crate::world::pkg::matches_pkg(f, pkg))
            .map(|(name, _)| *name)
            .collect();
        for t in f.types() {
            // the same bare gate the decl tiers read: own types always,
            // foreign types when ambient or imported from this package
            if !own && !t.ambient() && !used_from.contains(t.name()) {
                continue;
            }
            push_item(
                &mut out,
                CompletionOut::plain(
                    t.name().to_string(),
                    t.head(),
                    t.doc_lines(),
                    match t.form() {
                        TyForm::Interface => CompletionKind::Interface,
                        _ => CompletionKind::Type,
                    },
                ),
            );
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// Member completions after `<TypeHead>::` — the `::` law's member
/// lists whose head is a type, reusing the index lookup hover's member
/// lane keys on (`find_ty`): an enum head offers its members, any
/// other type head its statics and associated fns (the inherent
/// `impl T { fn .. }` rows — the owner string IS the type's name). An
/// interface head offers nothing: its signatures ride receivers,
/// never `::`. `None` = the head resolves to no indexed type (the
/// caller falls through); a resolved head is FINAL — the filtered
/// list is the answer even when empty (the known-head law, like the
/// module tier's).
pub(super) fn type_head_completions(chain: &Chain, segs: &[String]) -> Option<Vec<CompletionOut>> {
    let (head, rest) = segs.split_first()?;
    let ty = chain.type_decl(head)?;
    let mut out: Vec<CompletionOut> = Vec::new();
    // a deeper walk under a type head descends nothing — types are not
    // namespaces (`Color::Red::` answers empty, final)
    if rest.is_empty() {
        match ty.form() {
            // `Color::⏐` — the declared members
            TyForm::Enum => {
                for m in ty.fields() {
                    push_item(
                        &mut out,
                        CompletionOut::plain(
                            m.name().to_string(),
                            m.src(),
                            m.doc_lines(),
                            CompletionKind::EnumMember,
                        ),
                    );
                }
            }
            // `Greeter::⏐` — interface members ride receivers
            TyForm::Interface => {}
            // `Counter::⏐` — statics first, then the associated fns
            _ => {
                for m in ty.fields() {
                    // a declared static — the verdict rides the node's
                    // own modifier chain, never a string match on the
                    // rendered slice
                    if m.is_static() {
                        push_item(
                            &mut out,
                            CompletionOut::plain(
                                m.name().to_string(),
                                m.src(),
                                m.doc_lines(),
                                CompletionKind::Field,
                            ),
                        );
                    }
                }
                for f in chain.iter() {
                    for fv in f.fns() {
                        if fv.owner().as_deref() == Some(head.as_str()) {
                            push_item(
                                &mut out,
                                CompletionOut::plain(
                                    fv.name().to_string(),
                                    fv.src(),
                                    fv.doc_lines(),
                                    CompletionKind::Fn,
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Some(out)
}

/// the leaf scope for a resolved mod path — the open document itself
/// when the path IS its own module (the pkg head resolving to the
/// root from the root), else the chain file stamped for it
fn scope_index<'a>(
    chain: &'a Chain<'a>,
    doc: &DocMods,
    pkg: Option<&str>,
    path: &str,
) -> Option<&'a crate::world::collection::MountedFile> {
    if doc.mod_path.as_deref() == Some(path) {
        return Some(chain.head());
    }
    mods::find_mod(chain, pkg, path)
}

/// A use of the DOC that bound `head` as a member of a NAMED dep
/// (`use calc::{ math }`): answers the dep's pkg name and the walked
/// mod path (the use's own path minus the pkg head — `math`, or
/// `math/grid` for a deep binding). The dep must be a mounted, named
/// index (a module-leaf binding the engine resolves through the
/// dep-module lane); a use that bound a plain name answers nothing
/// here — its module resolution never lands.
fn use_bound_dep_module(
    chain: &Chain,
    doc_file: &crate::world::collection::MountedFile,
    head: &str,
) -> Option<(String, String)> {
    for u in doc_file.uses() {
        if u.name() != head || u.path().is_empty() {
            continue;
        }
        // the dep must be mounted and named (a named file answers for it)
        if !chain
            .iter()
            .any(|f| f.mount.module.as_deref() == Some(u.pkg()))
        {
            continue;
        }
        // the walked module: a FLAT use's leaf IS the module
        // (`use calc::{ math }` → `math`); a qualified use's deeper
        // path spells it (`use calc::math::{ grid }` → `math/grid`
        // when the leaf itself names a module)
        let path = if u.path().len() == 1 {
            u.name().to_string()
        } else {
            u.path()[1..].join("/")
        };
        // the binding must land on a MODULE of the dep (a fn/const
        // leaf heads no path)
        if !mods::find_mod(chain, Some(u.pkg()), &path).is_some() {
            continue;
        }
        return Some((u.pkg().to_string(), path));
    }
    None
}

/// Position-path completions: after `modname::` in a package's own
/// file, the resolved module's members VISIBLE to the current file —
/// the phase-3 tier predicate (`private` reaches the declaring module
/// and descendants, `pub(super)` the parent's subtree, `pub(pkg)` the
/// whole package, `pub` everywhere) applied to every row. Declared
/// children of any edge vis ride as walkable segments (the
/// intra-package walk gates leaves, not heads). The dep-module lane
/// rides beside the own-package one: a use of this doc that bound a
/// module of a named dep (`use calc::{ math }`) heads the path into
/// the DEP's tree — the editor's mirror of the engine's
/// `math::sqrt(x)` / `math::PI` resolution. `None` = the head did not
/// resolve to a module (the caller falls through).
pub(super) fn position_path_completions(
    chain: &Chain,
    doc: &DocMods,
    segs: &[String],
) -> Option<Vec<CompletionOut>> {
    let (head, rest) = segs.split_first()?;
    let doc_file = chain.head();
    let (pkg, start) = match mods::resolve_position_head(doc, doc_file, chain, head) {
        Some(start) => (doc.pkg.clone().unwrap_or_default(), start),
        None => use_bound_dep_module(chain, doc_file, head)?,
    };
    let pkg = pkg.as_str();
    let path = mods::walk_position_mods(chain, Some(pkg), &start, rest)?;
    let from = doc.mod_path.as_deref().unwrap_or("");
    let Some(scope) = scope_index(chain, doc, Some(pkg), &path) else {
        return Some(Vec::new());
    };
    let mut out: Vec<CompletionOut> = Vec::new();
    // the walkable children — any declared edge (intra-package)
    for m in scope.mods() {
        let child = mods::child_path(&path, m.name());
        push_item(
            &mut out,
            CompletionOut::plain(
                m.name().to_string(),
                format!(
                    "file module — {}",
                    mods::display_of(chain, Some(pkg), &child)
                ),
                m.doc_lines(),
                CompletionKind::Module,
            ),
        );
    }
    for t in scope.types() {
        if !mods::vis_allows(from, &path, t.vis()) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                t.name().to_string(),
                t.head(),
                t.doc_lines(),
                match t.form() {
                    TyForm::Interface => CompletionKind::Interface,
                    _ => CompletionKind::Type,
                },
            ),
        );
    }
    for f in scope.fns() {
        if f.owner().is_some() || !mods::vis_allows(from, &path, f.vis()) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                f.name().to_string(),
                f.src(),
                f.doc_lines(),
                CompletionKind::Fn,
            ),
        );
    }
    for l in scope.lets() {
        if !mods::vis_allows(from, &path, l.vis()) {
            continue;
        }
        push_item(
            &mut out,
            CompletionOut::plain(
                l.name().to_string(),
                l.src(),
                l.doc_lines(),
                CompletionKind::Const,
            ),
        );
    }
    // a `pub use` re-export: the imported name joined the module's
    // surface — the same tier the use-path exports ride, gated by the
    // use's declared vis (the engine's leaf gate reads the same row).
    // The cap law picks the icon until real resolution (same as the
    // use classifier)
    for u in scope.uses() {
        if !mods::vis_allows(from, &path, u.vis()) {
            continue;
        }
        let src = u.path().join("::");
        push_item(
            &mut out,
            CompletionOut::plain(
                u.name().to_string(),
                format!("re-export of {src}::{}", u.name()),
                Vec::new(),
                if crate::world::infer::is_cap(u.name()) {
                    CompletionKind::Type
                } else {
                    CompletionKind::Fn
                },
            ),
        );
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Some(out)
}
