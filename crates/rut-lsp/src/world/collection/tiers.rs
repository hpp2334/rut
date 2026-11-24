//! Completion's tier doors — the folded contract. Enumeration + GATES
//! live here, keyed on the walk's NODE; prefix filtering and the wire
//! mapping stay at the call site (presentation). The doors read
//! `Doc::bindings()` internally where a receiver needs it; no caller
//! runs a pass. THE FILE SCOPE — the one resolver every bare-name
//! lane answers through — lives here too.

use std::collections::HashSet;

use crate::world::types::TyForm;
use rut_ast::{AnyHandle, UseNode, Vis};

use super::{used_ifaces, FnView, GlobUse, LetView, MemberView, ModView, TyView, UseView};
use crate::resolve::cursor::Recv;
use crate::world::{File, FileId, World};

pub(crate) use super::prefer_bodies;
use super::prefer_mounted;

impl<'a> ExportRow<'a> {
    /// the row's exported name — `None` for a `mod` edge (a walkable
    /// child is a namespace segment, never a bound name): the glob
    /// expansion's name listing reads this
    pub(crate) fn name(&self) -> Option<&'a str> {
        match self {
            ExportRow::Mod(..) => None,
            ExportRow::Ty(t) => Some(t.name()),
            ExportRow::Fn(f) => Some(f.name()),
            ExportRow::Let(l) => Some(l.name()),
            ExportRow::ReExport(u) => Some(u.name()),
        }
    }

    /// the file the row declares in — the glob expansion keys its
    /// (name, pkg) rows on it (a chained re-export's rows answer from
    /// the deeper pkg, where the name actually resolves)
    pub(crate) fn file(&self) -> &'a File {
        match self {
            ExportRow::Mod(m, _) => m.file,
            ExportRow::Ty(t) => t.file,
            ExportRow::Fn(f) => f.file,
            ExportRow::Let(l) => l.file,
            ExportRow::ReExport(u) => u.file,
        }
    }
}

/// the seen-set key for a walked head — `pkg::interior` (one spelling,
/// both insert sites)
fn seen_key(pkg: &str, interior: &[String]) -> String {
    format!("{}::{}", pkg, interior.join("::"))
}

