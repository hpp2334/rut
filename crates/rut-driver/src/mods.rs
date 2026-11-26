//! File modules — the `mod` mount. Declarations are the graph: a
//! package's module set is the file tree its `mod` declarations name
//! (`NAME/mod.rut` beside the declaring file — or `NAME/mod.d.rut`
//! for a host surface tree), never a directory listing. This module is
//! the PURE half of that law:
//!
//! - [`mount_edges`] — the mounting core: depth-first in declaration
//!   order from a file's own `mod` declarations, every edge resolved
//!   through a caller-supplied child lookup, cycle-guarded by the
//!   canonical identity of each mounted file (the dep walk's law
//!   mirrored: the visiting chain is the cycle guard, first mount
//!   wins on a repeated path). `visit` sees each outcome and answers
//!   whether to keep walking — the editor's dep walk rides the same
//!   core with its budget lanes.
//! - [`mount_mod_children`] — the compile's strict door over the
//!   core: parses the root and collects the [`ModSource`] rows, and
//!   turns the first refusal into the loud error naming the declaring
//!   file and the spellings it probed (the FS lane in rut-native and
//!   the archive rows lane in the loader mount through it). The
//!   flavor rides the caller: a lib body tree speaks Impl and
//!   mounts `NAME/mod.rut`; a Decl surface tree (a `type = "host"`
//!   pkg's `.d.rut` set) speaks Decl and mounts `NAME/mod.d.rut`.
//!   Each child has TWO spellings: the directory form
//!   (`NAME/<child file>`) first, else the single-file form
//!   (`NAME.rut` beside the declaring file); both present is the loud
//!   ambiguity. Missing under both, or a directory without its module
//!   file, is a loud error naming the spellings.
//! - [`collect_module_set`] — the driver-side collection: every
//!   mounted file parses to its own `ItemKind::Module` root (children
//!   are NEVER spliced into the parent's AST) and its declarations are
//!   collected with the mod path attached — the plumbing phase 3
//!   (resolution/visibility) gates on.
//! - the `rut.mods` rows codec — the bundle envelope's additive
//!   section: a package's module set as path-keyed source rows
//!   (`""` = root, `"layout"` = child). The writer's shape law: a pkg
//!   with no mod children writes no `rut.mods` entry (byte-identical
//!   pack, the freshness gate proves it).
//!
//! No fs, no net, no `Path` — the crate purity law. The FS probes live
//! in rut-native's walk; the archive reads in the loader.

use std::collections::BTreeMap;

use rut_ast::parser::{parse_with_recover, Mode};
use rut_ast::{Ast, Doc, ItemRef, Linkage, Vis};

use crate::session::{Mounted, MountedBody};

/// The bundle entry that carries a package's module rows — the
/// additive envelope section (`<pkg>/rut.mods` for a dep group).
pub const ROWS_NAME: &str = "rut.mods";

/// One mounted file module (a `mod` child). The root file never rides
/// here — it is the pkg's body text; this map holds the tree's
/// children keyed by mod path (`"layout"`, `"layout/grid"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModSource {
    /// mod path relative to the pkg root — bare `[a-zA-Z0-9_]+`
    /// segments joined with `/`, never empty (the root is the body)
    pub path: String,
    /// the declaring edge's visibility (`mod` vs `pub mod`) — carried
    /// faithfully; the crossing gate itself is phase 3
    pub vis: Vis,
    /// the file's text
    pub text: String,
    /// the file's ROOT-RELATIVE path — the record the lane lays down
    /// where it mounts the file: the file lane from the found file's
    /// path relative to the root file's directory (`imp.rut`,
    /// `m/a/b.rut`, `imp/mod.rut`), the zip
    /// lane by stripping the source prefix; the rows/bundle/in-memory
    /// lanes keep the directory form (`<path>/mod.rut`). The display
    /// law ([`rut_lir::check::display`]) answers it verbatim.
    pub file: String,
}

/// What the FS/archive lane answers for one `mod` declaration's child
/// lookup. The four cases ARE the diagnostics: found (with the file's
/// canonical identity for the cycle guard and its root-relative
/// display record for the diagnostics — `imp/mod.rut` for a
/// directory-form child, `imp.rut`/`m/a/b.rut` for a single-file one),
/// both spellings present
/// (the loud ambiguity), a directory without its module file, and
/// nothing there at all. The directory form's spelling is the
/// caller's flavor (`mod.rut` for a lib body tree, `mod.d.rut` for a
/// host surface tree); the single-file form is always `NAME.rut`.
pub enum ChildLookup {
    /// a child file read; `canon` is its canonical identity (the
    /// cycle guard's key — a symlinked alias repeats it) and `file`
    /// the file's root-relative display record
    Found {
        text: String,
        canon: String,
        file: String,
    },
    /// both spellings exist — `NAME/<child file>` AND `NAME.rut`;
    /// which mounts is ambiguous, named loudly
    Ambiguous,
    /// the directory exists but carries no module file (and no
    /// `NAME.rut` beside it)
    NoModRut,
    /// nothing there under either spelling
    Missing,
}

/// The directory a `mod NAME;` declaration mounts its child in — the
/// parent FILE's directory, never a directory named after the parent's
/// mod path. `mod b;` in `m/a.rut` probes beside `a.rut` (`m/`), so the
/// child is `m/b.rut` / `m/b/mod.rut`; the root file (`mod.rut`,
/// `mod.d.rut`, a spelled surface) mounts at the package root (`""`).
///
/// THE ONE HOME of the rule: every lane that holds a parent's
/// root-relative record derives its probe base here — the source-zip
/// read and the FS walk alike — and the pack gate reads the record the
/// mount laid down. A lane that joins the parent's MOD PATH as a
/// directory is the mirror this law deleted.
pub fn child_dir(parent_file: &str) -> &str {
    parent_file
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("")
}

/// The two root-relative spellings a `mod NAME;` in `parent_file`
/// probes: the directory form beside the declaring file
/// (`<dir>/NAME/<child_file>`) and the single-file form
/// (`<dir>/NAME.rut`). `child_file` is the caller's flavor (`mod.rut`
/// for a lib body tree, `mod.d.rut` for a host surface tree).
pub fn child_probes(parent_file: &str, name: &str, child_file: &str) -> (String, String) {
    let dir = child_dir(parent_file);
    let stem = if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    };
    (format!("{stem}/{child_file}"), format!("{stem}.rut"))
}

// ---------------------------------------------------------------------
// the mount core (pure) — and the compile's strict door over it
// ---------------------------------------------------------------------

