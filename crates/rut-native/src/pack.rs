//! The pack lanes — a module directory (+ its whole `deps` closure) →
//! one deterministic `.rutbundle`. The FS half lives here: the root
//! manifest bytes, the `[deps]` walk, the dev-table mounts (compile
//! once per owner), the group-source collection, and the file reader
//! the emission reads through. The COMPILE + EMIT half is the
//! driver's pure [`rut_driver::pack::pack`] over the assembled world.
//! Writing the output file stays with the caller (the CLI). Same
//! input directory ⇒ byte-identical bundle.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rut_driver::bundle::{
    parse_manifest, read_srczip, rewrite_for_zip, write_bundle, Manifest, PkgType,
};
use rut_driver::pack::{PackError, PackOpts, PackWorld, PkgSource};

use crate::source::{FsSource, Source};
use crate::walk::{mount_dev_table, read_manifest, walk_dir, Archive, World, MANIFEST_RETIRED};

/// What the root becomes. `Source` (the default) is the source zip —
/// the directory grammar over the wire: `rut.jsonc` + `src/` +
/// `deps/<name>@<fp16>/` trees and IR files, everything embedded, the
/// manifest rewritten so every row points inside. `Bin` is the
/// rutbundle lane — the root compiled, the closure's groups riding as
/// `.rutc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackMode {
    Source,
    Bin,
}

/// The conventional pack output: `<dirname>.zip` for a source pack,
/// `<dirname>.rutbundle` for a bin pack — a sibling of the directory
/// (`demo/mod` → `demo/mod.zip`). A relative spell that has no
/// basename (`.`) resolves first, so the artifact names the directory
/// (`todolist.zip`), never a dot-shape.
pub fn default_out_path(dir: &Path, mode: PackMode) -> std::path::PathBuf {
    let ext = match mode {
        PackMode::Source => "zip",
        PackMode::Bin => "rutbundle",
    };
    let named = |d: &Path| -> Option<std::path::PathBuf> {
        let stem = d.file_name()?.to_string_lossy();
        Some(d.with_file_name(format!("{stem}.{ext}")))
    };
    named(dir)
        .or_else(|| std::fs::canonicalize(dir).ok().and_then(|d| named(&d)))
        .unwrap_or_else(|| dir.with_file_name(format!("out.{ext}")))
}

/// Pack a module directory into a deterministic `.rutbundle`.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, PackError> {
    Ok(pack_dir_opts(dir, &PackOpts::default())?.0)
}

/// [`pack_dir`] over pre-fetched url bytes — the fetched core the
/// `*_with` lane calls after awaiting the prefetch. The root's
/// `rut.lock` is read from disk (the realistic lane — a fixture that
/// fetches is a project that installed) and the url rows' pins are
/// verified at the mount door; a url row absent from the map is the
/// loud no-fetcher error.
pub fn pack_dir_fetched(dir: &Path, map: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, PackError> {
    Ok(pack_dir_opts_fetched(dir, &PackOpts::default(), map)?.0)
}

/// [`pack_dir`] with options. Returns the bundle bytes and — under
/// `strip` — the serialized symbol table. Deterministic end to end:
/// same input dir ⇒ byte-identical bundle (mangling is sorted-union
/// based) ⇒ byte-identical symtab. Url rows refuse here: pass a
/// fetcher (`pack_dir_opts_with`) or pre-fetch a map.
pub fn pack_dir_opts(dir: &Path, opts: &PackOpts) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    pack_dir_opts_fetched(dir, opts, &BTreeMap::new())
}

/// [`pack_dir_opts`] with a url-dep fetcher (the CLI's `--strip` lane).
pub async fn pack_dir_opts_with(
    dir: &Path,
    opts: &PackOpts,
    fetch: &dyn DepRemote,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    let src = FsSource::at(dir);
    let lock = crate::walk::read_lock(&src, dir.to_string_lossy().as_ref())
        .map_err(rut_driver::pack::PackError::from)?;
    let map = crate::walk::prefetch_urls(&src, dir.to_string_lossy().as_ref(), &lock, fetch)
        .await
        .map_err(rut_driver::pack::PackError::from)?;
    pack_dir_opts_locked(dir, opts, &map, &lock)
}

use crate::remote::DepRemote;

// ---------------------------------------------------------------------
// the source-zip lane (`--mode source`, the default)
// ---------------------------------------------------------------------

