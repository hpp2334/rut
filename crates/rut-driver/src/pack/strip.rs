//! The strip arm — extracted from the pack pipeline's tail: three
//! laws run here, all against the packer's OWN group-kind decisions
//! (the `sources` map + `units.linked`, the same classifier the encode
//! loop applies — never a re-derivation), then the mangle itself.

use std::collections::BTreeMap;

use super::groups::{group_manifest, has_open_generic_surface, rides_compiled};
use crate::pack::Archive;
use crate::pack::{PackRead, PkgSource};
use crate::PackError;

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
pub(super) fn strip_arm(
    units: &mut crate::graph::Units,
    session: &crate::session::Session,
    sources: &BTreeMap<String, PkgSource>,
    archives: &[Archive],
    read: &PackRead,
    root: &str,
    root_idx: usize,
    strip: bool,
) -> Result<Option<Vec<u8>>, PackError> {
    if !strip {
        return Ok(None);
    }
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
            for (spec, source) in sources.iter() {
                if *spec == root || session.resolve(spec).is_err() {
                    continue;
                }
                if !rides_compiled(session, spec, &units) {
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
                            PkgSource::Dir(d) => d.clone(),
                            PkgSource::Archive { slot, .. } => archives[*slot].origin.clone(),
                        }
                    )));
                }
            }
        }
        for (spec, source) in sources.iter() {
            if *spec == root || session.resolve(spec).is_err() {
                continue; // the root rides compiled above; unmounted names never ride
            }
            if rides_compiled(session, spec, &units) {
                continue; // a `.rutc` group — no source binds anything
            }
            let dm = group_manifest(source, archives, read).map_err(PackError::law)?;
            for dep in dm.deps.keys() {
                let dep_compiled = rides_compiled(session, dep, &units);
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
        let mut wanted: Vec<(String, usize)> = vec![(root.to_string(), root_idx)];
        for (spec, _) in sources.iter() {
            if *spec == root || session.resolve(spec).is_err() {
                continue;
            }
            if !rides_compiled(session, spec, &units) {
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
    Ok(Some(map.to_bytes()))

}