/// one member-tier row — the view the door found it from
#[derive(Clone, Copy)]
pub(crate) enum MemberRow<'a> {
    Field(MemberView<'a>),
    Method(MemberView<'a>),
    ImplFn(FnView<'a>),
}

/// one use-tier export row — `Mod` carries the child's mod path (the
/// detail renders the mounted display from it)
#[derive(Clone)]
pub(crate) enum ExportRow<'a> {
    Mod(ModView<'a>, String),
    Ty(TyView<'a>),
    Fn(FnView<'a>),
    Let(LetView<'a>),
    ReExport(UseView<'a>),
}

/// the use tier's answer: the package's module names (no walk yet),
/// or the walked module's exports (the pkg + resolved path ride along
/// for the rendering)
pub(crate) enum UseExports<'a> {
    Modules(Vec<(String, String)>),
    Exports {
        pkg: String,
        path: String,
        rows: Vec<ExportRow<'a>>,
    },
}

/// the bare tier's answer — the visible decls under the bare-name law
/// (keywords are the call site's vocabulary, not a collection fact)
pub(crate) struct Bare<'a> {
    pub types: Vec<TyView<'a>>,
    pub fns: Vec<FnView<'a>>,
}

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

/// completion's MEMBER tier — keyed on the walk's NODE: the
/// member/receiver derive inside (a method call, a field access, or a
/// `::` path's tail), the receiver resolves inside (the resolver
/// family — the bindings read internally, never a caller-run pass),
/// and the gates live here (the interface lane with the use-both rule;
/// the enum-member lane; the inherent impl fns by owner; the
/// member-vis dial against the queried file — the site context).
/// `None` = not a member position; an unresolved receiver is an EMPTY
/// tier, never a wrong one. The typing edge's absent member (`recv.`
/// mid-typing) enumerates the FULL list.
pub(crate) fn members<'a>(
    w: &'a World,
    file: FileId,
    pos: u32,
    node: AnyHandle,
) -> Option<Vec<MemberRow<'a>>> {
    let (_, recv) = crate::resolve::cursor::member_pos(w.file(file).doc.ast(), node, pos)?;
    members_for(w, file, pos, recv)
}

/// the member tier keyed on an already-derived receiver (the type
/// slot's `.`/`::` spelling: the head is the slot's own segments,
/// node-less data on the ty path). The member node's OWN name never
/// restricts the list — the typed prefix (the call site's filter) is
/// the only narrowing. The queried file is the site context the
/// member-vis dial reads.
pub(crate) fn members_for<'a>(
    w: &'a World,
    file: FileId,
    pos: u32,
    recv: Recv,
) -> Option<Vec<MemberRow<'a>>> {
    let ty_name = super::recv_ty_name(w, file, pos, recv).or_else(|| match recv {
        Recv::Expr(e) => super::expr_ty(w, file, e).map(|t| crate::world::types::head_of_ty(&t)),
        _ => None,
    });
    let Some(ty_name) = ty_name else {
        return Some(Vec::new());
    };
    // an interface receiver: the declared signatures are the whole
    // member list — satisfaction is structural, so no impl lookup
    // rides along. The use-both gate: a foreign interface's
    // members complete only when this document names the interface
    // in a `use`; an interface declared here is in scope natively
    if let Some(iface) = super::iface_decl(w, file, &ty_name) {
        let mut rows: Vec<MemberRow<'a>> = Vec::new();
        if std::ptr::eq(iface.file, w.file(file))
            || used_ifaces(w.file(file)).contains(iface.name())
        {
            for m in iface.methods() {
                rows.push(MemberRow::Method(m));
            }
        }
        return Some(rows);
    }
    // the receiver's type head is a COMPUTED string (the inferred
    // `RequestBuilder` behind `let b = builder()`), not a spelling
    // the file binds — the file scope answers first, the doc's
    // import pkgs the fallback ([`type_via_imports`])
    let ty = type_decl(w, file, &ty_name).or_else(|| type_via_imports(w, file, &ty_name))?;
    // the member-vis gate (the compiler's law mirrored): CLASS
    // fields + own-surface methods gate, impl fns by owner gate;
    // struct + host-struct fields carry no dial, enum variants are
    // the enum lane's rows, and the builtin/primitive surfaces are
    // compiler-lowered (core's plain-`fn` rows keep completing)
    let class = ty.form() == TyForm::Class;
    let mut rows: Vec<MemberRow<'a>> = Vec::new();
    for m in ty.fields() {
        if class && !super::member_dial_allows(w, file, m.file, m.vis()) {
            continue;
        }
        rows.push(MemberRow::Field(m));
    }
    for m in ty.methods() {
        if class && !super::member_dial_allows(w, file, m.file, m.vis()) {
            continue;
        }
        rows.push(MemberRow::Method(m));
    }
    // inherent impl-block methods — where methods live since type
    // bodies went fields-only; the owner string IS the target
    // type's name, and the sweep stays inside the resolved type's
    // own package (a same-named type elsewhere owns its own).
    // Gated rows (the compiler dials them like a class's own
    // members)
    let ty_pkg = ty.file.stamp.module.as_deref();
    for id in w.chain(file) {
        let i = w.file(id);
        if i.stamp.module.as_deref() != ty_pkg {
            continue;
        }
        for f in i.fns() {
            if f.owner().as_deref() != Some(ty_name.as_str())
                || !super::member_dial_allows(w, file, f.file, f.vis())
            {
                continue;
            }
            rows.push(MemberRow::ImplFn(f));
        }
    }
    Some(rows)
}

