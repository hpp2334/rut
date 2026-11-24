//! The tier outs — the bare/locals/member/use tiers. Enumeration and
//! the gates live in the collection doors (`members`,
//! `bare`, `use_exports`, keyed on the walk's node);
//! each tier here maps the rows it found to the wire items and
//! applies the typed prefix (presentation, at the call site).

use super::rows::{filtered, push_item, BindRow, CompletionOut, Row};
use super::CompletionKind;
use crate::doc::Mode;
use crate::env::Io;
use crate::resolve;
use crate::resolve::mods;
use crate::world::{FileId, World};
use rut_ast::AnyHandle;

/// The member tier — the enumeration and the gates live in the
/// collection door (`members`, keyed on the walk's node); this
/// maps its view rows to the wire items and applies the typed prefix
/// (presentation, at the call site).
pub(super) fn member_tier_out(
    rows: Vec<crate::resolve::MemberRow>,
    prefix: &str,
) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for r in rows {
        match r {
            crate::resolve::MemberRow::Field(m) | crate::resolve::MemberRow::Method(m) => {
                push_item(&mut out, CompletionOut::of(&Row::Member(m)));
            }
            crate::resolve::MemberRow::ImplFn(f) => {
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
pub(super) fn local_completions(w: &World, file: FileId, pos: u32) -> Vec<CompletionOut> {
    let doc = w.file(file);
    let mut best: Vec<&rut_ast::Binding> = Vec::new();
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
                ty: resolve::binding_ty(w, file, b),
            });
            CompletionOut::of(&row)
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// The bare tier — the keyword table (the grammar's reserved set, one
/// canonical list) ahead of the collection door's visible decls
/// (`bare`: the bare-name gates — own, ambient, used-from —
/// live inside). The label filter is the call site's.
pub(super) fn bare_tier_out(w: &World, file: FileId) -> Vec<CompletionOut> {
    let mut out: Vec<CompletionOut> = Vec::new();
    for kw in crate::doc::RESERVED_KW {
        out.push(CompletionOut::of(&Row::Kw(kw)));
    }
    let bare = resolve::bare(w, file);
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

/// The mod-name tier — the `mod NAME;` name slot's answer: the module
/// files beside the open document, listed by the disk probe in
/// `super::mod_name` (the loader's child spellings — the mount law's
/// home is rut-driver's `mount_edges`). The slot's FILE facts derive
/// from the file's own entry: the directory from its origin
/// (`file://` uris only — [`super::mod_name::file_dir`]), the flavor
/// from its stamp (a `.d.rut` doc — children spell `mod.d.rut`).
/// No filesystem to ask (no env, or a non-`file://` uri) is an empty
/// list — a miss is an empty list, never the bare dump. The gate: a
/// name the open document ALREADY declares never re-offers (a
/// duplicate `mod` is an error) — EXCEPT the decl under the cursor,
/// whose own typed name stays a candidate.
pub(super) fn mod_name_tier_out(
    w: &World,
    file: FileId,
    io: Option<&dyn Io>,
    node: AnyHandle,
    prefix: &str,
) -> Vec<CompletionOut> {
    let dir = super::mod_name::file_dir(&w.file(file).stamp.origin);
    let decl = w.file(file).stamp.mode == Mode::Decl;
    let (Some(dir), Some(io)) = (dir.as_deref(), io) else {
        return Vec::new();
    };
    let mut out: Vec<CompletionOut> = Vec::new();
    for c in super::mod_name::list_child_mods(io, dir, decl) {
        // the declared gate — another `mod` decl of the open doc
        // owning the name kills the candidate
        if w.file(file)
            .mods()
            .iter()
            .any(|m| m.h != node && m.name() == c.name)
        {
            continue;
        }
        // hover's own spelling for a mod edge (`file module —
        // \`harness/mod.rut\``)
        out.push(CompletionOut::plain(
            c.name,
            format!("file module — `{}`", c.file),
            Vec::new(),
            CompletionKind::Module,
        ));
    }
    filtered(out, prefix)
}

/// The use tier — the enumeration, the walk and the pub gates live in
/// the collection door (`use_exports`, keyed on the walk's use
/// node); this maps its view rows to the wire items and applies the
/// typed prefix (presentation, at the call site).
pub(super) fn use_tier_out(
    w: &World,
    ux: Option<crate::resolve::UseExports>,
    prefix: &str,
) -> Vec<CompletionOut> {
    let Some(ux) = ux else {
        return Vec::new();
    };
    let mut out: Vec<CompletionOut> = Vec::new();
    match ux {
        crate::resolve::UseExports::Modules(list) => {
            for (m, origin) in list {
                push_item(
                    &mut out,
                    CompletionOut::plain(m, origin, Vec::new(), CompletionKind::Module),
                );
            }
        }
        crate::resolve::UseExports::Exports { pkg, path, rows } => {
            for r in rows {
                match r {
                    // the walkable children — the detail renders the
                    // mounted display from the child's mod path
                    crate::resolve::ExportRow::Mod(m, child) => push_item(
                        &mut out,
                        CompletionOut::plain(
                            m.name().to_string(),
                            format!(
                                "file module — {}",
                                mods::display_of(w, Some(pkg.as_str()), &child)
                            ),
                            m.doc_lines(),
                            CompletionKind::Module,
                        ),
                    ),
                    crate::resolve::ExportRow::Ty(t) => {
                        push_item(&mut out, CompletionOut::of(&Row::Ty(t)));
                    }
                    crate::resolve::ExportRow::Fn(fv) => {
                        push_item(&mut out, CompletionOut::of(&Row::Fn(fv)));
                    }
                    crate::resolve::ExportRow::Let(l) => {
                        push_item(&mut out, CompletionOut::of(&Row::Let(l)));
                    }
                    // a `pub use` re-export: the imported name joined
                    // the module's surface. The cap law picks the icon
                    // until real resolution (same as the use classifier)
                    crate::resolve::ExportRow::ReExport(u) => {
                        let src = u.path().join("::");
                        push_item(
                            &mut out,
                            CompletionOut::plain(
                                u.name().to_string(),
                                format!("re-export of {src}::{}", u.name()),
                                Vec::new(),
                                if crate::doc::is_cap(u.name()) {
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