/// Pack a module directory into a deterministic SOURCE ZIP — the
/// directory grammar over the wire: `rut.jsonc` (rewritten: every dep
/// row points inside at `deps/<name>@<fp16>`, the wire rows drop) +
/// `src/**` + every dep embedded under `deps/` — a `path` dep or a
/// `.zip` url dep as its own self-contained tree, a `.rutbundle` url
/// dep as its IR file verbatim. Url bytes come from the fetched map
/// (pack never downloads); the closure walk runs first, so a tree that
/// cannot mount refuses here too.
pub fn pack_dir_zip(dir: &Path) -> Result<Vec<u8>, PackError> {
    pack_dir_zip_fetched(dir, &BTreeMap::new())
}

/// [`pack_dir_zip`] with a url-dep fetcher — the CLI's lane.
pub async fn pack_dir_zip_with(dir: &Path, fetch: &dyn DepRemote) -> Result<Vec<u8>, PackError> {
    let src = FsSource::at(dir);
    let lock = crate::walk::read_lock(&src, dir.to_string_lossy().as_ref())
        .map_err(rut_driver::pack::PackError::from)?;
    let map = crate::walk::prefetch_urls(&src, dir.to_string_lossy().as_ref(), &lock, fetch)
        .await
        .map_err(rut_driver::pack::PackError::from)?;
    pack_dir_zip_locked(dir, &map, &lock)
}

/// [`pack_dir_zip`] over pre-fetched url bytes.
pub fn pack_dir_zip_fetched(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, PackError> {
    let src = FsSource::at(dir);
    let lock = crate::walk::read_lock(&src, dir.to_string_lossy().as_ref())
        .map_err(rut_driver::pack::PackError::from)?;
    pack_dir_zip_locked(dir, map, &lock)
}

/// The zip lane's core: the closure walk (mounts + validates
/// everything the zip embeds), then the post-order collector — a
/// node's fingerprint covers its REWRITTEN manifest, which embeds its
/// deps' identities, so deps fingerprint first.
fn pack_dir_zip_locked(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
    lock: &BTreeMap<String, String>,
) -> Result<Vec<u8>, PackError> {
    let manifest_path = dir.join(rut_driver::bundle::MANIFEST_NAME);
    let manifest_bytes = match std::fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(e) if std::fs::read(dir.join(MANIFEST_RETIRED)).is_ok() => {
            let _ = e;
            return Err(PackError::law(format!(
                "{} found — the manifest is `{}` (JSONC: comments and trailing commas legal) \
                 since wire 9; re-name the file or re-pack the directory",
                dir.join(MANIFEST_RETIRED).display(),
                rut_driver::bundle::MANIFEST_NAME,
            )));
        }
        Err(e) => return Err(PackError::io(manifest_path.display(), e)),
    };
    let manifest_text = String::from_utf8(manifest_bytes)
        .map_err(|_| PackError::law(format!("{}: not UTF-8", manifest_path.display())))?;
    let manifest = parse_manifest(&manifest_text)?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| PackError::law(format!("{} has no `name`", manifest_path.display())))?;
    let src = FsSource::at(dir);
    let dir_key = dir.to_string_lossy().into_owned();
    let mut world = World::new();
    let root = walk_dir(&mut world, &src, &dir_key, map, lock)
        .map_err(rut_driver::pack::PackError::from)?;
    if root != name {
        return Err(PackError::law(format!(
            "{} names itself `{root}` — expected `{name}`",
            manifest_path.display()
        )));
    }
    let mut memo: BTreeMap<String, StagedNode> = BTreeMap::new();
    let staged = collect_tree(
        dir,
        &dir_key,
        &manifest_text,
        &name,
        &manifest,
        &src,
        &world,
        map,
        &mut memo,
    )?;
    // the entry order: `rut.jsonc` first, everything else sorted —
    // the deterministic face of the archive
    let mut entries = staged.entries;
    entries.sort_by(|a, b| {
        let a_root = a.0 == rut_driver::bundle::MANIFEST_NAME;
        let b_root = b.0 == rut_driver::bundle::MANIFEST_NAME;
        b_root.cmp(&a_root).then_with(|| a.0.cmp(&b.0))
    });
    rut_driver::bundle::write_srczip(&entries).map_err(|e| PackError::law(e.to_string()))
}

/// One collected package's staged shape: its entries relative to the
/// package root (`rut.jsonc`, `src/**`, `deps/<ident>/…`) and its tree
/// fingerprint — the REWRITTEN manifest plus the `src/` sources, so
/// the same closure content lands at the same identity wherever it
/// embeds.
#[derive(Clone)]
struct StagedNode {
    entries: Vec<(String, Vec<u8>)>,
    ident: String,
}

