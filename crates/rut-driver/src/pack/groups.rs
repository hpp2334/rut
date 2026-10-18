//! Where a group's files live: the dir-vs-archive reads the encode
//! loop and the strip arm share. Dir reads go through the world's
//! string-keyed reader (the FS math is rut-native's); archive reads
//! index the walked archives' entries.


use crate::bundle::{bundle_key, parse_manifest, parse_manifest_compat, read_entry};
use crate::pack::Archive;
use crate::pack::{PackRead, PkgSource};
use crate::session::MountedBody;

use std::collections::BTreeMap;

/// A group's parsed manifest, read from wherever its files live. A
/// DIRECTORY group parses strict (the post-repeal grammar — the body
/// keys refuse); an ARCHIVE group parses compat (a published bundle's
/// manifest is forever old — the reader-compat law).
pub(super) fn group_manifest(
    source: &PkgSource,
    archives: &[Archive],
    read: &PackRead,
) -> Result<crate::bundle::Manifest, String> {
    match source {
        PkgSource::Dir(gdir) => {
            let bytes = read(gdir, crate::bundle::files::MANIFEST_NAME)?;
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("{}: not UTF-8", crate::bundle::files::MANIFEST_NAME))?;
            parse_manifest(&text).map_err(|e| e.to_string())
        }
        PkgSource::Archive { slot, prefix } => {
            let archive = &archives[*slot];
            let name = crate::bundle::files::MANIFEST_NAME;
            let text = read_entry(&archive.entries, &format!("{prefix}{name}"))
                .map_err(|e| format!("{}: {e}", archive.origin))?;
            parse_manifest_compat(&text)
                .map_err(|e| format!("{}: {prefix}{name}: {e}", archive.origin))
        }
    }
}

