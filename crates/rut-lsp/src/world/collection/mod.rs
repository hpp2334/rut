//! The mounted-file store — every file in the session's world is a
//! [`MountedFile`]: the document (the kept tree), its provenance stamp,
//! and the decl map (name → node address, built once per file version).
//! The [`DefIndexCollection`] owns them and answers the cross-file
//! questions. The subtree: `member` (the member law's resolver family
//! — hover, definition, and completion share it), `tiers` (completion's
//! folded tier doors and THE file-scope resolver), `views` (the typed
//! views over `(Doc, AnyHandle)` — a view's verbatim text is a span×src
//! join on the OWNING file's document, its line number is `doc.line_of`,
//! its `ambient`/vis verdict derives from the node's modifier chain —
//! nothing stored, nothing stale).
//!
//! The doors are questions with one home each: the FILE SCOPE is
//! [`Chain::file_scope`] (tiers) — what the queried file binds by one
//! name, own decls ∪ use-bound ∪ ambient, the law the one-name Act 2
//! spelled (sibling modules may share a name; a name the file never
//! binds answers nothing). The three flat doors re-spell that one
//! home: [`Chain::type_decl`] the file scope's first type,
//! [`Chain::functions`] the 0/1/many honesty rule,
//! [`Chain::module_let`] the unique-or-none rule. The member law
//! lives in `member`, completion's tiers in `tiers`, and the
//! body-over-decl preference once in [`DefIndexCollection::prefer_bodies`].

mod member;
mod tiers;
mod views;

pub(crate) use member::{enclosing_type, used_ifaces, MemberHit, MemberTarget, RecvNode};
pub(crate) use tiers::{bound_has, ExportRow, MemberRow, UseExports};
pub(crate) use views::{FnView, GlobUse, LetView, MemberView, ModView, TyView, UseView};

use rut_ast::{
    AnyHandle, BuiltinPrimitiveNode, BuiltinTyNode, ItemRef, MethodRef, SurfaceFnNode, UseNode,
};

use crate::semantic::{mode_of, Document};
use crate::world::mods::DocMods;
use crate::world::types::Mount;

// ---- the decl map — name → node address, one pass per file version ----

/// one file's decl inventory: the node ADDRESSES in source order, the
/// name lookups resolving through the interner at query time. Built
/// once per file version (at mount / at sync); views derive every
/// other fact on read.
#[derive(Debug, Default)]
pub(crate) struct DeclMap {
    /// type decls — class/struct/interface/enum/builtin/primitive/
    /// host-struct/alias, in source order
    types: Vec<AnyHandle>,
    /// fns — free fns, surface fns, and impl-block methods (an impl
    /// method's owner derives from its parent impl's target), in
    /// source order. `builtin impl` blocks record nothing: their
    /// methods are compiler-lowered, core's own surfaces answer.
    fns: Vec<AnyHandle>,
    /// module-scope lets
    lets: Vec<AnyHandle>,
    /// use edges (one row per imported name rides the view, not here)
    uses: Vec<AnyHandle>,
    /// `mod NAME;` namespace edges
    mods: Vec<AnyHandle>,
}

/// The decl map builder — the small pass that replaced the row
/// extractor: walk the root items once, record the node addresses.
/// Error slots render no rows (errors are the panel's business).
pub(crate) fn decls_of(doc: &Document) -> DeclMap {
    let ast = doc.ast();
    let mut m = DeclMap::default();
    for item in ast.root().items(ast) {
        match item {
            ItemRef::Class(h) => m.types.push(h.any()),
            ItemRef::Struct(h) => m.types.push(h.any()),
            ItemRef::Interface(h) => m.types.push(h.any()),
            ItemRef::Enum(h) => m.types.push(h.any()),
            ItemRef::BuiltinTy(h) => m.types.push(h.any()),
            ItemRef::BuiltinPrimitive(h) => m.types.push(h.any()),
            ItemRef::SurfaceStruct(h) => m.types.push(h.any()),
            ItemRef::Alias(h) => m.types.push(h.any()),
            ItemRef::Fn(h) => m.fns.push(h.any()),
            ItemRef::SurfaceFn(h) => m.fns.push(h.any()),
            // the inherent impl owns its methods — they ride the fns
            // list at the impl's own position; `BuiltinImpl` records
            // nothing (compiler-lowered)
            ItemRef::Impl(h) => {
                for meth in &h.get(ast).methods {
                    let MethodRef::Method(mh) = meth else {
                        continue; // the recovery slot renders no row
                    };
                    m.fns.push(mh.any());
                }
            }
            ItemRef::ModuleLet(h) => m.lets.push(h.any()),
            ItemRef::Use(h) => m.uses.push(h.any()),
            ItemRef::ModDecl(h) => m.mods.push(h.any()),
            ItemRef::Module(_) | ItemRef::BuiltinImpl(_) | ItemRef::Error(_) => {}
        }
    }
    m
}

