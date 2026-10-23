//! the cache-first remote lane: root override, the block_on seam,
//! poisoned-entry eviction, the fetched load.

use std::future::Future;
use std::path::PathBuf;
use std::task::{Context, Poll};

use rut_native::DepRemote;

/// The remote for a project: `RUT_CACHE_DIR` overrides the root (the
/// CLI-side override — the walk never reads env), otherwise the
/// project-local cache (`<project>/.rut/cache`). CACHE-ONLY: a hit
/// never touches the network and a miss REFUSES — `rut run`/`rut
/// pack` never download (the install-first law).
pub(crate) fn offline_remote_for(project: &std::path::Path) -> rut_native::HttpRemote {
    match std::env::var_os("RUT_CACHE_DIR") {
        Some(dir) => rut_native::HttpRemote::at(PathBuf::from(dir)),
        None => rut_native::HttpRemote::project_local(project),
    }
}

/// The online twin — the wire armed — for the two lanes whose JOB is
/// downloading: `install` and `fetch`. Same root resolution.
pub(crate) fn online_remote_for(project: &std::path::Path) -> rut_native::HttpRemote {
    match std::env::var_os("RUT_CACHE_DIR") {
        Some(dir) => rut_native::HttpRemote::online(PathBuf::from(dir)),
        None => rut_native::HttpRemote::online_local(project),
    }
}

/// The install-first gate: every url dep of the closure must already
/// sit in the cache — run/pack never download. `Ok(())` when the
/// closure is fully cached (or has no url deps, or carries no
/// manifest — that one is the walk's diagnostic, not ours). A missing
/// set is ONE aggregated refusal naming the fix, not a first-miss
/// mid-walk.
pub(crate) fn ensure_deps_cached(
    project: &std::path::Path,
    cmd: &str,
    remote: &rut_native::HttpRemote,
) -> Result<(), String> {
    let rows = match rut_native::url_dep_rows(project) {
        Ok(rows) => rows,
        Err(_) => return Ok(()), // no manifest / bad manifest — the walk diagnoses
    };
    let missing: Vec<&(String, String)> = rows
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

/// The eviction law: a driver pin error names dep + url; the cache
/// entry for that url is poisoned by definition (the pin is checked at
/// the mount door on every load, so a bad entry reloads bad forever) —
/// delete it, and the next run re-fetches. Idempotent: any other error
/// evicts nothing.
pub(crate) fn evict_poisoned_cache(project: &std::path::Path, err: &str) {
    let Some(rest) = err.split("sha256 pin mismatch for ").nth(1) else {
        return;
    };
    let Some(url) = rest.split(": ").next() else {
        return;
    };
    let remote = offline_remote_for(project);
    let path = remote.path_for(url);
    if remote.evict(url) {
        eprintln!(
            "evicted the poisoned cache entry for {url} ({}) — it re-fetches on the next run",
            path.display()
        );
    }
}

/// The fetched load lane for `run`/`fetch`: a module dir or a
/// `.rutbundle`, with url deps served by the CACHE-ONLY remote — every
/// dep is already cached (run/fetch never download; the gate ran, or
/// the progress loop just primed).
pub(crate) fn load_offline(path: &std::path::Path) -> Result<rut_driver::Walk, String> {
    let remote = offline_remote_for(path);
    block_on(rut_native::load_path_session_with(path, &remote)).map_err(|e| e.to_string())
}

// ── the run cache: cache-first `rut run <dir>` ────────────────────────
//
// The whole-program compile is the deterministic reference (L-ref):
// the same declared closure compiles byte-identically everywhere, so
// the miss path may emit its archive into `.rut/cache/` and every
// later run of the SAME closure can mount the cached bundle instead of
// recompiling. The key is what makes a hit trustworthy: sha256 over
// the closure's sources, manifests, `rut.lock` pins, and the rut
// toolchain version — same key ⇒ same closure ⇒ byte-identical bundle
// ⇒ the key cannot lie.
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

/// The cache entry for a run key: `.rut/cache/<key>.rutbundle` beside
/// the project — the same convention (and `RUT_CACHE_DIR` override) as
/// the url-dep cache.
fn run_cache_entry(dir: &std::path::Path, key: &str) -> PathBuf {
    match std::env::var_os("RUT_CACHE_DIR") {
        Some(root) => PathBuf::from(root).join(format!("{key}.rutbundle")),
        None => dir
            .join(".rut")
            .join("cache")
            .join(format!("{key}.rutbundle")),
    }
}

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
    // the url-dep cache root (the url cache's own convention): a url
    // dep's resolved BYTES key the run — a poisoned or re-fetched
    // cache entry changes the key, so the walk (and its pin door)
    // always sees the truth
    let cache_root = match std::env::var_os("RUT_CACHE_DIR") {
        Some(root) => PathBuf::from(root),
        None => dir.join(".rut").join("cache"),
    };
    let mut seen = std::collections::BTreeSet::new();
    hash_closure(dir, &cache_root, &mut buf, &mut seen, resolved)?;
    Ok(rut_driver::sha256_hex(&buf))
}

/// One length-prefixed field — canonical framing, no ambiguity.
fn field(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// Walk one package directory into the key: manifest, lock, the dir's
/// files, then the path deps' directories, depth-first, cycle-guarded.
/// A url dep whose resolved bytes are not cached yet clears `resolved`
/// — the key is honest (it cannot pretend to know the bytes) but no
/// entry may ride it (see `run_cache_path`).
fn hash_closure(
    dir: &std::path::Path,
    cache_root: &std::path::Path,
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
    // inclusive on purpose (dot entries skipped: `.rut/` is the cache
    // itself, and editor droppings are not closure)
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
            hash_closure(&dep_dir, cache_root, buf, seen, resolved)?;
        }
        if let Some(url) = dep.get("url") {
            field(buf, b"url");
            field(buf, name.as_bytes());
            field(buf, url.as_bytes());
            // the resolved bytes: the cache entry's content keys the
            // run (absent = uncached — the key differs until the fetch
            // lands, and the run itself does the fetching); the
            // entry's extension follows the url's artifact kind, the
            // remote's own naming law
            let entry = cache_root.join(format!(
                "{}{}",
                rut_driver::sha256_hex(url.as_bytes()),
                rut_native::cache_ext(url)
            ));
            match std::fs::read(&entry) {
                Ok(bytes) => field(buf, &bytes),
                Err(_) => {
                    *resolved = false;
                    field(buf, b"<uncached>");
                }
            }
        }
    }
    Ok(())
}

/// Every file under `dir`, recursively; dot entries (`.rut/`, `.git`)
/// are the cache and tooling's own — not the closure.
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
/// are not in the cache yet — a `<uncached>` key is dead by
/// construction (the fetch lands during the run and moves the key), so
/// no entry is ever written under one. Reads tolerate the absence: the
/// hit lane mounts only a fully-keyed entry, and the temp+rename write
/// means a reader never sees a partial one.
pub(crate) fn run_cache_path(dir: &std::path::Path) -> Result<Option<std::path::PathBuf>, String> {
    let mut resolved = true;
    let key = run_cache_key(dir, &mut resolved)?;
    if !resolved {
        return Ok(None);
    }
    Ok(Some(run_cache_entry(dir, &key)))
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
