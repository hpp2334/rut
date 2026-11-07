//! The per-project install lock — `rut install`'s kernel-enforced
//! mutual exclusion, acquired ONCE at CLI entry before any download
//! or rut.lock write. One lockfile per project under
//! `$RUT_HOME/locks/` (the CANONICAL dir's sha256 names it —
//! rut-driver's derivation law, so symlinked spellings share one
//! lock); different projects never wait on each other, readers
//! (`run`/`pack`, the editor) never lock at all, and the global
//! store stays lock-free — content addressing + idempotent publishes
//! make cross-project races safe by construction.
//!
//! The primitive is fd-lock (flock on Unix, LockFileEx on Windows):
//! a kernel-owned ADVISORY lock on a STABLE file — created if
//! missing, never renamed or deleted (locking rut.lock itself would
//! be broken by its own tmp+rename write). A second install WAITS
//! (announced to stderr, bounded by a deadline — never an open-ended
//! block), then proceeds; a crashed or Ctrl-C'd holder releases
//! automatically because the lock rides the fd, so there is no
//! stale-lock cleanup code anywhere. The lockfile's CONTENT is
//! advisory-only — the holder's pid, start time, and command, what a
//! waiter announces and a deadline failure names.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fd_lock::{RwLock, RwLockWriteGuard};

/// The production wait: how long a second install keeps trying before
/// failing loudly — ten minutes, never an open-ended block.
const WAIT: Duration = Duration::from_secs(10 * 60);

/// The try-again cadence while waiting.
const POLL: Duration = Duration::from_millis(250);

/// The lockfile's open recipe: create if missing (a STABLE file — it
/// is never renamed or deleted), read+write (the holder truncates its
/// info in, waiters read it out). Deliberately NOT truncate-on-open:
/// a waiter must still read the holder's info the current holder
/// wrote — truncation is the holder's move, post-acquisition
/// ([`write_holder`]).
#[allow(clippy::suspicious_open_options)] // no truncate by design — see above
fn open_lock_file(path: &Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: cannot open the install lock: {e}", path.display()))
}

/// WouldBlock (held elsewhere — flock conflicts are per
/// open-file-description, so a second open in THIS process conflicts
/// too) and Interrupted (a Unix signal landed mid-syscall) both mean
/// try again; anything else is a real failure.
fn would_retry(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted)
}

/// The held install lock — releasing is Drop's whole job; the kernel
/// owns the rest (process death releases too). fd-lock's guard
/// borrows the lock it guards, and a guard riding the entry scope
/// needs its lock pinned for the process's lifetime — one lock per
/// run, so `Box::leak` (the tiny allocation rides to process exit).
#[derive(Debug)]
pub(crate) struct InstallLock {
    #[allow(dead_code)] // the FIELD is never read — holding it is the point
    guard: RwLockWriteGuard<'static, std::fs::File>,
}

/// Acquire the project's install lock with the production wait
/// (10 minutes, the contention announced on first try).
pub(crate) fn acquire(project_dir: &Path) -> Result<InstallLock, String> {
    acquire_with_deadline(project_dir, WAIT)
}

