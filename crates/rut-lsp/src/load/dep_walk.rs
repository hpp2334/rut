//! The read-only dep walk — the extension's manifest walk
//! (`integrations/vscode-extension/src/deps.ts`) moved INTO the crate,
//! reading only through the injected host environment
//! ([`crate::env::Io`]): pure reads, one fast handler turn, no walk
//! task, no awaits (the actor seam's law — the walk is a sync body
//! over `&mut Session` exactly like every other handler). The mod
//! tree mounts through rut-driver's ONE mount core (`mount_edges`) —
//! the compile's own law, no second walker. The mod-name completion
//! slot's disk probe (the module files beside the current file, the
//! env capability's second pure read) lives in
//! `crate::feature::completion::mod_name`.
//!
//! What the port keeps, law for law:
//!
//! - **the lock feed rides the walk root** — `<root>/rut.lock` parses
//!   BEFORE the root manifest, so its url rows carry the lock's pins.
//!   A PRESENT-but-malformed lock is the one hint (and the empty
//!   ledger stands — the honest unpinned refusal); a MISSING lock
//!   clears the previous root's ledger silently (multi-root
//!   precedence: root order decides).
//! - **entries + mod trees index through the ABI's own lanes** — the
//!   [`Session::index_named_at`] / [`Session::index_mod_at`] bodies the
//!   `index_mod_at` handler runs, so a walked dep
//!   resolves in hover/completion/definition exactly like a
//!   host-indexed one (one store, replace-by-origin, no drift). The
//!   mod-tree recursion follows the wasm-parsed decls (the
//!   `index_mod_at` reply), `NAME/mod.rut` (or `NAME/mod.d.rut` in a
//!   host surface) then `NAME.rut` spellings, beside the declaring
//!   file.
//! - **url rows mount from the GLOBAL DEP STORE** — the address is
//!   DERIVED through rut-driver's helpers (`<name>@<sha256[:16]>`
//!   under `$RUT_HOME/deps` — the same address `rut install`
//!   publishes to, never a second table). Dir present AND its
//!   `meta.json` sha256 == the lock pin ⇒ the `unpack/` tree mounts
//!   through the SAME dir lanes (its manifest entries, mod tree,
//!   embedded dep groups — go-to-definition lands in real files under
//!   the store, cargo-registry-style). Absent or meta-mismatch ⇒ ONE
//!   hint per dep naming `rut install`; NEVER fetched, NEVER written
//!   — the store owns extraction, drift healing, and eviction. An
//!   unpinned row hints and refuses, never mounts.
//! - **budgets** — `MAX_DEP_DEPTH` / `MAX_DEP_FILES` (the extension
//!   constants, mirrored): depth caps the manifest recursion, files
//!   cap everything the walk indexes, per root. First-mount-wins
//!   `mounted` spans the whole resolve (a diamond's pins cannot
//!   double-stamp); `dev-deps` walk at the root only; the root
//!   manifest's own names accumulate into the direct-dep set.
//! - **no stale sweep, no evict** — the extension's `.rut/deps/`
//!   sweep is GONE by design: the walk never wrote those trees, so it
//!   never deletes one.
//!
//! Paths join with `/` only — the host environment normalizes
//! (`std::path` never enters; a path row may spell `..`, and the env
//! it reads through resolves it).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};

use rut_ast::Vis;
use rut_driver::mods::{mount_edges, Child, ChildLookup, Edge, Refusal};

use crate::doc::{mode_of, Document, Mode};
use crate::env::Io;
use crate::load::manifest::{DepRow, DepSource};
use crate::session::Session;

/// the manifest recursion cap — the extension's `MAX_DEP_DEPTH`
/// mirrored (a guest, not an indexer daemon)
pub(crate) const MAX_DEP_DEPTH: usize = 8;

/// the per-root file budget — the extension's `MAX_DEP_FILES`
/// mirrored: everything the walk indexes (entry files, mod-tree
/// files) decrements it; a dead budget truncates the root
pub(crate) const MAX_DEP_FILES: usize = 200;

/// one url row the walk refused — the informational hint row the
/// host's answer carries (the `rut install` recipe itself rides the
/// notify lane, once per reason).
#[derive(Debug, Clone)]
pub(crate) struct MissingDep {
    pub url: String,
    pub sha256: Option<String>,
    pub cache_path: String,
}