/// completion's USE tier — keyed on the walk's NODE (the use edge
/// the cursor sits in): the walk derives from the node's own
/// recorded spans (the segments BEFORE the cursor; a sync that ran
/// past the incomplete leaf can leave later tokens in the path —
/// the statement's recovery — and they are not the walk). The pub
/// gates and the walk live inside. `None` = the cursor is not in a
/// use statement.
///
/// `direct_deps` is the open document's own dep set — the module
/// NAMES arm's gate (`use <cursor>`): mounting is TRANSITIVE by
/// design (a walked dep's own deps mount too — goto-def/hover into
/// its files need them), but nameability is DECLARATION-scoped —
/// the compiler's closure check refuses `use` of a
/// mounted-but-undeclared package (the D5 diagnostic) — so a
/// transitive-only module never rides the plain offer (a plain
/// offer the compile would reject is a trap). The ambient prelude
/// rides unconditionally on its origin (`core` — D5's ambient
/// arm). The set threads LIVE — the state-derived
/// [`Stamp`](crate::world::Stamp) facts, never the state itself.
pub(crate) fn use_exports<'a>(
    w: &'a World,
    file: FileId,
    node: AnyHandle,
    pos: u32,
    direct_deps: &HashSet<String>,
) -> Option<UseExports<'a>> {
    let uh = node.downcast::<UseNode>()?;
    let ast = w.file(file).doc.ast();
    let d = uh.get(ast);
    // the walk: the path BEFORE the segment the cursor rides (the
    // riding segment is the typed prefix, never a walk segment —
    // `use po⏐` is the module-NAME tier); nothing ridden → every
    // segment that closed at-or-before the cursor (`use
    // gadgets::layout::⏐` walks both)
    let before: Vec<String> = match d
        .path
        .iter()
        .position(|n| n.span.lo <= pos && pos <= n.span.hi)
    {
        Some(k) => d.path[..k]
            .iter()
            .map(|n| ast.name(n.ident).to_string())
            .collect(),
        None => d
            .path
            .iter()
            .filter(|n| n.span.hi <= pos)
            .map(|n| ast.name(n.ident).to_string())
            .collect(),
    };
    let Some(pkg) = before.first() else {
        // `use <cursor>`: the world's module names — every NAMED
        // file the document may NAME (the auto-import tier's
        // `direct` gate, mirrored byte-for-byte in spirit: origin
        // `core` passes, every other name must be a row of the
        // document's own manifest; the unnamed/stem-derived
        // workspace files stay out by the filter_map below, as
        // before)
        return Some(UseExports::Modules(
            w.chain(file)
                .filter_map(|id| {
                    let f = w.file(id);
                    (f.stamp.origin == "core"
                        || f.stamp
                            .module
                            .as_deref()
                            .is_some_and(|m| direct_deps.contains(m)))
                    .then(|| f.stamp.module.clone().map(|m| (m, f.stamp.origin.clone())))
                })
                .flatten()
                .collect(),
        ));
    };
    let (path, rows) = walked_surface(w, file, pkg, &before[1..]);
    Some(UseExports::Exports {
        pkg: pkg.clone(),
        path,
        rows,
    })
}

/// the walked (pkg, interior mod path)'s export rows beside the
/// resolved path — the ONE surface enumeration: the use tier
/// offers it and the glob expansion rides it (`import_map`'s name
/// listing; the `pub use pkg::*` re-export arm). The pub gates and
/// the walk live inside; the path is `""` when the walk answers
/// nothing (the callers render an empty tier, never a wrong one).
pub(crate) fn walked_surface<'a>(
    w: &'a World,
    file: FileId,
    pkg: &str,
    interior: &[String],
) -> (String, Vec<ExportRow<'a>>) {
    let mut seen = HashSet::new();
    // the walked head itself is seen before the walk: a self-glob
    // or a cycle that returns to the start re-lists nothing (the
    // initial pkg's rows stay single)
    seen.insert(seen_key(pkg, interior));
    walked_surface_in(w, file, pkg, interior, &mut seen)
}

/// the walked surface's exported names beside the pkg each name
/// declares in — the DECLARING file's module stamp when one rides
/// (a chained re-export's rows answer from the deeper pkg, where
/// the name actually resolves), else the spelled head (the flat
/// stem laws answer for it). `import_map`'s glob expansion reads
/// this projection of [`walked_surface`]; a `mod` child is
/// a walkable segment, never a bound name.
pub(crate) fn surface_names<'a>(
    w: &'a World,
    file: FileId,
    pkg: &'a str,
    interior: &[String],
) -> Vec<(&'a str, &'a str)> {
    let mut out = Vec::new();
    for r in walked_surface(w, file, pkg, interior).1 {
        let Some(name) = r.name() else {
            continue;
        };
        let owner = r.file().stamp.module.as_deref().unwrap_or(pkg);
        out.push((name, owner));
    }
    out
}

