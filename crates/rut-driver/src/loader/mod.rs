//! The driver's pure walk remainder: the pin arithmetic
//! ([`sha256_hex`]), the packed-container walk ([`bundle_walk_bytes`],
//! the engine of [`Pkg::from_bundle`] and rut-native's
//! `load_bundle_session`), the in-archive entry builder
//! ([`bundle_entry_pkg`]), the generic-source reader
//! ([`riding_gen_source`]), and the PURE peer gate ([`peer_gate`]).
//! Everything with a file, a path, or a wire lives in `rut-native`;
//! every read here is an in-memory entry.

use std::collections::BTreeMap;

use rut_core::id::ScopeId;

use crate::bundle::{
    bundle_key, entry_rel, parse_manifest, read_entry, Bundle, GroupKind, Layout, Manifest,
    PkgType,
};
use crate::run::{Loaded, RunError};
use crate::session::{Pkg, PkgBody, PeerDecl};

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

/// The pure container walk: a packed `.rutbundle`'s bytes → the walked
/// yield ([`Loaded`]) — gates, groups, the scope ledger, and the ONE
/// peer gate over the archive's own pkgs. No filesystem, no network:
/// the same contract a url dep rides. [`Pkg::from_bundle`] is its
/// public face; rut-native's `load_bundle_session` adds only the file
/// read.
pub fn bundle_walk_bytes(bytes: &[u8]) -> Result<Loaded, RunError> {
    let origin = "bundle";
    let err = |msg: String| RunError::law(format!("{origin}: {msg}"));
    // gate 1: the container — every entry's CRC-32 verified
    let bundle = Bundle::parse(bytes)
        .map_err(|e| RunError::law(format!("{origin}: {e}")))?;
    // gate 2 + 3: the manifest/version, then every group's decode +
    // verification — the layout parse refuses anything it cannot
    // decode, so a bad binary never reaches the table
    let layout =
        Layout::parse(&bundle).map_err(|e| RunError::law(format!("{origin}: {e}")))?;
    let mut pkgs: BTreeMap<String, Pkg> = BTreeMap::new();
    let (manifest, root, scopes, groups) = match layout {
        Layout::Compiled { manifest, root, scopes, groups } => (manifest, root, scopes, groups),
        Layout::Decl { manifest, .. } => {
            // the decl root: a host pkg — the surface mounts as the
            // pkg's host rows through `bundle_entry_pkg` (the lane a
            // host group rides inside a v7 bundle). Single-package: no
            // groups, no ledger, no closure — a LEAF, done.
            let entries = bundle.entries();
            let root_spec = manifest
                .name
                .clone()
                .ok_or_else(|| err("rut.jsonc has no `name`".into()))?;
            let pkg = fold_peers(
                bundle_entry_pkg(entries, "", &manifest)
                    .map_err(|e| RunError::law(format!("{origin}: {e}")))?,
                &manifest,
                archive_peer_libs(entries, "", &manifest).map_err(err)?,
            );
            let mut pkg = pkg;
            pkg.spec = root_spec.clone();
            pkgs.insert(root_spec.clone(), pkg);
            return Ok(Loaded { pkgs: pkgs.into_values().collect(), root: root_spec });
        }
    };
    let entries = bundle.entries();
    let root_spec = manifest
        .name
        .clone()
        .ok_or_else(|| err("rut.jsonc has no `name`".into()))?;
    // the scope ledger first, namespaced: the rows shift into a fresh
    // range (the container walk starts from an empty ledger — boot
    // passes through) and the root's program rebases with the same map
    // BEFORE mounting — the graph later resolves every decoded foreign
    // id through these rows
    let recorded: BTreeMap<ScopeId, String> = BTreeMap::new();
    let remap = namescope_ledger(&recorded, &scopes).map_err(err)?;
    let scopes: Vec<(ScopeId, String)> =
        scopes.into_iter().map(|(s, spec)| (remap(s), spec)).collect();
    let root = rut_core::link::rebase(root, &remap);
    // the root: a compiled module (the packer refuses any other root).
    // A generic-owning root rides its source beside the binary — the
    // on-demand recompile's input (generic-source riding).
    let gen_source = riding_gen_source(entries, "", &manifest).map_err(err)?;
    let libs = archive_peer_libs(entries, "", &manifest).map_err(err)?;
    pkgs.insert(
        root_spec.clone(),
        Pkg {
            spec: root_spec.clone(),
            body: PkgBody::Compiled(root),
            entry: manifest.entry.clone(),
            gen_source,
            peers: manifest_peers(&manifest),
            peer_libs: libs,
            bundle_scopes: scopes.clone(),
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
        let dm = parse_manifest(&dep_toml)
            .map_err(|e| RunError::law(format!("{origin}: {prefix}/rut.jsonc: {e}")))?;
        let name = dm
            .name
            .clone()
            .ok_or_else(|| err(format!("{prefix}/rut.jsonc has no `name`")))?;
        if pkgs.contains_key(&name) {
            continue; // first mount wins (the root, an earlier group)
        }
        let pkg = match kind {
            GroupKind::Compiled(program) => {
                // rebase first, THEN check own-vs-ledger — both sides
                // of the comparison shift by the same map, so the law
                // (the row must be the scope the binary itself
                // carries) is unchanged, and the stored binary is the
                // namespaced one
                let program = rut_core::link::rebase(program.clone(), &remap);
                // the ledger must name the group, and the row must be
                // the scope the binary itself carries (refuse, never
                // guess — a mismatch is a corrupt or doctored bundle)
                let Some(own) = rut_core::link::own_scope(&program) else {
                    return Err(err(format!(
                        "{prefix}/{}: the program carries no scope blocks",
                        name
                    )));
                };
                match scopes.iter().find(|(_, s)| s == &name) {
                    Some(&(row, _)) if row == own => {}
                    Some(&(row, _)) => {
                        return Err(err(format!(
                            "`{name}`'s ledger row says scope {row}, but its binary carries {own}"
                        )));
                    }
                    None => {
                        return Err(err(format!(
                            "the scope ledger does not name `{name}` — the bundle is incomplete"
                        )));
                    }
                }
                let gen_source = riding_gen_source(entries, &format!("{prefix}/"), &dm)
                    .map_err(err)?;
                let libs = archive_peer_libs(entries, &format!("{prefix}/"), &dm).map_err(err)?;
                Pkg {
                    spec: name.clone(),
                    body: PkgBody::Compiled(program),
                    entry: dm.entry.clone(),
                    gen_source,
                    peers: manifest_peers(&dm),
                    peer_libs: libs,
                    ..Default::default()
                }
            }
            GroupKind::Source => {
                let mut pkg = fold_peers(
                    bundle_entry_pkg(entries, &format!("{prefix}/"), &dm)
                        .map_err(|e| RunError::law(format!("{origin}: {e}")))?,
                    &dm,
                    archive_peer_libs(entries, &format!("{prefix}/"), &dm).map_err(err)?,
                );
                pkg.spec = name.clone();
                pkg
            }
        };
        pkgs.insert(name.clone(), pkg);
        prefixes.insert(name.clone(), format!("{prefix}/"));
    }
    // every declared dep must be satisfied by a group
    for spec in manifest.deps.keys() {
        if !pkgs.contains_key(spec) {
            return Err(err(format!(
                "the bundle is missing its `{spec}` dependency group"
            )));
        }
    }
    // the archive's rows ride ITS ROOT — the run chain's close-of-world
    // replays the archive from them (grouping the members, shifting the
    // numbering); group members stay row-free, so the graph's
    // compiled-mount arm sees exactly the pack-time shape
    peer_gate(&mut pkgs)?;
    Ok(Loaded { pkgs: pkgs.into_values().collect(), root: root_spec })
}

/// A manifest's `[peer-deps]` as the pkg's own declaration map — the
/// walk reads every mounted pkg's manifest anyway, so folding the
/// table into the pkg costs no extra I/O. The peer gate reads it
/// post-closure; the reference-site D2 diagnostic resolves against it.
pub(crate) fn manifest_peers(manifest: &Manifest) -> BTreeMap<String, PeerDecl> {
    manifest
        .peer_deps
        .iter()
        .map(|(peer, desc)| (peer.clone(), PeerDecl::of(desc)))
        .collect()
}

/// A manifest's declared peer-INTEGRATION texts, read from archive
/// entries at `prefix` — the mount's half of the peer gate's input
/// (see [`Pkg::peer_libs`]). A missing entry records `None`: the gate
/// errors only when the peer is present (an absent optional peer stays
/// inert, read or not).
fn archive_peer_libs(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<BTreeMap<String, Option<String>>, String> {
    let mut out = BTreeMap::new();
    for (peer, desc) in &manifest.peer_deps {
        let Some(lib) = desc.get("lib") else {
            continue; // presence declared, no integration file to mount
        };
        let rel = lib.strip_prefix("./").unwrap_or(lib);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        let text = match read_entry(entries, &key) {
            Ok(t) => Some(t),
            Err(_) => None,
        };
        out.insert(peer.clone(), text);
    }
    Ok(out)
}

/// [`manifest_peers`] + the integration texts folded into a walked pkg
/// — the constructor + fold pair the entry lanes share.
pub(crate) fn fold_peers(
    mut pkg: Pkg,
    manifest: &Manifest,
    libs: BTreeMap<String, Option<String>>,
) -> Pkg {
    pkg.peers.extend(manifest_peers(manifest));
    pkg.peer_libs.extend(libs);
    pkg
}

/// Build a package's entry [`Pkg`] from bundle entries under
/// `prefix` (empty for the root, `<pkg>/` for a dep group) — the
/// in-archive counterpart of a directory mount: the declared kind
/// dispatches (a `type = "host"` pkg is a decl surface; a `type =
/// "lib"` pkg with a surface and no body is the surface-only dev
/// state). The manifest's `namespace`/`consts` rows ride here too —
/// the manifest is their only spelling.
pub fn bundle_entry_pkg(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Pkg, String> {
    let read = |rel: &str| -> Result<String, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        read_entry(entries, &key)
    };
    let pkg = match manifest.pkg_type {
        PkgType::Host => {
            let rel = manifest.entry.type_path.as_ref().ok_or_else(|| {
                format!("module at bundle prefix `{prefix}` is a host pkg with no `entry.type`")
            })?;
            let src = read(rel)?;
            let mut m = crate::decl::lower_decl_module(&src, &format!("{prefix}{rel}"))?;
            m.entry = manifest.entry.clone();
            m.namespace = manifest.namespace.clone();
            let consts = manifest
                .consts
                .iter()
                .map(|(name, v)| (name.clone(), rut_core::types::TY_F64, v.to_bits()))
                .collect();
            if let PkgBody::Host { consts: rows, .. } = &mut m.body {
                *rows = consts;
            }
            m
        }
        PkgType::Lib => {
            if let Some(rel) = &manifest.entry.type_path {
                let origin = format!("{prefix}{}", rel.strip_prefix("./").unwrap_or(rel));
                let src = read(rel)?;
                crate::decl::refuse_host_rows(&src, &origin)?;
                if manifest.entry.lib.is_none() {
                    // the surface-only dev state: a decl unit — no host
                    // rows, nothing exported (use sites resolve-miss,
                    // correctly)
                    Pkg {
                        body: PkgBody::Source { text: src, is_decl: true },
                        entry: manifest.entry.clone(),
                        ..Default::default()
                    }
                } else {
                    plain_entry_pkg(entries, prefix, manifest)?
                }
            } else {
                plain_entry_pkg(entries, prefix, manifest)?
            }
        }
    };
    Ok(pkg)
}

/// The plain (non-decl) entry pkg: the entry lib + `entry.libs`
/// spliced — ONE source string.
fn plain_entry_pkg(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Pkg, String> {
    let read = |rel: &str| -> Result<String, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        read_entry(entries, &key)
    };
    let rel = entry_rel(manifest)
        .ok_or_else(|| format!("module at bundle prefix `{prefix}` has no entry"))?;
    let mut src = read(rel)?;
    // the multi-lib splice, the directory-side twin: base first, then
    // `libs` in manifest order, '\n'-joined — ONE source string
    for lib in &manifest.entry.libs {
        src.push('\n');
        src.push_str(&read(lib)?);
    }
    Ok(Pkg {
        body: PkgBody::Source { text: src, is_decl: false },
        entry: manifest.entry.clone(),
        ..Default::default()
    })
}

/// The generic-bearing source a compiled unit rides, read from the
/// archive under `prefix` (empty for the root, `<pkg>/` for a group):
/// the entry lib + `entry.libs` spliced, plus the `[peer-deps]` group
/// files keyed by peer. The entry lib's PRESENCE is the dispatch
/// marker the packer laid down — its absence is a legacy bundle (or a
/// non-generic pkg), which rides nothing and refuses consumer-spelled
/// shapes at link. Once the marker answers, every other riding file
/// must be there (refuse, never guess — a corrupt archive is a load
/// error).
pub fn riding_gen_source(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    manifest: &Manifest,
) -> Result<Option<crate::session::GenSource>, String> {
    let read = |rel: &str| -> Result<String, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        read_entry(entries, &key)
    };
    let Some(base) = &manifest.entry.lib else {
        return Ok(None); // no body — nothing could ride
    };
    let mut text = match read(base) {
        Ok(t) => t,
        Err(_) => return Ok(None), // the marker's absence: a legacy bundle
    };
    for lib in &manifest.entry.libs {
        text.push('\n');
        text.push_str(&read(lib)?);
    }
    let mut peers = Vec::new();
    for (peer, desc) in &manifest.peer_deps {
        if let Some(lib) = desc.get("lib") {
            peers.push((peer.clone(), read(lib)?));
        }
    }
    Ok(Some(crate::session::GenSource { text, peers }))
}

/// Pass 3 — the peer gate, the PURE presence law (the walk's
/// directory-time D3 checks live in rut-native; this half reads
/// nothing):
///
/// - required peer absent → the loud D1 mount error: names the pkg, the
///   peer, and the fix. Never auto-pulled.
/// - peer present (any reason) → the pkg's recorded integration text
///   (its `[peer-deps]` `lib`, read at mount) moves onto
///   [`Pkg::peer_groups`] — presence-based mounting; the mounted body
///   stays pristine.
/// - optional peer absent → inert; the group simply never mounts.
/// - a COMPILED declarer's rows are already in its `.rutc` (the
///   pack-time closure's dev-deps supplied the peers — compile once
///   per owner), so only the presence law runs.
///
/// Pkgs whose groups an earlier pass (or the offering host) already
/// appended are skipped — never double-appended.
pub fn peer_gate(pkgs: &mut BTreeMap<String, Pkg>) -> Result<(), RunError> {
    // collected first, applied after — the map borrows nothing
    let mut appends: Vec<(String, String)> = Vec::new();
    let mut pre_compiled: Vec<String> = Vec::new();
    let declared: Vec<(String, BTreeMap<String, PeerDecl>)> = pkgs
        .iter()
        .filter(|(_, m)| !m.peers.is_empty() && !m.groups_mounted)
        .map(|(s, m)| (s.clone(), m.peers.clone()))
        .collect();
    for (pkg, peers) in declared {
        // a compiled declarer's rows ride its binary — the presence law
        // still runs below (D1), but there is nothing to move, and the
        // declarer is done after this pass
        let compiled = matches!(&pkgs[&pkg].body, PkgBody::Compiled(_));
        for (peer, decl) in peers {
            if !pkgs.contains_key(&peer) {
                if decl.optional {
                    continue; // inert — the group simply never mounts
                }
                // D1: loud at mount, naming pkg + peer + fix
                return Err(RunError::law(format!(
                    "pkg `{pkg}` requires the peer `{peer}`, and `{peer}` is not in this program's closure — peers are not pulled transitively: add `\"{peer}\": {{ \"path\": \"..\" }}` to your `rut.jsonc` `deps`"
                )));
            }
            if compiled {
                continue; // rows ride the binary — nothing to move
            }
            let Some(lib) = &decl.lib else {
                continue; // presence declared, no integration file to mount
            };
            match pkgs[&pkg].peer_libs.get(&peer) {
                Some(Some(text)) => appends.push((pkg.clone(), text.clone())),
                Some(None) => {
                    return Err(RunError::law(format!(
                        "pkg `{pkg}`'s [peer-deps] entry `{peer}` names the group `{lib}` — cannot read it (a packaging bug in {pkg})"
                    )));
                }
                None => {
                    return Err(RunError::law(format!(
                        "pkg `{pkg}` declares `[peer-deps]` but is not mounted"
                    )));
                }
            }
        }
        if compiled {
            pre_compiled.push(pkg);
        }
    }
    for (pkg, text) in appends {
        let m = pkgs.get_mut(&pkg).unwrap();
        m.peer_groups.push(text);
        m.groups_mounted = true;
    }
    for pkg in pre_compiled {
        pkgs.get_mut(&pkg).unwrap().groups_mounted = true;
    }
    Ok(())
}
