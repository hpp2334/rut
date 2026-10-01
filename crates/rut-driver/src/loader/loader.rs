//! THE embedder door — the [`Loader`] builder and its [`Loaded`]
//! product. Non-generic: the remote rides as `Option<Box<dyn
//! DepRemote>>`, the fs policy as `Option<&dyn Source>` (default
//! [`crate::bundle::FsSource`]).
//!
//! The no-remote law is LAZY: `build()` validates only the path shape;
//! the remote matters only when the dep walk actually reaches a url
//! row — THAT is where [`Loaded::load`] panics, naming the dep, the
//! url, and the fix. Url-free projects and `.rutbundle`s (closed —
//! they never fetch) load with no remote named at all. No silent
//! policy, no env fallback: the embedder owns its transport.

use std::path::{Path, PathBuf};

use crate::bundle::{FsSource, Source};
use crate::loader::error::LoadError;
use crate::loader::DepRemote;
use crate::session::Session;

/// The embedder's builder: a project path (a module directory or a
/// `.rutbundle`), an optional [`Source`] override, an optional
/// [`DepRemote`].
pub struct Loader<'a> {
    project: PathBuf,
    source: Option<&'a dyn Source>,
    remote: Option<Box<dyn DepRemote>>,
}

impl<'a> Loader<'a> {
    /// Aim the loader at a project: a module directory (`rut.jsonc`) or
    /// a packed `.rutbundle`.
    pub fn new(project: impl Into<PathBuf>) -> Self {
        Loader {
            project: project.into(),
            source: None,
            remote: None,
        }
    }

    /// The fs policy: where a directory's files come from. Default:
    /// the real filesystem. (A `.rutbundle` reads only its own bytes —
    /// the source never touches it.)
    pub fn source(mut self, src: &'a dyn Source) -> Self {
        self.source = Some(src);
        self
    }

    /// The remote policy: how url-dep bytes arrive and cache. Explicit
    /// when needed — the lazy check in [`Loaded::load`] fires only if
    /// a url row is ever reached.
    pub fn dep_remote(mut self, r: impl DepRemote + 'static) -> Self {
        self.remote = Some(Box::new(r));
        self
    }

    /// Cheap validation only (path shape: dir / `.rutbundle`) — NO
    /// remote check here; the remote matters only when a url row is
    /// reached.
    pub fn build(self) -> Loaded<'a> {
        Loaded {
            project: self.project,
            source: self.source,
            remote: self.remote,
        }
    }
}

/// A validated, ready-to-load project — [`Loader::build`]'s product.
pub struct Loaded<'a> {
    project: PathBuf,
    source: Option<&'a dyn Source>,
    remote: Option<Box<dyn DepRemote>>,
}

impl Loaded<'_> {
    /// Load the project: a directory through the manifest lane (the
    /// remote, when set, serves its url rows), a `.rutbundle` through
    /// the closed bundle lane (never fetches, so it never needs a
    /// remote).
    ///
    /// PANICS only at the fetch point — a url dep is reached and no
    /// remote was set:
    /// "dep `pouch` needs `https://…` but this Loader has no
    /// dep_remote — call .dep_remote(HttpRemote::project_local(
    /// <project>)) (or your own DepRemote)"
    pub async fn load(self) -> Result<(Session, String), LoadError> {
        let path = Path::new(&self.project);
        if path.is_dir() {
            let map = super::prefetch_urls_lazy(
                path,
                self.source.unwrap_or(&FsSource),
                self.remote.as_deref(),
            )
            .await?;
            return super::load_dir_session_fetched(path, self.source.unwrap_or(&FsSource), &map)
                .map(|loaded| (loaded.session, loaded.root));
        }
        if path.extension().map_or(false, |e| e == "rutbundle") {
            return super::load_bundle_session(path);
        }
        Err(LoadError::Shape {
            path: self.project.display().to_string(),
        })
    }
}
