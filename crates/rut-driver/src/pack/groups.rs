//! Where a group's files live: the dir-vs-archive reads the encode
//! loop and the strip arm share. Dir reads go through the world's
//! string-keyed reader (the FS math is rut-native's); archive reads
//! index the walked archives' entries.


use crate::bundle::{bundle_key, parse_manifest, parse_manifest_compat, read_entry};
use crate::pack::Archive;
use crate::pack::{PackRead, PkgSource};
use crate::session::PkgBody;

use std::collections::BTreeMap;

/// Emit a generic-owning compiled pkg's riding source under `prefix`
/// (empty for the root, `<pkg>/` for a group): the root module's
/// source (`mod.rut` — the post-repeal envelope), then each
/// `[peer-deps]` descriptor's `lib` group file in peer-name order (the
/// manifest's BTreeMap order — deterministic). Verbatim bytes, one
/// entry per path. An OLD published archive being re-packed (its
/// manifest rides the compat lane) rides at the legacy `entry.lib` +
/// `libs` spelling instead — the reader-compat law's writer twin.
pub(super) fn ride_generic_source(
    manifest: &crate::bundle::Manifest,
    prefix: &str,
    entries: &mut Vec<(String, Vec<u8>)>,
    read: &impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let mut push = |rel: &str, entries: &mut Vec<(String, Vec<u8>)>| -> Result<bool, String> {
        let rel = rel.strip_prefix("./").unwrap_or(rel);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        if entries.iter().any(|(n, _)| *n == key) {
            return Ok(false); // already riding (a legacy libs overlap) — one entry, one copy
        }
        let text = read(rel)?;
        entries.push((key, text));
        Ok(true)
    };
    // the body: `mod.rut` (the root module), or the old envelope's
    // legacy keys. Nothing rides when neither names a body — no body,
    // nothing to recompile from.
    if let Some(base) = &manifest.legacy_entry.lib {
        push(base, entries)?;
        for lib in &manifest.legacy_entry.libs {
            push(lib, entries)?;
        }
    } else {
        push("mod.rut", entries)?;
    }
    for desc in manifest.peer_deps.values() {
        if let Some(lib) = desc.get("lib") {
            push(lib, entries)?;
        }
    }
    Ok(())
}

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
/// byte-for-byte, its root module (`mod.rut`) when the pkg is flat, and
/// each `[peer-deps]` descriptor's `lib` group file. A pkg with mounted
/// mod children rides its module rows too (`rut.mods` — the additive
/// envelope section; its root source is the `""` row, so the tree needs
/// no standalone file). An OLD published archive being re-packed rides
/// at the legacy `entry.lib` + `libs` spelling instead (the
/// reader-compat law's writer twin). Descriptor order is the
/// manifest's (BTreeMap), so the archive stays deterministic. A
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
        // group: the surface IS the whole pkg), which rides the
        // `entry.type` surface
        if let Ok(body) = read(dir_key, "mod.rut") {
            let key = bundle_key(&format!("{prefix}mod.rut"))?;
            out.push((key, body));
        } else if let Some(rel) = &manifest.entry.type_path {
            let rel = rel.strip_prefix("./").unwrap_or(rel);
            let key = bundle_key(&format!("{prefix}{rel}"))?;
            let body = read(dir_key, rel)?;
            out.push((key, body));
        } else {
            return Err(format!(
                "module at {dir_key} has no entry — the root module is `mod.rut` beside the manifest"
            ));
        }
    }
    for desc in manifest.peer_deps.values() {
        let Some(lib) = desc.get("lib") else {
            continue; // presence declared, no integration file to pack
        };
        let rel = lib.strip_prefix("./").unwrap_or(lib);
        let key = bundle_key(&format!("{prefix}{rel}"))?;
        let body = read(dir_key, rel)?;
        out.push((key, body));
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
        Ok(PkgBody::Source { .. }) => units.linked.contains_key(spec),
        Ok(PkgBody::Compiled(_)) => true,
        _ => false,
    }
}

/// Does a compiled program's surface export an OPEN generic surface —
/// the generic-source riding trigger: exported generic fns
/// (`launch_future<T>`), generic type exports (`Vec<T>`), or generic
/// methods on an inherent row (`MutCtx::set<A, R>`). (The old
/// generic-target impl arm died with the impl registrations — a
/// generic class's own methods trigger through the inherents arm.) A
/// false positive costs a few inert archive entries; a false negative
/// would refuse a shape the ridden source could have served.
pub fn has_open_generic_surface(prog: &rut_core::binary::Program) -> bool {
    let s = &prog.surface;
    !s.fn_generics.is_empty()
        || s.type_exports.iter().any(|t| t.is_generic)
        || s.inherents
            .iter()
            .any(|ih| ih.methods.iter().any(|m| !m.generics.is_empty()))
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

