//! The global dep store's CLI lanes: `$RUT_HOME` resolution, the
//! store-backed CACHE-ONLY remote run/pack read through, the
//! verify-then-publish writer `install`/`fetch` share, poisoned-entry
//! eviction, and the run-key cache.
//!
//! The store lives at `$RUT_HOME/deps/<name>@<sha256[:16]>/` (the
//! derivation law is rut-driver's pure core — one helper set, no
//! second table) and holds three things per dep: the pinned bytes as
//! downloaded (`artifact.<ext>`), the verification surface
//! (`meta.json` — full sha256, url, artifact kind; readers never
//! re-hash), and the extracted tree (`unpack/`). Publishing is
//! ATOMIC: everything lands in a tmp sibling, then ONE rename. The
//! project's own directory carries no cache at all — a project is
//! `rut.jsonc` + `rut.lock` + `src/`.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::task::{Context, Poll};

use rut_driver::{parse_store_meta, StoreMeta};
use rut_native::{DepRemote, RemoteError};

// ── the store-backed remote: run/pack's CACHE-ONLY url lane ──────────

/// The remote for a project: url bytes come from the GLOBAL store,
/// addressed by the derivation law (`<spec>@<lock-pin16>` — the
/// manifest row's name + the lock's pin). CACHE-ONLY: a hit never
/// touches the network and a miss REFUSES — `rut run`/`rut pack`
/// never download (the install-first law: `rut install` fills the
/// store).
pub(crate) fn offline_remote_for(project: &std::path::Path) -> StoreRemote {
    StoreRemote::for_project(project)
}

/// The GET door for the two lanes whose JOB is downloading: the wire
/// armed (`HttpRemote::online`'s hard timeouts, redirects, size cap)
/// but NO cache write — the store's publish is verify-then-rename,
/// never the remote's flat cache write. Only [`get_bytes`] is ever
/// called on this handle.
pub(crate) fn get_remote() -> rut_native::HttpRemote {
    rut_native::HttpRemote::online(PathBuf::from(rut_driver::deps_dir()))
}

/// The store's read face over one project's url rows: the rows (spec,
/// url — the walk's collect order, the lock's `pkg` law) plus the root
/// lock's pins (url → sha256). A store address is derived per row;
/// `lookup` answers with the artifact bytes when the dir's meta
/// vouches the pin. A malformed lock travels as [`Self::lock_error`]
/// so the install-first gate can refuse with the walk's own message
/// instead of a misleading "not cached".
pub(crate) struct StoreRemote {
    /// the closure's url rows in collect order — (spec, url)
    pub(crate) rows: Vec<(String, String)>,
    /// the root rut.lock's pins — url → sha256
    pub(crate) pins: BTreeMap<String, String>,
    /// the lock could not be parsed (its path-targeted message) —
    /// None when there is no lock or it parses
    pub(crate) lock_error: Option<String>,
}

impl StoreRemote {
    fn for_project(project: &std::path::Path) -> StoreRemote {
        // the rows: no manifest / bad manifest ⇒ empty — the walk
        // diagnoses with its own message
        let rows = rut_native::url_dep_rows(project).unwrap_or_default();
        // the pins: the ONE lock per walk root
        let lock_path = project.join("rut.lock");
        let (pins, lock_error) = match std::fs::read_to_string(&lock_path) {
            Err(_) => (BTreeMap::new(), None), // no lock ⇒ no pins
            Ok(text) => match rut_driver::parse_lock(&text) {
                Ok(lock) => (
                    lock.deps()
                        .iter()
                        .map(|r| (r.url.clone(), r.sha256.clone()))
                        .collect(),
                    None,
                ),
                Err(e) => (
                    BTreeMap::new(),
                    Some(format!("{}: {e}", lock_path.display())),
                ),
            },
        };
        StoreRemote {
            rows,
            pins,
            lock_error,
        }
    }

    /// The store's bytes for one url row: derive the address (spec +
    /// pin), let the meta vouch the pin (the FULL sha256 — the dir
    /// name carries only its 16-hex prefix), then hand back the
    /// artifact bytes. A reader never re-hashes: install-time
    /// verification is the only hash pass. Any miss along the chain
    /// (no spec, no pin, no dir, no meta, unvouching meta, unreadable
    /// artifact) is a cache MISS, not a failure.
    pub(crate) fn lookup(&self, url: &str) -> Option<Vec<u8>> {
        let spec = self.rows.iter().find(|(_, u)| u == url)?.0.as_str();
        let pin = self.pins.get(url)?;
        let dir = store_dep_dir(spec, pin);
        let meta = store_meta_of(&dir)?;
        if meta.sha256 != *pin {
            return None;
        }
        std::fs::read(dir.join(rut_driver::store_artifact_name(url))).ok()
    }
}

