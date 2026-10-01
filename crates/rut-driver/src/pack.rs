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

/// Pack a lib directory into a deterministic v5 `.rutbundle` (compiled
/// root), or a `type = "host"` directory into a v6 decl root.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, String> {
    Ok(pack_dir_opts(dir, &PackOpts::default())?.0)
}

/// [`pack_dir`] over pre-fetched url bytes — the fetched core the
/// `*_with` lane calls after awaiting `dep_fetch`. Url rows read the
/// map; a url row absent from it is the loud no-fetcher error.
pub fn pack_dir_fetched(
    dir: &Path,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, String> {
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
pub fn pack_dir_opts(dir: &Path, opts: &PackOpts) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    pack_dir_opts_fetched(dir, opts, &BTreeMap::new())
}

/// [`pack_dir_opts`] over pre-fetched url bytes.
pub fn pack_dir_opts_fetched(
    dir: &Path,
    opts: &PackOpts,
    map: &BTreeMap<String, Vec<u8>>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let manifest_bytes = std::fs::read(dir.join("rut.toml"))
        .map_err(|e| format!("cannot read {}: {e}", dir.join("rut.toml").display()))?;
    let manifest = parse_manifest(&String::from_utf8(manifest_bytes.clone()).map_err(
        |_| format!("{}: not UTF-8", dir.join("rut.toml").display()),
    )?)
    .map_err(|e| e.to_string())?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{} has no `name`", dir.join("rut.toml").display()))?;
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
        return Err(format!(
            "{} is not bundle-shaped — add `format = \"rutbundle\"` and `format_version = {want_version}`",
            dir.join("rut.toml").display()
        ));
    }
    // the host-root arm: no closure (the grammar refuses a host
    // manifest's deps tables), no compile walk — the surface verifies
    // by parsing + lowering once, then rides as source
    if manifest.pkg_type == crate::bundle::PkgType::Host {
        if opts.strip {
            return Err(
                "a host bundle has no symbols to strip — its root is a declaration surface, \
                 not a program; there is no binary and no sidecar"
                    .into(),
            );
        }
        return pack_host_root(dir, &manifest, manifest_bytes).map(|bytes| (bytes, None));
    }
    // the closure's session: the [deps] walk + the dev pass + the peer
    // gate, then the engine mounts the closure's code needs (the
    // prelude always; `calc` when a program reaches `Math`)
    let loaded = load_dir_session_fetched(dir, &FsSource, map).map_err(|e| e.to_string())?;
    let (mut session, root, archives) = (loaded.session, loaded.root, loaded.archives);
    crate::mount_std(&mut session);
    if root != name {
        return Err(format!(
            "{} names itself `{root}` — expected `{name}`",
            dir.join("rut.toml").display()
        ));
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
            let dm = group_manifest(source, &archives)?;
            for dep in dm.deps.keys() {
                if !sources.contains_key(dep) && *dep != root {
                    return Err(format!(
                        "packed dep `{spec}` declares `{dep}`, but that group did not ride — the output would be missing its `{dep}` group; vendor this dep (unpack the url dep into your project) instead"
                    ));
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
                let dm = crate::bundle::read_manifest(gdir, &FsSource)?;
                mount_dev_table_fetched(&mut session, gdir, &dm, map).map_err(|e| e.to_string())?;
            }
            PkgSource::Archive { .. } => {
                let dm = group_manifest(source, &archives)?;
                let rides_source = matches!(
                    session.resolve(spec).map(|m| &m.body),
                    Ok(ModuleBody::Source { .. })
                );
                if rides_source && !dm.dev_deps.is_empty() {
                    return Err(format!(
                        "packed dep `{spec}` needs its own [dev-deps] directories, which do not travel in a bundle — vendor this dep (unpack the url dep into your project) instead"
                    ));
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
    run_peer_gate(&mut session, &root, &sources_all, &archives).map_err(|e| e.to_string())?;
    let mut units = compile_units(&session, &root);
    if !units.diags.is_empty() || !units.ok {
        let msgs: Vec<String> = units.diags.iter().map(|d| d.msg.clone()).collect();
        return Err(format!("pack: {}", msgs.join("; ")));
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
                return Err(format!(
                    "compiled group `{spec}` (from {}) is not part of this program's compiled closure — a bundled binary cannot ride unpackaged; use it or vendor this dep",
                    archives[*slot].origin
                ));
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
        return Err(format!(
            "pack: {root} produced no compiled program — a host pkg (a `.d.rut` surface with no body) cannot be a bundle root; pack a source pkg instead"
        ));
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
                return Err(format!(
                    "--strip refuses a generic-owning closure: the root `{root}` exports \
                     generics, so its source rides the bundle to serve consumer-spelled \
                     shapes at load — the ridden text would recompile clean-named beside \
                     mangled binaries. Pack without `--strip`"
                ));
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
                    return Err(format!(
                        "--strip refuses a generic-owning closure: the compiled group \
                         `{spec}` (from {}) exports generics, so its source rides the \
                         bundle to serve consumer-spelled shapes at load — the ridden \
                         text would recompile clean-named beside mangled binaries. Pack \
                         without `--strip`",
                        match source {
                            PkgSource::Dir(d) => d.display().to_string(),
                            PkgSource::Archive { slot, .. } => archives[*slot].origin.clone(),
                        }
                    ));
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
            let dm = group_manifest(source, &archives)?;
            for dep in dm.deps.keys() {
                let dep_compiled = rides_compiled(&session, dep, &units);
                if dep_compiled {
                    return Err(format!(
                        "--strip needs a fully-compiled closure: the source group \
                         `{spec}` binds compiled `{dep}`'s surface at load, whose \
                         names would be mangled — drop the `inline` flag / \
                         restructure the closure, or pack without `--strip`"
                    ));
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
        let map = rut_core::strip::strip_programs(&mut groups)?;
        symtab = Some(map.to_bytes());
    }
    entries.push((format!("{name}.rutc"), rut_core::binary::encode(&units.programs[root_idx])));
    if let Some(rel) = &manifest.entry.type_path {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let text = std::fs::read(dir.join(rel))
            .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
        entries.push((bundle_key(&format!("{name}.d.rut"))?, text));
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
        })?;
    }
    // the dep groups, name order: source pkgs → compiled, host/decl
    // pkgs and declared-but-unused pkgs → the declaration file set
    for (spec, source) in &sources {
        if *spec == root || session.resolve(spec).is_err() {
            continue; // the root rode above; names are already mounted
        }
        let dm = group_manifest(source, &archives)?;
        let prefix = format!("{spec}/");
        if rides_compiled(&session, spec, &units) {
            let &(idx, _) = units.linked.get(spec).unwrap();
            // the group's manifest rides byte-for-byte: the loader reads
            // its name, mount flags, and peer declarations from it
            let toml = group_file(source, "rut.toml", &archives)?;
            entries.push((bundle_key(&format!("{prefix}rut.toml"))?, toml));
            entries.push((bundle_key(&format!("{prefix}{spec}.rutc"))?, rut_core::binary::encode(&units.programs[idx])));
            if let Some(rel) = &dm.entry.type_path {
                let rel = rel.strip_prefix("./").unwrap_or(rel);
                let text = group_file(source, rel, &archives)?;
                entries.push((bundle_key(&format!("{prefix}{rel}"))?, text));
            }
            // the generic-source riding law, group flavor (see the root
            // arm above): a generic-owning compiled group rides its
            // source under the group prefix
            if has_open_generic_surface(&units.programs[idx]) {
                let source = source.clone();
                ride_generic_source(&dm, &prefix, &mut entries, &|rel| {
                    group_file(&source, rel, &archives)
                })?;
            }
        } else {
            match source {
                PkgSource::Dir(gdir) => {
                    collect_source_group(gdir, &dm, &prefix, &FsSource, &mut entries)?;
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
                        entries.push((bundle_key(&format!("{prefix}{rest}"))?, bytes.clone()));
                    }
                }
            }
        }
    }
    let bundle = write_bundle(&entries).map_err(|e| e.to_string())?;
    Ok((bundle, symtab))
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
) -> Result<Vec<u8>, String> {
    let rel = manifest.entry.type_path.as_deref().ok_or_else(|| {
        format!("{} is a host pkg with no `entry.type`", dir.join("rut.toml").display())
    })?;
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    let src_path = dir.join(rel);
    let src = std::fs::read_to_string(&src_path)
        .map_err(|e| format!("cannot read {}: {e}", src_path.display()))?;
    // the verification: the surface parses and lowers into host rows —
    // a broken `.d.rut` refuses to pack (discard the output)
    crate::decl::lower_decl_module(&src, &src_path.display().to_string())?;
    // entries = rut.toml + the surface (the root file set, prefix "") —
    // the exact shape `bundle_entry_module` reads back at load
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    entries.push(("rut.toml".into(), manifest_bytes));
    let key = bundle_key(rel)?;
    entries.push((key, src.into_bytes()));
    write_bundle(&entries).map_err(|e| e.to_string())
}

/// The `.rutc`-or-source classifier — the packer's ONE group-kind
/// decision, shared by the strip arm and the encode loop. A source
/// body rides compiled iff the graph linked it; a COMPILED body (an
/// archive-backed module, rebased onto this session's numbering by the
/// mount→compile pipeline) always encodes from `units`; a host body
/// has nothing to compile.
fn rides_compiled(
    session: &crate::session::Session,
    spec: &str,
    units: &crate::graph::Units,
) -> bool {
    match session.resolve(spec).map(|m| &m.body) {
        Ok(ModuleBody::Source { .. }) => units.linked.contains_key(spec),
        Ok(ModuleBody::Compiled(_)) => true,
        _ => false,
    }
}

/// Does a compiled program's surface export an OPEN generic surface —
/// the generic-source riding trigger: exported generic fns
/// (`launch_future<T>`), generic type exports (`Vec<T>`), generic
/// methods on an inherent row (`MutCtx::set<A, R>`), or a
/// generic-target impl registration (`impl JsonSerialize for Vec<T>`).
/// The impl arm reads the template law, not fn ids: a generic target's
/// row carries `#`-placeholder fields (the dispatch matcher's own
/// convention), so its field closure names one — a concrete-target impl
/// (`impl Tag for Badge`) never does. A false positive costs a few
/// inert archive entries; a false negative would refuse a shape the
/// ridden source could have served.
fn has_open_generic_surface(prog: &rut_core::binary::Program) -> bool {
    let s = &prog.surface;
    if !s.fn_generics.is_empty()
        || s.type_exports.iter().any(|t| t.is_generic)
        || s.inherents
            .iter()
            .any(|ih| ih.methods.iter().any(|m| !m.generics.is_empty()))
    {
        return true;
    }
    // the generic-target impl arm: resolve the target row through the
    // carried blocks and look for a placeholder in its field closure
    let boot_len = rut_core::types::TypeTable::boot().types.len() as u32;
    let row_of = |id: rut_core::types::TypeId| -> Option<&rut_core::types::RutType> {
        let sc = rut_core::id::scope_of(id);
        let l = rut_core::id::local_of(id);
        let dense = if sc == rut_core::id::BOOT_SCOPE {
            l
        } else {
            let off = s
                .scope_blocks
                .iter()
                .rev()
                .find(|&&(b, _)| b == sc)
                .map(|&(_, off)| off)?;
            boot_len + off + l
        };
        prog.types.types.get(dense as usize)
    };
    let placeholder_in = |id: rut_core::types::TypeId| -> bool {
        let mut stack = vec![id];
        while let Some(t) = stack.pop() {
            let Some(row) = row_of(t) else { continue };
            if is_placeholder(prog.interner.name(row.name)) {
                return true;
            }
            match &row.kind {
                rut_core::types::TyKind::Array { elem }
                | rut_core::types::TyKind::Opt { elem }
                | rut_core::types::TyKind::Weak { elem } => stack.push(*elem),
                rut_core::types::TyKind::Data { fields } => {
                    stack.extend(fields.iter().map(|f| f.ty));
                }
                _ => {}
            }
        }
        false
    };
    s.impls
        .iter()
        .any(|im| placeholder_in(im.target))
}

/// The generic-parameter placeholder spelling (`#<param>` — the `#` is
/// unspellable in rut source), distinguished from the async lane's
/// engine-reserved `#frame@`/`#hframe@`/`#ckpt@` rows: a real
/// placeholder is a bare parameter name, never one of the reserved
/// prefixes.
fn is_placeholder(name: &str) -> bool {
    use rut_core::async_frame::{CKPT_PREFIX, FRAME_PREFIX, HOST_FRAME_PREFIX};
    name.starts_with('#')
        && !name.starts_with(FRAME_PREFIX)
        && !name.starts_with(HOST_FRAME_PREFIX)
        && !name.starts_with(CKPT_PREFIX)
}

/// Emit a generic-owning compiled pkg's riding source under `prefix`
/// (empty for the root, `<pkg>/` for a group): the entry lib, each
/// `entry.libs` file in manifest order, then each `[peer-deps]`
/// descriptor's `lib` group file in peer-name order (the manifest's
/// BTreeMap order — deterministic). Verbatim bytes, one entry per
/// manifest-named path — the same file set a source group rides, minus
/// the manifest (this pkg's manifest already rode).
fn ride_generic_source(
    manifest: &crate::bundle::Manifest,
    prefix: &str,
    entries: &mut Vec<(String, Vec<u8>)>,
    read: &impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let Some(base) = manifest.entry.lib.clone() else {
        return Ok(()); // nothing rides — no body, nothing to recompile from
    };
    let mut push = |rel: &str, entries: &mut Vec<(String, Vec<u8>)>| -> Result<bool, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        if entries.iter().any(|(n, _)| *n == key) {
            return Ok(false); // already riding (an entry.libs overlap) — one entry, one copy
        }
        let text = read(rel)?;
        entries.push((key, text));
        Ok(true)
    };
    push(&base, entries)?;
    for lib in &manifest.entry.libs {
        push(lib, entries)?;
    }
    for desc in manifest.peer_deps.values() {
        if let Some(lib) = desc.get("lib") {
            push(lib, entries)?;
        }
    }
    Ok(())
}

/// A group's parsed manifest, read from wherever its files live.
fn group_manifest(source: &PkgSource, archives: &[Archive]) -> Result<crate::bundle::Manifest, String> {
    match source {
        PkgSource::Dir(gdir) => crate::bundle::read_manifest(gdir, &FsSource),
        PkgSource::Archive { slot, prefix } => {
            let archive = &archives[*slot];
            let text = read_entry(&archive.entries, &format!("{prefix}rut.toml"))
                .map_err(|e| format!("{}: {e}", archive.origin))?;
            parse_manifest(&text).map_err(|e| format!("{}: {prefix}rut.toml: {e}", archive.origin))
        }
    }
}

/// One group file's bytes, read from wherever its files live (a
/// `.rutc`-riding group's manifest / surface text).
fn group_file(
    source: &PkgSource,
    rel: &str,
    archives: &[Archive],
) -> Result<Vec<u8>, String> {
    match source {
        PkgSource::Dir(gdir) => std::fs::read(gdir.join(rel))
            .map_err(|e| format!("cannot read {}: {e}", gdir.join(rel).display())),
        PkgSource::Archive { slot, prefix } => {
            let archive = &archives[*slot];
            let rel = rel.strip_prefix("./").unwrap_or(rel);
            let key = bundle_key(&format!("{prefix}{rel}"))?;
            archive
                .entries
                .iter()
                .find(|(n, _)| n == &key)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| format!("{}: the archive has no entry `{key}`", archive.origin))
        }
    }
}

/// `[pack_dir]` with a url-dep fetcher — the `*_with` lane: collect the
/// url rows, await `dep_fetch` per url sequentially, pack over the
/// bytes map.
pub async fn pack_dir_with(dir: &Path, fetch: &dyn crate::loader::DepRemote) -> Result<Vec<u8>, String> {
    Ok(pack_dir_opts_with(dir, &PackOpts::default(), fetch).await?.0)
}

/// [`pack_dir_opts`] with a url-dep fetcher (the CLI's `--strip` lane).
pub async fn pack_dir_opts_with(
    dir: &Path,
    opts: &PackOpts,
    fetch: &dyn crate::loader::DepRemote,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let map = crate::loader::prefetch_urls(dir, &FsSource, fetch)
        .await
        .map_err(|e| e.to_string())?;
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
) -> Result<(), String> {
    for (spec, desc) in &manifest.deps {
        if !seen.insert(spec.clone()) {
            continue;
        }
        if desc.contains_key("url") {
            // the archive root's location, then every group the same
            // archive contributed (first-mount-wins decided which)
            let Some((slot, _)) = session.archive_mount(spec) else {
                return Err(format!(
                    "dep `{spec}` is declared by url but was not mounted from an archive"
                ));
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
            .ok_or_else(|| format!("dep `{spec}` has no `path`"))?;
        let dep_dir = dir.join(rel);
        let dm = crate::bundle::read_manifest(&dep_dir, &FsSource)?;
        if dm.name.as_deref() != Some(spec.as_str()) {
            return Err(format!(
                "dep `{spec}` points at `{}` — the manifest there names it `{}`",
                dep_dir.display(),
                dm.name.as_deref().unwrap_or("<unnamed>")
            ));
        }
        out.insert(spec.clone(), PkgSource::Dir(dep_dir.clone()));
        collect_group_sources(&dep_dir, &dm, session, out, seen)?;
    }
    Ok(())
}
