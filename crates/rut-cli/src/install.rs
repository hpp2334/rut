//! the `install` subcommand: resolve the closure's url deps and fill
//! the GLOBAL dep store — store-hit first, else GET → verify → unpack
//! → publish (one atomic rename) — and write `rut.lock`, the pin
//! ledger the walk travels with. Also home of the progress loop
//! `fetch` shares.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rut_driver::{sha256_hex, LockFile, LockRow};

use crate::cache;

/// the `install` arg surface.
#[derive(clap::Args)]
pub(crate) struct InstallArgs {
    /// the module directory whose url deps to resolve and pin
    /// — omitted, the working directory (`.`)
    #[arg(default_value = ".")]
    dir: String,
    /// verify the closure against the existing rut.lock — any drift
    /// refuses (exit 2) and nothing is written
    #[arg(long)]
    locked: bool,
}

/// One pass of the progress loop: the closure's url rows plus the
/// payloads and counters the validation and the summaries report.
pub(crate) struct Progress {
    /// the closure's (spec, url) rows — deduped by url, the FIRST spec
    /// in collect order names it (the lock's `pkg` column, the store
    /// address's left half, the same identity law as the rows)
    rows: Vec<(String, String)>,
    /// url → the bytes (a store hit or a fresh download)
    map: BTreeMap<String, Vec<u8>>,
    /// url → the pins the bytes verify to (the meta's vouch on a hit,
    /// the computed hash on a miss) — the lock install is about to
    /// write, or the law `--locked` verifies against
    pins: BTreeMap<String, String>,
    downloaded: usize,
    hits: usize,
    bytes: usize,
    /// `--locked`'s drift rows (`+` unpinned, `~` pin mismatch) — the
    /// plain lane never drifts (it IS the repair lane)
    drift: Vec<String>,
}