/// one declared `mod NAME;` edge the mount core resolved through the caller's lookup
pub enum Edge<'a> {
    Found {
        path: &'a str,
        vis: Vis,
        text: String,
        canon: String,
        file: String,
        depth: usize,
    },
    Refused {
        path: &'a str,
        name: &'a str,
        why: Refusal,
        depth: usize,
    },
}

pub enum Refusal {
    Ambiguous,
    NoModRut,
    Missing,
}

/// what the visitor did with a found child
pub enum Child {
    Declares(Vec<(String, Vis)>),
    Childless,
}

/// The mount core: depth-first in declaration order from the root file's own
/// `mod` declarations, every edge resolved through `read`, cycle-guarded by
/// canonical identity, first mount wins on a repeated path. `visit` sees each
/// outcome and answers whether to keep walking. The only `Err` is a cycle or a
/// failing `read`.
pub fn mount_edges(
    root_edges: Vec<(String, Vis)>,
    read: &mut dyn FnMut(&str, &str) -> Result<ChildLookup, String>,
    visit: &mut dyn FnMut(Edge<'_>) -> (Child, bool),
) -> Result<(), String> {
    let mut chain: Vec<String> = Vec::new();
    let mut done: Vec<String> = Vec::new();
    // edge depth: the root's own edges walk at 0, a child edge its
    // parent's + 1 — the budget lanes read it off the edge
    walk_edges("", 0, root_edges, read, visit, &mut chain, &mut done)
}

/// one descent level of the core: resolve `edges` — the `parent`
/// file's own declarations, `depth` levels below it — and recurse into
/// what `visit` declares. The visiting `chain` is the cycle guard
/// (pushed before a found child's descent, popped after); `done`
/// carries the paths already mounted (first mount wins on a repeated
/// declaration).
fn walk_edges(
    parent: &str,
    depth: usize,
    edges: Vec<(String, Vis)>,
    read: &mut dyn FnMut(&str, &str) -> Result<ChildLookup, String>,
    visit: &mut dyn FnMut(Edge<'_>) -> (Child, bool),
    chain: &mut Vec<String>,
    done: &mut Vec<String>,
) -> Result<(), String> {
    // the lookup's answer, narrowed to what the core does with it: the
    // found triple rides the edge, a refusal carries its why
    enum Resolved {
        Found {
            text: String,
            canon: String,
            file: String,
        },
        Refused(Refusal),
    }
    for (name, vis) in edges {
        let path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        if done.iter().any(|p| p == &path) {
            continue; // first mount wins (a repeated declaration)
        }
        let resolved = match read(parent, &name)? {
            ChildLookup::Found { text, canon, file } => Resolved::Found { text, canon, file },
            ChildLookup::Ambiguous => Resolved::Refused(Refusal::Ambiguous),
            ChildLookup::NoModRut => Resolved::Refused(Refusal::NoModRut),
            ChildLookup::Missing => Resolved::Refused(Refusal::Missing),
        };
        match resolved {
            Resolved::Found { text, canon, file } => {
                if chain.contains(&canon) {
                    return Err(format!(
                        "cyclic mod: `{name}` ({file}) is already being mounted"
                    ));
                }
                chain.push(canon.clone());
                let (child, keep) = visit(Edge::Found {
                    path: &path,
                    vis,
                    text,
                    canon,
                    file,
                    depth,
                });
                if !keep {
                    return Ok(()); // the visitor stopped the walk
                }
                if let Child::Declares(declared) = child {
                    walk_edges(&path, depth + 1, declared, read, visit, chain, done)?;
                }
                chain.pop();
                done.push(path);
            }
            Resolved::Refused(why) => {
                let (_, keep) = visit(Edge::Refused {
                    path: &path,
                    name: &name,
                    why,
                    depth,
                });
                if !keep {
                    return Ok(()); // the visitor stopped the walk
                }
            }
        }
    }
    Ok(())
}

/// the `mod` declarations of a parsed file, in source order
pub fn mod_edges_of(ast: &Ast) -> Vec<(String, Vis)> {
    let mut out = Vec::new();
    for it in ast.root().items(ast) {
        let ItemRef::ModDecl(h) = it else {
            continue;
        };
        let d = h.get(ast);
        out.push((ast.name(d.name.ident).to_string(), d.vis()));
    }
    out
}

/// Mount a file's mod tree: parse `root_text`, resolve every
/// `ModDecl` against `read(parent_mod_path, name)`, recursively —
/// the compile's STRICT DOOR over the pure core [`mount_edges`]: the
/// first refusal is the loud `Err`, naming the declaring file and the
/// spellings it probed. Cycle-guarded by canonical file identity: a
/// file already in the current mount chain is the loud `cyclic mod`
/// error (the dep walk's `cyclic use` mirrored); a repeated NAME
/// mounts once (first mount wins). The root's own identity
/// (`root_canon`) opens the chain — a child aliasing back onto the
/// root file is a cycle too.
///
/// The flavor is explicit: `mode` is the parse mode the tree speaks
/// ([`Mode::Impl`] for a lib body tree, [`Mode::Decl`] for a host
/// surface tree) and `child_file` the directory form's module file
/// name the lookup resolves against (`"mod.rut"` / `"mod.d.rut"`) —
/// the diagnostics name that spelling (the single-file form spells
/// itself). The ROOT is always the directory form (`mod.rut`/`mod.d.rut`
/// beside the manifest) — only child edges carry two spellings.
///
/// A file that fails to PARSE mounts with no children: the syntax
/// error is the compile's loud half, and decl-walking a broken file
/// would guess. Every lookup failure is the loud error naming the
/// spellings (the declaration and the file it expected).
pub fn mount_mod_children(
    root_text: &str,
    root_canon: &str,
    mode: Mode,
    child_file: &str,
    read: &mut dyn FnMut(&str, &str) -> Result<ChildLookup, String>,
) -> Result<BTreeMap<String, ModSource>, String> {
    let p = parse_with_recover(root_text.to_string(), mode);
    if !p.ast().is_clean() {
        return Ok(BTreeMap::new()); // the compile reports the syntax, loudly
    }
    let root_edges = mod_edges_of(p.ast());
    let mut out = BTreeMap::new();
    // the declaring file's RECORD, by mod path — the refusal messages
    // name it ([`rut_lir::check::display`]): the root speaks the
    // directory form, each cleanly parsed child its own root-relative
    // path
    let mut files: BTreeMap<String, String> =
        BTreeMap::from([(String::new(), child_file.to_string())]);
    let mut refusal: Option<String> = None;
    let mut visit = |e: Edge<'_>| -> (Child, bool) {
        match e {
            Edge::Found {
                path,
                vis,
                text,
                canon,
                file,
                depth: _,
            } => {
                // the root's own identity opens the chain — a child
                // aliasing back onto the root file is a cycle too (the
                // core's chain sees only the children's canons)
                if canon == root_canon {
                    let name = path.rsplit('/').next().unwrap_or(path);
                    refusal = Some(format!(
                        "cyclic mod: `{name}` ({file}) is already being mounted"
                    ));
                    return (Child::Childless, false);
                }
                // the child parses to its OWN module root — its
                // declarations are the next edges; a broken parse
                // mounts childless (the compile reports the syntax,
                // loudly)
                let cp = parse_with_recover(text.clone(), mode);
                let child = if cp.ast().is_clean() {
                    files.insert(path.to_string(), file.clone());
                    Child::Declares(mod_edges_of(cp.ast()))
                } else {
                    Child::Childless
                };
                out.insert(
                    path.to_string(),
                    ModSource {
                        path: path.to_string(),
                        vis,
                        text,
                        file,
                    },
                );
                (child, true)
            }
            Edge::Refused {
                path,
                name,
                why,
                depth: _,
            } => {
                // `path` is the refused edge's own (would-be) mount
                // path — the declaring file is the parent's
                let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
                let parent_file = files.get(parent).map(String::as_str);
                refusal = Some(match why {
                    Refusal::Ambiguous => format!(
                        "`mod {name};` in {} — both `{name}/{child_file}` and `{name}.rut` exist; \
                         the mount is ambiguous — keep one",
                        rut_lir::check::display(parent, parent_file),
                    ),
                    Refusal::NoModRut => format!(
                        "`mod {name};` in {} — `{name}/` has no `{child_file}`",
                        rut_lir::check::display(parent, parent_file),
                    ),
                    Refusal::Missing => format!(
                        "`mod {name};` in {} — no `{name}/{child_file}` beside it",
                        rut_lir::check::display(parent, parent_file),
                    ),
                });
                (Child::Childless, false) // the first refusal stops the mount
            }
        }
    };
    mount_edges(root_edges, read, &mut visit)?;
    match refusal {
        Some(msg) => Err(msg),
        None => Ok(out),
    }
}

// ---------------------------------------------------------------------
// the driver-side collection (the compile lane's plumbing)
// ---------------------------------------------------------------------

/// One collected declaration: the name, the visibility it was declared
/// with, the kind, the fn async marker, and — for a `pub use`
/// re-export row — the full path it re-exports (`use a::b::{ c }` →
/// `[a, b, c]`). The path is what makes the row an EDGE: law 5's walk
/// follows it to the declaring module (a row that is not a re-export
/// carries none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclRow {
    pub name: String,
    pub vis: Vis,
    pub kind: DeclKind,
    /// the decl's async marker (plan10 phase 3): the unbound-use law
    /// speaks the async gap only for an async row — a plain `pub async
    /// fn` crosses through its start wrapper now, so what still binds
    /// nothing is the generic shape
    pub is_async: bool,
    /// the re-exported path ([`DeclKind::Use`] only): the use's spelled
    /// head and interior segments, then the imported name
    pub path: Option<Vec<String>>,
}

