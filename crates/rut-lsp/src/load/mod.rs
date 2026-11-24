//! The load layer — how files ENTER the world: the read-only dep walk
//! ([`dep_walk`], the extension's manifest walk moved into the crate),
//! the manifest and lock bridge ([`manifest`]), and the engine
//! prelude ([`prelude`], the boot's one ambient entry). Pure reads
//! over the injected host environment — the layer never names the
//! features above it; the session calls down into it.

pub(crate) mod dep_walk;
pub(crate) mod manifest;
pub(crate) mod prelude;
