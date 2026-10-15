//! Content identity — the `deps/<name>@<fp16>/` law (wire v11). A
//! package's identity is `name@<16 hex>` where fp16 is the first 16
//! hex chars of sha256 over its own rode source set in fixed order
//! (`rut.jsonc` + `mod.rut` + `mod.d.rut` + `rut.mods`, raw bytes,
//! absent files skipped). Same source ⇒ same identity (the
//! determinism law); `@` is not in the spec alphabet, so an identity
//! can never collide with a bare package name. Two identities of one
//! name mount side by side — first-mount-wins becomes per-identity
//! exact dedup.
//!
//! Three readers enforce the shape: bundle group dir names
//! (`deps/<identity>/`), scope-ledger rows, and the mount table's
//! keys (a mount key is a bare name OR an identity). The packer
//! computes the fingerprint from the staged entries it is about to
//! emit; the loader recomputes it from the archive and REFUSES a
//! mismatch (a doctored dirname).

use crate::bundle::manifest::valid_spec;

/// The rode source set's fixed order — the fingerprint's input, by
/// exact entry name under the group's prefix. Absent files skip.
pub const RIDE_FILES: [&str; 4] = ["rut.jsonc", "mod.rut", "mod.d.rut", "rut.mods"];

/// The `deps/` directory groups ride under — ONE level, every group.
pub const DEPS_DIR: &str = "deps";

/// sha256 over `parts`, concatenated in order; the first 16 hex chars,
/// lowercase.
fn fp16_of_parts(parts: [&[u8]; 4]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for p in parts {
        digest.update(p);
    }
    let out = digest.finalize();
    let mut s = String::with_capacity(16);
    for b in out.iter().take(8) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The fingerprint over an archive's entries at `prefix` — the four
/// canonical names under it, raw bytes, fixed order, absent skipped.
/// The reader's half of the identity law (and the packer's, over the
/// staged rels at prefix `""`).
pub fn fp16_of(entries: &[(String, Vec<u8>)], prefix: &str) -> String {
    let pick = |name: &str| -> &[u8] {
        let key = format!("{prefix}{name}");
        entries
            .iter()
            .find(|(n, _)| *n == key)
            .map(|(_, b)| b.as_slice())
            .unwrap_or(b"")
    };
    let [a, b, c, d] = RIDE_FILES;
    fp16_of_parts([pick(a), pick(b), pick(c), pick(d)])
}

/// The identity string: `name@fp16`.
pub fn identity(name: &str, fp16: &str) -> String {
    format!("{name}@{fp16}")
}

/// The identity-shape law: `<name>@<16 lowercase hex>` with `name` a
/// bare package name — answers the two halves. `valid_spec` alone no
/// longer suffices for anything that may carry an identity.
pub fn parse_identity(s: &str) -> Option<(&str, &str)> {
    let (name, fp) = s.split_once('@')?;
    if !valid_spec(name) || fp.len() != 16 || !fp.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if fp.bytes().any(|b| b.is_ascii_uppercase()) {
        return None;
    }
    Some((name, fp))
}

/// Is `s` identity-shaped?
pub fn is_identity(s: &str) -> bool {
    parse_identity(s).is_some()
}

/// A mount table key: a bare package name or a content identity.
pub fn valid_mount_key(s: &str) -> bool {
    valid_spec(s) || is_identity(s)
}

/// The bare name a mount key spells — the name-part of an identity,
/// else the key itself (diagnostics, host-row registration scopes,
/// name-vs-key gates all spell the bare name).
pub fn mount_name(key: &str) -> &str {
    parse_identity(key).map(|(n, _)| n).unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_is_the_empty_hash() {
        // a pkg with no rode files (an engine mount's ledger row)
        // fingerprints the empty byte string — deterministic
        assert_eq!(fp16_of(&[], ""), "e3b0c44298fc1c14");
    }

    #[test]
    fn fixed_order_absent_skipped() {
        let entries = vec![
            ("mod.rut".to_string(), b"body".to_vec()),
            ("rut.jsonc".to_string(), b"manifest".to_vec()),
        ];
        let a = fp16_of(&entries, "");
        // reordering the INPUT list must not move the hash — the order
        // is the file-name law's, never the archive's
        let entries2 = vec![
            ("rut.jsonc".to_string(), b"manifest".to_vec()),
            ("mod.rut".to_string(), b"body".to_vec()),
        ];
        assert_eq!(a, fp16_of(&entries2, ""));
        // a prefixed read picks the group's own files
        let grouped = vec![
            ("deps/u@abc/rut.jsonc".to_string(), b"manifest".to_vec()),
            ("rut.jsonc".to_string(), b"OTHER".to_vec()),
        ];
        assert_ne!(a, fp16_of(&grouped, "deps/u@abc/"));
        // a foreign entry never enters the hash
        let noisy = vec![
            ("rut.jsonc".to_string(), b"manifest".to_vec()),
            ("mod.rut".to_string(), b"body".to_vec()),
            ("extra.rut".to_string(), b"noise".to_vec()),
        ];
        assert_eq!(a, fp16_of(&noisy, ""));
    }

    #[test]
    fn identity_shape_law() {
        assert!(parse_identity("util@0123456789abcdef").is_some());
        assert!(parse_identity("util@e3b0c44298fc1c14").is_some());
        // 15 hex, 17 hex, uppercase, bad name, no @
        assert!(parse_identity("util@0123456789abcde").is_none());
        assert!(parse_identity("util@0123456789abcdef0").is_none());
        assert!(parse_identity("util@0123456789ABCDEF").is_none());
        assert!(parse_identity("ut-il@0123456789abcdef").is_none());
        assert!(parse_identity("util").is_none());
        // the separator cannot collide with a real name
        assert!(valid_spec("util@0123456789abcdef") == false);
        assert!(valid_mount_key("util"));
        assert!(valid_mount_key("util@0123456789abcdef"));
        assert!(!valid_mount_key("ut-il"));
        assert_eq!(mount_name("util@0123456789abcdef"), "util");
        assert_eq!(mount_name("util"), "util");
    }
}