/// The declaration kinds the collection records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind {
    Fn,
    Struct,
    Enum,
    Class,
    Interface,
    Alias,
    ModuleLet,
    /// a `mod NAME;` declaration (the mounted child's edge)
    Mod,
    /// a `pub use` re-export edge — the imported name joins the
    /// module's surface as a row carrying the use's declared vis AND
    /// the full path it re-exports (a private `use` imports only: no
    /// row, unchanged). The leaf gates read it like any decl; law 5's
    /// walk follows its [`DeclRow::path`] to the DECLARING module
    Use,
    /// a `host fn` surface row (a `type = "host"` pkg's `.d.rut`
    /// files) — the leaf a qualified use validates against
    HostFn,
}

/// One mounted file, parsed: its mod path (`""` = the pkg root), the
/// parsed document (whose root is the module — never spliced into the
/// parent's), and the declarations collected off it.
pub struct ModuleUnit {
    pub path: String,
    pub doc: Doc,
    pub decls: Vec<DeclRow>,
}

/// A package's mounted module set, parsed per file — the phase-2
/// plumbing the compile lanes see; phase 3 gates resolution and
/// visibility on it.
#[derive(Default)]
pub struct ModuleSet {
    /// the root unit first (`""`), then the children in mod-path order
    pub units: Vec<ModuleUnit>,
}

impl ModuleSet {
    /// The mounted mod paths, in unit order (the root's `""` first).
    pub fn paths(&self) -> Vec<&str> {
        self.units.iter().map(|u| u.path.as_str()).collect()
    }
}

