//! Filesystem loader — read a module directory (`rut.jsonc`) or a packed
//! `.rutbundle` into a [`Session`].
//!
//! One module is one directory with ONE entry file: its `use` statements
//! are all inter-module paths (`use <pkg>::{A, B};`), resolved by the
//! `Session` — there is no intra-module include form. A `.rutbundle` is
//! the same contract zipped: the manifest plus the compiled root binary,
//! its scope ledger, and each dep group as a compiled `.rutc` or a
//! source file set. A `[deps]` row may also declare a **url**: a remote
//! `.rutbundle` the CALL SITE fetches ([`DepRemote`]) and the loader
//! mounts — the sha256 pin is manifest law, verified here at the mount
//! door on every load.
//!
//! The `Session` itself does no I/O (wasm hosts mount in memory); this
//! native helper is the counterpart that reads files — over the
//! embedder's [`Source`] (the default: the real filesystem).

mod error;
mod remote;

pub(crate) use error::LoadError;
pub use error::RemoteError;
pub use remote::{DepRemote, HttpRemote};

use std::collections::BTreeMap;
use std::path::Path;

use crate::bundle::{
    bundle_key, entry_rel, parse_manifest, read_entry, Bundle, Entry, FsSource, GroupKind, Layout,
    Manifest, PkgType, Source,
};
use crate::run::{Loaded, RunError};
use crate::session::{Pkg, PkgBody, PeerDecl, Session};
use std::sync::atomic::{AtomicU64, Ordering};