/// The acquisition itself, deadline-parameterized so tests wait ~100 ms
/// instead of 10 minutes — the ONLY reason the seam exists; there are
/// no env knobs. Canonicalize → derive the lockfile → mkdir -p the
/// locks root → open (create) → TRY-AND-WAIT: `try_write` every 250 ms
/// until the deadline, announcing the holder (pid, start time) on
/// first contention. On success the holder info is truncated into the
/// lockfile (advisory content — the fd is the lock); on deadline the
/// error names the holder the first acquirer wrote.
pub(crate) fn acquire_with_deadline(
    project_dir: &Path,
    timeout: Duration,
) -> Result<InstallLock, String> {
    let canonical = project_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", project_dir.display()))?;
    let path = PathBuf::from(rut_driver::install_lock_path(&canonical.to_string_lossy()));
    let root = path.parent().unwrap_or_else(|| Path::new("/"));
    std::fs::create_dir_all(root).map_err(|e| {
        format!(
            "{}: cannot create the install locks dir: {e}",
            root.display()
        )
    })?;
    let start = Instant::now();
    let mut announced = false;
    let guard = loop {
        // TRY-AND-WAIT: each probe opens fresh and tries; the probe's
        // guard drops at the try, so nothing escapes the iteration and
        // the fd closes with the probe (see `would_retry` for the two
        // retryable errors). Never an open-ended block: the deadline
        // below bounds the whole wait.
        loop {
            let mut probe = RwLock::new(open_lock_file(&path)?);
            match probe.try_write() {
                Ok(_) => break, // acquirable — held by no one right now
                Err(e) if would_retry(&e) => {}
                Err(e) => return Err(format!("{}: cannot lock: {e}", path.display())),
            }
            if start.elapsed() >= timeout {
                return Err(format!(
                    "{}: another install still holds this project after {timeout:?} — {}",
                    path.display(),
                    holder_line(&path),
                ));
            }
            if !announced {
                announced = true;
                eprintln!(
                    "rut: waiting — another install holds this project ({})",
                    holder_summary(&path)
                );
            }
            std::thread::sleep(POLL.min(timeout - start.elapsed()));
        }
        // THE TAKE: one fresh fd, its lock leaked for the process
        // lifetime (the escaping guard needs its lock pinned — fd-lock's
        // guard borrows the lock it guards; one box per run). A
        // nanosecond race that loses it — another holder slipped
        // between the probe and the take — simply waits again.
        let held = Box::leak(Box::new(RwLock::new(open_lock_file(&path)?)));
        match held.try_write() {
            Ok(guard) => break guard,
            Err(e) if would_retry(&e) => continue,
            Err(e) => return Err(format!("{}: cannot lock: {e}", path.display())),
        }
    };
    let info = holder_info();
    if let Err(e) = write_holder(&guard, &info) {
        return Err(format!("{}: cannot record the holder: {e}", path.display()));
    }
    Ok(InstallLock { guard })
}

/// The one line a holder truncates into the lockfile — advisory
/// content, read by waiters: the pid, the start time (`HH:MM`, UTC —
/// std has no local-time story), and the command as invoked.
fn holder_info() -> String {
    let command = std::env::args().collect::<Vec<_>>().join(" ");
    format!(
        "pid={} started={} command={command}\n",
        std::process::id(),
        wall_clock()
    )
}

/// `HH:MM` of now, UTC.
fn wall_clock() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let day = secs % 86_400;
    format!("{:02}:{:02}", day / 3_600, day % 3_600 / 60)
}

/// Truncate + write the holder info through the guard (its Deref is
/// the locked file).
fn write_holder(mut file: &std::fs::File, info: &str) -> std::io::Result<()> {
    file.set_len(0)?;
    file.write_all(info.as_bytes())
}

/// The holder line as written (trimmed), or an honest unknown when the
/// file is empty or unreadable — the deadline failure's naming.
fn holder_line(path: &Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "holder unknown (the lockfile is empty or unreadable)".into())
}