/// Collect a pkg's module set: every mounted file parses to its own
/// root with its mod path attached, and the declarations are gathered.
/// Answers the set PLUS the loud problems (a declared child with no
/// mounted module, a mounted module nothing declares, a child that
/// fails to parse) — the caller folds each into its diagnostics. A
/// pkg with no mod children collects to the root unit alone.
///
/// The flavor rides the body: a source body speaks Impl and names
/// `mod.rut`; a host body (`type = "host"`) speaks Decl over its
/// retained surface text (`root_surface`) and names `mod.d.rut` — the
/// same files the loader mounted, re-parsed for their declaration
/// rows (the host rows ride as [`DeclKind::HostFn`], so a qualified
/// use validates against the path-carrying tree). A hand-built host
/// body with no retained surface (`core`, `Mounted::host`) collects no
/// root unit — its rows carry no mod tree to speak of.
pub fn collect_module_set(spec: &str, pkg: &Mounted) -> (ModuleSet, Vec<String>) {
    let mut problems: Vec<String> = Vec::new();
    let mut set = ModuleSet::default();
    // the flavor: a host body's tree speaks Decl and mounts
    // `NAME/mod.d.rut`; everything else is the lib shape
    let host = matches!(&pkg.body, MountedBody::Host { .. });
    let (mode, child_file) = if host {
        (Mode::Decl, crate::decl::DECL_CHILD_FILE)
    } else {
        (Mode::Impl, "mod.rut")
    };
    // the root unit: the body text (source bodies and host surfaces —
    // a compiled root's children still collect below, the root's AST
    // rides the binary)
    let root_declares: BTreeMap<String, Vis> = match &pkg.body {
        MountedBody::Source { text, is_decl } => {
            let m = if *is_decl { Mode::Decl } else { Mode::Impl };
            // the body stays mounted (the match borrows) — clone to move
            let p = parse_with_recover(text.to_string(), m);
            if !p.ast().is_clean() {
                problems.extend(
                    p.ast()
                        .errors()
                        .iter()
                        .map(|d| format!("`{spec}` (mod.rut): {}", d.msg)),
                );
                BTreeMap::new()
            } else {
                let decls = decl_rows(p.ast());
                let declared: BTreeMap<String, Vis> = decls
                    .iter()
                    .filter(|d| d.kind == DeclKind::Mod)
                    .map(|d| (d.name.clone(), d.vis))
                    .collect();
                set.units.push(ModuleUnit {
                    path: String::new(),
                    doc: p,
                    decls,
                });
                declared
            }
        }
        MountedBody::Host {
            root_surface: Some(text),
            ..
        } => {
            // the surface stays mounted (the match borrows) — clone to move
            let p = parse_with_recover(text.to_string(), Mode::Decl);
            if !p.ast().is_clean() {
                problems.extend(
                    p.ast()
                        .errors()
                        .iter()
                        .map(|d| format!("`{spec}` ({child_file}): {}", d.msg)),
                );
                BTreeMap::new()
            } else {
                let mut decls = decl_rows(p.ast());
                // the manifest consts ride the root's rows: they bind
                // BARE at the pkg's root (the surface grammar has no
                // `static` spelling), so the leaf gate reads them like
                // any declared name — `use calc_host::{ PI }` names a
                // row the pkg did declare
                if let MountedBody::Host { consts, .. } = &pkg.body {
                    for (name, _ty, _bits) in consts {
                        if !decls.iter().any(|d| &d.name == name) {
                            decls.push(DeclRow {
                                name: name.clone(),
                                vis: Vis::Pub,
                                kind: DeclKind::ModuleLet,
                                is_async: false,
                                path: None,
                            });
                        }
                    }
                }
                let declared: BTreeMap<String, Vis> = decls
                    .iter()
                    .filter(|d| d.kind == DeclKind::Mod)
                    .map(|d| (d.name.clone(), d.vis))
                    .collect();
                set.units.push(ModuleUnit {
                    path: String::new(),
                    doc: p,
                    decls,
                });
                declared
            }
        }
        _ => BTreeMap::new(),
    };
    // the children, mod-path order — each its own module root
    let mut child_decls: BTreeMap<String, BTreeMap<String, Vis>> = BTreeMap::new();
    for (path, m) in &pkg.mods {
        // the child file's text stays mounted (the map borrows) — clone
        // to move into the child parse
        let p = parse_with_recover(m.text.clone(), mode);
        if !p.ast().is_clean() {
            problems.extend(
                p.ast()
                    .errors()
                    .iter()
                    .map(|d| format!("`{spec}` ({}): {}", m.file, d.msg)),
            );
            child_decls.insert(path.clone(), BTreeMap::new());
            set.units.push(ModuleUnit {
                path: path.clone(),
                doc: p,
                decls: vec![],
            });
            continue;
        }
        let decls = decl_rows(p.ast());
        let declared: BTreeMap<String, Vis> = decls
            .iter()
            .filter(|d| d.kind == DeclKind::Mod)
            .map(|d| (d.name.clone(), d.vis))
            .collect();
        child_decls.insert(path.clone(), declared);
        set.units.push(ModuleUnit {
            path: path.clone(),
            doc: p,
            decls,
        });
    }
    // every DECLARED child must be mounted (the loader's law; a
    // hand-offered pkg that disagrees is named, never guessed around)
    // the declaring file's record: the mounted child's own path, the
    // root's flavor spelling
    let file_of = |path: &str| -> Option<&str> {
        pkg.mods
            .get(path)
            .map(|m| m.file.as_str())
            .or(if path.is_empty() {
                Some(child_file)
            } else {
                None
            })
    };
    let mut check_declared = |parent: &str, declared: &BTreeMap<String, Vis>| {
        for name in declared.keys() {
            let child = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}/{name}")
            };
            if !pkg.mods.contains_key(&child) {
                let file = rut_lir::check::display(parent, file_of(parent));
                problems.push(format!(
                    "`mod {name};` in `{spec}`'s {file} — no `{child}` module is mounted \
                     (the loader mounts `{child}/{child_file}` beside it)"
                ));
            }
        }
    };
    check_declared("", &root_declares);
    for (path, declared) in &child_decls {
        check_declared(path, declared);
    }
    // and every MOUNTED child must be declared — declarations are the
    // graph, never a directory listing
    for path in pkg.mods.keys() {
        let (parent, name) = match path.rsplit_once('/') {
            Some((p, n)) => (p.to_string(), n.to_string()),
            None => (String::new(), path.clone()),
        };
        let declared = if parent.is_empty() {
            &root_declares
        } else {
            match child_decls.get(&parent) {
                Some(d) => d,
                // the parent itself failed to parse / is missing — its
                // own problem is already reported above
                None => continue,
            }
        };
        if !declared.contains_key(&name) {
            let mounted = pkg.mods.get(path).map(|m| m.file.as_str());
            problems.push(format!(
                "`{spec}`'s `{}` is mounted but no `mod {name};` declares it — \
                 declarations are the graph: spell `mod {name};` in {}",
                rut_lir::check::display(path, mounted),
                rut_lir::check::display(&parent, file_of(&parent)),
            ));
        }
    }
    (set, problems)
}

