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
    /// fns by owner). `None` = not a member position; an unresolved
    /// receiver is an EMPTY tier, never a wrong one. The typing edge's
    /// absent member (`recv.` mid-typing) enumerates the FULL list.
    pub(crate) fn members(&self, pos: u32, node: AnyHandle) -> Option<Vec<MemberRow<'a>>> {
        let (_, recv) = self.member_at(pos, node)?;
        self.members_for(pos, recv)
    }

    /// the member tier keyed on an already-derived receiver (the type
    /// slot's `.`/`::` spelling: the head is the slot's own segments,
    /// node-less data on the ty path). The member node's OWN name never
    /// restricts the list — the typed prefix (the call site's filter)
    /// is the only narrowing.
    pub(crate) fn members_for(&self, pos: u32, recv: RecvNode) -> Option<Vec<MemberRow<'a>>> {
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
        let mut rows: Vec<MemberRow<'a>> = Vec::new();
        for m in ty.fields() {
            rows.push(MemberRow::Field(m));
        }
        for m in ty.methods() {
            rows.push(MemberRow::Method(m));
        }
        // inherent impl-block methods — where methods live since type
        // bodies went fields-only; the owner string IS the target
        // type's name
        for i in self.iter() {
            for f in i.fns() {
                if f.owner().as_deref() == Some(ty_name.as_str()) {
                    rows.push(MemberRow::ImplFn(f));
                }
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
    pub(crate) fn use_exports(&self, node: AnyHandle, pos: u32) -> Option<UseExports<'a>> {
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
            // index (the names come from the uniform index set, no
            // hardcoded table)
            return Some(UseExports::Modules(
                self.iter()
                    .filter_map(|f| f.mount.module.clone().map(|m| (m, f.mount.origin.clone())))
                    .collect(),
            ));
        };
        // the import is the gate-opener, so the `pub builtin` gate
        // does not bind here (this IS the use that names them)
        let scoped: Vec<&MountedFile> = self
            .iter()
            .filter(|f| crate::world::pkg::matches_pkg(f, pkg))
            .collect();
        if scoped.is_empty() {
            return Some(UseExports::Exports {
                pkg: pkg.clone(),
                path: String::new(),
                rows: Vec::new(),
            });
        }
        // resolve the walked module: the flat spelling answers the
        // root — for a mod-carrying package that is the ROOT file only
        // (members live under their module path), for a flat package
        // today's whole chain; deeper segments walk the pub edges (the
        // trailing PARTIAL ident is not a walk segment — `use
        // pkg::layout::Co⏐` walks `layout` only)
        let carrying = scoped.iter().any(|f| f.mount.mod_path.is_some());
        let interior: &[String] = &before[1..];
        let walked = if interior.is_empty() {
            Some(String::new())
        } else {
            crate::world::mods::walk_use_mods(self, pkg, interior)
        };
        let Some(path) = walked else {
            return Some(UseExports::Exports {
                pkg: pkg.clone(),
                path: String::new(),
                rows: Vec::new(),
            });
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
        }
        Some(UseExports::Exports {
            pkg: pkg.clone(),
            path,
            rows,
        })
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
        let imported = crate::world::pkg::import_map(doc);
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
