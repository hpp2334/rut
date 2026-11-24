//! The mounted world — the session's accumulated language facts, ONE
//! store: every file the session knows (the prelude, the walked deps,
//! the workspace pushes, the open documents) is one [`File`] in one
//! [`World`], keyed by origin. Opening a document adopts the file its
//! loader lane laid down under that origin; syncing edits it in place;
//! an open file's buffer is the truth for every reader. No second
//! store, no side parameter carrying a file's own identity.
//!
//! Two read layers live beside the store (the lsp-features survey §3.2
//! design): the **decl layer** (the files' decl maps — types, fns,
//! fields, enum members, module lets, use names, all keyed on the
//! nodes' own recorded name spans; docs ride the decl nodes' `DocData`
//! chains — see [`decls`]) and the **binding pass** (`bindings` — every
//! fn-local binding as `{ name, decl_ident_span, scope_span, kind, ty }`,
//! shadow-correct resolve). `infer` is the display-side type heuristic
//! both consume. Heuristic, like the classifier: a miss is an empty
//! answer, never wrong text.
//!
//! Layout: [`World`]/[`File`]/[`Stamp`] here, the per-file doors in
//! `decls`, `types` (the shared vocabulary — TyForm, type text) →
//! `collection` (the resolver family in `member`, completion's tier
//! doors in `tiers`, the per-node views in `views/`) → `bindings` +
//! `infer` (what's under the cursor) → `mods` (the file-module
//! namespace walk) + `pkg` (package identity and the doc's import
//! table). The queries above spell this layer — never the reverse.

pub mod bindings;
pub(crate) mod collection;
pub(crate) mod decls;
pub(crate) mod infer;
pub(crate) mod mods;
pub(crate) mod pkg;
pub(crate) mod types;

pub use bindings::{BindKind, Binding};
pub(crate) use decls::{decls_of, DeclMap};

use std::collections::HashMap;

use crate::semantic::{mode_of, Document, Mode};

pub(crate) type FileId = u32;

/// how a file entered the session: the boot prelude, the dep walk, the host's
/// workspace push (`index_file` / `index_mod_at`), or an open document no loader
/// ever mounted. Scratch files are visible only as themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    Prelude,
    Dep,
    Workspace,
    Scratch,
}

/// a file's identity, complete the moment it mounts and replaced only by a later
/// push under the same origin. A plain record: many construction sites, no
/// invariant beyond "the fields spell one file's provenance".
#[derive(Debug, Clone)]
pub(crate) struct Stamp {
    pub lane: Lane,
    pub origin: String, // the replace key: a uri, a rooted path, or `core`
    pub src_path: Option<String>, // an embedded source's true repo path (the prelude)
    pub module: Option<String>, // the manifest name a `use` path spells
    pub mod_path: Option<String>, // `""` is the package root; `None` is not a package file
    pub mode: Mode,     // always derived: the lane's word, else `mode_of(origin)`
}

impl Stamp {
    /// an open document no loader ever mounted — origin only
    pub(crate) fn scratch(uri: &str) -> Stamp {
        Stamp {
            lane: Lane::Scratch,
            origin: uri.to_string(),
            src_path: None,
            module: None,
            mod_path: None,
            mode: mode_of(uri),
        }
    }

    /// the host's workspace push (`index_file` unstamped, `index_mod_at`
    /// at a package's mod path); the parse mode is the uri's filename
    /// convention — the same authority the manifest rows follow
    pub(crate) fn workspace(uri: &str, pkg: Option<&str>, mod_path: Option<&str>) -> Stamp {
        Stamp {
            lane: Lane::Workspace,
            origin: uri.to_string(),
            src_path: None,
            module: pkg.map(str::to_string),
            mod_path: mod_path.map(str::to_string),
            mode: mode_of(uri),
        }
    }

    /// a mounted dep file — the flat shape (`mod_path` `None`) or one
    /// file module of a mod-carrying pkg; the mode is the uri's
    /// filename convention
    pub(crate) fn dep(name: &str, uri: &str, mod_path: Option<&str>) -> Stamp {
        Stamp {
            lane: Lane::Dep,
            origin: uri.to_string(),
            src_path: None,
            module: Some(name.to_string()),
            mod_path: mod_path.map(str::to_string),
            mode: mode_of(uri),
        }
    }
}

/// One file in the session's world: the document (tree KEPT), its
/// identity stamp, whether it is open (an open file's buffer is the
/// truth for every reader), and the decl map built once per version.
/// A plain record: many construction sites (the boot prelude, the dep
/// walk, the workspace scan, the doc sync), no invariant beyond
/// "decls describes doc".
pub(crate) struct File {
    /// the document — the pipeline owner (text, tokens, tree) the
    /// views join over
    pub doc: Document,
    /// the per-file stamp — lane/origin/module/mod_path/mode, ONE
    /// record built at the mount site
    pub stamp: Stamp,
    /// an open file's buffer is the truth
    pub open: bool,
    /// the decl map — node addresses, built once per file version
    pub decls: DeclMap,
}

impl File {
    /// parse mounted source into a world file — closed (a loader's
    /// lane laid it down; the buffer opens later)
    pub(crate) fn from_src(stamp: Stamp, src: String) -> File {
        let doc = Document::parse(stamp.mode, src);
        let decls = decls_of(&doc);
        File {
            doc,
            stamp,
            open: false,
            decls,
        }
    }

    /// an open document no loader ever mounted
    pub(crate) fn open_scratch(uri: &str, src: String) -> File {
        let mut f = File::from_src(Stamp::scratch(uri), src);
        f.open = true;
        f
    }
}

