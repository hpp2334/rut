//! The tier outs — the bare/locals/member/use tiers. Enumeration and
//! the gates live in the collection doors (`Chain::members`,
//! `Chain::bare`, `Chain::use_exports`, keyed on the walk's node);
//! each tier here maps the rows it found to the wire items and
//! applies the typed prefix (presentation, at the call site).

use super::rows::{filtered, push_item, BindRow, CompletionOut, Row};
use super::CompletionKind;
use crate::world::collection::Chain;
use crate::world::mods;

/// The member tier — the enumeration and the gates live in the
/// collection door (`Chain::members`, keyed on the walk's node); this
/// maps its view rows to the wire items and applies the typed prefix
/// (presentation, at the call site).
pub(super) fn member_tier_out(
    rows: Vec<crate::world::collection::MemberRow>,
    prefix: &str,
) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for r in rows {
        match r {
            crate::world::collection::MemberRow::Field(m)
            | crate::world::collection::MemberRow::Method(m) => {
                push_item(&mut out, CompletionOut::of(&Row::Member(m)));
            }
            crate::world::collection::MemberRow::ImplFn(f) => {
                push_item(&mut out, CompletionOut::of(&Row::Fn(f)));
            }
        }
    }
    filtered(out, prefix)
}

/// The locals tier at a bare position: every fn-local binding visible at
/// `pos`, under the same shadow-correct, scope-exit-correct rule the
/// resolve rides — per NAME the innermost in-scope declaration wins (an
/// inner re-decl replaces the outer inside its scope; past the block the
/// name is gone), and a binding whose own declaring ident holds the
/// cursor stays off (don't offer the name mid-typing at its own
/// declaration). Emitted ahead of the keyword table — the name being
/// typed is most often a binding just declared. The doc's own pass is
/// read internally (`Doc::bindings()` — lazy per version).
pub(super) fn local_completions(chain: &Chain, pos: u32) -> Vec<CompletionOut> {
    let doc = chain.head();
    let mut best: Vec<&crate::world::Binding> = Vec::new();
    for b in doc.doc.bindings() {
        if b.decl_ident_span.lo > pos || !(b.scope_span.lo <= pos && pos < b.scope_span.hi) {
            continue; // declared later, or its scope ended before the cursor
        }
        if b.decl_ident_span.lo <= pos && pos <= b.decl_ident_span.hi {
            continue; // the cursor sits in the declaration's own ident
        }
        match best.iter().position(|x| x.name == b.name) {
            Some(i) => {
                if best[i].decl_ident_span.lo < b.decl_ident_span.lo {
                    best[i] = b; // the inner shadow wins inside its scope
                }
            }
            None => best.push(b),
        }
    }
    let mut out: Vec<CompletionOut> = best
        .into_iter()
        .map(|b| {
            let row = Row::Bind(BindRow {
                b,
                doc,
                ty: chain.binding_ty(b),
            });
            CompletionOut::of(&row)
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// The bare tier — the keyword table (the grammar's reserved set, one
/// canonical list) ahead of the collection door's visible decls
/// (`Chain::bare`: the bare-name gates — own, ambient, used-from —
/// live inside). The label filter is the call site's.
pub(super) fn bare_tier_out(chain: &Chain) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for kw in crate::semantic::RESERVED_KW {
        out.push(CompletionOut::of(&Row::Kw(kw)));
    }
    let bare = chain.bare();
    for t in bare.types {
        push_item(&mut out, CompletionOut::of(&Row::Ty(t)));
    }
    for f in bare.fns {
        push_item(&mut out, CompletionOut::of(&Row::Fn(f)));
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

// ---- the use-path tiers ----

/// The use tier — the enumeration, the walk and the pub gates live in
/// the collection door (`Chain::use_exports`, keyed on the walk's use
/// node); this maps its view rows to the wire items and applies the
/// typed prefix (presentation, at the call site).
pub(super) fn use_tier_out(
    chain: &Chain,
    ux: Option<crate::world::collection::UseExports>,
    prefix: &str,
) -> Vec<CompletionOut> {
    let Some(ux) = ux else {
        return Vec::new();
    };
    let mut out: Vec<CompletionOut> = Vec::new();
    match ux {
        crate::world::collection::UseExports::Modules(list) => {
            for (m, origin) in list {
                push_item(
                    &mut out,
                    CompletionOut::plain(m, origin, Vec::new(), CompletionKind::Module),
                );
            }
        }
        crate::world::collection::UseExports::Exports { pkg, path, rows } => {
            for r in rows {
                match r {
                    // the walkable children — the detail renders the
                    // mounted display from the child's mod path
                    crate::world::collection::ExportRow::Mod(m, child) => push_item(
                        &mut out,
                        CompletionOut::plain(
                            m.name().to_string(),
                            format!(
                                "file module — {}",
                                mods::display_of(chain, Some(pkg.as_str()), &child)
                            ),
                            m.doc_lines(),
                            CompletionKind::Module,
                        ),
                    ),
                    crate::world::collection::ExportRow::Ty(t) => {
                        push_item(&mut out, CompletionOut::of(&Row::Ty(t)));
                    }
                    crate::world::collection::ExportRow::Fn(fv) => {
                        push_item(&mut out, CompletionOut::of(&Row::Fn(fv)));
                    }
                    crate::world::collection::ExportRow::Let(l) => {
                        push_item(&mut out, CompletionOut::of(&Row::Let(l)));
                    }
                    // a `pub use` re-export: the imported name joined
                    // the module's surface. The cap law picks the icon
                    // until real resolution (same as the use classifier)
                    crate::world::collection::ExportRow::ReExport(u) => {
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
                }
            }
            let _ = path;
        }
    }
    filtered(out, prefix)
}
