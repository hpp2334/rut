//! The bundle reader: parse the bytes once, then read. [`Bundle`] is
//! the entry-level view (CRC-verified at parse, entries by name); the
//! [`Layout`] is the structured view, ONE of two root kinds — the
//! manifest's `type` routes: a lib root (no `type`) is the **compiled**
//! layout (`.rutc` + scope ledger + ridden source), a `type = "host"`
//! root is the **decl** layout (its `.d.rut` surface, single-package).
//! One wire number: `format_version` is 10, always. Parsing a layout
//! verifies what each kind carries, so a bad archive never reaches the
//! session.

use super::container::{parse_bundle, BundleError};
use super::manifest::{parse_manifest_compat, Manifest};
use super::files::read_entry;
use rut_core::binary::{decode, Program};

/// A parsed `.rutbundle`: its entries in archive order. Loaders pick
/// the entries they know (`rut.jsonc`, the rut sources); unknown extra
/// entries ride along (forward compatibility).
pub struct Bundle {
    entries: Vec<(String, Vec<u8>)>,
}

impl Bundle {
    /// Parse and CRC-verify a `.rutbundle` from its bytes.
    pub fn parse(data: &[u8]) -> Result<Bundle, BundleError> {
        Ok(Bundle { entries: parse_bundle(data)? })
    }

    /// All entries in archive order — `rut.jsonc` first for a packed
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

/// The one bundle layout — ONE wire number, the manifest's `type`
/// routes the root kind: a lib root (no `type`) is **compiled**
/// (`<pkg>.rutc` + ledger + groups), a `type = "host"` root is
/// **decl** (the manifest + its surface, single-package, nothing
/// else).
#[derive(Clone, Debug)]
pub enum Layout {
    Compiled {
        /// the root manifest (byte-for-byte `rut.jsonc`, parsed)
        manifest: Manifest,
        /// the root's decoded program
        root: Program,
        /// pack-time scope → owning spec, ascending by scope
        scopes: Vec<(rut_core::id::ScopeId, String)>,
        /// `(archive prefix, payload)` per dep group, archive order
        groups: Vec<(String, GroupKind)>,
    },
    /// a decl root: the pkg's declaration surface rides as its own
    /// source — a single-package bundle, no ledger, no groups, nothing
    /// to decode
    Decl {
        /// the root manifest (byte-for-byte `rut.jsonc`, parsed)
        manifest: Manifest,
        /// the surface text (the manifest's `entry.type` file) —
        /// presence- and UTF-8-checked here
        surface: String,
    },
}

impl Layout {
    /// Parse a bundle: manifest + version gate first (`format_version`
    /// is 10 — one wire number for both root kinds; anything else
    /// refuses with the one re-pack recipe, never guesses), then the
    /// kind's own checks (compiled: the scope ledger, the root and
    /// every group's binary decode + verification; decl: the
    /// single-package law and the surface read). The load order is:
    /// container CRC (the [`Bundle::parse`] this takes),
    /// manifest/version, then the kind's payloads — the mount is the
    /// caller's.
    pub fn parse(bundle: &Bundle) -> Result<Layout, String> {
        let manifest_text = match bundle.read(super::files::MANIFEST_NAME) {
            Ok(text) => text,
            Err(_) => return Err("no `rut.jsonc` entry — not a rut bundle".to_string()),
        };
        let manifest = parse_manifest_compat(&manifest_text).map_err(|e| e.to_string())?;
        if manifest.format.as_deref() != Some("rutbundle") {
            return Err("rut.jsonc has no `format = \"rutbundle\"` — not a rut bundle".into());
        }
        match manifest.format_version {
            // ONE wire number, both root kinds: the manifest's `type`
            // routes the layout — a host root is its own decl surface,
            // a lib root the compiled layout
            Some(10) => match manifest.pkg_type {
                super::manifest::PkgType::Host => Self::parse_decl(bundle, manifest),
                super::manifest::PkgType::Lib => Self::parse_compiled(bundle, manifest),
            },
            other => Err(format!(
                "this toolchain reads bundle format_version 10 only (found {}) — re-pack the directory",
                match other {
                    Some(v) => v.to_string(),
                    None => "none".to_string(),
                }
            )),
        }
    }

    /// The compiled arm: a lib root (no `type` — the grammar routes a
    /// `type = "host"` root to the decl arm), the layout the packer
    /// emits: root `.rutc` + scope ledger + dep groups.
    fn parse_compiled(bundle: &Bundle, manifest: Manifest) -> Result<Layout, String> {
        let name = manifest
            .name
            .clone()
            .ok_or_else(|| "rut.jsonc has no `name`".to_string())?;
        // the scope ledger: one `<scope> = "<spec>"` row per linked
        // module of the packed closure, ascending by scope
        let ledger = bundle
            .read("rut.scopes")
            .map_err(|_| "no `rut.scopes` entry — a compiled bundle carries the scope ledger".to_string())?;
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
            if !super::manifest::valid_spec(&spec) {
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
                .ok_or_else(|| format!("no `{name}.rutc` entry — a compiled bundle's root is a `.rutc`"))?,
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
            let manifest_text = bundle
                .read(&format!("{p}/{}", super::files::MANIFEST_NAME))
                .map_err(|_| {
                    format!("bundle group `{p}/` has no `{}`", super::files::MANIFEST_NAME)
                })?;
            let dm = parse_manifest_compat(&manifest_text)
                .map_err(|e| format!("{p}/{}: {e}", super::files::MANIFEST_NAME))?;
            let gname = dm.name.clone().ok_or_else(|| {
                format!("{p}/{} has no `name`", super::files::MANIFEST_NAME)
            })?;
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

    /// The decl arm: a `type = "host"` root, its surface riding as
    /// source. Anything compiled-shaped the archive cannot legally
    /// carry refuses: a root `.rutc` (a host bundle's root is its
    /// surface), a scope ledger (no programs, no ledger), any dep
    /// group (a host bundle is single-package — the grammar refuses a
    /// host manifest's deps tables, so there is nothing for a group
    /// to be).
    fn parse_decl(bundle: &Bundle, manifest: Manifest) -> Result<Layout, String> {
        let name = manifest
            .name
            .clone()
            .ok_or_else(|| "rut.jsonc has no `name`".to_string())?;
        if bundle.bytes(&format!("{name}.rutc")).is_some() {
            return Err(format!(
                "a host bundle's root is its surface, not a compiled unit — found a `{name}.rutc` entry"
            ));
        }
        if bundle.bytes("rut.scopes").is_some() {
            return Err(
                "a host bundle carries no programs — found a `rut.scopes` ledger entry".into()
            );
        }
        for (n, _) in bundle.entries() {
            if n.contains('/') {
                return Err(format!(
                    "a host bundle is single-package — found group entry `{n}`"
                ));
            }
        }
        let Some(rel) = &manifest.entry.type_path else {
            return Err(
                "rut.jsonc has no `entry.type` — a host bundle's root is its surface".into(),
            );
        };
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let surface = bundle
            .read(rel)
            .map_err(|e| format!("{e} — a host bundle's root is its surface"))?;
        Ok(Layout::Decl { manifest, surface })
    }
}