/// the session's world — every file the session knows, in store order
/// (mount order; the prelude is `files[0]`), keyed by origin. Rebuilt
/// never — the store mutates in place per mount/sync/close.
pub(crate) struct World {
    files: Vec<File>,
    by_origin: HashMap<String, FileId>,
}

impl World {
    /// the world as it boots — the engine prelude pre-mounted (`core`
    /// alone, the ONE ambient entry; every other std pkg rides the dep
    /// lane like any third-party lib)
    pub(crate) fn new(prelude: File) -> World {
        let key = crate::world::pkg::file_key(&prelude.stamp.origin);
        let mut by_origin = HashMap::new();
        by_origin.insert(key, 0);
        World {
            files: vec![prelude],
            by_origin,
        }
    }

    /// the file mounted at `origin` (by identity — either spelling of
    /// its path), if any
    pub(crate) fn get(&self, origin: &str) -> Option<FileId> {
        self.by_origin
            .get(&crate::world::pkg::file_key(origin))
            .copied()
    }

    /// the file mounted at `origin`, only when open — the position
    /// queries' gate (a closed file answers no position query)
    pub(crate) fn open(&self, origin: &str) -> Option<FileId> {
        self.get(origin).filter(|id| self.files[*id as usize].open)
    }

    /// the file at `id`
    pub(crate) fn file(&self, id: FileId) -> &File {
        &self.files[id as usize]
    }

    /// the file at `id`, mutable (the sync lane's door)
    pub(crate) fn file_mut(&mut self, id: FileId) -> &mut File {
        &mut self.files[id as usize]
    }

    /// THE BINDING REPLACE LAW — mount `file` under its origin:
    ///
    /// - No file under that origin: push at the end, return the new id.
    /// - A file exists: it takes the incoming stamp and keeps `open`.
    ///   An open file keeps its `Document` and `decls` (the incoming
    ///   text is dropped — the buffer is the truth); a closed file
    ///   takes the incoming `Document` and `decls`.
    /// - A `Scratch` file is removed from its slot and pushed at the
    ///   end first (its id changes) — this reproduces the old store
    ///   order, where the indexed copy of a scratch document was
    ///   appended after the mounted world.
    ///
    /// The entry keeps the origin it FIRST mounted under (the spelling
    /// the session's lookups key on); its stamp and text are the latest
    /// lane's — the walk's is the loader's law.
    pub(crate) fn mount(&mut self, mut file: File) -> FileId {
        let key = crate::world::pkg::file_key(&file.stamp.origin);
        let Some(slot) = self.by_origin.get(&key).copied() else {
            let id = self.files.len() as FileId;
            self.by_origin.insert(key, id);
            self.files.push(file);
            return id;
        };
        // the entry keeps the origin it first mounted under
        file.stamp
            .origin
            .clone_from(&self.files[slot as usize].stamp.origin);
        let open = self.files[slot as usize].open;
        file.open = open;
        if open {
            // the buffer is the truth — the incoming text drops (the
            // swap hands the kept document to `file`; the slot's own
            // contents are replaced below either way)
            let slot_file = &mut self.files[slot as usize];
            std::mem::swap(&mut file.doc, &mut slot_file.doc);
            std::mem::swap(&mut file.decls, &mut slot_file.decls);
        }
        if matches!(self.files[slot as usize].stamp.lane, Lane::Scratch) {
            // store order: the scratch slot empties, the file rides at
            // the end (the old defs-append order)
            self.by_origin.remove(&key);
            self.remove_slot(slot);
            let id = self.files.len() as FileId;
            self.by_origin.insert(key, id);
            self.files.push(file);
            id
        } else {
            self.files[slot as usize] = file;
            slot
        }
    }

    /// THE CLOSE LAW — the file under `origin` closes: a `Scratch`
    /// file is removed (later files shift down; its ids die), any
    /// other file sets `open = false` and KEEPS its text — the buffer
    /// it held at close is what every reader answers from then on.
    pub(crate) fn close(&mut self, origin: &str) {
        let Some(id) = self.get(origin) else {
            return;
        };
        if matches!(self.files[id as usize].stamp.lane, Lane::Scratch) {
            self.by_origin.remove(&crate::world::pkg::file_key(origin));
            self.remove_slot(id);
        } else {
            self.files[id as usize].open = false;
        }
    }

    /// remove slot `i`: every later file shifts down one, and the
    /// origin index follows (ids are stable only between mutations)
    fn remove_slot(&mut self, i: FileId) {
        self.files.remove(i as usize);
        for id in self.by_origin.values_mut() {
            if *id > i {
                *id -= 1;
            }
        }
    }

    /// every file in store order — Scratch excluded (a scratch file is
    /// visible only as itself: never a store answer)
    pub(crate) fn files(&self) -> impl Iterator<Item = FileId> + '_ {
        let files = &self.files;
        (0..files.len() as FileId)
            .filter(move |id| !matches!(files[*id as usize].stamp.lane, Lane::Scratch))
    }

    /// THE QUERY CHAIN — `from` first (the queried file), then every
    /// non-Scratch file in store order except `from`. This is the old
    /// `[head] + defs` sequence with ONE store behind it: one entry
    /// per origin, no file speaks twice.
    pub(crate) fn chain(&self, from: FileId) -> impl Iterator<Item = FileId> + '_ {
        let files = &self.files;
        std::iter::once(from).chain((0..files.len() as FileId).filter(move |id| {
            *id != from && !matches!(files[*id as usize].stamp.lane, Lane::Scratch)
        }))
    }
}