impl DepRemote for StoreRemote {
    fn fetch(
        &self,
        url: &str,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Vec<u8>, RemoteError>> + '_>> {
        // hit ⇒ never touch the network; a miss is the loud offline
        // error (run/pack never download — the gate names the fix
        // first, this is the walk's belt-and-braces)
        match StoreRemote::lookup(self, url) {
            Some(bytes) => Box::pin(std::future::ready(Ok(bytes))),
            None => Box::pin(std::future::ready(Err(RemoteError::new(format!(
                "{url} is not in the dep store and this remote is cache-only — \
                 run `rut install` first (or `rut fetch <dir>` to pre-warm)"
            ))))),
        }
    }

    fn lookup(&self, url: &str) -> Option<Vec<u8>> {
        StoreRemote::lookup(self, url)
    }
}

// ── the store lanes: hit / publish / meta / eviction ─────────────────

/// The store dir for one pinned row: `<deps>/<spec>@<pin16>/`.
fn store_dep_dir(spec: &str, pin: &str) -> PathBuf {
    PathBuf::from(rut_driver::deps_dir()).join(rut_driver::store_dir(spec, pin))
}

/// A dep dir's meta, parsed — `None` on any absence or parse refusal
/// (a missing/corrupt meta is a miss, never a failure).
fn store_meta_of(dir: &std::path::Path) -> Option<StoreMeta> {
    let text = std::fs::read_to_string(dir.join(rut_driver::META_NAME)).ok()?;
    parse_store_meta(&text).ok()
}

/// The store hit: the dir present AND its meta vouches the pin (the
/// full sha256, the same check the dir's 16-hex suffix abbreviates)
/// ⇒ the artifact bytes. `install`/`fetch` warm through here — a hit
/// never touches the wire and never re-hashes.
pub(crate) fn store_hit(spec: &str, url: &str, pin: &str) -> Option<Vec<u8>> {
    let dir = store_dep_dir(spec, pin);
    let meta = store_meta_of(&dir)?;
    if meta.sha256 != pin {
        return None;
    }
    std::fs::read(dir.join(rut_driver::store_artifact_name(url))).ok()
}

/// The extracted tree for an artifact's bytes: a source zip's entries
/// via rut-driver's pure reader (CRC, grammar, zip64 all enforced), a
/// bundle's entries via `Bundle::parse` — laid verbatim under
/// `unpack/`.
fn unpack_entries(url: &str, bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    match rut_native::cache_ext(url) {
        ".zip" => rut_driver::bundle::read_srczip(bytes).map_err(|e| format!("{url}: {e}")),
        _ => {
            let bundle =
                rut_driver::bundle::Bundle::parse(bytes).map_err(|e| format!("{url}: {e}"))?;
            Ok(bundle.entries().to_vec())
        }
    }
}

