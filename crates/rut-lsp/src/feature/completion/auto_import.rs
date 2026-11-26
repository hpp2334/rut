//! The auto-import tier — the foreign public names matching the bare
//! prefix, each carrying the `use mod::Name;` insert, sorted after
//! every local item.

use std::collections::HashSet;

use ls_types::Range;
use rut_ast::{Ast, ItemRef};

use super::rows::CompletionOut;
use super::CompletionKind;
use crate::doc::LineIndex;
use crate::world::ty::TyForm;
use crate::world::{File, FileId, World};

// ---- the auto-import tier ----

/// the foreign public names (indexes whose module is named) matching
/// the bare prefix, each carrying the `use mod::Name;` insert — after
/// the local results (the caller extends bare_completions' output), so
/// `sortText` sorts them after everything already offered. Names the
/// document already imports are NOT re-offered and the `pub builtin`
/// gate keeps holding (an import offer for a gated name would complete
/// a name the compile still rejects until the use lands — the gate is
/// the compiler's ambient split, mirrored).
///
/// `direct_deps` is the open document's own dep set — its manifest's
/// `deps`/`dev-deps` rows, a state-derived FACT threaded beside the
/// world (never the state itself). The
/// direct-dep gate DERIVES here, at the read, never stamped on the
/// index: the compiler's closure check (`Session::resolve_use` →
/// `declares`) refuses `use` of a mounted-but-undeclared package (the
/// D5 diagnostic), so a TRANSITIVE-only dep stays off — its names
/// would complete into a compile the program rejects. The gate speaks
/// only on mounted dep indexes (a named module); plain workspace
/// files never ride this tier.
pub(super) fn auto_import_completions(
    w: &World,
    file: FileId,
    ast: &Ast,
    src: &str,
    prefix: &str,
    direct_deps: &HashSet<String>,
) -> Vec<CompletionOut> {
    // the gate itself: the ambient prelude (origin `core` — a walked
    // dep's origin is always a uri, never the bare label, so no
    // collision) or a module the document's own manifest names
    let direct = |f: &File| {
        f.stamp.origin == "core"
            || f.stamp
                .module
                .as_deref()
                .is_some_and(|m| direct_deps.contains(m))
    };
    // the (name, pkg) set the import map spells — an imported name
    // is never re-offered with an insert edit
    let imported: HashSet<(&str, &str)> = crate::resolve::pkg::import_map(w.file(file))
        .into_iter()
        .collect();
    let gated: HashSet<String> = w
        .chain(file)
        .flat_map(|id| w.file(id).pub_gated())
        .collect();
    let insert = import_insert(src, ast);
    let mut out: Vec<CompletionOut> = Vec::new();
    for id in w.chain(file).skip(1) {
        let f = w.file(id);
        let Some(m) = &f.stamp.module else { continue };
        if !direct(f) {
            continue; // a transitive-only dep — `use` of it is the D5 error
        }
        let m = m.as_str();
        let mut foreign = |name: &str, doc: &[String], kind: CompletionKind| {
            if imported.contains(&(name, m)) {
                return; // already imported — never re-offered
            }
            if gated.contains(name) {
                return; // the ambient split keeps holding
            }
            if !prefix.is_empty() && !name.starts_with(prefix) {
                return;
            }
            // a fresh line either way: after a use the newline leads,
            // at the body top it trails — `rut fmt` owns any tidying
            let text = if insert.1 {
                format!("\nuse {m}::{name};")
            } else {
                format!("use {m}::{name};\n")
            };
            out.push(CompletionOut {
                label: name.to_string(),
                detail: format!("{m}::{name} — import"),
                doc: doc.to_vec(),
                kind,
                sort_text: Some(format!("~{name}")),
                additional_text_edits: vec![(insert.0, text)],
            });
        };
        for t in f.types() {
            if !t.is_pub() {
                continue;
            }
            let doc = t.doc_lines();
            foreign(
                t.name(),
                &doc,
                match t.form() {
                    TyForm::Interface => CompletionKind::Interface,
                    _ => CompletionKind::Type,
                },
            );
        }
        for fv in f.fns() {
            if fv.owner().is_some() {
                continue;
            }
            let doc = fv.doc_lines();
            foreign(fv.name(), &doc, CompletionKind::Fn);
        }
        for l in f.lets() {
            let doc = l.doc_lines();
            foreign(l.name(), &doc, CompletionKind::Const);
        }
    }
    // label dedup across modules: the first module (chain order) wins,
    // so one name offers one import
    let mut seen: HashSet<String> = HashSet::new();
    out.retain(|c| seen.insert(c.label.clone()));
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// the bare-position prefix: the partial ident the cursor sits in
/// (`Ve⏐` → `Ve`), else empty
fn import_insert(src: &str, ast: &Ast) -> (Range, bool) {
    let lines = LineIndex::new(src);
    let after_use = leading_use_end(ast);
    let (offset, newline_first) = match after_use {
        Some(off) => (off, true),
        None => (body_start(src), false),
    };
    let (line, ch) = lines.position(src, offset);
    (
        Range {
            start: ls_types::Position {
                line,
                character: ch,
            },
            end: ls_types::Position {
                line,
                character: ch,
            },
        },
        newline_first,
    )
}

/// the source end of the last `use` in the leading Use run — `None`
/// when the module opens with anything else. A use item's span is
/// EXACT (v2's own-terminator law): it ends at the statement's own `;`
fn leading_use_end(ast: &Ast) -> Option<u32> {
    let mut last: Option<u32> = None;
    for item in ast.root().items(ast) {
        if let ItemRef::Use(h) = item {
            last = Some(h.span(ast).hi);
        } else {
            break; // the run ends at the first item that isn't a use
        }
    }
    last
}

/// the byte offset where the file body starts: past the leading blank
/// and `//` comment lines (a file of only comments → the end)
fn body_start(src: &str) -> u32 {
    let mut off = 0usize;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            off += line.len();
        } else {
            break;
        }
    }
    off as u32
}
