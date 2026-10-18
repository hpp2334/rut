//! The packer — the driver's PURE half: compile the walked closure and
//! emit one deterministic `.rutbundle`. The FS half lives in
//! rut-native (`pack_dir*`): the root manifest bytes, the `[deps]`
//! walk, the dev-table mounts (compile once per owner — a dep's unit
//! is the same bytes wherever it is packed), and the
//! file reader. They meet here: the [`PackWorld`] the walk assembles
//! compiles over the crate-internal table (auto core — the §0.14 law;
//! every OTHER package is the program's own closure, `calc` included,
//! an ordinary tree package) and emits from the pkgs and the reader.
//! Same input directory ⇒ byte-identical bundle.
//!
//! TWO root kinds, ONE wire family (`format_version` is 12, always;
//! the manifest's `type` routes): a **lib** root packs **compiled**
//! (the root and every source dep ride as `.rutc` binaries (bodies +
//! surface — the linking truth); host/decl pkgs ride as their
//! declaration file sets), and a **host** root packs a **decl root**
//! (single-package: its `.d.rut` surface rides as source, nothing to
//! compile — that arm lives in rut-native, no compiler needed).
//! Instantiation is owner-anchored (a generic export
//! links, its consumers request), class methods cross on the surface's
//! inherent rows, and the pack-time scope ledger lets a loader rebase
//! every decoded program onto its own numbering.
//!
//! A `[deps]` row may pin a **url**: the fetched `.rutbundle`'s groups
//! ride along ([`PkgSource::Archive`] locations, enumerated closed — no
//! recursion). `.rutc` emission stays ONE path: archive-backed
//! compiled modules were rebased onto the run's numbering by the
//! walk, so they encode from `units` exactly like dir deps; only the
//! group's FILE reads (manifests, `.d.rut`, source sets) dispatch on
//! the source. The manifest rides byte-for-byte (the law) — url+sha256
//! rows carry into the output satisfied by the rode-along groups.
//!
//! Groups emit at `deps/<name>@<fp16>/` — the package's CONTENT
//! IDENTITY, fingerprinted over its own rode source set (the four
//! canonical names among the staged entries, fixed order, absent
//! skipped). Same source ⇒ same identity: a repack of the same group
//! lands at the same dirname, and two identities of one name coexist
//! side by side. The identity order is the archive's group order.
//!
//! Refusals (never guesses): cycles and name collisions — the graph's
//! own laws, surfaced as pack errors; plus the v1 url-dep refusals
//! below. Source sharing stays what it always was outside bundles: a
//! directory (`rut run <dir>`).

mod decl_entry;
mod error;
mod groups;
mod strip;

pub use decl_entry::{
    check_round_trip, facts_of_program, facts_of_text, lower_inline_entry, normalize_decl_tree,
    render_program_entry, rows_agree,
};

pub use error::PackError;
pub use groups::{collect_source_group, has_generic_bodies};
use groups::{group_file, group_manifest, rides_compiled};
use strip::strip_arm;

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::bundle::{bundle_key, write_bundle, Manifest, MANIFEST_NAME};
use crate::graph::compile_units;
use crate::mods::{rows_json, ROWS_NAME};
use crate::run::Walk;
use crate::session::{MountedBody, Session};

/// The bundle wire version — 12, always, ONE number for both root
/// kinds. The manifest's `type` routes the layout (a lib root packs
/// compiled, a `type = "host"` root packs its decl surface); a reader
/// refuses anything else with one re-pack recipe — never guesses.
/// v12: every group carries ONE generated declaration entry — a
/// compiled group's `mod.d.rut` is rendered from the validated surface
/// (the `.d.ts` pattern: generated, never hand-maintained; omitted
/// under `--strip`), a host group's rides as the tree normalized into
/// the one-file inline form (`mod NAME { .. }`, structure in the
/// grammar, never in paths). (v11: dep groups ride at
/// `deps/<name>@<fp16>/…` (content identity), the archive root carries
/// only the root's own files, and the entry names are canonical —
/// unchanged; the reader still loads the 11 lane, so pinned v11
/// artifacts keep working and migration is a re-pin, not a break.)
pub const FORMAT_VERSION: u64 = 12;