/// The declaration rows off one parsed file. `pub(crate)`: the graph
/// compiler's dep tables run it per module — a module's rows are the
/// qualified-use leaf gate's reads. A `pub use`'s imported names ride
/// as [`DeclKind::Use`] re-export rows — the use's declared vis and
/// the full re-exported path on each, so the row is an EDGE and law
/// 5's walk can land it on the declaring module; a private `use`
/// contributes nothing.
pub(crate) fn decl_rows(ast: &Ast) -> Vec<DeclRow> {
    let mut out = Vec::new();
    for it in ast.root().items(ast) {
        // a `pub use` re-export EDGE, not a private import: the
        // imported names join the module's surface as rows carrying
        // the use's declared vis — the leaf gates read them like any
        // decl. A private `use` stays what it was: an import, no row.
        if let ItemRef::Use(h) = it {
            let d = h.get(ast);
            if d.vis() != Vis::Self_ {
                // the row NAMES its target: the use's spelled path
                // (head + interior) then the imported name
                let path: Vec<String> = d
                    .path
                    .iter()
                    .map(|n| ast.name(n.ident).to_string())
                    .collect();
                for n in &d.names {
                    let name = ast.name(n.ident).to_string();
                    let mut row_path = path.clone();
                    row_path.push(name.clone());
                    out.push(DeclRow {
                        name,
                        vis: d.vis(),
                        kind: DeclKind::Use,
                        is_async: false,
                        path: Some(row_path),
                    });
                }
            }
            continue;
        }
        let (name, vis, kind, is_async) = match it {
            ItemRef::Fn(h) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::Fn, d.is_async())
            }
            ItemRef::Alias(h) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::Alias, false)
            }
            ItemRef::ModuleLet(h) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::ModuleLet, false)
            }
            ItemRef::Enum(h) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::Enum, false)
            }
            ItemRef::Struct(h) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::Struct, false)
            }
            ItemRef::Class(h) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::Class, false)
            }
            ItemRef::Interface(h) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::Interface, false)
            }
            ItemRef::ModDecl(h) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::Mod, false)
            }
            // a `host fn` surface row — the host tree's leaf decl: the
            // declaring `pub` rides it, so a qualified use validates
            // (`pub host fn` crosses, a bare `host fn` is private)
            ItemRef::SurfaceFn(h) if matches!(h.get(ast).linkage(), Linkage::Host) => {
                let d = h.get(ast);
                (d.name.ident, d.vis(), DeclKind::HostFn, d.is_async())
            }
            _ => continue,
        };
        out.push(DeclRow {
            name: ast.name(name).to_string(),
            vis,
            kind,
            is_async,
            path: None,
        });
    }
    out
}

// ---------------------------------------------------------------------
// the `rut.mods` rows codec (the additive envelope section)
// ---------------------------------------------------------------------

/// Serialize a package's module rows: one JSON object, mod path →
/// source text, `""` = the pkg's own source (the assembled root).
/// Key order is sorted (serde's BTreeMap-backed object) — same tree ⇒
/// same bytes, the pack determinism law.
pub fn rows_json(root_text: &str, mods: &BTreeMap<String, ModSource>) -> String {
    let mut map = serde_json::Map::new();
    map.insert(
        String::new(),
        serde_json::Value::String(root_text.to_string()),
    );
    for (path, m) in mods {
        map.insert(path.clone(), serde_json::Value::String(m.text.clone()));
    }
    serde_json::Value::Object(map).to_string()
}

/// Parse a rows entry: the root text (`""`) plus the child rows keyed
/// by mod path. Malformed keys and shapes are loud, naming the entry.
pub fn parse_rows(text: &str) -> Result<(String, BTreeMap<String, String>), String> {
    let bad = |msg: String| format!("{ROWS_NAME}: {msg}");
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| bad(e.to_string()))?;
    let obj = value
        .as_object()
        .ok_or_else(|| bad("expected a JSON object of module rows — `\"\": \"<root source>\"` plus one row per child".to_string()))?;
    let mut root = None;
    let mut rows = BTreeMap::new();
    for (k, v) in obj {
        let text = v
            .as_str()
            .ok_or_else(|| bad(format!("`{k}` — expected the module's source text")))?;
        if k.is_empty() {
            root = Some(text.to_string());
            continue;
        }
        if !valid_mod_path(k) {
            return Err(bad(format!(
                "`{k}` is not a module path — bare `[a-zA-Z0-9_]+` segments joined with `/`"
            )));
        }
        rows.insert(k.clone(), text.to_string());
    }
    let root = root.ok_or_else(|| {
        bad(
            "no `\"\"` row — the root module's source is missing (re-pack the directory)"
                .to_string(),
        )
    })?;
    Ok((root, rows))
}

/// A mod path: bare `[a-zA-Z0-9_]+` segments joined with `/` — the
/// same charset package names speak, per segment.
fn valid_mod_path(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(crate::bundle::valid_spec)
}

