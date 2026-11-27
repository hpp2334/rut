//! The file scope — THE resolver every bare-name lane answers
//! through: what the QUERIED file binds by one name (own decls ∪
//! use-bound ∪ ambient), the qualified-path lane beside it, and the
//! two staged preferences every collector rides. The scopes' rows are
//! the views; the answers feed hover, definition, references, and the
//! completion tiers alike ([`crate::resolve::exports`] enumerates the
//! surfaces).
//!
//! Assignment law: a SCOPE or binding question lives here
//! (`file_scope` and its layers, `ambiguous_scope`, the one-name
//! refusal, the qualified lane, `prefer_mounted`/`prefer_bodies`);
//! a surface or enumeration question lives beside it in
//! [`crate::resolve::exports`].

use rut_ast::Vis;

use crate::doc::Mode;
use crate::world::ty::Ty;
use crate::world::views::{FnView, LetView, ModView, TyView, UseView};
use crate::world::{File, FileId, World};
/// one file-scope resolve — the layers' views in the walk order the
/// queries spell ([`file_scope`]). Every layer is exact inside:
/// own decls in source order, use-bound rows body-preferred
/// (`prefer_bodies`), ambient rows mounted-preferred.
#[derive(Clone, Default)]
pub(crate) struct FileScope<'a> {
    /// the type decls the scope binds, walk order
    pub types: Vec<TyView<'a>>,
    /// the module lets, walk order
    pub lets: Vec<LetView<'a>>,
    /// the fns — free fns and impl methods alike, walk order
    pub fns: Vec<FnView<'a>>,
}

impl<'a> FileScope<'a> {
    /// the next layer's rows join the walk order
    fn extend(&mut self, more: FileScope<'a>) {
        self.types.extend(more.types);
        self.lets.extend(more.lets);
        self.fns.extend(more.fns);
    }

    fn is_empty(&self) -> bool {
        self.types.is_empty() && self.lets.is_empty() && self.fns.is_empty()
    }
}

// ---- the file scope — the one resolver the flat doors re-spell ----

/// THE file-scope resolver — what the QUERIED file binds by `name`,
/// walked in the compiler's layer order: the file's own decls, then
/// the name its use edges bind, then the ambient prelude rows
/// (`prelude builtin` — the checker's wildcard-site split). A name
/// the file never binds answers an EMPTY scope — never a foreign
/// decl: sibling modules may share a name legally (the one-name
/// law's Act 1), and the old flat sweep answered for all of them.
/// This is the one home — hover, definition, and references answer
/// bare names through it (the flat doors re-spell it on top), the
/// same walk the completion tiers enumerate (`bare`, `use_exports`).
///
/// THE ONE-NAME LAW rides the front door: a name the file binds
/// TWICE by DISTINCT bindings ([`NameBindings::refused`]) is a file
/// the compiler refuses — the single-answer scope is EMPTY here
/// (never a pick between the bindings); the hover lists the
/// candidates through [`ambiguous_scope`].
pub(crate) fn file_scope<'a>(w: &'a World, file: FileId, name: &str) -> FileScope<'a> {
    let mut out = FileScope::default();
    let bindings = name_bindings(w, file, name);
    if bindings.refused() {
        return out;
    }
    out.extend(own_scope_of(w.file(file), name));
    bindings.layer2(w, file, name, &mut out);
    ambient_layers(w, file, name, &mut out);
    out
}

/// the type named `name` — the QUERIED file's view: the file's own
/// decl, the use-bound decl, or the ambient prelude row (the
/// mounted-over-embed preference survives INSIDE the ambient
/// layer — the mounted carrier over the embedded prelude, the
/// fallback shape the old flat `type_decl` carried). A name the
/// file never binds answers `None` — never a foreign decl
/// (sibling modules may share a name; the old flat sweep answered
/// for all of them).
pub(crate) fn type_decl<'a>(w: &'a World, file: FileId, name: &str) -> Option<TyView<'a>> {
    file_scope(w, file, name).types.into_iter().next()
}

/// the fns named `name` in the queried file's view — 0/1/many, the
/// honesty rule typed (the file scope's own fns + its use-bound +
/// ambient rows; a consumer that wants free fns only filters on
/// `owner()`)
pub(crate) fn functions<'a>(w: &'a World, file: FileId, name: &str) -> Vec<FnView<'a>> {
    file_scope(w, file, name).fns
}

/// a module let's type — the WRITTEN annotation when the decl carries
/// one, else the initializer-derived type (the same display-side
/// heuristic the binding pass uses, priced against THIS file's own
/// decls). The resolver's door: the view stays world-side
/// ([`LetView::annotated`]) and names no layer above it — the split
/// that keeps the layering downward-only without changing an answer.
pub(crate) fn module_let_ty(w: &World, file: FileId, l: &LetView<'_>) -> Option<Ty> {
    match l.annotated() {
        Some(t) => Some(t),
        None => {
            crate::resolve::ty_infer::expr_ty(w.file(file), &[w.file(file)], l.init(), &|_, _| None)
        }
    }
}

