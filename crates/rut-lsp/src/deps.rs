//! Deps — the manifest/bundle bridge between a rut project's
//! `rut.jsonc` files and the definition index. Everything here is PURE
//! (text/bytes in, values out): the faces (wasm shim, stdio server)
//! move the bytes, this module reads them — one engine, two faces, no
//! JS-side rut parsing.
//!
//! Three entries:
//! - [`dep_table`] parses one manifest into a [`DepTable`]: the
//!   module's own identity (`name`, entry sources with the mode each
//!   indexes in, `namespace`, `consts`) plus its dep rows (`deps`,
//!   `dev-deps`), each a `path` directory or a `url`
//!   bundle with the `.rut/cache/` cache path computed here. The pin
//!   is NOT the manifest's business — [`lock_pins`] reads `rut.lock`
//!   (the walk root's ledger) and [`dep_table_pinned`] stamps its rows
//!   onto the url rows.
//! - [`bundle_sources`] unpacks a `.rutbundle` into the module's OWN
//!   source files — the zip's own `rut.jsonc` names the module; the
//!   rode-along dep groups are the loader's business, not the
//!   editor's.
//! - [`index_dep`] indexes one dep source file: [`analysis::index_at`]
//!   plus `module = Some(name)`, with `namespace`/`consts` minted onto
//!   the index (a namespace-head type row, module-scope lets for the
//!   consts — the `calc`/`Math::PI` shape: the engine binds
//!   `use calc::{Math}` then routes `Math::sqrt`/`Math::PI` as qualified
//!   access, and the member machinery resolves both through the minted
//!   row).

use rut_ast::ast::Vis;
use rut_driver::bundle::files::{bundle_key, read_entry, MANIFEST_NAME};
use rut_driver::bundle::manifest::{parse_manifest, parse_manifest_compat};
use rut_driver::bundle::{parse_bundle, Manifest};
use rut_lexer::span::Span;

/// A bundle's rows head: the owning spec plus its mounted sources.
type ModHead = (
    String,
    std::collections::BTreeMap<String, rut_driver::ModSource>,
);
use rut_parser::Mode;
use sha2::{Digest, Sha256};

use crate::hover::types::{MemberSrc, TyDef, TyForm};
use crate::hover::DefIndex;

/// One entry source a dep indexes: the manifest-relative file path and
/// the mode it parses in (`entry.type` → Decl, `entry.lib`/`libs` →
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
    pub optional: bool,
    pub lib: Option<String>,
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
    /// `mod.rut` (Impl mode; the convention, not a key). An OLD
    /// published bundle's manifest (parsed through
    /// [`dep_table_compat`]) carries the repealed keys instead, mapped
    /// exactly as the loader's reader-compat law does.
    pub entries: Vec<EntrySource>,
    /// the qualified-access head (`calc`'s `Math`)
    pub namespace: Option<String>,
    /// the compiler-materialized constants (`Math::PI`), name → f64
    pub consts: Vec<(String, f64)>,
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
/// relative to the workspace root both faces share with the CLI; the
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
/// retired `sha256` (an old bundle manifest's pin rides as inert
/// metadata the compat lane ignores, exactly like the loader does) —
/// the pin arrives from `rut.lock` via [`dep_table_pinned`].
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
        optional: desc.get("optional").map(|o| o == "true").unwrap_or(false),
        lib: desc.get("lib").cloned(),
    }
}

/// Parse one `rut.jsonc` (JSONC: comments + trailing commas legal)
/// into the editor's dep table. A malformed manifest is the error —
/// the faces surface it as a hint, never silent bytes. STRICT: the
/// repealed `entry.lib`/`entry.libs` keys refuse, and the body rides
/// the `mod.rut` convention. The bundle face parses OLD published
/// manifests through [`dep_table_compat`].
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
    let mut t = dep_table(manifest_text)?;
    for row in t.deps.iter_mut().chain(t.dev_deps.iter_mut()) {
        if let DepSource::Url { url, sha256, .. } = &mut row.source {
            *sha256 = pins.get(url).cloned();
        }
    }
    Ok(t)
}

