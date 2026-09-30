//! Filesystem loader — read a module directory (`rut.toml`) or a packed
//! `.rutbundle` into a [`Session`].
//!
//! One module is one directory with ONE entry file: its `use` statements
//! are all inter-module paths (`use <pkg>::{A, B};`), resolved by the
//! `Session` — there is no intra-module include form. A `.rutbundle` is
//! the same contract zipped: the manifest plus the compiled root binary,
//! its scope ledger, and each dep group as a compiled `.rutc` or a
//! source file set. A `[deps]` row may also declare a **url**: a remote
//! `.rutbundle` the CALL SITE fetches ([`DepFetch`]) and the loader
//! mounts — the sha256 pin is manifest law, verified here at the mount
//! door on every load.
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); this
//! native helper is the counterpart that reads files.

use std::collections::BTreeMap;
use std::future::Future;
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

/// HOW url-dep bytes arrive — the call site's half of the layer split.
/// Transport, caching, and offline policy are the embedder's; the
/// loader owns only WHAT the bytes are declared to be (the `sha256`
/// pin, verified at the mount door on every load — fresh fetch, cache
/// hit, vendored map, test fixture).
///
/// Explicit `-> impl Future`, deliberately NOT `async fn`: the same
/// RPITIT mechanics, but the `Send` knob stays expressible at the
/// definition, and sync impls (cache hits, test fixtures) are
/// first-class via `std::future::ready`. Deliberately NOT `+ Send`:
/// JS-backed futures (a browser fetch bridge) are `!Send`, and the
/// loader stays wasm-clean — the CLI blocks on one thread, a spawning
/// host wraps at its own boundary. Static dispatch via generics
/// (RPITIT is not dyn-compatible); no async-trait crate.
pub trait DepFetch {
    fn dep_fetch(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, String>>;
}

/// The hex sha256 of `bytes` — the pin law's arithmetic. Public so a
/// call site can key its cache by url (the CLI does) without a sha2
/// dependency of its own.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// One url dep row collected by the phase-1 walk: the declaring key,
/// the url, and the pin (normalized lowercase by the grammar, `None`
/// when unpinned).
struct UrlDep {
    spec: String,
    url: String,
    pin: Option<String>,
}

/// One fetched archive's decoded entries, kept by the CALLER (the
/// session stays I/O-free — it records only where files live: slot +
/// prefix). `origin` is the url, for error messages.
#[derive(Debug)]
pub struct Archive {
    pub origin: String,
    pub entries: Vec<(String, Vec<u8>)>,
}

/// Where a mounted pkg's files live — the peer gate's group reads
/// dispatch on it: a `Dir` pkg reads from the directory, an `Archive`
/// pkg from the fetched archive's entries at its prefix.
#[derive(Clone, Debug)]
pub(crate) enum PkgSource {
    Dir(std::path::PathBuf),
    Archive { slot: usize, prefix: String },
}

/// The url-dep context the sync core walks with: the fetched bytes
/// (url → bytes) and the archives those bytes opened (slot-ordered).
/// The OLD sync wrappers pass an empty map — a url row there is the
/// loud no-fetcher error, never a network call (the loader never
/// fetches; bundles stay closed at load, url rows are leaves).
struct Fetched<'a> {
    map: &'a BTreeMap<String, Vec<u8>>,
    archives: &'a mut Vec<Archive>,
    /// defense-in-depth cycle guard: url deps are leaves, so no
    /// recursion can re-enter a url — the guard keeps that true even
    /// if the walk ever grows one.
    visiting_urls: std::collections::BTreeSet<String>,
}

/// Phase 1 of the two-phase design — SYNC: walk the root manifest
/// (`[deps]` + `[dev-deps]`) and every directory dep's manifest,
/// collecting the url rows. Url deps are LEAVES — the walk never
/// follows them; directory recursion is cycle-guarded by canonical
/// dirs.
fn collect_url_deps(dir: &Path) -> Result<Vec<UrlDep>, String> {
    let mut out = Vec::new();
    let mut visiting = Vec::new();
    collect_url_deps_walk(dir, &mut visiting, &mut out)?;
    Ok(out)
}