/// Collect one package's staged tree, post-order: its deps first
/// (the rewritten manifest embeds their identities), then the
/// rewrite, the fingerprint, the validation gates.
#[allow(clippy::too_many_arguments)]
fn collect_tree(
    dir: &Path,
    dir_key: &str,
    manifest_text: &str,
    mount_key: &str,
    manifest: &Manifest,
    src: &FsSource,
    world: &World,
    map: &BTreeMap<String, Vec<u8>>,
    memo: &mut BTreeMap<String, StagedNode>,
) -> Result<StagedNode, PackError> {
    let canon = std::fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .into_owned();
    if let Some(hit) = memo.get(&canon) {
        return Ok(StagedNode {
            entries: hit.entries.clone(),
            ident: hit.ident.clone(),
        });
    }
    // the deps, post-order
    let mut rows: BTreeMap<String, String> = BTreeMap::new();
    let mut embedded: Vec<(String, Vec<u8>)> = Vec::new();
    for (spec, desc) in &manifest.deps {
        let node = if let Some(url) = desc.get("url") {
            collect_url_node(spec, url, map)?
        } else if let Some(rel) = desc.get("path") {
            let dep_dir_key = src.resolve(dir_key, rel).map_err(PackError::law)?;
            let dep_dir = PathBuf::from(&dep_dir_key);
            let dep_manifest_path = dep_dir.join(rut_driver::bundle::MANIFEST_NAME);
            let dep_text = String::from_utf8(
                std::fs::read(&dep_manifest_path)
                    .map_err(|e| PackError::io(dep_manifest_path.display(), e))?,
            )
            .map_err(|_| PackError::law(format!("{}: not UTF-8", dep_manifest_path.display())))?;
            let dep_manifest = parse_manifest(&dep_text)?;
            let dep_mount_key = crate::walk::mount_key_for(&dep_dir_key, spec);
            collect_tree(
                &dep_dir,
                &dep_dir_key,
                &dep_text,
                &dep_mount_key,
                &dep_manifest,
                src,
                world,
                map,
                memo,
            )?
        } else {
            return Err(PackError::law(format!(
                "dep `{spec}` has no `path` or `url`"
            )));
        };
        rows.insert(spec.clone(), format!("deps/{}", node.ident));
        for (n, b) in node.entries {
            embedded.push((format!("deps/{}/{}", node.ident, n), b));
        }
    }
    // the tree files: everything under `src/`, dot entries skipped
    let src_files = collect_src_files(dir)?;
    // the validation gates: no `.rut` outside `src/`; nothing under
    // `src/` the mount never reads
    let mounted = world.pkgs.get(mount_key).ok_or_else(|| {
        PackError::law(format!(
            "dep `{mount_key}` walked but never mounted — the closure and the world disagree"
        ))
    })?;
    validate_tree(dir, mounted, manifest)?;
    // the rewrite: every row points inside, the wire rows drop
    let rewritten =
        rewrite_for_zip(manifest_text, &rows).map_err(|e| PackError::law(e.to_string()))?;
    // the fingerprint: the rewritten manifest + the `src/` `.rut`
    // sources — content-derived, context-free
    let mut fp_files: Vec<(String, Vec<u8>)> = vec![(
        rut_driver::bundle::MANIFEST_NAME.to_string(),
        rewritten.clone().into_bytes(),
    )];
    for (rel, bytes) in &src_files {
        if rel.ends_with(".rut") {
            fp_files.push((rel.clone(), bytes.clone()));
        }
    }
    let ident = format!(
        "{}@{}",
        mount_key,
        rut_driver::bundle::fp16_of_tree(&fp_files)
    );
    let mut entries = vec![(
        rut_driver::bundle::MANIFEST_NAME.to_string(),
        rewritten.into_bytes(),
    )];
    entries.extend(src_files);
    entries.extend(embedded);
    let node = StagedNode { entries, ident };
    memo.insert(canon, node.clone());
    Ok(node)
}