// ---- the mounted file + the collection ----

/// One file in the session's world: the document (tree KEPT), the
/// provenance stamp, and the decl map built once per version. A plain
/// record: many construction sites (boot prelude, dep walk, workspace
/// scan, doc sync), no invariant beyond "decls describes doc".
pub(crate) struct MountedFile {
    /// the document — the pipeline owner (text, tokens, tree) the
    /// views join over
    pub(crate) doc: Document,
    /// the per-file stamp — origin/module/mod_path/mode, ONE record
    /// built at the mount site (shape unchanged from the row era)
    pub(crate) mount: Mount,
    /// the decl map — node addresses, built once per file version
    pub(crate) decls: DeclMap,
}

impl MountedFile {
    /// parse fixture/mounted source into a mounted file — the mode
    /// derives from the mount's stamp when it carries one, else the
    /// origin's filename convention (`mode_of`)
    pub(crate) fn from_src(mount: Mount, src: String) -> MountedFile {
        let mode = mount.mode.unwrap_or_else(|| mode_of(&mount.origin));
        MountedFile::from_document(Document::parse(mode, src), mount)
    }

    /// mount an already-parsed document (the sync lane's shape)
    pub(crate) fn from_document(doc: Document, mount: Mount) -> MountedFile {
        let decls = decls_of(&doc);
        MountedFile { doc, mount, decls }
    }

    /// the file's origin (its provenance label; the jump targets'
    /// uri source)
    pub(crate) fn origin(&self) -> &str {
        &self.mount.origin
    }

    /// the file's IDENTITY — one physical file however many lanes
    /// mounted it (the host's scan under its `file://` uri, the dep
    /// walk under its rooted string): [`crate::world::pkg::file_key`]
    /// over the origin
    pub(crate) fn key(&self) -> String {
        crate::world::pkg::file_key(&self.mount.origin)
    }

    /// is this the same physical file as `other`? The open document's
    /// entry and its stamped twin are one file by this test (their
    /// origins spell the same path).
    pub(crate) fn same_file(&self, other: &MountedFile) -> bool {
        std::ptr::eq(self, other) || self.key() == other.key()
    }

    // ---- per-file decl views, in source order ----

