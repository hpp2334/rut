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
//! - `template/mod.rut` — the root module, the book's first program
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
const MOD_RUT: &str = include_str!("../template/mod.rut");

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
/// `mod.rut` (the ink hello). The name comes from the target's
/// basename and obeys the manifest law — refused, never sanitized; an
/// existing `rut.jsonc`, `mod.rut`, or `rut.lock` is never clobbered;
/// other pre-existing files are left alone.
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
    for file in ["rut.jsonc", "mod.rut", "rut.lock"] {
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
        ("mod.rut", MOD_RUT.to_string()),
    ] {
        let path = p.join(file);
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("new: cannot write {}: {e}", path.display());
            std::process::exit(2);
        }
    }
    println!("created {dir} — rut.jsonc + rut.lock + mod.rut (the std set as deps, pinned by the lock, the ink hello)");
    println!("install first: rut install {dir} — fetches the 9 std bundles into {dir}/.rut/cache (run and pack never download)");
    println!("then run it: rut run {dir} — after a dep edit, rut install re-pins rut.lock; rut fetch {dir} pre-warms the same set");
}

/// The scaffolded manifest: the template with `__NAME__` substituted —
/// the name is the law-checked basename, so the substitution can only
/// produce what the manifest grammar accepts.
fn manifest_text(name: &str) -> String {
    MANIFEST_TEMPLATE.replace("__NAME__", name)
}
