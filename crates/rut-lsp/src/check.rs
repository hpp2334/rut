//! The semantic diagnostics lane (B1+B2) — "check package → per-file
//! diagnostics" shared by both faces: the native stdio server (over its
//! workspace/manifest context) and the wasm shim (over the extension's
//! mounted defs). The compiler's checker (rut-lir, through the driver's
//! graph walk) runs over the document's package and its dep closure;
//! every diagnostic carries the file stamp the graph lane lays down
//! (`spec` for a pkg's root module, `spec/mod-path` for a child), so
//! the mapping back to owning documents is data, not guessing:
//!
//! - **parse-only fallback**: no package context (`pkg: None`) or a
//!   world that cannot assemble → the parse lane's diags alone
//!   (today's behavior, never worse);
//! - **unattributable spans** (the graph-level refusals' `file: None`)
//!   and spans naming files that are not the open document → skipped,
//!   no misplaced squiggle;
//! - **the merge** with the parse set dedupes TRUE duplicates only
//!   (same range AND message — the check lane re-parses the doc, so a
//!   parse error arrives again) and keeps every distinct site the
//!   checker reports (the B3 emission law: multiple sites per body).
//!
//! The world assembly is conservative by law: a package mounts from its
//! (module, mod_path)-stamped file indexes; a lib body mounts as
//! `Mounted::source` + its mod tree, a host surface (a `.d.rut` group)
//! lowers through `lower_decl_module` into real host rows (so `use
//! ink_host::{ create_logger }` binds and type-checks); `core` never
//! mounts (the engine auto-offers the real prelude, and first-pkg-wins
//! would let a def-store group replace it); a group that refuses to
//! assemble mounts nothing, and its consumers' misses degrade to
//! span-0 diags, which skip.

use std::collections::BTreeMap;
use std::str::FromStr;

use ls_types::{Diagnostic, Uri};
use rut_ast::ast::Vis;
use rut_driver::{lower_decl_module, Manifest, ModSource, Mounted, RutRun};
use rut_lexer::diag::Diag;
use rut_lexer::lexer::normalize;
use rut_parser::Mode;

use crate::hover::DefIndex;
use crate::line_index::LineIndex;
use crate::std_surface;

/// One assembled package's file modules, keyed by mod path (`""` = the
/// pkg root's module file).
pub struct PkgFiles {
    /// the bare pkg name — the `use` head and the diag stamp's prefix
    pub module: String,
    /// the qualified-access head the pkg's manifest spells (`calc`'s
    /// `Math`) — the surface grammar cannot say it, so the assembly
    /// carries it; `None` mounts the pkg namespace-free
    pub namespace: Option<String>,
    pub files: BTreeMap<String, FileSrc>,
}

/// One file of an assembled package.
pub struct FileSrc {
    /// the diag-mapping target — an open doc's URI key or a def origin
    pub uri: String,
    /// the NORMALIZED text — the byte space every span of the lane
    /// speaks (the parse lane's ranges ride the same normalization)
    pub text: String,
    pub mode: Mode,
    /// the `mod` edge the file was mounted under (its parent's spelling)
    pub vis: Vis,
}

/// The dep world off the defs store: every (module, mod_path)-stamped
/// index joins its module's group (first-wins per mod path — a re-walk
/// or a bundle twin never doubles a file). Unstamped indexes (plain
/// workspace files, the server's scan) are nobody's package and stay
/// out; `core` stays out (the engine auto-offers the real prelude).
/// Group order follows first appearance — deterministic runs.
pub fn groups_from_defs(defs: &[DefIndex]) -> Vec<PkgFiles> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, PkgFiles> = BTreeMap::new();
    for d in defs {
        let Some(module) = d.module.clone() else { continue };
        if module == "core" {
            continue;
        }
        let path = d.mod_path.clone().unwrap_or_default();
        let mode = d.mode.unwrap_or_else(|| crate::analysis::mode_of(&d.origin));
        let name = module.clone();
        if !groups.contains_key(&name) {
            order.push(name.clone());
            groups.insert(
                name.clone(),
                PkgFiles {
                    namespace: std_surface::NAMESPACES
                        .iter()
                        .find(|(m, _)| *m == module)
                        .map(|(_, head)| head.to_string()),
                    module,
                    files: BTreeMap::new(),
                },
            );
        }
        let g = groups.get_mut(&name).expect("just inserted");
        if g.files.contains_key(&path) {
            continue;
        }
        let vis = edge_vis(defs, &g.module, &path);
        g.files.insert(
            path,
            FileSrc { uri: d.origin.clone(), text: d.src.clone(), mode, vis },
        );
    }
    order.into_iter().filter_map(|m| groups.remove(&m)).collect()
}