    pub(crate) fn types(&self) -> Vec<TyView<'_>> {
        self.decls
            .types
            .iter()
            .map(|&h| TyView { file: self, h })
            .collect()
    }

    pub(crate) fn fns(&self) -> Vec<FnView<'_>> {
        self.decls
            .fns
            .iter()
            .map(|&h| FnView { file: self, h })
            .collect()
    }

    pub(crate) fn lets(&self) -> Vec<LetView<'_>> {
        self.decls
            .lets
            .iter()
            .map(|&h| LetView { file: self, h })
            .collect()
    }

    /// use views — one per imported name (a brace list fans out, the
    /// names in order; a glob leaf fans out none — the star is never
    /// offered)
    pub(crate) fn uses(&self) -> Vec<UseView<'_>> {
        let mut out = Vec::new();
        for &h in &self.decls.uses {
            let Some(u) = h.downcast::<UseNode>() else {
                continue;
            };
            for i in 0..u.get(self.doc.ast()).names.len() {
                out.push(UseView { file: self, h, i });
            }
        }
        out
    }

    /// the glob leaves — the `use pkg::*` edges, in source order (they
    /// fan out no name rows above, so the glob lanes read the nodes
    /// directly)
    pub(crate) fn glob_uses(&self) -> Vec<GlobUse<'_>> {
        let ast = self.doc.ast();
        let mut out = Vec::new();
        for &h in &self.decls.uses {
            let Some(u) = h.downcast::<UseNode>() else {
                continue;
            };
            let d = u.get(ast);
            if d.glob.is_none() {
                continue;
            }
            let Some(head) = d.path.first() else {
                continue;
            };
            out.push(GlobUse {
                pkg: ast.name(head.ident),
                interior: d.path[1..]
                    .iter()
                    .map(|n| ast.name(n.ident).to_string())
                    .collect(),
                vis: d.vis(),
            });
        }
        out
    }

    pub(crate) fn mods(&self) -> Vec<ModView<'_>> {
        self.decls
            .mods
            .iter()
            .map(|&h| ModView { file: self, h })
            .collect()
    }

    /// the type decl named `name` — first match in source order
    pub(crate) fn ty(&self, name: &str) -> Option<TyView<'_>> {
        self.types().into_iter().find(|t| t.name() == name)
    }

    /// the names whose decl spelled the import-gated builtin linkage
    /// (`pub builtin` — the core disposal pair): completion offers them
    /// only when the document's `use` names them, the same ambient
    /// split the compiler binds by. DERIVED at read from the modifier
    /// chains — never stored.
    pub(crate) fn pub_gated(&self) -> Vec<String> {
        let ast = self.doc.ast();
        let mut out = Vec::new();
        for &h in &self.decls.fns {
            if let Some(d) = h.downcast::<SurfaceFnNode>() {
                let d = d.get(ast);
                if matches!(d.linkage(), rut_ast::Linkage::Builtin { ambient: false }) {
                    out.push(ast.name(d.name.ident).to_string());
                }
            }
        }
        for &h in &self.decls.types {
            if let Some(d) = h.downcast::<BuiltinTyNode>() {
                if !d.get(ast).ambient() {
                    out.push(ast.name(d.get(ast).name.ident).to_string());
                }
            } else if let Some(d) = h.downcast::<BuiltinPrimitiveNode>() {
                if !d.get(ast).ambient() {
                    out.push(ast.name(d.get(ast).name.ident).to_string());
                }
            }
        }
        out
    }
}

/// the session's mounted world — the files in chain order (the prelude
/// boots first, every mount replaces by origin in place), answering the
/// cross-file questions. Rebuilt on dep-walk mount — never per query.
/// The OPEN documents live in the session's versioned cache
/// (`SessionState.docs`, each a [`MountedFile`]); the query chain is
/// the two combined — the doc first, the world after.
pub(crate) struct DefIndexCollection {
    files: Vec<MountedFile>,
}

impl DefIndexCollection {
    /// the empty world — the boot lane mounts the prelude first
    pub(crate) fn new() -> DefIndexCollection {
        DefIndexCollection { files: Vec::new() }
    }

    /// the defs store's replace-by-FILE law — one entry per physical
    /// file, re-mounting replaces (the world stays idempotent under
    /// re-walks, and the two lanes that may mount one workspace file —
    /// the host's scan under its `file://` uri, the dep walk under its
    /// rooted string — collapse into the ONE entry, so a file's decls
    /// never speak twice). The entry keeps the origin it FIRST mounted
    /// under (the spelling the open document's lookups key on); its
    /// stamp and text are the latest lane's — the walk's is the
    /// loader's law.
    pub(crate) fn mount(&mut self, mut file: MountedFile) {
        let key = file.key();
        match self.files.iter().position(|f| f.key() == key) {
            Some(slot) => {
                file.mount.origin = self.files[slot].mount.origin.clone();
                self.files[slot] = file;
            }
            None => self.files.push(file),
        }
    }

    /// the file mounted at `origin` (by identity — either spelling of
    /// its path), if any
    pub(crate) fn file_for(&self, origin: &str) -> Option<&MountedFile> {
        let key = crate::world::pkg::file_key(origin);
        self.files.iter().find(|f| f.key() == key)
    }

