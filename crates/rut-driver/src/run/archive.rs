//! The archive handle — decode + verify once, offer many.
//!
//! [`Bundle`] is the run-side door to a `.rutbundle`: [`Bundle::parse`]
//! is the load stage's decode half (container CRC, the
//! manifest/`format_version` gate, every group's binary decode +
//! verification), done ONCE per process; the handle is reusable across
//! runs, and every [`RutRun::pkg`](super::RutRun::pkg) offer only
//! relocates + mounts per world ([`Bundle::walk`]). Strip restore is a
//! property of the decoded artifact, not of a run:
//! [`Bundle::restore_symbols`] applies once, before any offer, and
//! every later offer carries the restored names.
//!
//! The handle is opaque (L6): no variant-shaped constructors. Ephemeral
//! source rides a source bundle — `crate::pack::pack_inline_pkg`
//! assembles one (identity + text, never compiles); an archive arrives
//! through [`Bundle::parse`]. A source member compiles when a run
//! offers it; a compiled archive mounts as decoded. The offering verb
//! never changes.

use super::{Mounted, RunError, Walk};
use crate::bundle::{GroupKind, Layout};

/// A decoded `.rutbundle` — or an assembled one-member source bundle —
/// as a reusable handle. Parse (or assemble) once, offer to any number
/// of runs.
pub struct Bundle {
    inner: Inner,
}

enum Inner {
    /// a decoded archive: the entry table (CRC-verified at parse) and
    /// the parsed layout (the manifest/version gate and every group's
    /// binary decode + verification, also at parse)
    Archive {
        entries: Vec<(String, Vec<u8>)>,
        layout: Layout,
    },
    /// an assembled one-member source bundle
    /// (`crate::pack::pack_inline_pkg`): identity + the text. No
    /// container bytes, no CRC, never compiled here — a run compiles
    /// the member when it is offered.
    Source { name: String, text: String },
}

impl Bundle {
    /// The source-member constructor — crate-internal by law (L6: the
    /// handle stays opaque; the public door is
    /// [`crate::pack::pack_inline_pkg`], the pack verb).
    pub(crate) fn from_source_member(name: &str, text: String) -> Bundle {
        Bundle { inner: Inner::Source { name: name.to_string(), text } }
    }

    /// Parse + verify a `.rutbundle` once: the container's entry CRCs,
    /// the manifest/`format_version` gate, and every group's binary
    /// decode + verification. The one decode per process — the handle
    /// is reusable across runs, and each offer only relocates + mounts.
    pub fn parse(bytes: &[u8]) -> Result<Bundle, RunError> {
        let origin = "bundle";
        let container =
            crate::bundle::Bundle::parse(bytes).map_err(|e| RunError::law(format!("{origin}: {e}")))?;
        let layout =
            Layout::parse(&container).map_err(|e| RunError::law(format!("{origin}: {e}")))?;
        Ok(Bundle {
            inner: Inner::Archive { entries: container.entries().to_vec(), layout },
        })
    }

    /// Restore a symbol table's names and positions into the decoded
    /// programs — the load half of compile-time symbol stripping,
    /// applied ONCE, before any offer; every subsequent offer carries
    /// the restored names. Names restore closure-wide (the mangle was
    /// one map over every compiled group); the span/pos sections match
    /// by mount spec — the root answers its manifest name, a compiled
    /// group its identity key — and a section whose pkg the archive
    /// does not carry (or a map from a different build, a fn-count
    /// mismatch) matches nothing, tolerated, not an error. A source
    /// member carries nothing compiled — a no-op.
    pub fn restore_symbols(&mut self, map: &rut_core::strip::SymbolMap) {
        let Inner::Archive { layout, .. } = &mut self.inner else {
            return;
        };
        let Layout::Compiled { manifest, root, groups, .. } = layout else {
            return; // a decl root carries no programs
        };
        // names first: linking and the VM both see the real thing
        let restore: std::collections::HashMap<&str, &str> = map
            .names
            .iter()
            .map(|(m, o)| (m.as_str(), o.as_str()))
            .collect();
        let remap = |prog: &mut rut_core::binary::Program| {
            prog.interner.remap_tail(|s| restore.get(s).copied().unwrap_or(s).to_string());
        };
        remap(root);
        for (_, group) in groups.iter_mut() {
            if let GroupKind::Compiled(prog) = group {
                remap(prog);
            }
        }
        // then the span/pos sections, spec-resolved like the names: the
        // root's mount spec is its manifest name, a compiled group's is
        // its identity key. A fn-count mismatch means the section was
        // taken from a different build — matches nothing, tolerated
        // silently (the same law as the name rows).
        for section in &map.sections {
            let target: Option<&mut rut_core::binary::Program> =
                if manifest.name.as_deref() == Some(section.spec.as_str()) {
                    Some(root)
                } else {
                    groups.iter_mut().find_map(|(prefix, group)| {
                        let key = prefix.strip_prefix(&format!("{}/", crate::bundle::DEPS_DIR))?;
                        if key != section.spec {
                            return None;
                        }
                        match group {
                            GroupKind::Compiled(prog) => Some(prog),
                            _ => None,
                        }
                    })
                };
            if let Some(prog) = target {
                if prog.funcs.len() == section.fns.len() {
                    for (f, sy) in prog.funcs.iter_mut().zip(&section.fns) {
                        f.spans = sy.spans.clone();
                        f.pos = sy.pos.clone();
                    }
                }
            }
        }
    }

    /// The mount assembly, per world: the decoded archive relocates
    /// (scope-ledger namespacing, identity rebind) and mounts as the
    /// walked pkgs — the same yield a url dep rides. Decode happens
    /// once, at [`Bundle::parse`]; this half repeats per offer, which
    /// is exactly the per-world relocation law. A source member walks
    /// to its one source pkg.
    pub fn walk(&self) -> Result<Walk, RunError> {
        match &self.inner {
            Inner::Archive { entries, layout } => {
                crate::loader::walk_decoded(entries, layout.clone())
            }
            Inner::Source { name, text } => Ok(Walk {
                pkgs: vec![Mounted::source(name, text)],
                root: name.clone(),
            }),
        }
    }

    /// `true` for a host archive — its root is a declaration surface
    /// the embedding Rust binds, never a program (the run lane's door
    /// refuses it; an embedder may still mount the rows).
    pub fn is_host(&self) -> bool {
        matches!(
            &self.inner,
            Inner::Archive { layout: Layout::Decl { .. }, .. }
        )
    }

    /// The mount specs the archive carries: the root's manifest name
    /// and, for a compiled root, every group's identity key — the
    /// spellings a symbol table's sections answer to. A source member
    /// carries its own name.
    pub fn carried_specs(&self) -> Vec<String> {
        match &self.inner {
            Inner::Archive { layout, .. } => match layout {
                Layout::Compiled { manifest, groups, .. } => {
                    let mut out = Vec::new();
                    if let Some(name) = &manifest.name {
                        out.push(name.clone());
                    }
                    for (prefix, _) in groups {
                        if let Some(key) =
                            prefix.strip_prefix(&format!("{}/", crate::bundle::DEPS_DIR))
                        {
                            out.push(key.to_string());
                        }
                    }
                    out
                }
                Layout::Decl { manifest, .. } => manifest.name.clone().into_iter().collect(),
            },
            Inner::Source { name, .. } => vec![name.clone()],
        }
    }
}
