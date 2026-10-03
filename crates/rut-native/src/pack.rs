//! The pack lanes — a module directory (+ its whole `deps` closure) →
//! one deterministic `.rutbundle`. The FS half lives here: the root
//! manifest bytes, the `[deps]` walk, the dev-table mounts (compile
//! once per owner), the group-source collection, and the file reader
//! the emission reads through. The COMPILE + EMIT half is the
//! driver's pure [`rut_driver::pack::pack`] over the assembled world.
//! Writing the output file stays with the caller (the CLI). Same
//! input directory ⇒ byte-identical bundle.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rut_driver::bundle::{parse_manifest, write_bundle, Manifest, PkgType};
use rut_driver::pack::{PackError, PackOpts, PackWorld, PkgSource};

use crate::source::{FsSource, Source};
use crate::walk::{mount_dev_table, read_manifest, run_gate, walk_dir, Archive, World, MANIFEST_RETIRED};

/// The conventional pack output: `<dirname>.rutbundle`, a sibling of
/// the directory (`demo/mod` → `demo/mod.rutbundle`).
pub fn default_out_path(dir: &Path) -> std::path::PathBuf {
    let stem = dir.file_name().unwrap_or(dir.as_os_str()).to_string_lossy();
    dir.with_file_name(format!("{stem}.rutbundle"))
}

/// Pack a module directory into a deterministic `.rutbundle`.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, PackError> {
    Ok(pack_dir_opts(dir, &PackOpts::default())?.0)
}

/// [`pack_dir`] over pre-fetched url bytes — the fetched core the
/// `*_with` lane calls after awaiting the prefetch. Url rows read the
/// map; a url row absent from it is the loud no-fetcher error.
pub fn pack_dir_fetched(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, PackError> {
    Ok(pack_dir_opts_fetched(dir, &PackOpts::default(), map)?.0)
}

/// [`pack_dir`] with options. Returns the bundle bytes and — under
/// `strip` — the serialized symbol table. Deterministic end to end:
/// same input dir ⇒ byte-identical bundle (mangling is sorted-union
/// based) ⇒ byte-identical symtab. Url rows refuse here: pass a
/// fetcher (`pack_dir_opts_with`) or pre-fetch a map.
pub fn pack_dir_opts(
    dir: &Path,
    opts: &PackOpts,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    pack_dir_opts_fetched(dir, opts, &BTreeMap::new())
}

/// `[pack_dir]` with a url-dep fetcher — the `*_with` lane: collect the
/// url rows, await `fetch` per url sequentially, pack over the
/// bytes map.
pub async fn pack_dir_with(dir: &Path, fetch: &dyn DepRemote) -> Result<Vec<u8>, PackError> {
    Ok(pack_dir_opts_with(dir, &PackOpts::default(), fetch).await?.0)
}

/// [`pack_dir_opts`] with a url-dep fetcher (the CLI's `--strip` lane).
pub async fn pack_dir_opts_with(
    dir: &Path,
    opts: &PackOpts,
    fetch: &dyn DepRemote,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    let src = FsSource::at(dir);
    let map = crate::walk::prefetch_urls(&src, &dir.to_string_lossy(), fetch)
        .await
        .map_err(rut_driver::pack::PackError::from)?;
    pack_dir_opts_fetched(dir, opts, &map)
}

use crate::remote::DepRemote;

/// The pack lane's sync core: the root manifest's gates, the walk, the
/// dev tables, the one peer gate — then the driver's pure pack over
/// the assembled world.
pub fn pack_dir_opts_fetched(
    dir: &Path,
    opts: &PackOpts,
    map: &BTreeMap<String, Vec<u8>>,
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
    let manifest = parse_manifest(&String::from_utf8(manifest_bytes.clone()).map_err(
        |_| PackError::law(format!("{}: not UTF-8", manifest_path.display())),
    )?)?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| PackError::law(format!("{} has no `name`", manifest_path.display())))?;
    // the packed rut.jsonc is the directory's rut.jsonc byte-for-byte,
    // so the bundle keys must already be there — directory loading
    // ignores them, but a bundle loader refuses without them (refuse,
    // never guess): v9 since the jsonc-manifest cutover, v10 for a host
    // root (its surface rides as source — there is nothing to compile)
    let want_version = match manifest.pkg_type {
        PkgType::Host => rut_driver::pack::FORMAT_VERSION_DECL,
        PkgType::Lib => rut_driver::pack::FORMAT_VERSION,
    };
    if manifest.format.as_deref() != Some("rutbundle") || manifest.format_version != Some(want_version)
    {
        return Err(PackError::law(format!(
            "{} is not bundle-shaped — add `format = \"rutbundle\"` and `format_version = {want_version}`",
            manifest_path.display()
        )));
    }
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
        return pack_host_root(dir, &manifest, manifest_bytes).map(|bytes| (bytes, None));
    }
    // the closure's world: the [deps] walk + the dev pass
    let src = FsSource::at(dir);
    let dir_key = dir.to_string_lossy().into_owned();
    let mut world = World::new();
    let root = walk_dir(&mut world, &src, &dir_key, map).map_err(rut_driver::pack::PackError::from)?;
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
    // too — the presence a declarer's peer groups ride (json's rows
    // ship with json, wherever json travels). Archive-backed groups'
    // dev tables are IGNORED, not walked (their paths never crossed the
    // pack boundary); one that still declares dirs it needs is the v1
    // refusal.
    for (spec, source) in &sources {
        match source {
            PkgSource::Dir(gdir) => {
                let dm = read_manifest(&src, gdir).map_err(rut_driver::pack::PackError::from)?;
                mount_dev_table(&mut world, &src, gdir, &dm, map).map_err(rut_driver::pack::PackError::from)?;
            }
            PkgSource::Archive { .. } => {
                let dm = group_manifest(source, &world.archives);
                let rides_source = world
                    .pkg_body(spec)
                    .map(|b| matches!(b, rut_driver::PkgBody::Source { .. }))
                    .unwrap_or(false);
                if rides_source && !dm?.dev_deps.is_empty() {
                    return Err(PackError::law(format!(
                        "packed dep `{spec}` needs its own [dev-deps] directories, which do not travel in a bundle — vendor this dep (unpack the url dep into your project) instead"
                    )));
                }
            }
        }
    }
    // re-run the ONE peer gate over the grown closure (the dev mounts
    // may have supplied peers) — the D3 checks plus the pure presence
    // law, exactly the walk's own pass
    run_gate(&mut world, &root).map_err(rut_driver::pack::PackError::from)?;
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