/// the seen-set threaded form — the recursion's spine. A `pub use
/// pkg::*;` edge met mid-walk expands the walked head's surface
/// through the world behind the seen-set on walked heads: chained
/// glob re-exports (`a` pub-globs `b`, the consumer globs `a`)
/// terminate, so a hand-built host world can never loop the
/// editor.
fn walked_surface_in<'a>(
    w: &'a World,
    file: FileId,
    pkg: &str,
    interior: &[String],
    seen: &mut HashSet<String>,
) -> (String, Vec<ExportRow<'a>>) {
    // the import is the gate-opener, so the `pub builtin` gate
    // does not bind here (this IS the use that names them)
    let scoped: Vec<FileId> = w
        .chain(file)
        .filter(|id| crate::world::pkg::matches_pkg(w.file(*id), pkg))
        .collect();
    if scoped.is_empty() {
        return (String::new(), Vec::new());
    }
    // resolve the walked module: the flat spelling answers the
    // root — for a mod-carrying package that is the ROOT file only
    // (members live under their module path), for a flat package
    // the whole world; deeper segments walk the pub edges (the
    // trailing PARTIAL ident is not a walk segment — `use
    // pkg::layout::Co⏐` walks `layout` only)
    let carrying = scoped.iter().any(|id| w.file(*id).stamp.mod_path.is_some());
    let walked = if interior.is_empty() {
        Some(String::new())
    } else {
        crate::resolve::mods::use_walk(w, pkg, file, interior)
    };
    let Some(path) = walked else {
        return (String::new(), Vec::new());
    };
    // the queried file speaks only through its STAMP (it
    // IS the walked module when its stamp says so) — never because
    // a path segment names the package; an UNSTAMPED document
    // joins an unnamed flat package only (the legacy shape, where
    // no lane named anything)
    let head = w.file(file);
    let named = scoped.iter().any(|id| w.file(*id).stamp.module.is_some());
    let member = |id: FileId| {
        id != file
            || match head.stamp.module.as_deref() {
                Some(p) => p == pkg && head.stamp.mod_path.as_deref().is_none_or(|mp| mp == path),
                None => !named,
            }
    };
    let scope: Vec<FileId> = if carrying {
        scoped
            .into_iter()
            .filter(|id| member(*id) && crate::resolve::mods::mod_path_of(w.file(*id)) == path)
            .collect()
    } else {
        scoped.into_iter().filter(|id| member(*id)).collect()
    };
    let mut rows: Vec<ExportRow<'a>> = Vec::new();
    for id in &scope {
        let f = w.file(*id);
        // the walkable children — pub edges only (cross-package law)
        for m in f.mods() {
            if m.vis() != Vis::Pub {
                continue;
            }
            let child = rut_lir::check::child_path(&path, m.name());
            rows.push(ExportRow::Mod(m, child));
        }
        for t in f.types() {
            if t.is_pub() {
                rows.push(ExportRow::Ty(t));
            }
        }
        for fv in f.fns() {
            // the walked path binds through the compiler's
            // qualified-use gate: only `pub` names cross — on
            // walked AND flat spellings alike (one gate, no
            // ungated legacy shape)
            if fv.owner().is_none() && fv.vis() == Vis::Pub {
                rows.push(ExportRow::Fn(fv));
            }
        }
        for l in f.lets() {
            if l.vis() == Vis::Pub {
                rows.push(ExportRow::Let(l));
            }
        }
        // a `pub use` re-export: the imported name joined the
        // module's surface — the tier's gate mirrors the fns/lets
        // above (a walked path binds only `pub` edges across
        // packages, flat spellings included)
        for u in f.uses() {
            if u.vis() == Vis::Pub {
                rows.push(ExportRow::ReExport(u));
            }
        }
        // the glob re-export arm: `pub use json::*;` — the walked
        // head's whole pub surface joins this module's own, NAMES
        // only (a `mod` child is a walkable segment, never a
        // re-exported name; the star itself is never offered).
        // The expansion rides this same walk; the seen-set on
        // walked heads keeps chained re-exports from looping
        for g in f.glob_uses() {
            if g.vis != Vis::Pub {
                continue;
            }
            let key = seen_key(g.pkg, &g.interior);
            if !seen.insert(key) {
                continue;
            }
            for r in walked_surface_in(w, file, g.pkg, &g.interior, seen).1 {
                if matches!(r, ExportRow::Mod(..)) {
                    continue;
                }
                rows.push(r);
            }
        }
    }
    (path, rows)
}

/// completion's BARE tier — the visible decls under the compiler's
/// own bare-name law: the queried file's decls, the world's AMBIENT
/// builtin rows (`prelude builtin` — bound in
/// every unit, no `use` needed), and the rows the document's use
/// edges BIND (the file scope's own resolve, [`use_bound_scope`]
/// — an own-package edge, a dep's leaf, a glob all answer the same
/// way hover does; never a package-name match). Every other
/// foreign name rides ONLY the auto-import tier: a plain offer the
/// compile would reject is a trap. Free fns and types only;
/// methods are reached through a receiver, not named bare.
pub(crate) fn bare<'a>(w: &'a World, file: FileId) -> Bare<'a> {
    // the names the document's use edges bind — through the ONE
    // home ([`use_bound_scope`]): a foreign row is offered
    // exactly when the file scope answers its name with it (an
    // own-package edge, a dep's leaf, a glob — the resolve, never
    // a package-name match)
    let bound = use_bound_scope(w, file);
    let mut out = Bare {
        types: Vec::new(),
        fns: Vec::new(),
    };
    for id in w.chain(file) {
        let i = w.file(id);
        let own = id == file;
        let offered = |name: &str, ambient: bool| own || ambient || bound_has(&bound, i, name);
        for t in i.types() {
            if offered(t.name(), t.ambient()) {
                out.types.push(t);
            }
        }
        for f in i.fns() {
            if f.owner().is_none() && offered(f.name(), f.ambient()) {
                out.fns.push(f);
            }
        }
    }
    out
}

