//! The driver's pure walk remainder: the pin arithmetic
//! ([`sha256_hex`]), the decoded-archive mount assembly
//! (`walk_decoded` — the per-world half of
//! [`crate::Bundle::parse`](crate::Bundle) + offer), the in-archive
//! entry builder ([`bundle_entry_pkg`]), and the generic-source reader
//! Everything with a file, a path, or a wire
//! lives in `rut-native`; every read here is an in-memory entry.

use std::collections::BTreeMap;

use rut_core::id::ScopeId;

use crate::bundle::{
    fp16_of, bundle_key, parse_identity, parse_manifest_compat, read_entry, GroupKind,
    Layout, Manifest, PkgType, DEPS_DIR,
};
use crate::decl::DECL_CHILD_FILE;
use crate::mods::{
    mount_mod_children, mount_rows, parse_rows, ChildLookup, ModSource, ROWS_NAME,
};
use crate::run::{Walk, RunError};
use crate::session::{Mounted, MountedBody};
use crate::Mode;

/// The hex sha256 of `bytes` — the pin law's arithmetic. Public so a
/// call site can key its cache by url (the CLI does) without a sha2
/// dependency of its own.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Namespace one archive's scope ledger into `recorded`'s numbering and
/// answer the map that rebases the archive's programs with it — the
/// CDN law: per-package bundles are packed INDEPENDENTLY, so their
/// pack-time numberings argue; each archive's rows shift above
/// everything already recorded (boot passes through) and its binaries
/// rebase BEFORE mounting, so every id resolves through its OWN
/// archive's rows. Fails only when the numbering space itself is
/// exhausted (4096 scopes; boot owns 0).
fn namescope_ledger(
    recorded: &BTreeMap<ScopeId, String>,
    scopes: &[(ScopeId, String)],
) -> Result<impl Fn(ScopeId) -> ScopeId, String> {
    let max = recorded.keys().copied().max().unwrap_or(rut_core::id::BOOT_SCOPE);
    let base = max
        .checked_add(1)
        .filter(|next| *next as u32 <= rut_core::id::MAX_SCOPE)
        .ok_or_else(|| "the bundle scope numbering space is exhausted".to_string())?;
    for &(s, _) in scopes {
        if s != rut_core::id::BOOT_SCOPE && base as u32 + s as u32 > rut_core::id::MAX_SCOPE {
            return Err(format!(
                "the bundle's scope ledger reaches scope {s}, which does not fit above this program's {base} — the numbering space is exhausted"
            ));
        }
    }
    Ok(move |s: ScopeId| {
        if s == rut_core::id::BOOT_SCOPE {
            s
        } else {
            base + s
        }
    })
}

/// Resolve one scope-ledger row's spelling to the mount key it binds:
/// the loader's half of the identity law. Every row arrives
/// identity-shaped (`name@fp16` — the reader's gate); the binding is
/// (1) the group the archive carries under that exact identity —
/// per-identity exact dedup makes the key unambiguous; (2) the
/// archive's OWN root, when the identity is the root's own (the
/// self-row — it maps to the bare-named root mount); (3) otherwise the
/// bare name (engine mounts like `core`, dev-table pkgs — pkgs with no
/// rode file set mount bare). Public so rut-native's `mount_url_dep`
/// keeps the ONE law.
pub fn resolve_ledger_row(
    entries: &[(String, Vec<u8>)],
    root_spec: &str,
    root_fp: &str,
    row: &str,
) -> String {
    let Some((name, fp)) = parse_identity(row) else {
        return row.to_string(); // the reader gated the shape; defensive passthrough
    };
    let group_prefix = format!("{DEPS_DIR}/{name}@{fp}/");
    if entries.iter().any(|(n, _)| n.starts_with(&group_prefix)) {
        return row.to_string();
    }
    if name == root_spec && fp == root_fp {
        return root_spec.to_string();
    }
    name.to_string()
}

