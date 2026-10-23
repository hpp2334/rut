//! The directory walk — the loader's whole law set, moved out of the
//! driver and rebased on string keys. One module is one directory with
//! ONE entry file: its `use` statements are all inter-module paths
//! (`use <pkg>::{A, B};`), resolved by exact name — there is no
//! intra-module include form. A `.rutbundle` is the same contract
//! zipped; a `[deps]` row may also declare a **url**: a remote
//! `.rutbundle` the CALL SITE fetches ([`DepRemote`]) and the walk
//! mounts — the sha256 pin is LOCK law (`rut.lock` beside the root
//! manifest; the manifest row's `sha256` is retired), read fresh on
//! every load and verified here at the mount door.
//!
//! The walk yields [`rut_driver::Walk`] — pure pkgs plus the root's
//! name. The mount order is the law:
//! 1. the `[deps]` walk;
//! 2. the dev pass — the ROOT's `[dev-deps]` mount exactly like
//!    `[deps]` (a dep's dev table is never walked, so a consumer's
//!    world never contains it);
//! 3. compile — the run chain's `.compile()`; the graph sees ordinary
//!    sources.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rut_core::id::ScopeId;

use rut_driver::bundle::{
    parse_manifest, parse_manifest_compat, read_entry, Manifest, PkgType, MANIFEST_NAME,
};
use rut_driver::mods::{mount_mod_children, ChildLookup, ModSource};
use rut_driver::{Mode, Mounted, MountedBody, RunError, Walk};

use crate::error::LoadError;
use crate::remote::DepRemote;
use crate::source::{FsSource, Source};

// ---------------------------------------------------------------------
// the walk's world
// ---------------------------------------------------------------------

/// One fetched archive's decoded entries, kept by the CALLER (the walk
/// stays I/O-free beyond the source; it records only where files
/// live: slot + prefix). `origin` is the url, for error messages.
/// The value is the driver's (the pack emission reads it); this
/// module re-exports it at its walk-side home.
pub use rut_driver::pack::Archive;

/// Where a walked pkg's files live — the D3 checks and the pack lanes'
/// group reads dispatch on it: a `Dir` pkg reads from its directory,
/// an `Unpacked` pkg from the content-addressed directory a source-zip
/// url dep unpacked to (the pack lanes read its tree like any dir's),
/// an `Archive` pkg from the fetched archive's entries at its prefix.
/// WALK-LOCAL bookkeeping: it never rides the [`Mounted`] (pure data).
#[derive(Clone, Debug)]
pub(crate) enum Loc {
    Dir,
    Unpacked(PathBuf),
    Archive { slot: usize, prefix: String },
}

/// The walk's world: the mounted table (first mount wins — enforced by
/// the callers), where each pkg's files live, the pack-time scope
/// ledger (per-archive namespacing shifts rows above everything
/// recorded so far), the archives the url deps opened (slot-ordered),
/// and the root source-zip url deps unpack under
/// (`<project>/.rut/unpacked`, seeded by the walk entry from the
/// project dir).
#[derive(Default)]
pub(crate) struct World {
    pub(crate) pkgs: BTreeMap<String, Mounted>,
    pub(crate) locs: BTreeMap<String, Loc>,
    ledger: BTreeMap<ScopeId, String>,
    pub(crate) archives: Vec<Archive>,
    pub(crate) unpack_root: PathBuf,
}

impl World {
    pub(crate) fn new() -> World {
        World::default()
    }

    /// A mounted pkg's body — the pack lane's group-kind decisions read
    /// it.
    pub(crate) fn pkg_body(&self, spec: &str) -> Option<&MountedBody> {
        self.pkgs.get(spec).map(|p| &p.body)
    }

    /// The archive location a spec mounted from: `(slot, prefix)`.
    pub(crate) fn archive_of(&self, spec: &str) -> Option<(usize, String)> {
        match self.locs.get(spec) {
            Some(Loc::Archive { slot, prefix }) => Some((*slot, prefix.clone())),
            _ => None,
        }
    }

    /// The unpacked directory a source-zip url dep mounted from — the
    /// pack lanes read its tree like any dir dep's.
    pub(crate) fn unpacked_of(&self, spec: &str) -> Option<&Path> {
        match self.locs.get(spec) {
            Some(Loc::Unpacked(p)) => Some(p.as_path()),
            _ => None,
        }
    }

    /// Every archive-mounted spec: `(slot, prefix)`, spec order.
    pub(crate) fn archive_locs(&self) -> BTreeMap<String, (usize, String)> {
        self.locs
            .iter()
            .filter_map(|(spec, loc)| match loc {
                Loc::Archive { slot, prefix } => Some((spec.clone(), (*slot, prefix.clone()))),
                Loc::Dir | Loc::Unpacked(_) => None,
            })
            .collect()
    }

    /// Mount a pkg under `spec` — the table keeps the last; the
    /// first-mount-wins law is the CALLERS' (they resolve-check first).
    /// The spec rides the pkg (the walk's registration IS the name —
    /// the same law the run chain's table holds). A key is a bare
    /// package name or a content identity (`name@fp16` — the key a
    /// bundle group mounts under).
    fn register(&mut self, spec: &str, pkg: Mounted) -> Result<(), LoadError> {
        if !rut_driver::bundle::valid_mount_key(spec) {
            return Err(LoadError::law(format!(
                "`{spec}` is not a package name — expected `[a-zA-Z0-9_]+` or `name@<16 hex>`"
            )));
        }
        let mut pkg = pkg;
        pkg.spec = spec.to_string();
        self.pkgs.insert(spec.to_string(), pkg);
        Ok(())
    }

    /// The first free scope number above every recorded ledger row —
    /// a freshly mounted archive's namespace base (`None` when the
    /// numbering space is exhausted). Boot owns 0 and passes through
    /// every map.
    fn ledger_next_base(&self) -> Option<ScopeId> {
        let max = self
            .ledger
            .keys()
            .copied()
            .max()
            .unwrap_or(rut_core::id::BOOT_SCOPE);
        let next = max.checked_add(1)?;
        if next as u32 > rut_core::id::MAX_SCOPE {
            None
        } else {
            Some(next)
        }
    }

    /// Namespace one archive's scope ledger into the walk's numbering
    /// and answer the map that rebases the archive's programs with it —
    /// the CDN law: per-package bundles are packed INDEPENDENTLY, so
    /// their pack-time numberings argue; each archive's rows shift
    /// above everything already recorded (boot passes through) and its
    /// binaries rebase BEFORE mounting, so every id resolves through
    /// its OWN archive's rows. Fails only when the numbering space
    /// itself is exhausted (4096 scopes; boot owns 0).
    fn namescope_ledger(
        &self,
        scopes: &[(ScopeId, String)],
    ) -> Result<impl Fn(ScopeId) -> ScopeId, LoadError> {
        let base = self.ledger_next_base().ok_or_else(|| {
            LoadError::law("the bundle scope numbering space is exhausted".to_string())
        })?;
        for &(s, _) in scopes {
            if s != rut_core::id::BOOT_SCOPE && base as u32 + s as u32 > rut_core::id::MAX_SCOPE {
                return Err(LoadError::law(format!(
                    "the bundle's scope ledger reaches scope {s}, which does not fit above this program's {base} — the numbering space is exhausted"
                )));
            }
        }
        Ok(move |s: ScopeId| {
            if s == rut_core::id::BOOT_SCOPE {
                s
            } else {
                base + s
            }
        })
    }

    /// Record one row of a mounted archive's pack-time scope ledger.
    fn record_scope(&mut self, scope: ScopeId, spec: &str) {
        self.ledger.insert(scope, spec.to_string());
    }

    /// The yield: drain the pkgs (the world dies, the pkgs travel) —
    /// [`Walk`].
    pub(crate) fn into_loaded(self, root: String) -> Walk {
        Walk {
            pkgs: self.pkgs.into_values().collect(),
            root,
        }
    }
}

// ---------------------------------------------------------------------
// manifest + text reads over a Source
// ---------------------------------------------------------------------

/// Read one file's text through the embedder's [`Source`]. The FsSource
/// default spells today's messages: a missing file is
/// `cannot read <path>: <io>`, a non-UTF-8 body is
/// `cannot read <path>: stream did not contain valid UTF-8` — the
/// text `read_to_string` produced.
fn read_source_text(src: &dyn Source, key: &str) -> Result<String, LoadError> {
    let bytes = src
        .read(key)
        .map_err(|e| LoadError::law(format!("cannot read {key}: {e}")))?;
    String::from_utf8(bytes).map_err(|_| {
        LoadError::law(format!(
            "cannot read {key}: stream did not contain valid UTF-8"
        ))
    })
}

