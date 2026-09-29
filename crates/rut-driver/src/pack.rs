//! The v5 packer: a module directory (+ its whole `[deps]` closure) →
//! one deterministic **compiled** `.rutbundle`. The root and every
//! linkable dep ride as `.rutc` binaries (bodies + surface — the
//! linking truth); explicitly `inline`d deps and host pkgs ride as
//! source file sets — with instantiation owner-anchored, a generic
//! export links and its consumers request the instantiations — and the
//! pack-time scope ledger lets a loader rebase every decoded program
//! onto its own numbering.
//!
//! The packer lives in the driver because it needs the compiler: it
//! loads the directory's session, walks the graph once
//! ([`compile_units`]), and classifies each package with the graph's
//! own splice law ([`linkable`] — there is no second implementation).
//! The bytes come back; writing the output file stays with the caller
//! (the CLI). Same input directory ⇒ byte-identical bundle.
//!
//! Refusals (never guesses): a root that cannot link, a would-be
//! compiled group declaring `[peer-deps]` lib files, and a manifest
//! that is not bundle-shaped v5. Source sharing stays what it always
//! was outside bundles: a directory (`rut run <dir>`).

use std::collections::BTreeMap;
use std::path::Path;

use rut_bundle::{collect_source_group, parse_manifest, write_bundle, FsSource};

use crate::graph::{compile_units, linkable, Linkability};
use crate::loader::load_dir_session;
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
    let units = compile_units(&session, &root);
    if !units.diags.is_empty() || !units.ok {
        let msgs: Vec<String> = units.diags.iter().map(|d| d.msg.clone()).collect();
        return Err(format!("pack: {}", msgs.join("; ")));
    }
    // the root must be linkable — an explicitly inlined pkg carries no
    // publishable boundary (its whole source is its interface)
    let root_linked = units.linked.get(&root);
    let root_ok = match (&session.resolve(&root).unwrap().body, root_linked) {
        (ModuleBody::Source { .. }, Some(&(idx, _))) => {
            linkable(&units.programs[idx], manifest.inline) == Linkability::Linkable
        }
        _ => false,
    };
    if !root_ok {
        return Err(format!(
            "pack: {root} is inline — its source is its interface and it cannot be published compiled; share the directory instead"
        ));
    }
    // the dep groups: the manifest's [deps] walk (recursively,
    // deduplicated by package name, name-checked) — the same set the
    // source packer carried, now classified per package
    let mut groups_tree = BTreeMap::new();
    collect_group_dirs(dir, &manifest, &mut groups_tree, &mut std::collections::BTreeSet::new())?;
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
    let root_idx = root_linked.unwrap().0;
    entries.push((format!("{name}.rutc"), rut_core::binary::encode(&units.programs[root_idx])));
    if let Some(rel) = &manifest.entry.type_path {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let text = std::fs::read(dir.join(rel))
            .map_err(|e| format!("cannot read {}: {e}", dir.join(rel).display()))?;
        entries.push((rut_bundle::bundle_key(&format!("{name}.d.rut"))?, text));
    }
    // the dep groups, name order — mixed kinds: linkable → compiled,
    // splice-needed / host / unused → the source file set
    for (spec, gdir) in &groups_tree {
        if *spec == root || session.resolve(spec).is_err() {
            continue; // the root rode above; names are already mounted
        }
        let dm = rut_bundle::read_manifest(gdir, &FsSource)?;
        let prefix = format!("{spec}/");
        let compiled = match &session.resolve(spec).unwrap().body {
            ModuleBody::Source { .. } => match units.linked.get(spec) {
                Some(&(idx, _)) => {
                    linkable(&units.programs[idx], dm.inline) == Linkability::Linkable
                }
                None => false, // mounted but never ensured — rides as source
            },
            _ => false, // host pkgs have nothing to compile
        };
        if compiled {
            // a compiled group may not declare `[peer-deps]` lib files
            // (appending source into a compiled pkg is impossible —
            // the v1-precedent refusal)
            if dm.peer_deps.values().any(|d| d.contains_key("lib")) {
                return Err(format!(
                    "pack: {spec} declares `[peer-deps]` lib group files — a linkable pkg cannot be published compiled while its groups append source; share the directory instead"
                ));
            }
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
/// ever reads manifest-named paths).
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
