//! Filesystem loader — read a module directory (`rut.toml`) or a packed
//! `.rutbundle` (RFC 0038) into a [`Session`].
//!
//! One module is one directory with ONE entry file: its `use` statements
//! are all inter-module paths (`use <pkg>::{A, B};`), resolved by the
//! `Session` — there is no intra-module include form. A `.rutbundle` is
//! the same contract zipped: `rut.toml` plus the rut entry source.
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); this
//! native helper is the counterpart that reads files.

use std::collections::BTreeMap;
use std::path::Path;

use crate::bundle::{parse_bundle, write_bundle};
use crate::session::{parse_manifest, Entry, Module, Session};

/// Bundle entry normalization: `./x.rut` → `x.rut`; anything reaching
/// outside the archive root is refused (v1 bundles are flat).
fn bundle_key(rel: &str) -> Result<String, String> {
    let key = rel.strip_prefix("./").unwrap_or(rel);
    if key.starts_with('/') || key.split('/').any(|p| p == "..") {
        return Err(format!("bundle entry `{rel}` escapes the bundle root"));
    }
    Ok(key.to_string())
}

fn read_entry(entries: &[(String, Vec<u8>)], key: &str) -> Result<String, String> {
    match entries.iter().find(|(n, _)| n == key) {
        Some((_, bytes)) => String::from_utf8(bytes.clone())
            .map_err(|_| format!("bundle entry `{key}` is not UTF-8")),
        None => Err(format!("bundle has no entry `{key}`")),
    }
}

/// Read one module source file. One file is one module unit — there is
/// no include form to expand (RFC 0035 §1: use paths are inter-module).
pub fn load_module_source(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Mount a directory's consumer manifest: its own module (if named) and
/// every `[deps]` module, reading each dep's `rut.toml` and entry source,
/// then the RFC 0045 mount passes over the finished closure. Returns the
/// session and the root spec (the directory's own `name`).
///
/// The mount order is the law (RFC 0045 §3):
/// 1. the `[deps]` walk — unchanged;
/// 2. the dev pass — the ROOT's `[dev-deps]` mount exactly like `[deps]`
///    (a dep's dev table is never walked, so a consumer's world never
///    contains it);
/// 3. the peer gate — ONE post-closure pass (a peer may mount after its
///    declarer alphabetically, so it cannot run during the walk);
/// 4. compile — unchanged; `compile_graph` sees ordinary sources.
pub fn load_dir_session(dir: &Path) -> Result<(Session, String), String> {
    let text = std::fs::read_to_string(dir.join("rut.toml")).map_err(|e| e.to_string())?;
    let manifest = parse_manifest(&text).map_err(|e| e.to_string())?;
    let mut session = Session::new();

    let root = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{} has no `name`", dir.join("rut.toml").display()))?;
    let root_module = load_entry_module(dir, &manifest)?;
    session
        .register_module(&root, root_module)
        .map_err(|e| e.to_string())?;
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

/// Mount a `.rutbundle` file (RFC 0038): one module, its source read from
/// the archive. The manifest's `format`/`format_version` are checked
/// before anything else is read — an unknown layout is refused, never
/// guessed at.
pub fn load_bundle_session(path: &Path) -> Result<(Session, String), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    load_bundle_bytes(&bytes, path)
}

/// Build a package's entry [`Module`] from bundle entries under
/// `prefix` (empty for the root, `<pkg>/` for a dep group) — the
/// in-archive counterpart of `load_entry_module`: `entry.type`-only is
/// a host pkg (decl surface), else the lib source.
fn bundle_entry_module(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &crate::session::Manifest,
) -> Result<Module, String> {
    let read = |rel: &str| -> Result<String, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        read_entry(entries, &key)
    };
    if manifest.entry.lib.is_none() && manifest.entry.type_path.is_some() {
        let rel = manifest.entry.type_path.as_ref().unwrap();
        let src = read(rel)?;
        let mut m = crate::decl::lower_decl_module(&src, &format!("{prefix}{rel}"))?;
        m.host_scope = manifest.host_scope.clone();
        m.inline = manifest.inline;
        m.entry = manifest.entry.clone();
        return Ok(m);
    }
    let rel = entry_rel(manifest)
        .ok_or_else(|| format!("module at bundle prefix `{prefix}` has no entry"))?;
    let src = read(rel)?;
    Ok(Module {
        source: Some(src),
        entry: manifest.entry.clone(),
        inline: manifest.inline,
        host_scope: manifest.host_scope.clone(),
        ..Default::default()
    })
}

