//! the cache-first remote lane: root override, the block_on seam,
//! poisoned-entry eviction, the fetched load.

use std::future::Future;
use std::path::PathBuf;
use std::task::{Context, Poll};

/// The remote for a project: `RUT_CACHE_DIR` overrides the root (the
/// CLI-side override — the walk never reads env), otherwise the
/// project-local cache (`<project>/.rut/cache`). Cache-first either
/// way: a hit never touches the network.
pub(crate) fn remote_for(project: &std::path::Path) -> rut_native::HttpRemote {
    match std::env::var_os("RUT_CACHE_DIR") {
        Some(dir) => rut_native::HttpRemote::at(PathBuf::from(dir)),
        None => rut_native::HttpRemote::project_local(project),
    }
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
    let remote = remote_for(project);
    let path = remote.path_for(url);
    if remote.evict(url) {
        eprintln!(
            "evicted the poisoned cache entry for {url} ({}) — it re-fetches on the next run",
            path.display()
        );
    }
}

/// The fetched load lane for `run`/`fetch`: a module dir or a
/// `.rutbundle`, with url deps served by the cache-first remote.
pub(crate) fn load_with_cache(path: &std::path::Path) -> Result<rut_driver::Loaded, String> {
    let remote = remote_for(path);
    block_on(rut_native::load_path_session_with(path, &remote)).map_err(|e| e.to_string())
}
