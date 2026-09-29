//! The v5 packer: a module directory (+ its whole `[deps]` closure) →
//! one deterministic **compiled** `.rutbundle`. The root and every
//! source dep ride as `.rutc` binaries (bodies + surface — the
//! linking truth); host/decl pkgs ride as their declaration file sets.
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
//! Refusals (never guesses): cycles and name collisions — the graph's
//! own laws, surfaced as pack errors. Source sharing stays what it
//! always was outside bundles: a directory (`rut run <dir>`).

use std::collections::BTreeMap;
use std::path::Path;

use rut_bundle::{collect_source_group, parse_manifest, write_bundle, FsSource};

use crate::graph::compile_units;
use crate::loader::{load_dir_session};
use crate::session::ModuleBody;

/// The v5 bundle layout version — the only one this toolchain packs or
/// loads.
pub const FORMAT_VERSION: u64 = 5;

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

/// Pack a module directory into a deterministic v5 `.rutbundle`.
pub fn pack_dir(dir: &Path) -> Result<Vec<u8>, String> {
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
    // guess): v5 since the compiled-bundle batch.
    if manifest.format.as_deref() != Some("rutbundle") || manifest.format_version != Some(FORMAT_VERSION)
    {
        return Err(format!(
            "{} is not bundle-shaped — add `format = \"rutbundle\"` and `format_version = {FORMAT_VERSION}`",
            dir.join("rut.toml").display()
        ));
    }
    // the closure's session: the [deps] walk + the dev pass + the peer
    // gate, then the engine mounts the closure's code needs (the
    // prelude always; `calc` when a program reaches `Math`)
    let (mut session, root) = load_dir_session(dir)?;
    crate::mount_std(&mut session);
    if root != name {
        return Err(format!(
            "{} names itself `{root}` — expected `{name}`",
            dir.join("rut.toml").display()
        ));
    }
    // compile once per owner: every dep group's unit is the same bytes
    // wherever it is packed, so each dep's own `[dev-deps]` mount too —
    // the presence a declarer's peer groups ride (json's rows ship with
    // json, wherever json travels) — then re-run the gate over the
    // grown closure
    let mut dirs = BTreeMap::new();
    collect_group_dirs(dir, &manifest, &mut dirs, &mut std::collections::BTreeSet::new())?;
    for (spec, gdir) in &dirs {
        let dm = rut_bundle::read_manifest(gdir, &FsSource)?;
        crate::loader::mount_dev_table(&mut session, gdir, &dm)?;
    }
    crate::loader::assemble_peers(&mut session)?;
    let units = compile_units(&session, &root);
    if !units.diags.is_empty() || !units.ok {
        let msgs: Vec<String> = units.diags.iter().map(|d| d.msg.clone()).collect();
        return Err(format!("pack: {}", msgs.join("; ")));
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
    entries.push((format!("{name}.rutc"), rut_core::binary::encode(&units.programs[root_idx])));
    if let Some(rel) = &manifest.entry.type_path {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let text = std::fs::read(dir.join(rel))
            .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
        entries.push((rut_bundle::bundle_key(&format!("{name}.d.rut"))?, text));
    }
    // the dep groups, name order: source pkgs → compiled, host/decl
    // pkgs and declared-but-unused pkgs → the declaration file set
    for (spec, gdir) in &dirs {
        if *spec == root || session.resolve(spec).is_err() {
            continue; // the root rode above; names are already mounted
        }
        let dm = rut_bundle::read_manifest(gdir, &FsSource)?;
        let prefix = format!("{spec}/");
        let compiled = match &session.resolve(spec).unwrap().body {
            ModuleBody::Source { .. } => units.linked.contains_key(spec),
            _ => false, // host pkgs have nothing to compile
        };
        if compiled {
            let &(idx, _) = units.linked.get(spec).unwrap();
            // the group's manifest rides byte-for-byte: the loader reads
            // its name, mount flags, and peer declarations from it
            let toml = std::fs::read(gdir.join("rut.toml"))
                .map_err(|e| format!("cannot read {}: {e}", gdir.join("rut.toml").display()))?;
            entries.push((rut_bundle::bundle_key(&format!("{prefix}rut.toml"))?, toml));
            entries.push((rut_bundle::bundle_key(&format!("{prefix}{spec}.rutc"))?, rut_core::binary::encode(&units.programs[idx])));
            if let Some(rel) = &dm.entry.type_path {
                let rel = rel.strip_prefix("./").unwrap_or(rel);
                let text = std::fs::read(gdir.join(rel))
                    .map_err(|e| format!("cannot read {}: {e}", gdir.join(rel).display()))?;
                entries.push((rut_bundle::bundle_key(&format!("{prefix}{spec}.d.rut"))?, text));
            }
        } else {
            collect_source_group(gdir, &dm, &prefix, &FsSource, &mut entries)?;
        }
    }
    write_bundle(&entries).map_err(|e| e.to_string())
}

/// The `[deps]` graph, recursively — each package under its name,
/// deduplicated, name-checked against its manifest (the packer only
/// ever reads manifest-named paths). Exposed for the packer's
/// compile-once-per-owner dev mounting.
fn collect_group_dirs(
    dir: &Path,
    manifest: &rut_bundle::Manifest,
    out: &mut BTreeMap<String, std::path::PathBuf>,
    seen: &mut std::collections::BTreeSet<String>,
) -> Result<(), String> {
    for (spec, desc) in &manifest.deps {
        if !seen.insert(spec.clone()) {
            continue;
        }
        let rel = desc
            .get("path")
            .ok_or_else(|| format!("dep `{spec}` has no `path`"))?;
        let dep_dir = dir.join(rel);
        let dm = rut_bundle::read_manifest(&dep_dir, &FsSource)?;
        if dm.name.as_deref() != Some(spec.as_str()) {
            return Err(format!(
                "dep `{spec}` points at `{}` — the manifest there names it `{}`",
                dep_dir.display(),
                dm.name.as_deref().unwrap_or("<unnamed>")
            ));
        }
        out.insert(spec.clone(), dep_dir.clone());
        collect_group_dirs(&dep_dir, &dm, out, seen)?;
    }
    Ok(())
}
