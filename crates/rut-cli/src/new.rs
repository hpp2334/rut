//! the `new` subcommand: scaffold a runnable hello-world module dir.
//!
//! The scaffold IS the template — three real files under
//! `crates/rut-cli/template/`, embedded at compile time so the binary
//! stays self-contained and the templates stay editable as normal
//! files (no string-literal escaping, no generated rows):
//!
//! - `template/rut.jsonc` — the manifest; `__NAME__` is the target's
//!   basename, substituted twice (the header comment and the `name`
//!   row). The dep rows are spelled out literally — unpinned url rows,
//!   `{ "url": … }` only.
//! - `template/rut.lock` — the pins, written verbatim: byte-identical
//!   to what `rut install` writes for that manifest (canonical — rows
//!   sorted by url, columns aligned; `render_lock` is the normalizer).
//! - `template/src/mod.rut` — the root module, the book's first program
//!   byte-for-byte (docs/src/quick-start/first-program.md).
//!
//! LOCKSTEP: the lock's shas are the sha256 of the frozen
//! `dist/std-<tag>/` artifacts the urls name; when the tag lane moves,
//! the template rows move with it — `tests/new_lane.rs` holds the law
//! by checking the SCAFFOLDED lock's pins against the committed
//! snapshot (the lock is the carrier — no duplicate table anywhere).

/// The scaffolded manifest — `__NAME__` rides twice (header + name row).
const MANIFEST_TEMPLATE: &str = include_str!("../template/rut.jsonc");

/// The scaffolded lock — the template bytes, verbatim.
const LOCK_TEMPLATE: &str = include_str!("../template/rut.lock");

/// The scaffolded root module — the book's hello.
const MOD_RUT: &str = include_str!("../template/src/mod.rut");

/// The `--version` string — the CLI's semver beside the std pin baked
/// into the lock template: `rut 0.2.0 (std-v19)`. The pin is DERIVED
/// from the already-baked template at runtime — no second pin table
/// anywhere (the lock is the carrier). The answer is `&'static str`
/// because clap's `version` door wants a `Str`: the string is a pure
/// function of compile-time-baked inputs (`CARGO_PKG_VERSION` + the
/// `include_str!`d template), computed once into a static — that
/// static IS the lifetime, no leak call.
pub(crate) fn full_version() -> &'static str {
    static FULL_VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FULL_VERSION
        .get_or_init(|| format!("{} ({})", env!("CARGO_PKG_VERSION"), std_pin()))
        .as_str()
}

/// The baked std pin — the numeric MAX of the `@std-vNN` tokens in the
/// lock template (numeric, not lexicographic — `std-v10` beats
/// `std-v9`; defensive against any mixed-tag lock). The template always
/// carries pins (`tests/new_lane.rs` holds the law); the `0` fallback
/// answers `std-v0` only if a template ever drains.
fn std_pin() -> String {
    const TAG: &str = "@std-v";
    let mut rest = LOCK_TEMPLATE;
    let mut max = 0u64;
    while let Some(at) = rest.find(TAG) {
        let tail = &rest[at + TAG.len()..];
        let end = tail
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(tail.len());
        if let Ok(n) = tail[..end].parse::<u64>() {
            max = max.max(n);
        }
        rest = tail;
    }
    format!("std-v{max}")
}

/// the `new` arg surface.
#[derive(clap::Args)]
pub(crate) struct NewArgs {
    /// the module directory to scaffold (the package name is its basename)
    dir: String,
}

/// `rut new <dir>` — scaffold a hello-world module directory: a
/// `rut.jsonc` (the derived name + the whole swappable std set as
/// UNPINNED url deps — the pins live in rut.lock), a `rut.lock` (the
/// std set's pins, exactly what `rut install` would write), and a
/// `src/mod.rut` (the ink hello). The name comes from the target's
/// basename and obeys the manifest law — refused, never sanitized; an
/// existing `rut.jsonc`, `src/mod.rut`, or `rut.lock` is never
/// clobbered; other pre-existing files are left alone.
pub(crate) fn new(args: &NewArgs) {
    let dir = args.dir.as_str();
    let p = std::path::Path::new(dir);
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // the manifest law: [a-zA-Z0-9_]+, non-empty — the same charset as
    // dep keys (`rut_driver::bundle::valid_spec`). Refuse, never sanitize.
    if !rut_driver::bundle::valid_spec(&name) {
        eprintln!("new: `{name}` is not a legal package name — [a-zA-Z0-9_]+, non-empty");
        std::process::exit(2);
    }
    // refuse to clobber: a dir that already carries any file of the
    // trio keeps it — other pre-existing files are fine
    for file in ["rut.jsonc", "src/mod.rut", "rut.lock"] {
        if p.join(file).exists() {
            eprintln!("new: `{dir}` already has a `{file}` — refusing to overwrite");
            std::process::exit(2);
        }
    }
    if let Err(e) = std::fs::create_dir_all(p) {
        eprintln!("new: cannot create {dir}: {e}");
        std::process::exit(2);
    }
    for (file, text) in [
        ("rut.jsonc", manifest_text(&name)),
        ("rut.lock", LOCK_TEMPLATE.to_string()),
        ("src/mod.rut", MOD_RUT.to_string()),
    ] {
        let path = p.join(file);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("new: cannot create {}: {e}", parent.display());
                std::process::exit(2);
            }
        }
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("new: cannot write {}: {e}", path.display());
            std::process::exit(2);
        }
    }
    println!("created {dir} — rut.jsonc + rut.lock + src/mod.rut (the std set as deps, pinned by the lock, the ink hello)");
    println!("install first: rut install {dir} — fetches the 9 std source zips into the global dep store at $RUT_HOME/deps (run and pack never download)");
    println!("then run it: rut run {dir} — after a dep edit, rut install re-pins rut.lock; rut install --locked re-warms the store without writing it");
}

/// The scaffolded manifest: the template with `__NAME__` substituted —
/// the name is the law-checked basename, so the substitution can only
/// produce what the manifest grammar accepts.
fn manifest_text(name: &str) -> String {
    MANIFEST_TEMPLATE.replace("__NAME__", name)
}

#[cfg(test)]
mod tests {
    use super::{full_version, LOCK_TEMPLATE};

    /// `--version` carries the semver beside the baked pin — and the
    /// pin IS the MAX `@std-v` token in the lock template (the same
    /// scan `std_pin` runs, recomputed here against the raw bytes).
    #[test]
    fn version_carries_the_baked_std_pin() {
        let full = full_version();
        let (ver, pin) = full
            .split_once(" (")
            .and_then(|(v, rest)| rest.strip_suffix(')').map(|p| (v, p)))
            .expect("full_version must read `SEMVER (std-vNN)`");
        assert!(is_semver(ver), "{ver:?} is not bare semver");
        let expected = LOCK_TEMPLATE
            .match_indices("@std-v")
            .map(|(at, _)| {
                let tail = &LOCK_TEMPLATE[at + "@std-v".len()..];
                let end = tail
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(tail.len());
                tail[..end].parse::<u64>().unwrap()
            })
            .max()
            .expect("the lock template always carries std pins");
        assert_eq!(pin, format!("std-v{expected}"));
    }

    /// `X.Y.Z` — three dot-separated all-digit parts, nothing else.
    fn is_semver(s: &str) -> bool {
        let mut parts = s.split('.');
        let three = (&mut parts)
            .take(3)
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        three && parts.next().is_none()
    }
}