/// THE mid-edit candidates — the file binds `name` twice by
/// DISTINCT bindings ([`NameBindings::refused`]: two decls of its
/// own, two use leaves, a decl beside a leaf): the hover LISTS every
/// binding, each with its kind and provenance — the own decls first,
/// then each use leaf's rows with the `use` they rode. `None`
/// = the name binds once or never: the single-answer lanes answer
/// it. (Definition stays silent on an ambiguous name — never a
/// pick.)
pub(crate) fn ambiguous_scope<'a>(
    w: &'a World,
    file: FileId,
    name: &str,
) -> Option<Vec<Candidate<'a>>> {
    let bindings = name_bindings(w, file, name);
    if !bindings.refused() {
        return None;
    }
    let mut out: Vec<Candidate<'a>> = Vec::new();
    for t in &bindings.own.types {
        out.push(Candidate::own(CandRow::Ty(*t)));
    }
    for l in &bindings.own.lets {
        out.push(Candidate::own(CandRow::Let(*l)));
    }
    for f in &bindings.own.fns {
        out.push(Candidate::own(CandRow::Fn(*f)));
    }
    for m in &bindings.own.mods {
        out.push(Candidate::own(CandRow::Mod(*m)));
    }
    for u in &bindings.leaves {
        let mut rows = FileScope::default();
        leaf_scope(w, file, u, name, &mut rows);
        if rows.is_empty() {
            // the leaf binds nothing of its own (a miss, a
            // visibility gate) — the spelling itself is the
            // candidate
            out.push(Candidate::own(CandRow::Leaf(*u)));
            continue;
        }
        out.extend(rows.into_candidates(*u));
    }
    Some(out)
}

/// ONE name's bindings in the queried file — the analysis every
/// scope door reads ([`NameBindings`]): the own module-scope decls
/// and the explicit use leaves
fn name_bindings<'a>(w: &'a World, file: FileId, name: &str) -> NameBindings<'a> {
    NameBindings {
        own: own_binders(w.file(file), name),
        leaves: w
            .file(file)
            .uses()
            .into_iter()
            .filter(|u| u.name() == name)
            .collect(),
    }
}

/// EVERY name the head's use edges bind — the file scope's layer 2
/// over all names at once (the same analysis
/// [`file_scope`] runs per name). Completion's enumeration gates
/// ride it: a foreign row is offered exactly when the file scope
/// would answer its name with it (a twice-bound name offers
/// nothing).
pub(crate) fn use_bound_scope<'a>(w: &'a World, file: FileId) -> FileScope<'a> {
    let doc = w.file(file);
    let mut names: Vec<&str> = doc.uses().iter().map(|u| u.name()).collect();
    names.sort_unstable();
    names.dedup();
    let mut out = FileScope::default();
    for name in names {
        let bindings = name_bindings(w, file, name);
        if !bindings.refused() {
            bindings.layer2(w, file, name, &mut out);
        }
    }
    out
}

/// the one-name law's refusal for `name` in the queried file — it
/// binds the name twice by DISTINCT bindings
/// ([`NameBindings::refused`]): the compiler refuses the file at
/// the second spelling, so no lane PICKS an answer for it
/// (definition jumps nowhere; the hover lists the candidates)
pub(crate) fn one_name_refused(w: &World, file: FileId, name: &str) -> bool {
    name_bindings(w, file, name).refused()
}

/// the head of a use path speaks for the doc's OWN package — the
/// intra-package import. Two sources, the qualified lane's own
/// family mirrored: the stamped own lane
/// ([`crate::resolve::mods::position_head`]
/// — the doc's own `mod` edge, an ancestor's edge, or the own-pkg
/// name naming the root; answers the START mod path), and the
/// unstamped document's shape (its own `mod NAME;` edge off the
/// DeclMap — the child's path beside the document). `None` = a
/// foreign head: the caller rides the cross-package lane.
fn own_use_head(w: &World, file: FileId, head: &str) -> Option<String> {
    let doc = w.file(file);
    if let Some(p) = crate::resolve::mods::position_head(w, file, head) {
        return Some(p);
    }
    doc.mods()
        .iter()
        .any(|m| m.name() == head)
        .then(|| rut_lir::check::child_path(crate::resolve::mods::mod_path_of(doc), head))
}

/// the module an intra-package use path lands on: the start module
/// (the head's answer) walked through the path's interior segments
/// — any declared edge walks
/// ([`crate::resolve::mods::position_walk`], the compiler's
/// own-use walk), the LEAF visibility-gates at the caller
fn own_module(w: &World, file: FileId, start: &str, interior: &[String]) -> Option<FileId> {
    let pkg = w.file(file).stamp.module.as_deref();
    let path = crate::resolve::mods::position_walk(w, pkg, file, start, interior)?;
    module_at(w, file, pkg, &path)
}

/// A `pub use` row's full path in `module` — the world's rows (the
/// use's declared vis, its spelled path, and the imported name).
/// `None` = the module has no re-export row for `name`.
fn reexport_row(
    w: &World,
    pkg: Option<&str>,
    prefer: FileId,
    module: &str,
    name: &str,
) -> Option<Vec<String>> {
    let f = module_at(w, prefer, pkg, module)?;
    let u = w
        .file(f)
        .uses()
        .into_iter()
        .find(|u| u.vis() != Vis::Self_ && u.name() == name)?;
    let mut path = u.path();
    path.push(u.name().to_string());
    Some(path)
}

/// The DECLARING module when `module` re-exports `name` — law 5's one
/// walk (`rut_lir::check::reexport_edge`), the world's rows carried
/// in. `None` = no row (the name is the module's own business) or the
/// edge cycles / leaves the package (cross-package edges are the
/// surface's business, never a path chase).
pub(crate) fn reexport_declaring(
    w: &World,
    pkg: Option<&str>,
    prefer: FileId,
    module: &str,
    name: &str,
) -> Option<String> {
    reexport_row(w, pkg, prefer, module, name)?;
    let tree = crate::resolve::mods::mod_tree(w, pkg, prefer);
    match rut_lir::check::reexport_edge(
        &tree,
        pkg.unwrap_or(""),
        |m, n| reexport_row(w, pkg, prefer, m, n),
        module,
        name,
    ) {
        rut_lir::check::ReexportEdge::Declaring(d) if d != module => Some(d),
        _ => None,
    }
}