/// [`dep_table`] over the bundle reader's compat lane: an archive's
/// own manifest may be OLD (a published bundle is forever old — the
/// pinned CDN tags), so the repealed keys map onto Impl entries
/// exactly as the loader's reader mounts them (the lib, then the
/// `libs` splice, in listed order), REPLACING the `mod.rut`
/// convention row an old archive cannot answer.
pub fn dep_table_compat(manifest_text: &str) -> Result<DepTable, String> {
    let m: Manifest = parse_manifest_compat(manifest_text).map_err(|e| e.to_string())?;
    let mut t = dep_table_of(m.clone())?;
    if m.legacy_entry.lib.is_some() || !m.legacy_entry.libs.is_empty() {
        t.entries.retain(|e| e.mode != Mode::Impl);
        if let Some(lib) = &m.legacy_entry.lib {
            t.entries.push(EntrySource {
                path: bundle_key(lib)?,
                mode: Mode::Impl,
            });
        }
        for lib in &m.legacy_entry.libs {
            t.entries.push(EntrySource {
                path: bundle_key(lib)?,
                mode: Mode::Impl,
            });
        }
    }
    Ok(t)
}

fn dep_table_of(m: Manifest) -> Result<DepTable, String> {
    let mut entries = Vec::new();
    // the decl surface indexes in Decl mode; the body in Impl mode —
    // the same split `analysis::mode_of` speaks for open documents.
    // On a host pkg with no spelled surface, `src/mod.d.rut` under the
    // shell IS the root (the loader's default-root discovery
    // mirrored) — an unreadable row simply indexes nothing.
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
        namespace: m.namespace,
        consts: m.consts.into_iter().collect(),
        host: m.pkg_type == rut_driver::bundle::PkgType::Host,
        deps: rows(&m.deps),
        dev_deps: rows(&m.dev_deps),
    })
}

/// One source file unpacked from a `.rutbundle`: archive-relative
/// path, text, and the mode it indexes in (the manifest's entry key
/// decided — the authoritative spelling, not the `.d.rut` convention).
/// A mounted mod child rides as `<mod path>/mod.rut`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFile {
    pub path: String,
    pub src: String,
    pub mode: Mode,
}

/// A `.rutbundle`'s editable surface: the module's own name (the zip's
/// own `rut.jsonc` spells it) and its own source files. Compiled-only
/// bundles (no ridden source) yield zero files — the compiled groups
/// are the loader's truth, never guessed back into text. The unpack
/// law's ingredients ride beside: the archive's own manifest text
/// (laid down beside the files, the pkg's own rows mount through the
/// ordinary dir lane) and the embedded `deps/` entries (a source zip's
/// ride-along groups; a compiled bundle's ride-alongs are compiled —
/// never text, so always empty here).
#[derive(Debug, Clone, PartialEq)]
pub struct BundleIndex {
    pub module: String,
    pub namespace: Option<String>,
    pub consts: Vec<(String, f64)>,
    pub files: Vec<BundleFile>,
    /// the archive's own `rut.jsonc` text
    pub manifest: String,
    /// the embedded `deps/<identity>/` entries as (zip-relative path,
    /// UTF-8 text) — the unpack law lays them under the extracted tree
    /// and the pkg's own path rows resolve into them
    pub embedded: Vec<(String, String)>,
}

/// The archive's own `rut.mods` rows, when the bundle carries the
/// additive envelope section: the root text (`""`) mounted with its
/// child tree through rut-driver's rows codec (the same declaration-
/// driven mount a directory runs). `Ok(None)` = no rows entry — the
/// old flat envelope, which keeps loading byte-for-byte. The flavor
/// rides the pkg: a host tree speaks Decl and mounts `mod.d.rut`
/// children, a lib body's tree the Impl `mod.rut` shape.
fn bundle_rows(entries: &[(String, Vec<u8>)], host: bool) -> Result<Option<ModHead>, String> {
    let text = match read_entry(entries, rut_driver::mods::ROWS_NAME) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let (root, rows) = rut_driver::mods::parse_rows(&text)?;
    let (mode, child_file) = if host {
        (Mode::Decl, rut_driver::decl::DECL_CHILD_FILE)
    } else {
        (Mode::Impl, "mod.rut")
    };
    let mods = rut_driver::mods::mount_rows(&root, &rows, mode, child_file)?;
    Ok(Some((root, mods)))
}