/// The walk sequence: one id per walk — archive slots are walk-local,
/// so the chain's per-archive namespacing groups by (walk, slot).
static WALK_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_walk_id() -> u64 {
    WALK_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Stamp every pkg of the session with the walk's id (the archive
/// group identity).
fn stamp(session: &mut Session, walk: u64) {
    for (_, m) in session.modules() {
        let _ = m;
    }
    let specs: Vec<String> = session.modules().map(|(s, _)| s.clone()).collect();
    for spec in specs {
        if let Ok(m) = session.resolve_mut(&spec) {
            m.walk = walk;
        }
    }
}

/// Read a directory's `rut.jsonc` — the real-filesystem lane of
/// [`crate::bundle::read_manifest`] (the module itself never touches the
/// filesystem).
fn read_manifest(dir: &Path, src: &dyn Source) -> Result<Manifest, LoadError> {
    crate::bundle::read_manifest(dir, src).map_err(LoadError::law)
}

/// Read one file's text through the embedder's [`Source`]. The FsSource
/// default spells today's messages: a missing file is
/// `cannot read <path>: <io>`, a non-UTF-8 body is
/// `cannot read <path>: stream did not contain valid UTF-8` — the
/// text `read_to_string` produced.
fn read_source_text(src: &dyn Source, path: &Path) -> Result<String, LoadError> {
    let bytes = src
        .read(path)
        .map_err(|e| LoadError::law(format!("cannot read {}: {e}", path.display())))?;
    String::from_utf8(bytes).map_err(|_| {
        LoadError::law(format!(
            "cannot read {}: stream did not contain valid UTF-8",
            path.display()
        ))
    })
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

/// One url dep row collected by the phase-1 walk: the url (the pin is
/// re-read from the manifest row at the mount door).
struct UrlDep {
    url: String,
}

/// One fetched archive's decoded entries, kept by the CALLER (the
/// session stays I/O-free — it records only where files live: slot +
/// prefix). `origin` is the url, for error messages.
#[derive(Clone, Debug)]
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
fn collect_url_deps(dir: &Path, src: &dyn Source) -> Result<Vec<UrlDep>, LoadError> {
    let mut out = Vec::new();
    let mut visiting = Vec::new();
    collect_url_deps_walk(dir, src, &mut visiting, &mut out)?;
    Ok(out)
}

fn collect_url_deps_walk(
    dir: &Path,
    src: &dyn Source,
    visiting: &mut Vec<std::path::PathBuf>,
    out: &mut Vec<UrlDep>,
) -> Result<(), LoadError> {
    let canonical = dir
        .canonicalize()
        .map_err(|e| LoadError::law(format!("cannot resolve {}: {e}", dir.display())))?;
    if visiting.contains(&canonical) {
        return Err(LoadError::law(format!(
            "cyclic use: `{}` is already being scanned for url deps",
            dir.display()
        )));
    }
    visiting.push(canonical);
    let manifest = read_manifest(dir, src)?;
    // [deps] and [dev-deps]: the tables the loader and the packer's
    // compile-once-per-owner pass walk. A dep's dev table stays
    // unmounted at load (pass 2 is root-only); the collected url rows
    // for it only ever matter to the pack lane, and unused map entries
    // are inert.
    for table in [&manifest.deps, &manifest.dev_deps] {
        for (spec, desc) in table {
            if let Some(url) = desc.get("url") {
                out.push(UrlDep { url: url.clone() });
                let _ = spec; // the mount door re-keys off the manifest row
            } else if let Some(rel) = desc.get("path") {
                let dep_dir = dir.join(rel);
                collect_url_deps_walk(&dep_dir, src, visiting, out)?;
            }
        }
    }
    visiting.pop();
    Ok(())
}

/// Phase 2 — the async join: await `fetch` per url row, SEQUENTIALLY,
/// into the bytes map the sync core reads. The map is the join (a call
/// site wanting concurrency does it inside its `fetch`); equal urls
/// fetch once.
pub(crate) async fn prefetch_urls(
    dir: &Path,
    src: &dyn Source,
    remote: &dyn DepRemote,
) -> Result<BTreeMap<String, Vec<u8>>, LoadError> {
    let rows = collect_url_deps(dir, src)?;
    let mut map = BTreeMap::new();
    for row in &rows {
        if map.contains_key(&row.url) {
            continue;
        }
        let bytes = remote.fetch(&row.url).await?;
        map.insert(row.url.clone(), bytes);
    }
    Ok(map)
}


/// The no-fetcher law: a url row in a loader that has no bytes for it.
/// Names the FIX, matching the D-style diagnostics.
fn no_fetcher(spec: &str, url: &str) -> LoadError {
    LoadError::law(format!(
        "dep `{spec}` is declared by url (`{url}`) — this loader has no `dep_fetch`: call \
         `load_dir_with` / `pack_dir_with` (the CLI does), or vendor the dep"
    ))
}

/// The pkg → source map the peer gate walks, read off the mounted
/// pkgs' own bookkeeping: a walked dir pkg carries its directory, a
/// url-mounted pkg its archive location (first mount wins — an
/// archive group shadowed by an earlier dir mount keeps the dir as
/// its source).
fn sources_of(session: &Session) -> BTreeMap<String, PkgSource> {
    let mut sources: BTreeMap<String, PkgSource> = BTreeMap::new();
    for (spec, m) in session.modules() {
        if let Some(dir) = &m.dir {
            sources.insert(spec.clone(), PkgSource::Dir(dir.clone()));
        } else if let Some((slot, prefix)) = &m.archive {
            sources.insert(
                spec.clone(),
                PkgSource::Archive { slot: *slot, prefix: prefix.clone() },
            );
        }
    }
    sources
}

/// Read one module source file. The language has no include form to
/// expand (use paths are inter-module) — but a module may
/// be AUTHORED as several files: the loader splices `entry.libs` into
/// one source, which is assembly, not expansion.
pub fn load_module_source(path: &Path) -> Result<String, LoadError> {
    std::fs::read_to_string(path).map_err(|e| LoadError::io(path.display(), e))
}

/// The temporary dir collector — the walk yields its pkgs and its
/// root name ([`Loaded`]): offer them to a run with
/// `RutRun::new().pkg(..)`. The mount order is the law:
/// 1. the `[deps]` walk — unchanged;
/// 2. the dev pass — the ROOT's `[dev-deps]` mount exactly like `[deps]`
///    (a dep's dev table is never walked, so a consumer's world never
///    contains it);
/// 3. the peer gate — ONE post-closure pass (a peer may mount after its
///    declarer alphabetically, so it cannot run during the walk);
/// 4. compile — unchanged; the graph sees ordinary sources.
pub fn load_dir(dir: &Path, src: &dyn Source) -> Result<Loaded, RunError> {
    // the back-compat lane: no fetcher, empty map — a url dep here is
    // the loud no-fetcher error, never a network call
    walk_dir_fetched(dir, src, &BTreeMap::new()).map(|w| w.into_loaded()).map_err(RunError::from)
}

/// A walk's internal yield: the mounted table, its root spec, and
/// the archives the url deps opened (slot-ordered) — pass 3's archive
/// group reads and the packer's rode-along copies index into the list.
/// The collectors drain the pkgs out ([`Loaded`]); the packer keeps
/// the session form.
pub(crate) struct WalkOutput {
    pub session: Session,
    pub root: String,
    pub archives: Vec<Archive>,
}

impl WalkOutput {
    /// The collector's shape: the pkgs travel, the table dies.
    pub(crate) fn into_loaded(self) -> Loaded {
        let mut session = self.session;
        Loaded { pkgs: session.drain_pkgs(), root: self.root }
    }
}

/// [`load_dir`] over PRE-FETCHED url bytes — the pub sync core
/// the `*_with` lanes call after `prefetch_urls`, and the fixture lane
/// tests use directly. Every url row's `sha256` pin is verified HERE,
/// at the mount door, on every load.
pub fn load_dir_fetched(
    dir: &Path,
    src: &dyn Source,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<Loaded, RunError> {
    walk_dir_fetched(dir, src, map).map(|w| w.into_loaded()).map_err(RunError::from)
}

/// The internal sync core — [`load_dir_fetched`] keeping the session
/// form (the packer compiles over it).
pub(crate) fn walk_dir_fetched(
    dir: &Path,
    src: &dyn Source,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<WalkOutput, LoadError> {
    let manifest = read_manifest(dir, src)?;
    let mut session = Session::new();

    let root = manifest
        .name
        .clone()
        .ok_or_else(|| LoadError::law(format!("{} has no `name`", dir.join("rut.jsonc").display())))?;
    let root_module = fold_peers(load_entry_module(dir, &manifest, src)?, &manifest);
    session.register_module(&root, root_module)?;
    // the root's dir rides its pkg too, so a later gate pass over this
    // table can answer for the root's own peer declarations (the gate
    // below already ran with the complete map; this keeps the pkg
    // self-consistent)
    session.resolve_mut(&root).unwrap().dir = Some(dir.to_path_buf());
    let mut visiting = vec![dir.to_path_buf()];
    let mut archives: Vec<Archive> = Vec::new();
    {
        let mut fetched = Fetched {
            map,
            archives: &mut archives,
            visiting_urls: Default::default(),
        };
        resolve_table(&mut session, dir, src, &manifest.deps, &mut visiting, &mut fetched)?;
        resolve_table(&mut session, dir, src, &manifest.dev_deps, &mut visiting, &mut fetched)?;
    }
    run_peer_gate(&mut session, &root, &archives)?;
    stamp(&mut session, next_walk_id());
    Ok(WalkOutput { session, root, archives })
}

/// Namespace one archive's scope ledger into `session`'s numbering and
/// answer the map that rebases the archive's programs with it — the
/// CDN law: per-package bundles are packed INDEPENDENTLY, so their
/// pack-time numberings argue; each archive's rows shift above
/// everything already recorded (boot passes through) and its binaries
/// rebase BEFORE mounting, so every id resolves through its OWN
/// archive's rows. Fails only when the numbering space itself is
/// exhausted (4096 scopes; boot owns 0).
fn namescope_ledger(
    session: &Session,
    scopes: &[(rut_core::id::ScopeId, String)],
) -> Result<impl Fn(rut_core::id::ScopeId) -> rut_core::id::ScopeId, LoadError> {
    let base = session.bundle_scope_next_base().ok_or_else(|| {
        LoadError::law("the bundle scope numbering space is exhausted".to_string())
    })?;
    for &(s, _) in scopes {
        if s != rut_core::id::BOOT_SCOPE && base as u32 + s as u32 > rut_core::id::MAX_SCOPE {
            return Err(LoadError::law(format!(
                "the bundle's scope ledger reaches scope {s}, which does not fit above this program's {base} — the numbering space is exhausted"
            )));
        }
    }
    Ok(move |s: rut_core::id::ScopeId| {
        if s == rut_core::id::BOOT_SCOPE {
            s
        } else {
            base + s
        }
    })
}

/// Build a package's entry [`Pkg`] from bundle entries under
/// `prefix` (empty for the root, `<pkg>/` for a dep group) — the
/// in-archive counterpart of `load_entry_module`: the declared kind
/// dispatches (a `type = "host"` pkg is a decl surface; a `type =
/// "lib"` pkg with a surface and no body is the surface-only dev
/// state).
fn bundle_entry_module(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Pkg, LoadError> {
    let read = |rel: &str| -> Result<String, LoadError> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}")).map_err(LoadError::law)?;
        read_entry(entries, &key).map_err(LoadError::law)
    };
    match manifest.pkg_type {
        PkgType::Host => {
            let rel = manifest.entry.type_path.as_ref().ok_or_else(|| {
                LoadError::law(format!(
                    "module at bundle prefix `{prefix}` is a host pkg with no `entry.type`"
                ))
            })?;
            let src = read(rel)?;
            let mut m = crate::decl::lower_decl_module(&src, &format!("{prefix}{rel}"))
                .map_err(LoadError::law)?;
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
                    return Ok(Pkg {
                        body: PkgBody::Source { text: src, is_decl: true },
                        entry: manifest.entry.clone(),
                        ..Default::default()
                    });
                }
            }
        }
    }
    let rel = entry_rel(manifest)
        .ok_or_else(|| LoadError::law(format!("module at bundle prefix `{prefix}` has no entry")))?;
    let mut src = read(rel)?;
    // the multi-lib splice, the archive-side twin of
    // `load_entry_module`'s: base first, then `libs` in manifest
    // order, '\n'-joined — ONE source string
    for lib in &manifest.entry.libs {
        src.push('\n');
        src.push_str(&read(lib)?);
    }
    Ok(Pkg {
        body: PkgBody::Source { text: src, is_decl: false },
        entry: manifest.entry.clone(),
        ..Default::default()
    })
}

/// The generic-bearing source a compiled unit rides, read from the
/// archive under `prefix` (empty for the root, `<pkg>/` for a group):
/// the entry lib + `entry.libs` spliced, plus the `[peer-deps]` group
/// files keyed by peer. The entry lib's PRESENCE is the dispatch
/// marker the packer laid down — its absence is a legacy bundle (or a
/// non-generic pkg), which rides nothing and refuses consumer-spelled
/// shapes at link. Once the marker answers, every other riding file
/// must be there (refuse, never guess — a corrupt archive is a load
/// error).
fn riding_gen_source(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Option<crate::session::GenSource>, LoadError> {
    let read = |rel: &str| -> Result<String, LoadError> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}")).map_err(LoadError::law)?;
        read_entry(entries, &key).map_err(LoadError::law)
    };
    let Some(base) = &manifest.entry.lib else {
        return Ok(None); // no body — nothing could ride
    };
    let mut text = match read(base) {
        Ok(t) => t,
        Err(_) => return Ok(None), // the marker's absence: a legacy bundle
    };
    for lib in &manifest.entry.libs {
        text.push('\n');
        text.push_str(&read(lib)?);
    }
    let mut peers = Vec::new();
    for (peer, desc) in &manifest.peer_deps {
        if let Some(lib) = desc.get("lib") {
            peers.push((peer.clone(), read(lib)?));
        }
    }
    Ok(Some(crate::session::GenSource { text, peers }))
}