/// Is a `pub use` row declared in `row_file` visible to `consumer`? A
/// row in the consumer's OWN package may be package-private (law 2's
/// edge — the leaf's visibility is gated separately); a row in another
/// package must be a full `pub use` (law 3 is the across-package law,
/// and the wire carries no vis).
fn row_crosses(w: &World, consumer: FileId, row_file: &File, vis: Vis) -> bool {
    let same_pkg = match (
        row_file.stamp.module.as_deref(),
        w.file(consumer).stamp.module.as_deref(),
    ) {
        (Some(a), Some(b)) => a == b,
        (None, None) => true,
        _ => false,
    };
    if same_pkg {
        vis != Vis::Self_
    } else {
        vis == Vis::Pub
    }
}

/// The file a `pub use` row of one of `in_scope` targets for `name` —
/// the surface edge the compiler binds (law 3). The row's path resolves
/// through the world: an own-package head walks the intra-package law,
/// a foreign head is the dependency's `(package, module)` pair, and the
/// walk is transitive with a seen-set. `[]` = no re-export row, or a
/// cycle. This is a SURFACE-row lookup, not a declaration-path chase:
/// the row's spelled path names the target's exporting spelling.
pub(crate) fn reexport_target_files(
    w: &World,
    prefer: FileId,
    name: &str,
    in_scope: &[FileId],
) -> Vec<FileId> {
    fn resolve(
        w: &World,
        prefer: FileId,
        file: FileId,
        name: &str,
        seen: &mut Vec<(FileId, String)>,
    ) -> Option<FileId> {
        if seen.iter().any(|(f, n)| *f == file && n == name) {
            return None;
        }
        seen.push((file, name.to_string()));
        let f = w.file(file);
        // the row's target is a declaration here — the walk's end
        if f.ty(name).is_some()
            || f.lets().into_iter().any(|l| l.name() == name)
            || f.fns()
                .into_iter()
                .any(|fv| fv.name() == name && fv.owner().is_none())
        {
            return Some(file);
        }
        let u = f
            .uses()
            .into_iter()
            .find(|u| u.name() == name && row_crosses(w, prefer, f, u.vis()))?;
        let path = u.path();
        let (head, interior) = path.split_first()?;
        let pkg = f.stamp.module.as_deref();
        let target = match crate::resolve::mods::position_head(w, file, head) {
            Some(start) => {
                let walked = crate::resolve::mods::position_walk(w, pkg, file, &start, interior)
                    .unwrap_or(start);
                module_at(w, prefer, pkg, &walked)
            }
            None => {
                let module = interior.join("/");
                crate::resolve::mods::find_mod(w, Some(head), &module)
            }
        }?;
        resolve(w, prefer, target, name, seen)
    }
    let mut out: Vec<FileId> = Vec::new();
    for &start in in_scope {
        let mut seen = Vec::new();
        if let Some(t) = resolve(w, prefer, start, name, &mut seen) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// The file a name's re-export edge lands on: `cur` when the name is a
/// declaration there, the DECLARING module's file when it is one of
/// `cur`'s re-export rows (the chain walked to its end).
fn declaring_file(w: &World, pkg: Option<&str>, prefer: FileId, cur: FileId, name: &str) -> FileId {
    let module = crate::resolve::mods::mod_path_of(w.file(cur)).to_string();
    reexport_declaring(w, pkg, prefer, &module, name)
        .and_then(|d| module_at(w, prefer, pkg, &d))
        .unwrap_or(cur)
}

/// one EXPLICIT use leaf's resolve: an own-package head lands on
/// the walked module and binds the name's DECLARATION — its own decl
/// there, or the one its `pub use` re-export row names (law 5's walk:
/// the row carries its target, so the chain resolves to the declaring
/// module; a cycle binds nothing, and a row that names a DEPENDENCY is
/// law 3's surface edge — [`reexport_target_files`] follows it to the
/// declaring package's decl) — visible
/// from the importing file ([`rut_lir::check::vis_crosses`] —
/// `import is no visibility bypass`; the compiler's own-use binding
/// reads the same walk and then the TARGET module's decl tables:
/// `fn_in`/`let_in`/`resolve_qualified_leaf_ty`). A module name is no
/// import (no module rows here). A foreign head rides the
/// cross-package lane.
fn leaf_scope<'a>(
    w: &'a World,
    file: FileId,
    u: &UseView<'a>,
    name: &str,
    out: &mut FileScope<'a>,
) {
    let path = u.path();
    let (head, interior) = (&path[0], &path[1..]);
    let Some(start) = own_use_head(w, file, head) else {
        use_bound(w, file, u.pkg(), interior, name, out);
        return;
    };
    let Some(cur) = own_module(w, file, &start, interior) else {
        return;
    };
    let pkg = w.file(file).stamp.module.as_deref();
    let cur = declaring_file(w, pkg, file, cur, name);
    let from = w.file(file).stamp.mod_path.as_deref().unwrap_or("");
    let def_mod = crate::resolve::mods::mod_path_of(w.file(cur));
    let mut rows = decl_rows_visible(w.file(cur), name, |vis| {
        rut_lir::check::vis_crosses(from, def_mod, vis)
    });
    // law 3's cross-package hand-off: the walked module's row names a
    // DEPENDENCY (the compiler hands the binding to that surface) — the
    // target's declaration is the answer, `pub` like any crossing leaf
    if rows.is_empty() {
        for t in reexport_target_files(w, file, name, &[cur]) {
            rows.extend(decl_rows_visible(w.file(t), name, |vis| vis == Vis::Pub));
        }
    }
    out.extend(rows);
}

/// one CROSS-package use edge's resolve — the (pkg, interior)
/// scope's `pub` decls of `name` (the compiler's leaf gate: "only
/// `pub` names cross packages" — a `pub(pkg)` row, a private, and
/// the module's own private imports never bind). The interior walk
/// rides the use tier's own machinery
/// ([`crate::resolve::mods::use_walk`]); the walk's
/// own carrying law (the use tier's): a mod-carrying package
/// scopes the flat spelling to its root, a flat package's whole
/// world answers. The body-over-decl preference rides inside
/// ([`prefer_bodies`] — a pkg indexing both its `.d.rut`
/// surface and its body answers from the body); the queried file
/// is excluded (layer 1 already spoke for it — by identity).
fn use_bound<'a>(
    w: &'a World,
    file: FileId,
    pkg: &str,
    interior: &[String],
    name: &str,
    out: &mut FileScope<'a>,
) {
    let walked = if interior.is_empty() {
        Some(String::new())
    } else {
        crate::resolve::mods::use_walk(w, pkg, file, interior)
    };
    // the walk's own carrying law (the use tier's): a mod-carrying
    // package scopes the flat spelling to its root; a flat
    // package's whole world answers
    let carrying = w.chain(file).any(|id| {
        crate::resolve::pkg::matches_pkg(w.file(id), pkg) && w.file(id).stamp.mod_path.is_some()
    });
    let in_scope: Vec<FileId> = match walked {
        Some(p) if carrying => w
            .chain(file)
            .filter(|id| {
                crate::resolve::pkg::matches_pkg(w.file(*id), pkg)
                    && crate::resolve::mods::mod_path_of(w.file(*id)) == p
            })
            .collect(),
        _ => w
            .chain(file)
            .filter(|id| crate::resolve::pkg::matches_pkg(w.file(*id), pkg))
            .collect(),
    };
    let mut tys: Vec<(&File, TyView<'a>)> = Vec::new();
    let mut lets: Vec<(&File, LetView<'a>)> = Vec::new();
    let mut fns: Vec<(&File, FnView<'a>)> = Vec::new();
    // a module let crosses NO package surface — the compiler's own law
    // (`program.rs`: "a module let does not cross package surfaces; it
    // binds only inside its own package"), and the editor never answers
    // what the compiler refuses. The let sweep therefore keeps only
    // files of the asking file's own package (the `Option<String>`
    // stamp comparison: an unowned asking file keeps only unowned
    // rows). Pub TYPES and FNS still cross — their push is unchanged.
    let asking = w.file(file).stamp.module.as_deref();
    for &id in &in_scope {
        if id == file {
            continue;
        }
        let f = w.file(id);
        // only `pub` names cross packages
        if let Some(t) = f.ty(name).filter(|t| t.vis() == Vis::Pub) {
            tys.push((f, t));
        }
        for l in f.lets() {
            if l.name() == name && l.vis() == Vis::Pub && f.stamp.module.as_deref() == asking {
                lets.push((f, l));
            }
        }
        // the use leaves bind free fns — methods are reached
        // through a receiver, never named bare
        for fv in f.fns() {
            if fv.name() == name && fv.owner().is_none() && fv.vis() == Vis::Pub {
                fns.push((f, fv));
            }
        }
    }
    // no declaration in scope: the name may be a `pub use` re-export row
    // of the dep — the surface edge the compiler binds, its target
    // resolved through the world (law 3). Only the kinds the surface
    // carries: a module `let` is not a surface const, so it stays silent.
    if tys.is_empty() && lets.is_empty() && fns.is_empty() {
        for t in reexport_target_files(w, file, name, &in_scope) {
            let f = w.file(t);
            if let Some(ty) = f.ty(name).filter(|t| t.vis() == Vis::Pub) {
                tys.push((f, ty));
            }
            for fv in f.fns() {
                if fv.name() == name && fv.owner().is_none() && fv.vis() == Vis::Pub {
                    fns.push((f, fv));
                }
            }
        }
    }
    out.types
        .extend(prefer_bodies(&tys).into_iter().map(|(_, t)| t));
    out.lets
        .extend(prefer_bodies(&lets).into_iter().map(|(_, l)| l));
    out.fns
        .extend(prefer_bodies(&fns).into_iter().map(|(_, f)| f));
}

/// layer 3 — the world's AMBIENT rows (`prelude builtin`: the
/// names the checker binds under its wildcard site, in every
/// file, no `use` needed). The queried file is excluded (its own
/// decls spoke at layer 1 — by identity); the
/// mounted carrier wins over the embedded prelude — the
/// mounted-over-embed preference, the fallback shape the old flat
/// `type_decl` carried. Module lets have no ambient linkage — no
/// lane.
fn ambient_layers<'a>(w: &'a World, file: FileId, name: &str, out: &mut FileScope<'a>) {
    let mut tys: Vec<(&File, TyView<'a>)> = Vec::new();
    let mut fns: Vec<(&File, FnView<'a>)> = Vec::new();
    for id in w.chain(file) {
        if id == file {
            continue;
        }
        let f = w.file(id);
        for t in f.types() {
            if t.name() == name && t.ambient() {
                tys.push((f, t));
            }
        }
        for fv in f.fns() {
            // impl methods are type-scoped, never file scope (the
            // one-name law) — the ambient fn lane answers free fns
            if fv.name() == name && fv.owner().is_none() && fv.ambient() {
                fns.push((f, fv));
            }
        }
    }
    out.types
        .extend(super::prefer_mounted(&tys).into_iter().map(|(_, t)| t));
    out.fns
        .extend(super::prefer_mounted(&fns).into_iter().map(|(_, f)| f));
}

/// the computed-head lane — a type name REACHED THROUGH A VALUE
/// (the receiver's inferred `RequestBuilder`, a binding's type
/// head) is not a spelling the file binds, so the file-scope law
/// does not gate it: the candidate set is the asking file's own
/// declaration plus the pkgs the document's use edges NAME (the
/// import machinery's own shape), the world's flat
/// mounted-over-embed sweep the fallback. The lane answers a type
/// only when that set is ONE declaration — every hit the same node
/// after the body/mount staging; distinct candidates are the
/// one-name law's refusal and the lane is silent (never a pick
/// between the bindings). A spelled type token never rides this
/// door — the spelling lanes answer through [`file_scope`].
pub(crate) fn type_via_imports<'a>(w: &'a World, file: FileId, name: &str) -> Option<TyView<'a>> {
    let doc = w.file(file);
    // the heads resolve to PACKAGES: a foreign head is its pkg, an
    // own-package head (a `mod` edge, the own-pkg name) is the
    // doc's own package — the type behind the value lives where
    // the value's edge points
    let mut pkgs: Vec<&str> = Vec::new();
    for head in doc.uses().into_iter().map(|u| u.pkg()) {
        let p = match own_use_head(w, file, head) {
            Some(_) => doc.stamp.module.as_deref().unwrap_or(head),
            None => head,
        };
        if !pkgs.contains(&p) {
            pkgs.push(p);
        }
    }
    let mut hits: Vec<(&File, TyView<'a>)> = Vec::new();
    // the asking file's own declaration is a candidate like any
    // other: [`type_decl`]'s own lane answers a name the file binds
    // once, so it rides here exactly where the one-name law refused
    // the name there — and then it decides with the rest
    if let Some(t) = doc.ty(name) {
        hits.push((doc, t));
    }
    for pkg in &pkgs {
        for id in w
            .chain(file)
            .filter(|id| crate::resolve::pkg::matches_pkg(w.file(*id), pkg))
        {
            if id == file {
                continue;
            }
            let f = w.file(id);
            if let Some(t) = f.ty(name) {
                hits.push((f, t));
            }
        }
    }
    let hits = prefer_bodies(&hits);
    // one DECLARATION answers — every hit the same node; a distinct
    // candidate is silence, never the old first-take. The staging
    // above is the preference (mounted over embeds, bodies over
    // surfaces), NOT a pick: what survives it must still be one
    // declaration
    let mut one = hits.iter().map(|&(_, t)| t);
    if let Some(first) = one.next() {
        let same = one.all(|t| std::ptr::eq(t.file, first.file) && t.h == first.h);
        return same.then_some(first);
    }
    // the flat fallback — the old `type_decl` law, kept for the
    // computed heads ONLY: a type the doc neither declares nor
    // imports still navigates to its carrier (the
    // ctor-inference shape — `Logger("h")` typing a binding off a
    // mounted dep without a use edge); the mounted carrier wins
    // over the embedded prelude. Spelled names never ride this —
    // the spelling lanes stop at [`file_scope`]. The staging is the
    // preference, never a pick: two same-named mounted types are two
    // declarations, and the lane answers NOTHING (the one-name law).
    let mut flat: Vec<(&File, TyView<'a>)> = Vec::new();
    for id in w.chain(file) {
        if id == file {
            continue;
        }
        let f = w.file(id);
        if let Some(t) = f.ty(name) {
            flat.push((f, t));
        }
    }
    let staged = prefer_mounted(&flat);
    let mut one = staged.iter().map(|&(_, t)| t);
    let first = one.next()?;
    let same = one.all(|t| std::ptr::eq(t.file, first.file) && t.h == first.h);
    same.then_some(first)
}

// ---- the qualified path — the module namespace lane ----

/// the qualified-path lane — `head::…::tail` where the HEAD opens
/// a module the queried file binds. A qualified path is the other
/// explicit form of file-scope access (cross-module access is
/// qualified paths or use bindings), so the file scope answers
/// BOTH forms. Four head sources, each an established walk —
/// never a new rule:
/// - the OWN lane ([`crate::resolve::mods::position_head`]): the doc's own
///   `mod` edge, an ancestor's edge, or the own-pkg name — any
///   declared edge walks (the intra-package rule);
/// - the doc's own `mod NAME;` edge read off the DeclMap — the
///   unstamped-doc shape ([`module_at`]'s unnamed-owner
///   law);
/// - a use-bound module leaf (`use calc::{ math }`) — the
///   cross-package `pub` walk ([`crate::resolve::mods::use_walk`]; a
///   walkable child is a namespace segment, never a bound name);
/// - a pkg-rooted DEP name (`calc::math::…`, the dep merely
///   declared) — the manifest's own naming (the dep stamps:
///   [`Stamp::dep`](crate::world::Stamp::dep) carries `module: Some(name)`), bound
///   AMBIENTLY by the compiler — "the manifest's own naming, never
///   a use's" — but ONLY when the dep carries a module tree ("a
///   flat dep — no module path can root inside") AND the path
///   roots INSIDE that tree: the dep root's own members are not
///   nameable by position (`calc::ctop()` refuses — "unknown name
///   `calc::ctop`", tree dep and flat dep alike; the use is their
///   only door).
///
/// The TYPE ladder walks the own family ONLY
/// ([`crate::resolve::mods::position_head`] — `resolve_qualified_ty`'s law: a
/// used package's name in a type position refuses, "uses are the
/// only cross-package door"); the value ladder adds the two
/// cross-package heads. The interior segments walk the mounted
/// mod tree edge by edge (any declared edge intra-package,
/// `pub`-gated cross-package); the TAIL resolves against the
/// target module's DECLARATIONS ([`decl_rows_visible`] — the
/// compiler's qualified leaf is decls-plus-rows: an own-package
/// `pub use` row NAMES its target, so the tail follows law 5's walk
/// to the DECLARING module exactly as the compiler's `*_in` lookups
/// do; a cross-package path stays decls-only — the re-export edge is
/// a USE row's (the surface owns it), never a path chased across the
/// boundary — with the leaves tier-gated by visibility:
/// [`rut_lir::check::vis_crosses`] against the reading file
/// intra-package, `pub`-only cross-package, ambient rows passing. A
/// head the file does not bind answers NONE — never the flat sweep.
pub(crate) fn qualified_scope<'a>(
    w: &'a World,
    file: FileId,
    segs: &[String],
    is_ty: bool,
) -> Option<FileScope<'a>> {
    let (head, rest) = segs.split_first()?;
    let (tail, middles) = rest.split_last()?;
    let (mut cur, src) = module_head(w, file, head, is_ty)?;
    // the dep-root source roots inside the dep's module tree — a
    // leaf directly behind the dep name is the dep root's own
    // member, which no position names
    if src == HeadSrc::DepRoot && middles.is_empty() {
        return None;
    }
    (cur, _) = module_walk(w, file, cur, src, middles)?;
    let cross = src != HeadSrc::Own;
    // an own-package leaf that names a re-export row resolves to the
    // DECLARING module (law 5's one walk, the world's rows carried
    // in); a cross-package leaf stays decls-only — the surface owns
    // that edge, never a path chase
    if !cross {
        let pkg = w.file(file).stamp.module.as_deref();
        cur = declaring_file(w, pkg, file, cur, tail);
    }
    let mut scope = decl_rows_visible(w.file(cur), tail, |_| true);
    // the leaf's fns are the module's own FREE fns — impl methods
    // are type-scoped, never module rows (`fn_in`'s shape)
    scope.fns.retain(|f| f.owner().is_none());
    // the leaves tier-gate by visibility (the compiler's leaf
    // rule — the phase-3 machinery's own law for exactly this
    // position); the READER is the queried file — its true mod
    // path is its own stamp
    let from = crate::resolve::mods::mod_path_of(w.file(file));
    let def_mod = crate::resolve::mods::mod_path_of(w.file(cur));
    let gate = |vis: Vis, ambient: bool| {
        ambient
            || if cross {
                vis == Vis::Pub
            } else {
                rut_lir::check::vis_crosses(from, def_mod, vis)
            }
    };
    scope.types.retain(|t| gate(t.vis(), t.ambient()));
    scope.fns.retain(|f| gate(f.vis(), f.ambient()));
    // module lets cross NO package boundary through a qualified
    // path (the compiler refuses `mathkit::advanced::SCALE` and
    // the use-bound `advanced::SCALE` in every lane probed) —
    // intra-package heads only
    if cross {
        scope.lets.clear();
    } else {
        scope.lets.retain(|l| gate(l.vis(), false));
    }
    Some(scope)
}

/// the module file a qualified path's PREFIX lands on — the
/// head-plus-interior walk where every spelled segment names a
/// module (the mid-segment hover's target; each ladder renders
/// its own heads — the flag threads through)
pub(crate) fn qualified_module(
    w: &World,
    file: FileId,
    segs: &[String],
    is_ty: bool,
) -> Option<FileId> {
    let (head, rest) = segs.split_first()?;
    let (cur, src) = module_head(w, file, head, is_ty)?;
    let (landed, _) = module_walk(w, file, cur, src, rest)?;
    Some(landed)
}

/// the head's module file — the own family for both ladders, the
/// cross-package heads for the value ladder only; the source
/// marks the cross-package walks (everything beyond stays
/// `pub`-gated) and the dep root (its own members are no leaf)
fn module_head(w: &World, file: FileId, head: &str, is_ty: bool) -> Option<(FileId, HeadSrc)> {
    let doc = w.file(file);
    // 1. the own lane — the machinery's own walk (own/ancestor
    //    edges + the own-pkg head; the doc's stamp keys it)
    if let Some(p) = crate::resolve::mods::position_head(w, file, head) {
        if let Some(f) = module_at(w, file, doc.stamp.module.as_deref(), &p) {
            return Some((f, HeadSrc::Own));
        }
    }
    // 2. the doc's own mod edge off the DeclMap — the
    //    unstamped-doc shape (no doc context: the edge's own
    //    record walks, the unnamed-owner law finds the child)
    if doc.mods().iter().any(|m| m.name() == head) {
        let child = rut_lir::check::child_path(crate::resolve::mods::mod_path_of(doc), head);
        if let Some(f) = module_at(w, file, doc.stamp.module.as_deref(), &child) {
            return Some((f, HeadSrc::Own));
        }
    }
    // the TYPE ladder stops at the own family — a used package's
    // name in a type position refuses ("uses are the only
    // cross-package door" — `resolve_qualified_ty`'s law)
    if is_ty {
        return None;
    }
    // 3. a use-bound module leaf — the cross-package pub walk
    for u in doc.uses() {
        if u.name() != head {
            continue;
        }
        let pkg = u.pkg();
        // the walk starts at the pkg root — the edge's interior
        // (after the pkg head) rides ahead of the leaf
        let mut walk: Vec<String> = u.path().into_iter().skip(1).collect();
        walk.push(head.to_string());
        if let Some(p) = crate::resolve::mods::use_walk(w, pkg, file, &walk) {
            if let Some(f) = module_at(w, file, Some(pkg), &p) {
                return Some((f, HeadSrc::UseBound));
            }
        }
    }
    // 4. a pkg-rooted DEP name — the manifest's own naming: the
    //    dep's ROOT module, stamped by the dep lanes
    //    ([`Stamp::dep`](crate::world::Stamp::dep) carries `module: Some(name)`),
    //    found by that stamp alone (a dep is manifest-named; no
    //    URI derivation lands it). The mounted carrier wins over
    //    the embedded prelude (the mounted-over-embed law), and
    //    the dep must carry a module tree ("a flat dep — no
    //    module path can root inside").
    let named = |f: &File| {
        f.stamp.module.as_deref() == Some(head) && crate::resolve::mods::mod_path_of(f).is_empty()
    };
    let tree_carrying = |f: &File| {
        !f.mods().is_empty()
            || w.chain(file).any(|x| {
                w.file(x).stamp.module.as_deref() == Some(head)
                    && !crate::resolve::mods::mod_path_of(w.file(x)).is_empty()
            })
    };
    let root = w
        .chain(file)
        .find(|id| named(w.file(*id)) && w.file(*id).stamp.src_path.is_none())
        .or_else(|| w.chain(file).find(|id| named(w.file(*id))));
    root.map(|f| (f, HeadSrc::DepRoot))
        .filter(|(f, _)| tree_carrying(w.file(*f)))
}

/// the interior walk — each segment a declared edge of the module
/// the walk stands at, resolved to its mounted file (the
/// cross-package sources keep the use walk's gate: only `pub`
/// crosses)
fn module_walk(
    w: &World,
    file: FileId,
    mut cur: FileId,
    src: HeadSrc,
    segs: &[String],
) -> Option<(FileId, HeadSrc)> {
    for seg in segs {
        if src != HeadSrc::Own {
            // the use walk's gate continues: only `pub` crosses
            let pub_edge = w
                .file(cur)
                .mods()
                .into_iter()
                .find(|m| m.name() == seg)
                .is_some_and(|m| m.vis() == Vis::Pub);
            if !pub_edge {
                return None;
            }
        }
        cur = module_hop(w, file, cur, seg)?;
    }
    Some((cur, src))
}

/// one hop — the current module's declared edge `seg` (any vis —
/// the intra rule; the cross gate runs at the call site) and the
/// mounted file at the child's mod path
fn module_hop(w: &World, file: FileId, cur: FileId, seg: &str) -> Option<FileId> {
    if !w.file(cur).mods().iter().any(|m| m.name() == seg) {
        return None;
    }
    let child = rut_lir::check::child_path(crate::resolve::mods::mod_path_of(w.file(cur)), seg);
    module_at(w, file, w.file(cur).stamp.module.as_deref(), &child)
}

/// the mounted file at `(pkg, path)` — the
/// [`crate::resolve::mods::matches_mod`] law (the STAMP: a package's root is its
/// stamped root, never a file by a path coincidence), with two
/// doors ahead of it: the queried file IS the module when its
/// stamp says so (its own entry carries the freshest decls — the
/// sync lane keeps it current), and the unnamed-owner
/// fallback (a pkg-less owner answers any world file stamped at
/// the path — the unstamped document's lenience)
fn module_at(w: &World, file: FileId, pkg: Option<&str>, path: &str) -> Option<FileId> {
    let doc = w.file(file);
    if doc.stamp.module.as_deref() == pkg && doc.stamp.mod_path.as_deref() == Some(path) {
        return Some(file);
    }
    crate::resolve::mods::find_mod(w, pkg, path).or_else(|| {
        (pkg.is_none())
            .then(|| {
                w.files()
                    .find(|id| crate::resolve::mods::mod_path_of(w.file(*id)) == path)
            })
            .flatten()
    })
}

/// where a qualified path's head resolved from — the walk's gates key
/// on it: the own family (any declared edge, the visibility dial),
/// the cross-package heads (the `pub` walk), the dep root (whose own
/// members no position names)
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeadSrc {
    Own,
    UseBound,
    DepRoot,
}

/// does the use-bound scope bind a row of `name` declared in `f`?
/// (The bare and type tiers' imported gate — one answer with
/// [`file_scope`].)
pub(crate) fn bound_has(bound: &FileScope<'_>, f: &File, name: &str) -> bool {
    bound
        .types
        .iter()
        .any(|t| std::ptr::eq(t.file, f) && t.name() == name)
        || bound
            .fns
            .iter()
            .any(|fv| std::ptr::eq(fv.file, f) && fv.name() == name)
        || bound
            .lets
            .iter()
            .any(|l| std::ptr::eq(l.file, f) && l.name() == name)
}

/// the OWN layer of one file's scope — the shared core both scope
/// doors call: `f`'s DeclMap decls named `name`, source order.
/// [`file_scope`] layers the use-bound + ambient rows on top
/// (the BARE name's scope); the qualified leaf answers from this
/// alone plus law 5's re-export walk (`decl_rows_visible` on the
/// DECLARING module when the target re-exports the name).
pub(crate) fn own_scope_of<'a>(f: &'a File, name: &str) -> FileScope<'a> {
    let mut out = FileScope::default();
    if let Some(t) = f.ty(name) {
        out.types.push(t);
    }
    out.lets
        .extend(f.lets().into_iter().filter(|l| l.name() == name));
    out.fns
        .extend(f.fns().into_iter().filter(|fv| fv.name() == name));
    out
}

