//! Deps — the manifest/bundle bridge between a rut project's
//! `rut.jsonc` files and the definition index. Everything here is PURE
//! (text/bytes in, values out): the hosts move the bytes, this module
//! reads them — one engine, no JS-side rut parsing.
//!
//! Two entries:
//! - [`dep_table`] parses one manifest into a [`DepTable`]: the
//!   module's own identity (`name`, entry sources with the mode each
//!   indexes in) plus its dep rows (`deps`, `dev-deps`), each a `path`
//!   directory or a `url` bundle with the `.rut/cache/` cache path
//!   computed here. The pin is NOT the manifest's business —
//!   [`lock_pins`] reads `rut.lock` (the walk root's ledger) and
//!   [`dep_table_pinned`] stamps its rows onto the url rows.
//! - [`mount_file`] mounts one dep source file under its stamp (the
//!   module NAMED, the mod path spelled); the surface `pub let` rows
//!   of a current tree ride the ordinary ModuleLet path; the
//!   qualified forms ride the dep's own `mod` tree: `use calc::{
//!   math }` + `math::PI`).
//!
//! (The archive unpack lane is GONE from this crate: production mounts
//! the store trees pre-unpacked — `rut install` lays the `unpack/`
//! dirs and the walk mounts directories — so the editor never parses
//! bytes; the zip/bundle codecs are rut-driver's own gated lane. The
//! test-only unpack surface that used to live here was removed with
//! the unit-test tree that pinned it.)

use rut_driver::bundle::manifest::parse_manifest;
use rut_driver::bundle::Manifest;

use crate::semantic::Mode;
use sha2::{Digest, Sha256};

use crate::world::{File, Stamp};

/// One entry source a dep indexes: the manifest-relative file path and
/// the mode it parses in (`entry.type` → Decl, the `mod.rut` root →
/// Impl).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySource {
    pub path: String,
    pub mode: Mode,
}

/// Where a dep row's bytes live: a directory beside the manifest, or a
/// remote `.rutbundle` pinned by its sha256 (the pin lock-sourced —
/// `rut.lock` at the dep-walk root, never the manifest). The cache
/// path is computed HERE (`.rut/cache/<sha256(url)>.rutbundle`) — the
/// faces never hash URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepSource {
    Path {
        dir: String,
    },
    Url {
        url: String,
        sha256: Option<String>,
        cache_path: String,
    },
}

/// One dep row (`deps` / `dev-deps`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepRow {
    pub name: String,
    pub source: DepSource,
}

/// One parsed manifest, editor-shaped: the module's own identity plus
/// its dep rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DepTable {
    /// the manifest's `name` — what a `use` path spells
    pub name: Option<String>,
    /// the entry sources, in indexing order: the decl surface first
    /// (`entry.type`, Decl mode — or the default root `mod.d.rut` when
    /// a host pkg spells none), then the body — the root module
    /// `mod.rut` (Impl mode; the convention, not a key).
    pub entries: Vec<EntrySource>,
    /// `type = "host"` — the entry surface is a pure `.d.rut` and its
    /// `mod` tree mounts the Decl flavor (`NAME/mod.d.rut` children,
    /// Decl mode) instead of a lib body's `mod.rut` tree
    pub host: bool,
    /// `deps` — transitively mounted
    pub deps: Vec<DepRow>,
    /// `dev-deps` — mounted only for the pkg's own build/tests
    pub dev_deps: Vec<DepRow>,
}

/// The cache path for a url dep — `.rut/cache/<sha256(url)>.<ext>`,
/// relative to the workspace root the face shares with the CLI; the
/// extension follows the url's own artifact kind (`.zip` = source
/// transport, `.rutbundle` = compiled) — the remote's naming law.
pub fn cache_path(url: &str) -> String {
    let hex: String = Sha256::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!(".rut/cache/{hex}{}", rut_driver::bundle::cache_ext(url))
}

/// One dep row from a parsed descriptor map. `deps` rows may be path-
/// or url-sourced; `dev-deps` descriptors are path-only by
/// grammar (a url there refused at parse) — the url arm simply never
/// fires for them. Url rows parse UNPINNED: the manifest grammar
/// retired `sha256` (pins live in `rut.lock`) — the pin arrives from
/// `rut.lock` via [`dep_table_pinned`].
fn dep_row(name: &str, desc: &std::collections::BTreeMap<String, String>) -> DepRow {
    let source = match (desc.get("path"), desc.get("url")) {
        (_, Some(url)) => DepSource::Url {
            url: url.clone(),
            sha256: None,
            cache_path: cache_path(url),
        },
        (Some(dir), _) => DepSource::Path { dir: dir.clone() },
        _ => DepSource::Path { dir: String::new() },
    };
    DepRow {
        name: name.to_string(),
        source,
    }
}