/// A fetched url dep's staged node: a `.zip` rides VERBATIM (its
/// extracted entries — its own rewritten rows resolve inside its
/// subtree), its fingerprint over its own `rut.jsonc` + `src/`
/// sources; a `.rutbundle` rides as its IR file, named by the wire
/// fingerprint of the bundle's root.
fn collect_url_node(
    spec: &str,
    url: &str,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<StagedNode, PackError> {
    let bytes = map.get(url).ok_or_else(|| {
        PackError::law(format!(
            "dep `{spec}` is declared by url ({url}) — the bytes are not in the fetched map; \
             rut pack never downloads, run `rut install` first"
        ))
    })?;
    if url
        .rsplit('.')
        .next()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        let entries = read_srczip(bytes).map_err(|e| PackError::law(format!("{url}: {e}")))?;
        let manifest_text = entries
            .iter()
            .find(|(n, _)| n == rut_driver::bundle::MANIFEST_NAME)
            .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
            .ok_or_else(|| {
                PackError::law(format!(
                    "{url}: no `{}` at the archive root",
                    rut_driver::bundle::MANIFEST_NAME
                ))
            })?;
        let dm = parse_manifest(&manifest_text)?;
        let pkg_name = dm
            .name
            .ok_or_else(|| PackError::law(format!("{url}: rut.jsonc has no `name`")))?;
        let own: Vec<(String, Vec<u8>)> = entries
            .iter()
            .filter(|(n, _)| {
                n == rut_driver::bundle::MANIFEST_NAME
                    || (n.starts_with("src/") && n.ends_with(".rut"))
            })
            .cloned()
            .collect();
        let fp = rut_driver::bundle::fp16_of_tree(&own);
        Ok(StagedNode {
            entries,
            ident: rut_driver::bundle::identity(&pkg_name, &fp),
        })
    } else {
        // IR verbatim: the bundle's own wire fingerprint names the dir
        let bundle_entries = rut_driver::bundle::parse_bundle(bytes)
            .map_err(|e| PackError::law(format!("{url}: {e}")))?;
        let manifest_text =
            rut_driver::bundle::read_entry(&bundle_entries, rut_driver::bundle::MANIFEST_NAME)
                .map_err(|e| PackError::law(format!("{url}: {e}")))?;
        let dm = parse_manifest(&manifest_text)?;
        let pkg_name = dm
            .name
            .ok_or_else(|| PackError::law(format!("{url}: rut.jsonc has no `name`")))?;
        let fp = rut_driver::bundle::fp16_of(&bundle_entries, "");
        Ok(StagedNode {
            entries: vec![(format!("{pkg_name}.rutbundle"), bytes.clone())],
            ident: rut_driver::bundle::identity(&pkg_name, &fp),
        })
    }
}