/// The identity rebind, load side: the archive's binaries spell their
/// pack-time BARE owner names (`inst_fn` rows anchor instantiations on
/// the owner's NAME); the mount keys are content identities. Every
/// bare name this archive binds to EXACTLY ONE group identity rewrites
/// — the group's module name and every `inst_fn` owner spelling the
/// bare name, in the root and every compiled group — so link's
/// owner-anchored keys unify per identity and two identities of one
/// name (the dissolved diamond) never collide. A name bound to SEVERAL
/// identities is the loud refusal while a binary still spells it bare:
/// a bare-name mirror cannot name one — re-pack with distinct dep
/// names.
pub fn identity_rebind_map(group_identities: &[String]) -> (BTreeMap<String, String>, Vec<String>) {
    let mut by_name: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for ident in group_identities {
        if let Some((n, _)) = parse_identity(ident) {
            by_name.entry(n.to_string()).or_default().push(ident);
        }
    }
    let mut map = BTreeMap::new();
    let mut ambiguous = Vec::new();
    for (name, idents) in by_name {
        if idents.len() == 1 {
            map.insert(name, idents[0].clone());
        } else {
            ambiguous.push(name);
        }
    }
    (map, ambiguous)
}

/// Rewrite one program's identity-bound owner names (see
/// [`identity_rebind_map`]). `rename_module` renames the module itself
/// to its mount key (a group rides under its identity; the root keeps
/// its bare name).
pub fn rebind_identity_owners(
    prog: &mut rut_core::binary::Program,
    map: &BTreeMap<String, String>,
    rename_module: bool,
    ambiguous: &[String],
    origin: &str,
) -> Result<(), String> {
    if map.is_empty() && !rename_module {
        return Ok(());
    }
    if rename_module {
        if let Some(ident) = map.get(prog.name.as_str()) {
            prog.name = ident.clone();
        }
    }
    // the owner swap: collect, intern, assign (the interner borrow ends
    // before the rows mutate)
    let mut swaps: Vec<(rut_core::sym::IdentId, String)> = Vec::new();
    for row in &prog.inst_fns {
        let text = prog.interner.name(row.owner).to_string();
        if let Some(ident) = map.get(&text) {
            swaps.push((row.owner, ident.clone()));
        } else if ambiguous.contains(&text) {
            return Err(format!(
                "{origin}: a compiled module's instantiation rows spell `{text}`, but this \
                 archive carries several identities of that name ({}) — bare-name mirrors \
                 cannot name one; re-pack with distinct dep names",
                ambiguous.join(", "),
            ));
        }
    }
    if swaps.is_empty() {
        return Ok(());
    }
    let mut ids: BTreeMap<String, rut_core::sym::IdentId> = BTreeMap::new();
    for (_, ident) in &swaps {
        ids.entry(ident.clone()).or_insert_with(|| prog.interner.intern(ident));
    }
    for row in &mut prog.inst_fns {
        let text = prog.interner.name(row.owner).to_string();
        if let Some(ident) = map.get(&text) {
            row.owner = ids[ident];
        }
    }
    Ok(())
}