    /// the query chain — `origin`'s file at the head (the open
    /// document first), the rest of the world after. The chain is an
    /// ordering VIEW over the world's files: the open document's entry
    /// (whose home is the session's versioned cache) rides first, its
    /// stamped twin (the store entry of the same physical file — the
    /// doc's stamp carrier) keeps its place behind it; the doors read
    /// the head's decls and the twin's stamp, never the twin's decls
    /// ([`MountedFile::same_file`] is the exclusion).
    pub(crate) fn chain_with<'a>(&'a self, head: &'a MountedFile) -> Chain<'a> {
        let mut files = Vec::with_capacity(1 + self.files.len());
        files.push(head);
        files.extend(self.files.iter());
        Chain::new(files)
    }

    pub(crate) fn files(&self) -> &[MountedFile] {
        &self.files
    }

    /// body-over-decl — the preference's one home (the chain's door
    /// delegates here)
    pub(crate) fn prefer_bodies<'a, T: Copy>(
        hits: &[(&'a MountedFile, T)],
    ) -> Vec<(&'a MountedFile, T)> {
        let mounted: Vec<(&MountedFile, T)> = hits
            .iter()
            .copied()
            .filter(|(f, _)| f.mount.src_path.is_none())
            .collect();
        let pool: &[(&MountedFile, T)] = if mounted.is_empty() { hits } else { &mounted };
        let bodies: Vec<(&MountedFile, T)> = pool
            .iter()
            .copied()
            .filter(|(f, _)| f.mount.mode != Some(crate::semantic::Mode::Decl))
            .collect();
        if bodies.is_empty() {
            pool.to_vec()
        } else {
            bodies
        }
    }
}

/// the query chain — an ORDERING VIEW over the world's files, the
/// queried document at the head. The head IS the open document (its
/// fresh entry, rebuilt on sync); queries iterate `iter()` exactly
/// where the old code iterated the `&[&DefIndex]` chain. The
/// collection's DOORS hang off the chain (one home per question).
pub(crate) struct Chain<'a> {
    files: Vec<&'a MountedFile>,
    /// the head's position — the open document
    head: usize,
    /// the head's file-module stamp — where in its package's mod tree
    /// the head sits (a stamped head speaks for itself; the open
    /// document's entry is unstamped, its place is its stamped twin's)
    stamp: DocMods,
}

/// the head's stamp over a file list: its own mount when the head is
/// a stamped file, else the stamped twin's — the same physical file's
/// store entry ([`MountedFile::same_file`])
fn stamp_of(files: &[&MountedFile]) -> DocMods {
    let head = files[0];
    let of = |f: &MountedFile| DocMods {
        mod_path: f.mount.mod_path.clone(),
        pkg: f.mount.module.clone(),
    };
    if head.mount.module.is_some() {
        return of(head);
    }
    files
        .iter()
        .skip(1)
        .find(|f| f.mount.module.is_some() && head.same_file(f))
        .map_or_else(DocMods::default, |f| of(f))
}

impl<'a> Chain<'a> {
    /// the chain over the files, `files[0]` the queried document —
    /// its stamp read off the list (the stamped twin rides behind)
    pub(crate) fn new(files: Vec<&'a MountedFile>) -> Chain<'a> {
        assert!(!files.is_empty(), "a chain rides at least its head");
        let stamp = stamp_of(&files);
        Chain {
            files,
            head: 0,
            stamp,
        }
    }

    /// the open document's own file — the chain head
    pub(crate) fn head(&self) -> &'a MountedFile {
        self.files[self.head]
    }

    /// the head's file-module context — the (pkg, mod path) the
    /// own-package lanes key on ([`DocMods`]); the default (all
    /// `None`) for a document no lane stamped
    pub(crate) fn stamp(&self) -> &DocMods {
        &self.stamp
    }

    /// the package a chain file speaks for: a stamped file's own
    /// manifest name; the open document's entry — its twin's stamp
    /// ([`Chain::stamp`])
    pub(crate) fn pkg_of(&self, f: &'a MountedFile) -> Option<&str> {
        if std::ptr::eq(f, self.head()) {
            self.stamp.pkg.as_deref()
        } else {
            f.mount.module.as_deref()
        }
    }

    /// the mod path a chain file sits at: a stamped file's own; the
    /// open document's entry — its twin's stamp
    pub(crate) fn mod_path_of(&self, f: &'a MountedFile) -> &str {
        if std::ptr::eq(f, self.head()) {
            self.stamp.mod_path.as_deref().unwrap_or("")
        } else {
            crate::world::mods::mod_path_of(f)
        }
    }