/// The `mod` edge a child file was mounted under, read off its parent's
/// index (`pub mod layout;`). An unreadable parent is optimistic
/// (`Pub`): a wrong-priv edge costs a lint, a false "`x` is not a pub
/// module of `y`" squiggle costs trust.
fn edge_vis(defs: &[DefIndex], module: &str, path: &str) -> Vis {
    let (parent, name) = match path.rsplit_once('/') {
        Some((p, n)) => (p, n),
        None => ("", path),
    };
    for d in defs {
        if d.module.as_deref() == Some(module) && crate::mods::mod_path_of(d) == parent {
            if let Some(e) = d.mods.iter().find(|m| m.name == name) {
                return e.vis;
            }
        }
    }
    Vis::Pub
}

/// One face's whole diagnostics answer for one document: the parse
/// lane's diags merged with the check lane's. `pkg: None` — no package
/// context — is the parse-only fallback (exactly today's answer).
pub fn merged_doc_diags(
    doc_uri: &str,
    doc_src: &str,
    pkg: Option<String>,
    mod_path: Option<String>,
    groups: Vec<PkgFiles>,
    parse_diags: Vec<Diagnostic>,
) -> Vec<Diagnostic> {
    let Some(pkg) = pkg else { return parse_diags };
    let hits = check_doc(doc_uri, doc_src, &pkg, mod_path.as_deref(), groups);
    crate::analysis::merge_diags(parse_diags, hits)
}

/// Check one open document's package: assemble the world, run the graph
/// walk (the checker), map every attributed diag back through its file
/// stamp, keep only the open document's own squiggles. Empty = nothing
/// to add to the parse set.
pub fn check_doc(
    doc_uri: &str,
    doc_src: &str,
    doc_pkg: &str,
    doc_mod_path: Option<&str>,
    mut groups: Vec<PkgFiles>,
) -> Vec<Diagnostic> {
    let path = doc_mod_path.unwrap_or("").to_string();
    let doc_text = normalize(doc_src);
    // the doc's package rides its LIVE text: find (or synthesize) the
    // group and upsert the doc's file — an unsaved buffer is the truth
    // for its own mod path, the stamps only name the place
    let slot = match groups.iter_mut().find(|g| g.module == doc_pkg) {
        Some(g) => g,
        None => {
            if !path.is_empty() {
                return Vec::new(); // a child of a pkg with no root cannot assemble
            }
            groups.push(PkgFiles {
                module: doc_pkg.to_string(),
                namespace: None,
                files: BTreeMap::new(),
            });
            groups.last_mut().expect("just pushed")
        }
    };
    let vis = slot.files.get(&path).map(|f| f.vis).unwrap_or(Vis::Pub);
    slot.files.insert(
        path,
        FileSrc {
            uri: doc_uri.to_string(),
            text: doc_text,
            mode: crate::analysis::mode_of(doc_uri),
            vis,
        },
    );
    // (spec, mod_path) -> (uri, text) — the mapping table the file
    // stamps name; the doc's upsert already owns its own row
    let mut where_: BTreeMap<(String, String), (String, String)> = BTreeMap::new();
    for g in &groups {
        for (p, f) in &g.files {
            where_
                .entry((g.module.clone(), p.clone()))
                .or_insert_with(|| (f.uri.clone(), f.text.clone()));
        }
    }
    // the run: shape/closure refusals are `Err` carrying no spans —
    // the caller's parse set stands alone
    let mut run = RutRun::new();
    for p in build_world(&groups) {
        run = run.pkg(p);
    }
    let diags: Vec<Diag> = match run.entrypoint(doc_pkg).compile() {
        Ok(c) => c.graph.diags,
        Err(_) => return Vec::new(),
    };
    let stamp_of = |s: &str, p: &str| {
        if p.is_empty() {
            s.to_string()
        } else {
            format!("{s}/{p}")
        }
    };
    let mut out = Vec::new();
    for d in &diags {
        let Some(file) = d.file.as_ref() else {
            continue; // graph-level refusal — no position, no squiggle
        };
        let Some((_, (uri, text))) =
            where_.iter().find(|((s, p), _)| *file == stamp_of(s, p))
        else {
            continue; // a file the assembly never carried — never guessed at
        };
        if uri != doc_uri {
            continue; // another file's squiggle — its own analyze publishes it
        }
        let (lo, hi) = (d.span.lo as usize, d.span.hi as usize);
        if lo > hi || hi > text.len() {
            continue; // out of bounds — never a misplaced squiggle
        }
        let index = LineIndex::new(text);
        let mut diag = crate::analysis::to_diagnostic(d, text, &index);
        match Uri::from_str(uri) {
            Ok(u) => crate::analysis::set_related_uri(std::slice::from_mut(&mut diag), &u),
            Err(_) => diag.related_information = None,
        }
        out.push(diag);
    }
    out
}

