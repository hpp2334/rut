//! Where a group's files live: the dir-vs-archive reads the encode
//! loop and the dev-mount pass share, plus the group-location walk.

use std::collections::BTreeMap;
use std::path::Path;

use crate::bundle::{bundle_key, parse_manifest, read_entry, FsSource};
use crate::session::ModuleBody;
use crate::loader::{Archive, PkgSource};

/// Emit a generic-owning compiled pkg's riding source under `prefix`
/// (empty for the root, `<pkg>/` for a group): the entry lib, each
/// `entry.libs` file in manifest order, then each `[peer-deps]`
/// descriptor's `lib` group file in peer-name order (the manifest's
/// BTreeMap order — deterministic). Verbatim bytes, one entry per
/// manifest-named path — the same file set a source group rides, minus
/// the manifest (this pkg's manifest already rode).
pub(super) fn ride_generic_source(
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
pub(super) fn group_manifest(source: &PkgSource, archives: &[Archive]) -> Result<crate::bundle::Manifest, String> {
    match source {
        PkgSource::Dir(gdir) => crate::bundle::read_manifest(gdir, &FsSource),
        PkgSource::Archive { slot, prefix } => {
            let archive = &archives[*slot];
            let name = crate::bundle::files::MANIFEST_NAME;
            let text = read_entry(&archive.entries, &format!("{prefix}{name}"))
                .map_err(|e| format!("{}: {e}", archive.origin))?;
            parse_manifest(&text).map_err(|e| format!("{}: {prefix}{name}: {e}", archive.origin))
        }
    }
}

/// One group file's bytes, read from wherever its files live (a
/// `.rutc`-riding group's manifest / surface text).
pub(super) fn group_file(
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
pub(super) fn has_open_generic_surface(prog: &rut_core::binary::Program) -> bool {
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
pub(super) fn is_placeholder(name: &str) -> bool {
    use rut_core::async_frame::{CKPT_PREFIX, FRAME_PREFIX, HOST_FRAME_PREFIX};
    name.starts_with('#')
        && !name.starts_with(FRAME_PREFIX)
        && !name.starts_with(HOST_FRAME_PREFIX)
        && !name.starts_with(CKPT_PREFIX)
}