/// The archive-entries child lookup for the source lane's mod mount —
/// the loader's `entry_child_lookup` case law mirrored over the
/// editor's decoded entries: the directory form `NAME/mod.rut` first,
/// else the single-file `NAME.rut`, both present the loud ambiguity.
/// A bundle is DIRECTORY-FORM only (loose file entries never mount as
/// modules), so the single-file spelling falling alone is the loud
/// missing error. Keys are bundle-normalized exactly like the
/// loader's.
fn entry_child_lookup(
    entries: &[(String, Vec<u8>)],
) -> impl FnMut(&str, &str) -> Result<rut_driver::ChildLookup, String> + '_ {
    move |parent: &str, name: &str| {
        let base = if parent.is_empty() {
            String::new()
        } else {
            format!("{parent}/")
        };
        let mod_key = bundle_key(&format!("{base}{name}/mod.rut"))?;
        let file_key = bundle_key(&format!("{base}{name}.rut"))?;
        let dir_text = read_entry(entries, &mod_key).ok();
        let has_file = read_entry(entries, &file_key).is_ok();
        if dir_text.is_some() && has_file {
            return Ok(rut_driver::ChildLookup::Ambiguous);
        }
        if let Some(text) = dir_text {
            return Ok(rut_driver::ChildLookup::Found {
                text,
                canon: mod_key,
                file: "mod.rut".to_string(),
            });
        }
        // the single-file entry alone: not a module here — bundles are
        // directory-form (the loud misses name the dir-form fix)
        if entries
            .iter()
            .any(|(k, _)| k.starts_with(&format!("{base}{name}/")))
        {
            return Ok(rut_driver::ChildLookup::NoModRut);
        }
        Ok(rut_driver::ChildLookup::Missing)
    }
}

/// Unpack a `.rutbundle`'s OWN sources: CRC-verified entries, the
/// archive's `rut.jsonc` naming the module, then the root module —
/// the `mod.rut` entry beside the manifest (an OLD published bundle
/// rides at its legacy `entry.lib` spelling, mapped by
/// [`dep_table_compat`] — the loader's reader-compat law mirrored) —
/// plus the `rut.mods` rows entry when the bundle carries
/// one (the additive envelope section: the tree as path-keyed rows),
/// else the file-entry mount the loader's source lane runs. Rode-along
/// dep groups (`<pkg>/…` prefixes) never ride — they are the loader's
/// business. A missing entry file (a compiled-only bundle whose source
/// did not ride) is skipped, not an error: the editor indexes what
/// exists. A declared child whose file is missing is the loud mount
/// error — declarations are the graph.
/// A SOURCE-ZIP cache entry's index — the source transport's editor
/// face: the zip's own package (`rut.jsonc` + `src/**`) indexes as
/// Impl files at their zip-relative paths; the embedded `deps/` trees
/// stay out of THIS pkg's index (a consumer's own dep rows cover them)
/// but ride home as [`BundleIndex::embedded`] — the unpack law lays
/// them under the extracted tree and the pkg's own path rows mount
/// them through the ordinary dir lane, first mount wins (the native
/// `mount_zip_dep` gate). UTF-8 entries only: the dir lane reads
/// `rut.jsonc` + `.rut` sources, and a non-text ride-along has no
/// editor face. The zip64/grammar/CRC laws ride
/// [`rut_driver::bundle::read_srczip`].
pub fn srczip_sources(bytes: &[u8]) -> Result<BundleIndex, String> {
    let entries = rut_driver::bundle::read_srczip(bytes).map_err(|e| e.to_string())?;
    let manifest_text = entries
        .iter()
        .find(|(n, _)| n == MANIFEST_NAME)
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .ok_or_else(|| {
            format!("no `{MANIFEST_NAME}` at the archive root — not a rut source zip")
        })?;
    let table = dep_table(&manifest_text)?;
    let module = table
        .name
        .clone()
        .ok_or_else(|| format!("`{MANIFEST_NAME}` has no `name`"))?;
    let mut files = Vec::new();
    let mut embedded = Vec::new();
    for (name, bytes) in &entries {
        if name.starts_with("deps/") {
            // the ride-along groups: text entries only, archive-
            // relative paths (the unpack law lays them verbatim)
            if let Ok(src) = std::str::from_utf8(bytes) {
                embedded.push((name.clone(), src.to_owned()));
            }
            continue;
        }
        if !name.starts_with("src/") || !name.ends_with(".rut") {
            continue; // the manifest, inert files — not this pkg's code
        }
        let src = String::from_utf8(bytes.clone()).map_err(|_| format!("{name}: not UTF-8"))?;
        files.push(BundleFile {
            path: name.clone(),
            src,
            mode: Mode::Impl,
        });
    }
    if files.is_empty() {
        return Err("the source zip carries no `src/**.rut` files".to_string());
    }
    Ok(BundleIndex {
        module,
        namespace: table.namespace,
        consts: table.consts,
        files,
        manifest: manifest_text,
        embedded,
    })
}

