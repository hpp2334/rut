//! The packer: a module directory (+ its whole `[deps]` closure) →
//! one deterministic `.rutbundle`. TWO root kinds, paired with the
//! format version: a **lib** root packs **compiled** (v5 — the root and
//! every source dep ride as `.rutc` binaries (bodies + surface — the
//! linking truth); host/decl pkgs ride as their declaration file sets),
//! and a **host** root packs as a **v6 decl root** (single-package: its
//! `.d.rut` surface rides as source, nothing to compile — the surface
//! verifies at pack time by parsing + lowering once, then the module
//! is discarded).
//! Instantiation is owner-anchored (a generic export links, its
//! consumers request), class methods cross on the surface's inherent
//! rows, and the pack-time scope ledger lets a loader rebase every
//! decoded program onto its own numbering.
//!
//! The packer lives in the driver because it needs the compiler: it
//! loads the directory's session, mounts every dep's `[dev-deps]` too
//! (compile once per owner — a dep's unit is the same bytes wherever
//! it is packed), and walks the graph once ([`compile_units`]). The
//! bytes come back; writing the output file stays with the caller
//! (the CLI). Same input directory ⇒ byte-identical bundle.
//!
//! A `[deps]` row may pin a **url**: the fetched `.rutbundle`'s groups
//! ride along (`PkgSource::Archive` locations, enumerated closed — no
//! recursion). `.rutc` emission stays ONE path: archive-backed
//! compiled modules were rebased onto this session's numbering by the
//! mount→compile pipeline, so they encode from `units` exactly like
//! dir deps; only the group's FILE reads (`rut.toml`, `.d.rut`, source
//! sets) dispatch on the source. The manifest rides byte-for-byte (the
//! law) — url+sha256 rows carry into the output satisfied by the
//! rode-along groups.
//!
//! Refusals (never guesses): cycles and name collisions — the graph's
//! own laws, surfaced as pack errors; plus the v1 url-dep refusals
//! below. Source sharing stays what it always was outside bundles: a
//! directory (`rut run <dir>`).


mod error;
mod groups;
mod strip;

pub use error::PackError;
use groups::{group_file, group_manifest, has_open_generic_surface, ride_generic_source, rides_compiled};
use strip::strip_arm;

use std::collections::BTreeMap;
use std::path::Path;

use crate::bundle::{bundle_key, collect_source_group, parse_manifest, read_entry, write_bundle, FsSource};

use crate::graph::compile_units;
use crate::loader::{
    load_dir_session_fetched, mount_dev_table_fetched, run_peer_gate, Archive, PkgSource,
};
use crate::session::ModuleBody;

/// The v5 bundle layout version — a **compiled** root (a lib pkg).
pub const FORMAT_VERSION: u64 = 5;

/// The v6 bundle layout version — a **decl** root (a `type = "host"`
/// pkg: its `.d.rut` surface rides as source, single-package). Writers
/// emit 6 ONLY for decl roots; readers accept 5|6 — the pairing is
/// total, both directions refused at the gate.
pub const FORMAT_VERSION_DECL: u64 = 6;

/// The `rut.scopes` ledger: one `<scope> = "<spec>"` row per linked
/// module of the packed closure (engine mounts included), ascending by
/// scope. A loader rebases every decoded program's foreign ids through
/// it — a reference with no row is refused, never guessed.
fn ledger_text(rows: &[(rut_core::id::ScopeId, String)]) -> String {
    let mut out = String::new();
    for (scope, spec) in rows {
        out.push_str(&format!("{scope} = \"{spec}\"\n"));
    }
    out
}

