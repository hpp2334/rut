//! The bundle reader: parse the bytes once, read entries by name.
//! Parsing CRC-verifies every entry — a corrupt archive never hands
//! out bytes.

use crate::container::{parse_bundle, BundleError};
use crate::pack::read_entry;

/// A parsed `.rutbundle`: its entries in archive order. Loaders pick
/// the entries they know (`rut.toml`, the rut sources); unknown extra
/// entries ride along (forward compatibility).
pub struct Bundle {
    entries: Vec<(String, Vec<u8>)>,
}

impl Bundle {
    /// Parse and CRC-verify a `.rutbundle` from its bytes.
    pub fn parse(data: &[u8]) -> Result<Bundle, BundleError> {
        Ok(Bundle { entries: parse_bundle(data)? })
    }

    /// All entries in archive order — `rut.toml` first for a packed
    /// bundle, then the sources, then dep groups.
    pub fn entries(&self) -> &[(String, Vec<u8>)] {
        &self.entries
    }

    /// One entry's text, UTF-8-checked — a missing entry or a
    /// non-UTF-8 payload is a load error naming the key.
    pub fn read(&self, key: &str) -> Result<String, String> {
        read_entry(&self.entries, key)
    }
}
