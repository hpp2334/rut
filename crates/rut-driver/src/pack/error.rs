//! The pack lane's error currency — real types, not `String`.
//! Same pattern as the loader's [`crate::LoadError`]: structured
//! variants for the classes that carry data, a `Law` tail for the
//! long-tail refusals. Display text is byte-for-byte today's
//! messages.

use thiserror::Error;

use crate::bundle::ManifestError;

/// Why a directory did not pack.
#[derive(Debug, Error)]
pub enum PackError {
    /// a filesystem read failed — the path names the file
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// the manifest grammar refused
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// the load half refused (the pack walks the same mount passes)
    #[error(transparent)]
    Load(#[from] crate::LoadError),
    /// the long tail: today's refusal text, verbatim
    #[error("{0}")]
    Law(String),
}

impl PackError {
    /// A long-tail refusal — the message IS the error's text.
    pub(crate) fn law(msg: impl Into<String>) -> Self {
        PackError::Law(msg.into())
    }

    /// A filesystem read failure at `path`.
    pub(crate) fn io(path: impl std::fmt::Display, source: std::io::Error) -> Self {
        PackError::Io {
            path: path.to_string(),
            source,
        }
    }
}