/// The publish law: download and verify all happened at the caller's
/// (the fetched bytes hash to `pin`); the whole
/// `<spec>@<pin16>/` dir — artifact, meta, extracted tree — lands by
/// ONE atomic rename, so a reader never sees a partial dep. Idempotent:
/// a present dir whose meta vouches the pin ⇒ skip (a concurrent
/// install published the same bytes); a dir under the address that
/// does NOT vouch the pin is corrupt by definition and gets
/// republished over.
pub(crate) fn store_publish(spec: &str, url: &str, pin: &str, bytes: &[u8]) -> Result<(), String> {
    let dir = store_dep_dir(spec, pin);
    match store_meta_of(&dir) {
        Some(meta) if meta.sha256 == pin => return Ok(()),
        _ => {
            // absent, unreadable, corrupt, or vouching a different
            // pin: the rename needs the address clear
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    let entries = unpack_entries(url, bytes)?;
    let deps = PathBuf::from(rut_driver::deps_dir());
    std::fs::create_dir_all(&deps)
        .map_err(|e| format!("{}: cannot create the dep store: {e}", deps.display()))?;
    // the tmp sibling fills first — the whole dir lands by ONE rename
    let addr = rut_driver::store_dir(spec, pin);
    let tmp = deps.join(format!(".{addr}.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let artifact = tmp.join(rut_driver::store_artifact_name(url));
    std::fs::write(&artifact, bytes).map_err(|e| format!("{}: {e}", artifact.display()))?;
    let meta = StoreMeta {
        name: spec.to_string(),
        url: url.to_string(),
        sha256: pin.to_string(),
        format: rut_driver::store_format(url).to_string(),
        format_version: rut_driver::STORE_META_FORMAT_VERSION,
    };
    let meta_path = tmp.join(rut_driver::META_NAME);
    std::fs::write(&meta_path, rut_driver::store_meta_json(&meta))
        .map_err(|e| format!("{}: {e}", meta_path.display()))?;
    let unpack = tmp.join(rut_driver::UNPACK_DIR);
    std::fs::create_dir_all(&unpack).map_err(|e| format!("{}: {e}", unpack.display()))?;
    for (entry_name, entry_bytes) in &entries {
        // archive-relative keys only (no `..`, no absolute) — the
        // driver's bundle-key law; every path a safe join
        let key = rut_driver::bundle::bundle_key(entry_name).map_err(|e| format!("{url}: {e}"))?;
        let p = unpack.join(&key);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&p, entry_bytes).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    match std::fs::rename(&tmp, &dir) {
        Ok(()) => Ok(()),
        // a concurrent publish won the rename — it carried the same
        // pin ⇒ the same verified bytes ⇒ idempotently done
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

/// The eviction law: a driver pin error names dep + url; the store
/// dir whose meta names that url is poisoned by definition (the pin
/// is checked at the mount door on every load, so a bad entry reloads
/// bad forever) — delete it, and the next install re-fetches.
/// Idempotent: any other error evicts nothing.
pub(crate) fn evict_poisoned_cache(err: &str) {
    let Some(rest) = err.split("sha256 pin mismatch for ").nth(1) else {
        return;
    };
    let Some(url) = rest.split(": ").next() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(rut_driver::deps_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        match store_meta_of(&dir) {
            Some(meta) if meta.url == url => {}
            _ => continue,
        }
        if std::fs::remove_dir_all(&dir).is_ok() {
            eprintln!(
                "evicted the poisoned cache entry for {url} ({}) — it re-fetches on the next install",
                dir.display()
            );
        }
    }
}

// ── the install-first gate ───────────────────────────────────────────

/// The install-first gate: every url dep of the closure must already
/// sit in the store — run/pack never download. `Ok(())` when the
/// closure is fully cached (or has no url deps, or carries no
/// manifest — that one is the walk's diagnostic, not ours). A missing
/// set is ONE aggregated refusal naming the fix, not a first-miss
/// mid-walk.
pub(crate) fn ensure_deps_cached(cmd: &str, remote: &StoreRemote) -> Result<(), String> {
    // a malformed lock refuses with the walk's own message — the
    // "not cached" complaint would misname the problem
    if let Some(e) = &remote.lock_error {
        return Err(e.clone());
    }
    let missing: Vec<&(String, String)> = remote
        .rows
        .iter()
        .filter(|(_, url)| remote.lookup(url).is_none())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut msg = format!(
        "{cmd}: {} url dep{} not in the bundle cache — run `rut install` first:",
        missing.len(),
        if missing.len() == 1 { "" } else { "s" },
    );
    for (spec, url) in &missing {
        msg.push_str(&format!("\n  - {spec} — {url}"));
    }
    Err(msg)
}

/// The block_on seam for the `_with` lanes: the fetch futures come
/// back READY (the remote's wire runs on its own worker thread), so a
/// noop-waker poll loop settles them — no runtime dep of our own.
pub(crate) fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// The fetched load lane for `run`/`fetch`: a module dir or a
/// `.rutbundle`, with url deps served by the store-backed CACHE-ONLY
/// remote — every dep is already in the store (run/fetch never
/// download; the gate ran, or the warm loop just primed).
pub(crate) fn load_offline(path: &std::path::Path) -> Result<rut_driver::Walk, String> {
    let remote = offline_remote_for(path);
    block_on(rut_native::load_path_session_with(path, &remote)).map_err(|e| e.to_string())
}

// ── the run cache: cache-first `rut run <dir>` ────────────────────────
//
// The whole-program compile is the deterministic reference (L-ref):
// the same declared closure compiles byte-identically everywhere, so
// the miss path may emit its archive into the run cache and every
// later run of the SAME closure can mount the cached bundle instead
// of recompiling. The key is what makes a hit trustworthy: sha256 over
// the closure's sources, manifests, `rut.lock` pins, and the rut
// toolchain version — same key ⇒ same closure ⇒ byte-identical bundle
// ⇒ the key cannot lie.
//
// Entries land at `$RUT_HOME/run/<key>.rutbundle` (the derivation is
// rut-driver's; `RUT_HOME` overrides, `~/.rut` the fallback) — shared
// across projects, one global cache, no per-project state.
//
// Key inputs, in order (all content, no timestamps):
// - a domain-separation tag + the toolchain version (rut-cli's crate
//   version — the workspace's crates version in lockstep, and the
//   compiler's behavior IS the version's),
// - per package directory, walked depth-first from the root through
//   the manifests' `path` deps (cycle-guarded): the dir label, the
//   manifest bytes, `rut.lock` bytes when present (url deps' content
//   is pinned there — a pin row IS the url bundle's hash), then every
//   file under the directory, sorted by relative path (label + bytes).
//   Over-inclusive by design: any file change ⇒ new key. A key that
//   misses extra files only recompiles; it never serves stale bytes.
//
// `--no-cache` (alias `--repack`) is the debug door: it forces the
// miss path (a fresh whole-program compile) but still refreshes the
// cache entry — one consistent rule: the entry always mirrors what the
// reference engine just produced.
//
// Host-rooted directories (`type = "host"`) never reach the cache:
// they refuse at the run door before anything packs (a host root is a
// declaration leaf — nothing to run).

/// The content key of a run closure, or `Err` when the closure cannot
/// be keyed (no manifest — the walk will refuse with its own message;
/// the run proceeds uncached).
///
/// Framing: every field rides length-prefixed into one buffer, hashed
/// once through the driver's `sha256_hex` (public exactly so a call
/// site keys its cache without a sha2 dependency of its own).
fn run_cache_key(dir: &std::path::Path, resolved: &mut bool) -> Result<String, String> {
    let mut buf: Vec<u8> = Vec::new();
    // the toolchain version: the compiler's behavior is part of the key
    field(&mut buf, b"rut-run-cache-v1");
    field(&mut buf, env!("CARGO_PKG_VERSION").as_bytes());
    // the store's address law travels with the key: a url dep's
    // resolved BYTES key the run — a re-pinned dep (new pin ⇒ new
    // store address ⇒ new bytes) moves the key, so the walk (and its
    // pin door) always sees the truth
    let rows = rut_native::url_dep_rows(dir).unwrap_or_default();
    let specs: BTreeMap<String, String> = rows.into_iter().collect();
    let pins = lock_pins(dir)?;
    let mut seen = std::collections::BTreeSet::new();
    hash_closure(dir, &specs, &pins, &mut buf, &mut seen, resolved)?;
    Ok(rut_driver::sha256_hex(&buf))
}

/// The root lock's pins (url → sha256) — the store addresses' right
/// half. A missing lock is an empty map (a project with no url deps
/// needs no lock); a malformed lock is the loud keyed-refusal (the
/// walk's own message shape), not a silent fallback.
fn lock_pins(dir: &std::path::Path) -> Result<BTreeMap<String, String>, String> {
    let lock_path = dir.join("rut.lock");
    let text = match std::fs::read_to_string(&lock_path) {
        Ok(t) => t,
        Err(_) => return Ok(BTreeMap::new()),
    };
    let lock =
        rut_driver::parse_lock(&text).map_err(|e| format!("{}: {e}", lock_path.display()))?;
    Ok(lock
        .deps()
        .iter()
        .map(|r| (r.url.clone(), r.sha256.clone()))
        .collect())
}

/// One length-prefixed field — canonical framing, no ambiguity.
fn field(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// Walk one package directory into the key: manifest, lock, the dir's
/// files, then the path deps' directories, depth-first, cycle-guarded.
/// A url dep whose resolved bytes are not in the store yet clears
/// `resolved` — the key is honest (it cannot pretend to know the
/// bytes) but no entry may ride it (see `run_cache_path`).
fn hash_closure(
    dir: &std::path::Path,
    specs: &BTreeMap<String, String>,
    pins: &BTreeMap<String, String>,
    buf: &mut Vec<u8>,
    seen: &mut std::collections::BTreeSet<PathBuf>,
    resolved: &mut bool,
) -> Result<(), String> {
    let canonical = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    if !seen.insert(canonical.clone()) {
        return Ok(()); // a dep cycle: the closure already keyed this dir
    }
    let label = canonical.to_string_lossy();
    // the manifest — the closure's grammar and the dep table's source
    let manifest_path = canonical.join("rut.jsonc");
    let manifest_bytes =
        std::fs::read(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    field(buf, b"dir");
    field(buf, label.as_bytes());
    field(buf, b"manifest");
    field(buf, &manifest_bytes);
    // the pin ledger — url deps ride by their sha256 pins
    let lock_path = canonical.join("rut.lock");
    if let Ok(lock_bytes) = std::fs::read(&lock_path) {
        field(buf, b"lock");
        field(buf, &lock_bytes);
    }
    // every file under the directory, sorted by relative path — over-
    // inclusive on purpose (dot entries skipped: editor droppings and
    // the loader's own unpack area are not closure)
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(&canonical, &mut files)?;
    files.sort();
    for file in files {
        let rel = file
            .strip_prefix(&canonical)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        if rel == "rut.jsonc" || rel == "rut.lock" {
            continue; // keyed above, exactly once
        }
        let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        field(buf, b"file");
        field(buf, rel.as_bytes());
        field(buf, &bytes);
    }
    // the path deps: parse the manifest (compat lane — any era's
    // manifest rides) and follow each `path` row, depth-first
    let text = String::from_utf8_lossy(&manifest_bytes).into_owned();
    let manifest = rut_driver::bundle::parse_manifest_compat(&text)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    for (name, dep) in &manifest.deps {
        if let Some(rel) = dep.get("path") {
            let dep_dir = canonical.join(rel);
            field(buf, b"dep");
            field(buf, name.as_bytes());
            hash_closure(&dep_dir, specs, pins, buf, seen, resolved)?;
        }
        if let Some(url) = dep.get("url") {
            field(buf, b"url");
            field(buf, name.as_bytes());
            field(buf, url.as_bytes());
            // the resolved bytes: the store artifact's content keys
            // the run, addressed by the derivation law (the closure's
            // collect-order spec — the lock's `pkg` law — plus the
            // lock's pin). Absent = uncached — the key differs until
            // the install lands, and no entry may ride it.
            let spec = specs.get(url).map(String::as_str).unwrap_or(name);
            let entry = pins
                .get(url)
                .map(|pin| store_dep_dir(spec, pin).join(rut_driver::store_artifact_name(url)));
            match entry.and_then(|p| std::fs::read(&p).ok()) {
                Some(bytes) => field(buf, &bytes),
                None => {
                    *resolved = false;
                    field(buf, b"<uncached>");
                }
            }
        }
    }
    Ok(())
}

/// Every file under `dir`, recursively; dot entries (`.git`, the
/// loader's `.rut` unpack area) are tooling's own — not the closure.
fn collect_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// The cache-first run lane for a directory: the entry path for this
/// closure's key — `Ok(None)` when the closure cannot be cached
/// honestly. Two shapes refuse: no manifest (unkeyable — the walk
/// diagnoses with its own message) and a url dep whose resolved bytes
/// are not in the store yet — a `<uncached>` key is dead by
/// construction (the install lands during the run and moves the key),
/// so no entry is ever written under one. Reads tolerate the absence:
/// the hit lane mounts only a fully-keyed entry, and the temp+rename
/// write means a reader never sees a partial one.
pub(crate) fn run_cache_path(dir: &std::path::Path) -> Result<Option<std::path::PathBuf>, String> {
    let mut resolved = true;
    let key = run_cache_key(dir, &mut resolved)?;
    if !resolved {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(rut_driver::run_cache_path(&key))))
}

/// Write a fresh cache entry (temp file + rename, best-effort).
pub(crate) fn store_run_cache(entry: &std::path::Path, bytes: &[u8]) {
    let Some(parent) = entry.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = parent.join(format!(
        ".{}.tmp",
        entry
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("entry")
    ));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, entry).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rut-cli-cache-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A temp `$RUT_HOME` for the test's lifetime — the store lanes
    /// never touch the real `~/.rut`.
    struct Home(PathBuf);
    impl Home {
        fn new(tag: &str) -> Home {
            let d = scratch(tag);
            std::env::set_var("RUT_HOME", &d);
            Home(d)
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            std::env::remove_var("RUT_HOME");
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(dir: &std::path::Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, text).unwrap();
    }

    fn hello_world(tag: &str) -> PathBuf {
        let root = scratch(tag);
        let dir = root.join("hello");
        write(
            &dir,
            "rut.jsonc",
            r#"{"format": "rutbundle", "format_version": 12, "name": "hello"}"#,
        );
        write(&dir, "src/mod.rut", "entry fn main() -> nil { return; }\n");
        dir
    }

    // ---- the run-key cache round trip ----

    #[test]
    fn run_cache_round_trips_through_the_run_dir() {
        let _home = Home::new("roundtrip");
        let dir = hello_world("roundtrip");

        // the key is deterministic: the same closure keys the same
        // entry, and the path rides $RUT_HOME/run/<key>.rutbundle
        let a = run_cache_path(&dir)
            .unwrap()
            .expect("a dep-free closure keys");
        let b = run_cache_path(&dir).unwrap().expect("deterministic");
        assert_eq!(a, b);
        let run_dir = PathBuf::from(rut_driver::rut_home()).join(rut_driver::RUN_DIR);
        assert!(
            a.starts_with(&run_dir),
            "{} rides $RUT_HOME/run",
            a.display()
        );
        assert_eq!(a.extension().and_then(|e| e.to_str()), Some("rutbundle"));
        assert!(!a.is_file(), "nothing written until a run lands");

        // write → read: the entry round-trips byte-for-byte
        store_run_cache(&a, b"the-archive-bytes");
        assert_eq!(std::fs::read(&a).unwrap(), b"the-archive-bytes");
        // a second write replaces atomically (no partial reads)
        store_run_cache(&a, b"fresh");
        assert_eq!(std::fs::read(&a).unwrap(), b"fresh");
    }

    #[test]
    fn an_uncached_url_dep_keys_honestly() {
        let _home = Home::new("uncached");
        let root = scratch("uncached-proj");
        let dir = root.join("app");
        write(
            &dir,
            "rut.jsonc",
            r#"{"format": "rutbundle", "format_version": 12, "name": "app", "deps": {"util": {"url": "https://x/util.rutbundle"}}}"#,
        );
        write(&dir, "src/mod.rut", "entry fn main() -> nil { return; }\n");

        // no lock ⇒ no pin ⇒ no derivable address ⇒ the key refuses
        // to pretend (`<uncached>`): no entry path at all, and nothing
        // lands under the run dir
        let p = run_cache_path(&dir).unwrap();
        assert_eq!(p, None);
        let run_dir = PathBuf::from(rut_driver::rut_home()).join(rut_driver::RUN_DIR);
        assert!(!run_dir.exists(), "nothing written under an unresolved key");

        // WITH the lock pinning the url, the key resolves through the
        // store address — still a miss (nothing installed), still no
        // path, but for the honest reason
        let lock = rut_driver::LockFile::from_rows(vec![rut_driver::LockRow {
            pkg: "util".into(),
            url: "https://x/util.rutbundle".into(),
            sha256: "4197b14904bd9a5d8b0995d86b7d1b1e50d80e9cb1a1c1e6e2a5e8e9c0d1b2a3".into(),
        }]);
        write(&dir, "rut.lock", &rut_driver::render_lock(&lock));
        assert_eq!(run_cache_path(&dir).unwrap(), None);
    }

    #[test]
    fn a_resolved_store_entry_keys_the_run() {
        let _home = Home::new("resolved");
        let root = scratch("resolved-proj");
        let dir = root.join("app");
        let url = "https://x/util.rutbundle";
        let pin = "4197b14904bd9a5d8b0995d86b7d1b1e50d80e9cb1a1c1e6e2a5e8e9c0d1b2a3";
        write(
            &dir,
            "rut.jsonc",
            &format!(
                r#"{{"format": "rutbundle", "format_version": 12, "name": "app", "deps": {{"util": {{"url": "{url}"}}}}}}"#
            ),
        );
        let lock = rut_driver::LockFile::from_rows(vec![rut_driver::LockRow {
            pkg: "util".into(),
            url: url.into(),
            sha256: pin.into(),
        }]);
        write(&dir, "rut.lock", &rut_driver::render_lock(&lock));
        write(&dir, "src/mod.rut", "entry fn main() -> nil { return; }\n");
        assert_eq!(
            run_cache_path(&dir).unwrap(),
            None,
            "cold store ⇒ unresolved"
        );

        // install's publish: the artifact lands — the key resolves
        let bytes = rut_driver::bundle::write_bundle(&[(
            "mod.rutc".to_string(),
            b"the-compiled-ir".to_vec(),
        )])
        .expect("pack the bundle");
        store_publish("util", url, pin, &bytes).expect("publish");
        let entry = run_cache_path(&dir).unwrap().expect("resolved now");
        let run_dir = PathBuf::from(rut_driver::rut_home()).join(rut_driver::RUN_DIR);
        assert!(
            entry.starts_with(&run_dir),
            "{} rides $RUT_HOME/run",
            entry.display()
        );
        // and the artifact the key rode is the store's own bytes
        let artifact = store_dep_dir("util", pin).join(rut_driver::store_artifact_name(url));
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
    }

    // ---- the store publish / hit laws ----

    #[test]
    fn store_publish_lays_out_the_dir_and_hit_answers() {
        let _home = Home::new("publish");
        let url = "https://x/util.rutbundle";
        let pin = "2f6e6a20c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e";
        let bytes = rut_driver::bundle::write_bundle(&[(
            "mod.rutc".to_string(),
            b"the-compiled-ir".to_vec(),
        )])
        .expect("pack the bundle");
        store_publish("util", url, pin, &bytes).expect("publish");

        // the layout: artifact + meta + the extracted tree
        let dir = store_dep_dir("util", pin);
        let artifact = dir.join(rut_driver::store_artifact_name(url));
        assert_eq!(artifact.file_name().unwrap(), "artifact.rutbundle");
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
        let meta = store_meta_of(&dir).expect("the meta parses");
        assert_eq!(meta.name, "util");
        assert_eq!(meta.url, url);
        assert_eq!(meta.sha256, pin);
        assert_eq!(meta.format, "rutbundle");
        assert_eq!(meta.format_version, rut_driver::STORE_META_FORMAT_VERSION);
        assert!(dir.join(rut_driver::UNPACK_DIR).is_dir());
        // the bundle's entries ride the unpack tree verbatim
        assert_eq!(
            std::fs::read(dir.join(rut_driver::UNPACK_DIR).join("mod.rutc")).unwrap(),
            b"the-compiled-ir"
        );

        // the hit: the meta vouches the pin ⇒ the bytes, no re-hash
        assert_eq!(
            store_hit("util", url, pin).as_deref(),
            Some(bytes.as_slice())
        );
        // a different pin's address is simply a miss
        assert_eq!(
            store_hit("util", url, &pin[..63]),
            None,
            "a different pin is a different address"
        );
        let deps = PathBuf::from(rut_driver::deps_dir());
        let leftovers = || {
            std::fs::read_dir(&deps)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with('.'))
                .collect::<Vec<_>>()
        };
        // garbage bytes publish NOTHING (the unpack reader refuses
        // before any tmp dir survives) — and no tmp siblings survive
        // the rename either
        let bad = store_publish("bad", url, pin, b"definitely not a bundle");
        assert!(bad.is_err(), "{bad:?}");
        assert!(leftovers().is_empty(), "{:?}", leftovers());
    }

    #[test]
    fn store_publish_is_idempotent_and_repairs_corruption() {
        let _home = Home::new("idempotent");
        let url = "https://x/util.rutbundle";
        let pin = "4197b14904bd9a5d8b0995d86b7d1b1e50d80e9cb1a1c1e6e2a5e8e9c0d1b2a3";
        let bytes = rut_driver::bundle::write_bundle(&[(
            "mod.rutc".to_string(),
            b"the-compiled-ir".to_vec(),
        )])
        .expect("pack the bundle");
        store_publish("util", url, pin, &bytes).expect("first publish");
        let artifact = store_dep_dir("util", pin).join(rut_driver::store_artifact_name(url));
        let stamp = std::fs::metadata(&artifact).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        // the second publish of the same pin: the meta vouches ⇒ skip,
        // the artifact untouched
        store_publish("util", url, pin, &bytes).expect("idempotent");
        assert_eq!(
            std::fs::metadata(&artifact).unwrap().modified().unwrap(),
            stamp,
            "a vouched dir is never rewritten"
        );

        // a corrupted ARTIFACT under a vouching meta is not publish's
        // problem (the meta is the verification surface; publish never
        // re-hashes) — the mount door refuses it and EVICTION heals:
        std::fs::write(&artifact, b"garbage").unwrap();
        store_publish("util", url, pin, &bytes).expect("the vouch still skips");
        assert_eq!(std::fs::read(&artifact).unwrap(), b"garbage");
        let err = format!(
            "dep `util` — sha256 pin mismatch for {url}: rut.lock pins {pin}, the fetched bytes hash to deadbeef"
        );
        evict_poisoned_cache(&err);
        assert!(!store_dep_dir("util", pin).exists(), "evicted");
        store_publish("util", url, pin, &bytes).expect("re-publish after eviction");
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes, "healed");

        // a corrupt META, by contrast, vouches nothing — the publish
        // repairs the whole dir in place
        let meta_path = store_dep_dir("util", pin).join(rut_driver::META_NAME);
        std::fs::write(&meta_path, b"not json at all").unwrap();
        store_publish("util", url, pin, &bytes).expect("republish over the corrupt meta");
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
        assert!(
            store_meta_of(&store_dep_dir("util", pin)).is_some(),
            "the meta is whole again"
        );
    }

    #[test]
    fn zip_publish_unpacks_the_tree_verbatim() {
        let _home = Home::new("zip");
        let url = "https://x/ink.zip";
        let pin = "4197b14904bd9a5d8b0995d86b7d1b1e50d80e9cb1a1c1e6e2a5e8e9c0d1b2a3";
        let entries = vec![
            (
                "rut.jsonc".to_string(),
                br#"{"format": "rutbundle", "format_version": 12, "name": "ink"}"#.to_vec(),
            ),
            ("src/mod.rut".to_string(), b"pub fn dot()\n".to_vec()),
            (
                "deps/pouch@2f6e6a20c3d4e5f6/rut.jsonc".to_string(),
                br#"{"name": "pouch"}"#.to_vec(),
            ),
        ];
        let zip = rut_driver::bundle::write_srczip(&entries).expect("pack the zip");
        store_publish("ink", url, pin, &zip).expect("publish");

        let dir = store_dep_dir("ink", pin);
        // the artifact is the zip, named by the url's kind
        assert_eq!(
            std::fs::read(dir.join("artifact.zip")).unwrap(),
            zip,
            "the pinned bytes as downloaded"
        );
        // the meta names the SOURCE transport
        let meta = store_meta_of(&dir).expect("the meta parses");
        assert_eq!(meta.format, "zip");
        // the tree rides unpack/, verbatim, dep groups included
        let unpack = dir.join(rut_driver::UNPACK_DIR);
        assert_eq!(
            std::fs::read_to_string(unpack.join("rut.jsonc")).unwrap(),
            r#"{"format": "rutbundle", "format_version": 12, "name": "ink"}"#
        );
        assert_eq!(
            std::fs::read_to_string(unpack.join("src/mod.rut")).unwrap(),
            "pub fn dot()\n"
        );
        assert!(
            unpack
                .join("deps/pouch@2f6e6a20c3d4e5f6/rut.jsonc")
                .is_file(),
            "the embedded dep group rides"
        );
        // and the hit answers with the zip bytes
        assert_eq!(store_hit("ink", url, pin).as_deref(), Some(zip.as_slice()));
    }

    #[test]
    fn eviction_removes_only_the_poisoned_url() {
        let _home = Home::new("evict");
        let pin = "4197b14904bd9a5d8b0995d86b7d1b1e50d80e9cb1a1c1e6e2a5e8e9c0d1b2a3";
        let other = "2f6e6a20c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e";
        let bundle = |b: &[u8]| {
            rut_driver::bundle::write_bundle(&[("mod.rutc".to_string(), b.to_vec())]).unwrap()
        };
        store_publish("util", "https://x/util.rutbundle", pin, &bundle(b"u")).unwrap();
        store_publish("pouch", "https://x/pouch.rutbundle", other, &bundle(b"p")).unwrap();

        // a non-pin error evicts nothing
        evict_poisoned_cache("some unrelated failure");
        assert!(store_dep_dir("util", pin).is_dir());

        // the pin error's phrase (the mount door's exact law) names the
        // url — its store dir goes, the innocent neighbor stays
        let err = format!(
            "dep `util` — sha256 pin mismatch for https://x/util.rutbundle: rut.lock pins {pin}, the fetched bytes hash to deadbeef"
        );
        evict_poisoned_cache(&err);
        assert!(
            !store_dep_dir("util", pin).exists(),
            "the poisoned dir is gone"
        );
        assert!(store_dep_dir("pouch", other).is_dir(), "the neighbor stays");
    }
}
