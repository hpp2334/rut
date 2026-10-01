//! The `.rutbundle` container — a deterministic zip archive
//! carrying a module's `rut.toml` and its rut sources (the packaging
//! reference lives at `docs/src/reference/bundles.md`). The archive
//! mechanics live in [`super::zip`] (the community `zip` crate behind
//! a thin layer); this module is the format's POLICY: which entries
//! may ride, which methods v1 accepts, and where the zip64 refusal
//! sits — the crate reads zip64 transparently, v1 refuses it.
//!
//! **Write** is deterministic: fixed entry order as given, STORE
//! entries, the 1980-01-01 (DOS epoch) timestamp, no extra fields,
//! no zip64 — same input ⇒ byte-identical output, so bundles are
//! content-cacheable. The subset acceptance rules (what a v1 reader
//! owes: the zip essentials, nothing richer) are the other half of
//! the contract.
//!
//! **Read** accepts what we write plus the zip essentials: the central
//! directory is the index, each entry's CRC-32 is verified, and anything
//! outside the subset (compression, zip64, overlapping entries) is a
//! load error naming the problem — a bad bundle never reaches the
//! compiler.

use super::zip::{self, ZipLayerError};

/// A malformed or unsupported bundle — a load error, never a runtime trap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleError(pub String);

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for BundleError {}

/// CRC-32 (IEEE 802.3), the zip entry checksum — table-driven.
pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Pack `(name, bytes)` entries into a `.rutbundle`. Names must be
/// ASCII-ish short paths (`rut.toml`, `plugin.rut`) — v1 has no
/// directories inside the archive.
pub fn write_bundle(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, BundleError> {
    for (name, _) in entries {
        if name.is_empty() || name.len() > 0xFFFF || name.starts_with('/') || name.contains('\\') {
            return Err(BundleError(format!("bad bundle entry name `{name}`")));
        }
    }
    zip::write_archive(entries).map_err(policy)
}

/// Unpack a `.rutbundle`: all entries in archive order, each CRC-verified.
/// Unknown extra entries are returned too — the loader picks what it
/// knows (forward compatibility).
pub fn parse_bundle(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, BundleError> {
    // the zip64 gate is OURS — the sentinel scan over the end-of-
    // central-directory record: the crate reads zip64 archives
    // transparently, v1 refuses them
    let mut eocd = None;
    let min = 22usize;
    if data.len() >= min {
        let start = data.len().saturating_sub(min + 0xFFFF);
        for i in (start..=data.len() - min).rev() {
            if data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
                eocd = Some(i);
                break;
            }
        }
    }
    let Some(eocd) = eocd else {
        return Err(BundleError("not a zip archive (no end-of-central-directory)".into()));
    };
    let n = get16(data, eocd + 10).unwrap_or(0) as usize;
    let cd_size = get32(data, eocd + 12).unwrap_or(0);
    let cd_offset = get32(data, eocd + 16).unwrap_or(0);
    if n == 0xFFFF || cd_size == u32::MAX || cd_offset == u32::MAX {
        return Err(BundleError("zip64 bundles are not supported in v1".into()));
    }
    zip::read_archive(data)
        .map_err(policy)
        .map(|entries| entries.into_iter().map(|e| (e.name, e.bytes)).collect())
}

/// The mechanical layer's failures become the format's diagnostics.
fn policy(e: ZipLayerError) -> BundleError {
    match e {
        ZipLayerError::NotAZip => {
            BundleError("not a zip archive (no end-of-central-directory)".into())
        }
        ZipLayerError::Checksum { name } => BundleError(format!(
            "corrupt bundle: entry `{name}` fails its CRC-32 check"
        )),
        ZipLayerError::UnsupportedMethod { name, method } => BundleError(format!(
            "bundle entry `{name}` uses compression method {method} — v1 bundles are STORE-only"
        )),
        ZipLayerError::Corrupt(what) => BundleError(format!("corrupt bundle: {what}")),
    }
}

fn get16(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn get32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_determinism() {
        let entries = vec![
            ("rut.toml".to_string(), b"name = \"app:x\"\n".to_vec()),
            ("x.rut".to_string(), b"fn main() -> i32 { return 7; }\n".to_vec()),
        ];
        let a = write_bundle(&entries).unwrap();
        let b = write_bundle(&entries).unwrap();
        assert_eq!(a, b, "same input => byte-identical bundle");
        let parsed = parse_bundle(&a).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "rut.toml");
        assert_eq!(parsed[1].1, entries[1].1);
    }

    #[test]
    fn crc_corruption_is_caught() {
        let mut bytes = write_bundle(&[("x.rut".to_string(), b"hello".to_vec())]).unwrap();
        // flip a byte inside the stored payload: the local header is
        // 30 bytes + the entry name, the payload follows
        let at = bytes
            .windows(5)
            .position(|w| w == b"x.rut")
            .expect("the local header's name is present")
            + 5;
        assert_ne!(bytes[at], bytes[at] ^ 0x01);
        bytes[at] ^= 0x01;
        let err = parse_bundle(&bytes).unwrap_err();
        assert!(err.0.contains("CRC"), "{err}");
    }

    #[test]
    fn not_a_zip_and_deflate_refused() {
        assert!(parse_bundle(b"definitely not a zip").is_err());
        // a real deflate entry: method 8 in the central directory must refuse
        let mut bytes = write_bundle(&[("x.rut".to_string(), b"hi".to_vec())]).unwrap();
        // central dir: find the CDH signature and patch its method field
        let cd = bytes
            .windows(4)
            .position(|w| w == [0x50, 0x4b, 0x01, 0x02])
            .expect("cdh present");
        bytes[cd + 10] = 8;
        let err = parse_bundle(&bytes).unwrap_err();
        assert!(err.0.contains("STORE"), "{err}");
    }

    #[test]
    fn zip64_sentinels_are_refused() {
        // a bundle whose EOCD carries the zip64 sentinel fields is a
        // v1 refusal even though the zip crate would read it fine
        let mut bytes = write_bundle(&[("x.rut".to_string(), b"hi".to_vec())]).unwrap();
        let eocd = bytes
            .windows(4)
            .rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])
            .expect("eocd present");
        let cd_offset_at = eocd + 16;
        bytes[cd_offset_at..cd_offset_at + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        let err = parse_bundle(&bytes).unwrap_err();
        assert_eq!(err.0, "zip64 bundles are not supported in v1");
    }

    #[test]
    fn golden_bytes_the_writer_is_stable() {
        // the determinism contract, pinned at the byte level: any
        // upgrade that shifts the writer's output fails here loudly.
        // (The one-time shift off the hand-rolled writer was accepted;
        // stability starts at this hash.)
        let entries = vec![
            (
                "rut.toml".to_string(),
                b"format = \"rutbundle\"\nformat_version = 5\nname = \"golden\"\n".to_vec(),
            ),
            ("golden.rut".to_string(), b"fn main() -> i32 { return 7; }\n".to_vec()),
        ];
        let bytes = write_bundle(&entries).unwrap();
        use sha2::{Digest, Sha256};
        let hex: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hex,
            "7959b18f74ee6c1731edff10f2ddcd3198affb5bf4878896ffad627211e78437",
            "the .rutbundle byte layout moved — is the shift intended? re-pin consciously"
        );
    }
}