/// Every file under `dir/src/`, relative to `dir`, dot entries
/// skipped — the tree a source zip rides.
fn collect_src_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, PackError> {
    let src = dir.join("src");
    let mut out = Vec::new();
    collect_files_under(&src, dir, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn collect_files_under(
    at: &Path,
    root: &Path,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), PackError> {
    let entries = std::fs::read_dir(at).map_err(|e| PackError::io(at.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| PackError::io(at.display(), e))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_files_under(&path, root, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let bytes = std::fs::read(&path).map_err(|e| PackError::io(path.display(), e))?;
            out.push((rel, bytes));
        }
    }
    Ok(())
}

/// The pack-time tree laws (both modes' worth of hygiene, one gate):
/// no `.rut` file outside the `src/` shell (a nested package root — a
/// dir with its own `rut.jsonc` — is not this pkg's code and skips),
/// and nothing under `src/` the mount never reads — an undeclared
/// `.rut` is either dead (delete it) or a mistake (declare it), never
/// a silent ride-along.
fn validate_tree(
    dir: &Path,
    mounted: &rut_driver::Mounted,
    manifest: &Manifest,
) -> Result<(), PackError> {
    // every file under the dir, dot entries skipped
    let mut all = Vec::new();
    collect_files_under(dir, dir, &mut all)?;
    // nested package roots (a dir with its own `rut.jsonc`) are their
    // own business — their trees drop from this pkg's gates entirely
    let nested: Vec<String> = all
        .iter()
        .filter_map(|(rel, _)| {
            rel.strip_suffix(rut_driver::bundle::MANIFEST_NAME)
                .filter(|prefix| !prefix.is_empty())
                .map(str::to_string)
        })
        .collect();
    let own: Vec<&(String, Vec<u8>)> = all
        .iter()
        .filter(|(rel, _)| {
            rel == rut_driver::bundle::MANIFEST_NAME
                || !nested.iter().any(|prefix| rel.starts_with(prefix))
        })
        .collect();
    // gate A — no `.rut` outside the `src/` shell
    for (rel, _) in &own {
        if rel.ends_with(".rut") && !rel.starts_with("src/") {
            return Err(PackError::law(format!(
                "{}: rut code rides only under a `src/` shell — declare it (`mod …;`) and move \
                 it under `src/`, or delete it",
                dir.join(rel).display()
            )));
        }
    }
    // gate B — every `src/**.rut` is mounted: the root file, the
    // spelled surface, or a declared child's file
    let surface_rel = manifest
        .entry
        .type_path
        .as_deref()
        .map(rut_driver::bundle::src_rel);
    for (rel, _) in &own {
        if !rel.starts_with("src/") || !rel.ends_with(".rut") {
            continue;
        }
        let src_rel = &rel["src/".len()..];
        let declared = src_rel == "mod.rut"
            || src_rel == "mod.d.rut"
            || surface_rel.as_deref() == Some(format!("src/{src_rel}").as_str())
            || mounted.mods.contains_key(derive_mod_path(src_rel));
        if !declared {
            return Err(PackError::law(format!(
                "{}: nothing declares this module — `mod …;` it from its parent or delete it",
                dir.join(rel).display()
            )));
        }
    }
    Ok(())
}

/// The mod path a `src/`-relative module file mounts as:
/// `mod.rut`/`mod.d.rut` is the root, `a/mod.rut` (or `a/mod.d.rut`)
/// the `a` module, `a.rut` the single-file `a`.
fn derive_mod_path(src_rel: &str) -> &str {
    src_rel
        .strip_suffix("/mod.rut")
        .or_else(|| src_rel.strip_suffix("/mod.d.rut"))
        .or_else(|| src_rel.strip_suffix(".rut"))
        .unwrap_or(src_rel)
}

/// The pack lane's fetched core: read the root's `rut.lock` from disk
/// (the realistic lane — the walk's mount door verifies against it),
/// then the manifest's gates, the walk, the dev tables — then the
/// driver's pure pack over the assembled world.
pub fn pack_dir_opts_fetched(
    dir: &Path,
    opts: &PackOpts,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    let src = FsSource::at(dir);
    let lock = crate::walk::read_lock(&src, dir.to_string_lossy().as_ref())
        .map_err(rut_driver::pack::PackError::from)?;
    pack_dir_opts_locked(dir, opts, map, &lock)
}

/// [`pack_dir_opts_fetched`] with the lock read — the pack lanes'
/// resolved core (no separate pub resolved name: the pack has no
/// install-side caller yet). The root manifest's gates, the walk, the
/// dev tables — then the driver's pure pack over the assembled world.
fn pack_dir_opts_locked(
    dir: &Path,
    opts: &PackOpts,
    map: &BTreeMap<String, Vec<u8>>,
    lock: &BTreeMap<String, String>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    let manifest_path = dir.join(rut_driver::bundle::MANIFEST_NAME);
    let manifest_bytes = match std::fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(e) if std::fs::read(dir.join(MANIFEST_RETIRED)).is_ok() => {
            // the retired manifest name: the pointed cutover refusal —
            // no fallback lane reads it
            let _ = e;
            return Err(PackError::law(format!(
                "{} found — the manifest is `{}` (JSONC: comments and trailing commas legal) \
                 since wire 9; re-name the file or re-pack the directory",
                dir.join(MANIFEST_RETIRED).display(),
                rut_driver::bundle::MANIFEST_NAME,
            )));
        }
        Err(e) => return Err(PackError::io(manifest_path.display(), e)),
    };
    let manifest = parse_manifest(
        &String::from_utf8(manifest_bytes.clone())
            .map_err(|_| PackError::law(format!("{}: not UTF-8", manifest_path.display())))?,
    )?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| PackError::law(format!("{} has no `name`", manifest_path.display())))?;
    // the packed rut.jsonc is the directory's rut.jsonc — with the
    // wire rows ENSURED: a source-authored manifest (no
    // `format`/`format_version`) gains them at the pack door, because
    // the bundle reader refuses without them and the same project
    // should publish both ways without hand-editing. Everything else
    // rides byte-for-byte (the law).
    let want_version = rut_driver::pack::FORMAT_VERSION;
    let (manifest, manifest_bytes) = if manifest.format.as_deref() != Some("rutbundle")
        || manifest.format_version != Some(want_version)
    {
        let staged = ensure_wire_rows(
            &String::from_utf8(manifest_bytes.clone())
                .map_err(|_| PackError::law(format!("{}: not UTF-8", manifest_path.display())))?,
            want_version,
        )?;
        let staged_manifest = parse_manifest(&staged)?;
        if staged_manifest.name != manifest.name {
            return Err(PackError::law(format!(
                "{}: staging the wire rows moved the `name` — normalize the manifest",
                manifest_path.display()
            )));
        }
        (staged_manifest, staged.into_bytes())
    } else {
        (manifest, manifest_bytes)
    };
    // the host-root arm: no closure (the grammar refuses a host
    // manifest's deps tables), no compile walk — the surface verifies
    // by parsing + lowering once, then rides as source
    if manifest.pkg_type == PkgType::Host {
        if opts.strip {
            return Err(PackError::law(
                "a host bundle has no symbols to strip — its root is a declaration surface, \
                 not a program; there is no binary and no sidecar",
            ));
        }
        // the wire rows are ENSURED above (the gate covers host
        // manifests too) — the host arm rides the staged bytes as-is
        return pack_host_root(dir, &manifest, manifest_bytes).map(|bytes| (bytes, None));
    }
    // the closure's world: the [deps] walk + the dev pass
    let src = FsSource::at(dir);
    let dir_key = dir.to_string_lossy().into_owned();
    let mut world = World::new();
    let root = walk_dir(&mut world, &src, &dir_key, map, lock)
        .map_err(rut_driver::pack::PackError::from)?;
    if root != name {
        return Err(PackError::law(format!(
            "{} names itself `{root}` — expected `{name}`",
            manifest_path.display()
        )));
    }
    // the closure's group set: dir deps as directories, url deps as
    // their archive's locations (enumerated closed — no recursion)
    let sources = collect_group_sources(&src, &dir_key, &manifest, &world)?;
    // compile once per owner: every DIR dep group's unit is the same
    // bytes wherever it is packed, so each one's own `[dev-deps]` mount
    // too — its unit compiles over the same closure wherever it packs
    // (json's rows ship with json, wherever json travels).
    // Archive-backed groups' dev tables are IGNORED, not walked (their
    // paths never crossed the pack boundary); one that still declares
    // dirs it needs is the v1 refusal.
    for (spec, source) in &sources {
        match source {
            PkgSource::Dir(gdir) | PkgSource::Unpacked(gdir) => {
                // a source-zip dep's dev tables never travel — its tree
                // is sealed, the same law an archive-backed group keeps
                if world.unpacked_of(spec).is_some() {
                    let dm =
                        read_manifest(&src, gdir).map_err(rut_driver::pack::PackError::from)?;
                    if !dm.dev_deps.is_empty() {
                        return Err(PackError::law(format!(
                            "packed dep `{spec}` needs its own [dev-deps] directories, which do \
                             not travel in a source zip — vendor this dep instead"
                        )));
                    }
                    continue;
                }
                let dm = read_manifest(&src, gdir).map_err(rut_driver::pack::PackError::from)?;
                mount_dev_table(&mut world, &src, gdir, spec, &dm, map, lock)
                    .map_err(rut_driver::pack::PackError::from)?;
            }
            PkgSource::Archive { .. } => {
                let dm = group_manifest(source, &world.archives);
                let rides_source = world
                    .pkg_body(spec)
                    .map(|b| matches!(b, rut_driver::MountedBody::Source { .. }))
                    .unwrap_or(false);
                if rides_source && !dm?.dev_deps.is_empty() {
                    return Err(PackError::law(format!(
                        "packed dep `{spec}` needs its own [dev-deps] directories, which do not travel in a bundle — vendor this dep (unpack the url dep into your project) instead"
                    )));
                }
            }
        }
    }
    // the file reader the emission reads through — (dir key, rel) →
    // bytes; the KEY MATH stays this side, the driver never sees a
    // `Path`
    let read = std::rc::Rc::new(|dir_key: &str, rel: &str| -> Result<Vec<u8>, String> {
        let joined = Path::new(dir_key).join(rel);
        std::fs::read(&joined).map_err(|e| format!("cannot read {}: {e}", joined.display()))
    });
    let archives = std::mem::take(&mut world.archives);
    let world_pack = PackWorld {
        loaded: world.into_loaded(root),
        manifest,
        manifest_bytes,
        root_dir: dir_key,
        sources,
        archives,
        read,
    };
    rut_driver::pack::pack(&world_pack, opts)
}

