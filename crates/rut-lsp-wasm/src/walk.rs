//! The read-only dep walk — the extension's manifest walk
//! (`integrations/vscode-extension/src/deps.ts`) moved INTO the crate,
//! reading only through the injected host environment
//! ([`crate::env::Io`]): pure reads, one fast handler turn, no walk
//! task, no awaits (the actor seam's law — the walk is a sync body
//! over `&mut SessionState` exactly like every other handler).
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
//!   [`SessionState::index_named_at`] / [`SessionState::index_mod_at`] bodies the
//!   `index_mod_at` handler runs, so a walked dep
//!   resolves in hover/completion/definition exactly like a
//!   host-indexed one (one store, replace-by-origin, no drift). The
//!   mod-tree recursion follows the wasm-parsed decls (the
//!   `index_mod_at` reply), `NAME/mod.rut` then `NAME.rut`
//!   spellings, beside the declaring file.
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

use std::collections::HashSet;

use crate::deps::{DepRow, DepSource};
use crate::env::Io;
use crate::SessionState;

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
pub(crate) fn resolve(state: &mut SessionState, io: &dyn Io, roots: &[String]) -> WalkOut {
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
fn feed_lock(state: &mut SessionState, io: &dyn Io, root: &str) {
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
/// law: a dep's dev table never enters a consumer's world).
fn walk_manifest(ctx: &mut WalkCtx, state: &mut SessionState, dir: &str, depth: usize) {
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
    let table = match crate::deps::dep_table_pinned(&text, &state.lock_pins) {
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
    // and the names must sit in it BEFORE anything mounts (the dep
    // indexes stamp the marker at index time). Names accumulate
    // across roots and re-walks, never shrink the world.
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
            // the manifest's consts ride the pkg's own rows (the check
            // lane threads them into the engine's Manifest)
            state.index_named_at(&path, &module, &src, &table.consts);
        } // a manifest naming a missing entry — index what exists
    }
    // the mod tree (the pkg's own modules): the root module stamps at
    // mod path `""` (the entries loop's flat stamp for the same file
    // REPLACES with it — same origin), then each `mod NAME;` edge
    // mounts beside the root file, recursively, depth- and
    // file-budget-capped. The flavor follows the pkg: a lib's root is
    // its first Impl entry (`src/mod.rut`, `NAME/mod.rut`/`NAME.rut`
    // children), a host pkg's root is its DECL entry (the `.d.rut`
    // surface, `NAME/mod.d.rut` children — the loader's own flavor
    // law), and an entry-less dev state probes the lib convention.
    let entry_rows: Vec<(&str, rut_semantic::Mode)> = table
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.mode))
        .collect();
    mount_mod_tree(ctx, state, dir, &module, &entry_rows, &table.consts, depth);
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
fn mount_row(ctx: &mut WalkCtx, state: &mut SessionState, dir: &str, row: &DepRow, depth: usize) {
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
                        "rut: dep '{}' — {rel} has no rut.jsonc; a path dep is a module \
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

/// one package's mod tree: the root module stamps at mod path `""`
/// (the entries loop's flat stamp for the same file REPLACES with it —
/// same origin), then each `mod NAME;` edge mounts beside the root
/// file, recursively, depth- and file-budget-capped. `entries` carries
/// the manifest's own rows: a lib's first IMPL entry is its root
/// (`src/mod.rut`), else a HOST pkg's DECL entry is (the `.d.rut`
/// surface — its tree mounts with the Decl flavor, `NAME/mod.d.rut`
/// children, the loader's own law), else the lib convention is probed
/// (a surface-only dev state).
fn mount_mod_tree(
    ctx: &mut WalkCtx,
    state: &mut SessionState,
    pkg_dir: &str,
    module: &str,
    entries: &[(&str, rut_semantic::Mode)],
    consts: &[(String, f64)],
    depth: usize,
) {
    if depth > MAX_DEP_DEPTH || ctx.files == 0 {
        return; // the caps truncate — the set stays incomplete
    }
    let (root_path, decl_flavor, charged) = if let Some((first, _)) =
        entries.iter().find(|(_, m)| *m == rut_semantic::Mode::Impl)
    {
        // the root-module lane: `src/mod.rut` is the root (the entries
        // loop already indexed and charged it — this re-stamp at mod
        // path "" is the same answer, replace-by-origin)
        (join(pkg_dir, first), false, false)
    } else if let Some((first, _)) = entries.iter().find(|(_, m)| *m == rut_semantic::Mode::Decl) {
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
    // the root re-stamp carries the manifest's consts (it REPLACES the
    // entries loop's row — same origin — so the rows ride again)
    let decls = state.index_mod_at(&root_path, module, "", &text, consts);
    // the children mount BESIDE THE ROOT FILE — inside the package's
    // `src/` shell
    let root_dir = dirname(&root_path);
    for decl in decls {
        if ctx.files == 0 {
            return; // the budget died — the set stays incomplete
        }
        mount_child_mod(
            ctx,
            state,
            &root_dir,
            module,
            &decl.name,
            "",
            depth,
            decl_flavor,
        );
    }
}

/// one `mod NAME;` edge: the lib flavor probes `NAME/mod.rut` beside
/// the declaring file first, then the single-file spelling `NAME.rut`;
/// BOTH present is the loader's loud ambiguity (hinted once, mirrored).
/// The Decl flavor (a host pkg's surface tree) probes the loader's own
/// spelling only — `NAME/mod.d.rut`, directory form, no single-file
/// twin. The found child stamps under the module at its mount path —
/// `NAME` under the root, `parent/NAME` nested — and its own decls
/// recurse from its physical directory (a file-form child's children
/// mount beside it, not under `NAME/`).
#[allow(clippy::too_many_arguments)] // the walk threads exactly this state
fn mount_child_mod(
    ctx: &mut WalkCtx,
    state: &mut SessionState,
    declaring_dir: &str,
    module: &str,
    name: &str,
    parent_path: &str,
    depth: usize,
    decl_flavor: bool,
) {
    let dir_form = if decl_flavor {
        // the host surface tree's child spelling (the loader's
        // DECL_CHILD_FILE law — dir form only)
        format!(
            "{declaring_dir}/{name}/{}",
            rut_driver::decl::DECL_CHILD_FILE
        )
    } else {
        format!("{declaring_dir}/{name}/mod.rut")
    };
    let file_form = format!("{declaring_dir}/{name}.rut");
    let dir_bytes = ctx.io.read(&dir_form);
    // the Decl flavor never probes the single-file twin — a stray
    // `NAME.rut` beside a host tree is nobody's module
    let file_bytes = if decl_flavor {
        None
    } else {
        ctx.io.read(&file_form)
    };
    let (child_path, bytes) = match (dir_bytes, file_bytes) {
        (Some(_), Some(_)) => {
            state.hint_once(
                ctx.io,
                &format!("mods:{file_form}"),
                &format!("rut: {module} — both {name}/mod.rut and {name}.rut exist; keep one"),
            );
            return;
        }
        (Some(b), None) => (dir_form.clone(), b),
        (None, Some(b)) => (file_form, b),
        (None, None) => return, // nothing under either spelling — mounts nothing
    };
    if depth + 1 > MAX_DEP_DEPTH || ctx.files == 0 {
        return; // the caps truncate — the child never mounts
    }
    ctx.files -= 1;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mod_path = if parent_path.is_empty() {
        name.to_string()
    } else {
        format!("{parent_path}/{name}")
    };
    // the consts mint on the pkg-ROOT row only — a child carries none
    let decls = state.index_mod_at(&child_path, module, &mod_path, &text, &[]);
    // a file-form child's children mount BESIDE it, not under `NAME/`
    let child_dir = if child_path == dir_form {
        format!("{declaring_dir}/{name}")
    } else {
        declaring_dir.to_string()
    };
    for decl in decls {
        if ctx.files == 0 {
            return; // the budget died — the set stays incomplete
        }
        mount_child_mod(
            ctx,
            state,
            &child_dir,
            module,
            &decl.name,
            &mod_path,
            depth + 1,
            decl_flavor,
        );
    }
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
fn mount_url_dep(ctx: &mut WalkCtx, state: &mut SessionState, row: &DepRow, depth: usize) {
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
                "rut: dep '{}' has no sha256 pin in rut.lock — the editor never mounts \
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
                "rut: dep '{}' ({url}) is not in the dep store — run `rut install` in the \
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
                "rut: dep '{}' — the store entry at {dir} does not verify against the \
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
    // own path rows)
    walk_manifest(
        ctx,
        state,
        &format!("{dir}/{}", rut_driver::UNPACK_DIR),
        depth + 1,
    );
}

// ---- the rooted-string path helpers (join with `/` only — the host
// normalizes; `std::path` never enters) ----

/// `<dir>/<rel>` (an absolute `rel` wins, mirroring `path.resolve`)
fn join(dir: &str, rel: &str) -> String {
    if rel.starts_with('/') {
        rel.to_string()
    } else {
        format!("{dir}/{rel}")
    }
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
