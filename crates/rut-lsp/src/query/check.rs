//! The semantic diagnostics lane (B1+B2) — "check package → per-file
//! diagnostics": the compiler's checker (rut-lir, through the driver's
//! graph walk) runs over the document's package and its dep closure;
//! every diagnostic carries the file stamp the graph lane lays down
//! (`spec` for a pkg's root module, `spec/mod-path` for a child), so
//! the mapping back to owning documents is data, not guessing:
//!
//! - **parse-only fallback**: no package context (`pkg: None`) → the
//!   parse lane's diags alone;
//! - **the degraded retry**: a world that cannot assemble (the run's
//!   shape/closure refusal, `Err` carrying no spans) retries ONCE with
//!   the doc's own package alone, its dangling `use` heads blanked
//!   span-preserving — silence is the bug: the doc's own name errors
//!   survive an unresolved dep closure, the unmounted heads surface as
//!   true unknown-name squiggles at their use sites, and a degraded
//!   refusal stands (the parse set alone; never a loop);
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

use crate::semantic::Diag;
use crate::world::collection::MountedFile;
use ls_types::Diagnostic;
use rut_ast::{ItemRef, Vis};
use rut_driver::{lower_decl_module, Manifest, ModSource, Mounted, RutRun};

/// One assembled package's file modules, keyed by mod path (`""` = the
/// pkg root's module file).
pub struct PkgFiles {
    /// the bare pkg name — the `use` head and the diag stamp's prefix
    pub module: String,
    pub files: BTreeMap<String, FileSrc>,
}

/// One file of an assembled package.
pub struct FileSrc {
    /// the diag-mapping target — an open doc's URI key or a def origin
    pub uri: String,
    /// the NORMALIZED text — the byte space every span of the lane
    /// speaks (the parse lane's ranges ride the same normalization)
    pub text: String,
    pub mode: crate::semantic::Mode,
    /// the `mod` edge the file was mounted under (its parent's spelling)
    pub vis: Vis,
}

/// The dep world off the mounted-file store: every (module,
/// mod_path)-stamped file joins its module's group (first-wins per mod
/// path — a re-walk or a bundle twin never doubles a file). Unstamped
/// files (plain workspace entries, the server's scan) are nobody's
/// package and stay out; `core` stays out (the engine auto-offers the
/// real prelude). Group order follows first appearance — deterministic
/// runs; pkgs mount namespace-free, always (the mechanism died — module
/// paths and bare consts are the spellings).
pub fn groups_from_files(files: &[MountedFile]) -> Vec<PkgFiles> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, PkgFiles> = BTreeMap::new();
    for f in files {
        let Some(module) = f.mount.module.clone() else {
            continue;
        };
        if module == "core" {
            continue;
        }
        let path = f.mount.mod_path.clone().unwrap_or_default();
        let mode = f
            .mount
            .mode
            .unwrap_or_else(|| crate::semantic::mode_of(&f.mount.origin));
        let name = module.clone();
        if !groups.contains_key(&name) {
            order.push(name.clone());
            groups.insert(
                name.clone(),
                PkgFiles {
                    module,
                    files: BTreeMap::new(),
                },
            );
        }
        let g = groups.get_mut(&name).expect("just inserted");
        if g.files.contains_key(&path) {
            continue;
        }
        let vis = edge_vis(files, &g.module, &path);
        g.files.insert(
            path,
            FileSrc {
                uri: f.mount.origin.clone(),
                text: f.doc.src().to_string(),
                mode,
                vis,
            },
        );
    }
    order
        .into_iter()
        .filter_map(|m| groups.remove(&m))
        .collect()
}