/// The decoded archive's mount assembly, per world: entries + layout
/// (already CRC-verified and decode-verified at
/// [`crate::Bundle::parse`]) → the walked yield ([`Walk`]) — the
/// scope-ledger namespacing, the identity rebind, the group mounts.
/// No filesystem, no network: the same contract a url dep rides.
/// This is the per-offer half; the decode half ran once at parse.
pub(crate) fn walk_decoded(
    entries: &[(String, Vec<u8>)],
    layout: Layout,
) -> Result<Walk, RunError> {
    let origin = "bundle";
    let err = |msg: String| RunError::law(format!("{origin}: {msg}"));
    let mut pkgs: BTreeMap<String, Mounted> = BTreeMap::new();
    let (manifest, root, scopes, groups) = match layout {
        Layout::Compiled { manifest, root, scopes, groups } => (manifest, root, scopes, groups),
        Layout::Decl { manifest, .. } => {
            // the decl root: a host pkg — the surface mounts as the
            // pkg's host rows through `bundle_entry_pkg` (the lane a
            // host group rides inside a bundle). Single-package: no
            // groups, no ledger, no closure — a LEAF, done.
            let root_spec = manifest
                .name
                .clone()
                .ok_or_else(|| err("rut.jsonc has no `name`".into()))?;
            let mut pkg = bundle_entry_pkg(entries, "", &manifest)
                .map_err(|e| RunError::law(format!("{origin}: {e}")))?;
            pkg.spec = root_spec.clone();
            pkgs.insert(root_spec.clone(), pkg);
            return Ok(Walk { pkgs: pkgs.into_values().collect(), root: root_spec });
        }
    };
    let root_spec = manifest
        .name
        .clone()
        .ok_or_else(|| err("rut.jsonc has no `name`".into()))?;
    // the identity rebind map: the group identities this archive
    // carries, by bare name — the owner-name law's inputs
    let group_idents: Vec<String> = groups
        .iter()
        .filter_map(|(p, _)| p.strip_prefix(&format!("{DEPS_DIR}/")).map(str::to_string))
        .collect();
    let (rebind, ambiguous) = identity_rebind_map(&group_idents);
    // the root's own content identity — the self-row's binding reads
    // it (the rode source set at the archive root)
    let root_fp = fp16_of(entries, "");
    // the scope ledger first, namespaced: the rows shift into a fresh
    // range (the container walk starts from an empty ledger — boot
    // passes through) and the root's program rebases with the same map
    // BEFORE mounting — the graph later resolves every decoded foreign
    // id through these rows. Each row's spelling resolves to its mount
    // key (the group identity, the bare root, or the bare name).
    let recorded: BTreeMap<ScopeId, String> = BTreeMap::new();
    let remap = namescope_ledger(&recorded, &scopes).map_err(err)?;
    let scopes: Vec<(ScopeId, String)> = scopes
        .into_iter()
        .map(|(s, spec)| {
            let key = resolve_ledger_row(entries, &root_spec, &root_fp, &spec);
            (remap(s), key)
        })
        .collect();
    let root = rut_core::link::rebase(root, &remap);
    // the identity rebind, root flavor: the root's mirrors name their
    // pack-time owners bare — rebind to the identities (the root's own
    // module name stays bare: that IS its mount key)
    let mut root = root;
    rebind_identity_owners(&mut root, &rebind, false, &ambiguous, origin)
        .map_err(err)?;
    // the root: a compiled module (the packer refuses any other root).
    // The mount seam's anti-doctoring law (v22): a generic-owning unit
    // must carry its template bodies INSIDE the binary — a generic
    // surface with an empty template section is a torn or doctored
    // bundle (the pack could not have produced it). Non-generic units
    // pass trivially.
    if crate::pack::has_generic_bodies(&root) && root.templates.is_empty() {
        return Err(err(format!(
            "`{root_spec}` owns an open generic surface but its binary carries no template \
             bodies — corrupt or doctored, re-pack the directory"
        )));
    }
    let mods = rows_mods(entries, "").map_err(err)?;
    pkgs.insert(
        root_spec.clone(),
        Mounted {
            spec: root_spec.clone(),
            body: MountedBody::Compiled(root),
            entry: manifest.entry.clone(),
            mods,
            bundle_scopes: scopes.clone(),
            manifest: manifest.clone(),
            ..Default::default()
        },
    );
    // the archive's group prefixes (root "" first) — the identity map
    // for the rows' stamping below
    let mut prefixes: BTreeMap<String, String> = BTreeMap::new();
    prefixes.insert(root_spec.clone(), String::new());
    for (prefix, kind) in &groups {
        let dep_toml = read_entry(entries, &format!("{prefix}/rut.jsonc"))
            .map_err(|e| RunError::law(format!("{origin}: {e}")))?;
        // the group manifest rides the compat lane: a repacked
        // archive's group manifests ride byte-for-byte from any era
        let dm = parse_manifest_compat(&dep_toml)
            .map_err(|e| RunError::law(format!("{origin}: {prefix}/rut.jsonc: {e}")))?;
        let name = dm
            .name
            .clone()
            .ok_or_else(|| err(format!("{prefix}/rut.jsonc has no `name`")))?;
        // the mount key IS the content identity — the reader validated
        // the dirname's shape, name-vs-manifest, and fingerprint-over-
        // content, so two identities of one name mount side by side
        // and the same identity mounts once (exact dedup)
        let key = prefix
            .strip_prefix(&format!("{DEPS_DIR}/"))
            .map(str::to_string)
            .unwrap_or_else(|| name.clone());
        if pkgs.contains_key(&key) {
            continue; // first mount wins — per-identity exact dedup
        }
        let pkg = match kind {
            GroupKind::Compiled(program) => {
                // rebase first, THEN check own-vs-ledger — both sides
                // of the comparison shift by the same map, so the law
                // (the row must be the scope the binary itself
                // carries) is unchanged, and the stored binary is the
                // namespaced one
                                let mut program = rut_core::link::rebase(program.clone(), &remap);
                // the module rides under its MOUNT KEY and its owner
                // rows rebind to the identities (the linker merges by
                // module name and anchors instantiations on owner
                // names — two identities of one name coexist)
                rebind_identity_owners(&mut program, &rebind, true, &ambiguous, origin)
                    .map_err(err)?;
                // the ledger must name the group's identity, and the
                // row must be the scope the binary itself carries
                // (refuse, never guess — a mismatch is a corrupt or
                // doctored bundle)
                let Some(own) = rut_core::link::own_scope(&program) else {
                    return Err(err(format!(
                        "{prefix}/{}: the program carries no scope blocks",
                        name
                    )));
                };
                match scopes.iter().find(|(_, s)| s == &key) {
                    Some(&(row, _)) if row == own => {}
                    Some(&(row, _)) => {
                        return Err(err(format!(
                            "`{key}`'s ledger row says scope {row}, but its binary carries {own}"
                        )));
                    }
                    None => {
                        return Err(err(format!(
                            "the scope ledger does not name `{key}` — the bundle is incomplete"
                        )));
                    }
                }
                if crate::pack::has_generic_bodies(&program) && program.templates.is_empty() {
                    return Err(err(format!(
                        "`{key}` owns an open generic surface but its binary carries no template \
                         bodies — corrupt or doctored, re-pack the directory"
                    )));
                }
                Mounted {
                    spec: key.clone(),
                    body: MountedBody::Compiled(program),
                    entry: dm.entry.clone(),
                    mods: rows_mods(entries, &format!("{prefix}/")).map_err(err)?,
                    manifest: dm.clone(),
                    // the first-mount edge: the archive root pulled its groups
                    mounted_by: Some(root_spec.clone()),
                    ..Default::default()
                }
            }
            GroupKind::Source => {
                let mut pkg = bundle_entry_pkg(entries, &format!("{prefix}/"), &dm)
                    .map_err(|e| RunError::law(format!("{origin}: {e}")))?;
                pkg.spec = key.clone();
                // the first-mount edge: the archive root pulled its groups
                pkg.mounted_by = Some(root_spec.clone());
                pkg
            }
        };
        pkgs.insert(key.clone(), pkg);
        prefixes.insert(key.clone(), format!("{prefix}/"));
    }
    // every declared dep must be satisfied by a group — by the dep's
    // BARE name: a group mounts under its identity, and the manifest's
    // deps keys stay bare (source-facing grammar)
    for spec in manifest.deps.keys() {
        if !pkgs.keys().any(|k| crate::bundle::mount_name(k) == spec.as_str()) {
            return Err(err(format!(
                "the bundle is missing its `{spec}` dependency group"
            )));
        }
    }
    Ok(Walk { pkgs: pkgs.into_values().collect(), root: root_spec })
}

