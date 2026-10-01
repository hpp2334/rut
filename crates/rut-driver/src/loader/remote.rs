//! The remote-deps policy: how url bytes arrive and how they are
//! cached. [`DepRemote`] is the trait — all bytes-level, no paths
//! cross it; [`HttpRemote`] is the standard implementation —
//! cache-first (a hit NEVER touches the network), a miss GETs on a
//! private worker thread and writes the cache back atomically.
//!
//! Layout law: `<root>/<sha256(url)>.rutbundle` — the filename never
//! carries the url's syntax, only its identity. The `http` feature
//! (default on) arms the wire; without it a miss errors loudly and
//! `offline()` remains the cache-only lane.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::loader::error::RemoteError;
use crate::loader::sha256_hex;

/// The fetch cap: a bundle beyond this is refused without buffering
/// it (the content-length is honored when present; the buffered length
/// is checked regardless).
const MAX_FETCH_BYTES: usize = 256 * 1024 * 1024;

/// HOW url-dep bytes arrive — the call site's half of the layer split.
/// Transport, caching, and offline policy are ALL the remote's, at the
/// bytes level: no `PathBuf` crosses the trait, and eviction is
/// concrete (`HttpRemote::path_for`/`evict`). The loader owns only
/// WHAT the bytes are declared to be (the `sha256` pin, verified at
/// the mount door on every load — fresh fetch, cache hit, vendored
/// map, test fixture).
///
/// Dyn-compatible by law: `fetch` returns a hand-rolled
/// `Pin<Box<dyn Future ..>>` (NOT the async-trait crate), so every
/// load lane takes `&dyn DepRemote` and the [`super::Loader`] stays
/// non-generic. One alloc per url fetch is nothing at dep granularity.
/// Sync impls (cache hits, test fixtures) are first-class via
/// `std::future::ready`; deliberately NOT `+ Send`: JS-backed futures
/// (a browser fetch bridge) are `!Send`.
pub trait DepRemote {
    /// Bytes for one url row — cache hit, network, memory; the
    /// policy's choice. The ONE required method.
    fn fetch(
        &self,
        url: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, RemoteError>> + '_>>;

    /// Cache hit-check (sync — a hit must not await). Default: none.
    fn lookup(&self, url: &str) -> Option<Vec<u8>> {
        let _ = url;
        None
    }

    /// Cache store. Default: the loud "this remote does not cache"
    /// error — a remote without a cache is legal, and saying so beats
    /// a silent drop.
    fn write(&self, url: &str, bytes: &[u8]) -> Result<(), RemoteError> {
        let _ = bytes;
        Err(RemoteError::new(format!(
            "this remote does not cache — refused to store {url}"
        )))
    }
}

/// The standard remote: cache-first, wire-on-miss. `project_local` /
/// `at` network on a miss (with the `http` feature); `offline` never
/// does — the guaranteed-offline lane (CI gates, wasm-adjacent hosts).
pub struct HttpRemote {
    root: PathBuf,
    /// `true` — a miss may GET; `false` — cache-only.
    wire: bool,
    #[cfg(feature = "http")]
    client: reqwest::Client,
}

impl HttpRemote {
    /// The project-local cache: `<project>/.rut/cache` — hermetic, no
    /// global state. Wire on: a cold start GETs its misses (the
    /// `http` feature) and caches them here; a warm start is pure
    /// cache hits.
    pub fn project_local(project: &Path) -> Self {
        Self::at(project.join(".rut").join("cache"))
    }

