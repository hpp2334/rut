//! The CLI's [`rut_driver::DepFetch`] — the call-site half of the url
//! dep layer split. HOW bytes arrive is entirely policy here:
//! cache-first (a hit NEVER touches the network), GET with redirects,
//! a size cap, and an atomic tmp+rename write so a half-fetched entry
//! never masquerades as a hit. WHAT the bytes are is none of this
//! file's business: the loader verifies the `sha256` pin at the mount
//! door on every load — and when that law refuses, `main` maps the url
//! back to its cache path and evicts, so a poisoned entry heals on the
//! next run.

use std::future::Future;
use std::path::PathBuf;

use rut_driver::DepFetch;

/// The fetch cap: a bundle beyond this is refused without buffering
/// it (the content-length is honored when present; the buffered length
/// is checked regardless).
const MAX_FETCH_BYTES: usize = 256 * 1024 * 1024;

pub struct CacheFetch {
    root: PathBuf,
    client: reqwest::Client,
}

impl CacheFetch {
    /// The cache root: `$RUT_CACHE_DIR` → `$XDG_CACHE_HOME/rut/bundles`
    /// → `~/.cache/rut/bundles`. Created on construction; no usable
    /// root is a loud error (set `RUT_CACHE_DIR`).
    pub fn new() -> Result<CacheFetch, String> {
        let root = match std::env::var_os("RUT_CACHE_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => match std::env::var_os("XDG_CACHE_HOME") {
                Some(dir) => PathBuf::from(dir).join("rut").join("bundles"),
                None => match std::env::var_os("HOME") {
                    Some(home) => PathBuf::from(home).join(".cache").join("rut").join("bundles"),
                    None => {
                        return Err(
                            "cannot locate a bundle cache dir — set RUT_CACHE_DIR".to_string()
                        )
                    }
                },
            },
        };
        std::fs::create_dir_all(&root)
            .map_err(|e| format!("cannot create cache dir {}: {e}", root.display()))?;
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(CacheFetch { root, client })
    }

    /// The cache path for a url: `<root>/<sha256(url)>.rutbundle` — the
    /// filename never carries the url's syntax, only its identity.
    pub fn cache_path(&self, url: &str) -> PathBuf {
        self.root
            .join(format!("{}.rutbundle", rut_driver::sha256_hex(url.as_bytes())))
    }
}

impl DepFetch for CacheFetch {
    fn dep_fetch(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, String>> {
        // the future owns its clones — it is `Send` in practice; the
        // trait deliberately does not demand it
        let url = url.to_string();
        let client = self.client.clone();
        let path = self.cache_path(&url);
        async move {
            // hit ⇒ never touch the network (offline reloads are the point)
            if let Ok(bytes) = std::fs::read(&path) {
                return Ok(bytes);
            }
            // miss ⇒ GET (redirects on by default), loud status errors
            let resp = client
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("{url}: {e}"))?;
            let resp = resp.error_for_status().map_err(|e| format!("{url}: {e}"))?;
            if let Some(len) = resp.content_length() {
                if len as usize > MAX_FETCH_BYTES {
                    return Err(format!(
                        "{url}: {len} bytes exceeds the fetch cap ({MAX_FETCH_BYTES})"
                    ));
                }
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("{url}: {e}"))?
                .to_vec();
            if bytes.len() > MAX_FETCH_BYTES {
                return Err(format!(
                    "{url}: {} bytes exceeds the fetch cap ({MAX_FETCH_BYTES})",
                    bytes.len()
                ));
            }
            // atomic tmp+rename: a reader never sees a partial entry
            let tmp = path.with_extension("part");
            std::fs::write(&tmp, &bytes)
                .map_err(|e| format!("{}: cannot write cache entry: {e}", tmp.display()))?;
            std::fs::rename(&tmp, &path)
                .map_err(|e| format!("{}: cannot finalize cache entry: {e}", tmp.display()))?;
            Ok(bytes)
        }
    }
}