pub fn bundle_sources(bytes: &[u8]) -> Result<BundleIndex, String> {
    let entries = parse_bundle(bytes).map_err(|e| e.to_string())?;
    let manifest_text = read_entry(&entries, MANIFEST_NAME)
        .map_err(|_| format!("no `{MANIFEST_NAME}` entry — not a rut bundle"))?;
    let table = dep_table_compat(&manifest_text)?;
    let module = table
        .name
        .clone()
        .ok_or_else(|| format!("`{MANIFEST_NAME}` has no `name`"))?;
    let mut files = Vec::new();
    // the declaration entry, ONE per group (wire v12 — the `.d.ts`
    // lane): the manifest's spelled surface first (`entry.type` — a
    // host root's, the default `mod.d.rut` included), then the
    // archive's own `mod.d.rut` probe — a compiled group's GENERATED
    // entry, which the manifest never names (a compiled manifest spells
    // no entry keys). The entry indexes as ONE Decl file; the nesting
    // (the inline `mod` blocks) stamps the mod paths at index time.
    // Absent (old bundles, stripped packs) the ladder falls to the
    // binary's object-surface floor below.
    let mut decl_pushed = false;
    if let Some(e) = table.entries.iter().find(|e| e.mode == Mode::Decl) {
        if let Ok(src) = read_entry(&entries, &e.path) {
            files.push(BundleFile {
                path: e.path.clone(),
                src,
                mode: e.mode,
            });
            decl_pushed = true;
        }
    }
    if !decl_pushed {
        if let Ok(src) = read_entry(&entries, rut_driver::decl::DECL_CHILD_FILE) {
            files.push(BundleFile {
                path: rut_driver::decl::DECL_CHILD_FILE.to_string(),
                src,
                mode: Mode::Decl,
            });
            decl_pushed = true;
        }
    }
    // the tree: rows entry first (the additive envelope — the pack
    // writer's shape), else the file-entry mount over the root module.
    // The HOST flavor's root already rode the decl-surface push above
    // (the entry.type surface) — only its children join here, each a
    // Decl file at `<mod path>/mod.d.rut`; a lib tree's root re-rides
    // as `mod.rut` beside its Impl children.
    let tree: Option<(Option<BundleFile>, Vec<BundleFile>)> =
        match bundle_rows(&entries, table.host)? {
            Some((root_text, mods)) => {
                let (child_file, mode) = if table.host {
                    (rut_driver::decl::DECL_CHILD_FILE, Mode::Decl)
                } else {
                    ("mod.rut", Mode::Impl)
                };
                let children = mods
                    .into_iter()
                    .map(|(path, m)| BundleFile {
                        path: format!("{path}/{child_file}"),
                        src: m.text,
                        mode,
                    })
                    .collect();
                if table.host {
                    Some((None, children))
                } else {
                    Some((
                        Some(BundleFile {
                            path: "mod.rut".to_string(),
                            src: root_text,
                            mode: Mode::Impl,
                        }),
                        children,
                    ))
                }
            }
            None if table.host => None, // a host surface carries no lib body
            None => {
                // the source lane: root = the `mod.rut` entry (the
                // post-repeal convention), or the OLD envelope's legacy
                // lib — dep_table_compat mapped it onto the Impl entries.
                // The table row spells the DIRECTORY path (`src/mod.rut`);
                // the archive keeps the ONE canonical root name — the
                // miss translates (a compat row rides its legacy key and
                // reads directly)
                let root = match table.entries.iter().find(|e| e.mode == Mode::Impl) {
                    Some(e) => {
                        let read = read_entry(&entries, &e.path)
                            .map(|src| (e.path.clone(), src))
                            .or_else(|_| {
                                read_entry(&entries, "mod.rut")
                                    .map(|src| ("mod.rut".to_string(), src))
                            });
                        match read {
                            Ok((path, src)) => Some(BundleFile {
                                path,
                                src,
                                mode: e.mode,
                            }),
                            Err(_) => None,
                        }
                    }
                    None => match read_entry(&entries, "mod.rut") {
                        Ok(src) => Some(BundleFile {
                            path: "mod.rut".to_string(),
                            src,
                            mode: Mode::Impl,
                        }),
                        Err(_) => None,
                    },
                };
                match root {
                    Some(root) => {
                        let mounted = rut_driver::mount_mod_children(
                            &root.src,
                            &format!("\u{0}{module}"),
                            Mode::Impl,
                            "mod.rut",
                            &mut entry_child_lookup(&entries),
                        )?;
                        let children = mounted
                            .into_iter()
                            .map(|(path, m)| BundleFile {
                                path: format!("{path}/mod.rut"),
                                src: m.text,
                                mode: Mode::Impl,
                            })
                            .collect();
                        Some((Some(root), children))
                    }
                    None => None,
                }
            }
        };
    if let Some((root, children)) = tree {
        if let Some(root) = root {
            files.push(root);
        }
        files.extend(children);
    } else if !table.host {
        // no source rode — the delivery law (v22): a published lib
        // bundle is COMPILED-ONLY, its generic bodies riding the binary
        // as templates. The editor's answer is the DECLARATION ENTRY
        // when the group carries one (wire v12 — pushed above); the
        // floor beneath it is the binary's own DECLARATION FACE: the
        // decoded surface rendered back as Impl-mode source (classes
        // with fields, the inherent methods, the free fns — signatures
        // only, empty bodies), indexed under the canonical root name so
        // a use-line `Vec` lands on `bundle:<module>/mod.rut`. Decode
        // refuses torn bytes loudly.
        if !decl_pushed {
            if let Some((_, bin)) = entries.iter().find(|(n, _)| n == "mod.rutc") {
                match rut_core::binary::decode(bin) {
                    Ok(prog) => {
                        // the wire carries the ids and the blocks but not
                        // the packed bookkeeping — rebase with the identity
                        // scope map rebuilds it (the mount's own first half)
                        let prog = rut_core::link::rebase(prog, &|s| s);
                        files.push(BundleFile {
                            path: "mod.rut".to_string(),
                            src: decl_face(&prog),
                            mode: Mode::Impl,
                        });
                    }
                    Err(e) => return Err(format!("`mod.rutc`: {e}")),
                }
            }
        }
        // the remaining entry files still index, exactly as before
        for e in &table.entries {
            if e.mode != Mode::Impl {
                continue;
            }
            if let Ok(src) = read_entry(&entries, &e.path) {
                files.push(BundleFile {
                    path: e.path.clone(),
                    src,
                    mode: e.mode,
                });
            }
        }
    } else {
        // a host surface carries no lib body; the remaining entry files
        // still index, exactly as before
        for e in &table.entries {
            if e.mode != Mode::Impl {
                continue;
            }
            if let Ok(src) = read_entry(&entries, &e.path) {
                files.push(BundleFile {
                    path: e.path.clone(),
                    src,
                    mode: e.mode,
                });
            }
        }
    }
    Ok(BundleIndex {
        module,
        namespace: table.namespace,
        consts: table.consts,
        files,
        // a compiled bundle's ride-alongs are compiled groups — never
        // text — so the unpack ingredients stay empty here (the
        // loader's business, unchanged law)
        manifest: manifest_text,
        embedded: Vec::new(),
    })
}