    /// An explicit cache root, wire on.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        HttpRemote {
            root: root.into(),
            wire: true,
            #[cfg(feature = "http")]
            client: reqwest::Client::builder()
                .build()
                .expect("http client"),
        }
    }

    /// The cache-only flavor — a miss NEVER touches the network, it
    /// errors loudly. The CI/wasm lane: prime the cache with
    /// [`DepRemote::write`] (the wire stand-in) and load.
    pub fn offline(root: impl Into<PathBuf>) -> Self {
        HttpRemote {
            root: root.into(),
            wire: false,
            #[cfg(feature = "http")]
            client: reqwest::Client::builder()
                .build()
                .expect("http client"),
        }
    }

    /// The cache path for a url: `<root>/<sha256(url)>.rutbundle` —
    /// concrete, so a host (or the CLI's eviction lane) can inspect or
    /// delete one entry.
    pub fn path_for(&self, url: &str) -> PathBuf {
        self.root
            .join(format!("{}.rutbundle", sha256_hex(url.as_bytes())))
    }

    /// Delete a poisoned entry (a pin mismatch poisons by definition —
    /// the pin is checked at the mount door on every load, so a bad
    /// entry reloads bad forever). `true` when an entry was removed.
    pub fn evict(&self, url: &str) -> bool {
        std::fs::remove_file(self.path_for(url)).is_ok()
    }

    /// The miss arm: the wire when this remote has one, the loud
    /// refusal when it does not (an `offline()` remote, or a build
    /// without the `http` feature).
    #[cfg(feature = "http")]
    fn fetch_miss(&self, url: &str) -> Result<Vec<u8>, RemoteError> {
        if self.wire {
            self.get(url)
        } else {
            self.get_offline(url)
        }
    }

    /// The miss arm without the wire feature: loud, naming the fix.
    #[cfg(not(feature = "http"))]
    fn fetch_miss(&self, url: &str) -> Result<Vec<u8>, RemoteError> {
        if self.wire {
            Err(RemoteError::new(format!(
                "{url} is not in the bundle cache and this build does not network — \
                 run `rut fetch <dir>` or build with `http`"
            )))
        } else {
            self.get_offline(url)
        }
    }

    /// The wire half: GET with redirects, loud status errors, the size
    /// cap — the CLI's old transport, verbatim, run to completion on a
    /// private worker thread so `fetch` can hand back a READY future
    /// (any executor works, noop-waker `block_on` included).
    #[cfg(feature = "http")]
    fn get(&self, url: &str) -> Result<Vec<u8>, RemoteError> {
        let url = url.to_string();
        let client = self.client.clone();
        let joined_url = url.clone();
        // one thread, one request: the trade for executor-agnosticism
        // (sequential misses = sequential requests; prefetch is
        // sequential today)
        let handle = std::thread::Builder::new()
            .name("rut-http".into())
            .spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .map_err(|e| RemoteError::new(format!("{url}: {e}")))?;
                rt.block_on(async move {
                    // GET (redirects on by default), loud status errors
                    let resp = client
                        .get(&url)
                        .send()
                        .await
                        .map_err(|e| RemoteError::new(format!("{url}: {e}")))?;
                    let resp = resp
                        .error_for_status()
                        .map_err(|e| RemoteError::new(format!("{url}: {e}")))?;
                    if let Some(len) = resp.content_length() {
                        if len as usize > MAX_FETCH_BYTES {
                            return Err(RemoteError::new(format!(
                                "{url}: {len} bytes exceeds the fetch cap ({MAX_FETCH_BYTES})"
                            )));
                        }
                    }
                    let bytes = resp
                        .bytes()
                        .await
                        .map_err(|e| RemoteError::new(format!("{url}: {e}")))?
                        .to_vec();
                    if bytes.len() > MAX_FETCH_BYTES {
                        return Err(RemoteError::new(format!(
                            "{url}: {} bytes exceeds the fetch cap ({MAX_FETCH_BYTES})",
                            bytes.len()
                        )));
                    }
                    Ok(bytes)
                })
            })
            .map_err(|e| {
                RemoteError::new(format!("{joined_url}: cannot spawn the fetch thread: {e}"))
            })?;
        let bytes = match handle.join() {
            Ok(r) => r,
            Err(_) => Err(RemoteError::new(format!(
                "{joined_url}: the fetch thread panicked"
            ))),
        }?;
        // write the cache back atomically — a half-fetched entry never
        // masquerades as a hit (the same law the CLI's fetcher had)
        DepRemote::write(self, &joined_url, &bytes)?;
        Ok(bytes)
    }

    /// The offline miss: loud — the cache has no such entry and this
    /// remote never networks.
    fn get_offline(&self, url: &str) -> Result<Vec<u8>, RemoteError> {
        Err(RemoteError::new(format!(
            "{url} is not in the bundle cache and this remote is offline \
             (cache-only) — prime the cache (`DepRemote::write`, or `rut fetch <dir>`)"
        )))
    }
}

impl DepRemote for HttpRemote {
    fn fetch(
        &self,
        url: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, RemoteError>> + '_>> {
        // hit ⇒ never touch the network (offline reloads are the point);
        // the sync lookup settles first, so a hit is a READY future
        if let Some(bytes) = DepRemote::lookup(self, url) {
            return Box::pin(std::future::ready(Ok(bytes)));
        }
        let r = self.fetch_miss(url);
        Box::pin(std::future::ready(r))
    }

    /// Read the sha-named entry — `None` on any miss or IO error (a
    /// missing or unreadable cache entry is a miss, not a failure).
    fn lookup(&self, url: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path_for(url)).ok()
    }

    /// Atomic tmp+rename store: a reader never sees a partial entry.
    fn write(&self, url: &str, bytes: &[u8]) -> Result<(), RemoteError> {
        let path = self.path_for(url);
        std::fs::create_dir_all(&self.root).map_err(|e| {
            RemoteError::new(format!(
                "{}: cannot create cache dir: {e}",
                self.root.display()
            ))
        })?;
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, bytes)
            .map_err(|e| RemoteError::new(format!("{}: cannot write cache entry: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| RemoteError::new(format!("{}: cannot finalize cache entry: {e}", tmp.display())))?;
        Ok(())
    }
}
