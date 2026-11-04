//! the `fetch` subcommand: warm the global dep store, discard the run.

use crate::{cache, install};

/// the `fetch` arg surface.
#[derive(clap::Args)]
pub(crate) struct FetchArgs {
    /// the module directory whose url deps to cache
    dir: String,
}

/// `rut fetch <dir>` — pre-populate the dep store: download the
/// closure's url deps through the shared progress loop (store-hit
/// first, else GET → verify → publish), then load the directory's
/// graph with the store-backed cache-only remote — every fetch a
/// store hit now — and discard the run. CI warms the store while
/// the network is up; the later offline `run` hits only the store. A
/// `.rutbundle` input is refused: bundles are closed — there is
/// nothing to fetch.
pub(crate) fn fetch_cmd(args: &FetchArgs) {
    let dir = args.dir.as_str();
    let p = std::path::Path::new(dir);
    if !p.is_dir() {
        eprintln!(
            "fetch: {dir} is not a module directory (a `.rutbundle` is closed — nothing to fetch)"
        );
        std::process::exit(2);
    }
    // the progress loop: per-dep stderr lines, the misses downloaded
    // and published — so the load below rides store hits only
    let remote = cache::get_remote();
    if let Err(e) = install::progress_loop("fetch", p, &remote, false) {
        eprintln!("fetch: {e}");
        std::process::exit(2);
    }
    match cache::load_offline(p) {
        Ok(loaded) => println!(
            "fetch: {dir} — all url deps are cached (root `{}`)",
            loaded.root
        ),
        Err(e) => {
            eprintln!("{e}");
            cache::evict_poisoned_cache(&e);
            std::process::exit(2);
        }
    }
}
