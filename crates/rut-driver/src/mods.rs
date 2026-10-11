//! File modules — the `mod` mount. Declarations are the graph: a
//! package's module set is the file tree its `mod` declarations name
//! (`NAME/mod.rut` beside the declaring file — or `NAME/mod.d.rut`
//! for a host surface tree), never a directory listing. This module is
//! the PURE half of that law:
//!
//! - [`mount_mod_children`] — the mounting core. Parses a file, walks
//!   its `ModDecl` items, and resolves each against a caller-supplied
//!   child lookup (the FS lane in rut-native, the archive rows lane in
//!   the loader), recursively, cycle-guarded by the canonical identity
//!   of each mounted file (the dep walk's law mirrored: the visiting
//!   chain is the cycle guard, first mount wins on a repeated name).
//!   The flavor rides the caller: a lib body tree speaks Impl and
//!   mounts `NAME/mod.rut`; a Decl surface tree (a `type = "host"`
//!   pkg's `.d.rut` set) speaks Decl and mounts `NAME/mod.d.rut`.
//!   Missing file / a same-named FILE sibling / a dir without its
//!   module file are each loud errors naming both spellings.
//! - [`collect_module_set`] — the driver-side collection: every
//!   mounted file parses to its own `ItemKind::Module` root (children
//!   are NEVER spliced into the parent's AST) and its declarations are
//!   collected with the mod path attached — the plumbing phase 3
//!   (resolution/visibility) gates on.
//! - the `rut.mods` rows codec — the bundle envelope's additive
//!   section: a package's module set as path-keyed source rows
//!   (`""` = root, `"layout"` = child). A package with no mod children
//!   writes no rows entry (byte-identical pack, the freshness gate
//!   proves it); the reader accepts the old flat envelope unchanged.
//!
//! No fs, no net, no `Path` — the crate purity law. The FS probes live
//! in rut-native's walk; the archive reads in the loader.

use std::collections::BTreeMap;

use rut_ast::ast::{Ast, ItemKind, Linkage, Vis};
use rut_parser::{parse, Mode};

use crate::session::{Pkg, PkgBody};

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
}

/// What the FS/archive lane answers for one `mod` declaration's child
/// lookup. The four cases ARE the diagnostics: found (with the file's
/// canonical identity for the cycle guard), a same-named FILE sibling
/// (not a module directory), a directory without its module file, and
/// nothing there at all. The module file's spelling is the caller's
/// (`mod.rut` for a lib body tree, `mod.d.rut` for a host surface
/// tree) — the lookup probes that one name.
pub enum ChildLookup {
    /// `NAME/<child file>` read; `canon` is the file's canonical
    /// identity (the cycle guard's key — a symlinked alias repeats it)
    Found { text: String, canon: String },
    /// a same-named FILE sibling exists — `NAME.rut`, not a module dir
    NotADir,
    /// the directory exists but carries no module file
    NoModRut,
    /// nothing there under either spelling
    Missing,
}

/// Mount a file's mod tree: parse `root_text`, resolve every
/// `ModDecl` against `read(parent_mod_path, name)`, recursively.
/// Cycle-guarded by canonical file identity: a file already in the
/// current mount chain is the loud `cyclic mod` error (the dep walk's
/// `cyclic use` mirrored); a repeated NAME mounts once (first mount
/// wins). The root's own identity (`root_canon`) opens the chain — a
/// child aliasing back onto the root file is a cycle too.
///
/// The flavor is explicit: `mode` is the parse mode the tree speaks
/// ([`Mode::Impl`] for a lib body tree, [`Mode::Decl`] for a host
/// surface tree) and `child_file` the module file's name the lookup
/// resolved against (`"mod.rut"` / `"mod.d.rut"`) — the diagnostics
/// name that spelling.
///
/// A file that fails to PARSE mounts with no children: the syntax
/// error is the compile's loud half, and decl-walking a broken file
/// would guess. Every lookup failure is the loud error naming both
/// spellings (the declaration and the file it expected).
pub fn mount_mod_children(
    root_text: &str,
    root_canon: &str,
    mode: Mode,
    child_file: &str,
    read: &mut dyn FnMut(&str, &str) -> Result<ChildLookup, String>,
) -> Result<BTreeMap<String, ModSource>, String> {
    let (ast, diags) = parse(root_text, mode);
    if !diags.is_empty() {
        return Ok(BTreeMap::new()); // the compile reports the syntax, loudly
    }
    let mut out = BTreeMap::new();
    let mut visiting = vec![root_canon.to_string()];
    mount_rec("", &ast, mode, child_file, read, &mut out, &mut visiting)?;
    Ok(out)
}