/// The project's law pins: the root `rut.lock` as url → sha256. A
/// missing or malformed lock is an empty map — the plain lane repairs
/// by rewriting (the diff sees no old rows), and `--locked` never
/// reaches here with a bad lock (it parses and refuses first).
fn law_pins(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(dir.join("rut.lock"))
        .ok()
        .and_then(|t| rut_driver::parse_lock(&t).ok())
        .map(|l| {
            l.deps()
                .iter()
                .map(|r| (r.url.clone(), r.sha256.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The progress loop `install` and `fetch` share — the walk owns WHAT
/// (the closure's rows, via `rut_native::url_dep_rows`), the CLI owns
/// the chatter and the store: per-dep lines on stderr (`{cmd}: {spec}
/// — downloading {url}` / `— downloaded (N bytes)` for a miss, one `—
/// cache hit (N bytes)` line when the store answers), sizes in plain
/// bytes (`pack`'s style, no KiB machinery). Fetches are SEQUENTIAL
/// (the existing law) so the line-per-dep output stays sane.
///
/// The protocol per url row: a store dir present under the law pin's
/// address with a meta that vouches it (full sha256 == pin) ⇒ HIT —
/// the artifact bytes ride, no wire, no re-hash (install-time
/// verification is the only hash pass). Else MISS: GET through the
/// remote's bare door, hash the bytes, and — `locked` mode — verify
/// against the law pin (a mismatch is a `~` drift row and publishes
/// NOTHING; a url the lock has no row for is a `+` drift row); a
/// verified fetch publishes into the store (tmp sibling → one atomic
/// rename).
///
/// Note on the rows: in practice they carry only `[deps]` url rows —
/// `[dev-deps]` descriptors are path-only by grammar (a url there is
/// refused at parse; the design text mentioning dev-deps url rows
/// is aspirational; do not fight it).
pub(crate) fn progress_loop(
    cmd: &str,
    dir: &Path,
    remote: &rut_native::HttpRemote,
    locked: bool,
) -> Result<Progress, String> {
    let rows = rut_native::url_dep_rows(dir).map_err(|e| e.to_string())?;
    let law = law_pins(dir);
    let mut out = Progress {
        rows,
        map: BTreeMap::new(),
        pins: BTreeMap::new(),
        downloaded: 0,
        hits: 0,
        bytes: 0,
        drift: Vec::new(),
    };
    for (spec, url) in &out.rows {
        let law_pin = law.get(url);
        // the store hit: the dir present, the meta vouching the pin
        if let Some(pin) = law_pin {
            if let Some(bytes) = cache::store_hit(spec, url, pin) {
                eprintln!("{cmd}: {spec} — cache hit ({} bytes)", bytes.len());
                out.hits += 1;
                out.bytes += bytes.len();
                out.pins.insert(url.clone(), pin.clone());
                out.map.insert(url.clone(), bytes);
                continue;
            }
        }
        // the miss: the GET through the bare door, the hash, the law
        eprintln!("{cmd}: {spec} — downloading {url}");
        let bytes = remote.get_bytes(url).map_err(|e| e.to_string())?;
        eprintln!("{cmd}: {spec} — downloaded ({} bytes)", bytes.len());
        out.downloaded += 1;
        out.bytes += bytes.len();
        let hashed = sha256_hex(&bytes);
        let mut publish = true;
        if locked {
            match law_pin {
                None => out
                    .drift
                    .push(format!("  + {url} (dep `{spec}` has no rut.lock row)")),
                Some(pin) if *pin != hashed => {
                    out.drift.push(format!(
                        "  ~ {url} (dep `{spec}` pins {pin}, the fetched bytes hash to {hashed})"
                    ));
                    publish = false; // drift publishes nothing
                }
                Some(_) => {}
            }
        }
        if publish {
            cache::store_publish(spec, url, &hashed, &bytes)?;
        }
        out.pins.insert(url.clone(), hashed);
        out.map.insert(url.clone(), bytes);
    }
    Ok(out)
}

/// `rut install <dir> [--locked]` — resolve the closure's url deps and
/// pin them: warm the store cache-first (sequential), then either
/// write `<dir>/rut.lock` (plain) or verify the existing one
/// (`--locked`). Not a directory (incl. a `.rutbundle`) ⇒ exit 2:
/// bundles are closed — there is nothing to install.
pub(crate) fn install(args: &InstallArgs) {
    let dir = args.dir.as_str();
    let p = Path::new(dir);
    if !p.is_dir() {
        eprintln!(
            "install: {dir} is not a module directory (a .rutbundle is closed — nothing to install)"
        );
        std::process::exit(2);
    }
    if args.locked {
        install_locked(p);
    } else {
        install_plain(p);
    }
}

/// The plain lane: warm the store with progress, mount-validate the
/// graph against the COMPUTED pins (install never writes a lock over
/// garbage bytes — a 200 HTML error page dies at the same mount doors
/// every load dies at), then atomically write `rut.lock` and report
/// the row diff vs the existing lock. The validation pass requires NO
/// existing lock — the computed pins travel injected (the
/// chicken-and-egg lane). An empty closure still writes an
/// empty-deps lock: the record that the closure resolved to nothing.
fn install_plain(p: &Path) {
    let remote = cache::get_remote();
    let got = match progress_loop("install", p, &remote, false) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("install: {e}");
            std::process::exit(2);
        }
    };
    println!(
        "install: {} url deps resolved — {} downloaded, {} cache hits ({} bytes total)",
        got.rows.len(),
        got.downloaded,
        got.hits,
        got.bytes,
    );
    if let Err(e) = rut_native::load_dir_resolved(p, &got.map, &got.pins) {
        eprintln!("install: {e}");
        cache::evict_poisoned_cache(&e.to_string());
        std::process::exit(2);
    }
    let lock_path = p.join("rut.lock");
    let rows: Vec<LockRow> = got
        .rows
        .iter()
        .map(|(spec, url)| LockRow {
            pkg: spec.clone(),
            url: url.clone(),
            sha256: got.pins[url].clone(),
        })
        .collect();
    // the row diff vs the existing lock (a missing or malformed lock
    // is no old rows — plain install is the repair tool)
    let old: Vec<LockRow> = std::fs::read_to_string(&lock_path)
        .ok()
        .and_then(|t| rut_driver::parse_lock(&t).ok())
        .map(|l| l.deps().to_vec())
        .unwrap_or_default();
    let old_shas: BTreeMap<&str, &str> = old
        .iter()
        .map(|r| (r.url.as_str(), r.sha256.as_str()))
        .collect();
    let new_urls: BTreeSet<&str> = rows.iter().map(|r| r.url.as_str()).collect();
    let added = rows
        .iter()
        .filter(|r| !old_shas.contains_key(r.url.as_str()))
        .count();
    let changed = rows
        .iter()
        .filter(|r| {
            old_shas
                .get(r.url.as_str())
                .is_some_and(|sha| *sha != r.sha256)
        })
        .count();
    let dropped = old
        .iter()
        .filter(|r| !new_urls.contains(r.url.as_str()))
        .count();
    if !old.is_empty() && added + changed + dropped == 0 {
        println!("rut.lock unchanged ({} rows, no drift)", rows.len());
        return;
    }
    let text = rut_driver::render_lock(&LockFile::from_rows(rows));
    // atomic: the lock lands whole or not at all (the store entry law)
    let tmp = lock_path.with_file_name(".rut.lock.tmp");
    if let Err(e) = std::fs::write(&tmp, &text) {
        eprintln!("install: cannot write {}: {e}", tmp.display());
        std::process::exit(2);
    }
    if let Err(e) = std::fs::rename(&tmp, &lock_path) {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("install: cannot write {}: {e}", lock_path.display());
        std::process::exit(2);
    }
    println!(
        "rut.lock written: {} rows ({} added, {} changed, {} dropped)",
        got.rows.len(),
        added,
        changed,
        dropped,
    );
}

/// The `--locked` lane: verify the closure against the existing lock —
/// drift refuses (exit 2), the lock is never written, nothing is ever
/// evicted. The checks: no lock (with a non-empty closure) is the
/// install recipe; a closure url absent from the lock is a `+` row; a
/// fetched hash that disagrees with the lock's pin is a `~` row naming
/// BOTH hashes (catches an unpinned-in-manifest url whose server
/// re-served different bytes, and stale locks after any pin change —
/// and it publishes nothing: only pin-vouched bytes may enter the
/// store); a lock row no closure url names anymore is a `-` row.
/// Pin-vouched bytes DO warm the store (the same side effect the old
/// online remote had) — CI's verify pass primes the offline runs.
fn install_locked(p: &Path) {
    let rows = match rut_native::url_dep_rows(p) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("install: {e}");
            std::process::exit(2);
        }
    };
    let lock_path = p.join("rut.lock");
    let lock_text = match std::fs::read_to_string(&lock_path) {
        Ok(t) => t,
        Err(_) if rows.is_empty() => {
            // nothing to verify passes — a no-dep project needs no lock
            println!("install: 0 url deps resolved — nothing to verify (no rut.lock needed)");
            return;
        }
        Err(_) => {
            eprintln!(
                "install: no rut.lock in {} — run `rut install` first",
                p.display()
            );
            std::process::exit(2);
        }
    };
    let lock = match rut_driver::parse_lock(&lock_text) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("install: {}: {e}", lock_path.display());
            std::process::exit(2);
        }
    };
    let remote = cache::get_remote();
    let got = match progress_loop("install", p, &remote, true) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("install: {e}");
            std::process::exit(2);
        }
    };
    let closure_urls: BTreeSet<&str> = got.rows.iter().map(|(_, u)| u.as_str()).collect();
    let mut drift = got.drift;
    for row in lock.deps() {
        if !closure_urls.contains(row.url.as_str()) {
            drift.push(format!("  - {} (no dep names this url anymore)", row.url));
        }
    }
    if !drift.is_empty() {
        eprintln!("install: --locked found drift, rut.lock untouched:");
        for line in &drift {
            eprintln!("{line}");
        }
        std::process::exit(2);
    }
    println!(
        "install: {} url deps verified against rut.lock — {} downloaded, {} cache hits, 0 drift",
        got.rows.len(),
        got.downloaded,
        got.hits,
    );
}