/// Stage the wire rows into a manifest text — right after the opening
/// brace, one line each, everything else byte-untouched. The staged
/// text must re-parse (the caller gates it).
fn ensure_wire_rows(text: &str, version: u64) -> Result<String, PackError> {
    let brace = text.find('{').ok_or_else(|| {
        PackError::law("the manifest has no `{` — not a JSONC object".to_string())
    })?;
    Ok(format!(
        "{}\n  \"format\": \"rutbundle\",\n  \"format_version\": {},{}",
        &text[..=brace],
        version,
        &text[brace + 1..]
    ))
}

/// Pack a `type = "host"` directory as a **decl root** —
/// single-package, byte-deterministic: the manifest byte-for-byte plus
/// its ONE consolidated declaration entry (wire v12 — the tree
/// normalized into the single-file inline form). The pack-time
/// VERIFICATION is two-fold: the surface's tree parse + lower (the
/// exact lane a mount runs — a broken `.d.rut` or a declared child
/// with no mounted file refuses), and the generation invariant — the
/// consolidated entry must lower to the same rows the file-tree form
/// did. A host pkg has no deps, no programs, no ledger — nothing else
/// to emit.
///
/// The root surface is the spelled `entry.type` (resolved under the
/// package's `src/` shell) when the manifest carries one, else
/// `src/mod.d.rut` — the load lane's default-root discovery, mirrored
/// (the pack must verify the exact layout the reader mounts back).
/// Neither present refuses, naming both doors. The entry rides at the
/// ONE canonical name (`mod.d.rut`): the archive is name-free, whatever
/// the directory grammar spelled. A FLAT host pkg's entry is its
/// surface verbatim — its bytes stay the two-entry shape (the rows
/// envelope is the v11 lane; the reader loads both forever).
fn pack_host_root(
    dir: &Path,
    manifest: &Manifest,
    manifest_bytes: Vec<u8>,
) -> Result<Vec<u8>, PackError> {
    let rel = match manifest.entry.type_path.as_deref() {
        Some(rel) => crate::walk::src_rel(rel),
        None => {
            if !dir.join(crate::walk::DECL_ROOT_FILE).is_file() {
                return Err(PackError::law(format!(
                    "{} is a host pkg with no entry — spell `\"entry\": {{ \"type\": \
                     \"<surface>.d.rut\" }}` or add a `src/mod.d.rut` beside the manifest",
                    dir.join(rut_driver::bundle::MANIFEST_NAME).display()
                )));
            }
            crate::walk::DECL_ROOT_FILE.to_string()
        }
    };
    let src_path = dir.join(&rel);
    let src =
        std::fs::read_to_string(&src_path).map_err(|e| PackError::io(src_path.display(), e))?;
    // the surface's mod tree mounts from the directory —
    // `NAME/mod.d.rut` beside the declaring file, recursively,
    // cycle-guarded (the load lane's Decl flavor, spelled identically)
    let root_canon = std::fs::canonicalize(&src_path)
        .unwrap_or_else(|_| src_path.clone())
        .to_string_lossy()
        .into_owned();
    let mut files = BTreeMap::new();
    files.insert(String::new(), src_path.clone());
    let mut lookup = |parent: &str, name: &str| {
        crate::walk::dir_child_lookup(dir, &mut files, parent, name, "mod.d.rut")
    };
    let mods: BTreeMap<String, rut_driver::ModSource> = rut_driver::mount_mod_children(
        &src,
        &root_canon,
        rut_driver::Mode::Decl,
        "mod.d.rut",
        &mut lookup,
    )
    .map_err(PackError::law)?;
    // the verification, half one: the surface + its tree parse and
    // lower into host rows — the exact lane a mount runs
    let tree = rut_driver::lower_decl_module(&src, &mods, &src_path.display().to_string())
        .map_err(PackError::law)?;
    // half two, the generation invariant: the ONE consolidated entry
    // (the tree normalized into the inline form) must lower to the same
    // rows the file-tree form did
    let tree_rows = match &tree.body {
        rut_driver::MountedBody::Host { host_funcs, .. } => host_funcs.clone(),
        _ => Vec::new(),
    };
    let entry = rut_driver::normalize_decl_tree(&src, &mods).map_err(PackError::law)?;
    let entry_rows = rut_driver::lower_inline_entry(&entry, &src_path.display().to_string())
        .map_err(PackError::law)?;
    if !rut_driver::rows_agree(&tree_rows, &entry_rows) {
        return Err(PackError::law(format!(
            "{}: the consolidated declaration entry does not lower to the tree's own rows — \
             the generation invariant refuses the pack",
            src_path.display()
        )));
    }
    // entries = rut.jsonc + the ONE consolidated entry — the exact shape
    // the container lane reads back at load (the canonical `mod.d.rut`
    // name; wire v12 carries no rows envelope for host roots)
    let entries: Vec<(String, Vec<u8>)> = vec![
        (rut_driver::bundle::MANIFEST_NAME.into(), manifest_bytes),
        (rut_driver::decl::DECL_CHILD_FILE.into(), entry.into_bytes()),
    ];
    write_bundle(&entries).map_err(|e| PackError::law(e.to_string()))
}

