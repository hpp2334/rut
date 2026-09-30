//! Filesystem loader — read a module directory (`rut.toml`) or a packed
//! `.rutbundle` into a [`Session`].
//!
//! One module is one directory with ONE entry file: its `use` statements
//! are all inter-module paths (`use <pkg>::{A, B};`), resolved by the
//! `Session` — there is no intra-module include form. A `.rutbundle` is
//! the same contract zipped: the manifest plus the compiled root binary,
//! its scope ledger, and each dep group as a compiled `.rutc` or a
//! source file set.
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); this
//! native helper is the counterpart that reads files.

use std::collections::BTreeMap;
use std::path::Path;

use rut_bundle::{
    bundle_key, entry_rel, parse_manifest, read_entry, Bundle, Entry, FsSource, GroupKind, Layout,
    Manifest, PkgType,
};
use crate::session::{Module, ModuleBody, Session};

/// Read a directory's `rut.toml` — the real-filesystem lane of
/// [`rut_bundle::read_manifest`] (the crate itself never touches the
/// filesystem).
fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    rut_bundle::read_manifest(dir, &FsSource)
}

/// Read one module source file. The language has no include form to
/// expand (use paths are inter-module) — but a module may
/// be AUTHORED as several files: the loader splices `entry.libs` into
/// one source, which is assembly, not expansion.
pub fn load_module_source(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Mount a directory's consumer manifest: its own module (if named) and
/// every `[deps]` module, reading each dep's `rut.toml` and entry source,
/// then the mount passes over the finished closure. Returns the
/// session and the root spec (the directory's own `name`).
///
/// The mount order is the law:
/// 1. the `[deps]` walk — unchanged;
/// 2. the dev pass — the ROOT's `[dev-deps]` mount exactly like `[deps]`
///    (a dep's dev table is never walked, so a consumer's world never
///    contains it);
/// 3. the peer gate — ONE post-closure pass (a peer may mount after its
///    declarer alphabetically, so it cannot run during the walk);
/// 4. compile — unchanged; `compile_graph` sees ordinary sources.
pub fn load_dir_session(dir: &Path) -> Result<(Session, String), String> {
    let manifest = read_manifest(dir)?;
    let mut session = Session::new();

    let root = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{} has no `name`", dir.join("rut.toml").display()))?;
    let root_module = load_entry_module(dir, &manifest)?;
    session
        .register_module(&root, root_module)
        .map_err(|e| e.to_string())?;
    // the root's dir rides the session's map too, so a later
    // `assemble_peers` over this session can answer for the root's own
    // peer declarations (the gate below already ran with the complete
    // map; this keeps the session self-consistent)
    session.record_peer_dir(&root, dir);
    record_peers(&mut session, &root, &manifest);
    // the loader's own spec → dir map, for the peer gate's group reads —
    // the Session itself stays I/O-free
    let mut mounted = std::collections::BTreeMap::new();
    mounted.insert(root.clone(), dir.to_path_buf());
    let mut visiting = vec![dir.to_path_buf()];
    resolve_deps(&mut session, dir, &manifest, &mut visiting, &mut mounted)?;
    resolve_table(&mut session, dir, &manifest.dev_deps, &mut visiting, &mut mounted)?;
    run_peer_gate(&mut session, &root, &mounted)?;
    Ok((session, root))
}

/// Build a package's entry [`Module`] from bundle entries under
/// `prefix` (empty for the root, `<pkg>/` for a dep group) — the
/// in-archive counterpart of `load_entry_module`: the declared kind
/// dispatches (a `type = "host"` pkg is a decl surface; a `type =
/// "lib"` pkg with a surface and no body is the surface-only dev
/// state).
fn bundle_entry_module(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Module, String> {
    let read = |rel: &str| -> Result<String, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        read_entry(entries, &key)
    };
    match manifest.pkg_type {
        PkgType::Host => {
            let rel = manifest.entry.type_path.as_ref().ok_or_else(|| {
                format!("module at bundle prefix `{prefix}` is a host pkg with no `entry.type`")
            })?;
            let src = read(rel)?;
            let mut m = crate::decl::lower_decl_module(&src, &format!("{prefix}{rel}"))?;
            m.entry = manifest.entry.clone();
            return Ok(m);
        }
        PkgType::Lib => {
            if let Some(rel) = &manifest.entry.type_path {
                let origin = format!("{prefix}{}", rel.strip_prefix("./").unwrap_or(rel));
                let src = read(rel)?;
                refuse_host_rows(&src, &origin)?;
                if manifest.entry.lib.is_none() {
                    // the surface-only dev state: a decl unit — no host
                    // rows, nothing exported (use sites resolve-miss,
                    // correctly)
                    return Ok(Module {
                        body: ModuleBody::Source { text: src, is_decl: true },
                        entry: manifest.entry.clone(),
                        ..Default::default()
                    });
                }
            }
        }
    }
    let rel = entry_rel(manifest)
        .ok_or_else(|| format!("module at bundle prefix `{prefix}` has no entry"))?;
    let mut src = read(rel)?;
    // the multi-lib splice, the archive-side twin of
    // `load_entry_module`'s: base first, then `libs` in manifest
    // order, '\n'-joined — ONE source string
    for lib in &manifest.entry.libs {
        src.push('\n');
        src.push_str(&read(lib)?);
    }
    Ok(Module {
        body: ModuleBody::Source { text: src, is_decl: false },
        entry: manifest.entry.clone(),
        ..Default::default()
    })
}

/// Mount a `.rutbundle` — a v5 **compiled** bundle: the
/// root's `.rutc` binary, its pack-time scope ledger, and each dep
/// group as a compiled `.rutc` or a source file set. The gate order is
/// the law: container CRC ([`Bundle::parse`]), manifest + exact
/// `format_version = 5` ([`Layout::parse`]), per-group decode and
/// verification (inside the layout parse) — then, and only then, the
/// mount. Older layouts are refused with the one-line version error:
/// refuse, never guess. Source sharing stays what it always was outside
/// bundles: a directory.
pub fn load_bundle_session(path: &Path) -> Result<(Session, String), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    load_bundle_bytes(&bytes, path)
}