/// The compiled bundle's DECLARATION FACE (v22 — the delivery law's
/// editor half): the decoded program's exported surface rendered back
/// as Impl-mode rut source — the exported classes with their fields,
/// the inherent method signatures (empty bodies — bodies are the link
/// lane's business, never source), and the exported free fns. The
/// interfaces ride as comments (the surface carries a generic COUNT,
/// not parameter names, so a faithful trait re-spelling is impossible
/// here); the consts ride [`BundleIndex::consts`] separately. This is
/// the indexed answer for a compiled-only url dep — a consumer's
/// `use pouch::{ Vec }` resolves against the bundle's own bytes.
fn decl_face(prog: &rut_core::binary::Program) -> String {
    use rut_core::types::TyKind;
    use rut_core::types::{TypeId, TY_NIL};
    let s = &prog.surface;
    let own = rut_core::link::own_scope(prog);
    let ty = |t: TypeId| -> String { ty_text(prog, t) };
    let name = |id: rut_core::sym::IdentId| -> String {
        prog.interner.name(id).trim_start_matches('#').to_string()
    };
    let mut out = String::from(
        "// the compiled bundle's declaration face (v22 — the delivery law):\n\
         // rendered from the binary's surface; the bodies are the link\n\
         // lane's business, never source.\n",
    );
    // the exported classes: fields from the type rows, methods from the
    // inherent surfaces (matched by the class's own type id)
    for t in &s.type_exports {
        if !t.is_class {
            continue;
        }
        let row_id = match t.scope {
            Some(sc) => rut_core::id::pack(sc, t.local),
            None => match own {
                Some(sc) => rut_core::id::pack(sc, t.local),
                None => continue,
            },
        };
        let gens = if t.params.is_empty() {
            String::new()
        } else {
            let ps: Vec<String> = t.params.iter().map(|&p| name(p)).collect();
            format!("<{}>", ps.join(", "))
        };
        out.push_str(&format!("\npub class {}{gens} {{\n", name(t.name)));
        if let TyKind::Data { fields } = prog.types.kind(row_id) {
            for f in fields {
                out.push_str(&format!("    {}: {};\n", name(f.name), ty(f.ty)));
            }
        }
        out.push_str("}\n");
        // the inherent methods — one impl block, the class's own
        // generics re-spelled
        if let Some(ih) = s.inherents.iter().find(|ih| ih.target == row_id) {
            let head_gens = gens.clone();
            out.push_str(&format!("\nimpl{head_gens} {}{gens} {{\n", name(t.name)));
            for m in &ih.methods {
                let mut ps: Vec<String> = Vec::with_capacity(m.params.len() + 1);
                if m.has_self {
                    ps.push("mut self".to_string());
                }
                for &p in m.params.iter() {
                    ps.push(format!("_: {}", ty(p)));
                }
                let ret = if m.ret == TY_NIL {
                    String::new()
                } else {
                    format!(" -> {}", ty(m.ret))
                };
                let mgens = if m.generics.is_empty() {
                    String::new()
                } else {
                    let gs: Vec<String> = m.generics.iter().map(|&g| name(g)).collect();
                    format!("<{}>", gs.join(", "))
                };
                let marker = match m.marker {
                    rut_core::binary::MARKER_NONE => String::new(),
                    rut_core::binary::MARKER_DISPOSAL => " /* [disposal] */".to_string(),
                    rut_core::binary::MARKER_ITERABLE => " /* [iterable] */".to_string(),
                    rut_core::binary::MARKER_CONSTRUCTOR => " /* [constructor] */".to_string(),
                    _ => String::new(),
                };
                out.push_str(&format!(
                    "    pub fn {}{mgens}({}){} {{ }}{marker}\n",
                    name(m.name),
                    ps.join(", "),
                    ret
                ));
            }
            out.push_str("}\n");
        }
    }
    // the free fns (ordinary exports only — host rows are decl/native
    // surface, never a compiled lib's face)
    for f in &s.funcs {
        if f.host.is_some() {
            continue;
        }
        let ps: Vec<String> = f
            .params
            .iter()
            .enumerate()
            .map(|(i, &p)| format!("_{}: {}", i, ty(p)))
            .collect();
        let ret = if f.ret == TY_NIL {
            String::new()
        } else {
            format!(" -> {}", ty(f.ret))
        };
        let kw = if f.is_async { "pub async fn" } else { "pub fn" };
        out.push_str(&format!(
            "\n{kw} {}({}){ret} {{ }}\n",
            name(f.name),
            ps.join(", ")
        ));
    }
    // the interfaces, as comments — the surface carries a generics
    // COUNT, not parameter names, so a faithful trait re-spelling is
    // impossible here; the names still document the surface
    for i in &s.ifaces {
        let ms: Vec<String> = i
            .methods
            .iter()
            .map(|m| prog.interner.name(m.name).to_string())
            .collect();
        out.push_str(&format!(
            "// trait {}<{} generic params>: {}\n",
            prog.interner.name(i.name),
            i.generics,
            ms.join(", ")
        ));
    }
    out
}

