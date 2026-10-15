//! The bundle reader: parse the bytes once, then read. [`Bundle`] is
//! the entry-level view (CRC-verified at parse, entries by name); the
//! [`Layout`] is the structured view, ONE of two root kinds — the
//! manifest's `type` routes: a lib root (no `type`) is the **compiled**
//! layout (`mod.rutc` + scope ledger + `deps/` groups), a
//! `type = "host"` root is the **decl** layout (its `mod.d.rut`
//! surface, single-package). One wire number: `format_version` is 11,
//! always. Parsing a layout verifies what each kind carries, so a bad
//! archive never reaches the session.

use super::container::{parse_bundle, BundleError};
use super::manifest::{parse_manifest_compat, Manifest};
use super::files::read_entry;
use rut_core::binary::{decode, Program};

use super::identity::{self, DEPS_DIR};

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
    /// decode-verified) rides `deps/<identity>/mod.rutc` — bodies +
    /// surface, the linking truth
    Compiled(Program),
    /// a host pkg (a `.d.rut` surface, no body) or a declared-but-
    /// never-compiled dep: the declaration file set rides under
    /// `deps/<identity>/`
    Source,
}

/// The one bundle layout — ONE wire number, the manifest's `type`
/// routes the root kind: a lib root (no `type`) is **compiled**
/// (`mod.rutc` + ledger + `deps/` groups), a `type = "host"` root is
/// **decl** (the manifest + its `mod.d.rut` surface, single-package,
/// nothing else). The archive root carries ONLY the root's own files
/// (`rut.jsonc`, `rut.scopes`, `mod.rutc` + its surface/source/rows);
/// every dep group rides at `deps/<name>@<fp16>/…` — the archive
/// mirrors the manifest's `deps` table, name-free.
#[derive(Clone, Debug)]
pub enum Layout {
    Compiled {
        /// the root manifest (byte-for-byte `rut.jsonc`, parsed)
        manifest: Manifest,
        /// the root's decoded program
        root: Program,
        /// pack-time scope → owning identity, ascending by scope —
        /// every row spells `name@fp16` (the group identities; the
        /// loader resolves the self-row and engine rows to their bare
        /// mounts)
        scopes: Vec<(rut_core::id::ScopeId, String)>,
        /// `(group prefix, payload)` per dep group, archive order —
        /// the prefix is `deps/<name>@<fp16>`, identity-validated
        /// (shape, manifest name, fingerprint-over-content)
        groups: Vec<(String, GroupKind)>,
    },
    /// a decl root: the pkg's declaration surface rides as its own
    /// source — a single-package bundle, no ledger, no groups, nothing
    /// to decode
    Decl {
        /// the root manifest (byte-for-byte `rut.jsonc`, parsed)
        manifest: Manifest,
        /// the surface text (`mod.d.rut` — the ONE canonical name)
        /// — presence- and UTF-8-checked here
        surface: String,
    },
}