/// The `mod` edge a child file was mounted under, read off its parent's
/// mounted file (`pub mod layout;`). An unreadable parent is optimistic
/// (`Pub`): a wrong-priv edge costs a lint, a false "`x` is not a pub
/// module of `y`" squiggle costs trust.
fn edge_vis(files: &[MountedFile], module: &str, path: &str) -> Vis {
    let (parent, name) = match path.rsplit_once('/') {
        Some((p, n)) => (p, n),
        None => ("", path),
    };
    for f in files {
        if f.mount.module.as_deref() == Some(module) && crate::world::mods::mod_path_of(f) == parent
        {
            if let Some(e) = f.mods().into_iter().find(|m| m.name() == name) {
                return e.vis();
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
    crate::query::analysis::merge_diags(parse_diags, hits)
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
    // the LIVE text, canonicalized through the Cow door — borrowed when
    // LF-clean, one pass when dirty (the check lane never hands out
    // tokens; it only builds text rows for the world's own parse)
    let doc_text = crate::semantic::canonical(doc_src).into_owned();
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
            mode: crate::semantic::mode_of(doc_uri),
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
    // the run: a shape/closure refusal is `Err` carrying no spans.
    // Silence is the bug — the doc's own name errors must survive an
    // unresolved dep closure — so the refusal retries ONCE, DEGRADED:
    // the doc's own package alone ([`degraded_diags`]), mapped through
    // the same stamps/guards below. A degraded refusal stands as today
    // (the parse set alone; one retry, never a loop) — and the parse
    // set merged first either way: the degraded set only ADDS.
    let diags: Vec<Diag> = match compile_world(build_world(&groups), doc_pkg) {
        Some(d) => d,
        None => {
            let own = groups
                .into_iter()
                .find(|g| g.module == doc_pkg)
                .expect("the doc's group rides its own upsert");
            degraded_diags(&own, doc_pkg).unwrap_or_default()
        }
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
        let Some((_, (uri, text))) = where_.iter().find(|((s, p), _)| *file == stamp_of(s, p))
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
        // the index builds per text — the rows name DIFFERENT files
        let index = crate::semantic::LineIndex::new(text);
        out.push(crate::query::analysis::to_diagnostic_parts(d, text, &index));
    }
    out
}

/// One compile attempt over an assembled world: `None` = the run's
/// shape/closure refusal — the `Err` carries no spans, the caller
/// decides what stands.
fn compile_world(world: Vec<Mounted>, doc_pkg: &str) -> Option<Vec<Diag>> {
    let mut run = RutRun::new();
    for p in world {
        run = run.mount(p);
    }
    Some(run.entrypoint(doc_pkg).compile().ok()?.graph.diags)
}

/// The degraded retry (the full world's compile refused): the doc's
/// OWN package alone — every dep group dropped (the `Err` carries no
/// per-pkg blame to drop finer) — compiled and answered, `None` when
/// the degraded run refuses too (the parse set stands alone; one
/// retry, never a loop).
///
/// The group's dangling `use` heads are blanked span-preserving first
/// ([`blank_dangling_heads`]): the closure gate refuses a run whose
/// source names a head nothing offered, so the doc-only world would
/// refuse again and the silence — the bug — would stand. The blanked
/// heads' names surface as true unknown-name squiggles at their use
/// sites; the span-0/URI/bounds skips below keep it honest.
fn degraded_diags(group: &PkgFiles, doc_pkg: &str) -> Option<Vec<Diag>> {
    let files = group
        .files
        .iter()
        .map(|(p, f)| {
            (
                p.clone(),
                FileSrc {
                    uri: f.uri.clone(),
                    text: blank_dangling_heads(&f.text, f.mode, group, p),
                    mode: f.mode,
                    vis: f.vis,
                },
            )
        })
        .collect();
    let degraded = PkgFiles {
        module: group.module.clone(),
        files,
    };
    compile_world(build_world(&[degraded]), doc_pkg)
}

/// One file's degraded parse copy: every `use` naming a head the
/// degraded world cannot offer — not `core` (the engine auto-offers
/// the real prelude), not the doc's own claim ([`own_head_claim`]) —
/// is blanked: its bytes become spaces, newlines kept. Byte-preserving
/// by construction, so every span the degraded run reports indexes the
/// same offsets the mapping table's ORIGINAL texts hold — never a
/// misplaced squiggle.
fn blank_dangling_heads(
    text: &str,
    mode: crate::semantic::Mode,
    group: &PkgFiles,
    path: &str,
) -> String {
    let doc = crate::semantic::Document::parse(mode, text.to_string());
    let ast = doc.ast();
    let mut bytes = text.as_bytes().to_vec();
    for it in ast.root().items(ast) {
        let ItemRef::Use(uh) = it else {
            continue;
        };
        let u = uh.get(ast);
        let Some(head) = u.path.first() else {
            continue; // the incomplete shape — no head to dangle
        };
        let head = ast.name(head.ident);
        if head == "core" || own_head_claim(group, path, head) {
            continue; // the degraded world offers it — the statement stands
        }
        let (lo, hi) = (u.span.lo as usize, u.span.hi as usize);
        if lo > hi || hi > bytes.len() {
            continue; // out of bounds — the statement stands (the run may refuse)
        }
        for b in &mut bytes[lo..hi] {
            if *b != b'\n' {
                *b = b' '; // ASCII writes only — the copy stays UTF-8
            }
        }
    }
    String::from_utf8(bytes).expect("blanking writes ASCII spaces only — the copy stays UTF-8")
}

/// The doc's own-package answer for a use head — the driver's own-lane
/// walk (`Session::own_use_head`), mirrored over the group's file set:
/// a `mod` edge walked from `from` (the importing file's mod path; the
/// current module first, then the ancestors, nearest wins), or the
/// doc's OWN spec naming the root module — a non-empty mod tree only
/// (a flat package's own name was never a legal use head there).
fn own_head_claim(group: &PkgFiles, from: &str, head: &str) -> bool {
    let mut cur = from.to_string();
    loop {
        let child = if cur.is_empty() {
            head.to_string()
        } else {
            format!("{cur}/{head}")
        };
        if group.files.contains_key(&child) {
            return true;
        }
        let parent = match cur.rsplit_once('/') {
            Some((p, _)) => p.to_string(),
            None => String::new(),
        };
        if parent == cur {
            break;
        }
        cur = parent;
    }
    let has_mods = group.files.keys().any(|p| !p.is_empty());
    has_mods && head == group.module
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
        let Some(root) = g.files.get("") else {
            continue;
        };
        let deps: Vec<&str> = names
            .iter()
            .filter(|n| n.as_str() != g.module)
            .map(|n| n.as_str())
            .collect();
        let mut pkg = if root.mode == crate::semantic::Mode::Impl {
            if g.files
                .values()
                .any(|f| f.mode != crate::semantic::Mode::Impl)
            {
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
                if path.is_empty() || f.mode != crate::semantic::Mode::Decl {
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
                // the registration scope rides [`Mounted::named`] — the
                // door's own case (the lowering's surface, named at the
                // offer). The surface's own `pub let NAME: f64 =
                // <literal>;` rows folded into the host body's const
                // rows INSIDE the lowering (bound bare at the pkg's
                // root) — a `pub use calc_host::{ PI }` re-export
                // resolves against them. The manifest carries no consts
                // of its own (the retired table refuses at parse)
                Ok(p) => p.named(&g.module),
                Err(_) => continue,
            }
        };
        pkg = pkg.manifest(Manifest::lib(&g.module, &deps));
        out.push(pkg);
    }
    out
}
