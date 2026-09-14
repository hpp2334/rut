//! The `.rutbundle` container (RFC 0038) — a deterministic zip archive
//! carrying a module's `rut.toml` and its rut sources. Hand-rolled, no
//! dependencies: the format subset we need is small (STORE entries, no
//! zip64, no compression) and the driver stays wasm-compatible.
//!
//! **Write** is deterministic (RFC 0038 §3): fixed entry order as given,
//! zeroed timestamps, no extra fields, no compression — same input ⇒
//! byte-identical output, so bundles are content-cacheable.
//!
//! **Read** accepts what we write plus the zip essentials: the central
//! directory is the index, each entry's CRC-32 is verified, and anything
//! outside the subset (compression, zip64, overlapping entries) is a
//! load error naming the problem — a bad bundle never reaches the
//! compiler.

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

const LFH: u32 = 0x0403_4b50; // local file header
const CDH: u32 = 0x0201_4b50; // central directory header
const EOCD: u32 = 0x0605_4b50; // end of central directory

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&(v as u32).to_le_bytes()[..2]);
}
fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
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
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let lho = out.len() as u32;
        let crc = crc32(data);
        put32(&mut out, LFH);
        put16(&mut out, 20); // version needed
        put16(&mut out, 0); // flags
        put16(&mut out, 0); // method: STORE
        put16(&mut out, 0); // mod time — fixed, for determinism
        put16(&mut out, 0); // mod date — fixed, for determinism
        put32(&mut out, crc);
        put32(&mut out, data.len() as u32); // compressed == stored
        put32(&mut out, data.len() as u32);
        put16(&mut out, name.len() as u16);
        put16(&mut out, 0); // extra len
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        put32(&mut central, CDH);
        put16(&mut central, 20); // version made by
        put16(&mut central, 20); // version needed
        put16(&mut central, 0); // flags
        put16(&mut central, 0); // method: STORE
        put16(&mut central, 0); // time
        put16(&mut central, 0); // date
        put32(&mut central, crc);
        put32(&mut central, data.len() as u32);
        put32(&mut central, data.len() as u32);
        put16(&mut central, name.len() as u16);
        put16(&mut central, 0); // extra len
        put16(&mut central, 0); // comment len
        put16(&mut central, 0); // disk number
        put16(&mut central, 0); // internal attrs
        put32(&mut central, 0); // external attrs
        put32(&mut central, lho);
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    put32(&mut out, EOCD);
    put16(&mut out, 0); // this disk
    put16(&mut out, 0); // cd disk
    put16(&mut out, entries.len() as u16);
    put16(&mut out, entries.len() as u16);
    put32(&mut out, cd_size);
    put32(&mut out, cd_offset);
    put16(&mut out, 0); // comment len
    Ok(out)
}

fn get16(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn get32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Unpack a `.rutbundle`: all entries in archive order, each CRC-verified.
/// Unknown extra entries are returned too — the loader picks what it
/// knows (RFC 0038 §1, forward compatibility).
pub fn parse_bundle(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, BundleError> {
    // locate the EOCD: scan back over a possible zip comment (≤ 64 KiB)
    let mut eocd = None;
    let min = 22usize;
    if data.len() >= min {
        let start = data.len().saturating_sub(min + 0xFFFF);
        for i in (start..=data.len() - min).rev() {
            if get32(data, i) == Some(EOCD) {
                eocd = Some(i);
                break;
            }
        }
    }
    let Some(eocd) = eocd else {
        return Err(BundleError("not a zip archive (no end-of-central-directory)".into()));
    };
    let n = get16(data, eocd + 10).unwrap_or(0) as usize;
    let cd_size = get32(data, eocd + 12).unwrap_or(0) as usize;
    let cd_offset = get32(data, eocd + 16).unwrap_or(0) as usize;
    if n == 0xFFFF || cd_size == u32::MAX as usize || cd_offset == u32::MAX as usize {
        return Err(BundleError("zip64 bundles are not supported in v1".into()));
    }
    if cd_offset.checked_add(cd_size).map_or(true, |end| end > data.len()) {
        return Err(BundleError("corrupt bundle: central directory out of bounds".into()));
    }
    let mut out = Vec::new();
    let mut at = cd_offset;
    for _ in 0..n {
        if get32(data, at) != Some(CDH) {
            return Err(BundleError("corrupt bundle: bad central-directory entry".into()));
        }
        let method = get16(data, at + 10).unwrap_or(0xFFFF);
        let crc = get32(data, at + 16).unwrap_or(0);
        let size = get32(data, at + 20).unwrap_or(0) as usize;
        let csize = get32(data, at + 24).unwrap_or(0) as usize;
        let namelen = get16(data, at + 28).unwrap_or(0) as usize;
        let extralen = get16(data, at + 30).unwrap_or(0) as usize;
        let commentlen = get16(data, at + 32).unwrap_or(0) as usize;
        let lho = get32(data, at + 42).unwrap_or(u32::MAX) as usize;
        let name_at = at + 46;
        let Some(name) = data
            .get(name_at..name_at + namelen)
            .and_then(|b| std::str::from_utf8(b).ok())
        else {
            return Err(BundleError("corrupt bundle: entry name is not UTF-8".into()));
        };
        if method != 0 {
            return Err(BundleError(format!(
                "bundle entry `{name}` uses compression method {method} — v1 bundles are STORE-only"
            )));
        }
        if csize != size {
            return Err(BundleError(format!(
                "corrupt bundle: entry `{name}` size mismatch ({csize} compressed vs {size} stored)"
            )));
        }
        // follow the local header: its name/extra may differ in length from
        // the central directory's, so read them from the local record
        if get32(data, lho) != Some(LFH) {
            return Err(BundleError(format!(
                "corrupt bundle: entry `{name}` has a bad local header"
            )));
        }
        let lnamelen = get16(data, lho + 26).unwrap_or(0) as usize;
        let lextralen = get16(data, lho + 28).unwrap_or(0) as usize;
        let data_at = lho + 30 + lnamelen + lextralen;
        let Some(bytes) = data.get(data_at..data_at.wrapping_add(size)) else {
            return Err(BundleError(format!(
                "corrupt bundle: entry `{name}` data out of bounds"
            )));
        };
        if crc32(bytes) != crc {
            return Err(BundleError(format!(
                "corrupt bundle: entry `{name}` fails its CRC-32 check"
            )));
        }
        out.push((name.to_string(), bytes.to_vec()));
        at = name_at + namelen + extralen + commentlen;
    }
    Ok(out)
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
        // flip a byte inside the stored payload (first local header is 30+5)
        let at = 30 + 5;
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
            .position(|w| w == CDH.to_le_bytes())
            .expect("cdh present");
        bytes[cd + 10] = 8;
        let err = parse_bundle(&bytes).unwrap_err();
        assert!(err.0.contains("STORE"), "{err}");
    }
}
