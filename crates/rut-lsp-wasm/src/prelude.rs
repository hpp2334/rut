//! The engine prelude's mirror — `core`, the ONE ambient index. The
//! engine auto-offers the real prelude to every compilation unit, so
//! the editor keeps exactly that asymmetry: `core` embeds here (no dep
//! lane can ever deliver it — a manifest's `core` row would be a
//! contradiction of the engine's own law), while EVERY other std pkg
//! is an ordinary third-party lib (manifest + store, or nothing). The
//! manifest/deps lane is std's one channel — the same walk that mounts
//! any url dep mounts the std source zips `rut new` scaffolds.
//!
//! The mirror indexes exactly like a dep's file: the ordinary hover
//! pass over the Decl-mode parse (the `.d.rut` filename convention),
//! origin `core`, the TRUE repo-relative source path (the definition
//! layer jumps there), and the manifest-shape stamps (`module`, mode).
//! The ambient pair (`StackTrace`, `panic` — `prelude builtin`
//! spellings) keeps completing plain in the bare tier exactly as the
//! engine binds them, while the import-gated `pub builtin` names ride
//! `DefIndex::pub_gated` (the compiler's ambient split, mirrored).

use crate::hover::{self, DefIndex, Mount};

/// the prelude's single embedded source — the engine's own decl file
pub const CORE: &str = include_str!("../../../rut/core/src/core.d.rut");

/// the engine decl file's repo-relative path — the `src_path` stamp
/// (the definition layer jumps there, so a workspace that is the rut
/// repo lands in the real file)
const CORE_PATH: &str = "rut/core/src/core.d.rut";

/// the prelude's definition index — `SessionState::boot`'s one ambient row
/// (every other index in the chain arrives through a lane: the dep
/// walk, the workspace scan, the open document). The mount — origin,
/// true source path, module name, mode — is ONE record built here and
/// handed to the builder (never patched on after the fact).
pub fn index() -> DefIndex {
    // the mode derives from the path (`mode_of` — the `.d.rut` suffix
    // IS the declaration surface's mode, the filename convention every
    // lane reads)
    let doc = rut_semantic::Document::parse(rut_semantic::mode_of(CORE_PATH), CORE.to_string());
    hover::index(
        &doc,
        Mount {
            origin: "core".to_string(),
            src_path: Some(CORE_PATH.to_string()),
            module: Some("core".to_string()),
            mode: Some(rut_semantic::mode_of(CORE_PATH)),
            ..Mount::default()
        },
    )
}