/// The packed-container walk: a packed `.rutbundle`'s bytes → the
/// walked yield ([`Walk`]). The retired spelling of the load stage's
/// decode + mount pair — one shot per call. Decode ONCE and reuse the
/// handle instead: `Bundle::parse(bytes)` then `run.pkg(&bundle)` (or
/// `bundle.walk()` for the raw mount assembly).
#[deprecated = "decode once, offer many: `Bundle::parse(bytes)` then \
                `run.pkg(&bundle)` — the handle relocates + mounts per \
                world without re-decoding"]
pub fn bundle_walk_bytes(bytes: &[u8]) -> Result<Walk, RunError> {
    let bundle = crate::run::Bundle::parse(bytes)?;
    bundle.walk()
}

/// The rows lane: the archive's `rut.mods` entry at `prefix` — the
/// additive envelope section carrying a package's module set as
/// path-keyed source rows (`""` = the root source, `<path>` = each
/// child). `Ok(None)` when the archive carries no rows entry — the old
/// flat envelope (every published bundle to date), which loads
/// unchanged. The mounted children ride the same declaration-driven
/// mount a directory would; a carried-but-undeclared row is refused.
fn rows_body(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
) -> Result<Option<(String, BTreeMap<String, ModSource>)>, String> {
    let key = bundle_key(&format!("{prefix}{ROWS_NAME}"))?;
    let text = match read_entry(entries, &key) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let (root, rows) = parse_rows(&text)?;
    let mods = mount_rows(&root, &rows, Mode::Impl, "mod.rut")?;
    Ok(Some((root, mods)))
}

