//! The zip mechanics layer: pure archive plumbing over the `zip`
//! crate. This module knows nothing about rut or bundles — it fixes
//! the DETERMINISTIC knobs once (STORE entries, the 1980-01-01 DOS
//! epoch timestamp, no extra fields, no zip64 on write, entry order
//! as given) and reads archives back verbatim: central-directory
//! order, CRC verified in-read, every entry riding through — the
//! layer has no opinion on names. The format's policy (which names
//! are legal, which methods v1 accepts, how far back the zip64
//! refusal reaches) lives one layer up, in `container`.

use std::io::{Cursor, Read, Write};

/// One entry read back out of an archive: its name, the compression
/// method exactly as the directory spells it (STORE is 0), its payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ZipEntry {
    pub(crate) name: String,
    pub(crate) method: u16,
    pub(crate) bytes: Vec<u8>,
}

/// A mechanical archive failure — never mentions what the archive is
/// for; the layer above maps these to its own diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ZipLayerError {
    /// no end-of-central-directory signature: not a zip archive
    NotAZip,
    /// an entry's payload fails its CRC-32 check
    Checksum { name: String },
    /// an entry rides a compression method this build cannot decode,
    /// named by its method id (STORE is 0)
    UnsupportedMethod { name: String, method: u16 },
    /// everything else: structure, bounds, offsets
    Corrupt(String),
}

/// Pack `(name, bytes)` entries into a zip archive — deterministically:
/// same entries ⇒ byte-identical bytes.
pub(crate) fn write_archive(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, ZipLayerError> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        for (name, data) in entries {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .last_modified_time(
                    zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
                        .expect("1980-01-01 is the DOS epoch, inside every zip range"),
                );
            w.start_file(name.as_str(), options)
                .map_err(|e| ZipLayerError::Corrupt(e.to_string()))?;
            w.write_all(data)
                .map_err(|e| ZipLayerError::Corrupt(e.to_string()))?;
        }
        w.finish()
            .map_err(|e| ZipLayerError::Corrupt(e.to_string()))?;
    }
    Ok(buf.into_inner())
}

/// Read every entry out of an archive — central-directory order, each
/// payload CRC-verified as its bytes stream through. Unknown entries
/// pass through with their method spelled.
pub(crate) fn read_archive(data: &[u8]) -> Result<Vec<ZipEntry>, ZipLayerError> {
    // the signature check first: a not-a-zip input is its own failure,
    // not a corruption report
    if !has_eocd(data) {
        return Err(ZipLayerError::NotAZip);
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| ZipLayerError::Corrupt(e.to_string()))?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        // the entry's name as the directory indexes it — fetched before
        // the file opens, so a method refusal still names its entry
        let dir_name = archive.name_for_index(i).unwrap_or_default().to_string();
        let opened = archive.by_index(i);
        let mut file = match opened {
            Ok(file) => file,
            // a method this build has no codec for — the id is the
            // directory's own spelling
            Err(zip::result::ZipError::CompressionMethodNotSupported(method)) => {
                return Err(ZipLayerError::UnsupportedMethod { name: dir_name, method });
            }
            Err(e) => return Err(ZipLayerError::Corrupt(e.to_string())),
        };
        let name = std::str::from_utf8(file.name_raw())
            .map_err(|_| ZipLayerError::Corrupt("entry name is not UTF-8".into()))?
            .to_string();
        let method = file.compression().to_u16();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| {
            // the CRC reader flags a checksum miss as InvalidData
            if e.kind() == std::io::ErrorKind::InvalidData {
                ZipLayerError::Checksum { name: name.clone() }
            } else {
                ZipLayerError::Corrupt(e.to_string())
            }
        })?;
        out.push(ZipEntry { name, method, bytes });
    }
    Ok(out)
}

/// Does the tail of the data carry an end-of-central-directory
/// signature? (Scans back over a possible zip comment, ≤ 64 KiB.)
fn has_eocd(data: &[u8]) -> bool {
    const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    if data.len() < 22 {
        return false;
    }
    let start = data.len().saturating_sub(22 + 0xFFFF);
    (start..=data.len() - 22).rev().any(|i| data[i..i + 4] == EOCD)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<(String, Vec<u8>)> {
        vec![
            ("rut.toml".to_string(), b"name = \"x\"\n".to_vec()),
            ("x.rut".to_string(), b"fn main() -> i32 { return 7; }\n".to_vec()),
        ]
    }

    #[test]
    fn round_trip_keeps_order_names_and_payloads() {
        let bytes = write_archive(&entries()).unwrap();
        let parsed = read_archive(&bytes).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "rut.toml");
        assert_eq!(parsed[0].method, 0);
        assert_eq!(parsed[0].bytes, b"name = \"x\"\n");
        assert_eq!(parsed[1].name, "x.rut");
        assert_eq!(parsed[1].bytes, entries()[1].1);
    }

    #[test]
    fn the_knobs_are_deterministic() {
        // same input ⇒ byte-identical archive; STORE + fixed timestamp
        // + no extra fields are all fixed here, once
        let a = write_archive(&entries()).unwrap();
        let b = write_archive(&entries()).unwrap();
        assert_eq!(a, b);
        // unknown names ride through untouched (forward compat is the
        // layer's default, the policy refines it)
        let odd = vec![("pouch/pouch.rut".to_string(), b"src".to_vec())];
        let parsed = read_archive(&write_archive(&odd).unwrap()).unwrap();
        assert_eq!(parsed[0].name, "pouch/pouch.rut");
    }

    #[test]
    fn mechanical_failures_are_their_own_variants() {
        assert_eq!(read_archive(b"definitely not a zip"), Err(ZipLayerError::NotAZip));
        // flip a payload byte → the CRC law of the layer itself
        let mut bytes = write_archive(&[("x.rut".to_string(), b"hello".to_vec())]).unwrap();
        let name = b"x.rut";
        let at = bytes
            .windows(name.len())
            .position(|w| w == name)
            .expect("name present")
            + name.len();
        bytes[at] ^= 0x01;
        assert_eq!(
            read_archive(&bytes),
            Err(ZipLayerError::Checksum { name: "x.rut".to_string() })
        );
    }
}