/// One type id as rut source text, over the decoded program's own
/// tables — boot ids answer through the shared boot prefix, carried
/// rows name through the program interner, placeholder rows (`#T`)
/// render as their parameter spelling (`T`).
fn ty_text(prog: &rut_core::binary::Program, t: rut_core::types::TypeId) -> String {
    use rut_core::types::TyKind;
    let raw_name = |id: rut_core::types::TypeId| -> String {
        prog.interner.name(prog.types.type_at(id).name).to_string()
    };
    let name = |id: rut_core::types::TypeId| -> String {
        raw_name(id).trim_start_matches('#').to_string()
    };
    match prog.types.kind(t) {
        TyKind::Nil => "nil".to_string(),
        TyKind::Prim(p) => format!("{p:?}").to_lowercase(),
        TyKind::Str => "str".to_string(),
        TyKind::Bytes => "bytes".to_string(),
        TyKind::Array { elem } => format!("[{}]", ty_text(prog, *elem)),
        TyKind::Enum { .. }
        | TyKind::Data { .. }
        | TyKind::IfaceObj { .. }
        | TyKind::Trace
        | TyKind::DisposalContext => name(t),
        TyKind::Opaque => "opaque".to_string(),
        TyKind::Opt { elem } => format!("?{}", ty_text(prog, *elem)),
        TyKind::Weak { elem } => format!("Weak<{}>", ty_text(prog, *elem)),
        TyKind::Future { elem } => format!("Future<{}>", ty_text(prog, *elem)),
        TyKind::Fn { params, ret } => {
            let ps: Vec<String> = params.iter().map(|&p| ty_text(prog, p)).collect();
            format!("fn({}) -> {}", ps.join(", "), ty_text(prog, *ret))
        }
    }
}