/// the walk's whole answer: the refused url rows. Mounted rows need
/// no row — the world they mounted into IS the answer.
#[derive(Debug, Clone, Default)]
pub(crate) struct WalkOut {
    pub missing: Vec<MissingDep>,
}

/// the per-root walk state. `seen` (manifest dirs — the cycle break)
/// and the file budget reset per root (the extension's per-folder
/// law); `mounted` and `missing` span the whole resolve. A dead
/// budget TRUNCATES: every mounting site checks the caps first and
/// simply stops — the referenced set stays incomplete, never a
/// failure (the extension's `truncated` flag fed the stale sweep this
/// walk does not have; the caps themselves are the law that remains).
struct WalkCtx<'a> {
    io: &'a dyn Io,
    /// manifest dirs this root resolved — the cycle break
    seen: HashSet<String>,
    /// mounted module names — first mount wins, across every root of
    /// the resolve (the native gate: a name an earlier row or
    /// embedded group mounted never re-mounts, so a diamond's pins
    /// cannot double-stamp)
    mounted: &'a mut HashSet<String>,
    /// remaining dep-file budget
    files: usize,
    /// the refused url rows, accumulating across roots
    missing: &'a mut Vec<MissingDep>,
}

/// the walk's entry: every root, in order — feed its lock, then walk
/// its manifest. One fast handler turn; the state ends in the walked
/// world (the ledger of the LAST root fed, the defs replace-by-origin,
/// the direct-dep set accumulated).
pub(crate) fn resolve(state: &mut Session, io: &dyn Io, roots: &[String]) -> WalkOut {
    let mut out = WalkOut::default();
    let mut mounted: HashSet<String> = HashSet::new();
    for root in roots {
        feed_lock(state, io, root);
        let mut ctx = WalkCtx {
            io,
            seen: HashSet::new(),
            mounted: &mut mounted,
            files: MAX_DEP_FILES,
            missing: &mut out.missing,
        };
        walk_manifest(&mut ctx, state, root, 1);
    }
    out
}

/// one root's lock feed: `<root>/rut.lock` parses into the ledger the
/// root manifest's url rows read. PRESENT-but-malformed is the one
/// error hint (and the empty ledger stands — never half-parsed pins);
/// a MISSING lock hands over empty bytes — clearing the previous
/// root's ledger silently (multi-root precedence: root order decides,
/// and the miss lane is expected, not a hint).
fn feed_lock(state: &mut Session, io: &dyn Io, root: &str) {
    let path = format!("{root}/rut.lock");
    match io.read(&path) {
        None => {
            let _ = state.set_lock_text("");
        }
        Some(bytes) => {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if let Err(e) = state.set_lock_text(&text) {
                state.hint_once(io, &format!("lock:{path}"), &format!("rut: {path} — {e}"));
            }
        }
    }
}