/// The manifest's file name — ONE name, no fallback lane: a directory
/// is one module and its manifest is `rut.jsonc` (JSONC: comments and
/// trailing commas legal). A directory still holding the retired name
/// below gets the pointed refusal in [`read_manifest`].
pub(crate) const MANIFEST_RETIRED: &str = "rut.json";

/// Read a directory's `rut.jsonc` — the real-filesystem lane of the
/// bundle module's reader (the module itself never touches the
/// filesystem). A directory still holding the RETIRED `rut.json` name
/// gets the pointed cutover refusal — no fallback lane reads it.
pub(crate) fn read_manifest(src: &dyn Source, dir: &str) -> Result<Manifest, LoadError> {
    let key = src.resolve(dir, MANIFEST_NAME).map_err(LoadError::law)?;
    let text = match read_source_text(src, &key) {
        Ok(text) => text,
        Err(missing) => {
            let retired = src.resolve(dir, "rut.json").map_err(LoadError::law)?;
            if src.read(&retired).is_ok() {
                return Err(LoadError::law(format!(
                    "{retired} found — the manifest is `{MANIFEST_NAME}` (JSONC: comments and \
                     trailing commas legal) since wire 9; re-name the file or re-pack the directory"
                )));
            }
            return Err(missing);
        }
    };
    parse_manifest(&text).map_err(LoadError::from)
}

// ---------------------------------------------------------------------
// the url-dep lanes
// ---------------------------------------------------------------------

/// One url dep row collected by the closure walk: the spec it was
/// declared under (display-only — the mount door re-keys off the
/// manifest row; the lock's `pkg` column and `rut install`'s rows are
/// the consumers) and the url (the identity — the fetch key and the
/// lock's).
struct UrlDep {
    spec: String,
    url: String,
}

/// Phase 1 of the two-phase design — SYNC: walk the root manifest
/// (`[deps]` + `[dev-deps]`) and every directory dep's manifest,
/// collecting the url rows. Url deps are LEAVES — the walk never
/// follows them; directory recursion is cycle-guarded by canonical
/// dirs.
fn collect_url_deps(src: &dyn Source, dir: &str) -> Result<Vec<UrlDep>, LoadError> {
    let mut out = Vec::new();
    let mut visiting = Vec::new();
    collect_url_deps_walk(src, dir, &mut visiting, &mut out)?;
    Ok(out)
}

fn collect_url_deps_walk(
    src: &dyn Source,
    dir: &str,
    visiting: &mut Vec<PathBuf>,
    out: &mut Vec<UrlDep>,
) -> Result<(), LoadError> {
    let canonical = canonical(dir)?;
    if visiting.contains(&canonical) {
        return Err(LoadError::law(format!(
            "cyclic use: `{dir}` is already being scanned for url deps"
        )));
    }
    visiting.push(canonical);
    let manifest = read_manifest(src, dir)?;
    // [deps] and [dev-deps]: the tables the walk and the packer's
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
                });
            } else if let Some(rel) = desc.get("path") {
                let dep_dir = src.resolve(dir, rel).map_err(LoadError::law)?;
                collect_url_deps_walk(src, &dep_dir, visiting, out)?;
            }
        }
    }
    visiting.pop();
    Ok(())
}

/// The canonical form of a directory key — the cycle guards' identity
/// (the one place the walk touches `Path` math beyond the source).
fn canonical(key: &str) -> Result<PathBuf, LoadError> {
    Path::new(key)
        .canonicalize()
        .map_err(|e| LoadError::law(format!("cannot resolve {key}: {e}")))
}

/// The lock file's one name — beside the walk root's `rut.jsonc`.
const LOCK_NAME: &str = "rut.lock";

/// The walk root's `rut.lock` as the pin map the walk travels with
/// (url → sha256). MISSING file ⇒ an empty map — a project with no
/// url deps needs no lock. Present but malformed ⇒ the loud load
/// error naming the file and the parse error (`line N:` syntax,
/// path-targeted value laws — the manifest's shapes, through the same
/// JSONC front stage). One lock per root: it pins the whole closure's
/// url rows (root + every path-dep manifest's `[deps]`/`[dev-deps]`;
/// url deps are leaves, so nothing beyond them is ever read), and the
/// url is the identity — the map's key.
pub(crate) fn read_lock(
    src: &dyn Source,
    dir: &str,
) -> Result<BTreeMap<String, String>, LoadError> {
    let key = src.resolve(dir, LOCK_NAME).map_err(LoadError::law)?;
    let bytes = match src.read(&key) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(BTreeMap::new()), // no lock file ⇒ no pins
    };
    let text = String::from_utf8(bytes).map_err(|_| LoadError::law(format!("{key}: not UTF-8")))?;
    let lock = rut_driver::parse_lock(&text).map_err(|e| LoadError::law(format!("{key}: {e}")))?;
    Ok(lock
        .deps()
        .iter()
        .map(|row| (row.url.clone(), row.sha256.clone()))
        .collect())
}

/// Phase 2 — the async join: await `fetch` per url row, SEQUENTIALLY,
/// into the bytes map the sync core reads. The map is the join (a call
/// site wanting concurrency does it inside its `fetch`); equal urls
/// fetch once. The LOCK gates the wire: an unpinned url is refused
/// BEFORE any fetch — the walk never fetches a dep the lock does not
/// pin (the mount door repeats the check for the injected-map lanes).
pub async fn prefetch_urls(
    src: &dyn Source,
    dir: &str,
    pins: &BTreeMap<String, String>,
    remote: &dyn DepRemote,
) -> Result<BTreeMap<String, Vec<u8>>, LoadError> {
    let rows = collect_url_deps(src, dir)?;
    let mut map = BTreeMap::new();
    for row in &rows {
        if map.contains_key(&row.url) {
            continue;
        }
        if !pins.contains_key(&row.url) {
            return Err(LoadError::Unpinned {
                spec: row.spec.clone(),
                url: row.url.clone(),
            });
        }
        let bytes = remote.fetch(&row.url).await?;
        map.insert(row.url.clone(), bytes);
    }
    Ok(map)
}

/// The closure's url dep rows — `(spec, url)` — the collector the CLI
/// walks BEFORE it can pin anything: root manifest + every path-dep
/// manifest's `[deps]`/`[dev-deps]` url rows, url deps are leaves.
/// Deduped by url (the identity): the FIRST spec in collect order
/// wins (the lock's `pkg` column — display-only). Deterministic: the
/// manifest tables are ordered maps, the recursion is depth-first.
pub fn url_dep_rows(dir: &Path) -> Result<Vec<(String, String)>, RunError> {
    let src = FsSource::at(dir);
    let rows = collect_url_deps(&src, dir.to_string_lossy().as_ref())?;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        if seen.insert(row.url.clone()) {
            out.push((row.spec, row.url));
        }
    }
    Ok(out)
}

/// The no-fetcher law: a url row in a loader that has no bytes for it.
/// Names the FIX, matching the D-style diagnostics — and the lock:
/// without a fetcher neither the bytes nor their rut.lock pin-check
/// can happen.
fn no_fetcher(spec: &str, url: &str) -> LoadError {
    LoadError::law(format!(
        "dep `{spec}` is declared by url (`{url}`) — this loader has no `dep_fetch`, so the \
         fetch and its rut.lock pin-check cannot run: call `load_dir_with` / `pack_dir_with` \
         (the CLI does), or vendor the dep"
    ))
}

/// The url-dep context the sync core walks with: the fetched bytes
/// (url → bytes), the LOCK's pins (url → sha256 — gate 0's law), and
/// the archives those bytes opened (slot-ordered). The OLD sync
/// wrappers pass an empty map — a url row there is the loud
/// no-fetcher error, never a network call (the walk never fetches;
/// bundles stay closed at load, url rows are leaves).
struct Fetched<'a> {
    map: &'a BTreeMap<String, Vec<u8>>,
    lock: &'a BTreeMap<String, String>,
    /// defense-in-depth cycle guard: url deps are leaves, so no
    /// recursion can re-enter a url — the guard keeps that true even
    /// if the walk ever grows one.
    visiting_urls: BTreeSet<String>,
}

// ---------------------------------------------------------------------
// entry modules (dir flavor)
// ---------------------------------------------------------------------

