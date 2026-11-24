//! References — the definition index read BACKWARDS (the lsp-features
//! survey §4.4). One law, zero duplicated resolution: **a token is a
//! reference to a declaration iff the one walk from that token lands
//! on the declaration.** The query resolves its own position through
//! the unified walk ([`crate::resolve::cursor`] →
//! [`crate::resolve::walk::resolve`]) to a set of typed targets
//! ([`crate::resolve::Def`]s), then scans the candidate files' name
//! positions through the SAME walk, keeping the sites whose answer
//! contains a target. The two directions cannot disagree about what
//! an ident means — a reference set is exactly the ctrl+click set.
//!
//! * within-file: the binding pass answers locals/params/iter-vars —
//!   shadow-aware by construction (a shadowing decl and its uses resolve
//!   to the shadow, never to the outer binding) and scope-exit correct;
//! * cross-file: for a module-level name (a fn, a type, a module let)
//!   EVERY world file is a candidate — each of its name positions runs
//!   through the same walk IN THAT FILE'S OWN CONTEXT (its own stamp —
//!   the own-package lanes key on it, so a sibling's `use m::{ x }` and
//!   its `app::x` both resolve), and counts iff the answer contains a
//!   target. No import-map pre-filter: a qualified spelling, a glob
//!   binding, and a pkg-rooted dep path all name no leaf the pre-filter
//!   could see;
//! * the one-name silence rides at the same cursor kinds it has for
//!   definition ([`crate::feature::definition::definition_silent`]) — on
//!   the query AND on every scanned site: a name the file binds twice
//!   by distinct bindings counts nothing, never a pick between them;
//! * a declaring ident joins the set only through `include_decl` (the
//!   LSP spec's toggle) — and only when it declares a TARGET: a
//!   same-named declaration of a different binding is never a
//!   reference of this one.
//!
//! Honest limits (recorded, never wrong): unimported (flat-world) names
//! are scanned in the asking file and the declaring file only — the
//! flat sweep does not pull the whole workspace in; a member reference needs
//! the receiver to resolve exactly where the walk resolves it. Field /
//! enum-member / local targets never cross files (locals can't escape;
//! imports carry type and fn names, not members).
//!
//! Every candidate is a world [`File`] — its document and tree are
//! KEPT, so there is no re-parse lane: the scan rides each file's own
//! recorded name inventory ([`rut_ast::name_positions`]).

use std::collections::HashSet;

use crate::feature::definition;
use crate::resolve::cursor::{self, Cursor};
use crate::resolve::def::{loc_at, DefLocation, View};
use crate::resolve::walk;
use crate::world::{FileId, World};

/// `textDocument/references` — every position whose definition is the
/// declaration under `pos`. Sorted by uri then position; deduplicated.
pub fn references(w: &World, file: FileId, pos: u32, include_decl: bool) -> Vec<DefLocation> {
    // 1. the targets — the one walk's answer at the query position,
    //    silenced at the same cursor kinds definition silences (a
    //    twice-bound name counts nothing). Multiple hits = the walk's
    //    candidate list: every candidate's references genuinely point
    //    at the ambiguous name, so they all count (never a wrong
    //    silent pick).
    let c = cursor::cursor(w, file, pos);
    if definition::definition_silent(w, file, pos, &c) {
        return Vec::new();
    }
    let targets = walk::resolve(w, file, &c);
    if targets.is_empty() {
        return Vec::new();
    }

    // 2. the files worth scanning — the asking document first, each
    //    target's own file; a module-level name (a fn, a type, a
    //    module let) reaches every world file (a sibling's bare use
    //    binding, its qualified spelling, a glob, a pkg-rooted dep
    //    path — all are references, none spells a leaf an import map
    //    could pre-filter on); locals stay in the doc and members add
    //    only their declaring file. One entry per file (the store's
    //    own law — one entry per origin, nothing to dedupe beyond the
    //    id set).
    let module_level = targets.iter().any(|d| {
        matches!(
            d.view(w),
            Some(View::Ty(_)) | Some(View::Fn(_)) | Some(View::Let(_))
        )
    });
    let mut cand: Vec<FileId> = vec![file];
    cand.extend(targets.iter().map(|d| d.file()));
    if module_level {
        cand.extend(w.files());
    }
    let mut seen: HashSet<FileId> = HashSet::new();
    let mut files: Vec<FileId> = Vec::new();
    for id in cand {
        if seen.insert(id) {
            files.push(id);
        }
    }

    // 3. scan each candidate file: its KEPT document rides (no
    //    re-parse — the mount lane's own law). The positions are the
    //    tree's own name inventory (`name_positions` — every recorded
    //    name span); no token scan. Each candidate resolves under ITS
    //    OWN STAMP (one world, one store: a file's own-package lanes
    //    key on the pair its loader lane laid down, exactly as when
    //    that file is the open document; the asking doc keeps its own
    //    for its idents) — and the matcher compares DEFS, the walk's
    //    typed answers on both sides.
    let names: HashSet<&str> = targets.iter().map(|d| d.name(w)).collect();
    let mut out: Vec<DefLocation> = Vec::new();
    let mut seen_locs: HashSet<(String, u32, u32, u32, u32)> = HashSet::new();
    for id in files {
        let f = w.file(id);
        let ast = f.doc.ast();
        for (sp, ident) in rut_ast::name_positions(ast) {
            // not one of the targets' names — no answer of this site
            // can name a target
            if !names.contains(ast.name(ident)) {
                continue;
            }
            let loc = loc_at(f, sp);
            let site = cursor::cursor(w, id, sp.lo);
            match site {
                // a declaring ident joins only through include_decl —
                // and only when it declares a TARGET (a same-named
                // decl of another binding is no reference of this one;
                // a use-statement leaf is no Decl — importing IS a
                // reference)
                Cursor::Decl(d) => {
                    if include_decl && targets.contains(&d) && seen_locs.insert(loc_key(&loc)) {
                        out.push(loc);
                    }
                }
                site => {
                    // the silence, at the same cursor kinds definition
                    // silences — a twice-bound name counts nothing
                    if definition::definition_silent(w, id, sp.lo, &site) {
                        continue;
                    }
                    if walk::resolve(w, id, &site)
                        .iter()
                        .any(|d| targets.contains(d))
                        && seen_locs.insert(loc_key(&loc))
                    {
                        out.push(loc);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        (
            &a.uri,
            a.range.start.line,
            a.range.start.character,
            a.range.end.line,
            a.range.end.character,
        )
            .cmp(&(
                &b.uri,
                b.range.start.line,
                b.range.start.character,
                b.range.end.line,
                b.range.end.character,
            ))
    });
    out
}

fn loc_key(l: &DefLocation) -> (String, u32, u32, u32, u32) {
    (
        l.uri.clone(),
        l.range.start.line,
        l.range.start.character,
        l.range.end.line,
        l.range.end.character,
    )
}