/// one manifest: its own entry sources index under the module name
/// (origin = the file's rooted path, so go-to-definition lands in it),
/// its mod tree mounts behind the parsed decls, then its dep rows walk
/// — `deps` at any depth, `dev-deps` only at the root (the loader's
/// law: a dep's dev table never enters a consumer's world). ONE
/// grammar everywhere: a store tree's manifest parses like any
/// directory's.
fn walk_manifest(ctx: &mut WalkCtx, state: &mut Session, dir: &str, depth: usize) {
    // the caps truncate: the referenced set stays incomplete, never a
    // failure
    if depth > MAX_DEP_DEPTH || ctx.files == 0 {
        return;
    }
    if ctx.seen.contains(dir) {
        return;
    }
    ctx.seen.insert(dir.to_string());
    let Some(bytes) = ctx.io.read(&format!("{dir}/rut.jsonc")) else {
        return; // no manifest here — nothing to parse, nothing to hint
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let table = match crate::load::manifest::dep_table_pinned(&text, &state.lock_pins) {
        Ok(t) => t,
        Err(e) => {
            state.hint_once(
                ctx.io,
                &format!("manifest:{dir}"),
                &format!("rut: rut.jsonc — {e}"),
            );
            return;
        }
    };
    let module = table.name.clone().unwrap_or_else(|| basename(dir));
    // the ROOT manifest's own rows are the doc's DIRECT deps — the
    // compiler's closure check refuses `use` of a mounted-but-
    // undeclared package, so the auto-import tier gates on this set
    // (reads it LIVE at query time — no ordering against the mounts,
    // the gate is structural). Names accumulate across roots and
    // re-walks, never shrink the world.
    if depth == 1 {
        let mut names: Vec<String> = table.deps.iter().map(|r| r.name.clone()).collect();
        names.extend(table.dev_deps.iter().map(|r| r.name.clone()));
        if let Some(n) = &table.name {
            names.push(n.clone());
        }
        for n in names {
            state.direct_deps.insert(n);
        }
    }
    // first mount wins — a module name an earlier row (or root, or
    // embedded group) mounted never re-mounts
    if ctx.mounted.contains(&module) {
        return;
    }
    ctx.mounted.insert(module.clone());
    for entry in &table.entries {
        if ctx.files == 0 {
            return; // the budget died — the set stays incomplete
        }
        let path = join(dir, &entry.path);
        if let Some(bytes) = ctx.io.read(&path) {
            ctx.files -= 1;
            let src = String::from_utf8_lossy(&bytes).into_owned();
            state.index_named_at(&path, &module, src);
        } // a manifest naming a missing entry — index what exists
    }
    // the mod tree (the pkg's own modules): the root module stamps at
    // mod path `""` (the entries loop's flat stamp for the same file
    // REPLACES with it — same origin), then each `mod NAME;` edge
    // mounts beside the root file, recursively, depth- and
    // file-budget-capped. The flavor names the child file: a lib's
    // root is its first Impl entry (`src/mod.rut`, children probe
    // `NAME/mod.rut` then `NAME.rut`), a host pkg's root is its DECL
    // entry (the `.d.rut` surface, children probe `NAME/mod.d.rut`
    // then `NAME.rut`) — both spellings for every flavor, the
    // compiler's own probe. An entry-less dev state probes the lib
    // convention.
    let entry_rows: Vec<(&str, crate::doc::Mode)> = table
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.mode))
        .collect();
    mount_mod_tree(ctx, state, dir, &module, &entry_rows, depth);
    for row in &table.deps {
        mount_row(ctx, state, dir, row, depth);
    }
    if depth == 1 {
        for row in &table.dev_deps {
            mount_row(ctx, state, dir, row, depth);
        }
    }
}

/// one dep row's mount door: url rows go through the store lane, path
/// rows recurse into the directory beside the manifest (OUTSIDE the
/// walk root is the point).
fn mount_row(ctx: &mut WalkCtx, state: &mut Session, dir: &str, row: &DepRow, depth: usize) {
    if ctx.files == 0 {
        return; // the budget died — the set stays incomplete
    }
    match &row.source {
        DepSource::Url { .. } => mount_url_dep(ctx, state, row, depth),
        DepSource::Path { dir: rel } => {
            let dep_dir = join(dir, rel);
            if ctx.io.read(&format!("{dep_dir}/rut.jsonc")).is_none() {
                state.hint_once(
                    ctx.io,
                    &format!("path:{dir}:{}", row.name),
                    &format!(
                        "rut: dep `{}` — {rel} has no rut.jsonc; a path dep is a module \
                         directory (one manifest per module)",
                        row.name
                    ),
                );
                return;
            }
            walk_manifest(ctx, state, &dep_dir, depth + 1);
        }
    }
}