/// [`load_bundle_session`] over in-memory bytes (tests, embedders).
pub fn load_bundle_bytes(bytes: &[u8], origin: &Path) -> Result<(Session, String), String> {
    let entries = parse_bundle(bytes).map_err(|e| format!("{}: {e}", origin.display()))?;
    let toml = read_entry(&entries, "rut.toml")
        .map_err(|_| format!("{}: no `rut.toml` entry — not a rut bundle", origin.display()))?;
    let manifest = parse_manifest(&toml).map_err(|e| format!("{}: {e}", origin.display()))?;
    // §4: the layout version gates everything — refuse before reading
    // any other entry. v1: one module, rut sources only. v2: the whole
    // dep graph rides `<pkg>/` groups (RFC 0038 OQ-3 answered).
    if manifest.format.as_deref() != Some("rutbundle") {
        return Err(format!(
            "{}: rut.toml has no `format = \"rutbundle\"` — not a rut bundle",
            origin.display()
        ));
    }
    match manifest.format_version {
        Some(1) | Some(2) => {}
        other => {
            return Err(format!(
                "{}: unknown bundle format_version {other:?} — this loader knows versions 1 and 2",
                origin.display()
            ));
        }
    }
    let root = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{}: rut.toml has no `name`", origin.display()))?;
    let mut session = Session::new();
    // Peer groups (RFC 0045 §3) ride format_version 3 — a v1/v2 loader
    // would silently mount base-only, which is semantically wrong:
    // refuse, never guess. v3 lands with the bundle land.
    if !manifest.peer_deps.is_empty() {
        return Err(format!(
            "{}: rut.toml declares `[peer-deps]` — mounting peer groups needs bundle format_version 3 (RFC 0045 §3)",
            origin.display()
        ));
    }
    if manifest.format_version == Some(1) {
        // §1: one module per bundle in v1
        if !manifest.deps.is_empty() || !manifest.dev_deps.is_empty() {
            return Err(format!(
                "{}: rut.toml declares dependency tables — a v1 bundle is one module (RFC 0038 OQ-3)",
                origin.display()
            ));
        }
        if manifest.entry.type_path.is_some() || manifest.entry.ir.is_some() {
            return Err(format!(
                "{}: rut.toml has entry.type/entry.ir — v1 bundles carry rut sources only",
                origin.display()
            ));
        }
        let rel = manifest
            .entry
            .lib
            .as_ref()
            .ok_or_else(|| format!("{}: rut.toml has no entry.lib", origin.display()))?;
        let key = bundle_key(rel)?;
        let src = read_entry(&entries, &key)?;
        session
            .register_module(
                &root,
                Module { source: Some(src), entry: manifest.entry.clone(), ..Default::default() },
            )
            .map_err(|e| e.to_string())?;
        return Ok((session, root));
    }

    // ---- v2: the root plus every `<pkg>/` dep group, resolved by NAME
    // (the deps' `path` keys are directory-time only) ----
    let root_module = bundle_entry_module(&entries, "", &manifest)
        .map_err(|e| format!("{}: {e}", origin.display()))?;
    session
        .register_module(&root, root_module)
        .map_err(|e| e.to_string())?;
    let mut groups: std::collections::BTreeSet<String> = entries
        .iter()
        .filter_map(|(n, _)| n.split_once('/').map(|(p, _)| p.to_string()))
        .collect();
    groups.remove("rut.toml");
    for pkg in groups {
        let dep_toml = read_entry(&entries, &format!("{pkg}/rut.toml")).map_err(|_| {
            format!("{}: bundle group `{pkg}/` has no `rut.toml`", origin.display())
        })?;
        let dm = parse_manifest(&dep_toml)
            .map_err(|e| format!("{}: {pkg}/rut.toml: {e}", origin.display()))?;
        if !dm.peer_deps.is_empty() {
            return Err(format!(
                "{}: {pkg}/rut.toml declares `[peer-deps]` — mounting peer groups needs bundle format_version 3 (RFC 0045 §3)",
                origin.display()
            ));
        }
        let name =
            dm.name.clone().ok_or_else(|| format!("{}: {pkg}/rut.toml has no `name`", origin.display()))?;
        if session.resolve(&name).is_ok() {
            continue; // first mount wins (the root, an earlier group)
        }
        let m = bundle_entry_module(&entries, &format!("{pkg}/"), &dm)
            .map_err(|e| format!("{}: {e}", origin.display()))?;
        session
            .register_module(&name, m)
            .map_err(|e| e.to_string())?;
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
    Ok((session, root))
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

/// Pack a module directory into a deterministic `.rutbundle` (RFC 0038 §3):
/// `rut.toml` first, then the entry source. Same input directory ⇒
/// byte-identical bundle.
/// The manifest's entry file (lib preferred, else the `.d.rut` surface).
fn entry_rel(manifest: &crate::session::Manifest) -> Option<&String> {
    manifest.entry.lib.as_ref().or(manifest.entry.type_path.as_ref())
}

/// Collect a package's files for a bundle: its `rut.toml` (byte-for-byte)
/// plus its entry file, under `prefix` (empty for the root, `<pkg>/`
/// for a dep).
fn collect_pkg_files(
    dir: &Path,
    manifest: &crate::session::Manifest,
    prefix: &str,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(dir.join("rut.toml")).map_err(|e| e.to_string())?;
    out.push((format!("{prefix}rut.toml"), text.into_bytes()));
    let rel = entry_rel(manifest)
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    // normalize the entry's `./` prefix before the group prefix joins it
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    let key = bundle_key(&format!("{prefix}{rel}"))?;
    let src = std::fs::read_to_string(dir.join(rel))
        .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
    out.push((key, src.into_bytes()));
    Ok(())
}

/// Pack a module directory into a deterministic `.rutbundle` (RFC 0038 §3):
/// `rut.toml` first, then the entry source, then — the v2 layout — the
/// whole `[deps]` graph, each package under its own `<pkg>/` group
/// (manifest + entry), recursively and deduplicated. Same input
/// directory ⇒ byte-identical bundle.
///
/// This answers RFC 0038 OQ-3: a bundle is self-contained; its deps'
/// `path` keys are directory-time only — the loader resolves groups by
/// NAME. A v1 bundle (no deps, source-only) is the one-module special
/// case the loader still accepts.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, String> {
    let manifest = read_manifest(dir)?;
    if manifest.name.is_none() {
        return Err(format!("{} has no `name`", dir.join("rut.toml").display()));
    }
    // the packed rut.toml is the directory's rut.toml byte-for-byte, so
    // the bundle keys must already be there — directory loading ignores
    // them, but a bundle loader refuses without them (RFC 0038 §2)
    if manifest.format.as_deref() != Some("rutbundle") || manifest.format_version != Some(2) {
        return Err(format!(
            "{} is not bundle-shaped — add `format = \"rutbundle\"` and `format_version = 2`",
            dir.join("rut.toml").display()
        ));
    }
    // a v2 bundle would carry the base entry only and silently mount
    // base-only in a consumer's world — semantically wrong. Peer groups
    // pack as format_version 3 (the bundle land, phase 2 of RFC 0045).
    if !manifest.peer_deps.is_empty() {
        return Err(format!(
            "{} declares `[peer-deps]` — packing peer groups needs bundle format_version 3 (RFC 0045 §3)",
            dir.join("rut.toml").display()
        ));
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    collect_pkg_files(dir, &manifest, "", &mut entries)?;
    // the dep graph, recursively, deduplicated by package name
    fn collect_deps(
        dir: &Path,
        manifest: &crate::session::Manifest,
        out: &mut Vec<(String, Vec<u8>)>,
        seen: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        for (spec, desc) in &manifest.deps {
            if !seen.insert(spec.clone()) {
                continue;
            }
            let rel = desc
                .get("path")
                .ok_or_else(|| format!("dep `{spec}` has no `path`"))?;
            let dep_dir = dir.join(rel);
            let dm = read_manifest(&dep_dir)?;
            if dm.name.as_deref() != Some(spec.as_str()) {
                return Err(format!(
                    "dep `{spec}` points at `{}` — the manifest there names it `{}`",
                    dep_dir.display(),
                    dm.name.as_deref().unwrap_or("<unnamed>")
                ));
            }
            collect_pkg_files(&dep_dir, &dm, &format!("{spec}/"), out)?;
            collect_deps(&dep_dir, &dm, out, seen)?;
        }
        Ok(())
    }
    collect_deps(dir, &manifest, &mut entries, &mut std::collections::BTreeSet::new())?;
    write_bundle(&entries).map_err(|e| e.to_string())
}

/// Build a directory's entry [`Module`] from its manifest (RFC 0029 §5):
///
/// - `entry.lib` (with or without `entry.type`) — a SOURCE module: the
///   body compiles; the surface derives from its exports.
/// - `entry.type` ALONE — a **host pkg**: a pure declaration surface.
///   The `.d.rut` parses in declaration mode and lowers into the
///   module's host fns (RFC 0025); `host_scope`/`inline` ride the
///   manifest. No body exists — the embedding Rust binds it at run
///   time.
fn load_entry_module(dir: &Path, manifest: &crate::session::Manifest) -> Result<Module, String> {
    if manifest.entry.lib.is_none() && manifest.entry.type_path.is_some() {
        let rel = manifest.entry.type_path.as_ref().unwrap();
        let origin = format!("{}/{}", dir.display(), rel);
        let src = load_module_source(&dir.join(rel))?;
        let mut m = crate::decl::lower_decl_module(&src, &origin)?;
        m.host_scope = manifest.host_scope.clone();
        m.inline = manifest.inline;
        m.entry = manifest.entry.clone();
        return Ok(m);
    }
    let src = load_entry(dir, &manifest.entry)?;
    Ok(Module {
        source: Some(src),
        entry: manifest.entry.clone(),
        // the manifest's mount properties ride the module regardless of
        // entry shape (`inline` is ink's; `host_scope` matters only for
        // host pkgs but is harmless elsewhere)
        inline: manifest.inline,
        host_scope: manifest.host_scope.clone(),
        ..Default::default()
    })
}

fn load_entry(dir: &Path, entry: &Entry) -> Result<String, String> {
    let rel = entry
        .lib
        .as_ref()
        .or(entry.type_path.as_ref())
        .or(entry.ir.as_ref())
        .ok_or_else(|| format!("module in {} has no entry", dir.display()))?;
    load_module_source(&dir.join(rel))
}

fn read_manifest(dir: &Path) -> Result<crate::session::Manifest, String> {
    let text = std::fs::read_to_string(dir.join("rut.toml")).map_err(|e| e.to_string())?;
    parse_manifest(&text).map_err(|e| e.to_string())
}

/// Record a manifest's `[peer-deps]` into the session's registry (RFC
/// 0045): the loader reads every mounted pkg's manifest anyway, so the
/// registry costs no extra I/O.
fn record_peers(session: &mut Session, pkg: &str, manifest: &crate::session::Manifest) {
    for (peer, desc) in &manifest.peer_deps {
        session.record_peer(pkg, peer, crate::session::PeerDecl::of(desc));
    }
}

/// Resolve a manifest's `[deps]` recursively (RFC 0041 §3) — pass 1 of
/// the mount order.
fn resolve_deps(
    session: &mut Session,
    dir: &Path,
    manifest: &crate::session::Manifest,
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

/// Pass 3 (RFC 0045 §3) — the peer gate, ONE post-closure pass over the
/// recorded peer declarations:
///
/// - required peer absent → the loud D1 mount error: names the pkg, the
///   peer, and the fix. Never auto-pulled — the consumer supplies.
/// - peer present (any reason) → the pkg's group file (the descriptor's
///   `lib`, an impl-only `.rut`) is appended to its source —
///   presence-based mounting, groups in peer-name order after the base.
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
    for (pkg, peers) in session.peer_decls() {
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
                            "pkg `{pkg}`'s [peer-deps] entry `{peer}` points at `{path}` — the manifest there names it `{actual}` (a packaging bug in {pkg}; RFC 0045 §3)",
                            path = decl.path,
                            actual = dm.name.as_deref().unwrap_or("<unnamed>"),
                        ));
                    }
                    Err(_) => {
                        return Err(format!(
                            "pkg `{pkg}`'s [peer-deps] entry `{peer}` points at `{path}` — cannot read a manifest there (a packaging bug in {pkg}; RFC 0045 §3)",
                            path = decl.path,
                        ));
                    }
                }
            }
            if session.resolve(peer).is_err() {
                if decl.optional {
                    continue; // inert — the group simply never mounts
                }
                // D1 (RFC 0045 §3): loud at mount, naming pkg + peer + fix
                return Err(format!(
                    "pkg `{pkg}` requires the peer `{peer}`, and `{peer}` is not in this program's closure — peers are not pulled transitively: add `{peer} = {{ path = \"..\" }}` to your `rut.toml` `[deps]` (RFC 0045 §3)"
                ));
            }
            let Some(lib) = &decl.lib else {
                continue; // presence declared, no integration file to mount
            };
            let group_path = pkg_dir.join(lib);
            let text = std::fs::read_to_string(&group_path).map_err(|_| {
                format!(
                    "pkg `{pkg}`'s [peer-deps] entry `{peer}` names the group `{lib}` — cannot read it (a packaging bug in {pkg}; RFC 0045 §3)"
                )
            })?;
            appends.push((pkg.clone(), text));
        }
    }
    for (pkg, text) in appends {
        session.append_source(&pkg, &text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Mount one package directory — and, recursively, its `[deps]` — into
/// an EXISTING session: the programmatic counterpart of a manifest's
/// dep walk (native hosts, tests, plugin loaders). Returns the
/// package's own name. A name already mounted wins (RFC 0029 §4).
///
/// This is an OFFER to someone else's program, not "building the pkg
/// itself": no dev-deps are mounted (pass 2 is root-only) and the peer
/// gate does not run here — the embedder's world grows incrementally,
/// so presence is a program-closure property the program's own
/// `load_dir_session`/compile owns (RFC 0045 §3). The pkg's peer
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
    record_peers(session, &name, &manifest);
    let mut visiting = vec![dir.to_path_buf()];
    let mut mounted = BTreeMap::new(); // the gate's map — not this path's pass
    resolve_deps(session, dir, &manifest, &mut visiting, &mut mounted)?;
    Ok(name)
}

/// Read a directory's `rut.toml` graph and compile it to one linked program.
pub fn compile_dir(dir: &Path) -> Result<crate::graph::GraphOutput, String> {
    let (session, root) = load_dir_session(dir)?;
    Ok(crate::compile_graph(&session, &root))
}