// ---- the file scope — the one resolver the flat doors re-spell ----

/// THE file-scope resolver — what the QUERIED file binds by `name`,
/// walked in the compiler's layer order: the file's own decls, then
/// the name its use edges bind (leaves and glob leaves), then the
/// ambient prelude rows (`prelude builtin` — the checker's
/// wildcard-site split). A name the file never binds answers an
/// EMPTY scope — never a foreign decl: sibling modules may share a
/// name legally (the one-name law's Act 1), and the old flat sweep
/// answered for all of them. This is the one home — hover,
/// definition, and references answer bare names through it (the
/// flat doors re-spell it on top), the same walk the completion
/// tiers enumerate (`bare`, `use_exports`).
///
/// THE ONE-NAME LAW rides the front door: a name the file binds
/// TWICE by DISTINCT bindings ([`NameBindings::refused`]) is a file
/// the compiler refuses — the single-answer scope is EMPTY here
/// (never a pick between the bindings); the hover lists the
/// candidates through [`ambiguous_scope`]. The shadowing the
/// law admits resolves to the winner: an own decl over every use
/// binding, an explicit leaf over any glob.
pub(crate) fn file_scope<'a>(w: &'a World, file: FileId, name: &str) -> FileScope<'a> {
    let mut out = FileScope::default();
    let bindings = name_bindings(w, file, name);
    if bindings.refused(w, file) {
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

/// THE mid-edit candidates — the file binds `name` twice by
/// DISTINCT bindings ([`NameBindings::refused`]: two decls of its
/// own, two use leaves, a decl beside a leaf, two globs of distinct
/// heads): the hover LISTS every binding, each with its kind and
/// provenance — the own decls first, then each use leaf's rows with
/// the `use` they rode, then each glob's with its spelling. `None`
/// = the name binds once or never: the single-answer lanes answer
/// it. (Definition stays silent on an ambiguous name — never a
/// pick.)
pub(crate) fn ambiguous_scope<'a>(
    w: &'a World,
    file: FileId,
    name: &str,
) -> Option<Vec<Candidate<'a>>> {
    let bindings = name_bindings(w, file, name);
    if !bindings.refused(w, file) {
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
        out.extend(rows.into_candidates(Via::Leaf(*u)));
    }
    for (g, rows) in &bindings.globs {
        out.extend(rows.clone().into_candidates(Via::Glob(g.clone())));
    }
    Some(out)
}

/// ONE name's bindings in the queried file — the analysis every
/// scope door reads ([`NameBindings`]): the own module-scope decls,
/// the explicit use leaves, and the globs (each glob's surface
/// resolved once per call)
fn name_bindings<'a>(w: &'a World, file: FileId, name: &str) -> NameBindings<'a> {
    let leaves: Vec<UseView<'a>> = w
        .file(file)
        .uses()
        .into_iter()
        .filter(|u| u.name() == name)
        .collect();
    let glob_scopes = glob_scopes(w, file);
    bindings_with(w, file, name, leaves, &glob_scopes)
}

/// the queried file's glob leaves beside their resolved surfaces —
/// each glob's surface resolves ONCE (a chain sweep), whatever many
/// names read it
fn glob_scopes<'a>(w: &'a World, file: FileId) -> Vec<(GlobUse<'a>, FileScope<'a>)> {
    w.file(file)
        .glob_uses()
        .into_iter()
        .map(|g| {
            let rows = glob_scope(w, file, &g);
            (g, rows)
        })
        .collect()
}

/// [`name_bindings`] over glob scopes a caller already
/// resolved (the all-names enumeration resolves each glob once)
fn bindings_with<'a>(
    w: &'a World,
    file: FileId,
    name: &str,
    leaves: Vec<UseView<'a>>,
    glob_scopes: &[(GlobUse<'a>, FileScope<'a>)],
) -> NameBindings<'a> {
    let own = own_binders(w.file(file), name);
    // the globs bind only under no explicit leaf (the compiler's
    // precedence — glob < explicit leaf < own declaration, the
    // shadowing silent; an own decl's glob is kept beside it for
    // the law's verdict below) — and ONE per distinct HEAD: the
    // glob ledger keys the head ("the same head re-globbed is
    // idempotent" — an identical spelling twice, or the same head
    // with a different interior, binds once, the FIRST binding
    // standing); only a DIFFERENT head's glob conflicts
    let mut globs: Vec<(GlobUse<'a>, FileScope<'a>)> = Vec::new();
    if leaves.is_empty() {
        for (g, rows) in glob_scopes {
            if globs.iter().any(|(kept, _)| kept.pkg == g.pkg) {
                continue;
            }
            if let Some(rows) = scope_named(rows, name) {
                globs.push((g.clone(), rows));
            }
        }
    }
    NameBindings { own, leaves, globs }
}

