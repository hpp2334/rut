//! the `fetch` subcommand: warm the bundle cache, discard the session.

use crate::cache;

/// `rut fetch <dir>` — pre-populate the bundle cache: load the
/// directory's graph with the cache-first fetcher and discard the
/// session. CI warms the cache while the network is up; the later
/// offline `run` hits only the cache. A `.rutbundle` input is refused:
/// bundles are closed — there is nothing to fetch.
pub(crate) fn fetch_cmd(dir: &str) {
    let p = std::path::Path::new(dir);
    if !p.is_dir() {
        eprintln!("fetch: {dir} is not a module directory (a `.rutbundle` is closed — nothing to fetch)");
        std::process::exit(2);
    }
    match cache::load_with_cache(p) {
        Ok(loaded) => println!("fetch: {dir} — all url deps are cached (root `{}`)", loaded.root),
        Err(e) => {
            eprintln!("{e}");
            cache::evict_poisoned_cache(p, &e);
            std::process::exit(2);
        }
    }
}
