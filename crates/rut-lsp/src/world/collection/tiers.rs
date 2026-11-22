//! Completion's tier doors — the folded contract. Enumeration + GATES
//! live here, keyed on the walk's NODE; prefix filtering and the wire
//! mapping stay at the call site (presentation). The doors read
//! `Doc::bindings()` internally where a receiver needs it; no caller
//! runs a pass.

use std::collections::HashSet;

use rut_ast::{AnyHandle, UseNode, Vis};

use super::{
    used_ifaces, Chain, FnView, LetView, MemberView, ModView, MountedFile, RecvNode, TyView,
    UseView,
};
use crate::world::mods::DocMods;
use crate::world::types::TyForm;

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
    pub(crate) fn file(&self) -> &'a MountedFile {
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

impl<'a> Chain<'a> {
    /// completion's MEMBER tier — keyed on the walk's NODE: the
    /// member/receiver derive inside (a method call, a field access,
    /// or a `::` path's tail), the receiver resolves inside (the
    /// resolver family — the bindings read internally, never a
    /// caller-run pass), and the gates live here (the interface lane
    /// with the use-both rule; the enum-member lane; the inherent impl
    /// fns by owner; the member-vis dial against `doc` — the site
    /// context). `None` = not a member position; an unresolved
    /// receiver is an EMPTY tier, never a wrong one. The typing edge's
    /// absent member (`recv.` mid-typing) enumerates the FULL list.
    pub(crate) fn members(
        &self,
        pos: u32,
        node: AnyHandle,
        doc: &DocMods,
    ) -> Option<Vec<MemberRow<'a>>> {
        let (_, recv) = self.member_at(pos, node)?;
        self.members_for(pos, recv, doc)
    }

    /// the member tier keyed on an already-derived receiver (the type
    /// slot's `.`/`::` spelling: the head is the slot's own segments,
    /// node-less data on the ty path). The member node's OWN name never
    /// restricts the list — the typed prefix (the call site's filter)
    /// is the only narrowing. `doc` is the site context the member-vis
    /// dial reads (the established [`DocMods`] carrier — the chain
    /// head's own mount is unstamped).
    pub(crate) fn members_for(
        &self,
        pos: u32,
        recv: RecvNode,
        doc: &DocMods,
    ) -> Option<Vec<MemberRow<'a>>> {
        let ty_name = self.recv_ty_name(pos, recv).or_else(|| match recv {
            RecvNode::Expr(e) => self
                .expr_ty(self.head(), e)
                .map(|t| crate::world::types::head_of_ty(&t)),
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
        if let Some(iface) = self.iface_decl(&ty_name) {
            let mut rows: Vec<MemberRow<'a>> = Vec::new();
            if std::ptr::eq(iface.file, self.head())
                || used_ifaces(self.head()).contains(iface.name())
            {
                for m in iface.methods() {
                    rows.push(MemberRow::Method(m));
                }
            }
            return Some(rows);
        }
        let ty = self.type_decl(&ty_name)?;
        // the member-vis gate (the compiler's law mirrored): CLASS
        // fields + own-surface methods gate, impl fns by owner gate;
        // struct + host-struct fields carry no dial, enum variants are
        // the enum lane's rows, and the builtin/primitive surfaces are
        // compiler-lowered (core's plain-`fn` rows keep completing)
        let class = ty.form() == TyForm::Class;
        let mut rows: Vec<MemberRow<'a>> = Vec::new();
        for m in ty.fields() {
            if class && !self.member_dial_allows(doc, m.file, m.vis()) {
                continue;
            }
            rows.push(MemberRow::Field(m));
        }
        for m in ty.methods() {
            if class && !self.member_dial_allows(doc, m.file, m.vis()) {
                continue;
            }
            rows.push(MemberRow::Method(m));
        }
        // inherent impl-block methods — where methods live since type
        // bodies went fields-only; the owner string IS the target
        // type's name. Gated rows (the compiler dials them like a
        // class's own members)
        for i in self.iter() {
            for f in i.fns() {
                if f.owner().as_deref() != Some(ty_name.as_str())
                    || !self.member_dial_allows(doc, f.file, f.vis())
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
    /// arm). The set threads LIVE — the
    /// [`DocMods`](crate::world::mods::DocMods) precedent, a
    /// state-derived fact, never the state itself.
    pub(crate) fn use_exports(
        &self,
        node: AnyHandle,
        pos: u32,
        direct_deps: &HashSet<String>,
    ) -> Option<UseExports<'a>> {
        let uh = node.downcast::<UseNode>()?;
        let ast = self.head().doc.ast();
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
            // `use <cursor>`: the chain's module names — every NAMED
            // index the document may NAME (the auto-import tier's
            // `direct` gate, mirrored byte-for-byte in spirit: origin
            // `core` passes, every other name must be a row of the
            // document's own manifest; the unnamed/stem-derived
            // workspace files stay out by the filter_map below, as
            // before)
            return Some(UseExports::Modules(
                self.iter()
                    .filter(|f| {
                        f.mount.origin == "core"
                            || f.mount
                                .module
                                .as_deref()
                                .is_some_and(|m| direct_deps.contains(m))
                    })
                    .filter_map(|f| f.mount.module.clone().map(|m| (m, f.mount.origin.clone())))
                    .collect(),
            ));
        };
        let (path, rows) = self.walked_surface(pkg, &before[1..]);
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
    pub(crate) fn walked_surface(
        &self,
        pkg: &str,
        interior: &[String],
    ) -> (String, Vec<ExportRow<'a>>) {
        let mut seen = HashSet::new();
        // the walked head itself is seen before the walk: a self-glob
        // or a cycle that returns to the start re-lists nothing (the
        // initial pkg's rows stay single)
        seen.insert(seen_key(pkg, interior));
        self.walked_surface_in(pkg, interior, &mut seen)
    }

    /// the walked surface's exported names beside the pkg each name
    /// declares in — the DECLARING file's module stamp when one rides
    /// (a chained re-export's rows answer from the deeper pkg, where
    /// the name actually resolves), else the spelled head (the flat
    /// stem laws answer for it). `import_map`'s glob expansion reads
    /// this projection of [`Chain::walked_surface`]; a `mod` child is
    /// a walkable segment, never a bound name.
    pub(crate) fn surface_names(
        &self,
        pkg: &'a str,
        interior: &[String],
    ) -> Vec<(&'a str, &'a str)> {
        let mut out = Vec::new();
        for r in self.walked_surface(pkg, interior).1 {
            let Some(name) = r.name() else {
                continue;
            };
            let owner = r.file().mount.module.as_deref().unwrap_or(pkg);
            out.push((name, owner));
        }
        out
    }

    /// the seen-set threaded form — the recursion's spine. A `pub use
    /// pkg::*;` edge met mid-walk expands the walked head's surface
    /// through the chain behind the seen-set on walked heads: chained
    /// glob re-exports (`a` pub-globs `b`, the consumer globs `a`)
    /// terminate, so a hand-built host world can never loop the
    /// editor.
    fn walked_surface_in(
        &self,
        pkg: &str,
        interior: &[String],
        seen: &mut HashSet<String>,
    ) -> (String, Vec<ExportRow<'a>>) {
        // the import is the gate-opener, so the `pub builtin` gate
        // does not bind here (this IS the use that names them)
        let scoped: Vec<&MountedFile> = self
            .iter()
            .filter(|f| crate::world::pkg::matches_pkg(f, pkg))
            .collect();
        if scoped.is_empty() {
            return (String::new(), Vec::new());
        }
        // resolve the walked module: the flat spelling answers the
        // root — for a mod-carrying package that is the ROOT file only
        // (members live under their module path), for a flat package
        // today's whole chain; deeper segments walk the pub edges (the
        // trailing PARTIAL ident is not a walk segment — `use
        // pkg::layout::Co⏐` walks `layout` only)
        let carrying = scoped.iter().any(|f| f.mount.mod_path.is_some());
        let walked = if interior.is_empty() {
            Some(String::new())
        } else {
            crate::world::mods::walk_use_mods(self, pkg, interior)
        };
        let Some(path) = walked else {
            return (String::new(), Vec::new());
        };
        let scope: Vec<&MountedFile> = if carrying {
            scoped
                .into_iter()
                .filter(|f| crate::world::mods::mod_path_of(f) == path)
                .collect()
        } else {
            scoped
        };
        let mut rows: Vec<ExportRow<'a>> = Vec::new();
        for f in &scope {
            // the walkable children — pub edges only (cross-package law)
            for m in f.mods() {
                if m.vis() != Vis::Pub {
                    continue;
                }
                let child = crate::world::mods::child_path(&path, m.name());
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
                for r in self.walked_surface_in(g.pkg, &g.interior, seen).1 {
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
    /// own bare-name law: the open document's decls (the chain head),
    /// the chain's AMBIENT builtin rows (`prelude builtin` — bound in
    /// every unit, no `use` needed), and the names the document
    /// already `use`s (matched `(name, pkg)` against the head's uses —
    /// the same set shape the auto-import tier dedups against). Every
    /// other foreign name rides ONLY the auto-import tier: a plain
    /// offer the compile would reject is a trap. Free fns and types
    /// only; methods are reached through a receiver, not named bare.
    pub(crate) fn bare(&self) -> Bare<'a> {
        let doc = self.head();
        let imported = crate::world::pkg::import_map(self, doc);
        let mut out = Bare {
            types: Vec::new(),
            fns: Vec::new(),
        };
        for i in self.iter() {
            let own = std::ptr::eq(i, doc);
            // the names this document's `use`s bound FROM this file's
            // package — the same (name, pkg) match the definition
            // layer's use-graph walk reads (`matches_pkg`, module name
            // first, the origin/stem derivation on an unnamed file)
            let used_from: HashSet<&str> = imported
                .iter()
                .filter(|(_, pkg)| crate::world::pkg::matches_pkg(i, pkg))
                .map(|(name, _)| *name)
                .collect();
            // the bare gate per row: own decls always; foreign rows
            // only when the ambient linkage bound them or the
            // document's `use` named them from this package
            let offered = |name: &str, ambient: bool| own || ambient || used_from.contains(name);
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
}