fn collect_url_deps_walk(
    dir: &Path,
    visiting: &mut Vec<std::path::PathBuf>,
    out: &mut Vec<UrlDep>,
) -> Result<(), String> {
    let canonical = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    if visiting.contains(&canonical) {
        return Err(format!(
            "cyclic use: `{}` is already being scanned for url deps",
            dir.display()
        ));
    }
    visiting.push(canonical);
    let manifest = read_manifest(dir)?;
    // [deps] and [dev-deps]: the tables the loader and the packer's
    // compile-once-per-owner pass walk. A dep's dev table stays
    // unmounted at load (pass 2 is root-only); the collected url rows
    // for it only ever matter to the pack lane, and unused map entries
    // are inert.
    for table in [&manifest.deps, &manifest.dev_deps] {
        for (spec, desc) in table {
            if let Some(url) = desc.get("url") {
                out.push(UrlDep {
                    spec: spec.clone(),
                    url: url.clone(),
                    pin: desc.get("sha256").cloned(),
                });
            } else if let Some(rel) = desc.get("path") {
                let dep_dir = dir.join(rel);
                collect_url_deps_walk(&dep_dir, visiting, out)?;
            }
        }
    }
    visiting.pop();
    Ok(())
}

/// Phase 2 — the async join: await `dep_fetch` per url row,
/// SEQUENTIALLY, into the bytes map the sync core reads. The map is the
/// join (a call site wanting concurrency does it inside its
/// `dep_fetch`); equal urls fetch once.
pub(crate) async fn prefetch_urls(
    dir: &Path,
    fetch: &impl DepFetch,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let rows = collect_url_deps(dir)?;
    let mut map = BTreeMap::new();
    for row in &rows {
        if map.contains_key(&row.url) {
            continue;
        }
        let bytes = fetch.dep_fetch(&row.url).await?;
        map.insert(row.url.clone(), bytes);
    }
    Ok(map)
}

/// The no-fetcher law: a url row in a loader that has no bytes for it.
/// Names the FIX, matching the D-style diagnostics.
fn no_fetcher(spec: &str, url: &str) -> String {
    format!(
        "dep `{spec}` is declared by url (`{url}`) — this loader has no `dep_fetch`: call \
         `load_dir_session_with` / `pack_dir_with` (the CLI does), or vendor the dep"
    )
}

/// The pkg → source map the peer gate walks: the directory walk's own
/// map, plus every archive mount the session recorded (first mount
/// wins — an archive group shadowed by an earlier dir mount keeps the
/// dir as its source).
fn sources_of(
    session: &Session,
    dirs: &BTreeMap<String, std::path::PathBuf>,
) -> BTreeMap<String, PkgSource> {
    let mut sources: BTreeMap<String, PkgSource> = dirs
        .iter()
        .map(|(k, v)| (k.clone(), PkgSource::Dir(v.clone())))
        .collect();
    for (pkg, (slot, prefix)) in session.archive_mounts() {
        sources.entry(pkg.clone()).or_insert(PkgSource::Archive {
            slot: *slot,
            prefix: prefix.clone(),
        });
    }
    sources
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
    // the back-compat lane: no fetcher, empty map — a url dep here is
    // the loud no-fetcher error, never a network call
    load_dir_session_fetched(dir, &BTreeMap::new()).map(|loaded| (loaded.session, loaded.root))
}

/// The result of the fetched dir core: the session, its root spec, and
/// the archives the url deps opened (slot-ordered) — pass 3's archive
/// group reads and the packer's rode-along copies index into the list.
#[derive(Debug)]
pub struct LoadedDir {
    pub session: Session,
    pub root: String,
    pub archives: Vec<Archive>,
}

