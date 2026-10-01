//! The load lane's error currency — real types, not `String`.
//!
//! [`LoadError`] is what the load/mount lanes return: structured
//! variants for the classes that carry data (`Io`, `Manifest`,
//! `Bundle`, `Pin`, `Remote`, `DepName`, `Shape`, `Resolve`), and a
//! `Law(String)` tail for the long-tail refusals whose text is built
//! at the refusal site. The taxonomy grows as variants are needed,
//! never blocks. [`RemoteError`] is the [`super::DepRemote`] trait's
//! currency: a message plus an optional boxed source.
//!
//! LAW: every `Display` here spells today's message byte-for-byte, so
//! existing assertions keep passing with only a `.to_string()` added.
//! All types derive with `thiserror` — one error style crate-wide.

use thiserror::Error;

use crate::bundle::ManifestError;
use crate::session::ResolveError;

/// The load/mount lanes' error: why a module directory or a
/// `.rutbundle` did not load.
#[derive(Debug, Error)]
pub enum LoadError {
    /// a filesystem read failed — the path names the file
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// the manifest grammar refused (a syntax or value law)
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// the container or the layout refused (CRC, version, decode)
    #[error("{origin}: {message}")]
    Bundle { origin: String, message: String },
    /// the sha256 pin mismatched at the mount door
    #[error(
        "dep `{spec}` — sha256 pin mismatch for {url}: the manifest pins {pin}, the fetched bytes hash to {got}"
    )]
    Pin {
        spec: String,
        url: String,
        pin: String,
        got: String,
    },
    /// the remote policy failed (a fetch, a lookup, a write)
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// a dep's manifest name disagrees with its key
    #[error("dep `{spec}` points at `{location}` — the manifest there names it `{actual}`")]
    DepName {
        spec: String,
        location: String,
        actual: String,
    },
    /// the input path is neither of the two loadable shapes
    #[error("{path} is neither a module directory (no `rut.jsonc`) nor a `.rutbundle`")]
    Shape { path: String },
    /// the session refused a mount (a duplicate name, a bad spec)
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// the long tail: today's refusal text, verbatim
    #[error("{0}")]
    Law(String),
}

impl LoadError {
    /// A long-tail refusal — the message IS the error's text.
    pub(crate) fn law(msg: impl Into<String>) -> Self {
        LoadError::Law(msg.into())
    }

    /// A filesystem read failure at `path`.
    pub(crate) fn io(path: impl std::fmt::Display, source: std::io::Error) -> Self {
        LoadError::Io {
            path: path.to_string(),
            source,
        }
    }
}

/// A remote-policy failure — the [`super::DepRemote`] trait's error
/// currency: a message plus an optional boxed source (the transport's
/// own error, when there is one).
#[derive(Debug, Error)]
#[error("{msg}")]
pub struct RemoteError {
    msg: String,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl RemoteError {
    /// A message-only failure.
    pub fn new(msg: impl Into<String>) -> Self {
        RemoteError {
            msg: msg.into(),
            source: None,
        }
    }

    /// A failure carrying the transport's own error as the source.
    pub fn with_source(
        msg: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        RemoteError {
            msg: msg.into(),
            source: Some(Box::new(source)),
        }
    }
}
