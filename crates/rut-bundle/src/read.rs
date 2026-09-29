//! The bundle reader: parse the bytes once, then read. [`Bundle`] is
//! the entry-level view (CRC-verified at parse, entries by name); the
//! v5 [`Layout`] is the structured view — the compiled root program,
//! the pack-time scope ledger, and each dep group as compiled `.rutc`
//! or source file set. Parsing a layout decodes and verifies every
//! group binary, so a bad archive never reaches the session.

use crate::container::{parse_bundle, BundleError};
use crate::manifest::{parse_manifest, Manifest};
use crate::pack::read_entry;
use rut_core::binary::{decode, Program};

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
    /// bundle, then the payloads, then dep groups.
    pub fn entries(&self) -> &[(String, Vec<u8>)] {
        &self.entries
    }

    /// One entry's text, UTF-8-checked — a missing entry or a
    /// non-UTF-8 payload is a load error naming the key.
    pub fn read(&self, key: &str) -> Result<String, String> {
        read_entry(&self.entries, key)
    }

    /// One entry's raw bytes — the binary payloads (`.rutc`); `None`
    /// when the archive does not carry the key.
    pub fn bytes(&self, key: &str) -> Option<&[u8]> {
        self.entries.iter().find(|(n, _)| n == key).map(|(_, b)| b.as_slice())
    }
}

/// A dep group's payload kind.
#[derive(Clone, Debug)]
pub enum GroupKind {
    /// a source pkg's compiled group: its decoded `.rutc` (v17,
    /// decode-verified) rides `<pkg>/<pkg>.rutc` — bodies + surface,
    /// the linking truth
    Compiled(Program),
    /// a host pkg (a `.d.rut` surface, no body) or a declared-but-
    /// never-compiled dep: the declaration file set rides under `<pkg>/`
    Source,
}

/// The one bundle layout — v5, compiled. The root rides `<pkg>.rutc`
/// (a linkable root is the packer's law); `scopes` is the pack-time
/// scope ledger (every scope the closure's programs reference, mapped
/// to the spec that owned it — engine mounts included); `groups` lists
/// each dep group by its archive prefix.
#[derive(Clone, Debug)]
pub enum Layout {
    Compiled {
        /// the root manifest (byte-for-byte `rut.toml`, parsed)
        manifest: Manifest,
        /// the root's decoded program
        root: Program,
        /// pack-time scope → owning spec, ascending by scope
        scopes: Vec<(rut_core::id::ScopeId, String)>,
        /// `(archive prefix, payload)` per dep group, archive order
        groups: Vec<(String, GroupKind)>,
    },
}

impl Layout {
    /// Parse a v5 compiled bundle: manifest + version gate first
    /// (`format_version` must be exactly 5 — refuse, never guess),
    /// then the scope ledger, then the root and every group's binary
    /// decode + verification. The load order is: container CRC (the
    /// [`Bundle::parse`] this takes), manifest/version, per-group
    /// decode+verify — the mount is the caller's.
    pub fn parse(bundle: &Bundle) -> Result<Layout, String> {
        let toml = bundle
            .read("rut.toml")
            .map_err(|_| "no `rut.toml` entry — not a rut bundle".to_string())?;
        let manifest = parse_manifest(&toml).map_err(|e| e.to_string())?;
        if manifest.format.as_deref() != Some("rutbundle") {
            return Err("rut.toml has no `format = \"rutbundle\"` — not a rut bundle".into());
        }
        match manifest.format_version {
            Some(5) => {}
            other => {
                return Err(format!(
                    "this toolchain reads bundle format_version 5 only (found {other:?}) — re-pack the directory"
                ));
            }
        }
        let name = manifest
            .name
            .clone()
            .ok_or_else(|| "rut.toml has no `name`".to_string())?;
        // the scope ledger: one `<scope> = "<spec>"` row per linked
        // module of the packed closure, ascending by scope
        let ledger = bundle
            .read("rut.scopes")
            .map_err(|_| "no `rut.scopes` entry — not a v5 compiled bundle".to_string())?;
        let mut scopes = Vec::new();
        for (lineno, raw) in ledger.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let bad = || format!("rut.scopes line {}: expected `<scope> = \"<spec>\"`", lineno + 1);
            let (l, r) = line.split_once('=').ok_or_else(bad)?;
            let scope: u16 = l.trim().parse().map_err(|_| bad())?;
            let spec = r.trim();
            if !(spec.len() >= 2 && spec.starts_with('"') && spec.ends_with('"')) {
                return Err(bad());
            }
            let spec = spec[1..spec.len() - 1].to_string();
            if !crate::manifest::valid_spec(&spec) {
                return Err(format!(
                    "rut.scopes line {}: `{spec}` is not a bare package name",
                    lineno + 1
                ));
            }
            scopes.push((scope, spec));
        }
        scopes.sort_by_key(|&(s, _)| s);
        // the root's compiled program
        let root = decode(
            bundle
                .bytes(&format!("{name}.rutc"))
                .ok_or_else(|| format!("no `{name}.rutc` entry — a v5 bundle's root is compiled"))?,
        )
        .map_err(|e| format!("{name}.rutc: {e}"))?;
        // the dep groups: every archive prefix, compiled iff its
        // `<pkg>/<pkg>.rutc` rides (the packer's group law)
        let mut prefixes = std::collections::BTreeSet::new();
        for (n, _) in bundle.entries() {
            if let Some((p, rest)) = n.split_once('/') {
                if rest.is_empty() || p == "rut.scopes" {
                    continue;
                }
                prefixes.insert(p.to_string());
            }
        }
        let mut groups = Vec::new();
        for p in prefixes {
            let toml = bundle
                .read(&format!("{p}/rut.toml"))
                .map_err(|_| format!("bundle group `{p}/` has no `rut.toml`"))?;
            let dm = parse_manifest(&toml)
                .map_err(|e| format!("{p}/rut.toml: {e}"))?;            let gname = dm
                .name
                .clone()
                .ok_or_else(|| format!("{p}/rut.toml has no `name`"))?;
            let kind = match bundle.bytes(&format!("{p}/{gname}.rutc")) {
                Some(bytes) => GroupKind::Compiled(
                    decode(bytes).map_err(|e| format!("{p}/{gname}.rutc: {e}"))?,
                ),
                None => GroupKind::Source,
            };
            groups.push((p, kind));
        }
        Ok(Layout::Compiled { manifest, root, scopes, groups })
    }
}