/// [`load_dir_session`] over PRE-FETCHED url bytes — the pub sync core
/// the `*_with` lanes call after `prefetch_urls`, and the fixture lane
/// tests use directly. Every url row's `sha256` pin is verified HERE,
/// at the mount door, on every load.
pub fn load_dir_session_fetched(dir: &Path, map: &BTreeMap<String, Vec<u8>>) -> Result<LoadedDir, String> {
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
    let mut archives: Vec<Archive> = Vec::new();
    {
        let mut fetched = Fetched {
            map,
            archives: &mut archives,
            visiting_urls: Default::default(),
        };
        resolve_table(&mut session, dir, &manifest.deps, &mut visiting, &mut mounted, &mut fetched)?;
        resolve_table(&mut session, dir, &manifest.dev_deps, &mut visiting, &mut mounted, &mut fetched)?;
    }
    let sources = sources_of(&session, &mounted);
    run_peer_gate(&mut session, &root, &sources, &archives)?;
    Ok(LoadedDir { session, root, archives })
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
    // pass 3 — the ONE peer gate: the archive's own pkgs are its
    // sources (slot 0), so a source declarer's group file reads from
    // the archive exactly as a dir declarer's reads from disk
    let mut sources: BTreeMap<String, PkgSource> = prefixes
        .iter()
        .map(|(pkg, prefix)| {
            (
                pkg.clone(),
                PkgSource::Archive {
                    slot: 0,
                    prefix: prefix.clone(),
                },
            )
        })
        .collect();
    let archives = [Archive {
        origin: origin.display().to_string(),
        entries: entries.to_vec(),
    }];
    run_peer_gate(&mut session, &root_spec, &sources, &archives)?;
    Ok((session, root_spec))
}