/// The `[deps]` graph, recursively — each package under its name, as
/// the source its files ride from: dir deps as [`PkgSource::Dir`] keys,
/// url deps as their archive's [`PkgSource::Archive`] locations (the
/// archive root at prefix `""`, every group at its prefix — enumerated
/// from the walk's locations, closed: no recursion into archive
/// manifests). Name-checked against each manifest (the packer only
/// ever reads manifest-named paths).
fn collect_group_sources(
    src: &FsSource,
    root_key: &str,
    manifest: &Manifest,
    world: &World,
) -> Result<BTreeMap<String, PkgSource>, PackError> {
    let mut out = BTreeMap::new();
    collect_group_sources_walk(
        src,
        root_key,
        manifest,
        world,
        &mut out,
        &mut BTreeSet::new(),
        false,
    )?;
    Ok(out)
}

fn collect_group_sources_walk(
    src: &FsSource,
    dir_key: &str,
    manifest: &Manifest,
    world: &World,
    out: &mut BTreeMap<String, PkgSource>,
    seen: &mut BTreeSet<String>,
    from_zip: bool,
) -> Result<(), PackError> {
    for (spec, desc) in &manifest.deps {
        if !seen.insert(spec.clone()) {
            continue;
        }
        if desc.contains_key("url") {
            // a SOURCE-ZIP url dep mounted as a directory (the unpack
            // lane): its group reads like any dir dep's, and its own
            // rewritten rows — pointing inside its unpacked `deps/` —
            // recurse the same way (the closure is the zip's own
            // subtree)
            if let Some(unpacked) = world.unpacked_of(spec) {
                let unpack_key = unpacked.to_string_lossy().into_owned();
                out.insert(spec.clone(), PkgSource::Unpacked(unpack_key.clone()));
                let dm =
                    read_manifest(src, &unpack_key).map_err(rut_driver::pack::PackError::from)?;
                collect_group_sources_walk(src, &unpack_key, &dm, world, out, seen, true)?;
                continue;
            }
            // the archive root's location, then every group the same
            // archive contributed (first-mount-wins decided which)
            let Some((slot, _prefix)) = world.archive_of(spec) else {
                return Err(PackError::law(format!(
                    "dep `{spec}` is declared by url but was not mounted from an archive"
                )));
            };
            out.insert(
                spec.clone(),
                PkgSource::Archive {
                    slot,
                    prefix: String::new(),
                },
            );
            for (group, (gslot, gprefix)) in world.archive_locs() {
                if gslot == slot {
                    out.insert(
                        group.clone(),
                        PkgSource::Archive {
                            slot,
                            prefix: gprefix,
                        },
                    );
                }
            }
            continue; // closed — the archive brought its whole closure
        }
        let rel = desc
            .get("path")
            .ok_or_else(|| PackError::law(format!("dep `{spec}` has no `path`")))?;
        let dep_dir = src.resolve(dir_key, rel).map_err(PackError::law)?;
        // the mount key the walk used: identity dirs mount under their
        // identities — the group sources answer under the same key
        let mount_key = crate::walk::mount_key_for(&dep_dir, spec);
        let dm = read_manifest(src, &dep_dir).map_err(rut_driver::pack::PackError::from)?;
        if dm.name.as_deref() != Some(rut_driver::bundle::mount_name(&mount_key)) {
            return Err(PackError::law(format!(
                "dep `{spec}` points at `{dep_dir}` — the manifest there names it `{}`",
                dm.name.as_deref().unwrap_or("<unnamed>")
            )));
        }
        out.insert(
            mount_key,
            if from_zip {
                PkgSource::Unpacked(dep_dir.clone())
            } else {
                PkgSource::Dir(dep_dir.clone())
            },
        );
        collect_group_sources_walk(src, &dep_dir, &dm, world, out, seen, from_zip)?;
    }
    Ok(())
}