/// The declaring file's display path, mod-path relative: the root is
/// the module file, a child `<parent>/<child file>` — the exact
/// spelling the diagnostics name.
fn file_display(parent: &str, child_file: &str) -> String {
    if parent.is_empty() {
        child_file.to_string()
    } else {
        format!("{parent}/{child_file}")
    }
}

fn mount_rec(
    parent: &str,
    ast: &Ast,
    mode: Mode,
    child_file: &str,
    read: &mut dyn FnMut(&str, &str) -> Result<ChildLookup, String>,
    out: &mut BTreeMap<String, ModSource>,
    visiting: &mut Vec<String>,
) -> Result<(), String> {
    for it in ast.module_items(ast.root).to_vec() {
        let ItemKind::ModDecl { vis, name } = ast.item(it) else {
            continue;
        };
        let name = ast.name(*name).to_string();
        let child_path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        if out.contains_key(&child_path) {
            continue; // first mount wins (a repeated declaration)
        }
        match read(parent, &name)? {
            ChildLookup::Found { text, canon } => {
                if visiting.contains(&canon) {
                    return Err(format!(
                        "cyclic mod: `{name}` ({child_path}/{child_file}) is already being mounted"
                    ));
                }
                visiting.push(canon);
                // the child parses to its OWN module root — recurse
                // into its declarations, never splice it here
                let (child_ast, diags) = parse(&text, mode);
                if diags.is_empty() {
                    mount_rec(&child_path, &child_ast, mode, child_file, read, out, visiting)?;
                }
                visiting.pop();
                out.insert(
                    child_path.clone(),
                    ModSource { path: child_path, vis: *vis, text },
                );
            }
            ChildLookup::NotADir => {
                return Err(format!(
                    "`mod {name};` in {} — `{name}.rut` is a file, not a module directory \
                     (expected `{name}/{child_file}`)",
                    file_display(parent, child_file),
                ));
            }
            ChildLookup::NoModRut => {
                return Err(format!(
                    "`mod {name};` in {} — `{name}/` has no `{child_file}`",
                    file_display(parent, child_file),
                ));
            }
            ChildLookup::Missing => {
                return Err(format!(
                    "`mod {name};` in {} — no `{name}/{child_file}` beside it",
                    file_display(parent, child_file),
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// the driver-side collection (the compile lane's plumbing)
// ---------------------------------------------------------------------

/// One collected declaration: the name, the visibility it was declared
/// with, and the kind — the flat per-module decl rows phase 3 keys
/// resolution and visibility on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclRow {
    pub name: String,
    pub vis: Vis,
    pub kind: DeclKind,
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
    /// a `host fn` surface row (a `type = "host"` pkg's `.d.rut`
    /// files) — the leaf a qualified use validates against
    HostFn,
}

/// One mounted file, parsed: its mod path (`""` = the pkg root), the
/// AST (whose root is `ItemKind::Module` — never spliced into the
/// parent's), and the declarations collected off it.
pub struct ModuleUnit {
    pub path: String,
    pub ast: Ast,
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
/// body with no retained surface (`core`, `Pkg::host`) collects no
/// root unit — its rows carry no mod tree to speak of.
pub fn collect_module_set(spec: &str, pkg: &Pkg) -> (ModuleSet, Vec<String>) {
    let mut problems: Vec<String> = Vec::new();
    let mut set = ModuleSet::default();
    // the flavor: a host body's tree speaks Decl and mounts
    // `NAME/mod.d.rut`; everything else is the lib shape
    let host = matches!(&pkg.body, PkgBody::Host { .. });
    let (mode, child_file) = if host {
        (Mode::Decl, crate::decl::DECL_CHILD_FILE)
    } else {
        (Mode::Impl, "mod.rut")
    };
    // the root unit: the body text (source bodies and host surfaces —
    // a compiled root's children still collect below, the root's AST
    // rides the binary)
    let root_declares: BTreeMap<String, Vis> = match &pkg.body {
        PkgBody::Source { text, is_decl } => {
            let m = if *is_decl { Mode::Decl } else { Mode::Impl };
            let (ast, diags) = parse(text, m);
            if !diags.is_empty() {
                problems.extend(diags.iter().map(|d| format!("`{spec}` (mod.rut): {}", d.msg)));
                BTreeMap::new()
            } else {
                let decls = decl_rows(&ast);
                let declared: BTreeMap<String, Vis> = decls
                    .iter()
                    .filter(|d| d.kind == DeclKind::Mod)
                    .map(|d| (d.name.clone(), d.vis))
                    .collect();
                set.units.push(ModuleUnit { path: String::new(), ast, decls });
                declared
            }
        }
        PkgBody::Host { root_surface: Some(text), .. } => {
            let (ast, diags) = parse(text, Mode::Decl);
            if !diags.is_empty() {
                problems.extend(diags.iter().map(|d| format!("`{spec}` ({child_file}): {}", d.msg)));
                BTreeMap::new()
            } else {
                let decls = decl_rows(&ast);
                let declared: BTreeMap<String, Vis> = decls
                    .iter()
                    .filter(|d| d.kind == DeclKind::Mod)
                    .map(|d| (d.name.clone(), d.vis))
                    .collect();
                set.units.push(ModuleUnit { path: String::new(), ast, decls });
                declared
            }
        }
        _ => BTreeMap::new(),
    };
    // the children, mod-path order — each its own module root
    let mut child_decls: BTreeMap<String, BTreeMap<String, Vis>> = BTreeMap::new();
    for (path, m) in &pkg.mods {
        let (ast, diags) = parse(&m.text, mode);
        if !diags.is_empty() {
            problems.extend(diags.iter().map(|d| format!("`{spec}` ({path}/{child_file}): {}", d.msg)));
            child_decls.insert(path.clone(), BTreeMap::new());
            set.units.push(ModuleUnit { path: path.clone(), ast, decls: vec![] });
            continue;
        }
        let decls = decl_rows(&ast);
        let declared: BTreeMap<String, Vis> = decls
            .iter()
            .filter(|d| d.kind == DeclKind::Mod)
            .map(|d| (d.name.clone(), d.vis))
            .collect();
        child_decls.insert(path.clone(), declared);
        set.units.push(ModuleUnit { path: path.clone(), ast, decls });
    }
    // every DECLARED child must be mounted (the loader's law; a
    // hand-offered pkg that disagrees is named, never guessed around)
    let mut check_declared = |parent: &str, declared: &BTreeMap<String, Vis>| {
        for (name, _) in declared {
            let child = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}/{name}")
            };
            if !pkg.mods.contains_key(&child) {
                let file = file_display(parent, child_file);
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
            problems.push(format!(
                "`{spec}`'s `{path}/{child_file}` is mounted but no `mod {name};` declares it — \
                 declarations are the graph: spell `mod {name};` in {}",
                file_display(&parent, child_file),
            ));
        }
    }
    (set, problems)
}

/// The declaration rows off one parsed file.
fn decl_rows(ast: &Ast) -> Vec<DeclRow> {
    let mut out = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        let (name, vis, kind) = match ast.item(it) {
            ItemKind::Fn(d) => (d.name, d.vis, DeclKind::Fn),
            ItemKind::Alias(d) => (d.name, d.vis, DeclKind::Alias),
            ItemKind::ModuleLet { vis, name, .. } => (*name, *vis, DeclKind::ModuleLet),
            ItemKind::Enum { vis, name, .. } => (*name, *vis, DeclKind::Enum),
            ItemKind::Struct { vis, name, .. } => (*name, *vis, DeclKind::Struct),
            ItemKind::Class { vis, name, .. } => (*name, *vis, DeclKind::Class),
            ItemKind::Interface { vis, name, .. } => (*name, *vis, DeclKind::Interface),
            ItemKind::ModDecl { vis, name } => (*name, *vis, DeclKind::Mod),
            // a `host fn` surface row — the host tree's leaf decl: the
            // declaring `pub` rides it, so a qualified use validates
            // (`pub host fn` crosses, a bare `host fn` is private)
            ItemKind::SurfaceFn { linkage: Linkage::Host, vis, name, .. } => {
                (*name, *vis, DeclKind::HostFn)
            }
            _ => continue,
        };
        out.push(DeclRow { name: ast.name(name).to_string(), vis, kind });
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
    map.insert(String::new(), serde_json::Value::String(root_text.to_string()));
    for (path, m) in mods {
        map.insert(path.clone(), serde_json::Value::String(m.text.clone()));
    }
    serde_json::Value::Object(map).to_string()
}

/// Parse a rows entry: the root text (`""`) plus the child rows keyed
/// by mod path. Malformed keys and shapes are loud, naming the entry.
pub fn parse_rows(text: &str) -> Result<(String, BTreeMap<String, String>), String> {
    let bad = |msg: String| format!("{ROWS_NAME}: {msg}");
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| bad(e.to_string()))?;
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
        bad("no `\"\"` row — the root module's source is missing (re-pack the directory)".to_string())
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
            Some(text) => Ok(ChildLookup::Found { text: text.clone(), canon: child }),
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
                return Err(format!(
                    "{ROWS_NAME}: `{path}/{child_file}` is carried but no `mod {name};` declares it — \
                     declarations are the graph: spell `mod {name};` in {} (re-pack the directory)",
                    file_display(&parent, child_file),
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
            let child = if parent.is_empty() {
                name.to_string()
            } else {
                format!("{parent}/{name}")
            };
            match self.files.get(&child) {
                Some(text) => {
                    let canon = self.aliases.get(&child).cloned().unwrap_or_else(|| child.clone());
                    Ok(ChildLookup::Found { text: text.clone(), canon })
                }
                None => Ok(ChildLookup::Missing),
            }
        }
    }

    #[test]
    fn mounts_two_levels_in_declaration_order() {
        let mut fs = FakeFs {
            files: BTreeMap::from([
                ("layout".into(), "pub mod grid;\npub fn span() -> i32 { return 1; }\n".into()),
                ("layout/grid".into(), "pub fn cell() -> i32 { return 2; }\n".into()),
            ]),
            aliases: BTreeMap::new(),
        };
        let mods = mount_mod_children("mod layout;\nentry fn main() {}\n", "root", Mode::Impl, "mod.rut", &mut |p, n| {
            fs.lookup(p, n)
        })
        .unwrap();
        assert_eq!(mods.keys().collect::<Vec<_>>(), vec!["layout", "layout/grid"]);
        assert_eq!(mods["layout"].vis, Vis::Self_);
        assert_eq!(mods["layout/grid"].vis, Vis::Pub);
        assert!(mods["layout"].text.starts_with("pub mod grid;"));
    }

    #[test]
    fn missing_child_names_both_spellings() {
        let mut fs = FakeFs { files: BTreeMap::new(), aliases: BTreeMap::new() };
        let err = mount_mod_children("mod grid;", "root", Mode::Impl, "mod.rut", &mut |p, n| fs.lookup(p, n))
            .unwrap_err();
        assert_eq!(
            err,
            "`mod grid;` in mod.rut — no `grid/mod.rut` beside it"
        );
        // a nested declaration names the declaring file
        let mut fs = FakeFs {
            files: BTreeMap::from([("layout".into(), "mod grid;\n".into())]),
            aliases: BTreeMap::new(),
        };
        let err = mount_mod_children("mod layout;", "root", Mode::Impl, "mod.rut", &mut |p, n| fs.lookup(p, n))
            .unwrap_err();
        assert_eq!(
            err,
            "`mod grid;` in layout/mod.rut — no `grid/mod.rut` beside it"
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
        let err = mount_mod_children("mod a;", "root", Mode::Impl, "mod.rut", &mut |p, n| fs.lookup(p, n))
            .unwrap_err();
        assert_eq!(err, "cyclic mod: `b` (a/b/mod.rut) is already being mounted");
    }

    #[test]
    fn first_mount_wins_on_a_repeated_declaration() {
        let mut fs = FakeFs {
            files: BTreeMap::from([("a".into(), "pub fn one() -> i32 { return 1; }\n".into())]),
            aliases: BTreeMap::new(),
        };
        let mods = mount_mod_children("mod a;\nmod a;\n", "root", Mode::Impl, "mod.rut", &mut |p, n| fs.lookup(p, n))
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
        let mods = mount_mod_children("mod a;", "root", Mode::Impl, "mod.rut", &mut |p, n| fs.lookup(p, n))
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
                ("net".into(), "pub mod tls;\npub host fn connect(url: str) -> opaque;\n".into()),
                ("net/tls".into(), "pub host fn handshake(c: opaque) -> opaque;\n".into()),
            ]),
            aliases: BTreeMap::new(),
        };
        let mods = mount_mod_children(
            "pub mod net;\npub host fn legacy_ping() -> bool;\n",
            "root",
            Mode::Decl,
            "mod.d.rut",
            &mut |p, n| fs.lookup(p, n),
        )
        .unwrap();
        assert_eq!(mods.keys().collect::<Vec<_>>(), vec!["net", "net/tls"]);
        assert_eq!(mods["net"].vis, Vis::Pub);
        assert!(mods["net/tls"].text.contains("handshake"));
        // the missing-child error names `mod.d.rut`, the declaring file included
        let mut fs = FakeFs { files: BTreeMap::new(), aliases: BTreeMap::new() };
        let err = mount_mod_children(
            "pub mod net;\n",
            "root",
            Mode::Decl,
            "mod.d.rut",
            &mut |p, n| fs.lookup(p, n),
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
            &mut |p, n| fs.lookup(p, n),
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
            ModSource { path: String::new(), vis: Vis::Self_, text: "pub fn span() -> i32 { return 1; }\n".into() },
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
        assert!(err.contains("`tls/mod.d.rut` is carried but no `mod tls;` declares it"), "{err}");
        // and the Decl mount itself rides the tree
        let (_, rows) = parse_rows(r#"{"": "pub mod net;\n", "net": "pub host fn c() -> opaque;\n"}"#).unwrap();
        let mounted = mount_rows("pub mod net;\n", &rows, Mode::Decl, "mod.d.rut").unwrap();
        assert_eq!(mounted.keys().collect::<Vec<_>>(), vec!["net"]);
    }

    /// A host-body pkg from a root surface + mounted tree — the shape
    /// `lower_decl_module` yields (the dep tables read it as found).
    fn host_pkg(root: &str, children: &[(&str, Vis, &str)]) -> Pkg {
        let mut mods = BTreeMap::new();
        for (path, vis, text) in children {
            mods.insert(
                path.to_string(),
                ModSource { path: path.to_string(), vis: *vis, text: text.to_string() },
            );
        }
        Pkg {
            spec: "myhost".to_string(),
            body: crate::session::PkgBody::Host {
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
                ("net", Vis::Pub, "pub mod tls;\npub host fn connect(url: str) -> opaque;\n"),
                ("net/tls", Vis::Pub, "host fn private_probe();\npub host fn handshake(c: opaque) -> opaque;\n"),
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
            problems.iter().any(|p| p.contains("`mod net;` in `myhost`'s mod.d.rut")
                && p.contains("the loader mounts `net/mod.d.rut` beside it")),
            "{problems:?}"
        );
        // a mounted child nothing declares — the reverse law, same flavor
        let pkg = host_pkg("pub host fn legacy_ping() -> bool;\n", &[("net", Vis::Pub, "")]);
        let (_, problems) = collect_module_set("myhost", &pkg);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`net/mod.d.rut` is mounted but no `mod net;` declares it")),
            "{problems:?}"
        );
        // a child that fails to parse names its `.d.rut` file
        let pkg = host_pkg("pub mod net;\n", &[("net", Vis::Pub, "pub host fn broken( {{")]);
        let (_, problems) = collect_module_set("myhost", &pkg);
        assert!(
            problems.iter().any(|p| p.starts_with("`myhost` (net/mod.d.rut):")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_source_tree_collects_exactly_as_before() {
        // the lib shape is untouched by the host arm: Impl children,
        // `mod.rut` spellings, no host rows
        let mut pkg = Pkg::source(
            "app",
            "mod layout;\nentry fn main() {}\n",
        );
        pkg.mods.insert(
            "layout".to_string(),
            ModSource {
                path: "layout".into(),
                vis: Vis::Pub,
                text: "pub fn span() -> i32 { return 1; }\n".into(),
            },
        );
        let (set, problems) = collect_module_set("app", &pkg);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(set.paths(), vec!["", "layout"]);
        assert!(set.units[1].decls.iter().any(|d| d.kind == DeclKind::Fn && d.name == "span"));
    }
}