/// one package's mod tree through rut-driver's ONE mount core
/// ([`mount_edges`]): the root module stamps at mod path `""` (the
/// entries loop's flat stamp for the same file REPLACES with it —
/// same origin), then every `mod NAME;` edge resolves through `read`
/// — EVERY flavor probes the directory form `NAME/<child file>`
/// (`mod.rut` for a lib tree, `mod.d.rut` for a host surface) beside
/// the declaring file first, then the single-file spelling
/// `NAME.rut`; BOTH present is the loader's loud ambiguity (hinted
/// once, neither mounts). `visit` charges the file budget, indexes the read file at
/// the canon origin, and answers with the file's own `mod` edges —
/// a file that fails to parse mounts CHILDLESS (the driver's law),
/// and a file-form child's children mount beside it, not under
/// `NAME/`. Depth- and file-budget-capped: a spent cap stops the
/// package's walk (the set stays incomplete, never a failure).
/// `entries` carries the manifest's own rows: a lib's first IMPL
/// entry is its root (`src/mod.rut`), else a HOST pkg's DECL entry is
/// (the `.d.rut` surface — its tree mounts with the Decl flavor, the
/// loader's own flavor law), else the lib convention is probed (a
/// surface-only dev state).
fn mount_mod_tree(
    ctx: &mut WalkCtx,
    state: &mut Session,
    pkg_dir: &str,
    module: &str,
    entries: &[(&str, crate::doc::Mode)],
    depth: usize,
) {
    if depth > MAX_DEP_DEPTH || ctx.files == 0 {
        return; // the caps truncate — the set stays incomplete
    }
    let (root_path, decl_flavor, charged) = if let Some((first, _)) =
        entries.iter().find(|(_, m)| *m == crate::doc::Mode::Impl)
    {
        // the root-module lane: `src/mod.rut` is the root (the entries
        // loop already indexed and charged it — this re-stamp at mod
        // path "" is the same answer, replace-by-origin)
        (join(pkg_dir, first), false, false)
    } else if let Some((first, _)) = entries.iter().find(|(_, m)| *m == crate::doc::Mode::Decl) {
        // the HOST lane: the DECL entry is the root module (indexed
        // flat by the entries loop — this re-stamp carries its own
        // `mod` edges), and the tree mounts Decl-flavored
        (join(pkg_dir, first), true, false)
    } else {
        // a surface-only dev state: no entry rows — `src/mod.rut`
        // under the shell is probed anyway (the ROOT is always the
        // directory form — only child edges carry the `NAME.rut`
        // spelling); this lane's read is its own charge
        (format!("{pkg_dir}/src/mod.rut"), false, true)
    };
    let Some(bytes) = ctx.io.read(&root_path) else {
        return; // no root module — nothing mounts
    };
    if charged {
        ctx.files -= 1;
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    // THE MOUNT'S MODE — the tree's flavor, one decision for the root
    // and every child below (the CLI's two mounts: `walk.rs`'s lib
    // tree is Impl, its host surface tree Decl). Everything this tree
    // mounts parses and stamps with it.
    let tree_mode = if decl_flavor {
        Mode::Decl
    } else {
        mode_of(&root_path)
    };
    // the root re-stamp REPLACES the entries loop's row (same origin) —
    // the fresh read moves into the parse; the reply's `mod` edges
    // open the mount
    let root_edges = mod_edges(state.index_mod_at(&root_path, module, "", text, tree_mode));
    // the children mount BESIDE THE ROOT FILE — inside the package's
    // `src/` shell. The map both closures share: mod path → the
    // directory its edges declare in (the found file's own directory
    // for a directory-form child, the declaring directory for a
    // single-file one — the `child_dir` rule).
    let root_dir = dirname(&root_path);
    let child_file = if decl_flavor {
        rut_driver::decl::DECL_CHILD_FILE
    } else {
        "mod.rut"
    };
    let dirs = RefCell::new(BTreeMap::from([(String::new(), root_dir.clone())]));
    // the child's display record: its found file's path relative to
    // the PACKAGE ROOT FILE's directory (`imp.rut`, `m/a/b.rut`,
    // `imp/mod.rut`) — the display law's fact, never the flavor guess
    // (`mod.rut`) or the bare name at a deeper path
    let root_prefix = format!("{root_dir}/");
    let record = |canon: &str| -> String {
        canon
            .strip_prefix(&root_prefix)
            .unwrap_or(canon)
            .to_string()
    };
    let io = ctx.io; // the env — a shared handle, copied out of the ctx
    let mut read = |parent: &str, name: &str| -> Result<ChildLookup, String> {
        let dir = dirs
            .borrow()
            .get(parent)
            .expect("the declaring directory is mapped")
            .clone();
        let dir_form = format!("{dir}/{name}/{child_file}");
        let file_form = format!("{dir}/{name}.rut");
        let dir_bytes = io.read(&dir_form);
        // BOTH child spellings, EVERY flavor — the compiler's own probe
        // (`rut_native::walk::dir_child_lookup`): a host tree's
        // `NAME.rut` beside the declaring surface is the child the book
        // spells, exactly as a lib tree's is. Both present is the loud
        // ambiguity below (the compiler's answer too).
        let file_bytes = io.read(&file_form);
        Ok(match (dir_bytes, file_bytes) {
            (Some(_), Some(_)) => ChildLookup::Ambiguous,
            (Some(b), None) => ChildLookup::Found {
                text: String::from_utf8_lossy(&b).into_owned(),
                file: record(&dir_form),
                canon: dir_form,
            },
            (None, Some(b)) => {
                let file = record(&file_form);
                ChildLookup::Found {
                    text: String::from_utf8_lossy(&b).into_owned(),
                    canon: file_form,
                    file,
                }
            }
            (None, None) => ChildLookup::Missing,
        })
    };
    let mut visit = |e: Edge<'_>| -> (Child, bool) {
        match e {
            Edge::Found {
                path,
                vis: _,
                text,
                canon,
                file,
                depth: edge_depth,
            } => {
                // the caps truncate: a spent depth or a dead budget
                // stops this package's walk — the child never mounts
                if edge_depth + depth >= MAX_DEP_DEPTH || ctx.files == 0 {
                    return (Child::Childless, false);
                }
                ctx.files -= 1;
                // ONE DECISION, BOTH READS: the mount's mode is the
                // tree's flavor — a host tree's single-file child is
                // Decl syntax whatever its filename says — and the
                // clean-parse and the stamp below ride it together
                let mode = if decl_flavor {
                    Mode::Decl
                } else {
                    mode_of(&canon)
                };
                // the driver's law: a child that fails to parse mounts
                // CHILDLESS — its `mod` edges never walk (the same
                // parse the mount below runs)
                let mod_path = path.to_string();
                let clean = Document::parse(mode, text.clone()).ast().is_clean();
                let decls = state.index_mod_at(&canon, module, &mod_path, text, mode);
                if !clean {
                    return (Child::Childless, true);
                }
                // the child's declaring directory is the record's own
                // parent — a directory-form child mounts under `NAME/`,
                // a single-file child beside its declaring file (the
                // `child_dir` rule, read off the record the lookup
                // carried)
                let (parent, _) = match path.rsplit_once('/') {
                    Some((p, n)) => (p, n),
                    None => ("", path),
                };
                let parent_dir = dirs
                    .borrow()
                    .get(parent)
                    .expect("the declaring directory is mapped")
                    .clone();
                let child_dir = match file.rsplit_once('/') {
                    Some((dir, _)) => format!("{root_dir}/{dir}"),
                    None => parent_dir,
                };
                dirs.borrow_mut().insert(mod_path, child_dir);
                (Child::Declares(mod_edges(decls)), true)
            }
            Edge::Refused {
                path,
                name,
                why,
                depth: _,
            } => {
                // BOTH spellings present — the loader's loud ambiguity,
                // hinted once, neither mounts; any other refusal mounts
                // nothing and the walk continues silently
                if let Refusal::Ambiguous = why {
                    let (parent, _) = match path.rsplit_once('/') {
                        Some((p, n)) => (p, n),
                        None => ("", path),
                    };
                    let parent_dir = dirs
                        .borrow()
                        .get(parent)
                        .expect("the declaring directory is mapped")
                        .clone();
                    let file_form = format!("{parent_dir}/{name}.rut");
                    state.hint_once(
                        io,
                        &format!("mods:{file_form}"),
                        &format!(
                            "rut: {module} — both {name}/{child_file} and {name}.rut exist; keep one"
                        ),
                    );
                }
                (Child::Childless, true)
            }
        }
    };
    // the core's Err — a cyclic mount (the chain repeats a canonical
    // identity) or a failing read — ends this package's walk with ONE
    // hint naming the reason
    if let Err(e) = mount_edges(root_edges, &mut read, &mut visit) {
        state.hint_once(
            io,
            &format!("mods:{root_path}"),
            &format!("rut: {module} — {e}"),
        );
    }
}