/// The rows lane's HOST flavor: the same envelope over a `type =
/// "host"` pkg's decl tree — `""` is the root `.d.rut`, `<path>` each
/// child, mounted with the Decl flavor (`mod.d.rut` spellings) and
/// read back as the mounted tree the lowering takes. `Ok(None)` when
/// the archive carries no rows entry — a flat host bundle (the shape
/// every pre-`mod` host pack has), which loads exactly as it did.
fn decl_rows_body(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
) -> Result<Option<(String, BTreeMap<String, ModSource>)>, String> {
    let key = bundle_key(&format!("{prefix}{ROWS_NAME}"))?;
    let text = match read_entry(entries, &key) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let (root, rows) = parse_rows(&text)?;
    let mods = mount_rows(&root, &rows, Mode::Decl, DECL_CHILD_FILE)?;
    Ok(Some((root, mods)))
}

/// The rows entry's mounted children only (the compiled arms' share —
/// their body is the binary; the tree still mounts beside it).
pub fn rows_mods(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
) -> Result<BTreeMap<String, ModSource>, String> {
    Ok(rows_body(entries, prefix)?.map(|(_, mods)| mods).unwrap_or_default())
}

/// The archive-entries child lookup for the mod mount — the same case
/// law the FS lane answers, over entry keys: the directory form
/// `NAME/mod.rut` first, else the single-file `NAME.rut`, both present
/// the loud ambiguity. A bundle is DIRECTORY-FORM only (the packer
/// mounts rows; loose file entries never mount as modules), so the
/// single-file spelling falling alone is the loud missing error — the
/// dir-form fix named. Entries are archive-relative and canonical —
/// the cycle guard keys on the key itself.
fn entry_child_lookup<'a>(
    entries: &'a [(String, Vec<u8>)],
    prefix: &'a str,
) -> impl FnMut(&str, &str) -> Result<ChildLookup, String> + 'a {
    move |parent: &str, name: &str| {
        let base = if parent.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix}{parent}/")
        };
        let key = bundle_key(&format!("{base}{name}/mod.rut"))?;
        let file = bundle_key(&format!("{base}{name}.rut"))?;
        let has_dir = read_entry(entries, &key).is_ok();
        let has_file = entries.iter().any(|(n, _)| *n == file);
        if has_dir && has_file {
            return Ok(ChildLookup::Ambiguous);
        }
        if has_dir {
            return Ok(ChildLookup::Found { text: read_entry(entries, &key)?, canon: key, file: "mod.rut".to_string() });
        }
        // the single-file entry alone: not a module here — bundles are
        // directory-form (the loud misses name the dir-form fix)
        let dir = bundle_key(&format!("{base}{name}/"))?;
        if entries.iter().any(|(n, _)| n.starts_with(&dir)) {
            return Ok(ChildLookup::NoModRut);
        }
        Ok(ChildLookup::Missing)
    }
}