/// a bundle file's mod path: the rows lane's root rides as `mod.rut`
/// (`Some("")`), a mounted child as `<path>/mod.rut` (a host tree's
/// as `<path>/mod.d.rut`), the host default root as `mod.d.rut`
/// (`Some("")` — the twin of the lib root's spelling) — `Some(path)`;
/// anything else is today's flat shape (`None`) — shared by both faces
/// (the stdio walk and the wasm shim index identically)
pub fn bundle_file_mod_path(path: &str) -> Option<String> {
    if let Some(p) = path.strip_suffix("/mod.rut") {
        return Some(p.to_string());
    }
    if let Some(p) = path.strip_suffix(format!("/{}", rut_driver::decl::DECL_CHILD_FILE).as_str()) {
        return Some(p.to_string());
    }
    if path == "mod.rut" || path == rut_driver::decl::DECL_CHILD_FILE {
        return Some(String::new());
    }
    None
}

/// Index one dep source file: the ordinary [`analysis::index_at`] pass
/// over `src` (origin stamped with `uri`, mode per the manifest), the
/// module NAMED, and the manifest's `namespace`/`consts` minted on —
/// a `TyForm::Namespace` type row for the head (the module's free fns
/// as methods, the consts as fields: the qualified `Math::sqrt`/`Math::PI`
/// shape resolves through the member machinery), module-scope lets for
/// the consts (the bare `PI` form the compiler binds as an extern
/// const once the pkg is used).
pub fn index_dep(
    name: &str,
    uri: &str,
    src: &str,
    mode: Mode,
    namespace: Option<&str>,
    consts: &[(String, f64)],
) -> DefIndex {
    // the ordinary index pass — but the MODE comes from the manifest's
    // entry key (the authoritative spelling), not the `.d.rut`
    // filename convention `analysis::mode_of` speaks for open documents
    let normalized = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&normalized);
    let (ast, _) = rut_parser::parse(&normalized, mode);
    let mut idx = crate::hover::index(&normalized, &ast, &toks);
    idx.origin = uri.to_string();
    idx.module = Some(name.to_string());
    // the parse mode rides the index — the definition layer's
    // body-over-decl preference reads it (the manifest's entry key is
    // the authoritative spelling, not the `.d.rut` filename convention
    // `analysis::mode_of` speaks for open documents)
    idx.mode = Some(mode);
    mint_pkg_rows(&mut idx, namespace, consts);
    idx
}

/// [`index_dep`] with the file's mod path stamped — one mounted
/// module of a mod-carrying dep (the dep walk / bundle loader call
/// this per file; `mod_path` `""` is the pkg root's `mod.rut`).
pub fn index_dep_mod(
    name: &str,
    uri: &str,
    mod_path: &str,
    src: &str,
    mode: Mode,
    namespace: Option<&str>,
    consts: &[(String, f64)],
) -> DefIndex {
    let mut idx = index_dep(name, uri, src, mode, namespace, consts);
    idx.mod_path = Some(mod_path.to_string());
    idx
}