/// the index door's reply — the pushed file's own `mod` declarations,
/// as the mount core's edge rows (the name, the declared vis)
fn mod_edges(decls: Vec<crate::actor::wire::ModDecl>) -> Vec<(String, Vis)> {
    decls
        .into_iter()
        .map(|d| (d.name, if d.vis == "pub" { Vis::Pub } else { Vis::Self_ }))
        .collect()
}

/// one url row — the store's mount door. The address is DERIVED
/// (rut-driver's `store_dir` + `deps_dir`: `<name>@<sha256[:16]>`
/// under `$RUT_HOME/deps` — the same address `rut install` publishes
/// to, never re-derived here). The door: dir present AND its
/// `meta.json` vouches the FULL sha256 == the lock pin (a reader never
/// re-hashes the artifact — install-time verification is the only
/// hash pass) ⇒ the `unpack/` tree mounts through the SAME dir lanes
/// [`walk_manifest`] runs — its manifest entries, its mod tree, its
/// embedded `deps/<identity>/` groups (path rows beside the manifest
/// resolve into them). Anything else is ONE hint per dep naming
/// `rut install`; never fetched, never written, never evicted — the
/// store owns all of that.
fn mount_url_dep(ctx: &mut WalkCtx, state: &mut Session, row: &DepRow, depth: usize) {
    let DepSource::Url {
        url,
        sha256,
        cache_path,
    } = &row.source
    else {
        return;
    };
    let Some(pin) = sha256 else {
        state.hint_once(
            ctx.io,
            &format!("unpinned:{url}"),
            &format!(
                "rut: dep `{}` has no sha256 pin in rut.lock — the editor never mounts \
                 unpinned url bytes; run `rut install` in the project to (re)pin, then \
                 reload the window",
                row.name
            ),
        );
        ctx.missing.push(MissingDep {
            url: url.clone(),
            sha256: None,
            cache_path: cache_path.clone(),
        });
        return;
    };
    let dir = format!(
        "{}/{}",
        rut_driver::deps_dir(),
        rut_driver::store_dir(&row.name, pin)
    );
    if !ctx.io.is_dir(&dir) {
        state.hint_once(
            ctx.io,
            &format!("store:{url}"),
            &format!(
                "rut: dep `{}` ({url}) is not in the dep store — run `rut install` in the \
                 project, then reload the window",
                row.name
            ),
        );
        ctx.missing.push(MissingDep {
            url: url.clone(),
            sha256: Some(pin.clone()),
            cache_path: cache_path.clone(),
        });
        return;
    }
    // the verification surface: the dir's meta.json carries the FULL
    // sha256 the dir name's 16-hex suffix abbreviates. Absent,
    // malformed, or vouching a different pin — same refusal, one hint.
    let verified = ctx
        .io
        .read(&format!("{dir}/{}", rut_driver::META_NAME))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .and_then(|t| rut_driver::parse_store_meta(&t).ok())
        .is_some_and(|m| m.sha256 == pin.to_ascii_lowercase());
    if !verified {
        state.hint_once(
            ctx.io,
            &format!("meta:{url}"),
            &format!(
                "rut: dep `{}` — the store entry at {dir} does not verify against the \
                 rut.lock pin; run `rut install` in the project, then reload the window",
                row.name
            ),
        );
        ctx.missing.push(MissingDep {
            url: url.clone(),
            sha256: Some(pin.clone()),
            cache_path: cache_path.clone(),
        });
        return;
    }
    // hit: the unpacked tree is a package directory like any other —
    // the SAME dir lanes mount it (and its embedded groups through its
    // own path rows). ONE grammar at the store door too — the unpacked
    // manifest parses like any directory's.
    walk_manifest(
        ctx,
        state,
        &format!("{dir}/{}", rut_driver::UNPACK_DIR),
        depth + 1,
    );
}

// ---- the rooted-string path helpers (join with `/` only — `std::path`
// never enters; the host resolves reads) ----

/// `<dir>/<rel>`, `<seg>/..` collapsed lexically (an absolute `rel`
/// wins, mirroring `path.resolve`) — ONE spelling per file, so the
/// origin a signature or a diagnostic names is the file's true path.
/// A `..` pops the last segment only when it is a real name; a leading
/// `..` stays (a path above the base is legal input the host's reads
/// resolve), and nothing here touches the fs.
fn join(dir: &str, rel: &str) -> String {
    if rel.starts_with('/') {
        return rel.to_string();
    }
    let joined = format!("{dir}/{rel}");
    let mut segs: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        if seg == ".."
            && segs
                .last()
                .is_some_and(|last| !last.is_empty() && *last != "." && *last != "..")
        {
            segs.pop();
        } else {
            segs.push(seg);
        }
    }
    segs.join("/")
}

/// the last `/` segment — the module name a manifest-less dir answers
fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// everything before the last `/` segment
fn dirname(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}