/// Assemble a one-member **source** bundle — a rut package whose body
/// is the text — as a [`Bundle`](crate::Bundle) handle. A pack method
/// that ONLY assembles: identity + the text, no container bytes, no
/// CRC, and NEVER a compile (a run compiles the source member when it
/// is offered — [`crate::Offer::Lib`]). The ephemeral-source door:
/// per-load apps, playground snippets, REPL — offering fresh source
/// must not pay the container/pack tax, while `pack_dir` (which
/// compiles a closure for distribution) stays the lane for *stable*
/// source being published. The handle is opaque either way — one
/// offering verb over both member kinds.
pub fn pack_inline_pkg(name: &str, text: impl Into<String>) -> crate::run::Bundle {
    crate::run::Bundle::from_source_member(name, text.into())
}

/// Where a group's files ride from — the emission's read dispatch. The
/// `Dir` arm carries the DIRECTORY KEY (an opaque string; the FS math
/// is the caller reader's), the `Archive` arm the walked archive's
/// slot and the group's in-archive prefix.
#[derive(Clone, Debug)]
pub enum PkgSource {
    Dir(String),
    Archive { slot: usize, prefix: String },
}

/// One fetched archive's decoded entries, kept by the CALLER (the walk
/// stays I/O-free beyond the source; it records only where files
/// live: slot + prefix). `origin` is the url, for error messages.
#[derive(Clone, Debug)]
pub struct Archive {
    pub origin: String,
    pub entries: Vec<(String, Vec<u8>)>,
}

/// The reader the emission's dir reads go through: `(dir key, rel) →
/// bytes`. String keys by law — the driver never does key math and
/// never sees a `Path`.
pub type PackRead = Rc<dyn Fn(&str, &str) -> Result<Vec<u8>, String>>;

/// The walked world the pack compiles over — rut-native's `pack_dir*`
/// assembles it: the walked closure (gate run, dev tables mounted),
/// the root manifest (bytes + parse), the group-source map, the
/// archives the url deps opened, and the file reader.
pub struct PackWorld {
    pub loaded: Walk,
    /// the root's `rut.jsonc`, byte-for-byte (the law)
    pub manifest_bytes: Vec<u8>,
    pub manifest: Manifest,
    /// the root's dir key — the emission's root-file reads go through
    /// the reader with it
    pub root_dir: String,
    pub sources: BTreeMap<String, PkgSource>,
    pub archives: Vec<Archive>,
    pub read: PackRead,
}

/// Pack options. `strip` mangles every renameable name and strips the
/// symbolication tables from the emitted binaries — the restore data
/// rides a PRIVATE symbol-table sidecar beside the bundle (never an
/// entry inside it).
#[derive(Clone, Debug, Default)]
pub struct PackOpts {
    pub strip: bool,
}