/// The lib body's root file — `src/mod.rut`, inside the package's
/// `src/` shell beside the manifest. Children mount beside the root
/// file, so the whole module tree lives under `src/`. The spelled-
/// surface helper and the d-flavored default root ride the driver's
/// shared spellings.
pub(crate) use rut_driver::bundle::src_rel;
pub(crate) use rut_driver::bundle::DECL_ROOT_MODULE as DECL_ROOT_FILE;
pub(crate) use rut_driver::bundle::ROOT_MODULE as ROOT_FILE;

/// The root-file probe: `rel` (a manifest-relative path) —
/// `Ok(Some((text, path)))` when the package carries it (it is the
/// root module / root surface), `Ok(None)` when absent. A file that
/// exists but cannot be read is the loud read error, never a silent
/// fallback. The path rides back: the caller seeds the mod mount's
/// child base with it (children probe beside the root file).
fn read_root_file(dir: &str, rel: &str) -> Result<Option<(String, PathBuf)>, LoadError> {
    let path = PathBuf::from(dir).join(rel);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some((text, path))),
        Err(e) if path.is_file() => Err(LoadError::law(format!(
            "cannot read {}: {e}",
            path.display()
        ))),
        Err(_) => Ok(None),
    }
}

/// The `src/mod.rut` probe: the lib body's root module.
fn read_root_module(dir: &str) -> Result<Option<(String, PathBuf)>, LoadError> {
    read_root_file(dir, ROOT_FILE)
}

/// The host pkg's default root file — `src/mod.d.rut`, the lib body
/// tree's `src/mod.rut` probe d-flavored (the Decl flavor's spelling;
/// [`rut_driver::decl::DECL_CHILD_FILE`] remains the child file name).
/// Re-exported from the driver above as [`DECL_ROOT_FILE`].
///
/// The `src/mod.d.rut` probe: the host arm's default root, [`read_root_module`]
/// d-flavored.
fn read_decl_root_module(dir: &str) -> Result<Option<(String, PathBuf)>, LoadError> {
    read_root_file(dir, DECL_ROOT_FILE)
}

/// The dir lane's child lookup for the mod mount — the same case law
/// the archive rows lane answers, over TWO spellings: the directory
/// form `NAME/<child file>` first, else the single-file form
/// `NAME.rut` beside the declaring file; BOTH present is the loud
/// ambiguity (which mounts is undecidable). A `NAME/` directory
/// without its module file, or nothing there at all, are the loud
/// misses. The found answer carries the file's canonical identity
/// (the cycle guard's key, so a symlinked alias repeats it) and its
/// physical name (the diagnostics spell the real shape). The child
/// file's spelling is the caller's flavor: `mod.rut` for a lib body
/// tree, `mod.d.rut` for a host surface tree.
///
/// `files` remembers the PHYSICAL file of every found child, keyed by
/// mod path: a file-form parent (`a.rut`) lives in its own directory —
/// its children mount beside it, never under a dir named after the mod
/// path (the parent's file dir IS the probe base, both forms). Seeded
/// with the root's physical file at mod path `""`, so the root's
/// children probe beside the root file — inside the package's `src/`
/// shell.
pub(crate) fn dir_child_lookup(
    dir: &Path,
    files: &mut BTreeMap<String, PathBuf>,
    parent: &str,
    name: &str,
    child_file: &str,
) -> Result<ChildLookup, String> {
    let base = match files.get(parent) {
        Some(f) => f
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.to_path_buf()),
        None if parent.is_empty() => dir.to_path_buf(),
        None => dir.join(parent),
    };
    let mod_rs = base.join(name).join(child_file);
    let file_form = base.join(format!("{name}.rut"));
    let has_dir = mod_rs.is_file();
    let has_file = file_form.is_file();
    if has_dir && has_file {
        return Ok(ChildLookup::Ambiguous);
    }
    let probe = if has_dir {
        (mod_rs, child_file.to_string())
    } else if has_file {
        (file_form, format!("{name}.rut"))
    } else if base.join(name).is_dir() {
        return Ok(ChildLookup::NoModRut);
    } else {
        return Ok(ChildLookup::Missing);
    };
    match std::fs::read_to_string(&probe.0) {
        Ok(text) => {
            let canon = std::fs::canonicalize(&probe.0)
                .unwrap_or_else(|_| probe.0.clone())
                .to_string_lossy()
                .into_owned();
            let child_path = if parent.is_empty() {
                name.to_string()
            } else {
                format!("{parent}/{name}")
            };
            files.insert(child_path, probe.0.clone());
            Ok(ChildLookup::Found {
                text,
                canon,
                file: probe.1,
            })
        }
        // a file that exists but cannot be read is the loud read
        // error, never a silent fallback
        Err(e) => Err(format!("cannot read {}: {e}", probe.0.display())),
    }
}