/// Build a package's entry [`Mounted`] from bundle entries under
/// `prefix` (empty for the root, `<pkg>/` for a dep group) — the
/// in-archive counterpart of a directory mount: the declared kind
/// dispatches (a `type = "host"` pkg is a decl surface whose mod tree
/// rides the rows envelope; a `type = "lib"` pkg with a surface and no
/// body is the surface-only dev state). The manifest's
/// `namespace`/`consts` rows ride here too — the manifest is their
/// only spelling.
pub fn bundle_entry_pkg(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Mounted, String> {
    // the host lane: the decl tree rides the rows envelope when the
    // tree mounts (the packer's host lane), else the flat surface
    // entry lowers alone — the byte-compat shape
    if manifest.pkg_type == PkgType::Host {
        return host_entry_pkg(entries, prefix, manifest);
    }
    let read = |rel: &str| -> Result<String, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        read_entry(entries, &key)
    };
    // the rows lane first: a new-format module set mounts whole (the
    // `""` row is the root source; children mount by declaration) —
    // regardless of the entry keys the manifest spells
    if let Some((root, mods)) = rows_body(entries, prefix)? {
        return Ok(Mounted {
            body: MountedBody::Source { text: root, is_decl: false },
            entry: manifest.entry.clone(),
            mods,
            manifest: manifest.clone(),
            ..Default::default()
        });
    }
    // a lib pkg from here: the surface's host-row refusal (the
    // lib-surface law), then the body — the `mod.rut` entry, else the
    // OLD envelope's legacy keys, else the surface-only dev state (a
    // decl unit — no host rows, nothing exported; use sites
    // resolve-miss, correctly). An ambiguity — `entry.type` spelled,
    // no declared kind, no body anywhere — refuses, naming both fixes.
    // The surface read is the ONE canonical name (`mod.d.rut`, wire
    // v11): the directory grammar's spelling never rides the archive.
    let pkg = if let Some(_) = &manifest.entry.type_path {
        let origin = format!("{prefix}{}", crate::decl::DECL_CHILD_FILE);
        let src = read(crate::decl::DECL_CHILD_FILE)?;
        crate::decl::refuse_host_rows(&src, &origin)?;
        match root_source(entries, prefix, manifest)? {
            Some(root) => plain_source_pkg(entries, prefix, manifest, root)?,
            None if !manifest.type_declared => {
                return Err(format!(
                    "module at bundle prefix `{prefix}` spells `entry.type` but no kind — \
                     `\"type\": \"host\"` for a host pkg, or `\"type\": \"lib\"` for the \
                     surface-only dev state"
                ));
            }
            None => Mounted {
                body: MountedBody::Source { text: src, is_decl: true },
                entry: manifest.entry.clone(),
                manifest: manifest.clone(),
                ..Default::default()
            },
        }
    } else {
        plain_entry_pkg(entries, prefix, manifest)?
    };
    Ok(pkg)
}