/// one file's OWN module-scope binders of `name` — every one, the
/// one-name registry's kinds: each type decl, each module let, each
/// FREE fn (impl methods are type-scoped and never meet the law), each
/// `mod` edge
fn own_binders<'a>(f: &'a File, name: &str) -> OwnBinders<'a> {
    OwnBinders {
        types: f.types().into_iter().filter(|t| t.name() == name).collect(),
        lets: f.lets().into_iter().filter(|l| l.name() == name).collect(),
        fns: f
            .fns()
            .into_iter()
            .filter(|fv| fv.name() == name && fv.owner().is_none())
            .collect(),
        mods: f.mods().into_iter().filter(|m| m.name() == name).collect(),
    }
}

/// one file's DECLARATION rows of `name` — its own type / module let
/// / FREE fn (impl methods are type-scoped, never module rows), each
/// passing `gate`: the leaf shape the compiler's use binding and its
/// qualified leaf both read (`fn_in`/`let_in`/the data tables of the
/// TARGET module)
fn decl_rows_visible<'a>(f: &'a File, name: &str, gate: impl Fn(Vis) -> bool) -> FileScope<'a> {
    let mut out = FileScope::default();
    if let Some(t) = f.ty(name).filter(|t| gate(t.vis())) {
        out.types.push(t);
    }
    out.lets.extend(
        f.lets()
            .into_iter()
            .filter(|l| l.name() == name && gate(l.vis())),
    );
    out.fns.extend(
        f.fns()
            .into_iter()
            .filter(|fv| fv.name() == name && fv.owner().is_none() && gate(fv.vis())),
    );
    out
}