/// Assemble a lib pkg from its assembled root text: the root's `mod`
/// declarations mount the child tree from the directory —
/// `NAME/mod.rut` beside the declaring file, else the single-file
/// `NAME.rut`, recursively, cycle-guarded by canonical file identity.
/// The root's own file (`root_file` — `src/mod.rut`) opens the guard's
/// chain and seeds the child base, so the tree mounts inside `src/`.
fn lib_pkg(
    dir: &str,
    root_text: String,
    root_file: &Path,
    manifest: &Manifest,
) -> Result<Mounted, LoadError> {
    let root_canon = std::fs::canonicalize(root_file)
        .unwrap_or_else(|_| root_file.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let mut files = BTreeMap::new();
    files.insert(String::new(), root_file.to_path_buf());
    let mut lookup = |parent: &str, name: &str| {
        dir_child_lookup(Path::new(dir), &mut files, parent, name, "mod.rut")
    };
    let mods: BTreeMap<String, ModSource> =
        mount_mod_children(&root_text, &root_canon, Mode::Impl, "mod.rut", &mut lookup)
            .map_err(|e| LoadError::law(format!("{dir}: {e}")))?;
    Ok(Mounted {
        body: MountedBody::Source {
            text: root_text,
            is_decl: false,
        },
        entry: manifest.entry.clone(),
        mods,
        manifest: manifest.clone(),
        ..Default::default()
    })
}

/// Build a directory's entry [`Mounted`] from its manifest:
///
/// - a `type = "host"` pkg — a pure declaration surface. The `.d.rut`
///   parses in declaration mode and lowers into the module's host fns;
///   its `mod` declarations mount the child tree (`NAME/mod.d.rut`
///   beside the declaring file, recursively, cycle-guarded) and every
///   row carries the path it lives under. No body exists — the
///   embedding Rust binds it at run time. The root surface is the
///   spelled `entry.type` (resolved under `src/`) when the manifest
///   carries one, else `src/mod.d.rut` — the default root, lib parity
///   with a body's `src/mod.rut`; neither present is the loud no-entry
///   error naming both spellings.
/// - a `type = "lib"` pkg (the default) — a source module: `src/mod.rut`
///   IS the body (the root module); the surface derives from its
///   exports. No `src/mod.rut` with a spelled surface (also under
///   `src/`) and the kind declared is the surface-only dev state (a
///   decl unit); the ambiguity (surface spelled, kind missing, no body)
///   refuses, naming both fixes. `host fn` text is refused in either
///   file — the lib-surface law.
///
/// The root's `mod` declarations mount the child tree —
/// `NAME/mod.rut` beside the declaring file (else the single-file
/// `NAME.rut`), recursively, cycle-guarded — the tree living under the
/// package's `src/` shell. The repealed `entry.lib`/`entry.libs` keys
/// refused at parse (the manifest walk); a published BUNDLE's old
/// manifest rides the reader's compat lane instead (a loading law).
///
/// The manifest's `namespace` row (when present) rides the pkg as the
/// qualified-access head (`calc`'s `Math`), and its `consts` rows ride
/// the host body's constants — the surface grammar has no spelling for
/// either, so the manifest carries what the engine mounts.
fn load_entry_module(
    src: &dyn Source,
    dir: &str,
    manifest: &Manifest,
) -> Result<Mounted, LoadError> {
    match manifest.pkg_type {
        PkgType::Host => {
            // the root surface: a spelled `entry.type` WINS, resolved
            // under `src/` (existing flat pkgs keep their
            // `calc.d.rut`-style roots — moved under the shell);
            // absent, `src/mod.d.rut` is the default root — neither
            // present is the loud no-entry error naming both doors
            // (the lib arm's ambiguity refusal, d-flavored)
            let rel = match manifest.entry.type_path.as_ref() {
                Some(rel) => src_rel(rel),
                None => match read_decl_root_module(dir)? {
                    Some(_) => DECL_ROOT_FILE.to_string(),
                    None => {
                        return Err(LoadError::law(format!(
                            "module in {dir} is a host pkg with no entry — spell \
                             `\"entry\": {{ \"type\": \"<surface>.d.rut\" }}` or add a \
                             `src/mod.d.rut` beside the manifest"
                        )));
                    }
                },
            };
            let key = src.resolve(dir, &rel).map_err(LoadError::law)?;
            let src_text = read_source_text(src, &key)?;
            // the surface's mod tree mounts beside it — `NAME/mod.d.rut`
            // next to the declaring file, recursively, cycle-guarded;
            // the root file's identity opens the guard's chain and
            // seeds the child base
            let root_canon = std::fs::canonicalize(&key)
                .unwrap_or_else(|_| PathBuf::from(&key))
                .to_string_lossy()
                .into_owned();
            let mut files = BTreeMap::new();
            files.insert(String::new(), PathBuf::from(&key));
            let mut lookup = |parent: &str, name: &str| {
                dir_child_lookup(Path::new(dir), &mut files, parent, name, "mod.d.rut")
            };
            let mods: BTreeMap<String, ModSource> =
                mount_mod_children(&src_text, &root_canon, Mode::Decl, "mod.d.rut", &mut lookup)
                    .map_err(|e| LoadError::law(format!("{dir}: {e}")))?;
            let mut m =
                rut_driver::lower_decl_module(&src_text, &mods, &key).map_err(LoadError::law)?;
            m.entry = manifest.entry.clone();
            m.namespace = manifest.namespace.clone();
            // the consts ride the host body (the manifest is their only
            // spelling)
            let consts = manifest_consts(manifest);
            if let MountedBody::Host { consts: rows, .. } = &mut m.body {
                *rows = consts;
            }
            m.manifest = manifest.clone();
            Ok(m)
        }
        PkgType::Lib => {
            // the surface read + the lib-surface law (kept from today);
            // a spelled surface resolves under the `src/` shell
            let mut surface_only: Option<String> = None;
            if let Some(rel) = &manifest.entry.type_path {
                let key = src.resolve(dir, &src_rel(rel)).map_err(LoadError::law)?;
                let src_text = read_source_text(src, &key)?;
                rut_driver::decl::refuse_host_rows(&src_text, &key).map_err(LoadError::law)?;
                surface_only = Some(src_text);
            }
            // the body: `src/mod.rut` — the root module, always.
            // Without one, the surface-only dev state stands (a decl
            // unit) — but the AMBIGUITY refuses: an `entry.type`
            // spelled with no declared kind and no body names neither
            // host nor lib.
            match read_root_module(dir)? {
                Some((root, root_file)) => lib_pkg(dir, root, &root_file, manifest),
                None => match surface_only {
                    Some(surface) if manifest.type_declared => Ok(Mounted {
                        body: MountedBody::Source {
                            text: surface,
                            is_decl: true,
                        },
                        entry: manifest.entry.clone(),
                        manifest: manifest.clone(),
                        ..Default::default()
                    }),
                    Some(_) => Err(LoadError::law(format!(
                        "module in {dir} spells `entry.type` but no kind — `\"type\": \"host\"` \
                         for a host pkg, or `\"type\": \"lib\"` for the surface-only dev state"
                    ))),
                    None => Err(LoadError::law(format!(
                        "module in {dir} has no entry — the root module is `src/mod.rut`"
                    ))),
                },
            }
        }
    }
}

/// The manifest's `consts` rows as the host body's constant table:
/// `(name, f64, raw bits)` — the compiler materializes them.
fn manifest_consts(manifest: &Manifest) -> Vec<(String, rut_core::types::TypeId, u64)> {
    manifest
        .consts
        .iter()
        .map(|(name, v)| (name.clone(), rut_core::types::TY_F64, v.to_bits()))
        .collect()
}

// ---------------------------------------------------------------------
// the walk cores
// ---------------------------------------------------------------------

/// Read a module directory (`rut.jsonc`) or a packed `.rutbundle` into
/// walked pkgs — the two packed forms of the same contract. A loose
/// `.rut` file is NOT this: it is a single-file module with no
/// manifest.
pub fn load_path_session(path: &Path) -> Result<Walk, RunError> {
    if path.is_dir() {
        return load_dir(path);
    }
    if path.extension().is_some_and(|e| e == "rutbundle") {
        return load_bundle_session(path);
    }
    Err(LoadError::Shape {
        path: path.display().to_string(),
    }
    .into())
}

/// [`load_path_session`] with a url-dep remote. A `.rutbundle` path
/// never fetches — bundles are closed; only a directory's `[deps]` can
/// name urls.
pub async fn load_path_session_with(path: &Path, remote: &dyn DepRemote) -> Result<Walk, RunError> {
    if path.is_dir() {
        return load_dir_with(path, remote).await;
    }
    if path.extension().is_some_and(|e| e == "rutbundle") {
        return load_bundle_session(path);
    }
    Err(LoadError::Shape {
        path: path.display().to_string(),
    }
    .into())
}

/// The walk: a module directory's whole closure as walked pkgs —
/// offer them to a run with `RutRun::new().pkg(..)`. Url deps need a
/// remote: this lane is the loud no-fetcher error, never a network
/// call.
pub fn load_dir(dir: &Path) -> Result<Walk, RunError> {
    load_dir_fetched(dir, &BTreeMap::new())
}

/// [`load_dir`] over PRE-FETCHED url bytes — the pub sync core the
/// `*_with` lanes call after [`prefetch_urls`], and the fixture lane
/// tests use directly. The root's `rut.lock` is read from disk (the
/// realistic lane — a fixture that fetches is a project that
/// installed) and every url row's pin is verified HERE, at the mount
/// door, on every load.
pub fn load_dir_fetched(dir: &Path, map: &BTreeMap<String, Vec<u8>>) -> Result<Walk, RunError> {
    let src = FsSource::at(dir);
    let lock = read_lock(&src, dir.to_string_lossy().as_ref())?;
    load_dir_resolved(dir, map, &lock)
}

/// [`load_dir_fetched`] with the pins INJECTED — install's
/// chicken-and-egg lane: mount-validate against COMPUTED pins
/// (url → sha256, the map `rut install` is about to write as
/// `rut.lock`) without requiring the lock file to exist. Same walk,
/// same mount-door laws; the only difference is where the pins come
/// from.
pub fn load_dir_resolved(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
    pins: &BTreeMap<String, String>,
) -> Result<Walk, RunError> {
    let src = FsSource::at(dir);
    let mut world = World::new();
    let root = walk_dir(&mut world, &src, dir.to_string_lossy().as_ref(), map, pins)?;
    Ok(world.into_loaded(root))
}

/// [`load_dir`] with a url-dep remote — the `*_with` lane: HOW
/// bytes arrive is the remote's (transport, cache, offline policy);
/// the walk still owns WHAT they are (the pin, at the mount door).
/// The lock is read from the root first — it gates the prefetch —
/// then the walk collects the url rows, awaits `fetch` per url
/// sequentially, and mounts over the bytes map.
pub async fn load_dir_with(dir: &Path, remote: &dyn DepRemote) -> Result<Walk, RunError> {
    let src = FsSource::at(dir);
    let lock = read_lock(&src, dir.to_string_lossy().as_ref())?;
    let map = prefetch_urls(&src, dir.to_string_lossy().as_ref(), &lock, remote).await?;
    load_dir_resolved(dir, &map, &lock)
}

/// The internal sync core — mount the root and every `[deps]` row into
/// `world`, first mount wins, and answer the root's name. `lock` is
/// the pin map the whole closure's url rows travel with.
pub(crate) fn walk_dir(
    world: &mut World,
    src: &dyn Source,
    dir: &str,
    map: &BTreeMap<String, Vec<u8>>,
    lock: &BTreeMap<String, String>,
) -> Result<String, LoadError> {
    let manifest = read_manifest(src, dir)?;
    let root = manifest.name.clone().ok_or_else(|| {
        LoadError::law(format!("{}{MANIFEST_NAME} has no `name`", dir_key_dir(dir),))
    })?;
    // the source-zip unpack root: `<project>/.rut/unpacked` — seeded
    // once from the project dir (the cache's sibling convention); a
    // walk that never meets a `.zip` url dep never touches it
    if world.unpack_root.as_os_str().is_empty() {
        world.unpack_root = Path::new(dir).join(".rut").join("unpacked");
    }
    let root_module = load_entry_module(src, dir, &manifest)?;
    world.register(&root, root_module)?;
    // the root's dir rides the world, so the pack lanes can answer for
    // the root's own files
    world.locs.insert(root.clone(), Loc::Dir);
    let mut visiting = vec![canonical(dir)?];
    let mut fetched = Fetched {
        map,
        lock,
        visiting_urls: Default::default(),
    };
    resolve_table(
        world,
        src,
        dir,
        &root,
        &manifest.deps,
        &mut visiting,
        &mut fetched,
    )?;
    // the dev pass — the ROOT's dev table mounts exactly like `[deps]`;
    // a dep's dev table is never walked (pass 2 is root-only)
    resolve_table(
        world,
        src,
        dir,
        &root,
        &manifest.dev_deps,
        &mut visiting,
        &mut fetched,
    )?;
    Ok(root)
}

/// The dep dir's IR file, if it carries one instead of a package
/// tree: exactly one `.rutbundle` entry and no manifest. `None` when
/// the dir is an ordinary package (or empty — the manifest read
/// diagnoses that).
fn ir_bundle_file(dep_dir: &Path) -> Result<Option<PathBuf>, LoadError> {
    let entries = match std::fs::read_dir(dep_dir) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    let mut bundles = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| LoadError::law(format!("{}: {e}", dep_dir.display())))?;
        let name = entry.file_name();
        let name = name.to_string_lossy().into_owned();
        if name == rut_driver::bundle::MANIFEST_NAME {
            return Ok(None); // an ordinary package tree
        }
        if name.ends_with(".rutbundle") && entry.path().is_file() {
            bundles.push(entry.path());
        }
    }
    match bundles.len() {
        0 => Ok(None),
        1 => Ok(Some(bundles.remove(0))),
        _ => Err(LoadError::law(format!(
            "{}: more than one `.rutbundle` — a dep dir carries exactly one IR file",
            dep_dir.display()
        ))),
    }
}