/// One group file's bytes, read from wherever its files live (a
/// `.rutc`-riding group's manifest / surface text).
pub(super) fn group_file(
    source: &PkgSource,
    rel: &str,
    archives: &[Archive],
    read: &PackRead,
) -> Result<Vec<u8>, String> {
    match source {
        PkgSource::Dir(gdir) => read(gdir, rel),
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

/// Collect a package's SOURCE file set — the group shape — under
/// `prefix` (empty for a root, `<pkg>/` for a dep group): its `rut.jsonc`
/// byte-for-byte and its root module (`mod.rut`) when the pkg is flat.
/// A pkg with mounted
/// mod children rides its module rows too (`rut.mods` — the additive
/// envelope section; its root source is the `""` row, so the tree needs
/// no standalone file). An OLD published archive being re-packed rides
/// at the legacy `entry.lib` + `libs` spelling instead (the
/// reader-compat law's writer twin). A
/// compiled group does not take this shape (its `.rutc` is the linking
/// truth); host pkgs and declared-but-unused pkgs ride the bundle
/// exactly like this. Dir reads go through the world's reader; same
/// input ⇒ same bytes.
pub fn collect_source_group(
    dir_key: &str,
    manifest: &crate::bundle::Manifest,
    mods: &BTreeMap<String, crate::mods::ModSource>,
    prefix: &str,
    read: &PackRead,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    let name = crate::bundle::files::MANIFEST_NAME;
    let text = read(dir_key, name)?;
    out.push((format!("{prefix}{name}"), text));
    // the body: the old envelope's legacy keys, else the root module —
    // the rows entry carries the tree when children mount (the root
    // text is the `""` row), so the standalone file is a FLAT pkg's
    // shape. A bodyless pkg (a host group — the surface is the whole
    // pkg) rides its `entry.type` surface instead, as always.
    if let Some(base) = &manifest.legacy_entry.lib {
        let rel = base.strip_prefix("./").unwrap_or(base);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        let body = read(dir_key, rel)?;
        out.push((key, body));
        // each legacy `libs` file rides beside the base, in manifest
        // order — the array IS the order the reader splices back, so
        // the archive stays deterministic
        for lib in &manifest.legacy_entry.libs {
            let rel = lib.strip_prefix("./").unwrap_or(lib);
            let key = bundle_key(&format!("{prefix}{rel}"))?;
            let body = read(dir_key, rel)?;
            out.push((key, body));
        }
    } else if !mods.is_empty() {
        // a mod-rooted pkg: the tree rides the rows entry the caller
        // writes beside this file set
    } else {
        // a flat pkg's root module — unless there is none (a host
        // group: the surface IS the whole pkg), which rides its
        // `entry.type` surface at the canonical `mod.d.rut` name
        // (wire v11: the archive is name-free; the directory keeps
        // whatever spelling its grammar allows)
        if let Ok(body) = read(dir_key, "mod.rut") {
            let key = bundle_key(&format!("{prefix}mod.rut"))?;
            out.push((key, body));
        } else if let Some(rel) = &manifest.entry.type_path {
            let rel = rel.strip_prefix("./").unwrap_or(rel);
            let body = read(dir_key, rel)?;
            let key = bundle_key(&format!("{prefix}mod.d.rut"))?;
            out.push((key, body));
        } else {
            return Err(format!(
                "module at {dir_key} has no entry — the root module is `mod.rut` beside the manifest"
            ));
        }
    }
    Ok(())
}

/// The `.rutc`-or-source classifier — the packer's ONE group-kind
/// decision, shared by the strip arm and the encode loop. A source
/// body rides compiled iff the graph linked it; a COMPILED body (an
/// archive-backed module, rebased onto this session's numbering by the
/// mount→compile pipeline) always encodes from `units`; a host body
/// has nothing to compile.
pub(super) fn rides_compiled(
    session: &crate::session::Session,
    spec: &str,
    units: &crate::graph::Units,
) -> bool {
    match session.resolve(spec).map(|m| &m.body) {
        Ok(MountedBody::Source { .. }) => units.linked.contains_key(spec),
        Ok(MountedBody::Compiled(_)) => true,
        _ => false,
    }
}

/// Does the program export generic BODIES — free generic fns or
/// generic-method rows (a generic class's members)? The
/// templates-must-emit trigger (v22 — the mount seam's anti-doctoring
/// check and the pack's gate both read it): these are the decls whose
/// bodies must ride the binary as template rows. A generic TYPE export
/// alone (a method-less `Pair<A, B>`) carries no bodies — its
/// instantiations are identity rows the link lane mints structurally,
/// no template needed — so it opens a generic surface without entering
/// this predicate. (A plain class's surface row can carry `local = 0`
/// for a `[constructor]` whose body the caller mints — that is not a
/// generic-method marker; the CLASS's genericity is.)
///
/// (v22 role note: the old `has_open_generic_surface` riding trigger
/// retired with the riding law — the binary-only delivery needs the
/// BODY-level predicate, and a generic type export alone passes the
/// seam trivially.)
pub fn has_generic_bodies(prog: &rut_core::binary::Program) -> bool {
    let s = &prog.surface;
    !s.fn_generics.is_empty()
        || s.inherents.iter().any(|ih| {
            let cname = prog.types.type_at(ih.target).name;
            let class_generic = s
                .type_exports
                .iter()
                .find(|t| t.name == cname)
                .map_or(false, |t| t.is_generic);
            class_generic || ih.methods.iter().any(|m| !m.generics.is_empty())
        })
}

/// The generic-parameter placeholder spelling (`#<param>` — the `#` is
/// unspellable in rut source), distinguished from the async lane's
/// engine-reserved `#frame@`/`#hframe@`/`#ckpt@` rows: a real
/// placeholder is a bare parameter name, never one of the reserved
/// prefixes.
pub(super) fn is_placeholder(name: &str) -> bool {
    use rut_core::async_frame::{CKPT_PREFIX, FRAME_PREFIX, HOST_FRAME_PREFIX};
    name.starts_with('#')
        && !name.starts_with(FRAME_PREFIX)
        && !name.starts_with(HOST_FRAME_PREFIX)
        && !name.starts_with(CKPT_PREFIX)
}