impl Layout {
    /// Parse a bundle: manifest + version gate first (`format_version`
    /// is 11 — one wire number for both root kinds; anything else
    /// refuses with the one re-pack recipe, never guesses), then the
    /// kind's own checks (compiled: the scope ledger's identity rows,
    /// the root and every group's binary decode + verification, every
    /// group dirname's fingerprint-over-content; decl: the
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
            Some(11) => match manifest.pkg_type {
                super::manifest::PkgType::Host => Self::parse_decl(bundle, manifest),
                super::manifest::PkgType::Lib => Self::parse_compiled(bundle, manifest),
            },
            other => Err(format!(
                "this toolchain reads bundle format_version 11 only (found {}) — re-pack the directory",
                match other {
                    Some(v) => v.to_string(),
                    None => "none".to_string(),
                }
            )),
        }
    }

    /// The compiled arm: a lib root (no `type` — the grammar routes a
    /// `type = "host"` root to the decl arm), the layout the packer
    /// emits: root `mod.rutc` + scope ledger + `deps/` groups.
    fn parse_compiled(bundle: &Bundle, manifest: Manifest) -> Result<Layout, String> {
        // the scope ledger: one `<scope> = "<identity>"` row per linked
        // module of the packed closure, ascending by scope — every row
        // spells the owner's content identity (`name@fp16`); the
        // explicit shape law (name + `@` + 16 lowercase hex) —
        // `valid_spec` alone no longer suffices
        let ledger = bundle
            .read("rut.scopes")
            .map_err(|_| "no `rut.scopes` entry — a compiled bundle carries the scope ledger".to_string())?;
        let mut scopes = Vec::new();
        for (lineno, raw) in ledger.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let bad = || format!("rut.scopes line {}: expected `<scope> = \"<identity>\"`", lineno + 1);
            let (l, r) = line.split_once('=').ok_or_else(bad)?;
            let scope: u16 = l.trim().parse().map_err(|_| bad())?;
            let row = r.trim();
            if !(row.len() >= 2 && row.starts_with('"') && row.ends_with('"')) {
                return Err(bad());
            }
            let row = row[1..row.len() - 1].to_string();
            if !identity::is_identity(&row) {
                return Err(format!(
                    "rut.scopes line {}: `{row}` is not a content identity (`name@<16 hex>`)",
                    lineno + 1
                ));
            }
            scopes.push((scope, row));
        }
        scopes.sort_by_key(|&(s, _)| s);
        // the root's compiled program — the ONE canonical name
        let root = decode(
            bundle
                .bytes("mod.rutc")
                .ok_or_else(|| "no `mod.rutc` entry — a compiled bundle's root is a `.rutc`".to_string())?,
        )
        .map_err(|e| format!("mod.rutc: {e}"))?;
        // the dep groups: every group rides at `deps/<identity>/…` —
        // discovered ONLY as `deps/<comp>/rut.jsonc`; a stray
        // top-level directory prefix refuses (the archive root
        // carries only the root's own files — refuse, never guess)
        let mut comps = std::collections::BTreeSet::new();
        for (n, _) in bundle.entries() {
            let Some((top, rest)) = n.split_once('/') else {
                continue; // the root's own files
            };
            if rest.is_empty() || top == "rut.scopes" {
                continue;
            }
            if top != DEPS_DIR {
                return Err(format!(
                    "bundle entry `{n}` — the archive root carries only the root's own files; \
                     dep groups ride at `{DEPS_DIR}/<name>@<fp16>/`"
                ));
            }
            let Some(comp) = rest.split('/').next() else {
                continue;
            };
            if comp.is_empty() {
                continue;
            }
            comps.insert(comp.to_string());
        }
        let mut groups = Vec::new();
        for comp in comps {
            let p = format!("{DEPS_DIR}/{comp}");
            // the dirname IS the content identity — shape law first
            let (gname, fp) = identity::parse_identity(&comp).ok_or_else(|| {
                format!(
                    "bundle group directory `{p}/` is not a content identity — \
                     expected `<name>@<16 hex>`"
                )
            })?;
            let manifest_text = bundle
                .read(&format!("{p}/{}", super::files::MANIFEST_NAME))
                .map_err(|_| {
                    format!("bundle group `{p}/` has no `{}`", super::files::MANIFEST_NAME)
                })?;
            let dm = parse_manifest_compat(&manifest_text)
                .map_err(|e| format!("{p}/{}: {e}", super::files::MANIFEST_NAME))?;
            // name-vs-key: the group's manifest `name` must equal the
            // deps key with `@fp16` stripped — the dirname's name half
            if dm.name.as_deref() != Some(gname) {
                return Err(format!(
                    "bundle group `{p}/` names itself `{}` — expected `{gname}`",
                    dm.name.as_deref().unwrap_or("<unnamed>"),
                ));
            }
            // the fingerprint-over-content law: the dirname's fp16 must
            // equal the hash of the group's rode source set — a doctored
            // or stale dirname refuses
            let got = identity::fp16_of(bundle.entries(), &format!("{p}/"));
            if got != fp {
                return Err(format!(
                    "bundle group directory `{p}/` spells fingerprint `{fp}`, but its rode \
                     source set hashes to `{got}` — the dirname and the content disagree"
                ));
            }
            let kind = match bundle.bytes(&format!("{p}/mod.rutc")) {
                Some(bytes) => GroupKind::Compiled(
                    decode(bytes).map_err(|e| format!("{p}/mod.rutc: {e}"))?,
                ),
                None => GroupKind::Source,
            };
            groups.push((p, kind));
        }
        Ok(Layout::Compiled { manifest, root, scopes, groups })
    }

    /// The decl arm: a `type = "host"` root, its surface riding as
    /// source at the ONE canonical name. Anything compiled-shaped the
    /// archive cannot legally carry refuses: a root `mod.rutc` (a host
    /// bundle's root is its surface), a scope ledger (no programs, no
    /// ledger), any dep group — a `deps/` entry included (a host
    /// bundle is single-package; the grammar refuses a host manifest's
    /// deps tables, so there is nothing for a group to be).
    fn parse_decl(bundle: &Bundle, manifest: Manifest) -> Result<Layout, String> {
        if bundle.bytes("mod.rutc").is_some() {
            return Err(
                "a host bundle's root is its surface, not a compiled unit — found a `mod.rutc` entry"
                    .to_string(),
            );
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
        // the surface entry: `mod.d.rut` beside the manifest — the ONE
        // canonical name (wire v11 canonicalizes `entry.type`'s
        // directory spelling; the override lane is gone)
        let surface = bundle.read(crate::decl::DECL_CHILD_FILE).map_err(|_| {
            "a host bundle's root is `mod.d.rut` beside the manifest — no such entry".to_string()
        })?;
        Ok(Layout::Decl { manifest, surface })
    }
}