    /// the whole chain, doc first — the old `idxs.iter()` shape
    pub(crate) fn iter(&self) -> impl Iterator<Item = &'a MountedFile> + Clone + '_ {
        let (head, tail) = self.files.split_at(self.head + 1);
        head.iter().copied().chain(tail.iter().copied())
    }

    // ---- the collection's doors — the flat re-spellings of the ONE
    //      file-scope home ([`Chain::file_scope`] in `tiers`) ----

    /// the type named `name` — the QUERIED file's view: the head's own
    /// decl, the use-bound decl, or the ambient prelude row (the
    /// mounted-over-embed preference survives INSIDE the ambient
    /// layer — the mounted carrier over the embedded twin, the
    /// fallback shape). A name the file never binds answers `None` —
    /// never a foreign decl (sibling modules may share a name; the
    /// old flat sweep answered for all of them).
    pub(crate) fn type_decl(&self, name: &str) -> Option<TyView<'a>> {
        self.file_scope(name).types.into_iter().next()
    }

    /// the fns named `name` in the queried file's view — 0/1/many, the
    /// honesty rule typed (the file scope's own fns + its use-bound +
    /// ambient rows; a consumer that wants free fns only filters on
    /// `owner()`)
    pub(crate) fn functions(&self, name: &str) -> Vec<FnView<'a>> {
        self.file_scope(name).fns
    }

    /// the module let named `name` in the queried file's view —
    /// unique-or-none inside (ambiguity stays a miss, the honesty
    /// rule)
    pub(crate) fn module_let(&self, name: &str) -> Option<LetView<'a>> {
        match self.file_scope(name).lets.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }

    /// body-over-decl — THE single preference: a package that indexes
    /// BOTH its `.d.rut` surface (`mode == Some(Decl)`) and its
    /// `mod.rut` body (`Impl` — or any mount whose builder did not
    /// stamp) answers from the body; the surface hits ride only when
    /// they are all there is. One precedence above the body law: a
    /// MOUNTED dep outranks the ambient prelude — the prelude is the
    /// zero-fetch fast path for the ONE embedded pkg, a fallback, never
    /// the answer when the real thing is mounted (its `src_path` marks
    /// it). The generic payload keeps one law for every shape the
    /// layers collect (spans, views).
    pub(crate) fn prefer_bodies<T: Copy>(
        &self,
        hits: &[(&'a MountedFile, T)],
    ) -> Vec<(&'a MountedFile, T)> {
        DefIndexCollection::prefer_bodies(hits)
    }

    /// the scan's per-candidate chain — `f` at the head, the rest of
    /// `base` after (no file speaks twice; the references scan's law).
    /// `stamp` is the candidate's own file-module context: the open
    /// document's is its query's (the two queries cannot disagree
    /// about the doc's idents), another file's is its own mount's.
    pub(crate) fn reordered(base: &Chain<'a>, f: &'a MountedFile, stamp: DocMods) -> Chain<'a> {
        let mut files: Vec<&'a MountedFile> = Vec::with_capacity(base.files.len());
        files.push(f);
        files.extend(base.files.iter().copied().filter(|x| !x.same_file(f)));
        Chain {
            files,
            head: 0,
            stamp,
        }
    }
}

/// `find_ty` over a plain file window — the inference pass rides
/// slices (a one-file window for a module-let's initializer, the
/// chain's pointer slice inside the binding pass); the mounted-over-
/// embed preference spelled here for the window shape. The windows
/// resolve SHAPES (a type head behind a value), not spellings — the
/// file-scope law gates the spelling lanes
/// ([`Chain::type_decl`]/[`Chain::file_scope`]), not this sweep.
pub(crate) fn find_ty<'a>(idxs: &[&'a MountedFile], name: &str) -> Option<TyView<'a>> {
    let mut fallback = None;
    for f in idxs.iter().copied() {
        if let Some(t) = f.ty(name) {
            if f.mount.src_path.is_none() {
                return Some(t);
            }
            if fallback.is_none() {
                fallback = Some(t);
            }
        }
    }
    fallback
}