/// One archive group's parsed manifest — the emission's decisions read
/// it (its deps keys, its surface).
pub(crate) fn group_manifest(
    source: &PkgSource,
    archives: &[Archive],
) -> Result<Manifest, PackError> {
    let bytes = group_file(source, rut_driver::bundle::MANIFEST_NAME, archives)?;
    parse_manifest(
        &String::from_utf8(bytes).map_err(|_| {
            PackError::law(format!("{}: not UTF-8", rut_driver::bundle::MANIFEST_NAME))
        })?,
    )
    .map_err(rut_driver::pack::PackError::from)
}

/// One group's file — `rel` is relative to the group's source: a dir
/// key or, for an archive group, archive-relative to its prefix.
pub(crate) fn group_file(
    source: &PkgSource,
    rel: &str,
    archives: &[Archive],
) -> Result<Vec<u8>, PackError> {
    match source {
        PkgSource::Dir(dir) | PkgSource::Unpacked(dir) => {
            let path = Path::new(dir).join(rel);
            std::fs::read(&path).map_err(|e| PackError::io(path.display(), e))
        }
        PkgSource::Archive { slot, prefix } => {
            let rel = rel.strip_prefix("./").unwrap_or(rel);
            let key = rut_driver::bundle::bundle_key(&format!("{prefix}{rel}"))
                .map_err(PackError::law)?;
            let archive = archives.get(*slot).ok_or_else(|| {
                PackError::law(format!(
                    "archive slot {slot} is not in this pack's archive list"
                ))
            })?;
            rut_driver::bundle::read_entry(&archive.entries, &key)
                .map(|b| b.into_bytes())
                .map_err(|e| PackError::law(format!("{}: {e}", archive.origin)))
        }
    }
}
