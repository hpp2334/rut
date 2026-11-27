//! The surface enumeration — completion's USE, BARE, and MEMBER
//! tiers keyed on the walk's NODE, and the one walked-surface walk
//! that answers them all. Enumeration + GATES live here; prefix
//! filtering and the wire mapping stay at the call site
//! (presentation). The doors read `Doc::bindings()` internally where
//! a receiver needs it; no caller runs a pass. The file-scope
//! resolver the bare tier prices against is [`crate::resolve::scope`].
//!
//! Assignment law: a SURFACE or enumeration question lives here
//! (`use_exports`, `walked_surface`, `bare`,
//! `members`/`members_for` and their row types); a scope or binding
//! question lives beside it in [`crate::resolve::scope`].

use std::collections::HashSet;

use rut_ast::{AnyHandle, UseNode, Vis};

use super::member::used_ifaces;
use super::scope::{bound_has, type_decl, type_via_imports, use_bound_scope};
use crate::resolve::cursor::Recv;
use crate::world::ty::TyForm;
use crate::world::views::{FnView, LetView, MemberView, ModView, TyView, UseView};
use crate::world::{FileId, World};

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
    let ty = super::recv_ty(w, file, pos, recv).or_else(|| match recv {
        Recv::Expr(e) => super::expr_ty(w, file, e).map(|t| t.headed()),
        _ => None,
    });
    let Some(ty) = ty else {
        return Some(Vec::new());
    };
    let ty_name = super::qualified_ty_name(w, file, &ty).unwrap_or_else(|| ty.head());
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
    // the walk's refusal decides: a head naming the asking file's
    // own package in a flat package is refused by the compiler, so
    // the tier offers nothing — never the package's surface
    if crate::resolve::walk::head_refused(w, file, pkg) {
        return Some(UseExports::Exports {
            pkg: pkg.clone(),
            path: String::new(),
            rows: Vec::new(),
        });
    }
    let (path, rows) = walked_surface(w, file, pkg, &before[1..]);
    Some(UseExports::Exports {
        pkg: pkg.clone(),
        path,
        rows,
    })
}

/// the walked (pkg, interior mod path)'s export rows beside the
/// resolved path — the ONE surface enumeration: the use tier
/// offers it. The pub gates and the walk live inside; the path is
/// `""` when the walk answers nothing (the callers render an empty
/// tier, never a wrong one).
pub(crate) fn walked_surface<'a>(
    w: &'a World,
    file: FileId,
    pkg: &str,
    interior: &[String],
) -> (String, Vec<ExportRow<'a>>) {
    // the import is the gate-opener, so the `pub builtin` gate
    // does not bind here (this IS the use that names them)
    let scoped: Vec<FileId> = w
        .chain(file)
        .filter(|id| crate::resolve::pkg::matches_pkg(w.file(*id), pkg))
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
    // the asking file's own package — the let gate's stamp comparison
    // ([`crate::resolve::pkg::same_pkg`], the gate's one home)
    let asking = w.file(file).stamp.module.as_deref();
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
            // the compiler's own law: a module let crosses no package
            // surface (`program.rs`) — a foreign let is no use-tier
            // row (the host surface's folded consts are the one
            // exemption the compiler spells; see `pkg::same_pkg`)
            if l.vis() == Vis::Pub && crate::resolve::pkg::same_pkg(asking, f) {
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
    }
    (path, rows)
}

/// completion's BARE tier — the visible decls under the compiler's
/// own bare-name law: the queried file's decls, the world's AMBIENT
/// builtin rows (`prelude builtin` — bound in
/// every unit, no `use` needed), and the rows the document's use
/// edges BIND (the file scope's own resolve, [`use_bound_scope`]
/// — an own-package edge and a dep's leaf answer the same
/// way hover does; never a package-name match). Every other
/// foreign name rides ONLY the auto-import tier: a plain offer the
/// compile would reject is a trap. Free fns and types only;
/// methods are reached through a receiver, not named bare.
pub(crate) fn bare<'a>(w: &'a World, file: FileId) -> Bare<'a> {
    // the names the document's use edges bind — through the ONE
    // home ([`use_bound_scope`]): a foreign row is offered
    // exactly when the file scope answers its name with it (an
    // own-package edge, a dep's leaf — the resolve, never
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