/// Pass 3 — the peer gate, ONE post-closure pass over the recorded
/// peer declarations, TWO source kinds:
///
/// - required peer absent → the loud D1 mount error: names the pkg, the
///   peer, and the fix. Never auto-pulled.
/// - peer present (any reason) → the pkg's group file (the descriptor's
///   `lib`, an impl-only `.rut`) is recorded for the graph to compile
///   INTO the declarer's unit (groups in peer-name order after the
///   base) — presence-based mounting; the mounted body stays pristine.
/// - optional peer absent → inert; the group simply never mounts.
///
/// Group reads dispatch on [`PkgSource`]: a `Dir` declarer's file is
/// read from its directory; an `Archive` declarer's from the fetched
/// archive's entries at its prefix (the same law, one gate — the old
/// dir lane and bundle lane never disagreed on presence, only on where
/// files live). A COMPILED declarer's rows are already in its `.rutc`
/// (the pack-time closure's dev-deps supplied the peers — compile once
/// per owner), so only the presence law runs.
///
/// The program root's own peer paths are read and name-checked even
/// when dev-deps already supplied presence — a broken path is the loud
/// D3 packaging-bug error at the pkg's own build (matrix row 6). The
/// check is Dir-ONLY law: an archive root has no directories, its
/// `path` keys never crossed the pack boundary, and presence is by
/// name. A dep's peer paths are never read: presence is by NAME
/// (first-mount-wins already guarantees the consumer's own path won),
/// so a broken peer path is inert for an optional peer and unreachable
/// for a required one (its absence is D1's business, not the path's).
pub(crate) fn run_peer_gate(
    session: &mut Session,
    root: &str,
    sources: &BTreeMap<String, PkgSource>,
    archives: &[Archive],
) -> Result<(), String> {
    // collected first, applied after — the registry borrows the session
    let mut appends: Vec<(String, String)> = Vec::new();
    let mut pre_compiled: Vec<String> = Vec::new();
    for (pkg, peers) in session.peer_decls() {
        if session.groups_mounted(pkg) {
            continue; // an earlier gate pass over this session mounted them
        }
        // a compiled declarer's rows ride its binary — the presence law
        // still runs below (D1), but there is nothing to read, and the
        // declarer is done after this pass
        let compiled = matches!(session.resolve(pkg).map(|m| &m.body), Ok(ModuleBody::Compiled(_)));
        let source = sources.get(pkg);
        for (peer, decl) in peers {
            if pkg.as_str() == root {
                if let Some(PkgSource::Dir(pkg_dir)) = source {
                    // self-build: the path must resolve and name the
                    // peer — even when dev-deps already supplied
                    // presence (D3 is directory-time law)
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
            if compiled {
                continue; // rows ride the binary — nothing to read
            }
            let Some(lib) = &decl.lib else {
                continue; // presence declared, no integration file to mount
            };
            let Some(source) = source else {
                return Err(format!("pkg `{pkg}` declares `[peer-deps]` but is not mounted"));
            };
            let text = match source {
                PkgSource::Dir(pkg_dir) => {
                    let group_path = pkg_dir.join(lib);
                    std::fs::read_to_string(&group_path).map_err(|_| {
                        format!(
                            "pkg `{pkg}`'s [peer-deps] entry `{peer}` names the group `{lib}` — cannot read it (a packaging bug in {pkg})"
                        )
                    })?
                }
                PkgSource::Archive { slot, prefix } => {
                    let archive = &archives[*slot];
                    let rel = lib.strip_prefix("./").unwrap_or(lib);
                    let key = bundle_key(&format!("{prefix}{rel}"))
                        .map_err(|e| format!("{}: {e}", archive.origin))?;
                    read_entry(&archive.entries, &key).map_err(|_| {
                        format!(
                            "pkg `{pkg}`'s [peer-deps] entry `{peer}` names the group `{lib}` — cannot read it (a packaging bug in {pkg})"
                        )
                    })?
                }
            };
            appends.push((pkg.clone(), text));
        }
        if compiled {
            pre_compiled.push(pkg.clone());
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
    mount_dev_table_fetched(session, dir, manifest, &BTreeMap::new())
}

/// [`mount_dev_table`] over pre-fetched url bytes — the pack lane's
/// flavor (a dir group's dev table may pin url deps of its own).
pub(crate) fn mount_dev_table_fetched(
    session: &mut Session,
    dir: &Path,
    manifest: &Manifest,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let mut visiting = Vec::new();
    let mut mounted = BTreeMap::new();
    let mut archives: Vec<Archive> = Vec::new();
    let mut fetched = Fetched {
        map,
        archives: &mut archives,
        visiting_urls: Default::default(),
    };
    resolve_table(session, dir, &manifest.dev_deps, &mut visiting, &mut mounted, &mut fetched)
}

/// Resolve a manifest's `[deps]` recursively — pass 1 of
/// the mount order.
fn resolve_deps(
    session: &mut Session,
    dir: &Path,
    manifest: &Manifest,
    visiting: &mut Vec<std::path::PathBuf>,
    mounted: &mut BTreeMap<String, std::path::PathBuf>,
    fetched: &mut Fetched<'_>,
) -> Result<(), String> {
    resolve_table(session, dir, &manifest.deps, visiting, mounted, fetched)
}

/// Walk one descriptor table — the `[deps]` walk (pass 1; pass 2 feeds
/// it the root's `[dev-deps]`, which mounts exactly the same way).
/// Each entry is a relative `path` to a package directory, loaded and
/// mounted under its key — or a `url`, whose pre-fetched bytes mount
/// through [`mount_url_dep`] (the sha256 pin is law at that door; the
/// walk NEVER fetches, and a url dep is a leaf). **First mount wins** —
/// a name already in the session (the embedder's, the root's, or an
/// earlier dep's) is never overwritten; a dep whose manifest `name`
/// disagrees with its key is an error naming both. `visiting` guards
/// cycles. Every mounted pkg's directory (or archive location) and
/// `[peer-deps]` declarations are recorded for pass 3.
fn resolve_table(
    session: &mut Session,
    dir: &Path,
    table: &BTreeMap<String, BTreeMap<String, String>>,
    visiting: &mut Vec<std::path::PathBuf>,
    mounted: &mut BTreeMap<String, std::path::PathBuf>,
    fetched: &mut Fetched<'_>,
) -> Result<(), String> {
    for (spec, desc) in table {
        if session.resolve(spec).is_ok() {
            continue; // already mounted — the embedder's (or an earlier) mount wins
        }
        if let Some(url) = desc.get("url") {
            let bytes = fetched
                .map
                .get(url)
                .ok_or_else(|| no_fetcher(spec, url))?;
            if !fetched.visiting_urls.insert(url.clone()) {
                return Err(format!("cyclic use: `{spec}` ({url}) is already being loaded"));
            }
            mount_url_dep(
                session,
                spec,
                url,
                desc.get("sha256").map(String::as_str),
                bytes,
                fetched.archives,
            )?;
            fetched.visiting_urls.remove(url);
            continue;
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
        resolve_deps(session, &dep_dir, &dm, visiting, mounted, fetched)?;
        visiting.pop();
    }
    Ok(())
}

/// Mount a url dep's fetched bytes — the mount door where the pin is
/// LAW. The fetcher saw only a url; the manifest row's `sha256` and
/// the bytes meet HERE, on EVERY load (fresh fetch, cache hit,
/// vendored map, test fixture) — a check the call site performed would
/// be a check the call site could skip. Gate order mirrors
/// [`load_bundle_bytes`]:
///
/// 0. the pin: `sha256_hex(bytes)` must equal the row's pin (lowercase
///    by grammar law) — a mismatch names the dep, the url, and BOTH
///    hashes;
/// 1. the container (every entry CRC-verified) — origin is the url;
/// 2. the layout: manifest + exact `format_version = 5`, every group
///    decode + verified;
/// 3. name-vs-key: the bundle's `name` must equal the `[deps]` key
///    (the path flavor's twin);
/// 4. the scope-ledger MERGE: rows record; a row colliding with a
///    DIFFERENT spec is a corrupt or doctored closure — refuse;
/// 5. the root mounts Compiled (first-mount-wins); `record_peers`;
/// 6. the group loop = `load_bundle_bytes`'s (first-mount-wins,
///    own-scope vs ledger consistency for compiled groups,
///    `bundle_entry_module` for source groups) — no peer gate inline:
///    archives record for the ONE pass-3 gate;
/// 7. the closure check against the COMBINED session — bundles are
///    closed: every declared dep rides in-archive (or mounted earlier).
///    No fetching at bundle load, ever.
fn mount_url_dep(
    session: &mut Session,
    spec: &str,
    url: &str,
    pin: Option<&str>,
    bytes: &[u8],
    archives: &mut Vec<Archive>,
) -> Result<(), String> {
    if session.resolve(spec).is_ok() {
        return Ok(()); // first mount wins — the embedder's (or an earlier) mount
    }
    // gate 0 — the pin law
    if let Some(pin) = pin {
        let got = sha256_hex(bytes);
        if got != pin {
            return Err(format!(
                "dep `{spec}` — sha256 pin mismatch for {url}: the manifest pins {pin}, the fetched bytes hash to {got}"
            ));
        }
    }
    // gate 1 — the container
    let bundle = Bundle::parse(bytes).map_err(|e| format!("{url}: {e}"))?;
    // gate 2 — the layout: manifest, exact version, every group decoded
    let layout = Layout::parse(&bundle).map_err(|e| format!("{url}: {e}"))?;
    let Layout::Compiled { manifest, root, scopes, groups } = layout;
    // gate 3 — name-vs-key
    let root_spec = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{url}: rut.toml has no `name`"))?;
    if root_spec != spec {
        return Err(format!(
            "dep `{spec}` points at {url} — the bundle names itself `{root_spec}`"
        ));
    }
    // gate 4 — the ledger merge: two archives share one session; a
    // scope row that flips owners mid-merge would misroute every id
    // rebased through it — corrupt or doctored, refuse
    for (scope, row_spec) in &scopes {
        if let Some(existing) = session.bundle_scope_row(*scope) {
            if existing != *row_spec {
                return Err(format!(
                    "{url}: the scope ledger maps {scope} to `{row_spec}`, but this program already maps it to `{existing}` — the closure is corrupt or doctored"
                ));
            }
        }
        session.record_bundle_scope(*scope, row_spec);
    }
    // gate 5 — the root: a compiled module (the packer refuses any other)
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
    record_peers(session, &root_spec, &manifest);
    // gate 6 — the group loop, no peer gate inline
    let entries = bundle.entries().to_vec();
    let slot = archives.len();
    let mut prefixes: BTreeMap<String, String> = BTreeMap::new();
    prefixes.insert(root_spec.clone(), String::new());
    for (prefix, kind) in &groups {
        let dep_toml =
            read_entry(&entries, &format!("{prefix}/rut.toml")).map_err(|e| format!("{url}: {e}"))?;
        let dm = parse_manifest(&dep_toml).map_err(|e| format!("{url}: {prefix}/rut.toml: {e}"))?;
        let name = dm
            .name
            .clone()
            .ok_or_else(|| format!("{url}: {prefix}/rut.toml has no `name`"))?;
        if session.resolve(&name).is_ok() {
            continue; // first mount wins (an earlier archive, or a dir dep)
        }
        let module = match kind {
            GroupKind::Compiled(program) => {
                // the ledger must name the group, and the row must be
                // the scope the binary itself carries (refuse, never
                // guess — a mismatch is a corrupt or doctored bundle)
                let Some(own) = rut_core::link::own_scope(program) else {
                    return Err(format!(
                        "{url}: {prefix}/{}: the program carries no scope blocks",
                        name
                    ));
                };
                match scopes.iter().find(|(_, s)| s == &name) {
                    Some(&(row, _)) if row == own => {}
                    Some(&(row, _)) => {
                        return Err(format!(
                            "{url}: `{name}`'s ledger row says scope {row}, but its binary carries {own}"
                        ));
                    }
                    None => {
                        return Err(format!(
                            "{url}: the scope ledger does not name `{name}` — the bundle is incomplete"
                        ));
                    }
                }
                Module {
                    body: ModuleBody::Compiled(program.clone()),
                    entry: dm.entry.clone(),
                    ..Default::default()
                }
            }
            GroupKind::Source => bundle_entry_module(&entries, &format!("{prefix}/"), &dm)
                .map_err(|e| format!("{url}: {e}"))?,
        };
        session
            .register_module(&name, module)
            .map_err(|e| e.to_string())?;
        record_peers(session, &name, &dm);
        prefixes.insert(name.clone(), format!("{prefix}/"));
    }
    // where this archive's mounted pkgs' files live (pass 3's reads,
    // the packer's rode-along copies — the session holds locations only)
    session.record_archive_mounts(&prefixes, slot);
    archives.push(Archive {
        origin: url.to_string(),
        entries,
    });
    // gate 7 — the closure check, against the COMBINED session: a
    // bundle's dep rides in-archive; if it is neither here nor mounted
    // earlier, the bundle is incomplete (a url row inside it is inert
    // metadata — the loader never fetches at bundle load)
    for dep in manifest.deps.keys() {
        if session.resolve(dep).is_err() {
            return Err(format!(
                "{url}: the bundle is missing its `{dep}` dependency group"
            ));
        }
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
    mount_dir_fetched(session, dir, &BTreeMap::new())
}

/// [`mount_dir`] over pre-fetched url bytes — the `mount_dir_with`
/// lane's sync core. The gate does not run here (the embedder's world
/// grows incrementally); archive mounts are recorded on the session, so
/// a later `assemble_peers` sees them — note it reads group files from
/// the archives the CALLER kept (an embedder that mounts url deps
/// mount-by-mount owns its archive list; the load/pack lanes run the
/// gate themselves).
pub(crate) fn mount_dir_fetched(
    session: &mut Session,
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<String, String> {
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
    let mut archives: Vec<Archive> = Vec::new();
    let mut fetched = Fetched {
        map,
        archives: &mut archives,
        visiting_urls: Default::default(),
    };
    resolve_deps(session, dir, &manifest, &mut visiting, &mut mounted, &mut fetched)?;
    Ok(name)
}

/// The peer gate for sessions built mount-by-mount: runs
/// the gate's append pass over the peer declarations every `mount_dir`/
/// dep walk recorded, using the pkg→location map those mounts left
/// behind (dirs from `record_peer_dir`, archives from the url dep
/// mounts). The CLI's and the probe's single-file convenience lanes
/// call this after their tree mounts, so a loose file that `use json::`
/// gets the peer-gated container groups exactly like a module-dir
/// program does. Presence-based as ever: an optional peer absent is
/// inert; a required peer absent is the loud D1 error. Groups already
/// mounted by an earlier gate pass over this session are skipped —
/// never double-appended. NOTE the archive half: this lane owns no
/// archive bytes, so an archive-mounted declarer's group file read
/// fails loudly — the load/pack lanes run the gate themselves; embedders
/// that mount url deps mount-by-mount run `run_peer_gate` with their own
/// archive list (crate-internal).
pub fn assemble_peers(session: &mut Session) -> Result<(), String> {
    let sources = sources_of(session, session.peer_dirs());
    run_peer_gate(session, "", &sources, &[])
}

/// [`load_dir_session`] with a url-dep fetcher — the `*_with` lane: HOW
/// bytes arrive is the fetcher's (transport, cache, offline policy);
/// the loader still owns WHAT they are (the pin, at the mount door).
/// The walk collects the url rows, awaits `dep_fetch` per url
/// sequentially, then mounts over the bytes map.
pub async fn load_dir_session_with(
    dir: &Path,
    fetch: &impl DepFetch,
) -> Result<(Session, String), String> {
    let map = prefetch_urls(dir, fetch).await?;
    load_dir_session_fetched(dir, &map).map(|loaded| (loaded.session, loaded.root))
}

/// [`load_path_session`] with a url-dep fetcher. A `.rutbundle` path
/// never fetches — bundles are closed; only a directory's `[deps]` can
/// name urls.
pub async fn load_path_session_with(
    path: &Path,
    fetch: &impl DepFetch,
) -> Result<(Session, String), String> {
    if path.is_dir() {
        return load_dir_session_with(path, fetch).await;
    }
    if path.extension().map_or(false, |e| e == "rutbundle") {
        return load_bundle_session(path);
    }
    Err(format!(
        "{} is neither a module directory (no `rut.toml`) nor a `.rutbundle`",
        path.display()
    ))
}

/// [`mount_dir`] with a url-dep fetcher — collects the url rows,
/// fetches, mounts over the bytes map. The gate still does not run
/// here (the offer law).
pub async fn mount_dir_with(
    session: &mut Session,
    dir: &Path,
    fetch: &impl DepFetch,
) -> Result<String, String> {
    let map = prefetch_urls(dir, fetch).await?;
    mount_dir_fetched(session, dir, &map)
}

/// Read a directory's `rut.toml` graph and compile it to one linked program.
pub fn compile_dir(dir: &Path) -> Result<crate::graph::GraphOutput, String> {
    let (session, root) = load_dir_session(dir)?;
    Ok(crate::compile_graph(&session, &root))
}

/// Restore a symbol table's names and positions into a mounted
/// session — the load half of compile-time symbol stripping. Each
/// section resolves its module **spec** through the session (never an
/// archive path: first-mount-wins has already decided the bodies) and
/// applies to `ModuleBody::Compiled` programs; the names restore first,
/// so linking (merge-by-name, ledger keys) and the VM both see the real
/// thing. Call BEFORE `compile_graph`. Sections whose spec is absent
/// (or not a compiled module) come back as skipped specs — the caller
/// decides to warn; a map from a different build simply matches
/// nothing, which is tolerated, not an error.
pub fn apply_symbols_to_session(
    session: &mut Session,
    map: &rut_core::strip::SymbolMap,
) -> Vec<String> {
    // names first: exact-key restore over every mounted compiled module
    // (kept strings and non-keys pass through)
    let restore: std::collections::HashMap<&str, &str> = map
        .names
        .iter()
        .map(|(m, o)| (m.as_str(), o.as_str()))
        .collect();
    let specs: Vec<String> = session.modules().map(|(s, _)| s.clone()).collect();
    for spec in &specs {
        if let Ok(module) = session.resolve_mut(spec) {
            if let ModuleBody::Compiled(prog) = &mut module.body {
                prog.interner.remap_tail(|s| {
                    restore.get(s).copied().unwrap_or(s).to_string()
                });
            }
        }
    }
    // then the span/pos sections, spec-resolved like the names
    let mut skipped: Vec<String> = Vec::new();
    for section in &map.sections {
        match session.resolve_mut(&section.spec) {
            Ok(module) => match &mut module.body {
                ModuleBody::Compiled(prog) => {
                    // a fn-count mismatch means the section was taken
                    // from a different build — matches nothing,
                    // tolerated silently (the same law as the name rows)
                    if prog.funcs.len() == section.fns.len() {
                        for (f, sy) in prog.funcs.iter_mut().zip(&section.fns) {
                            f.spans = sy.spans.clone();
                            f.pos = sy.pos.clone();
                        }
                    }
                }
                _ => skipped.push(section.spec.clone()),
            },
            Err(_) => skipped.push(section.spec.clone()),
        }
    }
    skipped
}
