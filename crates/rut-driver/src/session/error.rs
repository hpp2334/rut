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
    #[error("cannot resolve `{spec}` — no module with that name is mounted; declare it in your `rut.json` `deps`")]
    NoModule { spec: String },
    /// D2: the name is an OPTIONAL peer some mounted pkg
    /// declared, and the peer is absent — its integration group never
    /// mounted. Names the pkg, the peer, the integration it unlocks,
    /// and the fix. (A REQUIRED peer's absence is louder still: D1 at
    /// mount, so it never reaches resolve through the loader.)
    #[error("cannot resolve `{spec}` — `{pkg}`'s {spec} integration is not mounted because the optional peer `{spec}` is absent from this program's closure; add `\"{spec}\": {{ \"path\": \"..\" }}` to your `rut.json` `deps`")]
    PeerMissing { spec: String, pkg: String },
}

