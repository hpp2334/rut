//! the `new` subcommand: scaffold a runnable hello-world module dir.

/// The std set the template declares — the nine swappable std libs
/// (the stdlib book's "the swappable set"). Never `core` (it auto-rides
/// every `.compile()`), never the `*_host` surfaces (they ride
/// transitively), never `bench_cross` (dev tooling, not app std).
///
/// LOCKSTEP: the shas are the sha256 of the frozen `dist/std-v11/`
/// artifacts (the v11 freeze, tag live); keep in step with the
/// stdlib book pages and the tag lane — when the tag moves, these
/// rows move with it. The manifest rows are UNPINNED (the `sha256` key is retired
/// from the url descriptor grammar — pins live in rut.lock): the
/// scaffold renders this table into `rut.lock` via
/// `rut_driver::render_lock`, and `tests/new_lane.rs` holds the law by
/// checking the SCAFFOLDED lock's pins against the committed snapshot
/// (the lock is the carrier — no duplicate table here).
const STD: &[(&str, &str)] = &[
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/ink.rutbundle",
        "7e9e7f58582ae0a3cd5b4ae76a16b7051c0c79f9479edc34a3ab8261ee250205",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/pouch.rutbundle",
        "249aba803d8a70f05952041f667b5897f4733257f00d5d7b73c6032f4ba70373",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/flow.rutbundle",
        "48493535461a15bbe7bcdea881e221cce62354ee92651c527533241853a3c730",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/nmapset.rutbundle",
        "8b9b62ef68994115db7a8e75bf410fd4ab3a6c0213733b71df1001b4db83a7b1",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/json.rutbundle",
        "811e6f5b98ebb2816ea21782e42441183a1c5d262850567164540cc3d7801467",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/strbuild.rutbundle",
        "1c152f28ff2d55b9dbd9868c2abe15c7116840d6a4cc8bcd4f2cdf520f848dfd",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/calc.rutbundle",
        "0ded688375f71dc4241096faf4ac07bd5154f765705145908589d911fef918d0",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/futures.rutbundle",
        "a3b44897107b3a04aa605ad94f674dedd254755f0310448feb8d1c5102cfa89b",
    ),
    (
        "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v11/dist/std/http.rutbundle",
        "3494e2554e369119fa460213c00b3384ab6318892d09cc236cdea359c5388677",
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
  // compiled bundle (dist/std, jsDelivr-served at the std-v11 tag).
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