/// Mount the mod tree a rows set carries: the same declaration-driven
/// core the FS lane runs, over the row keys (a missing row is the
/// missing-file error; rows cannot alias, so the cycle guard keys on
/// the path itself). The flavor rides the caller — a lib body tree
/// speaks Impl and names `mod.rut`, a host surface tree speaks Decl
/// and names `mod.d.rut`. A row NOTHING declares is refused —
/// declarations are the graph, even inside a bundle.
pub fn mount_rows(
    root_text: &str,
    rows: &BTreeMap<String, String>,
    mode: Mode,
    child_file: &str,
) -> Result<BTreeMap<String, ModSource>, String> {
    let mut read = |parent: &str, name: &str| -> Result<ChildLookup, String> {
        let child = if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}/{name}")
        };
        match rows.get(&child) {
            // rows are keyed by LOGICAL mod path — the physical path is
            // unknowable here, so the directory form's record stands in
            // (bundles are directory-form)
            Some(text) => Ok(ChildLookup::Found {
                text: text.clone(),
                canon: child.clone(),
                file: format!("{child}/{child_file}"),
            }),
            None => Ok(ChildLookup::Missing),
        }
    };
    let mounted = mount_mod_children(root_text, "\u{0}rows-root", mode, child_file, &mut read)?;
    if mounted.len() != rows.len() {
        for path in rows.keys() {
            if !mounted.contains_key(path) {
                let (parent, name) = match path.rsplit_once('/') {
                    Some((p, n)) => (p.to_string(), n.to_string()),
                    None => (String::new(), path.clone()),
                };
                let mounted = mounted.get(&parent).map(|m| m.file.as_str());
                let parent_file = mounted.or(if parent.is_empty() {
                    Some(child_file)
                } else {
                    None
                });
                return Err(format!(
                    "{ROWS_NAME}: `{path}/{child_file}` is carried but no `mod {name};` declares it — \
                     declarations are the graph: spell `mod {name};` in {} (re-pack the directory)",
                    rut_lir::check::display(&parent, parent_file),
                ));
            }
        }
    }
    Ok(mounted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake FS: mod path → the file text, with canonical identities
    /// aliased through `aliases` (mod path → canon) to simulate
    /// symlinks. Everything else answers Missing.
    struct FakeFs {
        files: BTreeMap<String, String>,
        aliases: BTreeMap<String, String>,
    }

    impl FakeFs {
        fn lookup(&mut self, parent: &str, name: &str) -> Result<ChildLookup, String> {
            self.lookup_as(parent, name, "mod.rut")
        }

        /// the flavor-aware twin: `file` is the directory form's file
        /// name the found child reports (`mod.rut`, the Decl test's
        /// `mod.d.rut`)
        fn lookup_as(
            &mut self,
            parent: &str,
            name: &str,
            file: &str,
        ) -> Result<ChildLookup, String> {
            let child = if parent.is_empty() {
                name.to_string()
            } else {
                format!("{parent}/{name}")
            };
            match self.files.get(&child) {
                Some(text) => {
                    let canon = self
                        .aliases
                        .get(&child)
                        .cloned()
                        .unwrap_or_else(|| child.clone());
                    Ok(ChildLookup::Found {
                        text: text.clone(),
                        canon,
                        file: format!("{child}/{file}"),
                    })
                }
                None => Ok(ChildLookup::Missing),
            }
        }

        /// the single-file spelling: the file's record is the sibling
        /// `NAME.rut` (the map keyed by the logical mod path)
        fn lookup_file_form(&mut self, parent: &str, name: &str) -> Result<ChildLookup, String> {
            let dir = if parent.is_empty() {
                name.to_string()
            } else {
                format!("{parent}/{name}")
            };
            match self.files.get(&dir) {
                Some(text) => Ok(ChildLookup::Found {
                    text: text.clone(),
                    canon: dir.clone(),
                    file: format!("{dir}.rut"),
                }),
                None => Ok(ChildLookup::Missing),
            }
        }
    }

    #[test]
    fn mounts_two_levels_in_declaration_order() {
        let mut fs = FakeFs {
            files: BTreeMap::from([
                (
                    "layout".into(),
                    "pub mod grid;\npub fn span() -> i32 { return 1; }\n".into(),
                ),
                (
                    "layout/grid".into(),
                    "pub fn cell() -> i32 { return 2; }\n".into(),
                ),
            ]),
            aliases: BTreeMap::new(),
        };
        let mods = mount_mod_children(
            "mod layout;\nentry fn main() {}\n",
            "root",
            Mode::Impl,
            "mod.rut",
            &mut |p, n| fs.lookup(p, n),
        )
        .unwrap();
        assert_eq!(
            mods.keys().collect::<Vec<_>>(),
            vec!["layout", "layout/grid"]
        );
        assert_eq!(mods["layout"].vis, Vis::Self_);
        assert_eq!(mods["layout/grid"].vis, Vis::Pub);
        assert!(mods["layout"].text.starts_with("pub mod grid;"));
    }

    #[test]
    fn missing_child_names_both_spellings() {
        let mut fs = FakeFs {
            files: BTreeMap::new(),
            aliases: BTreeMap::new(),
        };
        let err = mount_mod_children("mod grid;", "root", Mode::Impl, "mod.rut", &mut |p, n| {
            fs.lookup(p, n)
        })
        .unwrap_err();
        assert_eq!(err, "`mod grid;` in mod.rut — no `grid/mod.rut` beside it");
        // a nested declaration names the declaring file
        let mut fs = FakeFs {
            files: BTreeMap::from([("layout".into(), "mod grid;\n".into())]),
            aliases: BTreeMap::new(),
        };
        let err = mount_mod_children("mod layout;", "root", Mode::Impl, "mod.rut", &mut |p, n| {
            fs.lookup(p, n)
        })
        .unwrap_err();
        assert_eq!(
            err,
            "`mod grid;` in layout/mod.rut — no `grid/mod.rut` beside it"
        );
    }

    #[test]
    fn a_file_child_mounts_and_the_diagnostics_spell_its_name() {
        // the single-file spelling: `mod imp;` mounts `imp.rut` — and
        // a declaration INSIDE it names `imp.rut` as the declaring
        // file, never the directory-form guess
        let mut fs = FakeFs {
            files: BTreeMap::from([
                (
                    "imp".into(),
                    "pub fn one() -> i32 { return 1; }\nmod deep;\n".into(),
                ),
                (
                    "imp/deep".into(),
                    "pub fn two() -> i32 { return 2; }\n".into(),
                ),
            ]),
            aliases: BTreeMap::new(),
        };
        let mods = mount_mod_children(
            "mod imp;\nentry fn main() {}\n",
            "root",
            Mode::Impl,
            "mod.rut",
            &mut |p, n| fs.lookup_file_form(p, n),
        )
        .unwrap();
        assert_eq!(mods.keys().collect::<Vec<_>>(), vec!["imp", "imp/deep"]);
        // the missing-child error inside the file child names ITS file
        // (the found child's root-relative record)
        let mut fs = FakeFs {
            files: BTreeMap::from([("imp".into(), "mod gone;\n".into())]),
            aliases: BTreeMap::new(),
        };
        let err = mount_mod_children("mod imp;", "root", Mode::Impl, "mod.rut", &mut |p, n| {
            fs.lookup_file_form(p, n)
        })
        .unwrap_err();
        assert_eq!(err, "`mod gone;` in imp.rut — no `gone/mod.rut` beside it");
    }

    #[test]
    fn both_spellings_are_the_loud_ambiguity() {
        let mut fs = FakeFs {
            files: BTreeMap::new(),
            aliases: BTreeMap::new(),
        };
        let err = mount_mod_children("mod imp;", "root", Mode::Impl, "mod.rut", &mut |p, n| {
            if p.is_empty() {
                Ok(ChildLookup::Ambiguous)
            } else {
                fs.lookup(p, n)
            }
        })
        .unwrap_err();
        assert_eq!(
            err,
            "`mod imp;` in mod.rut — both `imp/mod.rut` and `imp.rut` exist; \
             the mount is ambiguous — keep one"
        );
        // a nested declaration names the declaring file (here the
        // single-file child's own spelling)
        let err = mount_mod_children("mod layout;", "root", Mode::Impl, "mod.rut", &mut |p, n| {
            if p == "layout" {
                Ok(ChildLookup::Ambiguous)
            } else if p.is_empty() {
                Ok(ChildLookup::Found {
                    text: "mod grid;\n".into(),
                    canon: "layout".into(),
                    file: "layout.rut".to_string(),
                })
            } else {
                fs.lookup(p, n)
            }
        })
        .unwrap_err();
        assert_eq!(
            err,
            "`mod grid;` in layout.rut — both `grid/mod.rut` and `grid.rut` exist; \
             the mount is ambiguous — keep one"
        );
    }

    #[test]
    fn cyclic_mount_names_the_alias() {
        // a/mod.rut declares `mod b;`; the b file aliases a's file —
        // the canonical identity repeats inside the chain: the loud cycle
        let mut fs = FakeFs {
            files: BTreeMap::from([
                ("a".into(), "mod b;\n".into()),
                ("a/b".into(), "mod b;\n".into()),
            ]),
            aliases: BTreeMap::from([("a/b".into(), "a".into())]),
        };
        let err = mount_mod_children("mod a;", "root", Mode::Impl, "mod.rut", &mut |p, n| {
            fs.lookup(p, n)
        })
        .unwrap_err();
        assert_eq!(
            err,
            "cyclic mod: `b` (a/b/mod.rut) is already being mounted"
        );
    }

    #[test]
    fn first_mount_wins_on_a_repeated_declaration() {
        let mut fs = FakeFs {
            files: BTreeMap::from([("a".into(), "pub fn one() -> i32 { return 1; }\n".into())]),
            aliases: BTreeMap::new(),
        };
        let mods = mount_mod_children(
            "mod a;\nmod a;\n",
            "root",
            Mode::Impl,
            "mod.rut",
            &mut |p, n| fs.lookup(p, n),
        )
        .unwrap();
        assert_eq!(mods.len(), 1, "the second `mod a;` mounts nothing new");
    }

    #[test]
    fn a_broken_file_mounts_childless_and_the_compile_reports_it() {
        // the child's syntax error is the compile's loud half; the
        // mount itself stays silent (decl-walking a broken file would
        // guess)
        let mut fs = FakeFs {
            files: BTreeMap::from([("a".into(), "fn broken( {{".into())]),
            aliases: BTreeMap::new(),
        };
        let mods = mount_mod_children("mod a;", "root", Mode::Impl, "mod.rut", &mut |p, n| {
            fs.lookup(p, n)
        })
        .unwrap();
        assert_eq!(mods.len(), 1);
    }

    #[test]
    fn decl_flavor_mounts_mod_d_rut_and_names_it() {
        // the host surface tree: the same core over the Decl flavor —
        // `NAME/mod.d.rut` is the module file, and every diagnostic
        // names that spelling
        let mut fs = FakeFs {
            files: BTreeMap::from([
                (
                    "net".into(),
                    "pub mod tls;\npub host fn connect(url: str) -> opaque;\n".into(),
                ),
                (
                    "net/tls".into(),
                    "pub host fn handshake(c: opaque) -> opaque;\n".into(),
                ),
            ]),
            aliases: BTreeMap::new(),
        };
        let mods = mount_mod_children(
            "pub mod net;\npub host fn legacy_ping() -> bool;\n",
            "root",
            Mode::Decl,
            "mod.d.rut",
            &mut |p, n| fs.lookup_as(p, n, "mod.d.rut"),
        )
        .unwrap();
        assert_eq!(mods.keys().collect::<Vec<_>>(), vec!["net", "net/tls"]);
        assert_eq!(mods["net"].vis, Vis::Pub);
        assert!(mods["net/tls"].text.contains("handshake"));
        // the missing-child error names `mod.d.rut`, the declaring file included
        let mut fs = FakeFs {
            files: BTreeMap::new(),
            aliases: BTreeMap::new(),
        };
        let err = mount_mod_children(
            "pub mod net;\n",
            "root",
            Mode::Decl,
            "mod.d.rut",
            &mut |p, n| fs.lookup_as(p, n, "mod.d.rut"),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "`mod net;` in mod.d.rut — no `net/mod.d.rut` beside it"
        );
        // a nested declaration names the declaring child file
        let mut fs = FakeFs {
            files: BTreeMap::from([("net".into(), "pub mod tls;\n".into())]),
            aliases: BTreeMap::new(),
        };
        let err = mount_mod_children(
            "pub mod net;\n",
            "root",
            Mode::Decl,
            "mod.d.rut",
            &mut |p, n| fs.lookup_as(p, n, "mod.d.rut"),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "`mod tls;` in net/mod.d.rut — no `tls/mod.d.rut` beside it"
        );
    }

    #[test]
    fn rows_round_trip_and_laws() {
        let mut mods = BTreeMap::new();
        mods.insert(
            "layout".to_string(),
            ModSource {
                path: String::new(),
                vis: Vis::Self_,
                text: "pub fn span() -> i32 { return 1; }\n".into(),
                file: "layout/mod.rut".into(),
            },
        );
        let json = rows_json("mod layout;\n", &mods);
        let (root, rows) = parse_rows(&json).unwrap();
        assert_eq!(root, "mod layout;\n");
        assert_eq!(rows.keys().collect::<Vec<_>>(), vec!["layout"]);
        let mounted = mount_rows(&root, &rows, Mode::Impl, "mod.rut").unwrap();
        assert_eq!(mounted["layout"].text, mods["layout"].text);

        // no root row: loud
        let err = parse_rows(r#"{"layout": "x"}"#).unwrap_err();
        assert!(err.contains("no `\"\"` row"), "{err}");
        // a bad path key: loud
        let err = parse_rows(r#"{"": "x", "a/b/": "y"}"#).unwrap_err();
        assert!(err.contains("is not a module path"), "{err}");
        // a non-string row: loud
        let err = parse_rows(r#"{"": "x", "a": 3}"#).unwrap_err();
        assert!(err.contains("expected the module's source text"), "{err}");
        // a carried-but-undeclared row: loud (declarations are the graph)
        let (_, rows) = parse_rows(r#"{"": "mod a;\n", "a": "y", "b": "z"}"#).unwrap();
        let err = mount_rows("mod a;\n", &rows, Mode::Impl, "mod.rut").unwrap_err();
        assert!(err.contains("no `mod b;` declares it"), "{err}");
        // the Decl flavor names `mod.d.rut` in the undeclared-row error
        let (_, rows) = parse_rows(r#"{"": "pub mod net;\n", "net": "y", "tls": "z"}"#).unwrap();
        let err = mount_rows("pub mod net;\n", &rows, Mode::Decl, "mod.d.rut").unwrap_err();
        assert!(
            err.contains("`tls/mod.d.rut` is carried but no `mod tls;` declares it"),
            "{err}"
        );
        // and the Decl mount itself rides the tree
        let (_, rows) =
            parse_rows(r#"{"": "pub mod net;\n", "net": "pub host fn c() -> opaque;\n"}"#).unwrap();
        let mounted = mount_rows("pub mod net;\n", &rows, Mode::Decl, "mod.d.rut").unwrap();
        assert_eq!(mounted.keys().collect::<Vec<_>>(), vec!["net"]);
    }

    /// A host-body pkg from a root surface + mounted tree — the shape
    /// `lower_decl_module` yields (the dep tables read it as found).
    fn host_pkg(root: &str, children: &[(&str, Vis, &str)]) -> Mounted {
        let mut mods = BTreeMap::new();
        for (path, vis, text) in children {
            mods.insert(
                path.to_string(),
                ModSource {
                    path: path.to_string(),
                    vis: *vis,
                    text: text.to_string(),
                    file: format!("{path}/mod.d.rut"),
                },
            );
        }
        Mounted {
            spec: "myhost".to_string(),
            body: crate::session::MountedBody::Host {
                host_funcs: vec![],
                consts: vec![],
                native_types: vec![],
                native_fns: vec![],
                native_impls: vec![],
                root_surface: Some(root.to_string()),
            },
            mods,
            ..Default::default()
        }
    }

    #[test]
    fn a_host_tree_collects_its_decl_rows() {
        // the phase-2 collection: the root surface parses Decl, the
        // children parse Decl (`mod.d.rut` files), and the `host fn`
        // rows ride the per-module decl rows as `HostFn` — the
        // qualified-use leaf gate's input
        let pkg = host_pkg(
            "pub mod net;\npub host fn legacy_ping() -> bool;\n",
            &[
                (
                    "net",
                    Vis::Pub,
                    "pub mod tls;\npub host fn connect(url: str) -> opaque;\n",
                ),
                (
                    "net/tls",
                    Vis::Pub,
                    "host fn private_probe();\npub host fn handshake(c: opaque) -> opaque;\n",
                ),
            ],
        );
        let (set, problems) = collect_module_set("myhost", &pkg);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(set.paths(), vec!["", "net", "net/tls"]);
        // the root: the mod edge + the flat row
        let root = &set.units[0].decls;
        assert_eq!(root[0].name, "net");
        assert_eq!(root[0].kind, DeclKind::Mod);
        assert_eq!(root[0].vis, Vis::Pub);
        assert_eq!(root[1].name, "legacy_ping");
        assert_eq!(root[1].kind, DeclKind::HostFn);
        assert_eq!(root[1].vis, Vis::Pub, "`pub host fn` carries its pub");
        // the net child: its edge + its row
        let net = &set.units[1].decls;
        assert_eq!(net[0].kind, DeclKind::Mod);
        assert_eq!(net[1].name, "connect");
        assert_eq!(net[1].kind, DeclKind::HostFn);
        assert_eq!(net[1].vis, Vis::Pub);
        // net/tls: a bare `host fn` keeps its privacy — the leaf gate
        // reads `vis` off the row
        let tls = &set.units[2].decls;
        assert_eq!(tls[0].name, "private_probe");
        assert_eq!(tls[0].kind, DeclKind::HostFn);
        assert_eq!(tls[0].vis, Vis::Self_, "a bare `host fn` is private");
        assert_eq!(tls[1].name, "handshake");
        assert_eq!(tls[1].vis, Vis::Pub);
    }

    #[test]
    fn the_host_tree_checks_speak_the_mod_d_rut_spelling() {
        // the collection's loud half names `mod.d.rut` on the Decl
        // flavor — the files the errors point at are the ones that
        // exist
        let pkg = host_pkg("pub mod net;\n", &[]);
        let (_, problems) = collect_module_set("myhost", &pkg);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`mod net;` in `myhost`'s mod.d.rut")
                    && p.contains("the loader mounts `net/mod.d.rut` beside it")),
            "{problems:?}"
        );
        // a mounted child nothing declares — the reverse law, same flavor
        let pkg = host_pkg(
            "pub host fn legacy_ping() -> bool;\n",
            &[("net", Vis::Pub, "")],
        );
        let (_, problems) = collect_module_set("myhost", &pkg);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`net/mod.d.rut` is mounted but no `mod net;` declares it")),
            "{problems:?}"
        );
        // a child that fails to parse names its `.d.rut` file
        let pkg = host_pkg(
            "pub mod net;\n",
            &[("net", Vis::Pub, "pub host fn broken( {{")],
        );
        let (_, problems) = collect_module_set("myhost", &pkg);
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("`myhost` (net/mod.d.rut):")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_source_tree_collects_exactly_as_before() {
        // the lib shape is untouched by the host arm: Impl children,
        // `mod.rut` spellings, no host rows
        let mut pkg = Mounted::source("app", "mod layout;\nentry fn main() {}\n");
        pkg.mods.insert(
            "layout".to_string(),
            ModSource {
                path: "layout".into(),
                vis: Vis::Pub,
                text: "pub fn span() -> i32 { return 1; }\n".into(),
                file: "layout/mod.rut".into(),
            },
        );
        let (set, problems) = collect_module_set("app", &pkg);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(set.paths(), vec!["", "layout"]);
        assert!(set.units[1]
            .decls
            .iter()
            .any(|d| d.kind == DeclKind::Fn && d.name == "span"));
    }

    #[test]
    fn a_pub_use_joins_the_module_surface_a_private_use_does_not() {
        // the re-export law: a `pub use`'s imported names ride the
        // module's decl rows (the use's declared vis on each — the leaf
        // gates read them like any decl), while a private `use` stays
        // an import: no row, nothing crosses
        let pkg = Mounted::source(
            "mid",
            "pub use util::{ helper, Widget };\n\
             use util::{ hidden };\n\
             pub(pkg) use util::{ scoped };\n",
        );
        let (set, problems) = collect_module_set("mid", &pkg);
        assert!(problems.is_empty(), "{problems:?}");
        let rows = &set.units[0].decls;
        let row = |name: &str| rows.iter().find(|d| d.name == name);
        assert_eq!(
            row("helper").map(|d| (d.kind, d.vis)),
            Some((DeclKind::Use, Vis::Pub)),
            "a `pub use` leaf is a pub re-export row"
        );
        assert_eq!(
            row("Widget").map(|d| (d.kind, d.vis)),
            Some((DeclKind::Use, Vis::Pub)),
            "every leaf of the one `pub use` rides"
        );
        assert_eq!(
            row("scoped").map(|d| (d.kind, d.vis)),
            Some((DeclKind::Use, Vis::Pkg)),
            "the scoped spelling carries its own vis — the gates read it"
        );
        assert!(
            row("hidden").is_none(),
            "a private `use` imports only: no row"
        );
    }
}