/// The pkg's own source text as the rows' `""` spelling: a source
/// body's text, a host body's `.d.rut` root surface (the rows the
/// tree lowered from — the `type = "host"` flavor of the same
/// envelope). A pkg with mounted mod children always answers one of
/// the two; a compiled body's tree rows spell an empty root (the
/// binary IS the body — the `""` row is inert for it).
fn rows_root_text(pkg: &crate::session::Mounted) -> String {
    match &pkg.body {
        MountedBody::Source { text, .. } => text.clone(),
        MountedBody::Host { root_surface, .. } => root_surface.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Pack the walked world: compile the closure (auto core rides; every
/// other pkg is program-owned), run the strip arm under `opts`, emit
/// the entries. Answers the bundle bytes and — under `strip` — the
/// serialized symbol table. Deterministic end to end: same input dir
/// ⇒ byte-identical bundle ⇒ byte-identical symtab.
pub fn pack(w: &PackWorld, opts: &PackOpts) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    let _name = w
        .manifest
        .name
        .clone()
        .ok_or_else(|| PackError::law("the manifest has no `name`".to_string()))?;
    let root = w.loaded.root.clone();
    // the closure's session: the walked pkgs + the prelude (the ONLY
    // engine mount — `calc` and the rest are the program's own closure)
    let mut session = Session::new();
    // the scope ledger: the walk namespaced each archive against its
    // own growing ledger, so the union of the stamped rows IS the
    // walked ledger — record it, then mount (compiled bodies are
    // already rebased)
    let mut ledger_rows: Vec<(rut_core::id::ScopeId, String)> = Vec::new();
    for pkg in &w.loaded.pkgs {
        for (s, spec) in &pkg.bundle_scopes {
            ledger_rows.push((*s, spec.clone()));
        }
    }
    ledger_rows.sort();
    ledger_rows.dedup();
    for (s, spec) in &ledger_rows {
        session.record_bundle_scope(*s, spec);
    }
    for pkg in w.loaded.pkgs.clone() {
        let spec = pkg.spec.clone();
        session.register_module(&spec, pkg).map_err(|e| PackError::law(e.to_string()))?;
    }
    let _ = session.mount(crate::run::core_pkg());
    // the rode-along closure law: every dep an archive group's manifest
    // declares must ride in this output — a group the mount skipped
    // without a source of its own would leave the output declaring a
    // dep without a group (v1: loud, named). Deps keys stay BARE
    // (source-facing grammar); group mounts key on their identity, so
    // the check matches by name half.
    for (spec, source) in &w.sources {
        if let PkgSource::Archive { .. } = source {
            let dm = group_manifest(source, &w.archives, &w.read).map_err(PackError::law)?;
            for dep in dm.deps.keys() {
                let ridden = *dep == root
                    || w.sources.keys().any(|k| crate::bundle::mount_name(k) == dep.as_str());
                if !ridden {
                    return Err(PackError::law(format!(
                        "packed dep `{spec}` declares `{dep}`, but that group did not ride — the output would be missing its `{dep}` group; vendor this dep (unpack the url dep into your project) instead"
                    )));
                }
            }
        }
    }
    let mut units = compile_units(&session, &root);
    if !units.diags.is_empty() || !units.ok {
        let msgs: Vec<String> = units.diags.iter().map(|d| d.msg.clone()).collect();
        return Err(PackError::law(format!("pack: {}", msgs.join("; "))));
    }
    // the pinned stability law (v22): a generic that cannot lower to an
    // instantiation-independent body refuses the pack outright — no
    // fallback lane, a published bundle carries no source
    for (owner, decl, construct) in &units.template_failures {
        return Err(PackError::law(format!(
            "pack: `{owner}`'s generic `{decl}` does not lower to an instantiation-independent \
             body ({construct}) — restructure the surface or share the directory; a published \
             bundle carries no source"
        )));
    }
    // a compiled group the walk never ensured cannot ride: its binary
    // still spells ITS pack's scopes, which this output's ledger does
    // not carry — refuse, never guess (v1: use it or vendor the dep)
    for (spec, source) in &w.sources {
        if let PkgSource::Archive { slot, .. } = source {
            if matches!(
                session.resolve(spec).map(|m| &m.body),
                Ok(MountedBody::Compiled(_))
            ) && !units.linked.contains_key(spec)
            {
                return Err(PackError::law(format!(
                    "compiled group `{spec}` (from {}) is not part of this program's compiled closure — a bundled binary cannot ride unpackaged; use it or vendor this dep",
                    w.archives[*slot].origin
                )));
            }
        }
    }
    // the root's compiled program — produced before the strip arm (the
    // mangle mutates `units` in place); a host root cannot pack (the
    // decl arm lives in rut-native)
    let root_linked = units.linked.get(&root);
    let root_ok = matches!(
        (&session.resolve(&root).unwrap().body, root_linked),
        (MountedBody::Source { .. }, Some(&_))
    );
    if !root_ok {
        return Err(PackError::law(format!(
            "pack: {root} produced no compiled program — a host pkg (a `.d.rut` surface with no body) cannot be a bundle root; pack a source pkg instead"
        )));
    }
    let root_idx = root_linked.unwrap().0;
    // the strip arm — after the compile walk, before any encode. Two
    // laws run here, all against the packer's OWN group-kind decisions
    // (the `sources` map + `units.linked`, the same classifier the
    // encode loop applies — never a re-derivation):
    //
    // 1. the mixed-bundle refusal: a group riding as a SOURCE file set
    //    binds its compiled deps' surfaces at load, and those names
    //    would be mangled — refuse, never guess (host/decl leaf groups
    //    have no compiled deps and pass trivially);
    // 2. the mangle itself: one closure-wide string→string map over
    //    every encoded program, symbolication tables taken into the
    //    sidecar.
    // (The riding-source refusal of the pre-v22 strip arm is dead: no
    // source rides a compiled pkg anymore — the delivery law.)
    let symtab = strip_arm(&mut units, &session, &w.sources, &w.archives, &w.read, &root, root_idx, opts.strip)?;
    // the templates-must-emit law (v22): a compiled program whose
    // surface exports an OPEN generic surface must carry template
    // bodies — the delivery law's producer half. A miss here is a
    // compiler regression, never a packable shape (the consumer's
    // mount seam refuses the same shape loudly).
    for (spec, idx) in std::iter::once((root.clone(), root_idx)).chain(
        units.linked.iter().filter(|(s, _)| s.as_str() != root && session.resolve(s).is_ok()).map(|(s, (i, _))| (s.clone(), *i)),
    ) {
        let prog = &units.programs[idx];
        if !rides_compiled(&session, &spec, &units) {
            continue;
        }
        if has_generic_bodies(prog) && prog.templates.is_empty() {
            return Err(PackError::law(format!(
                "pack: `{spec}` owns an open generic surface but its binary carries no template \
                 bodies — corrupt or doctored, re-pack the directory"
            )));
        }
        // the stability law's pack half: a template body whose
        // repr-carrying ops are not grounded would lower wrongly for
        // some instantiation — the HARD refusal, the recipe spelled
        if let Some((decl, construct)) = crate::compile::material::template_stability_violation(prog) {
            return Err(PackError::law(format!(
                "pack: `{spec}`'s generic `{decl}` does not lower to an instantiation-independent \
                 body ({construct}) — restructure the surface or share the directory; a published \
                 bundle carries no source"
            )));
        }
    }
    // ---- staging: the root's own files, then every dep group — so
    // the scope ledger's identity rows read fingerprints off the exact
    // bytes about to be emitted, and the emission order stays the law:
    // rut.jsonc, rut.scopes, the root's files, then the deps/ groups
    // in identity order.

    // the root's own file set (prefix ""): the GENERATED declaration
    // entry at its canonical `mod.d.rut` name (wire v12 — rendered from
    // the validated surface, the `.d.ts` pattern; round-trip gated
    // below; `--strip` omits it — plaintext would leak the names the
    // binary mangles), the module rows. The manifest rides first (below)
    // and the binary (`mod.rutc`) is NOT part of the fingerprint basis —
    // the identity fingerprints the SOURCE set. (Since v22 a
    // generic-owning pkg rides NO source: its generic bodies are
    // template rows inside the binary — the delivery law.)
    let root_pkg = session.resolve(&root).map_err(|e| PackError::law(e.to_string()))?;
    let mut root_files: Vec<(String, Vec<u8>)> = Vec::new();
    if !opts.strip {
        let entry = decl_entry::render_program_entry(&units.programs[root_idx]);
        decl_entry::check_round_trip(&root, &units.programs[root_idx], &entry)
            .map_err(PackError::law)?;
        root_files.push(("mod.d.rut".into(), entry.into_bytes()));
    }
    if !root_pkg.mods.is_empty() {
        let rows = rows_json(&rows_root_text(root_pkg), &root_pkg.mods).into_bytes();
        root_files.push((ROWS_NAME.into(), rows));
    }
    // the root's identity fingerprint: the manifest plus the staged
    // root files' canonical four, fixed order, absent skipped
    let mut root_fp_src: Vec<(String, Vec<u8>)> =
        vec![(MANIFEST_NAME.to_string(), w.manifest_bytes.clone())];
    root_fp_src.extend(root_files.iter().cloned());
    let root_fp = crate::bundle::identity::fp16_of(&root_fp_src, "");

    // ---- the dep groups, staged per source and keyed by identity
    let mut staged: BTreeMap<String, Vec<(String, Vec<u8>)>> = BTreeMap::new();
    // sources key → the identity it staged under (dir deps and url
    // roots mount under BARE keys; their groups still ride under the
    // identity, and the ledger rows spell it)
    let mut staged_by_spec: BTreeMap<String, String> = BTreeMap::new();
    for (spec, source) in &w.sources {
        if *spec == root || session.resolve(spec).is_err() {
            continue; // the root rode above; unmounted names never ride
        }
        let dm = group_manifest(source, &w.archives, &w.read).map_err(PackError::law)?;
        let gname = crate::bundle::mount_name(spec);
        if dm.name.as_deref() != Some(gname) {
            return Err(PackError::law(format!(
                "dep `{spec}` names itself `{}` in its manifest — expected `{gname}`",
                dm.name.as_deref().unwrap_or("<unnamed>")
            )));
        }
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        if rides_compiled(&session, spec, &units) {
            let &(idx, _) = units.linked.get(spec).unwrap();
            // the group's manifest rides byte-for-byte: the loader reads
            // its name and mount flags from it
            let toml =
                group_file(source, MANIFEST_NAME, &w.archives, &w.read).map_err(PackError::law)?;
            files.push((MANIFEST_NAME.into(), toml));
            files.push(("mod.rutc".into(), rut_core::binary::encode(&units.programs[idx])));
            // the GENERATED declaration entry (wire v12) — rendered from
            // the validated surface, round-trip gated, `--strip` omitted
            // (the strip law). An ARCHIVE-sourced group copies the entry
            // its own archive carries instead: the group's content
            // identity is the archive's law (the mount key's fingerprint
            // pins the rode source set), and a v11 group's file set is
            // exactly what its archive vouches for — the ladder's
            // object-surface floor serves its editor face.
            match source {
                PkgSource::Dir(_) if !opts.strip => {
                    let entry = decl_entry::render_program_entry(&units.programs[idx]);
                    decl_entry::check_round_trip(spec, &units.programs[idx], &entry)
                        .map_err(PackError::law)?;
                    files.push(("mod.d.rut".into(), entry.into_bytes()));
                }
                PkgSource::Archive { slot, prefix } => {
                    let key = bundle_key(&format!("{prefix}mod.d.rut"))
                        .map_err(PackError::law)?;
                    if let Some((_, bytes)) = w.archives[*slot]
                        .entries
                        .iter()
                        .find(|(n, _)| *n == key)
                    {
                        files.push(("mod.d.rut".into(), bytes.clone()));
                    }
                }
                _ => {}
            }
            // the module rows, group flavor (see the root arm above)
            if let Ok(m) = session.resolve(spec) {
                if !m.mods.is_empty() {
                    files.push((
                        ROWS_NAME.into(),
                        rows_json(&rows_root_text(m), &m.mods).into_bytes(),
                    ));
                }
            }
        } else {
            match source {
                PkgSource::Dir(gdir) => {
                    let mods = session.resolve(spec).map(|m| m.mods.clone()).unwrap_or_default();
                    let is_host = matches!(
                        session.resolve(spec).map(|m| &m.body),
                        Ok(MountedBody::Host { .. })
                    );
                    if is_host {
                        // the host group's ONE consolidated declaration
                        // entry (wire v12): the tree normalized into the
                        // single-file inline form — and the round-trip
                        // gate: the entry must lower to the same rows the
                        // file-tree form did (the loader law, checked at
                        // pack)
                        let m = session.resolve(spec).map_err(|e| PackError::law(e.to_string()))?;
                        let (root_text, host_rows) = match &m.body {
                            MountedBody::Host { root_surface, host_funcs, .. } => (
                                root_surface.clone().unwrap_or_default(),
                                host_funcs.clone(),
                            ),
                            _ => (String::new(), Vec::new()),
                        };
                        let entry =
                            decl_entry::normalize_decl_tree(&root_text, &mods)
                                .map_err(PackError::law)?;
                        let entry_rows =
                            decl_entry::lower_inline_entry(&entry, &format!("{gdir}/mod.d.rut"))
                                .map_err(PackError::law)?;
                        if !decl_entry::rows_agree(&host_rows, &entry_rows) {
                            return Err(PackError::law(format!(
                                "dep `{spec}`: the consolidated declaration entry does not lower to \
                                 the tree's own rows — the generation invariant refuses the pack"
                            )));
                        }
                        let toml = group_file(source, MANIFEST_NAME, &w.archives, &w.read)
                            .map_err(PackError::law)?;
                        files.push((MANIFEST_NAME.into(), toml));
                        files.push(("mod.d.rut".into(), entry.into_bytes()));
                    } else {
                        collect_source_group(gdir, &dm, &mods, "", &w.read, &mut files)
                            .map_err(PackError::law)?;
                        // the module rows, group flavor (see the root arm
                        // above): the tree rides beside the file set
                        if !mods.is_empty() {
                            let text = session
                                .resolve(spec)
                                .map(rows_root_text)
                                .unwrap_or_default();
                            files.push((
                                ROWS_NAME.into(),
                                rows_json(&text, &mods).into_bytes(),
                            ));
                        }
                    }
                }
                PkgSource::Archive { slot, prefix: old } => {
                    // copy the archive group's file set under the new
                    // prefix — the archive already decided the set
                    // (manifest, binary, surface, rows), in
                    // archive order (deterministic)
                    let archive = &w.archives[*slot];
                    for (key, bytes) in &archive.entries {
                        let Some(rest) = key.strip_prefix(old.as_str()) else {
                            continue;
                        };
                        if rest.is_empty() {
                            continue;
                        }
                        files.push((rest.to_string(), bytes.clone()));
                    }
                }
            }
        }
        // the content identity: the fingerprint over the staged file
        // set's four canonical names, fixed order
        let fp = crate::bundle::identity::fp16_of(&files, "");
        let ident = crate::bundle::identity::identity(gname, &fp);
        // a source the walk mounted under its identity must stage to
        // exactly that identity — a mismatch is a law bug, refused
        if let Some((_, key_fp)) = crate::bundle::parse_identity(spec) {
            if key_fp != fp {
                return Err(PackError::law(format!(
                    "dep `{spec}`'s mounted identity disagrees with its staged file set \
                     (`{key_fp}` vs `{fp}`) — refuse, never guess"
                )));
            }
        }
        if let Some(prev) = staged.get(&ident) {
            if prev != &files {
                return Err(PackError::law(format!(
                    "two dep groups stage to the same identity `{ident}` with different file sets — \
                     refuse, never guess"
                )));
            }
            staged_by_spec.insert(spec.clone(), ident);
            continue; // the same identity rode twice — one group, exact dedup
        }
        staged.insert(ident.clone(), files);
        staged_by_spec.insert(spec.clone(), ident);
    }

    // ---- the scope ledger: one `<scope> = "<identity>"` row per
    // linked module of the packed closure (engine mounts included),
    // ascending by scope. Group rows spell their staged identity; the
    // root's row spells the root's own (the self-row — a loader maps
    // it to the bare-named root mount); a pkg with no rode file set
    // (an engine mount, a dev-table pkg) fingerprints the empty set.
    let mut ledger: Vec<(rut_core::id::ScopeId, String)> = units
        .linked
        .iter()
        .map(|(spec, &(_, scope))| {
            let row = if let Some((n, f)) = crate::bundle::parse_identity(spec) {
                let ident = crate::bundle::identity::identity(n, f);
                if !staged.contains_key(&ident) {
                    return Err(PackError::law(format!(
                        "the ledger names `{ident}`, but no group staged under that identity — \
                         the closure and the file set disagree"
                    )));
                }
                ident
            } else if let Some(ident) = staged_by_spec.get(spec) {
                // a group mounted under a bare key (a dir dep, a url
                // root riding along): its row spells the identity it
                // stages under — the load-side resolution binds the
                // identity-mounted group
                ident.clone()
            } else if spec == &root {
                crate::bundle::identity::identity(&root, &root_fp)
            } else {
                // no rode file set (an engine mount, a dev-table pkg):
                // the identity of the empty set; a loader binds the
                // bare name
                crate::bundle::identity::identity(
                    spec,
                    &crate::bundle::identity::fp16_of(&[], ""),
                )
            };
            Ok((scope, row))
        })
        .collect::<Result<Vec<_>, PackError>>()?;
    ledger.sort();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    // the manifest byte-for-byte, then the scope ledger
    entries.push((MANIFEST_NAME.into(), w.manifest_bytes.clone()));
    entries.push(("rut.scopes".into(), ledger_text(&ledger).into_bytes()));
    // the root's compiled binary, then its staged file set (the
    // surface, the module rows)
    entries.push(("mod.rutc".into(), rut_core::binary::encode(&units.programs[root_idx])));
    entries.extend(root_files);
    // the dep groups, identity order — the archive mirrors the
    // manifest's deps table
    for (ident, files) in &staged {
        for (rel, bytes) in files {
            entries.push((
                bundle_key(&format!("deps/{ident}/{rel}")).map_err(PackError::law)?,
                bytes.clone(),
            ));
        }
    }
    let bundle = write_bundle(&entries).map_err(|e| PackError::law(e.to_string()))?;
    Ok((bundle, symtab))
}

/// The `rut.scopes` ledger: one `<scope> = "<identity>"` row per linked
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