/// Pack a `type = "host"` directory as a **v6 decl root** —
/// single-package, byte-deterministic: the manifest byte-for-byte plus
/// its declaration surface file(s) (the same file-set shape a host
/// group rides inside a v5 bundle). The pack-time VERIFICATION is the
/// surface's own parse + lower (the exact lane a mount runs) — the
/// lowered module is discarded; decls ride as source and the embedding
/// Rust binds the bodies. A host pkg has no deps, no programs, no
/// ledger — nothing else to emit.
fn pack_host_root(
    dir: &Path,
    manifest: &crate::bundle::Manifest,
    manifest_bytes: Vec<u8>,
) -> Result<Vec<u8>, PackError> {
    let rel = manifest.entry.type_path.as_deref().ok_or_else(|| {
        PackError::law(format!("{} is a host pkg with no `entry.type`", dir.join("rut.toml").display()))
    })?;
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    let src_path = dir.join(rel);
    let src = std::fs::read_to_string(&src_path)
        .map_err(|e| PackError::io(src_path.display(), e))?;
    // the verification: the surface parses and lowers into host rows —
    // a broken `.d.rut` refuses to pack (discard the output)
    crate::decl::lower_decl_module(&src, &src_path.display().to_string())
        .map_err(PackError::law)?;
    // entries = rut.toml + the surface (the root file set, prefix "") —
    // the exact shape `bundle_entry_module` reads back at load
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    entries.push(("rut.toml".into(), manifest_bytes));
    let key = bundle_key(rel).map_err(PackError::law)?;
    entries.push((key, src.into_bytes()));
    write_bundle(&entries).map_err(|e| PackError::law(e.to_string()))
}

/// Pack a lib directory into a deterministic v5 `.rutbundle` (compiled
/// root), or a `type = "host"` directory into a v6 decl root.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, PackError> {
    Ok(pack_dir_opts(dir, &PackOpts::default())?.0)
}

/// [`pack_dir`] over pre-fetched url bytes — the fetched core the
/// `*_with` lane calls after awaiting `dep_fetch`. Url rows read the
/// map; a url row absent from it is the loud no-fetcher error.
pub fn pack_dir_fetched(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, PackError> {
    Ok(pack_dir_opts_fetched(dir, &PackOpts::default(), map)?.0)
}

/// Pack options. `strip` mangles every renameable name and strips the
/// symbolication tables from the emitted binaries — the restore data
/// rides a PRIVATE symbol-table sidecar beside the bundle (never an
/// entry inside it).
#[derive(Clone, Debug, Default)]
pub struct PackOpts {
    pub strip: bool,
}

/// [`pack_dir`] with options. Returns the bundle bytes and — under
/// `strip` — the serialized symbol table. Deterministic end to end:
/// same input dir ⇒ byte-identical bundle (mangling is sorted-union
/// based) ⇒ byte-identical symtab. Url rows refuse here: pass a
/// fetcher (`pack_dir_opts_with`) or pre-fetch a map.
pub fn pack_dir_opts(dir: &Path, opts: &PackOpts) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    pack_dir_opts_fetched(dir, opts, &BTreeMap::new())
}