/// Parse one `rut.jsonc` (JSONC: comments + trailing commas legal)
/// into the editor's dep table. A malformed manifest is the error —
/// the faces surface it as a hint, never silent bytes. ONE grammar,
/// every lane — the directory door, the store tree, the archive's own
/// manifest; the wire pair a packed manifest carries is legal-optional.
pub fn dep_table(manifest_text: &str) -> Result<DepTable, String> {
    let m: Manifest = parse_manifest(manifest_text).map_err(|e| e.to_string())?;
    dep_table_of(m)
}

/// The url → pin map one `rut.lock` spells — the pin ledger the faces
/// read at the dep-walk root and thread through [`dep_table_pinned`].
/// The lock is the pin's ONLY source (the manifest grammar retired
/// `sha256`); the map is keyed by url because that is the identity the
/// CLI's install writes rows under (one lock pins the whole closure,
/// nested path-dep url rows included). Malformed text is the error —
/// the caller (the server walk) surfaces it as a hint, never silent
/// pins.
pub fn lock_pins(lock_text: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
    let lock = rut_driver::parse_lock(lock_text).map_err(|e| e.to_string())?;
    Ok(lock
        .deps()
        .iter()
        .map(|r| (r.url.clone(), r.sha256.clone()))
        .collect())
}

/// [`dep_table`] with the dep-walk root's `rut.lock` applied: every url
/// row the lock pins answers `sha256: Some(pin)`; a url with no lock
/// row stays `None` — the walk refuses it with the install recipe
/// (the editor's mirror of the loader's `Unpinned` law).
pub fn dep_table_pinned(
    manifest_text: &str,
    pins: &std::collections::BTreeMap<String, String>,
) -> Result<DepTable, String> {
    stamp_pins(dep_table(manifest_text)?, pins)
}

/// the pin ledger's stamp: every url row the lock pins answers
/// `sha256: Some(pin)`; a url with no lock row stays `None`.
fn stamp_pins(
    mut t: DepTable,
    pins: &std::collections::BTreeMap<String, String>,
) -> Result<DepTable, String> {
    for row in t.deps.iter_mut().chain(t.dev_deps.iter_mut()) {
        if let DepSource::Url { url, sha256, .. } = &mut row.source {
            *sha256 = pins.get(url).cloned();
        }
    }
    Ok(t)
}

fn dep_table_of(m: Manifest) -> Result<DepTable, String> {
    let mut entries = Vec::new();
    // the decl surface indexes in Decl mode; the body in Impl mode —
    // the same split the filename convention (`crate::semantic::mode_of`)
    // speaks for every document. On a host pkg with no spelled
    // surface, `src/mod.d.rut` under the shell IS the root (the
    // loader's default-root discovery mirrored) — an unreadable row
    // simply indexes nothing.
    match &m.entry.type_path {
        Some(ty) => entries.push(EntrySource {
            path: rut_driver::bundle::src_rel(ty),
            mode: Mode::Decl,
        }),
        None if m.pkg_type == rut_driver::bundle::PkgType::Host => {
            entries.push(EntrySource {
                path: rut_driver::bundle::DECL_ROOT_MODULE.to_string(),
                mode: Mode::Decl,
            });
        }
        None => {}
    }
    // the body is the ROOT MODULE — `src/mod.rut`, the convention every
    // lane reads. A surface-only dev state (or a host pkg) names no
    // body: a Lib pkg with an entry.type spelled has its root probed by
    // the callers, and an unreadable row simply indexes nothing (what
    // exists is what mounts).
    if m.pkg_type == rut_driver::bundle::PkgType::Lib {
        entries.push(EntrySource {
            path: rut_driver::bundle::ROOT_MODULE.to_string(),
            mode: Mode::Impl,
        });
    }
    let rows =
        |table: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>| {
            table
                .iter()
                .map(|(name, desc)| dep_row(name, desc))
                .collect()
        };
    Ok(DepTable {
        name: m.name,
        entries,
        host: m.pkg_type == rut_driver::bundle::PkgType::Host,
        deps: rows(&m.deps),
        dev_deps: rows(&m.dev_deps),
    })
}

/// Mount one dep source file — the flat shape (`mod_path` `None`,
/// today's single-file packages) or one file module of a mod-carrying
/// pkg (`Some(path)`, `""` = the pkg root's `mod.rut`): the ordinary
/// parse over `src`, the mount — origin, module name, mod path, parse
/// mode — riding ONE [`Stamp`] built up front. The surface `pub let`
/// rows ride the ordinary ModuleLet path; the module-qualified forms
/// ride the dep's own `mod` tree.
pub fn mount_file(name: &str, uri: &str, mod_path: Option<&str>, src: String) -> File {
    // `mode_of(uri)` spells the filename convention (every caller
    // already derived it that way). `src` BY VALUE: it moves into the
    // doc (a fresh walk read moves for free, a borrowed text clones to
    // move)
    File::from_src(Stamp::dep(name, uri, mod_path), src)
}

// (the minted manifest-const rows are gone — the retired `consts`
// table refuses at parse; the surface `pub let` is the one spelling)
