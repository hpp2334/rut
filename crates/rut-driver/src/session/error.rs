//! Why a use path did not resolve — the resolver's error currency.
//! All derived with thiserror; the Display texts are the resolver's
//! law (byte-for-byte).

use thiserror::Error;

/// Why a use path did not resolve. A miss points at the consumer
/// manifest — the `[deps]` table (or the host) decides what exists —
/// unless the missed name is a declared optional peer of a mounted pkg
///: then the dedicated missing-peer error answers, never
/// the bare text.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ResolveError {
    /// not `[a-zA-Z0-9_]+`
    #[error("malformed package name `{spec}` — package names are bare `[a-zA-Z0-9_]+` identifiers")]
    BadSpec { spec: String },
    /// no module with that exact name is mounted
    #[error("cannot resolve `{spec}` — no module with that name is mounted; declare it in your `rut.jsonc` `deps`")]
    NoModule { spec: String },
    /// D2: the name is an OPTIONAL peer some mounted pkg
    /// declared, and the peer is absent — its integration group never
    /// mounted. Names the pkg, the peer, the integration it unlocks,
    /// and the fix. (A REQUIRED peer's absence is louder still: D1 at
    /// mount, so it never reaches resolve through the loader.)
    #[error("cannot resolve `{spec}` — `{pkg}`'s {spec} integration is not mounted because the optional peer `{spec}` is absent from this program's closure; add `\"{spec}\": {{ \"path\": \"..\" }}` to your `rut.jsonc` `deps`")]
    PeerMissing { spec: String, pkg: String },
    /// D5: the name IS mounted — the closure pulled it in — but the
    /// CONSUMER does not declare it. Nameability is declaration-scoped:
    /// a pkg may `use` exactly the heads its own manifest names (`deps`
    /// ∪ `peer-deps` ∪ the program root's `dev-deps`). Names the head,
    /// the mount edge (who pulled it in — the `mounted_by` first-mount
    /// edge; a hand offer has none and renders the structural
    /// "offered by the host" spelling), the consumer, and the fix.
    /// `via`/`fix` carry the two renderings, computed in
    /// [`ResolveError::not_declared`] — thiserror interpolates fields,
    /// not branches.
    #[error("cannot resolve `{head}` — `{head}` is mounted in this program ({via}) but `{consumer}` does not declare it; {fix}")]
    NotDeclared {
        head: String,
        consumer: String,
        via: String,
        fix: String,
    },
}

impl ResolveError {
    /// D5's two renderings of the mount edge and the fix. A walked
    /// (or archive-group) consumer names its puller and the jsonc row
    /// to add; a hand-mounted one (no `mounted_by` edge) gets the
    /// STRUCTURAL spelling — the host builds manifests in memory, so
    /// the fix names the builder doors, never a file.
    pub(crate) fn not_declared(head: &str, consumer: &str, via: Option<&str>) -> ResolveError {
        let (via, fix) = match via {
            Some(puller) => (
                format!("pulled by `{puller}`"),
                format!("add \"{head}\": {{ \"path\": \"..\" }} to `{consumer}`'s `rut.jsonc` `deps`"),
            ),
            None => (
                "offered by the host".to_string(),
                format!(
                    "declare it in the pkg's manifest — `Manifest::lib(\"{consumer}\", &[\"{head}\"])` or a `\"deps\"` row through `parse_manifest(..)`"
                ),
            ),
        };
        ResolveError::NotDeclared {
            head: head.to_string(),
            consumer: consumer.to_string(),
            via,
            fix,
        }
    }
}

