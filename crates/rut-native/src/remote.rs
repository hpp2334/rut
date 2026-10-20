//! The remote-deps policy: how url bytes arrive and how they are
//! cached. [`DepRemote`] is the trait — all bytes-level, no paths
//! cross it; [`HttpRemote`] is the standard implementation —
//! cache-first (a hit NEVER touches the network). THE DEFAULT IS
//! CACHE-ONLY: a miss errors loudly — the `rut install` / `rut fetch`
//! door (or an embedder's [`DepRemote::write`] priming) fills the
//! cache; run/pack-style loads never download. Networking is the
//! EXPLICIT [`HttpRemote::online`]/[`HttpRemote::online_local`]
//! choice: a miss GETs on a private worker thread and writes the
//! cache back atomically, under hard timeouts (10 s connect, 60 s
//! whole-request) — a stalled host fails loudly, never hangs.
//!
//! Layout law: `<root>/<sha256(url)>.rutbundle` — the filename never
//! carries the url's syntax, only its identity. The `http` feature
//! (default on) arms the wire for the online remotes; without it even
//! they error loudly on a miss.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use crate::error::RemoteError;
use rut_driver::sha256_hex;

/// The fetch cap: a bundle beyond this is refused without buffering
/// it (the content-length is honored when present; the buffered length
/// is checked regardless).
const MAX_FETCH_BYTES: usize = 256 * 1024 * 1024;

/// HOW url-dep bytes arrive — the call site's half of the layer split.
/// Transport, caching, and offline policy are ALL the remote's, at the
/// bytes level: no `PathBuf` crosses the trait, and eviction is
/// concrete (`HttpRemote::path_for`/`evict`). The walk owns only
/// WHAT the bytes are declared to be (the `sha256` pin, verified at
/// the mount door on every load — fresh fetch, cache hit, vendored
/// map, test fixture).
///
/// Dyn-compatible by law: `fetch` returns a hand-rolled
/// `Pin<Box<dyn Future ..>>` (NOT the async-trait crate), so every
/// load lane takes `&dyn DepRemote` and the walk stays non-generic. One
/// alloc per url fetch is nothing at dep granularity.
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

/// The standard remote: cache-first, and CACHE-ONLY by default —
/// `project_local` / `at` never network (a miss is the loud offline
/// error; prime the cache with `rut install`, `rut fetch`, or an
/// embedder's `DepRemote::write`). `online_local` / `online` are the
/// explicit wire-on flavors: a miss GETs, under hard timeouts.
pub struct HttpRemote {
    root: PathBuf,
    /// `true` — a miss may GET; `false` — cache-only.
    wire: bool,
    #[cfg(feature = "http")]
    client: reqwest::Client,
}

impl HttpRemote {
    /// The project-local cache: `<project>/.rut/cache` — hermetic, no
    /// global state. CACHE-ONLY: a miss never networks — run
    /// `rut install` first (the CLI's law: run/pack never download).
    pub fn project_local(project: &std::path::Path) -> Self {
        Self::at(project.join(".rut").join("cache"))
    }

    /// An explicit cache root, CACHE-ONLY: a miss is the loud offline
    /// error, never a GET. The default remote.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        HttpRemote {
            root: root.into(),
            wire: false,
            #[cfg(feature = "http")]
            client: Self::plain_client(),
        }
    }

    /// The project-local cache with the wire armed — the explicit
    /// opt-in for apps whose job is fetching (installers, plugin
    /// loaders). A miss GETs (under hard timeouts) and caches.
    pub fn online_local(project: &std::path::Path) -> Self {
        Self::online(project.join(".rut").join("cache"))
    }

    /// An explicit cache root with the wire armed — a miss GETs on a
    /// private worker thread (redirects on, loud status errors, the
    /// size cap) under hard timeouts and writes the cache back
    /// atomically. The ONLY flavor that networks.
    pub fn online(root: impl Into<PathBuf>) -> Self {
        HttpRemote {
            root: root.into(),
            wire: true,
            #[cfg(feature = "http")]
            client: Self::wired_client(),
        }
    }

    /// The plain HTTP client — a cache-only remote never uses it (the
    /// field exists because one struct serves both flavors).
    #[cfg(feature = "http")]
    fn plain_client() -> reqwest::Client {
        reqwest::Client::builder()
            .build()
            .expect("http client")
    }

    /// The wire client: hard timeouts so a stalled host fails loudly —
    /// 10 s to connect, 60 s for the whole request — never a silent
    /// hang (the run lane's old disease, kept out of the wire lanes).
    #[cfg(feature = "http")]
    fn wired_client() -> reqwest::Client {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("http client")
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
                 run `rut install <dir>` (or build with `http`)"
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
            "{url} is not in the bundle cache and this remote is cache-only — \
             run `rut install` first (or `rut fetch <dir>` to pre-warm; an \
             embedder primes via `DepRemote::write`)"
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
    /// The tmp name is unique per WRITER (pid + call counter) — two
    /// processes storing the same entry concurrently each own their
    /// tmp, so one's rename can never steal the other's (a shared
    /// `<sha>.part` made concurrent primes abort with a spurious
    /// `cannot finalize: No such file or directory`).
    fn write(&self, url: &str, bytes: &[u8]) -> Result<(), RemoteError> {
        let path = self.path_for(url);
        std::fs::create_dir_all(&self.root).map_err(|e| {
            RemoteError::new(format!(
                "{}: cannot create cache dir: {e}",
                self.root.display()
            ))
        })?;
        static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("part.{}-{}", std::process::id(), n));
        std::fs::write(&tmp, bytes)
            .map_err(|e| RemoteError::new(format!("{}: cannot write cache entry: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| RemoteError::new(format!("{}: cannot finalize cache entry: {e}", tmp.display())))?;
        Ok(())
    }
}
