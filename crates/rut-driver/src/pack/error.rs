//! The pack lane's error currency — real types, not `String`:
//! structured variants for the classes that carry data, a `Law` tail
//! for the long-tail refusals. Display text is byte-for-byte today's
//! messages. The FS half (rut-native) maps its own errors in.

use thiserror::Error;

use crate::bundle::ManifestError;

/// Why a directory did not pack.
#[derive(Debug, Error)]
pub enum PackError {
    /// a filesystem read failed — the path names the file. The pack
    /// lane's name rides the Display: every PackError text carries
    /// exactly one `pack: ` prefix (the message owns the lane name —
    /// one prefix per message), so this variant's Display is the one
    /// home for every Io construction.
    #[error("pack: cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// the manifest grammar refused. The grammar's own text is
    /// unprefixed (its lane speaks manifest law); surfacing THROUGH a
    /// pack failure, the pack lane's name rides this Display — the
    /// transparent passthrough is the one home for every
    /// ManifestError construction.
    #[error("pack: {0}")]
    Manifest(#[from] ManifestError),
    /// the long tail: today's refusal text, verbatim
    #[error("{0}")]
    Law(String),
}

impl PackError {
    /// A long-tail refusal — the message IS the error's text.
    pub fn law(msg: impl Into<String>) -> Self {
        PackError::Law(msg.into())
    }

    /// A filesystem read failure at `path`.
    pub fn io(path: impl std::fmt::Display, source: std::io::Error) -> Self {
        PackError::Io {
            path: path.to_string(),
            source,
        }
    }
}
