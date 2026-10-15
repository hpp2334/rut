//! the `new` subcommand: scaffold a runnable hello-world module dir.

/// The std set the template declares — the nine swappable std libs
/// (the stdlib book's "the swappable set"). Never `core` (it auto-rides
/// every `.compile()`), never the `*_host` surfaces (they ride
/// transitively), never `bench_cross` (dev tooling, not app std).
///
/// LOCKSTEP: the shas are the sha256 of the frozen `dist/std-v10/`
/// artifacts (the v10 freeze, tag live); keep in step with the
/// stdlib book pages and the tag lane — when the tag moves, these
/// rows move with it. The manifest rows are UNPINNED (the `sha256` key is retired
/// from the url descriptor grammar — pins live in rut.lock): the
/// scaffold renders this table into `rut.lock` via
/// `rut_driver::render_lock`, and `tests/new_lane.rs` holds the law by
/// checking the SCAFFOLDED lock's pins against the committed snapshot
/// (the lock is the carrier — no duplicate table here).
const STD: &[(&str, &str)] = &[
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/ink.rutbundle",
        "4197b149b08f1963aa42374aa988be162411dd244f07193d885f337c8a761ac6",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/pouch.rutbundle",
        "1bbd861905b6cc09435833d18122f0740be14b7b4bc306821634fb88d38e6f83",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/flow.rutbundle",
        "2a45f7279797446bc7bb8e1ed31f9ac0de275e1d615f50f5439ab87a5e1c4f8a",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/nmapset.rutbundle",
        "62b92c34beeda942ffb5cbf8ca84146c56bb33131ac3c0c0a0f8f9ebd643aa02",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/json.rutbundle",
        "3617601203bdd8d71eb34663ee7cc9790065b2ad49ff2e7ef5a77369b44ed039",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/strbuild.rutbundle",
        "c27400b68f9bb6ce4634f3b59fdcbdb76b974c0d86abfe57cf1aadc8b6c9bb6e",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/calc.rutbundle",
        "61974c5c60e8c85fe9a155b46cf965dfe2ba0dfcd0451d0d46d55145cabd5407",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/futures.rutbundle",
        "b22614a88d62f711ec061cebd573b452f93c6ef85b96fbed8531cf012aae8353",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v10/dist/std/http.rutbundle",
        "c165618cb6b1cd44efe00dacfeea3e53d9420f7037936c7018d66cb777f9dc5a",
    ),
];

/// The root module — the book's first program, byte-for-byte
/// (docs/src/quick-start/first-program.md).
const MOD_RUT: &str = r#"use ink::{ Logger };

entry fn main() {
    let log = Logger("hello");
    let name = "rut";
    log.info(f"hello, {name}!");
}
"#;

/// the `new` arg surface.
#[derive(clap::Args)]
pub(crate) struct NewArgs {
    /// the module directory to scaffold (the package name is its basename)
    dir: String,
}

/// `rut new <dir>` — scaffold a hello-world module directory: a
/// `rut.jsonc` (the derived name + the whole swappable std set as
/// UNPINNED url deps — the pins live in rut.lock), a `rut.lock` (the
/// std set's pins, rendered through the lock grammar), and a `mod.rut`
/// (the ink hello). The name comes from the target's basename and
/// obeys the manifest law — refused, never sanitized; an existing
/// `rut.jsonc`, `mod.rut`, or `rut.lock` is never clobbered; other
/// pre-existing files are left alone.
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
        ("rut.lock", lock_text()),
        ("mod.rut", MOD_RUT.to_string()),
    ] {
        let path = p.join(file);
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("new: cannot write {}: {e}", path.display());
            std::process::exit(2);
        }
    }
    println!("created {dir} — rut.jsonc + rut.lock + mod.rut (the std set as deps, pinned by the lock, the ink hello)");
    println!("run it: rut run {dir}");
    println!("first run fetches the std set (9 bundles) into {dir}/.rut/cache — rut fetch {dir} pre-warms; after a dep edit, rut install {dir} re-pins rut.lock");
}

/// The scaffolded manifest: the derived name, the std set as UNPINNED
/// url rows (the pin lives in the sibling `rut.lock` now — `rut
/// install` is its writer), the comment voice of the examples'
/// manifests (the CDN row callout).
fn manifest_text(name: &str) -> String {
    let mut rows = String::new();
    let last = STD.len() - 1;
    for (i, (url, _sha)) in STD.iter().enumerate() {
        rows.push_str(&format!(
            "    \"{}\": {{ \"url\": \"{url}\" }}{}\n",
            pkg_of(url),
            if i == last { "" } else { "," },
        ));
    }
    format!(
        r#"{{
  // {name} — scaffolded by `rut new`: a runnable hello-world module
  // dir — the manifest carries the deps, and `mod.rut` is the root
  // module the loader mounts.
  //
  // THE STD SET: all nine swappable std libs, each a COMMITTED
  // compiled bundle (dist/std, jsDelivr-served at the std-v10 tag).
  // The pins live in `rut.lock` beside this file (machine-generated:
  // the scaffold wrote them; `rut install` re-pins after a dep edit —
  // the sha256 key is retired from manifest rows). Unused deps are
  // legal: host pkgs merge inert unless the program mounts their pkg,
  // and every source pkg compiles only as part of the closure — this
  // hello (ink only) runs clean with the full set declared.
  "name": "{name}",
  "deps": {{
{rows}  }}
}}
"#
    )
}

/// The scaffolded `rut.lock`: this table's pins rendered through the
/// lock grammar — rows sorted by url, byte-deterministic, exactly what
/// `rut install` would write for the manifest above.
fn lock_text() -> String {
    let lock = rut_driver::LockFile::from_rows(
        STD.iter()
            .map(|(url, sha)| rut_driver::LockRow {
                pkg: pkg_of(url).to_string(),
                url: url.to_string(),
                sha256: sha.to_string(),
            })
            .collect(),
    );
    rut_driver::render_lock(&lock)
}

/// The dep name for a bundle url — the file's stem (the lock's `pkg`
/// is display-only; the url is the identity).
fn pkg_of(url: &str) -> &str {
    url.rsplit('/').next().unwrap().trim_end_matches(".rutbundle")
}