/// [`load_bundle_session`] over in-memory bytes (tests, embedders,
/// wasm hosts).
pub fn load_bundle_bytes(bytes: &[u8], origin: &Path) -> Result<(Session, String), String> {
    // gate 1: the container — every entry's CRC-32 verified
    let bundle = Bundle::parse(bytes).map_err(|e| format!("{}: {e}", origin.display()))?;
    // gate 2 + 3: the manifest/version, then every group's decode +
    // verification — the layout parse refuses anything it cannot
    // decode, so a bad binary never reaches the session
    let layout = Layout::parse(&bundle).map_err(|e| format!("{}: {e}", origin.display()))?;
    let Layout::Compiled { manifest, root, scopes, groups } = layout;
    let entries = bundle.entries();
    let root_spec = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{}: rut.toml has no `name`", origin.display()))?;
    let mut session = Session::new();
    // the scope ledger first: the graph's compiled-mount arm resolves
    // every decoded foreign id through it
    for (scope, spec) in &scopes {
        session.record_bundle_scope(*scope, spec);
    }
    // the root: a compiled module (the packer refuses any other root)
    session
        .register_module(
            &root_spec,
            Module {
                body: ModuleBody::Compiled(root),
                entry: manifest.entry.clone(),
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
    record_peers(&mut session, &root_spec, &manifest);
    // the archive's group prefixes (root "" first) — the peer gate's
    // group reads key on these
    let mut prefixes: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    prefixes.insert(root_spec.clone(), String::new());
    for (prefix, kind) in &groups {
        let dep_toml =
            read_entry(entries, &format!("{prefix}/rut.toml")).map_err(|e| format!("{}: {e}", origin.display()))?;
        let dm = parse_manifest(&dep_toml)
            .map_err(|e| format!("{}: {prefix}/rut.toml: {e}", origin.display()))?;
        let name = dm
            .name
            .clone()
            .ok_or_else(|| format!("{}: {prefix}/rut.toml has no `name`", origin.display()))?;
        if session.resolve(&name).is_ok() {
            continue; // first mount wins (the root, an earlier group)
        }
        let module = match kind {
            GroupKind::Compiled(program) => {
                // the ledger must name the group, and the row must be
                // the scope the binary itself carries (refuse, never
                // guess — a mismatch is a corrupt or doctored bundle)
                let Some(own) = rut_core::link::own_scope(program) else {
                    return Err(format!(
                        "{}: {prefix}/{}: the program carries no scope blocks",
                        origin.display(),
                        name
                    ));
                };
                match scopes.iter().find(|(_, s)| s == &name) {
                    Some(&(row, _)) if row == own => {}
                    Some(&(row, _)) => {
                        return Err(format!(
                            "{}: `{name}`'s ledger row says scope {row}, but its binary carries {own}",
                            origin.display()
                        ));
                    }
                    None => {
                        return Err(format!(
                            "{}: the scope ledger does not name `{name}` — the bundle is incomplete",
                            origin.display()
                        ));
                    }
                }
                Module {
                    body: ModuleBody::Compiled(program.clone()),
                    entry: dm.entry.clone(),
                    ..Default::default()
                }
            }
            GroupKind::Source => bundle_entry_module(entries, &format!("{prefix}/"), &dm)
                .map_err(|e| format!("{}: {e}", origin.display()))?,
        };
        session
            .register_module(&name, module)
            .map_err(|e| e.to_string())?;
        record_peers(&mut session, &name, &dm);
        prefixes.insert(name.clone(), format!("{prefix}/"));
    }
    // every declared dep must be satisfied by a group
    for spec in manifest.deps.keys() {
        if session.resolve(spec).is_err() {
            return Err(format!(
                "{}: the bundle is missing its `{spec}` dependency group",
                origin.display()
            ));
        }
    }
    run_bundle_peer_gate(&mut session, entries, &prefixes, origin)?;
    Ok((session, root_spec))
}

/// The peer gate over a mounted bundle — pass 3 of the
/// mount order, in-archive flavor: every group is already mounted
/// (presence is by NAME; first-mount-wins), so
///
/// - required peer absent → the loud D1 mount error (matrix row 1);
/// - optional peer absent → inert; the group simply never mounts;
/// - peer present → the declarer's group rows mount. A COMPILED
///   declarer's rows are already in its `.rutc` (the pack-time
///   closure's dev-deps supplied the peers — compile once per owner),
///   so only the presence law runs; a SOURCE declarer's group file
///   (the descriptor's `lib`, an impl-only `.rut`) is read from the
///   archive and recorded for the graph to compile into the
///   declarer's unit. A declared group a source declarer's archive
///   does not carry is a load error — the consistency row (a bundle
///   that declares a group must carry it).
///
/// The D3 path checks are directory-time law (a broken `[peer-deps]`
/// path is the declaring pkg's own packaging bug): a bundle has no
/// directories, the `path` keys never cross the pack boundary, and
/// presence is by name.
fn run_bundle_peer_gate(
    session: &mut Session,
    entries: &[(String, Vec<u8>)],
    prefixes: &std::collections::BTreeMap<String, String>,
    origin: &Path,
) -> Result<(), String> {
    // collected first, applied after — the registry borrows the session
    let mut appends: Vec<(String, String)> = Vec::new();
    for (pkg, peers) in session.peer_decls() {
        for (peer, decl) in peers {
            if session.resolve(peer).is_err() {
                if decl.optional {
                    continue; // inert — the group simply never mounts
                }
                // D1: loud at load, naming pkg + peer + fix
                return Err(format!(
                    "pkg `{pkg}` requires the peer `{peer}`, and `{peer}` is not in this program's closure — peers are not pulled transitively: add `{peer} = {{ path = \"..\" }}` to your `rut.toml` `[deps]`"
                ));
            }
            let Some(lib) = &decl.lib else {
                continue; // presence declared, no integration file to mount
            };
            // a compiled declarer's rows ride its binary — nothing to read
            if matches!(session.resolve(pkg).map(|m| &m.body), Ok(ModuleBody::Compiled(_))) {
                continue;
            }
            let Some(prefix) = prefixes.get(pkg) else {
                return Err(format!(
                    "pkg `{pkg}` declares `[peer-deps]` but is not mounted"
                ));
            };
            let rel = lib.strip_prefix("./").unwrap_or(lib);
            let key = bundle_key(&format!("{prefix}{rel}"))
                .map_err(|e| format!("{}: {e}", origin.display()))?;
            let text = read_entry(entries, &key)
                .map_err(|e| format!("{}: {e}", origin.display()))?;
            appends.push((pkg.clone(), text));
        }
    }
    for (pkg, text) in appends {
        session.record_peer_group(&pkg, &text);
        session.mark_groups_mounted(&pkg);
    }
    Ok(())
}

/// Load a module directory (`rut.toml`) or a `.rutbundle` file — the two
/// packed forms of the same contract. A loose `.rut` file is NOT this: it
/// is a single-file module with no manifest.
pub fn load_path_session(path: &Path) -> Result<(Session, String), String> {
    if path.is_dir() {
        return load_dir_session(path);
    }
    if path.extension().map_or(false, |e| e == "rutbundle") {
        return load_bundle_session(path);
    }
    Err(format!(
        "{} is neither a module directory (no `rut.toml`) nor a `.rutbundle`",
        path.display()
    ))
}

/// Build a directory's entry [`Module`] from its manifest:
///
/// - a `type = "host"` pkg — a pure declaration surface. The `.d.rut`
///   parses in declaration mode and lowers into the module's host fns.
///   No body exists — the embedding Rust binds it at run time.
/// - a `type = "lib"` pkg (the default) — a source module: the body
///   compiles; the surface derives from its exports. A declared
///   surface with no body is the surface-only dev state (a decl unit);
///   `host fn` text is refused in either file — the lib-surface law.
fn load_entry_module(dir: &Path, manifest: &Manifest) -> Result<Module, String> {
    match manifest.pkg_type {
        PkgType::Host => {
            let rel = manifest.entry.type_path.as_ref().ok_or_else(|| {
                format!(
                    "module in {} is a host pkg with no `entry.type`",
                    dir.display()
                )
            })?;
            let origin = format!("{}/{}", dir.display(), rel);
            let src = load_module_source(&dir.join(rel))?;
            let mut m = crate::decl::lower_decl_module(&src, &origin)?;
            m.entry = manifest.entry.clone();
            return Ok(m);
        }
        PkgType::Lib => {
            if let Some(rel) = &manifest.entry.type_path {
                let origin = format!("{}/{}", dir.display(), rel);
                let src = load_module_source(&dir.join(rel))?;
                refuse_host_rows(&src, &origin)?;
                if manifest.entry.lib.is_none() {
                    // the surface-only dev state: a decl unit — no host
                    // rows, nothing exported (use sites resolve-miss,
                    // correctly)
                    return Ok(Module {
                        body: ModuleBody::Source { text: src, is_decl: true },
                        entry: manifest.entry.clone(),
                        ..Default::default()
                    });
                }
            }
        }
    }
    let mut src = load_entry(dir, &manifest.entry)?;
    // The multi-lib splice: the base `lib` first, then
    // `libs` in manifest order, '\n'-joined exactly like the
    // peer-group append — the combined text stays ONE source string,
    // so every downstream consumer of the source body (the graph
    // splice, the wasm mounts) is untouched. The manifest's array
    // order is the canonical order: the splice never reads a directory
    // listing, so same manifest ⇒ same module (the determinism law).
    for rel in &manifest.entry.libs {
        let text = load_module_source(&dir.join(rel))?;
        src.push('\n');
        src.push_str(&text);
    }
    Ok(Module {
        body: ModuleBody::Source { text: src, is_decl: false },
        entry: manifest.entry.clone(),
        ..Default::default()
    })
}

/// The lib-surface law: `host fn` text lives only in `type = "host"`
/// pkgs. Non-parsing surfaces stay inert (doc-only, as today); a
/// surface that PARSES and declares host fns is the loud error.
fn refuse_host_rows(src: &str, origin: &str) -> Result<(), String> {
    if let Ok(m) = crate::decl::lower_decl_module(src, origin) {
        if let ModuleBody::Host { host_funcs, .. } = m.body {
            if let Some((name, ..)) = host_funcs.first() {
                return Err(format!(
                    "{origin}: `host fn {name}` — a lib pkg cannot declare \
                     host fns; split the rows into a `type = \"host\"` pkg \
                     and depend on it"
                ));
            }
        }
    }
    Ok(())
}

fn load_entry(dir: &Path, entry: &Entry) -> Result<String, String> {
    let rel = entry
        .lib
        .as_ref()
        .or(entry.type_path.as_ref())
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    load_module_source(&dir.join(rel))
}

/// Record a manifest's `[peer-deps]` into the session's registry (RFC
/// 0045): the loader reads every mounted pkg's manifest anyway, so the
/// registry costs no extra I/O.
fn record_peers(session: &mut Session, pkg: &str, manifest: &Manifest) {
    for (peer, desc) in &manifest.peer_deps {
        session.record_peer(pkg, peer, crate::session::PeerDecl::of(desc));
    }
}

/// Mount `dir`'s `[dev-deps]` table into an EXISTING session — the
/// packer's compile-once-per-owner pass (a dep's unit packs the same
/// bytes wherever it travels, its dev-mounted peers included).
/// First-mount-wins, exactly like the `[deps]` walk.
pub fn mount_dev_table(
    session: &mut Session,
    dir: &Path,
    manifest: &Manifest,
) -> Result<(), String> {
    let mut visiting = Vec::new();
    let mut mounted = BTreeMap::new();
    resolve_table(session, dir, &manifest.dev_deps, &mut visiting, &mut mounted)
}

/// Resolve a manifest's `[deps]` recursively — pass 1 of
/// the mount order.
fn resolve_deps(
    session: &mut Session,
    dir: &Path,
    manifest: &Manifest,
    visiting: &mut Vec<std::path::PathBuf>,
    mounted: &mut BTreeMap<String, std::path::PathBuf>,
) -> Result<(), String> {
    resolve_table(session, dir, &manifest.deps, visiting, mounted)
}

/// Walk one descriptor table — the `[deps]` walk (pass 1; pass 2 feeds
/// it the root's `[dev-deps]`, which mounts exactly the same way).
/// Each entry is a relative `path` to a package directory, loaded and
/// mounted under its key. **First mount wins** — a name already in the
/// session (the embedder's, the root's, or an earlier dep's) is never
/// overwritten; a dep whose manifest `name` disagrees with its key is
/// an error naming both. `visiting` guards cycles. Every mounted pkg's
/// directory and `[peer-deps]` declarations are recorded for pass 3.
fn resolve_table(
    session: &mut Session,
    dir: &Path,
    table: &BTreeMap<String, BTreeMap<String, String>>,
    visiting: &mut Vec<std::path::PathBuf>,
    mounted: &mut BTreeMap<String, std::path::PathBuf>,
) -> Result<(), String> {
    for (spec, desc) in table {
        if session.resolve(spec).is_ok() {
            continue; // already mounted — the embedder's (or an earlier) mount wins
        }
        let rel = desc
            .get("path")
            .ok_or_else(|| format!("dep `{spec}` has no `path`"))?;
        let dep_dir = dir.join(rel);
        if visiting.contains(&dep_dir) {
            return Err(format!(
                "cyclic use: `{spec}` ({}) is already being loaded",
                dep_dir.display()
            ));
        }
        let dm = read_manifest(&dep_dir)?;
        if dm.name.as_deref() != Some(spec.as_str()) {
            return Err(format!(
                "dep `{spec}` points at `{}` — the manifest there names it `{}`",
                dep_dir.display(),
                dm.name.as_deref().unwrap_or("<unnamed>")
            ));
        }
        let dep_module = load_entry_module(&dep_dir, &dm)?;
        session
            .register_module(spec, dep_module)
            .map_err(|e| e.to_string())?;
        session.record_peer_dir(spec, &dep_dir);
        record_peers(session, spec, &dm);
        mounted.insert(spec.clone(), dep_dir.clone());
        visiting.push(dep_dir.clone());
        // a dep's dev-deps are NEVER walked — pass 2 is root-only, so a
        // consumer's world never contains another pkg's dev table
        resolve_deps(session, &dep_dir, &dm, visiting, mounted)?;
        visiting.pop();
    }
    Ok(())
}

/// Pass 3 — the peer gate, ONE post-closure pass over the
/// recorded peer declarations:
///
/// - required peer absent → the loud D1 mount error: names the pkg, the
///   peer, and the fix. Never auto-pulled — the consumer supplies.
/// - peer present (any reason) → the pkg's group file (the descriptor's
///   `lib`, an impl-only `.rut`) is recorded for the graph to compile
///   INTO the declarer's unit (groups in peer-name order after the
///   base) — presence-based mounting; the mounted body stays pristine.
/// - optional peer absent → inert; the group simply never mounts.
///
/// The program root's own peer paths are read and name-checked even
/// when dev-deps already supplied presence — a broken path is the loud
/// D3 packaging-bug error at the pkg's own build (matrix row 6). A
/// dep's peer paths are never read: presence is by NAME
/// (first-mount-wins already guarantees the consumer's own path won),
/// so a broken peer path is inert for an optional peer and unreachable
/// for a required one (its absence is D1's business, not the path's).
fn run_peer_gate(
    session: &mut Session,
    root: &str,
    mounted: &BTreeMap<String, std::path::PathBuf>,
) -> Result<(), String> {
    // collected first, applied after — the registry borrows the session
    let mut appends: Vec<(String, String)> = Vec::new();
    let mut pre_compiled: Vec<String> = Vec::new();
    for (pkg, peers) in session.peer_decls() {
        if session.groups_mounted(pkg) {
            continue; // an earlier gate pass over this session mounted them
        }
        // a compiled declarer's rows ride its binary — presence law only
        if matches!(session.resolve(pkg).map(|m| &m.body), Ok(ModuleBody::Compiled(_))) {
            pre_compiled.push(pkg.clone());
            continue;
        }
        let Some(pkg_dir) = mounted.get(pkg) else {
            return Err(format!("pkg `{pkg}` declares `[peer-deps]` but is not mounted"));
        };
        for (peer, decl) in peers {
            if pkg.as_str() == root {
                // self-build: the path must resolve and name the peer —
                // even when dev-deps already supplied presence
                let peer_dir = pkg_dir.join(&decl.path);
                match read_manifest(&peer_dir) {
                    Ok(dm) if dm.name.as_deref() == Some(peer.as_str()) => {}
                    Ok(dm) => {
                        return Err(format!(
                            "pkg `{pkg}`'s [peer-deps] entry `{peer}` points at `{path}` — the manifest there names it `{actual}` (a packaging bug in {pkg})",
                            path = decl.path,
                            actual = dm.name.as_deref().unwrap_or("<unnamed>"),
                        ));
                    }
                    Err(_) => {
                        return Err(format!(
                            "pkg `{pkg}`'s [peer-deps] entry `{peer}` points at `{path}` — cannot read a manifest there (a packaging bug in {pkg})",
                            path = decl.path,
                        ));
                    }
                }
            }
            if session.resolve(peer).is_err() {
                if decl.optional {
                    continue; // inert — the group simply never mounts
                }
                // D1: loud at mount, naming pkg + peer + fix
                return Err(format!(
                    "pkg `{pkg}` requires the peer `{peer}`, and `{peer}` is not in this program's closure — peers are not pulled transitively: add `{peer} = {{ path = \"..\" }}` to your `rut.toml` `[deps]`"
                ));
            }
            let Some(lib) = &decl.lib else {
                continue; // presence declared, no integration file to mount
            };
            let group_path = pkg_dir.join(lib);
            let text = std::fs::read_to_string(&group_path).map_err(|_| {
                format!(
                    "pkg `{pkg}`'s [peer-deps] entry `{peer}` names the group `{lib}` — cannot read it (a packaging bug in {pkg})"
                )
            })?;
            appends.push((pkg.clone(), text));
        }
    }
    for (pkg, text) in appends {
        session.record_peer_group(&pkg, &text);
        session.mark_groups_mounted(&pkg);
    }
    for pkg in pre_compiled {
        session.mark_groups_mounted(&pkg);
    }
    Ok(())
}

/// Mount one package directory — and, recursively, its `[deps]` — into
/// an EXISTING session: the programmatic counterpart of a manifest's
/// dep walk (native hosts, tests, plugin loaders). Returns the
/// package's own name. A name already mounted wins.
///
/// This is an OFFER to someone else's program, not "building the pkg
/// itself": no dev-deps are mounted (pass 2 is root-only) and the peer
/// gate does not run here — the embedder's world grows incrementally,
/// so presence is a program-closure property the program's own
/// `load_dir_session`/compile owns. The pkg's peer
/// declarations are still recorded for the session's registry.
pub fn mount_dir(session: &mut Session, dir: &Path) -> Result<String, String> {
    let manifest = read_manifest(dir)?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{} has no `name`", dir.join("rut.toml").display()))?;
    if session.resolve(&name).is_ok() {
        return Ok(name); // the embedder's mount outranks the directory
    }
    let module = load_entry_module(dir, &manifest)?;
    session
        .register_module(&name, module)
        .map_err(|e| e.to_string())?;
    session.record_peer_dir(&name, dir);
    record_peers(session, &name, &manifest);
    let mut visiting = vec![dir.to_path_buf()];
    let mut mounted = BTreeMap::new(); // the gate's map — not this path's pass
    resolve_deps(session, dir, &manifest, &mut visiting, &mut mounted)?;
    Ok(name)
}

/// The peer gate for sessions built mount-by-mount: runs
/// the gate's append pass over the peer declarations every `mount_dir`/
/// dep walk recorded, using the pkg→dir map those mounts left behind.
/// The CLI's and the probe's single-file convenience lanes call this
/// after their tree mounts, so a loose file that `use json::` gets the
/// peer-gated container groups exactly like a module-dir program does.
/// Presence-based as ever: an optional peer absent is inert; a required
/// peer absent is the loud D1 error. Groups already mounted by an
/// earlier gate pass over this session are skipped — never
/// double-appended.
pub fn assemble_peers(session: &mut Session) -> Result<(), String> {
    let dirs = session.peer_dirs().clone();
    run_peer_gate(session, "", &dirs)
}

/// Read a directory's `rut.toml` graph and compile it to one linked program.
pub fn compile_dir(dir: &Path) -> Result<crate::graph::GraphOutput, String> {
    let (session, root) = load_dir_session(dir)?;
    Ok(crate::compile_graph(&session, &root))
}