/// Pack a `type = "host"` directory as a **v10 decl root** —
/// single-package, byte-deterministic: the manifest byte-for-byte plus
/// its declaration surface file(s) (the same file-set shape a host
/// group rides inside a v9 bundle). The pack-time VERIFICATION is the
/// surface's own parse + lower (the exact lane a mount runs) — the
/// lowered module is discarded; decls ride as source and the embedding
/// Rust binds the bodies. A host pkg has no deps, no programs, no
/// ledger — nothing else to emit.
fn pack_host_root(
    dir: &Path,
    manifest: &Manifest,
    manifest_bytes: Vec<u8>,
) -> Result<Vec<u8>, PackError> {
    let rel = manifest.entry.type_path.as_deref().ok_or_else(|| {
        PackError::law(format!(
            "{} is a host pkg with no `entry.type`",
            dir.join(rut_driver::bundle::MANIFEST_NAME).display()
        ))
    })?;
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    let src_path = dir.join(rel);
    let src = std::fs::read_to_string(&src_path)
        .map_err(|e| PackError::io(src_path.display(), e))?;
    // the verification: the surface parses and lowers into host rows —
    // a broken `.d.rut` refuses to pack (discard the output)
    rut_driver::lower_decl_module(&src, &src_path.display().to_string())
        .map_err(PackError::law)?;
    // entries = rut.jsonc + the surface (the root file set, prefix "") —
    // the exact shape the container lane reads back at load
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    entries.push((rut_driver::bundle::MANIFEST_NAME.into(), manifest_bytes));
    let key = rut_driver::bundle::bundle_key(rel).map_err(PackError::law)?;
    entries.push((key, src.into_bytes()));
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
    collect_group_sources_walk(src, root_key, manifest, world, &mut out, &mut BTreeSet::new())?;
    Ok(out)
}

fn collect_group_sources_walk(
    src: &FsSource,
    dir_key: &str,
    manifest: &Manifest,
    world: &World,
    out: &mut BTreeMap<String, PkgSource>,
    seen: &mut BTreeSet<String>,
) -> Result<(), PackError> {
    for (spec, desc) in &manifest.deps {
        if !seen.insert(spec.clone()) {
            continue;
        }
        if desc.contains_key("url") {
            // the archive root's location, then every group the same
            // archive contributed (first-mount-wins decided which)
            let Some((slot, _prefix)) = world.archive_of(spec) else {
                return Err(PackError::law(format!(
                    "dep `{spec}` is declared by url but was not mounted from an archive"
                )));
            };
            out.insert(
                spec.clone(),
                PkgSource::Archive { slot, prefix: String::new() },
            );
            for (group, (gslot, gprefix)) in world.archive_locs() {
                if gslot == slot {
                    out.insert(
                        group.clone(),
                        PkgSource::Archive { slot, prefix: gprefix },
                    );
                }
            }
            continue; // closed — the archive brought its whole closure
        }
        let rel = desc
            .get("path")
            .ok_or_else(|| PackError::law(format!("dep `{spec}` has no `path`")))?;
        let dep_dir = src.resolve(dir_key, rel).map_err(PackError::law)?;
        let dm = read_manifest(src, &dep_dir).map_err(rut_driver::pack::PackError::from)?;
        if dm.name.as_deref() != Some(spec.as_str()) {
            return Err(PackError::law(format!(
                "dep `{spec}` points at `{dep_dir}` — the manifest there names it `{}`",
                dm.name.as_deref().unwrap_or("<unnamed>")
            )));
        }
        out.insert(spec.clone(), PkgSource::Dir(dep_dir.clone()));
        collect_group_sources_walk(src, &dep_dir, &dm, world, out, seen)?;
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
    parse_manifest(&String::from_utf8(bytes).map_err(|_| {
        PackError::law(format!("{}: not UTF-8", rut_driver::bundle::MANIFEST_NAME))
    })?)
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
        PkgSource::Dir(dir) => {
            let path = Path::new(dir).join(rel);
            std::fs::read(&path).map_err(|e| PackError::io(path.display(), e))
        }
        PkgSource::Archive { slot, prefix } => {
            let rel = rel.strip_prefix("./").unwrap_or(rel);
            let key = rut_driver::bundle::bundle_key(&format!("{prefix}{rel}"))
                .map_err(PackError::law)?;
            let archive = archives.get(*slot).ok_or_else(|| {
                PackError::law(format!("archive slot {slot} is not in this pack's archive list"))
            })?;
            rut_driver::bundle::read_entry(&archive.entries, &key)
                .map(|b| b.into_bytes())
                .map_err(|e| PackError::law(format!("{}: {e}", archive.origin)))
        }
    }
}