/// The wait message's short form: `pid 4321, started 12:34` parsed out
/// of the holder line, else the line itself.
fn holder_summary(path: &Path) -> String {
    let line = holder_line(path);
    let pid = line.split_whitespace().find(|f| f.starts_with("pid="));
    let started = line.split_whitespace().find(|f| f.starts_with("started="));
    match (pid, started) {
        (Some(p), Some(s)) => format!("{}, {}", p.replace('=', " "), s.replace('=', " ")),
        _ => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp `$RUT_HOME` for the test's lifetime — the lock lanes
    /// never touch the real `~/.rut`. Mutating the process env here is
    /// safe under nextest's one-process-per-test model (the suite's
    /// runner), which is the only runner this repo uses.
    struct Home(PathBuf);

    impl Home {
        fn new(tag: &str) -> Home {
            let d = std::env::temp_dir().join(format!(
                "rut-cli-lock-{tag}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
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

    /// A scratch project dir (canonicalize needs it to exist).
    fn project(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("rut-cli-lock-proj-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The lockfile a project's install lock rides.
    fn lockfile(dir: &Path) -> PathBuf {
        let canonical = dir.canonicalize().unwrap();
        PathBuf::from(rut_driver::install_lock_path(&canonical.to_string_lossy()))
    }

    // a test's wait: short enough to fail fast, long enough for a
    // couple of POLL ticks
    const SOON: Duration = Duration::from_millis(400);

    #[test]
    fn the_lock_lands_with_holder_info() {
        let _home = Home::new("holder-info");
        let dir = project("holder-info");

        let _held = acquire_with_deadline(&dir, SOON).expect("an uncontended lock acquires");
        let path = lockfile(&dir);
        assert!(path.is_file(), "{}: the lockfile lands", path.display());
        assert!(
            path.starts_with(_home.0.join(rut_driver::LOCKS_DIR)),
            "the lockfile lives under $RUT_HOME/locks"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(&format!("pid={}", std::process::id())),
            "{text}"
        );
        assert!(text.contains("started="), "{text}");
        assert!(text.contains("command="), "{text}");
    }

    #[test]
    fn a_second_open_conflicts_until_the_guard_drops() {
        let _home = Home::new("double-fd");
        let dir = project("double-fd");

        let held = acquire_with_deadline(&dir, SOON).expect("acquire");
        let path = lockfile(&dir);
        // the SAME process, a SECOND open of the same lockfile: flock
        // conflicts are per open-file-description, so this open sees
        // the held lock — the kernel owns the exclusion, not us
        #[allow(clippy::suspicious_open_options)] // no truncate: this open reads the holder's info
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut second = RwLock::new(file);
        assert!(
            second.try_write().is_err(),
            "a second open of a held lockfile conflicts"
        );
        // and the guard's Drop releases it — no stale-lock residue
        drop(held);
        assert!(
            second.try_write().is_ok(),
            "the kernel lock releases with the guard"
        );
    }

    #[test]
    fn the_deadline_error_names_the_holder() {
        let _home = Home::new("deadline");
        let dir = project("deadline");

        let _held = acquire_with_deadline(&dir, SOON).expect("the first acquire");
        let err = acquire_with_deadline(&dir, Duration::from_millis(100))
            .expect_err("the second acquire must hit its deadline");
        // the error names the holder the first acquirer wrote — pid,
        // start time, command — so the failure is actionable
        assert!(
            err.contains(&format!("pid={}", std::process::id())),
            "{err}"
        );
        assert!(err.contains("started="), "{err}");
        assert!(err.contains("command="), "{err}");
        assert!(
            err.contains("another install still holds this project"),
            "{err}"
        );
    }

    #[test]
    fn different_projects_never_contend() {
        let _home = Home::new("parallel");
        let a = project("parallel-a");
        let b = project("parallel-b");

        // both held at once — different projects run in parallel (the
        // derivation gives them different lockfiles)
        let _a = acquire_with_deadline(&a, SOON).expect("project A acquires");
        let _b = acquire_with_deadline(&b, SOON).expect("project B acquires beside A");
        assert_ne!(lockfile(&a), lockfile(&b));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_spellings_share_one_lock() {
        let _home = Home::new("symlink");
        let dir = project("symlink");
        let alias = dir.parent().unwrap().join("alias-of-symlink");
        let _ = std::fs::remove_file(&alias);
        std::os::unix::fs::symlink(&dir, &alias).unwrap();

        // the canonicalization law: the alias resolves to the same
        // project, so it CONTENDS on the same lockfile
        let _held = acquire_with_deadline(&dir, SOON).expect("the dir spelling acquires");
        let err = acquire_with_deadline(&alias, Duration::from_millis(100))
            .expect_err("the alias spelling contends");
        assert!(
            err.contains(&format!("pid={}", std::process::id())),
            "the alias waits on the SAME lock: {err}"
        );
    }
}