/// The dir prefix in manifest error texts: today's messages spell
/// `<dir>/rut.jsonc`; a key already ends without the separator.
fn dir_key_dir(dir: &str) -> String {
    if dir.is_empty() {
        String::new()
    } else if dir.ends_with('/') {
        dir.to_string()
    } else {
        format!("{dir}/")
    }
}

/// The mount key a dep directory mounts under — the shared derivation
/// the walk and the pack lanes agree on: a directory named
/// `<name>@<fp16>` (an identity dir — the wire group loop's law,
/// dir-flavored) mounts under its identity, a plain name under the
/// row's bare key.
pub(crate) fn mount_key_for(dep_dir: &str, spec: &str) -> String {
    let base = Path::new(dep_dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if rut_driver::bundle::is_identity(&base) {
        base
    } else {
        spec.to_string()
    }
}

/// Resolve a manifest's `[deps]` recursively — pass 1 of
/// the mount order. `parent` is the spec of the pkg whose table walks
/// (the first-mount edge every mount below records).
fn resolve_deps(
    world: &mut World,
    src: &dyn Source,
    dir: &str,
    parent: &str,
    manifest: &Manifest,
    visiting: &mut Vec<PathBuf>,
    fetched: &mut Fetched<'_>,
) -> Result<(), LoadError> {
    resolve_table(world, src, dir, parent, &manifest.deps, visiting, fetched)
}

/// The dev-table mount — the pack lane's compile-once-per-owner pass:
/// a DIR dep group's own `[dev-deps]` mount into the pack's world, so
/// its unit compiles over the same closure wherever it is packed.
/// NEVER part of a consumer's world.
/// `parent` is the group's own spec (the first-mount edge).
pub(crate) fn mount_dev_table(
    world: &mut World,
    src: &dyn Source,
    dir: &str,
    parent: &str,
    manifest: &Manifest,
    map: &BTreeMap<String, Vec<u8>>,
    lock: &BTreeMap<String, String>,
) -> Result<(), LoadError> {
    let mut visiting = Vec::new();
    let mut fetched = Fetched {
        map,
        lock,
        visiting_urls: Default::default(),
    };
    resolve_table(
        world,
        src,
        dir,
        parent,
        &manifest.dev_deps,
        &mut visiting,
        &mut fetched,
    )
}

/// Walk one descriptor table — the `[deps]` walk (pass 1; pass 2 feeds
/// it the root's `[dev-deps]`, which mounts exactly the same way).
/// Each entry is a relative `path` to a package directory, loaded and
/// mounted under its key — or a `url`, whose pre-fetched bytes mount
/// through [`mount_url_dep`] (the rut.lock pin is law at that door;
/// the walk NEVER fetches, and a url dep is a leaf). **First mount
/// wins** — a name already in the world (the embedder's, the root's,
/// or an earlier dep's) is never overwritten; a dep whose manifest
/// `name` disagrees with its key is an error naming both. `visiting`
/// guards cycles; `parent` is the spec of the pkg whose table this is
/// — the first-mount edge every pkg mounted here records
/// ([`Mounted::mounted_by`]). Every mounted pkg's manifest and directory
/// (or archive location) are recorded — the pack lanes' group reads
/// dispatch on them.
fn resolve_table(
    world: &mut World,
    src: &dyn Source,
    dir: &str,
    parent: &str,
    table: &BTreeMap<String, BTreeMap<String, String>>,
    visiting: &mut Vec<PathBuf>,
    fetched: &mut Fetched<'_>,
) -> Result<(), LoadError> {
    for (spec, desc) in table {
        if world.pkgs.contains_key(spec) {
            continue; // already mounted — the embedder's (or an earlier) mount wins
        }
        if let Some(url) = desc.get("url") {
            let bytes = fetched.map.get(url).ok_or_else(|| no_fetcher(spec, url))?;
            if !fetched.visiting_urls.insert(url.clone()) {
                return Err(LoadError::law(format!(
                    "cyclic use: `{spec}` ({url}) is already being loaded"
                )));
            }
            mount_url_dep(world, spec, url, parent, fetched, bytes)?;
            fetched.visiting_urls.remove(url);
            continue;
        }
        let rel = desc
            .get("path")
            .ok_or_else(|| LoadError::law(format!("dep `{spec}` has no `path`")))?;
        let dep_dir = src.resolve(dir, rel).map_err(LoadError::law)?;
        let dep_canon = canonical(&dep_dir)?;
        if visiting.contains(&dep_canon) {
            return Err(LoadError::law(format!(
                "cyclic use: `{spec}` ({dep_dir}) is already being loaded"
            )));
        }
        // the mount key: a dep directory named `<name>@<fp16>` — an
        // IDENTITY dir, the wire group loop's law, dir-flavored —
        // mounts under its identity (two versions of one name coexist
        // side by side); a plain name mounts bare. The name-vs-key
        // gate reads the identity's name half either way.
        let mount_key = mount_key_for(&dep_dir, spec);
        if world.pkgs.contains_key(&mount_key) {
            continue; // first mount wins — per-identity exact dedup
        }
        // an embedded IR dep: a dep directory carrying a `.rutbundle`
        // file instead of a package tree — the dirname's identity is
        // the bundle's own root fingerprint (the packer wrote it; a
        // doctored dirname refuses), and the bytes mount through the
        // wire lane verbatim (its format_version gates apply: a
        // too-old bundle refuses loudly on a newer toolchain, by
        // design)
        if let Some(ir) = ir_bundle_file(Path::new(&dep_dir))? {
            let bytes = std::fs::read(&ir)
                .map_err(|e| LoadError::law(format!("cannot read {}: {e}", ir.display())))?;
            let parsed = rut_driver::bundle::parse_bundle(&bytes)
                .map_err(|e| LoadError::law(format!("{}: {e}", ir.display())))?;
            let got_fp = rut_driver::bundle::fp16_of(&parsed, "");
            let (iname, ifp) = rut_driver::bundle::parse_identity(&mount_key)
                .ok_or_else(|| LoadError::law(format!("{}: not an identity dir", ir.display())))?;
            if got_fp != ifp {
                return Err(LoadError::law(format!(
                    "`{iname}@{ifp}/` spells fingerprint `{ifp}`, but its bundle hashes to \
                     `{got_fp}` — the dirname and the content disagree"
                )));
            }
            return mount_bundle_bytes(
                world,
                &mount_key,
                &ir.display().to_string(),
                parent,
                &bytes,
            );
        }
        let dm = read_manifest(src, &dep_dir)?;
        if dm.name.as_deref() != Some(rut_driver::bundle::mount_name(&mount_key)) {
            return Err(LoadError::DepName {
                spec: spec.clone(),
                location: dep_dir.clone(),
                actual: dm.name.as_deref().unwrap_or("<unnamed>").to_string(),
            });
        }
        let mut dep_pkg = load_entry_module(src, &dep_dir, &dm)?;
        // the first-mount edge: the declaring pkg's spec (diagnostic
        // data — the uses diagnostic's "pulled by" clause reads it)
        dep_pkg.mounted_by = Some(parent.to_string());
        world.register(&mount_key, dep_pkg)?;
        world.locs.insert(mount_key.clone(), Loc::Dir);
        visiting.push(dep_canon);
        // a dep's dev-deps are NEVER walked — pass 2 is root-only, so a
        // consumer's world never contains another pkg's dev table
        resolve_deps(world, src, &dep_dir, &mount_key, &dm, visiting, fetched)?;
        visiting.pop();
    }
    Ok(())
}

/// Mount a `.zip` url dep — the SOURCE transport's lane: gate 0 (the
/// pin) ran at the caller; the bytes are a source zip ([`read_srczip`]
/// — CRC, grammar, zip64 all enforced), which unpacks ONCE into a
/// content-addressed directory (`<project>/.rut/unpacked/<sha-of-bytes>/`,
/// temp-dir + rename atomic, so a reader never sees a partial tree)
/// and mounts as the directory package it carries — the ordinary dir
/// lane from there (its rewritten rows resolve inside; an open url row
/// resolves from the fetched closure, loudly absent otherwise).
fn mount_zip_dep(
    world: &mut World,
    spec: &str,
    url: &str,
    parent: &str,
    sha_hex: &str,
    fetched: &mut Fetched<'_>,
    bytes: &[u8],
) -> Result<(), LoadError> {
    let entries = rut_driver::bundle::read_srczip(bytes)
        .map_err(|e| LoadError::law(format!("{url}: {e}")))?;
    if world.unpack_root.as_os_str().is_empty() {
        return Err(LoadError::law(format!(
            "{url}: no unpack root — the walk entry seeds `<project>/.rut/unpacked`"
        )));
    }
    let dir = world.unpack_root.join(sha_hex);
    unpack_once(&dir, &entries).map_err(|e| LoadError::law(format!("{url}: {e}")))?;
    // the directory lane over the unpacked tree — the manifest, the
    // name-vs-key gate, the entry mount, then the dep recursion (the
    // rewritten rows resolve inside; the first-mount edge records the
    // declaring pkg's spec)
    let dir_key = dir.to_string_lossy().into_owned();
    let src = FsSource::at(&dir);
    let manifest = read_manifest(&src, &dir_key)?;
    let root_name = manifest
        .name
        .clone()
        .ok_or_else(|| LoadError::law(format!("{url}: rut.jsonc has no `name`")))?;
    if root_name != rut_driver::bundle::mount_name(spec) {
        return Err(LoadError::law(format!(
            "dep `{spec}` points at {url} — the zip names itself `{root_name}`"
        )));
    }
    if world.pkgs.contains_key(spec) {
        return Ok(()); // first mount wins — an earlier dep of the same name
    }
    let mut root_module = load_entry_module(&src, &dir_key, &manifest)?;
    root_module.mounted_by = Some(parent.to_string());
    world.register(spec, root_module)?;
    world
        .locs
        .insert(spec.to_string(), Loc::Unpacked(dir.clone()));
    let mut visiting = vec![canonical(&dir_key)?];
    resolve_deps(
        world,
        &src,
        &dir_key,
        spec,
        &manifest,
        &mut visiting,
        fetched,
    )?;
    Ok(())
}

/// Lay a source zip's entries down under `dir` — atomic: a temp
/// sibling fills first, then one rename publishes the tree, so a
/// reader never sees a partial unpack (the cache entry law). The
/// entries are grammar-validated (no `..`, no absolute, no dot
/// components), so every path is a safe join.
fn unpack_once(dir: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
    if dir.is_dir() {
        return Ok(()); // content-addressed: present ⇒ whole
    }
    let parent = dir
        .parent()
        .ok_or_else(|| format!("{}: no parent", dir.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let stem = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "pkg".to_string());
    let tmp = parent.join(format!(".{stem}.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    for (name, bytes) in entries {
        let p = tmp.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    match std::fs::rename(&tmp, dir) {
        Ok(()) => Ok(()),
        // a concurrent unpack won the rename — its tree is whole too
        Err(_) if dir.is_dir() => {
            let _ = std::fs::remove_dir_all(&tmp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(format!("{}: {e}", dir.display()))
        }
    }
}

/// Mount a url dep's fetched bytes — the mount door where the pin is
/// LAW. The fetcher saw only a url; the LOCK's pin for that url and
/// the bytes meet HERE, on EVERY load (fresh fetch, cache hit,
/// vendored map, test fixture) — a check the call site performed would
/// be a check the call site could skip. Gate order mirrors the
/// driver's bundle walk:
///
/// 0. the pin: the LOCK pins the url (a missing row is the loud
///    `Unpinned` install recipe) and `sha256_hex(bytes)` must equal it
///    (lowercase by grammar law) — a mismatch names the dep, the url,
///    and BOTH hashes;
/// 1. the container (every entry CRC-verified) — origin is the url;
/// 2. the layout: manifest + exact `format_version`, every group
///    decode + verified;
/// 3. name-vs-key: the bundle's `name` must equal the `[deps]` key
///    (the path flavor's twin);
/// 4. the scope-ledger NAMESPACING: this archive's rows shift into a
///    fresh range of the combined world (boot passes through) and its
///    compiled programs rebase with the same map BEFORE mounting —
///    two independently packed bundles never argue about a number
///    (each binary's ids resolve through its own archive's rows);
/// 5. the root mounts (first-mount-wins) — compiled for a lib root,
///    the decl-surface host rows for a `type = "host"` root;
/// 6. the group loop = the driver's bundle walk's (first-mount-wins,
///    own-scope vs ledger consistency for compiled groups);
/// 7. the closure check against the COMBINED world — bundles are
///    closed: every declared dep rides in-archive (or mounted
///    earlier). No fetching at bundle load, ever.
fn mount_url_dep(
    world: &mut World,
    spec: &str,
    url: &str,
    parent: &str,
    fetched: &mut Fetched<'_>,
    bytes: &[u8],
) -> Result<(), LoadError> {
    if world.pkgs.contains_key(spec) {
        return Ok(()); // first mount wins — the embedder's (or an earlier) mount
    }
    // gate 0 — the pin law: the LOCK pins the url (the manifest row
    // lost its `sha256`); no row is the loud install recipe, a row
    // that disagrees names both hashes
    let pin = fetched.lock.get(url).ok_or_else(|| LoadError::Unpinned {
        spec: spec.to_string(),
        url: url.to_string(),
    })?;
    let got = rut_driver::sha256_hex(bytes);
    if got != *pin {
        return Err(LoadError::Pin {
            spec: spec.to_string(),
            url: url.to_string(),
            pin: pin.clone(),
            got,
        });
    }
    // gate 1 — the container, by the row's own spelling: a `.zip` url
    // is SOURCE transport — unzipped, unpacked once (content-addressed
    // by the pin's hash), and mounted as the directory package it
    // carries; anything else rides the compiled-bundle lane below
    if url
        .rsplit('.')
        .next()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        return mount_zip_dep(world, spec, url, parent, &got, fetched, bytes);
    }
    mount_bundle_bytes(world, spec, url, parent, bytes)
}

/// Mount a fetched `.rutbundle` — the COMPILED transport's lane, gates
/// 1-7 of the url-dep mount door (the pin, gate 0, ran at the caller;
/// an embedded IR FILE rides this same lane from the dir-lane path
/// resolution, its "origin" the file path the diagnostics spell).
/// Gate order mirrors the driver's bundle walk:
fn mount_bundle_bytes(
    world: &mut World,
    spec: &str,
    url: &str,
    parent: &str,
    bytes: &[u8],
) -> Result<(), LoadError> {
    if world.pkgs.contains_key(spec) {
        return Ok(()); // first mount wins — the embedder's (or an earlier) mount
    }
    // gate 1 — the container
    let bundle = match rut_driver::bundle::Bundle::parse(bytes) {
        Ok(b) => b,
        Err(e) => {
            return Err(LoadError::Bundle {
                origin: url.to_string(),
                message: e.to_string(),
            })
        }
    };
    // gate 2 — the layout: manifest, exact version, every group decoded
    let layout = match rut_driver::bundle::Layout::parse(&bundle) {
        Ok(l) => l,
        Err(e) => {
            return Err(LoadError::Bundle {
                origin: url.to_string(),
                message: e,
            })
        }
    };
    let (manifest, root, scopes, groups) = match layout {
        rut_driver::bundle::Layout::Compiled {
            manifest,
            root,
            scopes,
            groups,
        } => (manifest, root, scopes, groups),
        rut_driver::bundle::Layout::Decl { manifest, .. } => {
            // the decl root: a host pkg — the surface mounts as the
            // pkg's host rows (the group lane); single-package, no
            // ledger, no groups. The LEAF law: no fetch walk, the
            // closure check below is trivially true.
            let entries = bundle.entries().to_vec();
            let root_spec = manifest
                .name
                .clone()
                .ok_or_else(|| LoadError::law(format!("{url}: rut.jsonc has no `name`")))?;
            // gate 3 — name-vs-key: the bundle's `name` must equal the
            // `[deps]` key (an identity spelling on the key side
            // strips to its name half; keys stay bare by grammar)
            let want = rut_driver::bundle::mount_name(spec);
            if root_spec != want {
                return Err(LoadError::law(format!(
                    "dep `{spec}` points at {url} — the bundle names itself `{root_spec}`"
                )));
            }
            let mut pkg = bundle_entry_pkg(&entries, "", &manifest, url)?;
            // the first-mount edge: the declaring pkg's spec pulled this archive
            pkg.mounted_by = Some(parent.to_string());
            world.register(&root_spec, pkg)?;
            let slot = world.archives.len();
            world.locs.insert(
                root_spec,
                Loc::Archive {
                    slot,
                    prefix: String::new(),
                },
            );
            world.archives.push(Archive {
                origin: url.to_string(),
                entries,
            });
            return Ok(());
        }
    };
    // gate 3 — name-vs-key (the identity strip covers a spelled
    // identity; the grammar's keys are bare)
    let root_spec = manifest
        .name
        .clone()
        .ok_or_else(|| LoadError::law(format!("{url}: rut.jsonc has no `name`")))?;
    let want = rut_driver::bundle::mount_name(spec);
    if root_spec != want {
        return Err(LoadError::law(format!(
            "dep `{spec}` points at {url} — the bundle names itself `{root_spec}`"
        )));
    }
    let entries = bundle.entries().to_vec();
    // the archive root's own content identity — the self-row's binding
    // and the root's no-rode-file-set mount law read it
    let root_fp = rut_driver::bundle::fp16_of(&entries, "");
    // the identity rebind map: the group identities this archive
    // carries, by bare name — the owner-name law's inputs
    let group_idents: Vec<String> = groups
        .iter()
        .filter_map(|(p, _)| {
            p.strip_prefix(rut_driver::bundle::DEPS_DIR)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(str::to_string)
        })
        .collect();
    let (rebind, ambiguous) = rut_driver::loader::identity_rebind_map(&group_idents);
    // gate 4 — the ledger namespacing: this archive's rows shift into a
    // fresh range of the COMBINED world (boot passes through) and its
    // compiled programs rebase with the same map before mounting — two
    // archives never argue about a number, each binary's ids resolve
    // through its OWN archive's rows. Each row's spelling resolves to
    // its mount key (the group identity, the bare root, or the bare
    // name).
    let remap = world.namescope_ledger(&scopes)?;
    let scopes: Vec<(ScopeId, String)> = scopes
        .into_iter()
        .map(|(s, spec)| {
            let key = rut_driver::loader::resolve_ledger_row(&entries, &root_spec, &root_fp, &spec);
            (remap(s), key)
        })
        .collect();
    for (scope, row_spec) in &scopes {
        world.record_scope(*scope, row_spec);
    }
    let mut root = rut_core::link::rebase(root, &remap);
    // the identity rebind, root flavor (the root's own module name
    // stays bare: that IS its mount key)
    rut_driver::loader::rebind_identity_owners(&mut root, &rebind, false, &ambiguous, url)
        .map_err(LoadError::law)?;
    // gate 6's group reads + the generic-source marker share one entry
    // list — cloned before the root mounts
    // gate 5 — the root: a compiled module (the packer refuses any
    // other); the anti-doctoring law (v22): a generic-owning unit must
    // carry its template bodies inside the binary
    if rut_driver::pack::has_generic_bodies(&root) && root.templates.is_empty() {
        return Err(LoadError::law(format!(
            "{url}: `{root_spec}` owns an open generic surface but its binary carries no template bodies — corrupt or doctored, re-pack the directory"
        )));
    }
    let mods = rut_driver::loader::rows_mods(&entries, "")
        .map_err(|e| LoadError::law(format!("{url}: {e}")))?;
    world.register(
        &root_spec,
        Mounted {
            body: MountedBody::Compiled(root),
            entry: manifest.entry.clone(),
            mods,
            // the archive's rows ride ITS ROOT — the run chain's
            // close-of-world replays the archive from them (grouping
            // the members, shifting the numbering); group members stay
            // row-free, so the graph's compiled-mount arm sees exactly
            // the pack-time shape
            bundle_scopes: scopes.clone(),
            manifest: manifest.clone(),
            // the first-mount edge: the declaring pkg's spec pulled
            // this archive
            mounted_by: Some(parent.to_string()),
            ..Default::default()
        },
    )?;
    // gate 6 — the group loop, no gate inline. A group mounts under its
    // CONTENT IDENTITY (the reader validated the dirname's shape, its
    // manifest name, and its fingerprint-over-content) — two identities
    // of one name mount side by side, the same identity mounts once.
    let slot = world.archives.len();
    let mut prefixes: BTreeMap<String, String> = BTreeMap::new();
    prefixes.insert(root_spec.clone(), String::new());
    for (prefix, kind) in &groups {
        let dep_toml = read_entry(&entries, &format!("{prefix}/rut.jsonc")).map_err(|e| {
            LoadError::Bundle {
                origin: url.to_string(),
                message: e,
            }
        })?;
        // the group manifest rides the compat lane: a repacked
        // archive's group manifests ride byte-for-byte from any era
        let dm = parse_manifest_compat(&dep_toml)
            .map_err(|e| LoadError::law(format!("{url}: {prefix}/rut.jsonc: {e}")))?;
        let name = dm
            .name
            .clone()
            .ok_or_else(|| LoadError::law(format!("{url}: {prefix}/rut.jsonc has no `name`")))?;
        let key = prefix
            .strip_prefix(rut_driver::bundle::DEPS_DIR)
            .and_then(|rest| rest.strip_prefix('/'))
            .and_then(rut_driver::bundle::parse_identity)
            .map(|(n, f)| rut_driver::bundle::identity(n, f))
            .unwrap_or_else(|| name.clone());
        if world.pkgs.contains_key(&key) {
            continue; // first mount wins — per-identity exact dedup
        }
        let pkg = match kind {
            rut_driver::bundle::GroupKind::Compiled(program) => {
                // rebase first, THEN check own-vs-ledger — both sides
                // shift by the same map, so the law (the row must be
                // the scope the binary itself carries) is unchanged, and
                // the stored binary is the namespaced one
                let mut program = rut_core::link::rebase(program.clone(), &remap);
                // the module rides under its MOUNT KEY and its owner
                // rows rebind to the identities (see the loader's
                // container walk)
                rut_driver::loader::rebind_identity_owners(
                    &mut program,
                    &rebind,
                    true,
                    &ambiguous,
                    url,
                )
                .map_err(LoadError::law)?;
                // the ledger must name the group's identity, and the
                // row must be the scope the binary itself carries
                // (refuse, never guess — a mismatch is a corrupt or
                // doctored bundle)
                let Some(own) = rut_core::link::own_scope(&program) else {
                    return Err(LoadError::law(format!(
                        "{url}: {prefix}/{}: the program carries no scope blocks",
                        name
                    )));
                };
                match scopes.iter().find(|(_, s)| s == &key) {
                    Some(&(row, _)) if row == own => {}
                    Some(&(row, _)) => {
                        return Err(LoadError::law(format!(
                            "{url}: `{key}`'s ledger row says scope {row}, but its binary carries {own}"
                        )));
                    }
                    None => {
                        return Err(LoadError::law(format!(
                            "{url}: the scope ledger does not name `{key}` — the bundle is incomplete"
                        )));
                    }
                }
                if rut_driver::pack::has_generic_bodies(&program) && program.templates.is_empty() {
                    return Err(LoadError::law(format!(
                        "{url}: `{key}` owns an open generic surface but its binary carries no template bodies — corrupt or doctored, re-pack the directory"
                    )));
                }
                Mounted {
                    body: MountedBody::Compiled(program),
                    entry: dm.entry.clone(),
                    mods: rut_driver::loader::rows_mods(&entries, &format!("{prefix}/"))
                        .map_err(|e| LoadError::law(format!("{url}: {e}")))?,
                    manifest: dm.clone(),
                    // the first-mount edge: the archive root pulled its groups
                    mounted_by: Some(root_spec.clone()),
                    ..Default::default()
                }
            }
            rut_driver::bundle::GroupKind::Source => {
                let mut pkg = bundle_entry_pkg(&entries, &format!("{prefix}/"), &dm, url)?;
                // the first-mount edge: the archive root pulled its groups
                pkg.mounted_by = Some(root_spec.clone());
                pkg
            }
        };
        world.register(&key, pkg)?;
        prefixes.insert(key.clone(), format!("{prefix}/"));
    }
    // where this archive's mounted pkgs' files live (the packer's
    // rode-along copies — the world holds the location)
    for (spec, prefix) in &prefixes {
        world.locs.entry(spec.clone()).or_insert(Loc::Archive {
            slot,
            prefix: prefix.clone(),
        });
    }
    // the archive's rows ride ITS ROOT — the close-of-world replays the
    // archive from them (grouping the members, shifting the numbering);
    // group members stay row-free, so the graph's compiled-mount arm
    // sees exactly the pack-time shape
    world.archives.push(Archive {
        origin: url.to_string(),
        entries,
    });
    // gate 7 — the closure check, against the COMBINED world: a
    // bundle's dep rides in-archive; if it is neither here nor mounted
    // earlier, the bundle is incomplete (a url row inside it is inert
    // metadata — the walk never fetches at bundle load). Deps keys stay
    // BARE; groups mount under their identities, so the check matches
    // by name half.
    for dep in manifest.deps.keys() {
        let ridden = world
            .pkgs
            .keys()
            .any(|k| rut_driver::bundle::mount_name(k) == dep.as_str());
        if !ridden {
            return Err(LoadError::law(format!(
                "{url}: the bundle is missing its `{dep}` dependency group"
            )));
        }
    }
    Ok(())
}

/// Build a package's entry [`Mounted`] from bundle entries under
/// `prefix` — the in-archive counterpart of [`load_entry_module`]; the
/// driver's pure container lane owns the law (and the manifest's
/// namespace/consts rows ride there). This shim just keeps the error
/// currency local.
fn bundle_entry_pkg(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
    origin: &str,
) -> Result<Mounted, LoadError> {
    rut_driver::loader::bundle_entry_pkg(entries, prefix, manifest)
        .map_err(|e| LoadError::law(format!("{origin}: {e}")))
}

// ---------------------------------------------------------------------
// the offer lane (no dev pass)
// ---------------------------------------------------------------------

/// Offer one package directory — and, recursively, its `[deps]` — as
/// walked pkgs: the programmatic counterpart of a manifest's dep walk
/// (native hosts, tests, plugin loaders). The root is
/// [`Walk::root`]; offer every pkg to a run with `RutRun::new().pkg(..)`.
///
/// This is an OFFER to someone else's program, not "building the pkg
/// itself": no dev-deps are mounted (pass 2 is root-only).
pub fn dir_pkgs(dir: &Path) -> Result<Walk, RunError> {
    dir_pkgs_fetched(dir, &BTreeMap::new())
}

/// [`dir_pkgs`] over pre-fetched url bytes — the `dir_pkgs_with`
/// lane's sync core. The root's `rut.lock` is read from disk (the
/// same law the load lanes keep); archive locations ride the world —
/// the pack lanes' rode-along copies read through them (an
/// embedder that mounts url deps offer-by-offer owns its archive
/// list; the load/pack lanes keep theirs).
pub(crate) fn dir_pkgs_fetched(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<Walk, RunError> {
    let src = FsSource::at(dir);
    let lock = read_lock(&src, dir.to_string_lossy().as_ref())?;
    dir_pkgs_resolved(dir, map, &lock)
}

/// [`dir_pkgs_fetched`] with the pins injected — the offer lane's
/// resolved core (mirrors [`load_dir_resolved`]; no separate pub name
/// — the offer lane has no install-side caller yet).
fn dir_pkgs_resolved(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
    lock: &BTreeMap<String, String>,
) -> Result<Walk, RunError> {
    let mut world = World::new();
    let name = offer_dir(&mut world, dir, map, lock)?;
    Ok(world.into_loaded(name))
}

/// [`dir_pkgs`] with a url-dep remote — reads the root's lock (it
/// gates the prefetch), collects the url rows, fetches, mounts over
/// the bytes map.
pub async fn dir_pkgs_with(dir: &Path, remote: &dyn DepRemote) -> Result<Walk, RunError> {
    let src = FsSource::at(dir);
    let lock = read_lock(&src, dir.to_string_lossy().as_ref())?;
    let map = prefetch_urls(&src, dir.to_string_lossy().as_ref(), &lock, remote).await?;
    dir_pkgs_resolved(dir, &map, &lock)
}

/// The offer lane's core: walk `dir` + deps into an EXISTING world,
/// first-mount-wins, and answer the root's name.
pub(crate) fn offer_dir(
    world: &mut World,
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
    lock: &BTreeMap<String, String>,
) -> Result<String, LoadError> {
    let src = FsSource::at(dir);
    let dir_key = dir.to_string_lossy().into_owned();
    let manifest = read_manifest(&src, &dir_key)?;
    let name = manifest.name.clone().ok_or_else(|| {
        LoadError::law(format!(
            "{}{MANIFEST_NAME} has no `name`",
            dir_key_dir(&dir_key)
        ))
    })?;
    if world.pkgs.contains_key(&name) {
        return Ok(name); // the embedder's mount outranks the directory
    }
    let pkg = load_entry_module(&src, &dir_key, &manifest)?;
    world.register(&name, pkg)?;
    world.locs.insert(name.clone(), Loc::Dir);
    let mut visiting = vec![canonical(&dir_key)?];
    let mut fetched = Fetched {
        map,
        lock,
        visiting_urls: Default::default(),
    };
    resolve_deps(
        world,
        &src,
        &dir_key,
        &name,
        &manifest,
        &mut visiting,
        &mut fetched,
    )?;
    Ok(name)
}

// ---------------------------------------------------------------------
// bundles from disk
// ---------------------------------------------------------------------

/// Load a packed `.rutbundle` from disk — a **compiled** bundle (a lib
/// root: the root's `.rutc` binary, its pack-time scope ledger, and
/// each dep group as a compiled `.rutc` or a source file set) or a
/// **decl** bundle (a `type = "host"` root: the declaration surface
/// mounts as the pkg's host rows — the same lane a host group rides).
/// One wire number, the manifest's `type` routes. The gate order is
/// the law: container CRC, manifest + exact `format_version` (11 —
/// anything else refuses with the one re-pack recipe), then each
/// kind's payload checks — then, and only then, the mount. Refuse,
/// never guess. The walk itself is the driver's PURE container
/// lane (every read is an in-memory entry) — this lane adds only the
/// file read.
pub fn load_bundle_session(path: &Path) -> Result<Walk, RunError> {
    let bytes = std::fs::read(path).map_err(|e| LoadError::io(path.display(), e))?;
    let bundle = rut_driver::Bundle::parse(&bytes)?;
    bundle.walk()
}

// ---------------------------------------------------------------------
// loose files + the toolchain tree
// ---------------------------------------------------------------------

/// Read one module source file. The language has no include form to
/// expand (use paths are inter-module) — but a module may
/// be AUTHORED as several files: the loader splices `entry.libs` into
/// one source, which is assembly, not expansion.
pub fn load_module_source(path: &Path) -> Result<String, LoadError> {
    std::fs::read_to_string(path).map_err(|e| LoadError::io(path.display(), e))
}

/// The toolchain tree's `rut/` directory — the standard packages' home
/// (`calc`, `futures`, `pouch`, …), found relative to this crate (the
/// same math the old driver-side constructors used).
pub fn tree_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rut")
        .canonicalize()
        .expect("the toolchain tree's rut/ dir")
}

/// One toolchain-tree package as a walked pkg — `rut/<name>`. The
/// hosts' offer for the engine-adjacent surfaces (`calc`, …). A
/// package whose `[deps]` pull MORE pkgs into the walk is refused:
/// offer the whole closure instead (`load_dir` over the same
/// directory, or `dir_pkgs`), never a silently-truncated world.
pub fn tree_pkg(name: &str) -> Result<Mounted, RunError> {
    let loaded = dir_pkgs(&tree_dir().join(name))?;
    match loaded.pkgs.len() {
        1 => Ok(loaded.pkgs.into_iter().next().unwrap()),
        n => Err(RunError::law(format!(
            "rut/{name} walks to {n} pkgs (its `[deps]` pull a closure) — offer the walk's \
             whole yield (`load_dir`/`dir_pkgs`), not a single pkg"
        ))),
    }
}