/// The world's pkgs: a lib body mounts as `Mounted::source` + its mod tree;
/// a host surface lowers into real host rows (`lower_decl_module` — the
/// loader's own move, so `use ink_host::{ .. }` binds and type-checks).
/// Every pkg's manifest declares every other mounted head: the editor
/// knows no per-pkg dep tables, so the D5 nameability law stays the
/// CLI's business — an unmounted head misses as a span-0 diag, which
/// skips. Impl-only for source bodies: mixed-mode groups are not a
/// coherent unit and mount nothing.
fn build_world(groups: &[PkgFiles]) -> Vec<Mounted> {
    let names: Vec<String> = groups.iter().map(|g| g.module.clone()).collect();
    let mut out = Vec::new();
    for g in groups {
        let Some(root) = g.files.get("") else { continue };
        let deps: Vec<&str> = names
            .iter()
            .filter(|n| n.as_str() != g.module)
            .map(|n| n.as_str())
            .collect();
        let mut pkg = if root.mode == Mode::Impl {
            if g.files.values().any(|f| f.mode != Mode::Impl) {
                continue;
            }
            let mut p = Mounted::source(&g.module, root.text.clone());
            for (path, f) in &g.files {
                if path.is_empty() {
                    continue;
                }
                p.mods.insert(
                    path.clone(),
                    ModSource {
                        path: path.clone(),
                        vis: f.vis,
                        text: f.text.clone(),
                        file: "mod.rut".to_string(),
                    },
                );
            }
            p
        } else {
            // a host surface: the `.d.rut` tree lowers into bodyless
            // rows; a surface that refuses (a non-crossing signature, a
            // parse failure) is inert — consumers' misses skip
            let mut children: BTreeMap<String, ModSource> = BTreeMap::new();
            for (path, f) in &g.files {
                if path.is_empty() || f.mode != Mode::Decl {
                    continue;
                }
                children.insert(
                    path.clone(),
                    ModSource {
                        path: path.clone(),
                        vis: f.vis,
                        text: f.text.clone(),
                        file: "mod.rut".to_string(),
                    },
                );
            }
            match lower_decl_module(&root.text, &children, &g.module) {
                Ok(mut p) => {
                    p.spec = g.module.clone();
                    p
                }
                Err(_) => continue,
            }
        };
        pkg.namespace = g.namespace.clone();
        pkg = pkg.manifest(Manifest::lib(&g.module, &deps));
        out.push(pkg);
    }
    out
}