/// EVERY name the head's use edges bind — the file scope's layer 2
/// over all names at once (the same analysis
/// [`file_scope`] runs per name; each glob's surface
/// resolves once). Completion's enumeration gates ride it: a
/// foreign row is offered exactly when the file scope would answer
/// its name with it (a twice-bound name offers nothing).
pub(crate) fn use_bound_scope<'a>(w: &'a World, file: FileId) -> FileScope<'a> {
    let doc = w.file(file);
    let glob_scopes = glob_scopes(w, file);
    let mut names: Vec<&str> = doc
        .uses()
        .iter()
        .map(|u| u.name())
        .chain(glob_scopes.iter().flat_map(|(_, rows)| {
            rows.types
                .iter()
                .map(|t| t.name())
                .chain(rows.fns.iter().map(|f| f.name()))
                .chain(rows.lets.iter().map(|l| l.name()))
        }))
        .collect();
    names.sort_unstable();
    names.dedup();
    let mut out = FileScope::default();
    for name in names {
        let leaves: Vec<UseView<'a>> = doc
            .uses()
            .into_iter()
            .filter(|u| u.name() == name)
            .collect();
        let bindings = bindings_with(w, file, name, leaves, &glob_scopes);
        if !bindings.refused(w, file) {
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
    name_bindings(w, file, name).refused(w, file)
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

/// one EXPLICIT use leaf's resolve: an own-package head lands on
/// the walked module and binds its OWN declaration of the name —
/// a fn, a type, or a module let, visible from the importing file
/// ([`rut_lir::check::vis_crosses`] — `import is no visibility bypass`; the
/// compiler's own-use binding reads the TARGET module's decl
/// tables: `fn_in`/`let_in`/`resolve_qualified_leaf_ty`). A `pub
/// use` row of that module is NOT followed (the compiler's binding
/// refuses a re-export import today — its use-LINE gate admits the
/// row, the binding answers "`G` is declared in module `m/grid`";
/// mirrored AS THE COMPILER IS), and a module name is no import
/// (no module rows here). A foreign head rides the cross-package
/// lane.
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
    let from = w.file(file).stamp.mod_path.as_deref().unwrap_or("");
    let def_mod = crate::resolve::mods::mod_path_of(w.file(cur));
    out.extend(decl_rows_visible(w.file(cur), name, |vis| {
        rut_lir::check::vis_crosses(from, def_mod, vis)
    }));
}

/// one GLOB leaf's resolve — EVERY row it binds. The intra-package
/// lane (the compiler's own glob): the walked module's fn / type /
/// let decls visible from the importing file under the crossing
/// predicate (`pub`, `pub(pkg)`, `pub(super)` where it reaches, a
/// private for a reader inside the module's own subtree) and
/// nothing else — never its `pub use` rows (the same binding gap
/// the explicit leaf rides), never the names its own private
/// `use`s or globs bound, never a child module. The cross-package
/// lane: the walked surface's pub rows, each answered from the
/// package it declares in (a chained re-export's rows answer from
/// the deeper pkg, where the name actually resolves — the export
/// machinery's own law, [`surface_names`]) and the interior
/// when that owner IS the glob's own pkg.
fn glob_scope<'a>(w: &'a World, file: FileId, g: &GlobUse<'a>) -> FileScope<'a> {
    let mut out = FileScope::default();
    if let Some(start) = own_use_head(w, file, g.pkg) {
        if let Some(cur) = own_module(w, file, &start, &g.interior) {
            let from = w.file(file).stamp.mod_path.as_deref().unwrap_or("");
            let def_mod = crate::resolve::mods::mod_path_of(w.file(cur));
            let allows = |vis: Vis| rut_lir::check::vis_crosses(from, def_mod, vis);
            out.types
                .extend(w.file(cur).types().into_iter().filter(|t| allows(t.vis())));
            out.fns.extend(
                w.file(cur)
                    .fns()
                    .into_iter()
                    .filter(|f| f.owner().is_none() && allows(f.vis())),
            );
            out.lets
                .extend(w.file(cur).lets().into_iter().filter(|l| allows(l.vis())));
        }
        return out;
    }
    for (name, owner) in surface_names(w, file, g.pkg, &g.interior) {
        let interior = if owner == g.pkg {
            g.interior.clone()
        } else {
            Vec::new()
        };
        use_bound(w, file, owner, &interior, name, &mut out);
    }
    out
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
        crate::world::pkg::matches_pkg(w.file(id), pkg) && w.file(id).stamp.mod_path.is_some()
    });
    let in_scope: Vec<FileId> = match walked {
        Some(p) if carrying => w
            .chain(file)
            .filter(|id| {
                crate::world::pkg::matches_pkg(w.file(*id), pkg)
                    && crate::resolve::mods::mod_path_of(w.file(*id)) == p
            })
            .collect(),
        _ => w
            .chain(file)
            .filter(|id| crate::world::pkg::matches_pkg(w.file(*id), pkg))
            .collect(),
    };
    let mut tys: Vec<(&File, TyView<'a>)> = Vec::new();
    let mut lets: Vec<(&File, LetView<'a>)> = Vec::new();
    let mut fns: Vec<(&File, FnView<'a>)> = Vec::new();
    for id in in_scope {
        if id == file {
            continue;
        }
        let f = w.file(id);
        // only `pub` names cross packages
        if let Some(t) = f.ty(name).filter(|t| t.vis() == Vis::Pub) {
            tys.push((f, t));
        }
        for l in f.lets() {
            if l.name() == name && l.vis() == Vis::Pub {
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
/// does not gate it: the pkgs the document's use edges NAME are
/// the first candidate set (the import machinery's own shape),
/// the world's flat mounted-over-embed sweep the fallback. A
/// spelled type token never rides this door — the spelling lanes
/// answer through [`file_scope`].
pub(crate) fn type_via_imports<'a>(w: &'a World, file: FileId, name: &str) -> Option<TyView<'a>> {
    let doc = w.file(file);
    // the heads resolve to PACKAGES: a foreign head is its pkg, an
    // own-package head (a `mod` edge, the own-pkg name) is the
    // doc's own package — the type behind the value lives where
    // the value's edge points
    let mut pkgs: Vec<&str> = Vec::new();
    for head in doc
        .uses()
        .into_iter()
        .map(|u| u.pkg())
        .chain(doc.glob_uses().into_iter().map(|g| g.pkg))
    {
        let p = match own_use_head(w, file, head) {
            Some(_) => doc.stamp.module.as_deref().unwrap_or(head),
            None => head,
        };
        if !pkgs.contains(&p) {
            pkgs.push(p);
        }
    }
    let mut hits: Vec<(&File, TyView<'a>)> = Vec::new();
    for pkg in &pkgs {
        for id in w
            .chain(file)
            .filter(|id| crate::world::pkg::matches_pkg(w.file(*id), pkg))
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
    if let [(_, t), ..] = hits.as_slice() {
        return Some(*t);
    }
    // the flat fallback — the old `type_decl` law, kept for the
    // computed heads ONLY: a type the doc neither declares,
    // imports, nor globs still navigates to its carrier (the
    // ctor-inference shape — `Logger("h")` typing a binding off a
    // mounted dep without a use edge); the mounted carrier wins
    // over the embedded prelude. Spelled names never ride this —
    // the spelling lanes stop at [`file_scope`].
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
    let picked = prefer_mounted(&flat);
    picked.into_iter().next().map(|(_, t)| t)
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
///   cross-package `pub` walk ([`crate::resolve::mods::use_walk`]; glob leaves
///   bind no modules — a walkable child is a namespace segment,
///   never a bound name);
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
/// target module's OWN DECLARATIONS ([`own_scope_of`] — the
/// compiler's qualified leaf is decls-only:
/// `fn_in`/`data_in`/`enum_in`/`let_in` read the module's own
/// tables, so a use row never joins, `pub use` included) with the
/// leaves tier-gated by visibility: [`rut_lir::check::vis_crosses`] against the
/// reading file intra-package, `pub`-only cross-package, ambient
/// rows passing. A head the file does not bind answers NONE —
/// never the flat sweep.
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

/// the scope's rows of ONE name (`None` when it binds nothing of it)
fn scope_named<'a>(scope: &FileScope<'a>, name: &str) -> Option<FileScope<'a>> {
    let out = FileScope {
        types: scope
            .types
            .iter()
            .copied()
            .filter(|t| t.name() == name)
            .collect(),
        lets: scope
            .lets
            .iter()
            .copied()
            .filter(|l| l.name() == name)
            .collect(),
        fns: scope
            .fns
            .iter()
            .copied()
            .filter(|f| f.name() == name)
            .collect(),
    };
    (!out.is_empty()).then_some(out)
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
/// alone — the compiler's qualified leaf is DECLS-ONLY
/// (`fn_in`/`data_in`/`enum_in`/`let_in` read the module's own
/// tables, `lir/call` + `check/resolve`; a use row never joins,
/// `pub use` included — mirrored AS THE COMPILER IS TODAY; the book's
/// "imported names join the module's public surface as if declared
/// there" disagrees on qualified access — a compiler/doc follow-up,
/// not this lane's)
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
    /// the globs binding it — ONE per distinct head (the compiler's
    /// glob ledger: the same head re-globbed is idempotent), source
    /// order, each beside its rows of the name
    globs: Vec<(GlobUse<'a>, FileScope<'a>)>,
}

impl<'a> NameBindings<'a> {
    /// THE ONE-NAME LAW's refusal — every shape the compiler refuses:
    /// - the registry: two own decls ("duplicate name `mk` — the fn
    ///   already declares it in this file"), two use leaves (an
    ///   identical leaf twice included — "`use m::{ mk }` already
    ///   binds it in this file"), an own decl beside a leaf;
    /// - glob–glob: two globs of DISTINCT heads both bind it
    ///   ("`Counter` arrives from both `m::*` and `app::*`");
    /// - an own free fn beside an own-package glob's fn of the name,
    ///   an own type beside such a glob's type ("duplicate fn `mk` —
    ///   already declared in module `child`") —
    ///   [`NameBindings::own_glob_collision`].
    ///
    /// The shadowing the law admits is no refusal: an own decl over
    /// every other use binding, an explicit leaf over any glob; and the
    /// same head re-globbed is idempotent (never two bindings).
    fn refused(&self, w: &World, file: FileId) -> bool {
        match (self.own.len(), self.leaves.len()) {
            (0, 0) => self.globs.len() > 1,
            (0, 1) => false,
            (1, 0) => self.own_glob_collision(w, file),
            _ => true,
        }
    }

    /// the compiler's OWN-package glob lane refuses an own free fn
    /// beside a glob-bound fn of the name, and an own type beside a
    /// glob-bound type — "duplicate fn `mk` — already declared in
    /// module `child`", the span on the GLOB'S decl, though the
    /// glob-bound name is no decl of the module (`child::mk` stays
    /// "not declared in module `child`"). Mirrored AS THE COMPILER IS.
    /// Every other own-decl-beside-a-glob shape shadows silently, the
    /// documented precedence: a module let, a `mod` edge, the other
    /// kind (an own fn beside a glob's type or let), and any
    /// cross-package glob (the driver's own
    /// `own_decl_shadows_the_glob_silently` pins that one).
    fn own_glob_collision(&self, w: &World, file: FileId) -> bool {
        self.globs.iter().any(|(g, rows)| {
            own_use_head(w, file, g.pkg).is_some()
                && ((!self.own.fns.is_empty() && !rows.fns.is_empty())
                    || (!self.own.types.is_empty() && !rows.types.is_empty()))
        })
    }

    /// layer 2 — the ONE unshadowed use binding's rows: the explicit
    /// leaf's resolve, else the glob's (an own decl shadows both —
    /// layer 1 already spoke)
    fn layer2(&self, w: &'a World, file: FileId, name: &str, out: &mut FileScope<'a>) {
        if !self.own.is_empty() {
            return;
        }
        if let Some(u) = self.leaves.first() {
            leaf_scope(w, file, u, name, out);
        } else if let Some((_, rows)) = self.globs.first() {
            out.extend(rows.clone());
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
    pub via: Option<Via<'a>>,
}

impl<'a> Candidate<'a> {
    /// a row the doc declares itself (or a method beside a same-named
    /// free fn — the legal own ambiguity the fn lane lists)
    pub(crate) fn own(row: CandRow<'a>) -> Candidate<'a> {
        Candidate { row, via: None }
    }
}

/// the doc's use edge a candidate rode in on — its provenance beside
/// the row's own (which `use` leaf and its source, which glob)
#[derive(Clone)]
pub(crate) enum Via<'a> {
    /// an explicit leaf — the `use` statement it rode
    Leaf(UseView<'a>),
    /// a glob — its spelling
    Glob(GlobUse<'a>),
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
    fn into_candidates(self, via: Via<'a>) -> Vec<Candidate<'a>> {
        let mut out: Vec<Candidate<'a>> = Vec::new();
        for t in self.types {
            out.push(Candidate {
                row: CandRow::Ty(t),
                via: Some(via.clone()),
            });
        }
        for l in self.lets {
            out.push(Candidate {
                row: CandRow::Let(l),
                via: Some(via.clone()),
            });
        }
        for f in self.fns {
            out.push(Candidate {
                row: CandRow::Fn(f),
                via: Some(via.clone()),
            });
        }
        out
    }
}