/// [`pack_dir_opts`] over pre-fetched url bytes.
pub fn pack_dir_opts_fetched(
    dir: &Path,
    opts: &PackOpts,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    let manifest_bytes = std::fs::read(dir.join("rut.toml"))
        .map_err(|e| PackError::io(dir.join("rut.toml").display(), e))?;
    let manifest = parse_manifest(&String::from_utf8(manifest_bytes.clone()).map_err(
        |_| PackError::law(format!("{}: not UTF-8", dir.join("rut.toml").display())),
    )?)?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| PackError::law(format!("{} has no `name`", dir.join("rut.toml").display())))?;
    // the packed rut.toml is the directory's rut.toml byte-for-byte, so
    // the bundle keys must already be there — directory loading ignores
    // them, but a bundle loader refuses without them (refuse, never
    // guess): v5 since the compiled-bundle batch, v6 for a host root
    // (its surface rides as source — there is nothing to compile)
    let want_version = match manifest.pkg_type {
        crate::bundle::PkgType::Host => FORMAT_VERSION_DECL,
        crate::bundle::PkgType::Lib => FORMAT_VERSION,
    };
    if manifest.format.as_deref() != Some("rutbundle") || manifest.format_version != Some(want_version)
    {
        return Err(PackError::law(format!(
            "{} is not bundle-shaped — add `format = \"rutbundle\"` and `format_version = {want_version}`",
            dir.join("rut.toml").display()
        )));
    }
    // the host-root arm: no closure (the grammar refuses a host
    // manifest's deps tables), no compile walk — the surface verifies
    // by parsing + lowering once, then rides as source
    if manifest.pkg_type == crate::bundle::PkgType::Host {
        if opts.strip {
            return Err(PackError::law(
                "a host bundle has no symbols to strip — its root is a declaration surface, \
                 not a program; there is no binary and no sidecar",
            ));
        }
        return pack_host_root(dir, &manifest, manifest_bytes).map(|bytes| (bytes, None));
    }
    // the closure's session: the [deps] walk + the dev pass + the peer
    // gate, then the engine mounts the closure's code needs (the
    // prelude always; `calc` when a program reaches `Math`)
    let loaded = load_dir_session_fetched(dir, &FsSource, map)?;
    let (mut session, root, archives) = (loaded.session, loaded.root, loaded.archives);
    crate::mount_std(&mut session);
    if root != name {
        return Err(PackError::law(format!(
            "{} names itself `{root}` — expected `{name}`",
            dir.join("rut.toml").display()
        )));
    }
    // the closure's group set: dir deps as directories, url deps as
    // their archive's locations (enumerated closed — no recursion)
    let mut sources = BTreeMap::new();
    collect_group_sources(
        dir,
        &manifest,
        &session,
        &mut sources,
        &mut std::collections::BTreeSet::new(),
    )?;
    // the rode-along closure law: every dep an archive group's manifest
    // declares must ride in this output — a group the mount skipped
    // without a source of its own would leave the output declaring a
    // dep without a group (v1: loud, named)
    for (spec, source) in &sources {
        if let PkgSource::Archive { .. } = source {
            let dm = group_manifest(source, &archives).map_err(PackError::law)?;
            for dep in dm.deps.keys() {
                if !sources.contains_key(dep) && *dep != root {
                    return Err(PackError::law(format!(
                        "packed dep `{spec}` declares `{dep}`, but that group did not ride — the output would be missing its `{dep}` group; vendor this dep (unpack the url dep into your project) instead"
                    )));
                }
            }
        }
    }
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
                let dm = crate::bundle::read_manifest(gdir, &FsSource).map_err(PackError::law)?;
                mount_dev_table_fetched(&mut session, gdir, &dm, map)?;
            }
            PkgSource::Archive { .. } => {
                let dm = group_manifest(source, &archives).map_err(PackError::law)?;
                let rides_source = matches!(
                    session.resolve(spec).map(|m| &m.body),
                    Ok(ModuleBody::Source { .. })
                );
                if rides_source && !dm.dev_deps.is_empty() {
                    return Err(PackError::law(format!(
                        "packed dep `{spec}` needs its own [dev-deps] directories, which do not travel in a bundle — vendor this dep (unpack the url dep into your project) instead"
                    )));
                }
            }
        }
    }
    // re-run the ONE peer gate over the grown closure (the dev mounts
    // may have supplied peers) — archive group reads dispatch on the
    // recorded locations, through the archives the load opened
    let mut sources_all = sources.clone();
    sources_all.insert(root.clone(), PkgSource::Dir(dir.to_path_buf()));
    for (pkg, (slot, prefix)) in session.archive_mounts() {
        sources_all.entry(pkg.clone()).or_insert(PkgSource::Archive {
            slot: *slot,
            prefix: prefix.clone(),
        });
    }
    run_peer_gate(&mut session, &root, &sources_all, &archives)?;
    let mut units = compile_units(&session, &root);
    if !units.diags.is_empty() || !units.ok {
        let msgs: Vec<String> = units.diags.iter().map(|d| d.msg.clone()).collect();
        return Err(PackError::law(format!("pack: {}", msgs.join("; "))));
    }
    // a compiled group the walk never ensured cannot ride: its binary
    // still spells ITS pack's scopes, which this output's ledger does
    // not carry — refuse, never guess (v1: use it or vendor the dep)
    for (spec, source) in &sources {
        if let PkgSource::Archive { slot, .. } = source {
            if matches!(
                session.resolve(spec).map(|m| &m.body),
                Ok(ModuleBody::Compiled(_))
            ) && !units.linked.contains_key(spec)
            {
                return Err(PackError::law(format!(
                    "compiled group `{spec}` (from {}) is not part of this program's compiled closure — a bundled binary cannot ride unpackaged; use it or vendor this dep",
                    archives[*slot].origin
                )));
            }
        }
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    // the manifest byte-for-byte, then the scope ledger
    entries.push(("rut.toml".into(), manifest_bytes));
    let mut ledger: Vec<(rut_core::id::ScopeId, String)> = units
        .linked
        .iter()
        .map(|(spec, &(_, scope))| (scope, spec.clone()))
        .collect();
    ledger.sort();
    entries.push(("rut.scopes".into(), ledger_text(&ledger).into_bytes()));
    // the root's compiled binary (+ its surface text for humans, when
    // the package declares one)
    let root_linked = units.linked.get(&root);
    let root_ok = matches!(
        (&session.resolve(&root).unwrap().body, root_linked),
        (ModuleBody::Source { .. }, Some(&_))
    );
    if !root_ok {
        return Err(PackError::law(format!(
            "pack: {root} produced no compiled program — a host pkg (a `.d.rut` surface with no body) cannot be a bundle root; pack a source pkg instead"
        )));
    }
    let root_idx = root_linked.unwrap().0;
    // the strip arm — after the compile walk, before any encode. Three
    // laws run here, all against the packer's OWN group-kind decisions
    // (the `sources` map + `units.linked`, the same classifier the
    // encode loop applies — never a re-derivation):
    //
    // 1. the mixed-bundle refusal: a group riding as a SOURCE file set
    //    binds its compiled deps' surfaces at load, and those names
    //    would be mangled — refuse, never guess (host/decl leaf groups
    //    have no compiled deps and pass trivially);
    // 2. the generic-source refusal: a compiled pkg with an OPEN
    //    generic surface rides its source (the law below), and the
    //    ridden text would recompile clean-named beside mangled
    //    binaries — the combination cannot coexist soundly, refuse and
    //    say so;
    // 3. the mangle itself: one closure-wide string→string map over
    //    every encoded program, symbolication tables taken into the
    //    sidecar.
    let mut symtab: Option<Vec<u8>> = None;
    if opts.strip {
        {
            let root_prog = &units.programs[root_idx];
            if has_open_generic_surface(root_prog) {
                return Err(PackError::law(format!(
                    "--strip refuses a generic-owning closure: the root `{root}` exports \
                     generics, so its source rides the bundle to serve consumer-spelled \
                     shapes at load — the ridden text would recompile clean-named beside \
                     mangled binaries. Pack without `--strip`"
                )));
            }
            for (spec, source) in &sources {
                if *spec == root || session.resolve(spec).is_err() {
                    continue;
                }
                if !rides_compiled(&session, spec, &units) {
                    continue;
                }
                let &(idx, _) = units.linked.get(spec).unwrap();
                if has_open_generic_surface(&units.programs[idx]) {
                    return Err(PackError::law(format!(
                        "--strip refuses a generic-owning closure: the compiled group \
                         `{spec}` (from {}) exports generics, so its source rides the \
                         bundle to serve consumer-spelled shapes at load — the ridden \
                         text would recompile clean-named beside mangled binaries. Pack \
                         without `--strip`",
                        match source {
                            PkgSource::Dir(d) => d.display().to_string(),
                            PkgSource::Archive { slot, .. } => archives[*slot].origin.clone(),
                        }
                    )));
                }
            }
        }
        for (spec, source) in &sources {
            if *spec == root || session.resolve(spec).is_err() {
                continue; // the root rides compiled above; unmounted names never ride
            }
            if rides_compiled(&session, spec, &units) {
                continue; // a `.rutc` group — no source binds anything
            }
            let dm = group_manifest(source, &archives).map_err(PackError::law)?;
            for dep in dm.deps.keys() {
                let dep_compiled = rides_compiled(&session, dep, &units);
                if dep_compiled {
                    return Err(PackError::law(format!(
                        "--strip needs a fully-compiled closure: the source group \
                         `{spec}` binds compiled `{dep}`'s surface at load, whose \
                         names would be mangled — drop the `inline` flag / \
                         restructure the closure, or pack without `--strip`"
                    )));
                }
            }
        }
        // the encoded set: the root, then every compiled-riding group in
        // `sources` order — gathered in ONE pass so the programs reborrow
        // disjointly
        let mut wanted: Vec<(String, usize)> = vec![(root.clone(), root_idx)];
        for (spec, _) in &sources {
            if *spec == root || session.resolve(spec).is_err() {
                continue;
            }
            if !rides_compiled(&session, spec, &units) {
                continue; // the declaration file set — no binary to strip
            }
            let &(idx, _) = units.linked.get(spec).unwrap();
            wanted.push((spec.clone(), idx));
        }
        let mut groups: Vec<(String, &mut rut_core::binary::Program)> =
            Vec::with_capacity(wanted.len());
        for (i, prog) in units.programs.iter_mut().enumerate() {
            if let Some((spec, _)) = wanted.iter().find(|(_, ix)| *ix == i) {
                groups.push((spec.clone(), prog));
            }
        }
        let map = rut_core::strip::strip_programs(&mut groups).map_err(PackError::law)?;
        symtab = Some(map.to_bytes());
    }
    entries.push((format!("{name}.rutc"), rut_core::binary::encode(&units.programs[root_idx])));
    if let Some(rel) = &manifest.entry.type_path {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let text = std::fs::read(dir.join(rel))
            .map_err(|e| PackError::io(dir.join(rel).display(), e))?;
        entries.push((bundle_key(&format!("{name}.d.rut")).map_err(PackError::law)?, text));
    }
    // the generic-source riding law (v5, additive): a compiled pkg whose
    // surface exports generics ALSO rides the source that serves
    // consumer-spelled shapes — the entry lib + `entry.libs` + the
    // `[peer-deps]` group files, verbatim, beside the binary. The entry
    // lib's presence is the loader's dispatch marker; non-generic pkgs
    // stay source-free. (`--strip` refuses the combination above.)
    if has_open_generic_surface(&units.programs[root_idx]) {
        ride_generic_source(&manifest, "", &mut entries, &|rel| {
            std::fs::read(dir.join(rel))
                .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))
        })
        .map_err(PackError::law)?;
    }
    // the dep groups, name order: source pkgs → compiled, host/decl
    // pkgs and declared-but-unused pkgs → the declaration file set
    for (spec, source) in &sources {
        if *spec == root || session.resolve(spec).is_err() {
            continue; // the root rode above; names are already mounted
        }
        let dm = group_manifest(source, &archives).map_err(PackError::law)?;
        let prefix = format!("{spec}/");
        if rides_compiled(&session, spec, &units) {
            let &(idx, _) = units.linked.get(spec).unwrap();
            // the group's manifest rides byte-for-byte: the loader reads
            // its name, mount flags, and peer declarations from it
            let toml = group_file(source, "rut.toml", &archives).map_err(PackError::law)?;
            entries.push((bundle_key(&format!("{prefix}rut.toml")).map_err(PackError::law)?, toml));
            entries.push((bundle_key(&format!("{prefix}{spec}.rutc")).map_err(PackError::law)?, rut_core::binary::encode(&units.programs[idx])));
            if let Some(rel) = &dm.entry.type_path {
                let rel = rel.strip_prefix("./").unwrap_or(rel);
                let text = group_file(source, rel, &archives).map_err(PackError::law)?;
                entries.push((bundle_key(&format!("{prefix}{rel}")).map_err(PackError::law)?, text));
            }
            // the generic-source riding law, group flavor (see the root
            // arm above): a generic-owning compiled group rides its
            // source under the group prefix
            if has_open_generic_surface(&units.programs[idx]) {
                let source = source.clone();
                ride_generic_source(&dm, &prefix, &mut entries, &|rel| {
                    group_file(&source, rel, &archives)
                })
                .map_err(PackError::law)?;
            }
        } else {
            match source {
                PkgSource::Dir(gdir) => {
                    collect_source_group(gdir, &dm, &prefix, &FsSource, &mut entries)
                        .map_err(PackError::law)?;
                }
                PkgSource::Archive { slot, prefix: old } => {
                    // copy the archive group's file set under the new
                    // prefix — the archive already decided the set
                    // (manifest, entry, libs, peer-group files), in
                    // archive order (deterministic)
                    let archive = &archives[*slot];
                    for (key, bytes) in &archive.entries {
                        let Some(rest) = key.strip_prefix(old.as_str()) else {
                            continue;
                        };
                        if rest.is_empty() {
                            continue;
                        }
                        entries.push((bundle_key(&format!("{prefix}{rest}")).map_err(PackError::law)?, bytes.clone()));
                    }
                }
            }
        }
    }
    let bundle = write_bundle(&entries).map_err(|e| PackError::law(e.to_string()))?;
    Ok((bundle, symtab))
}

