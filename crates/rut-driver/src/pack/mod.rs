//! The packer — the driver's PURE half: compile the walked closure and
//! emit one deterministic `.rutbundle`. The FS half lives in
//! rut-native (`pack_dir*`): the root manifest bytes, the `[deps]`
//! walk, the dev-table mounts (compile once per owner — a dep's unit
//! is the same bytes wherever it is packed), the peer gate, and the
//! file reader. They meet here: the [`PackWorld`] the walk assembles
//! compiles over the crate-internal table (auto core — the §0.14 law;
//! every OTHER package is the program's own closure, `calc` included,
//! an ordinary tree package) and emits from the pkgs and the reader.
//! Same input directory ⇒ byte-identical bundle.
//!
//! TWO root kinds, ONE wire number (`format_version` is 10, always;
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
//! compiled modules were rebased onto this session's numbering by the
//! walk, so they encode from `units` exactly like dir deps; only the
//! group's FILE reads (manifests, `.d.rut`, source sets) dispatch on
//! the source. The manifest rides byte-for-byte (the law) — url+sha256
//! rows carry into the output satisfied by the rode-along groups.
//!
//! Refusals (never guesses): cycles and name collisions — the graph's
//! own laws, surfaced as pack errors; plus the v1 url-dep refusals
//! below. Source sharing stays what it always was outside bundles: a
//! directory (`rut run <dir>`).

mod error;
mod groups;
mod strip;

pub use error::PackError;
pub use groups::{collect_source_group, has_open_generic_surface};
use groups::{group_file, group_manifest, ride_generic_source, rides_compiled};
use strip::strip_arm;

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::bundle::{bundle_key, write_bundle, Manifest, MANIFEST_NAME};
use crate::graph::compile_units;
use crate::run::Loaded;
use crate::session::{PkgBody, Session};

/// The bundle wire version — 10, always, ONE number for both root
/// kinds. The manifest's `type` routes the layout (a lib root packs
/// compiled, a `type = "host"` root packs its decl surface); a reader
/// refuses anything else with one re-pack recipe — never guesses.
pub const FORMAT_VERSION: u64 = 10;

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
    pub loaded: Loaded,
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

/// Pack the walked world: compile the closure (auto core rides; every
/// other pkg is program-owned), run the strip arm under `opts`, emit
/// the entries. Answers the bundle bytes and — under `strip` — the
/// serialized symbol table. Deterministic end to end: same input dir
/// ⇒ byte-identical bundle ⇒ byte-identical symtab.
pub fn pack(w: &PackWorld, opts: &PackOpts) -> Result<(Vec<u8>, Option<Vec<u8>>), PackError> {
    let name = w
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
    // dep without a group (v1: loud, named)
    for (spec, source) in &w.sources {
        if let PkgSource::Archive { .. } = source {
            let dm = group_manifest(source, &w.archives, &w.read).map_err(PackError::law)?;
            for dep in dm.deps.keys() {
                if !w.sources.contains_key(dep) && *dep != root {
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
    // a compiled group the walk never ensured cannot ride: its binary
    // still spells ITS pack's scopes, which this output's ledger does
    // not carry — refuse, never guess (v1: use it or vendor the dep)
    for (spec, source) in &w.sources {
        if let PkgSource::Archive { slot, .. } = source {
            if matches!(
                session.resolve(spec).map(|m| &m.body),
                Ok(PkgBody::Compiled(_))
            ) && !units.linked.contains_key(spec)
            {
                return Err(PackError::law(format!(
                    "compiled group `{spec}` (from {}) is not part of this program's compiled closure — a bundled binary cannot ride unpackaged; use it or vendor this dep",
                    w.archives[*slot].origin
                )));
            }
        }
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    // the manifest byte-for-byte, then the scope ledger
    entries.push((MANIFEST_NAME.into(), w.manifest_bytes.clone()));
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
        (PkgBody::Source { .. }, Some(&_))
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
    let symtab = strip_arm(&mut units, &session, &w.sources, &w.archives, &w.read, &root, root_idx, opts.strip)?;
    entries.push((format!("{name}.rutc"), rut_core::binary::encode(&units.programs[root_idx])));
    if let Some(rel) = &w.manifest.entry.type_path {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let text = (w.read)(&w.root_dir, rel).map_err(PackError::law)?;
        entries.push((bundle_key(&format!("{name}.d.rut")).map_err(PackError::law)?, text));
    }
    // the generic-source riding law (v7, additive): a compiled pkg whose
    // surface exports generics ALSO rides the source that serves
    // consumer-spelled shapes — the entry lib + `entry.libs` + the
    // `[peer-deps]` group files, verbatim, beside the binary. The entry
    // lib's presence is the loader's dispatch marker; non-generic pkgs
    // stay source-free. (`--strip` refuses the combination above.)
    if has_open_generic_surface(&units.programs[root_idx]) {
        ride_generic_source(&w.manifest, "", &mut entries, &|rel| (w.read)(&w.root_dir, rel))
            .map_err(PackError::law)?;
    }
    // the dep groups, name order: source pkgs → compiled, host/decl
    // pkgs and declared-but-unused pkgs → the declaration file set
    for (spec, source) in &w.sources {
        if *spec == root || session.resolve(spec).is_err() {
            continue; // the root rode above; names are already mounted
        }
        let dm = group_manifest(source, &w.archives, &w.read).map_err(PackError::law)?;
        let prefix = format!("{spec}/");
        if rides_compiled(&session, spec, &units) {
            let &(idx, _) = units.linked.get(spec).unwrap();
            // the group's manifest rides byte-for-byte: the loader reads
            // its name, mount flags, and peer declarations from it
            let toml =
                group_file(source, MANIFEST_NAME, &w.archives, &w.read).map_err(PackError::law)?;
            entries.push((
                bundle_key(&format!("{prefix}{MANIFEST_NAME}")).map_err(PackError::law)?,
                toml,
            ));
            entries.push((bundle_key(&format!("{prefix}{spec}.rutc")).map_err(PackError::law)?, rut_core::binary::encode(&units.programs[idx])));
            if let Some(rel) = &dm.entry.type_path {
                let rel = rel.strip_prefix("./").unwrap_or(rel);
                let text = group_file(source, rel, &w.archives, &w.read).map_err(PackError::law)?;
                entries.push((bundle_key(&format!("{prefix}{rel}")).map_err(PackError::law)?, text));
            }
            // the generic-source riding law, group flavor (see the root
            // arm above): a generic-owning compiled group rides its
            // source under the group prefix
            if has_open_generic_surface(&units.programs[idx]) {
                ride_generic_source(&dm, &prefix, &mut entries, &|rel| {
                    group_file(source, rel, &w.archives, &w.read)
                })
                .map_err(PackError::law)?;
            }
        } else {
            match source {
                PkgSource::Dir(gdir) => {
                    collect_source_group(gdir, &dm, &prefix, &w.read, &mut entries)
                        .map_err(PackError::law)?;
                }
                PkgSource::Archive { slot, prefix: old } => {
                    // copy the archive group's file set under the new
                    // prefix — the archive already decided the set
                    // (manifest, entry, libs, peer-group files), in
                    // archive order (deterministic)
                    let archive = &w.archives[*slot];
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