/// ONE name's bindings in the queried file — what the own decls and
/// the use edges say, the analysis every scope door reads
/// ([`file_scope`], [`ambiguous_scope`],
/// [`use_bound_scope`]).
struct NameBindings<'a> {
    /// the doc's own module-scope decls of the name — the one-name
    /// registry's kinds (a type, a module let, a free fn, a `mod`
    /// edge; impl methods are type-scoped and never meet the law)
    own: OwnBinders<'a>,
    /// the explicit use leaves naming it, source order
    leaves: Vec<UseView<'a>>,
}

impl<'a> NameBindings<'a> {
    /// THE ONE-NAME LAW's refusal — every shape the compiler refuses:
    /// the registry's three: two own decls ("duplicate name `mk` — the
    /// fn already declares it in this file"), two use leaves (an
    /// identical leaf twice included — "`use m::{ mk }` already
    /// binds it in this file"), and an own decl beside a leaf.
    fn refused(&self) -> bool {
        !matches!(
            (self.own.len(), self.leaves.len()),
            (0, 0) | (0, 1) | (1, 0)
        )
    }

    /// layer 2 — the ONE use binding's rows: the explicit leaf's
    /// resolve (an own decl shadows it — layer 1 already spoke)
    fn layer2(&self, w: &'a World, file: FileId, name: &str, out: &mut FileScope<'a>) {
        if !self.own.is_empty() {
            return;
        }
        if let Some(u) = self.leaves.first() {
            leaf_scope(w, file, u, name, out);
        }
    }
}