/// `[pack_dir]` with a url-dep fetcher — the `*_with` lane: collect the
/// url rows, await `dep_fetch` per url sequentially, pack over the
/// bytes map.
pub async fn pack_dir_with(dir: &Path, fetch: &dyn crate::loader::DepRemote) -> Result<Vec<u8>, PackError> {
    Ok(pack_dir_opts_with(dir, &PackOpts::default(), fetch).await?.0)
}

/// [`pack_dir_opts`] with a url-dep fetcher (the CLI's `--strip` lane).
pub async fn pack_dir_opts_with(
    dir: &Path,
    opts: &PackOpts,
    fetch: &dyn crate::loader::DepRemote,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    let map = crate::loader::prefetch_urls(dir, &FsSource, fetch).await?;
    pack_dir_opts_fetched(dir, opts, &map)
}

/// The `[deps]` graph, recursively — each package under its name, as
/// the source its files ride from: dir deps as [`PkgSource::Dir`], url
/// deps as their archive's [`PkgSource::Archive`] locations (the
/// archive root at prefix `""`, every group at its prefix — enumerated
/// from the mount ledger, closed: no recursion into archive
/// manifests). Name-checked against each manifest (the packer only
/// ever reads manifest-named paths).
fn collect_group_sources(
    dir: &Path,
    manifest: &crate::bundle::Manifest,
    session: &crate::session::Session,
    out: &mut BTreeMap<String, PkgSource>,
    seen: &mut std::collections::BTreeSet<String>,
) -> Result<(), PackError> {
    for (spec, desc) in &manifest.deps {
        if !seen.insert(spec.clone()) {
            continue;
        }
        if desc.contains_key("url") {
            // the archive root's location, then every group the same
            // archive contributed (first-mount-wins decided which)
            let Some((slot, _)) = session.archive_mount(spec) else {
                return Err(PackError::law(format!(
                    "dep `{spec}` is declared by url but was not mounted from an archive"
                )));
            };
            out.insert(
                spec.clone(),
                PkgSource::Archive { slot, prefix: String::new() },
            );
            for (group, (gslot, gprefix)) in session.archive_mounts() {
                if *gslot == slot {
                    out.insert(
                        group.clone(),
                        PkgSource::Archive {
                            slot,
                            prefix: gprefix.clone(),
                        },
                    );
                }
            }
            continue; // closed — the archive brought its whole closure
        }
        let rel = desc
            .get("path")
            .ok_or_else(|| PackError::law(format!("dep `{spec}` has no `path`")))?;
        let dep_dir = dir.join(rel);
        let dm = crate::bundle::read_manifest(&dep_dir, &FsSource).map_err(PackError::law)?;
        if dm.name.as_deref() != Some(spec.as_str()) {
            return Err(PackError::law(format!(
                "dep `{spec}` points at `{}` — the manifest there names it `{}`",
                dep_dir.display(),
                dm.name.as_deref().unwrap_or("<unnamed>")
            )));
        }
        out.insert(spec.clone(), PkgSource::Dir(dep_dir.clone()));
        collect_group_sources(&dep_dir, &dm, session, out, seen)?;
    }
    Ok(())
}