/// Load a packed `.rutbundle` from disk — a **v9 compiled** bundle (the
/// root's `.rutc` binary, its pack-time scope ledger, and each dep group
/// as a compiled `.rutc` or a source file set) or a **v10 decl** bundle
/// (a host root: the declaration surface mounts as the pkg's host rows —
/// the same lane a host group rides). The gate order is the law:
/// container CRC ([`Bundle::parse`]), manifest + exact
/// `format_version` (the reader's pairing), then each kind's payload
/// checks (compiled: per-group decode and verification; decl: the
/// single-package law) — then, and only then, the mount. Older layouts
/// are refused with the one-line version error: refuse, never guess.
/// Source sharing stays what it always was outside bundles: a directory.
pub fn load_bundle_session(path: &Path) -> Result<Loaded, RunError> {
    let bytes = std::fs::read(path).map_err(|e| LoadError::io(path.display(), e))?;
    bundle_walk_bytes(&bytes).map(crate::loader::WalkOutput::into_loaded).map_err(RunError::from)
}

/// The bundle walk's sync core — the pure container parse
/// ([`Pkg::from_bundle`] is its public face): gates, groups, ledger,
/// the ONE peer gate over the archive's own pkgs, then the yield.
pub(crate) fn bundle_walk_bytes(bytes: &[u8]) -> Result<WalkOutput, LoadError> {
    let origin = "bundle";
    // gate 1: the container — every entry's CRC-32 verified
    let bundle = match Bundle::parse(bytes) {
        Ok(b) => b,
        Err(e) => return Err(LoadError::Bundle { origin: origin.to_string(), message: e.to_string() }),
    };
    // gate 2 + 3: the manifest/version, then every group's decode +
    // verification — the layout parse refuses anything it cannot
    // decode, so a bad binary never reaches the table
    let layout = match Layout::parse(&bundle) {
        Ok(l) => l,
        Err(e) => return Err(LoadError::Bundle { origin: origin.to_string(), message: e }),
    };
    let (manifest, root, scopes, groups) = match layout {
        Layout::Compiled { manifest, root, scopes, groups } => (manifest, root, scopes, groups),
        Layout::Decl { manifest, .. } => {
            // the decl root: a host pkg — the surface mounts as the
            // pkg's host rows through `bundle_entry_module` (the lane a
            // host group rides inside a v7 bundle). Single-package: no
            // groups, no ledger, no closure — a LEAF, done.
            let entries = bundle.entries();
            let root_spec = manifest.name.clone().ok_or_else(|| {
                LoadError::law(format!("{origin}: rut.jsonc has no `name`"))
            })?;
            let mut session = Session::new();
            let pkg = fold_peers(
                bundle_entry_module(entries, "", &manifest)
                    .map_err(|e| LoadError::law(format!("{origin}: {e}")))?,
                &manifest,
            );
            session.register_module(&root_spec, pkg)?;
            return Ok(WalkOutput { session, root: root_spec, archives: Vec::new() });
        }
    };
    let entries = bundle.entries();
    let root_spec = manifest
        .name
        .clone()
        .ok_or_else(|| LoadError::law(format!("{origin}: rut.jsonc has no `name`")))?;
    let mut session = Session::new();
    // the scope ledger first, namespaced: the rows shift into a fresh
    // range of this session and the root's program rebases with the
    // same map BEFORE mounting — the graph later resolves every
    // decoded foreign id through these rows
    let remap = namescope_ledger(&session, &scopes)?;
    let scopes: Vec<(rut_core::id::ScopeId, String)> =
        scopes.into_iter().map(|(s, spec)| (remap(s), spec)).collect();
    for (scope, spec) in &scopes {
        session.record_bundle_scope(*scope, spec);
    }
    let root = rut_core::link::rebase(root, &remap);
    // the root: a compiled module (the packer refuses any other root).
    // A generic-owning root rides its source beside the binary — the
    // on-demand recompile's input (generic-source riding).
    let gen_source = riding_gen_source(entries, "", &manifest)
        .map_err(|e| LoadError::law(format!("{origin}: {e}")))?;
    session.register_module(
        &root_spec,
        Pkg {
            body: PkgBody::Compiled(root),
            entry: manifest.entry.clone(),
            gen_source,
            bundle_scopes: scopes.clone(),
            peers: manifest_peers(&manifest),
            ..Default::default()
        },
    )?;
    // the archive's group prefixes (root "" first) — the peer gate's
    // group reads key on these
    let mut prefixes: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    prefixes.insert(root_spec.clone(), String::new());
    for (prefix, kind) in &groups {
        let dep_toml = read_entry(entries, &format!("{prefix}/rut.jsonc"))
            .map_err(|e| LoadError::Bundle { origin: origin.to_string(), message: e })?;
        let dm = parse_manifest(&dep_toml).map_err(|e| {
            LoadError::law(format!("{origin}: {prefix}/rut.jsonc: {e}"))
        })?;
        let name = dm.name.clone().ok_or_else(|| {
            LoadError::law(format!("{origin}: {prefix}/rut.jsonc has no `name`"))
        })?;
        if session.resolve(&name).is_ok() {
            continue; // first mount wins (the root, an earlier group)
        }
        let pkg = match kind {
            GroupKind::Compiled(program) => {
                // rebase first, THEN check own-vs-ledger — both sides
                // of the comparison shift by the same map, so the law
                // (the row must be the scope the binary itself
                // carries) is unchanged, and the stored binary is the
                // namespaced one
                let program = rut_core::link::rebase(program.clone(), &remap);
                // the ledger must name the group, and the row must be
                // the scope the binary itself carries (refuse, never
                // guess — a mismatch is a corrupt or doctored bundle)
                let Some(own) = rut_core::link::own_scope(&program) else {
                    return Err(LoadError::law(format!(
                        "{origin}: {prefix}/{}: the program carries no scope blocks",
                        name
                    )));
                };
                match scopes.iter().find(|(_, s)| s == &name) {
                    Some(&(row, _)) if row == own => {}
                    Some(&(row, _)) => {
                        return Err(LoadError::law(format!(
                            "{origin}: `{name}`'s ledger row says scope {row}, but its binary carries {own}"
                        )));
                    }
                    None => {
                        return Err(LoadError::law(format!(
                            "{origin}: the scope ledger does not name `{name}` — the bundle is incomplete"
                        )));
                    }
                }
                let gen_source = riding_gen_source(entries, &format!("{prefix}/"), &dm)
                    .map_err(|e| LoadError::law(format!("{origin}: {e}")))?;
                Pkg {
                    body: PkgBody::Compiled(program),
                    entry: dm.entry.clone(),
                    gen_source,
                    peers: manifest_peers(&dm),
                    ..Default::default()
                }
            }
            GroupKind::Source => fold_peers(
                bundle_entry_module(entries, &format!("{prefix}/"), &dm)
                    .map_err(|e| LoadError::law(format!("{origin}: {e}")))?,
                &dm,
            ),
        };
        session.register_module(&name, pkg)?;
        prefixes.insert(name.clone(), format!("{prefix}/"));
    }
    // every declared dep must be satisfied by a group
    for spec in manifest.deps.keys() {
        if session.resolve(spec).is_err() {
            return Err(LoadError::law(format!(
                "{origin}: the bundle is missing its `{spec}` dependency group"
            )));
        }
    }
    // pass 3 — the ONE peer gate: the archive's own pkgs are its
    // sources (slot 0), so a source declarer's group file reads from
    // the archive exactly as a dir declarer's reads from disk
    for (spec, prefix) in &prefixes {
        if let Ok(m) = session.resolve_mut(spec) {
            m.archive = Some((0, prefix.clone()));
        }
    }
    let archives = [Archive {
        origin: origin.to_string(),
        entries: entries.to_vec(),
    }];
    run_peer_gate(&mut session, &root_spec, &archives)?;
    let mut out = WalkOutput { session, root: root_spec, archives: archives.to_vec() };
    stamp(&mut out.session, next_walk_id());
    Ok(out)
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
    archives: &[Archive],
) -> Result<(), LoadError> {
    // collected first, applied after — the table borrows the session
    let mut appends: Vec<(String, String)> = Vec::new();
    let mut pre_compiled: Vec<String> = Vec::new();
    // the pass reads the mounted pkgs' own declarations and locations —
    // the walk folded them into the pkgs at mount
    let declared: Vec<(String, BTreeMap<String, PeerDecl>)> = session
        .modules()
        .filter(|(_, m)| !m.peers.is_empty() && !m.groups_mounted)
        .map(|(s, m)| (s.clone(), m.peers.clone()))
        .collect();
    let all_sources = sources_of(session);
    for (pkg, peers) in declared {
        // a compiled declarer's rows ride its binary — the presence law
        // still runs below (D1), but there is nothing to read, and the
        // declarer is done after this pass
        let compiled = matches!(session.resolve(&pkg).map(|m| &m.body), Ok(PkgBody::Compiled(_)));
        let source = all_sources.get(&pkg).cloned();
        for (peer, decl) in peers {
            if pkg.as_str() == root {
                if let Some(PkgSource::Dir(pkg_dir)) = &source {
                    // self-build: the path must resolve and name the
                    // peer — even when dev-deps already supplied
                    // presence (D3 is directory-time law)
                    let peer_dir = pkg_dir.join(&decl.path);
                    match read_manifest(&peer_dir, &FsSource) {
                        Ok(dm) if dm.name.as_deref() == Some(peer.as_str()) => {}
                        Ok(dm) => {
                            return Err(LoadError::law(format!(
                                "pkg `{pkg}`'s [peer-deps] entry `{peer}` points at `{path}` — the manifest there names it `{actual}` (a packaging bug in {pkg})",
                                path = decl.path,
                                actual = dm.name.as_deref().unwrap_or("<unnamed>"),
                            )));
                        }
                        Err(_) => {
                            return Err(LoadError::law(format!(
                                "pkg `{pkg}`'s [peer-deps] entry `{peer}` points at `{path}` — cannot read a manifest there (a packaging bug in {pkg})",
                                path = decl.path,
                            )));
                        }
                    }
                }
            }
            if session.resolve(&peer).is_err() {
                if decl.optional {
                    continue; // inert — the group simply never mounts
                }
                // D1: loud at mount, naming pkg + peer + fix
                return Err(LoadError::law(format!(
                    "pkg `{pkg}` requires the peer `{peer}`, and `{peer}` is not in this program's closure — peers are not pulled transitively: add `\"{peer}\": {{ \"path\": \"..\" }}` to your `rut.jsonc` `deps`"
                )));
            }
            if compiled {
                continue; // rows ride the binary — nothing to read
            }
            let Some(lib) = &decl.lib else {
                continue; // presence declared, no integration file to mount
            };
            let Some(source) = &source else {
                return Err(LoadError::law(format!(
                    "pkg `{pkg}` declares `[peer-deps]` but is not mounted"
                )));
            };
            let text = match source {
                PkgSource::Dir(pkg_dir) => {
                    let group_path = pkg_dir.join(lib);
                    std::fs::read_to_string(&group_path).map_err(|_| {
                        LoadError::law(format!(
                            "pkg `{pkg}`'s [peer-deps] entry `{peer}` names the group `{lib}` — cannot read it (a packaging bug in {pkg})"
                        ))
                    })?
                }
                PkgSource::Archive { slot, prefix } => {
                    let archive = &archives[*slot];
                    let rel = lib.strip_prefix("./").unwrap_or(lib);
                    let key = bundle_key(&format!("{prefix}{rel}"))
                        .map_err(|e| LoadError::law(format!("{}: {e}", archive.origin)))?;
                    read_entry(&archive.entries, &key).map_err(|_| {
                        LoadError::law(format!(
                            "pkg `{pkg}`'s [peer-deps] entry `{peer}` names the group `{lib}` — cannot read it (a packaging bug in {pkg})"
                        ))
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
        let m = session.resolve_mut(&pkg).unwrap();
        m.peer_groups.push(text);
        m.groups_mounted = true;
    }
    for pkg in pre_compiled {
        session.resolve_mut(&pkg).unwrap().groups_mounted = true;
    }
    Ok(())
}

/// Load a module directory (`rut.jsonc`) or a `.rutbundle` file — the two
/// packed forms of the same contract. A loose `.rut` file is NOT this: it
/// is a single-file module with no manifest.
pub fn load_path_session(path: &Path) -> Result<Loaded, RunError> {
    if path.is_dir() {
        return load_dir(path, &FsSource);
    }
    if path.extension().map_or(false, |e| e == "rutbundle") {
        return load_bundle_session(path);
    }
    Err(LoadError::Shape {
        path: path.display().to_string(),
    }.into())
}

/// Build a directory's entry [`Pkg`] from its manifest:
///
/// - a `type = "host"` pkg — a pure declaration surface. The `.d.rut`
///   parses in declaration mode and lowers into the module's host fns.
///   No body exists — the embedding Rust binds it at run time.
/// - a `type = "lib"` pkg (the default) — a source module: the body
///   compiles; the surface derives from its exports. A declared
///   surface with no body is the surface-only dev state (a decl unit);
///   `host fn` text is refused in either file — the lib-surface law.
fn load_entry_module(dir: &Path, manifest: &Manifest, src: &dyn Source) -> Result<Pkg, LoadError> {
    match manifest.pkg_type {
        PkgType::Host => {
            let rel = manifest.entry.type_path.as_ref().ok_or_else(|| {
                LoadError::law(format!(
                    "module in {} is a host pkg with no `entry.type`",
                    dir.display()
                ))
            })?;
            let origin = format!("{}/{}", dir.display(), rel);
            let src_text = read_source_text(src, &dir.join(rel))?;
            let mut m = crate::decl::lower_decl_module(&src_text, &origin).map_err(LoadError::law)?;
            m.entry = manifest.entry.clone();
            return Ok(m);
        }
        PkgType::Lib => {
            if let Some(rel) = &manifest.entry.type_path {
                let origin = format!("{}/{}", dir.display(), rel);
                let src_text = read_source_text(src, &dir.join(rel))?;
                refuse_host_rows(&src_text, &origin)?;
                if manifest.entry.lib.is_none() {
                    // the surface-only dev state: a decl unit — no host
                    // rows, nothing exported (use sites resolve-miss,
                    // correctly)
                    return Ok(Pkg {
                        body: PkgBody::Source { text: src_text, is_decl: true },
                        entry: manifest.entry.clone(),
                        ..Default::default()
                    });
                }
            }
        }
    }
    let mut src_text = load_entry(dir, src, &manifest.entry)?;
    // The multi-lib splice: the base `lib` first, then
    // `libs` in manifest order, '\n'-joined exactly like the
    // peer-group append — the combined text stays ONE source string,
    // so every downstream consumer of the source body (the graph
    // splice, the wasm mounts) is untouched. The manifest's array
    // order is the canonical order: the splice never reads a directory
    // listing, so same manifest ⇒ same module (the determinism law).
    for rel in &manifest.entry.libs {
        let text = read_source_text(src, &dir.join(rel))?;
        src_text.push('\n');
        src_text.push_str(&text);
    }
    Ok(Pkg {
        body: PkgBody::Source { text: src_text, is_decl: false },
        entry: manifest.entry.clone(),
        ..Default::default()
    })
}

/// The lib-surface law: `host fn` text lives only in `type = "host"`
/// pkgs. Non-parsing surfaces stay inert (doc-only, as today); a
/// surface that PARSES and declares host fns is the loud error.
fn refuse_host_rows(src: &str, origin: &str) -> Result<(), LoadError> {
    if let Ok(m) = crate::decl::lower_decl_module(src, origin) {
        if let PkgBody::Host { host_funcs, .. } = m.body {
            if let Some((name, ..)) = host_funcs.first() {
                return Err(LoadError::law(format!(
                    "{origin}: `host fn {name}` — a lib pkg cannot declare \
                     host fns; split the rows into a `type = \"host\"` pkg \
                     and depend on it"
                )));
            }
        }
    }
    Ok(())
}

fn load_entry(dir: &Path, src: &dyn Source, entry: &Entry) -> Result<String, LoadError> {
    let rel = entry
        .lib
        .as_ref()
        .or(entry.type_path.as_ref())
        .ok_or_else(|| {
            LoadError::law(format!("module in {} has no entry", dir.display()))
        })?;
    read_source_text(src, &dir.join(rel))
}

/// A manifest's `[peer-deps]` as the pkg's own declaration map — the
/// loader reads every mounted pkg's manifest anyway, so folding the
/// table into the pkg costs no extra I/O. The peer gate reads it
/// post-closure; the reference-site D2 diagnostic resolves against it.
pub(crate) fn manifest_peers(manifest: &Manifest) -> BTreeMap<String, PeerDecl> {
    manifest
        .peer_deps
        .iter()
        .map(|(peer, desc)| (peer.clone(), PeerDecl::of(desc)))
        .collect()
}

/// [`manifest_peers`] folded into a walked pkg — the constructor + fold
/// pair the entry lanes share.
pub(crate) fn fold_peers(mut pkg: Pkg, manifest: &Manifest) -> Pkg {
    pkg.peers.extend(manifest_peers(manifest));
    pkg
}


/// [`mount_dev_table`] over pre-fetched url bytes — the pack lane's
/// flavor (a dir group's dev table may pin url deps of its own).
pub(crate) fn mount_dev_table_fetched(
    session: &mut Session,
    dir: &Path,
    manifest: &Manifest,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<(), LoadError> {
    let mut visiting = Vec::new();
    let mut archives: Vec<Archive> = Vec::new();
    let mut fetched = Fetched {
        map,
        archives: &mut archives,
        visiting_urls: Default::default(),
    };
    resolve_table(session, dir, &FsSource, &manifest.dev_deps, &mut visiting, &mut fetched)
}

/// Resolve a manifest's `[deps]` recursively — pass 1 of
/// the mount order.
fn resolve_deps(
    session: &mut Session,
    dir: &Path,
    src: &dyn Source,
    manifest: &Manifest,
    visiting: &mut Vec<std::path::PathBuf>,
    fetched: &mut Fetched<'_>,
) -> Result<(), LoadError> {
    resolve_table(session, dir, src, &manifest.deps, visiting, fetched)
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
    src: &dyn Source,
    table: &BTreeMap<String, BTreeMap<String, String>>,
    visiting: &mut Vec<std::path::PathBuf>,
    fetched: &mut Fetched<'_>,
) -> Result<(), LoadError> {
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
                return Err(LoadError::law(format!(
                    "cyclic use: `{spec}` ({url}) is already being loaded"
                )));
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
            .ok_or_else(|| LoadError::law(format!("dep `{spec}` has no `path`")))?;
        let dep_dir = dir.join(rel);
        if visiting.contains(&dep_dir) {
            return Err(LoadError::law(format!(
                "cyclic use: `{spec}` ({}) is already being loaded",
                dep_dir.display()
            )));
        }
        let dm = read_manifest(&dep_dir, src)?;
        if dm.name.as_deref() != Some(spec.as_str()) {
            return Err(LoadError::DepName {
                spec: spec.clone(),
                location: dep_dir.display().to_string(),
                actual: dm.name.as_deref().unwrap_or("<unnamed>").to_string(),
            });
        }
        let dep_pkg = fold_peers(load_entry_module(&dep_dir, &dm, src)?, &dm);
        let dep_dir2 = dep_dir.clone();
        session.register_module(spec, dep_pkg)?;
        session.resolve_mut(spec).unwrap().dir = Some(dep_dir2);
        visiting.push(dep_dir.clone());
        // a dep's dev-deps are NEVER walked — pass 2 is root-only, so a
        // consumer's world never contains another pkg's dev table
        resolve_deps(session, &dep_dir, src, &dm, visiting, fetched)?;
        visiting.pop();
    }
    Ok(())
}

/// Mount a url dep's fetched bytes — the mount door where the pin is
/// LAW. The fetcher saw only a url; the manifest row's `sha256` and
/// the bytes meet HERE, on EVERY load (fresh fetch, cache hit,
/// vendored map, test fixture) — a check the call site performed would
/// be a check the call site could skip. Gate order mirrors
/// `bundle_walk_bytes`:
///
/// 0. the pin: `sha256_hex(bytes)` must equal the row's pin (lowercase
///    by grammar law) — a mismatch names the dep, the url, and BOTH
///    hashes;
/// 1. the container (every entry CRC-verified) — origin is the url;
/// 2. the layout: manifest + exact `format_version` 7|8 (compiled or
///    decl root), every group decode + verified;
/// 3. name-vs-key: the bundle's `name` must equal the `[deps]` key
///    (the path flavor's twin);
/// 4. the scope-ledger NAMESPACING: this archive's rows shift into a
///    fresh range of the combined session (boot passes through) and
///    its compiled programs rebase with the same map BEFORE mounting —
///    two independently packed bundles never argue about a number
///    (each binary's ids resolve through its own archive's rows);
/// 5. the root mounts (first-mount-wins) — compiled for a v7 bundle,
///    the decl-surface host rows for a v8 bundle
///    (`bundle_entry_module`, the group lane); the manifest's peers fold in;
/// 6. the group loop = `bundle_walk_bytes`'s (first-mount-wins,
///    own-scope vs ledger consistency for compiled groups,
///    `bundle_entry_module` for source groups) — no peer gate inline:
///    archives record for the ONE pass-3 gate;
/// 7. the closure check against the COMBINED session — bundles are
///    closed: every declared dep rides in-archive (or mounted earlier).
///    No fetching at bundle load, ever. A v8 host bundle is a LEAF:
///    single-package, no groups, the closure check trivially true.
fn mount_url_dep(
    session: &mut Session,
    spec: &str,
    url: &str,
    pin: Option<&str>,
    bytes: &[u8],
    archives: &mut Vec<Archive>,
) -> Result<(), LoadError> {
    if session.resolve(spec).is_ok() {
        return Ok(()); // first mount wins — the embedder's (or an earlier) mount
    }
    // gate 0 — the pin law
    if let Some(pin) = pin {
        let got = sha256_hex(bytes);
        if got != pin {
            return Err(LoadError::Pin {
                spec: spec.to_string(),
                url: url.to_string(),
                pin: pin.to_string(),
                got,
            });
        }
    }
    // gate 1 — the container
    let bundle = match Bundle::parse(bytes) {
        Ok(b) => b,
        Err(e) => return Err(LoadError::Bundle { origin: url.to_string(), message: e.to_string() }),
    };
    // gate 2 — the layout: manifest, exact version, every group decoded
    let layout = match Layout::parse(&bundle) {
        Ok(l) => l,
        Err(e) => return Err(LoadError::Bundle { origin: url.to_string(), message: e }),
    };
    let (manifest, root, scopes, groups) = match layout {
        Layout::Compiled { manifest, root, scopes, groups } => (manifest, root, scopes, groups),
        Layout::Decl { manifest, .. } => {
            // the decl root: a host pkg — the surface mounts as the
            // pkg's host rows (`bundle_entry_module`, the group lane);
            // single-package, no ledger, no groups. The LEAF law: no
            // fetch walk, the closure check below is trivially true.
            let entries = bundle.entries().to_vec();
            let root_spec = manifest
                .name
                .clone()
                .ok_or_else(|| LoadError::law(format!("{url}: rut.jsonc has no `name`")))?;
            if root_spec != spec {
                return Err(LoadError::law(format!(
                    "dep `{spec}` points at {url} — the bundle names itself `{root_spec}`"
                )));
            }
            let pkg = fold_peers(
                bundle_entry_module(&entries, "", &manifest)
                    .map_err(|e| LoadError::law(format!("{url}: {e}")))?,
                &manifest,
            );
            session.register_module(&root_spec, pkg)?;
            session.resolve_mut(&root_spec).unwrap().archive = Some((archives.len(), String::new()));
            archives.push(Archive { origin: url.to_string(), entries });
            return Ok(());
        }
    };
    // gate 3 — name-vs-key
    let root_spec = manifest
        .name
        .clone()
        .ok_or_else(|| LoadError::law(format!("{url}: rut.jsonc has no `name`")))?;
    if root_spec != spec {
        return Err(LoadError::law(format!(
            "dep `{spec}` points at {url} — the bundle names itself `{root_spec}`"
        )));
    }
    // gate 4 — the ledger namespacing: this archive's rows shift into a
    // fresh range of the COMBINED session (boot passes through) and its
    // compiled programs rebase with the same map before mounting — two
    // archives never argue about a number, each binary's ids resolve
    // through its OWN archive's rows
    let remap = namescope_ledger(session, &scopes)?;
    let scopes: Vec<(rut_core::id::ScopeId, String)> =
        scopes.into_iter().map(|(s, spec)| (remap(s), spec)).collect();
    for (scope, row_spec) in &scopes {
        session.record_bundle_scope(*scope, row_spec);
    }
    let root = rut_core::link::rebase(root, &remap);
    // gate 6's group reads + the generic-source marker share one entry
    // list — cloned before the root mounts
    let entries = bundle.entries().to_vec();
    // gate 5 — the root: a compiled module (the packer refuses any
    // other); a generic-owning root rides its source beside the binary
    let gen_source = riding_gen_source(&entries, "", &manifest)
        .map_err(|e| LoadError::law(format!("{url}: {e}")))?;
    session.register_module(
        &root_spec,
        Pkg {
            body: PkgBody::Compiled(root),
            entry: manifest.entry.clone(),
            gen_source,
            bundle_scopes: scopes.clone(),
            peers: manifest_peers(&manifest),
            ..Default::default()
        },
    )?;
    // gate 6 — the group loop, no peer gate inline
    let slot = archives.len();
    let mut prefixes: BTreeMap<String, String> = BTreeMap::new();
    prefixes.insert(root_spec.clone(), String::new());
    for (prefix, kind) in &groups {
        let dep_toml = read_entry(&entries, &format!("{prefix}/rut.jsonc"))
            .map_err(|e| LoadError::Bundle { origin: url.to_string(), message: e })?;
        let dm = parse_manifest(&dep_toml)
            .map_err(|e| LoadError::law(format!("{url}: {prefix}/rut.jsonc: {e}")))?;
        let name = dm
            .name
            .clone()
            .ok_or_else(|| LoadError::law(format!("{url}: {prefix}/rut.jsonc has no `name`")))?;
        if session.resolve(&name).is_ok() {
            continue; // first mount wins (an earlier archive, or a dir dep)
        }
        let module = match kind {
            GroupKind::Compiled(program) => {
                // rebase first, THEN check own-vs-ledger — both sides
                // shift by the same map, so the law (the row must be
                // the scope the binary itself carries) is unchanged,
                // and the stored binary is the namespaced one
                let program = rut_core::link::rebase(program.clone(), &remap);
                // the ledger must name the group, and the row must be
                // the scope the binary itself carries (refuse, never
                // guess — a mismatch is a corrupt or doctored bundle)
                let Some(own) = rut_core::link::own_scope(&program) else {
                    return Err(LoadError::law(format!(
                        "{url}: {prefix}/{}: the program carries no scope blocks",
                        name
                    )));
                };
                match scopes.iter().find(|(_, s)| s == &name) {
                    Some(&(row, _)) if row == own => {}
                    Some(&(row, _)) => {
                        return Err(LoadError::law(format!(
                            "{url}: `{name}`'s ledger row says scope {row}, but its binary carries {own}"
                        )));
                    }
                    None => {
                        return Err(LoadError::law(format!(
                            "{url}: the scope ledger does not name `{name}` — the bundle is incomplete"
                        )));
                    }
                }
                let gen_source = riding_gen_source(&entries, &format!("{prefix}/"), &dm)
                    .map_err(|e| LoadError::law(format!("{url}: {e}")))?;
                Pkg {
                    body: PkgBody::Compiled(program),
                    entry: dm.entry.clone(),
                    gen_source,
                    peers: manifest_peers(&dm),
                    ..Default::default()
                }
            }
            GroupKind::Source => fold_peers(
                bundle_entry_module(&entries, &format!("{prefix}/"), &dm)
                    .map_err(|e| LoadError::law(format!("{url}: {e}")))?,
                &dm,
            ),
        };
        session.register_module(&name, module)?;
        prefixes.insert(name.clone(), format!("{prefix}/"));
    }
    // where this archive's mounted pkgs' files live (pass 3's reads,
    // the packer's rode-along copies — the pkg holds its location)
    for (spec, prefix) in &prefixes {
        if let Ok(m) = session.resolve_mut(spec) {
            m.archive.get_or_insert((slot, prefix.clone()));
        }
    }
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
            return Err(LoadError::law(format!(
                "{url}: the bundle is missing its `{dep}` dependency group"
            )));
        }
    }
    Ok(())
}

/// Offer one package directory — and, recursively, its `[deps]` — as
/// walked pkgs: the programmatic counterpart of a manifest's dep walk
/// (native hosts, tests, plugin loaders). The root is
/// [`Loaded::root`]; offer every pkg to a run with `RutRun::new().pkg(..)`.
///
/// This is an OFFER to someone else's program, not "building the pkg
/// itself": no dev-deps are mounted (pass 2 is root-only) and the peer
/// gate does not run here — the embedder's world grows incrementally,
/// so presence is a program-closure property the run's own
/// `.compile()` owns (it runs the ONE append pass at close). The
/// pkgs' peer declarations ride along.
pub fn dir_pkgs(dir: &Path) -> Result<Loaded, RunError> {
    dir_pkgs_fetched(dir, &BTreeMap::new())
}

/// [`dir_pkgs`] over pre-fetched url bytes — the `dir_pkgs_with`
/// lane's sync core. The gate does not run here (the offer law);
/// archive locations ride the pkgs, so a later gate pass sees them —
/// note it reads group files from the archives the CALLER kept (an
/// embedder that mounts url deps offer-by-offer owns its archive
/// list; the load/pack lanes run the gate themselves).
pub(crate) fn dir_pkgs_fetched(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<Loaded, RunError> {
    let mut session = Session::new();
    let name = offer_dir(&mut session, dir, map)?;
    Ok(WalkOutput { session, root: name, archives: Vec::new() }.into_loaded())
}

/// The offer lane's core (crate-internal): walk `dir` + deps into an
/// EXISTING table, first-mount-wins, and answer the root's name.
pub(crate) fn offer_dir(
    session: &mut Session,
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<String, LoadError> {
    let manifest = read_manifest(dir, &FsSource)?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| LoadError::law(format!("{} has no `name`", dir.join("rut.jsonc").display())))?;
    if session.resolve(&name).is_ok() {
        return Ok(name); // the embedder's mount outranks the directory
    }
    let pkg = fold_peers(load_entry_module(dir, &manifest, &FsSource)?, &manifest);
    session.register_module(&name, pkg)?;
    session.resolve_mut(&name).unwrap().dir = Some(dir.to_path_buf());
    let mut visiting = vec![dir.to_path_buf()];
    let mut archives: Vec<Archive> = Vec::new();
    let mut fetched = Fetched {
        map,
        archives: &mut archives,
        visiting_urls: Default::default(),
    };
    resolve_deps(session, dir, &FsSource, &manifest, &mut visiting, &mut fetched)?;
    stamp(session, next_walk_id());
    Ok(name)
}

/// The peer gate for tables built offer-by-offer: runs the gate's
/// append pass over the peer declarations the offered pkgs carry,
/// using the locations those offers left on them (dirs and archive
/// locations). The run chain calls this at `.compile()` — its "close
/// the world" step — so a loose source that `use json::` gets the
/// peer-gated container groups exactly like a module-dir program
/// does. Presence-based as ever: an optional peer absent is inert; a
/// required peer absent is the loud D1 error. Groups already mounted
/// by an earlier gate pass are skipped — never double-appended. NOTE
/// the archive half: this lane owns no archive bytes, so an
/// archive-mounted declarer's group file read fails loudly — the
/// load/pack lanes run the gate themselves over their own archive
/// list (crate-internal).
pub(crate) fn assemble_peers(session: &mut Session) -> Result<(), LoadError> {
    run_peer_gate(session, "", &[])
}

/// [`load_dir`] with a url-dep remote — the `*_with` lane: HOW
/// bytes arrive is the remote's (transport, cache, offline policy);
/// the loader still owns WHAT they are (the pin, at the mount door).
/// The walk collects the url rows, awaits `fetch` per url
/// sequentially, then mounts over the bytes map.
pub async fn load_dir_with(
    dir: &Path,
    remote: &dyn DepRemote,
) -> Result<Loaded, RunError> {
    let map = prefetch_urls(dir, &FsSource, remote).await?;
    load_dir_fetched(dir, &FsSource, &map)
}

/// [`load_path_session`] with a url-dep remote. A `.rutbundle` path
/// never fetches — bundles are closed; only a directory's `[deps]` can
/// name urls.
pub async fn load_path_session_with(
    path: &Path,
    remote: &dyn DepRemote,
) -> Result<Loaded, RunError> {
    if path.is_dir() {
        return load_dir_with(path, remote).await;
    }
    if path.extension().map_or(false, |e| e == "rutbundle") {
        return load_bundle_session(path);
    }
    Err(LoadError::Shape {
        path: path.display().to_string(),
    }.into())
}

/// [`dir_pkgs`] with a url-dep remote — collects the url rows,
/// fetches, mounts over the bytes map. The gate still does not run
/// here (the offer law).
pub async fn dir_pkgs_with(
    dir: &Path,
    remote: &dyn DepRemote,
) -> Result<Loaded, RunError> {
    let map = prefetch_urls(dir, &FsSource, remote).await?;
    dir_pkgs_fetched(dir, &map)
}