/// the doc's own module-scope decls of one name — the one-name
/// registry's kinds
struct OwnBinders<'a> {
    types: Vec<TyView<'a>>,
    lets: Vec<LetView<'a>>,
    fns: Vec<FnView<'a>>,
    mods: Vec<ModView<'a>>,
}

impl OwnBinders<'_> {
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        self.types.len() + self.lets.len() + self.fns.len() + self.mods.len()
    }
}

/// one candidate of a twice-bound name — the row and the use edge
/// that bound it (`None` = the doc's own decl, or a leaf that binds
/// nothing: the spelling is the candidate)
pub(crate) struct Candidate<'a> {
    pub row: CandRow<'a>,
    /// the `use` leaf the row rode in on; `None` for the doc's own
    /// decl, or a leaf that binds nothing
    pub via: Option<UseView<'a>>,
}

impl<'a> Candidate<'a> {
    /// a row the doc declares itself (or a method beside a same-named
    /// free fn — the legal own ambiguity the fn lane lists)
    pub(crate) fn own(row: CandRow<'a>) -> Candidate<'a> {
        Candidate { row, via: None }
    }
}

/// a candidate's decl row — a type, a module let, a fn, a `mod` edge,
/// or a use leaf that binds nothing of its own
pub(crate) enum CandRow<'a> {
    Ty(TyView<'a>),
    Let(LetView<'a>),
    Fn(FnView<'a>),
    Mod(ModView<'a>),
    Leaf(UseView<'a>),
}

impl<'a> FileScope<'a> {
    /// the scope's rows as candidates of ONE use binding, each beside
    /// the edge that bound it
    fn into_candidates(self, u: UseView<'a>) -> Vec<Candidate<'a>> {
        let mut out: Vec<Candidate<'a>> = Vec::new();
        for t in self.types {
            out.push(Candidate {
                row: CandRow::Ty(t),
                via: Some(u),
            });
        }
        for l in self.lets {
            out.push(Candidate {
                row: CandRow::Let(l),
                via: Some(u),
            });
        }
        for f in self.fns {
            out.push(Candidate {
                row: CandRow::Fn(f),
                via: Some(u),
            });
        }
        out
    }
}

// ---- the staged preferences — mounted-over-embed, body-over-decl ----

/// the ambient layer's mounted-over-embed pick: the MOUNTED carriers
/// (real origins) when any exist, else the embedded prelude — the
/// zero-fetch fallback (the old flat `type_decl`'s precedence, kept
/// as the ambient fallback shape). THE one home of the preference;
/// [`prefer_bodies`] stages it first (a mounted carrier outranks the
/// prelude, above the body law), and the inference pass's ctor lane
/// rides it over its own window.
pub(crate) fn prefer_mounted<'a, T: Copy>(pairs: &[(&'a File, T)]) -> Vec<(&'a File, T)> {
    let mounted: Vec<(&File, T)> = pairs
        .iter()
        .copied()
        .filter(|(f, _)| f.stamp.src_path.is_none())
        .collect();
    if mounted.is_empty() {
        pairs.to_vec()
    } else {
        mounted
    }
}

/// body-over-decl — THE single preference: a package that indexes
/// BOTH its `.d.rut` surface (`mode == Decl`) and its `mod.rut` body
/// answers from the body; the surface hits ride only when they are all
/// there is. One precedence above the body law: a MOUNTED dep outranks
/// the ambient prelude — the prelude is the zero-fetch fast path for
/// the ONE embedded pkg, a fallback, never the answer when the real
/// thing is mounted (its `src_path` marks it) — staged first through
/// [`prefer_mounted`]. The generic payload keeps one law for every
/// shape the layers collect (spans, views).
pub(crate) fn prefer_bodies<'a, T: Copy>(hits: &[(&'a File, T)]) -> Vec<(&'a File, T)> {
    let pool = prefer_mounted(hits);
    let bodies: Vec<(&File, T)> = pool
        .iter()
        .copied()
        .filter(|(f, _)| f.stamp.mode != Mode::Decl)
        .collect();
    if bodies.is_empty() {
        pool
    } else {
        bodies
    }
}