/// One declaration entry's module indexes — the wire-v12 consumption
/// of a group's consolidated `mod.d.rut`: ONE Decl parse, and one
/// `(mod path, index)` pair per nested inline module, each stamped with
/// the mod path its nesting spells. Structure lives in the grammar,
/// never in paths: the file is one, the modules many, the parse one.
/// `mod_path` prefixes every stamp (the file's own position — `""` for
/// a group root). The namespace head and the consts mint on the
/// pkg-root module only (they are the MANIFEST's rows, the pkg's own).
pub fn index_dep_decl_group(
    name: &str,
    uri: &str,
    mod_path: &str,
    src: &str,
    namespace: Option<&str>,
    consts: &[(String, f64)],
) -> Vec<(String, DefIndex)> {
    // the ordinary parse once — the mode is the caller's (Decl: the
    // declaration entry lane)
    let normalized = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&normalized);
    let (ast, _) = rut_parser::parse(&normalized, Mode::Decl);
    // the module slices: the root's items, then each inline module's,
    // path-stamped, first-wins (the loader's own edge law)
    let mut modules: Vec<(String, Vec<rut_ast::ast::NodeHandle<rut_ast::ast::AnyItem>>)> =
        vec![(String::new(), ast.module_items(ast.root).to_vec())];
    collect_inline_modules(&ast, ast.module_items(ast.root), mod_path, &mut modules);
    modules
        .into_iter()
        .map(|(path, items)| {
            let mut idx = crate::hover::index_items(&normalized, &ast, &toks, &items);
            // the entry uri for the file's own module; a `#<mod path>`
            // suffix for the nested ones — one index per module, one
            // origin each (the faces' defs store dedups by origin), the
            // (module, mod_path) pair remaining the real key
            idx.origin = if path == mod_path {
                uri.to_string()
            } else {
                format!("{uri}#{path}")
            };
            idx.module = Some(name.to_string());
            // the parse mode rides the index — the definition layer's
            // body-over-decl preference reads it
            idx.mode = Some(Mode::Decl);
            idx.mod_path = Some(path.clone());
            // the manifest's head rows mint on the pkg root only
            if path == mod_path {
                mint_pkg_rows(&mut idx, namespace, consts);
            }
            (path, idx)
        })
        .collect()
}

/// The inline modules of one parsed item list, depth-first, path-stamped
/// under `prefix`. First-wins: a repeated inline edge never mounts
/// twice.
fn collect_inline_modules(
    ast: &rut_ast::ast::Ast,
    items: &[rut_ast::ast::NodeHandle<rut_ast::ast::AnyItem>],
    prefix: &str,
    out: &mut Vec<(String, Vec<rut_ast::ast::NodeHandle<rut_ast::ast::AnyItem>>)>,
) {
    for &it in items {
        let rut_ast::ast::ItemKind::ModInline {
            name,
            items: nested,
            ..
        } = ast.item(it)
        else {
            continue;
        };
        let name = ast.name(*name).to_string();
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if out.iter().any(|(p, _)| *p == path) {
            continue; // first mount wins
        }
        out.push((path.clone(), nested.clone()));
        collect_inline_modules(ast, nested, &path, out);
    }
}

/// The namespace head + consts, minted onto one dep index (the
/// qualified-access rows — [`index_dep`]'s tail, shared with the
/// declaration-entry splitter).
fn mint_pkg_rows(idx: &mut DefIndex, namespace: Option<&str>, consts: &[(String, f64)]) {
    if let Some(head) = namespace {
        // the module's free fns become the head's methods — the
        // qualified-access surface the engine routes by the bound
        // namespace
        let methods: Vec<MemberSrc> = idx
            .fns
            .iter()
            .filter(|f| f.owner.is_none())
            .map(|f| MemberSrc {
                name: f.name.clone(),
                src: f.src.clone(),
                ty: f.ret.clone(),
                name_span: f.name_span,
                doc: f.doc.clone(),
                line: f.line,
            })
            .collect();
        let fields: Vec<MemberSrc> = consts
            .iter()
            .map(|(n, v)| MemberSrc {
                name: n.clone(),
                src: format!("{n} = {:?}", v),
                ty: Some("f64".to_string()),
                name_span: None,
                doc: vec!["manifest const — compiler-materialized".to_string()],
                line: 1,
            })
            .collect();
        let module = idx.module.clone().unwrap_or_default();
        idx.types.push(TyDef {
            name: head.to_string(),
            name_span: None,
            form: TyForm::Namespace,
            vis: Vis::Pub,
            is_pub: true,
            ambient: false,
            generics: Vec::new(),
            fields,
            methods,
            alias_target: None,
            doc: vec![format!(
                "the module's namespace head — `use {}::{{ {head} }}` binds it; members are reached qualified",
                module
            )],
            span: Span::new(0, 0),
            line: 1,
        });
    }
    for (n, v) in consts {
        idx.lets.push(crate::hover::LetDef {
            name: n.clone(),
            name_span: None,
            ty: Some("f64".to_string()),
            src: format!("let {n} = {:?}", v),
            doc: vec!["manifest const — compiler-materialized".to_string()],
            vis: Vis::Pub,
            span: Span::new(0, 0),
            line: 1,
        });
    }
}