/// The host lane of [`bundle_entry_pkg`]: the decl tree rides the rows
/// envelope when the tree mounts (the packer's host lane — `""` is the
/// root `.d.rut`, each child its mod path), else the flat surface
/// entry lowers alone — the root file is `mod.d.rut`, the ONE
/// canonical name the packer emits at both levels. The manifest's
/// `namespace`/`consts` rows fold in either way.
fn host_entry_pkg(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Mounted, String> {
    let fold = |mut m: Mounted| -> Mounted {
        m.entry = manifest.entry.clone();
        m.namespace = manifest.namespace.clone();
        m.manifest = manifest.clone();
        let consts = manifest
            .consts
            .iter()
            .map(|(name, v)| (name.clone(), rut_core::types::TY_F64, v.to_bits()))
            .collect();
        if let MountedBody::Host { consts: rows, .. } = &mut m.body {
            *rows = consts;
        }
        m
    };
    if let Some((root, children)) = decl_rows_body(entries, prefix)? {
        let rel = manifest.entry.type_path.as_deref().unwrap_or(DECL_CHILD_FILE);
        let origin = format!("{prefix}{}", rel.strip_prefix("./").unwrap_or(rel));
        let m = crate::decl::lower_decl_module(&root, &children, &origin)?;
        return Ok(fold(m));
    }
    // the flat lane's root file is `mod.d.rut` beside the manifest —
    // the ONE canonical name (wire v11 canonicalizes the directory
    // grammar's `entry.type` spelling; the in-archive override lane is
    // gone). A packed host bundle always carries a root — a manifest
    // whose archive names none is a packaging bug, named like the
    // directory lanes'.
    let key = bundle_key(&format!("{prefix}{DECL_CHILD_FILE}"))?;
    let src = read_entry(entries, &key).map_err(|_| {
        format!(
            "module at bundle prefix `{prefix}` is a host pkg with no entry — the root surface \
             is `mod.d.rut` beside the manifest"
        )
    })?;
    let origin = format!("{prefix}{DECL_CHILD_FILE}");
    let m = crate::decl::lower_decl_module(&src, &Default::default(), &origin)?;
    Ok(fold(m))
}

/// The plain (non-decl) entry pkg. The root module is the `mod.rut`
/// ENTRY beside the manifest; an OLD published bundle (the flat
/// envelope — the pinned CDN tags are forever old) spells the repealed
/// keys, and its manifest rides the compat lane, so the legacy lib +
/// `libs` splice reads exactly as it always did. Either way the
/// source's `mod` declarations mount the child tree from the archive,
/// recursively, cycle-guarded.
fn plain_entry_pkg(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Mounted, String> {
    let src = match root_source(entries, prefix, manifest)? {
        Some(src) => src,
        None => {
            return Err(format!(
                "module at bundle prefix `{prefix}` has no entry — the root module is \
                 `mod.rut` beside the manifest"
            ));
        }
    };
    plain_source_pkg(entries, prefix, manifest, src)
}

/// The bundle mount's body reader, ONE dispatch: the `mod.rut` entry
/// (the post-repeal envelope — a source group's file set carries it,
/// so does a generic root's riding source), else the OLD envelope's
/// legacy keys read + spliced ('\n'-joined, the flat envelope's
/// law), else `None` (no body anywhere).
fn root_source(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Option<String>, String> {
    let read = |rel: &str| -> Result<String, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        read_entry(entries, &key)
    };
    if let Ok(root) = read("mod.rut") {
        return Ok(Some(root));
    }
    let Some(base) = &manifest.legacy_entry.lib else {
        return Ok(None);
    };
    let mut src = read(base)?;
    for lib in &manifest.legacy_entry.libs {
        src.push('\n');
        src.push_str(&read(lib)?);
    }
    Ok(Some(src))
}

/// A source pkg from its assembled root text: the `mod` declarations
/// mount the child tree from the archive — exactly what a directory
/// mount would read back.
fn plain_source_pkg(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
    src: String,
) -> Result<Mounted, String> {
    let mut lookup = entry_child_lookup(entries, prefix);
    let mods = mount_mod_children(&src, &format!("\u{0}{prefix}"), Mode::Impl, "mod.rut", &mut lookup)?;
    Ok(Mounted {
        body: MountedBody::Source { text: src, is_decl: false },
        entry: manifest.entry.clone(),
        mods,
        manifest: manifest.clone(),
        ..Default::default()
    })
}
