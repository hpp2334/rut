//! The walk lane's error currency — ported from the driver's old
//! `loader::error`: structured variants for the classes that carry
//! data, a `Law(String)` tail for the long-tail refusals whose text is
//! built at the refusal site. LAW: every `Display` spells the
//! messages byte-for-byte, so existing assertions keep passing.
//!
//! `From<LoadError> for rut_driver::RunError` flattens into the run
//! chain's one error type (the message IS the diagnostic).

use thiserror::Error;

use rut_driver::bundle::ManifestError;

/// The walk/mount lanes' error: why a module directory or a
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
    /// the long tail: today's refusal text, verbatim
    #[error("{0}")]
    Law(String),
}

impl LoadError {
    /// A long-tail refusal — the message IS the error's text.
    pub fn law(msg: impl Into<String>) -> Self {
        LoadError::Law(msg.into())
    }

    /// A filesystem read failure at `path`.
    pub fn io(path: impl std::fmt::Display, source: std::io::Error) -> Self {
        LoadError::Io {
            path: path.to_string(),
            source,
        }
    }
}

/// The pack lanes' currency: the walk's structured load errors flatten
/// into the pack error's long tail (the message IS the diagnostic).
impl From<LoadError> for rut_driver::pack::PackError {
    fn from(e: LoadError) -> rut_driver::pack::PackError {
        rut_driver::pack::PackError::law(e.to_string())
    }
}

impl From<LoadError> for rut_driver::RunError {
    fn from(e: LoadError) -> rut_driver::RunError {
        rut_driver::RunError::law(e.to_string())
    }
}

/// A remote-policy failure — the [`crate::DepRemote`] trait's error
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
