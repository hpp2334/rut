//! The one walk from a caret to declarations (law L3). [`resolve`]
//! matches the classification ([`super::cursor::Cursor`]) to one of
//! six arms; the lane order inside each arm is binding. `[]` is a
//! miss; several is the candidate peek (hover's to render —
//! `definition` gates the one-name refusal's rows to `[]`, the
//! silence law).

use rut_ast::{IdentId, ModDeclNode, UseNode, Vis};

use super::cursor::{member_span, path_seg, Cursor, UseAt};
use super::def::Def;
use crate::world::collection::{
    ambiguous_scope, file_scope, member, prefer_bodies, qualified_module, qualified_scope, CandRow,
    MemberHit, MemberTarget,
};
use crate::world::infer::is_cap;
use crate::world::pkg::{import_map, matches_pkg};
use crate::world::{File, FileId, World};

/// the caret's resolutions — one arm per cursor kind
pub(crate) fn resolve(w: &World, file: FileId, c: &Cursor) -> Vec<Def> {
    match c {
        Cursor::Decl(d) => decl_targets(w, file, d),
        Cursor::Use { .. } => use_targets(w, file, c),
        Cursor::Path { .. } => path_targets(w, file, c),
        Cursor::Member {
            recv: _,
            name: Some(_),
            ..
        } => member_targets(w, file, c),
        Cursor::Member { name: None, .. } => Vec::new(),
        Cursor::Name { .. } => name_targets(w, file, c),
        Cursor::Selfy { .. } | Cursor::Nothing => Vec::new(),
    }
}

/// the decl-site arm: a `mod` edge resolves to its mounted child (the
/// child's file top — [`Def::Module`]); unmounted, the walk continues
/// into the name lanes on the edge's name, which answers nothing for a
/// lowercase edge. Every other decl answers itself.
fn decl_targets(w: &World, file: FileId, d: &Def) -> Vec<Def> {
    let Def::Node { node, .. } = d else {
        // a binding row IS the declaration (the self-jump)
        return vec![*d];
    };
    if let Some(m) = node.downcast::<ModDeclNode>() {
        let ast = w.file(file).doc.ast();
        let md = m.get(ast);
        let name = ast.name(md.name.ident);
        // the child's mod path is `child_path(doc's mod path, NAME)`;
        // the `(pkg, mod_path)` pair keys the match — another
        // package's same-path child never leaks in. An unstamped doc
        // answers nothing here and falls through.
        if let Some(from) = w.file(file).stamp.mod_path.as_deref() {
            let child = rut_lir::check::child_path(from, name);
            if let Some(mounted) =
                crate::resolve::mods::find_mod(w, w.file(file).stamp.module.as_deref(), &child)
            {
                return vec![Def::Module { file: mounted }];
            }
        }
        // unmounted: the name lanes on the edge's name (today's
        // fall-through)
        return name_lane(w, file, md.name.ident, md.name.span.lo);
    }
    vec![*d]
}

/// the use-edge arm — today's use-site lanes: the leaf (own lane's
/// visible rows, else the use graph's `pub` rows, else a module leaf's
/// file top), the head (the own lane's module file, else the pkg root
/// index), a middle segment (the own lane's mounting edge, else the
/// pub-walk's edge in the parent module)
fn use_targets(w: &World, file: FileId, c: &Cursor) -> Vec<Def> {
    let Cursor::Use { node, segs, at } = c else {
        return Vec::new();
    };
    let ast = w.file(file).doc.ast();
    let uh = node
        .downcast::<UseNode>()
        .expect("a use cursor rides a use node");
    let ud = uh.get(ast);
    match at {
        UseAt::Leaf(i) => {
            let name = ast.name(ud.names[*i].ident);
            // the one-name refusal: the walk answers the candidate
            // rows (definition's gate silences them — rule B)
            if let Some(rows) = refusal_defs(w, file, name) {
                return rows;
            }
            let pkg = segs[0].clone();
            // the OWN lane — the walk is the intra-package one (any
            // declared edge; the doc's own rows read for its own
            // scope), the leaf visibility-gates at the door
            if let Some(start) = crate::resolve::mods::position_head(w, file, &pkg) {
                return match crate::resolve::mods::position_walk(
                    w,
                    w.file(file).stamp.module.as_deref(),
                    file,
                    &start,
                    &segs[1..],
                ) {
                    Some(p) => own_leaf_defs(w, file, &p, name),
                    None => Vec::new(),
                };
            }
            let walked = if ud.path.len() == 1 {
                Some(String::new())
            } else {
                crate::resolve::mods::use_walk(w, &pkg, file, &segs[1..])
            };
            let defs = use_graph_defs(w, file, &pkg, walked.as_deref(), name);
            if !defs.is_empty() {
                return defs;
            }
            // a MODULE leaf (`use calc::{ math }`): the walked module's
            // `pub` child opens at its file top — the pkg-head
            // convention
            if let Some(p) = walked {
                if let Some((child, _)) = crate::resolve::mods::use_module_leaf(w, &pkg, &p, name) {
                    return vec![Def::Module { file: child }];
                }
            }
            Vec::new()
        }
        UseAt::Head => {
            let pkg = &segs[0];
            // the OWN lane — the package's own name starts at the root
            // module, an own mod edge at the edge's child (the
            // compiler's bare-head law)
            if let Some(start) = crate::resolve::mods::position_head(w, file, pkg) {
                return match own_scope(w, file, &start) {
                    Some(f) => vec![Def::Module { file: f }],
                    None => Vec::new(),
                };
            }
            // the pkg's ROOT file, picked once for every consumer
            pkg_root_index(w, file, pkg)
                .map(|f| vec![Def::Module { file: f }])
                .unwrap_or_default()
        }
        UseAt::Segment(k) => {
            let pkg = &segs[0];
            // the OWN lane — deeper segments walk intra-package, the
            // mounting edge in the parent module answers
            if let Some(start) = crate::resolve::mods::position_head(w, file, pkg) {
                return match crate::resolve::mods::position_walk(
                    w,
                    w.file(file).stamp.module.as_deref(),
                    file,
                    &start,
                    &segs[1..=*k],
                ) {
                    Some(p) => own_edge_defs(w, file, &p),
                    None => Vec::new(),
                };
            }
            // the pub-walk's edge in the parent module
            let Some(walked) = crate::resolve::mods::use_walk(w, pkg, file, &segs[1..=*k]) else {
                return Vec::new();
            };
            let Some(parent) =
                crate::resolve::mods::find_mod(w, Some(pkg), rut_lir::check::parent_of(&walked))
            else {
                return Vec::new();
            };
            let edge_name = ast.name(ud.path[*k].ident);
            let Some(edge) = w
                .file(parent)
                .mods()
                .into_iter()
                .find(|m| m.name() == edge_name)
            else {
                return Vec::new();
            };
            vec![Def::Node {
                file: parent,
                node: edge.h,
            }]
        }
    }
}

/// a qualified path's head/mid segment — the module the prefix names:
/// its mounting `mod NAME;` edge in the parent module's file
/// ([`Def::Node`] — today's law, keyed on the queried file's own entry
/// when the parent IS its module), else — where that resolution fails
/// (an unstamped document's own edge, a parent the world does not
/// carry) — the module itself at its file top ([`Def::Module`], the
/// plan's delta 6: the walked module stands). `None` = no module —
/// the caller keeps the bare-name fall-through on the ridden segment.
fn module_segment_defs(w: &World, file: FileId, segs: &[String], is_ty: bool) -> Option<Vec<Def>> {
    let target = qualified_module(w, file, segs, is_ty)?;
    let path = crate::resolve::mods::mod_path_of(w.file(target));
    if path.is_empty() {
        // the package's own name — the root module, at its file top
        // (the pkg-head convention)
        return Some(vec![Def::Module { file: target }]);
    }
    // a `mod NAME;` edge in the parent module's file — inside the
    // TARGET's own package (the queried file's own entry when the
    // parent is the open document)
    let name = path.rsplit('/').next();
    let parent_path = rut_lir::check::parent_of(path);
    let parent = if w.file(file).stamp.module.as_deref() == w.file(target).stamp.module.as_deref()
        && w.file(file).stamp.mod_path.as_deref() == Some(parent_path)
    {
        Some(file)
    } else {
        crate::resolve::mods::find_mod(w, w.file(target).stamp.module.as_deref(), parent_path)
    };
    let edge = parent.and_then(|p| {
        w.file(p)
            .mods()
            .into_iter()
            .find(|m| Some(m.name()) == name)
            .map(|m| Def::Node { file: p, node: m.h })
    });
    Some(match edge {
        Some(def) => vec![def],
        // the module stands, its edge unresolvable — the file top
        None => vec![Def::Module { file: target }],
    })
}

/// the qualified-path arm — a head/middle segment names the module the
/// prefix lands on ([`module_segment_defs`]; `[]` when the walk
/// fails — the bare lanes never read a qualified spelling). The TAIL
/// rides the member law keyed on the path's own receiver first (a
/// found or final-miss receiver ends the walk; an unknown receiver
/// falls through), then the module namespace lane with the cursor's
/// `is_ty` — authoritative: an unbound head is `[]`.
fn path_targets(w: &World, file: FileId, c: &Cursor) -> Vec<Def> {
    let Cursor::Path {
        node,
        segs,
        at,
        is_ty,
    } = c
    else {
        return Vec::new();
    };
    if at + 1 != segs.len() {
        // a head/middle segment — the module the prefix names: its
        // mounting edge, else (where the edge does not resolve) the
        // module at its file top (delta 6). A walk that fails keeps
        // the bare-name fall-through on the ridden segment — today's
        // law, pinned (`Widget::new()`'s head answers the use-bound
        // class): a DEAD qualified prefix is no qualified spelling.
        let seg = path_seg(w.file(file).doc.ast(), *node, *at);
        return match module_segment_defs(w, file, &segs[..=*at], *is_ty) {
            Some(defs) => defs,
            None => name_lane(w, file, seg.0, seg.1.lo),
        };
    }
    // the tail's member law — the receiver is the segment before the
    // tail (the door derives it from the node)
    let ast = w.file(file).doc.ast();
    let sp = path_seg(ast, *node, *at).1;
    match member(w, file, sp.lo, *node) {
        MemberHit::Found(target) => return target_defs(w, &target),
        MemberHit::None => return Vec::new(),
        MemberHit::UnknownReceiver => {} // fall through
    }
    match qualified_scope(w, file, segs, *is_ty) {
        None => Vec::new(),
        Some(scope) => {
            let mut defs: Vec<Def> = scope
                .types
                .iter()
                .map(|t| node_def(w, t.file, t.h))
                .collect();
            if !*is_ty {
                defs.extend(scope.lets.iter().map(|l| node_def(w, l.file, l.h)));
                defs.extend(scope.fns.iter().map(|f| node_def(w, f.file, f.h)));
            }
            defs
        }
    }
}

/// the member arm — today's member law (`member`), the target mapped
/// to `Def`s; an unknown receiver falls through to the name lanes on
/// the member's name (hover's law — the more informative answer; a
/// resolved receiver never falls through, and a final miss ends the
/// walk). The classification's (member, receiver) record feeds the
/// door — one derivation, no re-walk.
fn member_targets(w: &World, file: FileId, c: &Cursor) -> Vec<Def> {
    let Cursor::Member {
        node,
        recv,
        name: Some(id),
    } = c
    else {
        return Vec::new();
    };
    let sp = member_span(w.file(file).doc.ast(), *node);
    match crate::world::collection::member_gated(w, file, sp.lo, Some((Some(*id), *recv))) {
        MemberHit::Found(target) => target_defs(w, &target),
        MemberHit::None => Vec::new(),
        // the bare-name fallback on the member's own name
        MemberHit::UnknownReceiver => name_lane(w, file, *id, sp.lo),
    }
}

/// the bare-name arm — the name lanes keyed on the cursor's own
/// recorded name
fn name_targets(w: &World, file: FileId, c: &Cursor) -> Vec<Def> {
    let Cursor::Name { id, span, .. } = c else {
        return Vec::new();
    };
    name_lane(w, file, *id, span.lo)
}

/// THE NAME LANES — the definition feature's layers 4-7, keyed on a
/// name: a capitalized or primitive name through the file scope's
/// types (a primitive with none is a miss), then the binding pass
/// (the declaring row), then the one-name refusal's candidate rows,
/// then the file scope's lets, then its free fns. `at` is the
/// binding pass's probe (a byte inside the name — a token never
/// straddles scopes, so its start is faithful).
pub(crate) fn name_lane(w: &World, file: FileId, id: IdentId, at: u32) -> Vec<Def> {
    let d = w.file(file);
    let name = d.doc.ast().name(id);
    // 4. type position — the file scope's spelled type (the ONE home)
    if is_cap(name) || crate::semantic::is_primitive_ty(name) {
        let tys: Vec<Def> = file_scope(w, file, name)
            .types
            .iter()
            .map(|t| node_def(w, t.file, t.h))
            .collect();
        if !tys.is_empty() {
            return tys;
        }
        // a primitive token with no surface decl (i32, bool …) is a
        // miss — never a wrong jump
        if crate::semantic::is_primitive_ty(name) {
            return Vec::new();
        }
    }
    // 5. the binding layer — locals/params/iter-vars, shadow-correct;
    // the declaring ident is the span-first contract
    if let Some(b) = rut_ast::resolve_binding(d.doc.bindings(), at, id) {
        let row = d
            .doc
            .bindings()
            .iter()
            .position(|r| std::ptr::eq(r, b))
            .expect("the binding pass answers a row of this doc");
        return vec![Def::Binding { file, row }];
    }
    // the one-name refusal — the walk answers the candidate list (the
    // peek is hover's; definition's gate silences it)
    if let Some(rows) = refusal_defs(w, file, name) {
        return rows;
    }
    // 6. module-let use sites — the file scope's lets, every hit
    let scope = file_scope(w, file, name);
    if !scope.lets.is_empty() {
        return scope
            .lets
            .iter()
            .map(|l| node_def(w, l.file, l.h))
            .collect();
    }
    // 7. fn names — the file scope's fns (free fns and impl methods),
    // every hit
    scope.fns.iter().map(|f| node_def(w, f.file, f.h)).collect()
}

/// a world node as a [`Def`] — the id joined back through identity
/// (the views carry `&File`; the answer keys files by id)
fn node_def(w: &World, f: &File, h: rut_ast::AnyHandle) -> Def {
    Def::Node {
        file: w.id_of(f),
        node: h,
    }
}

/// the one-name refusal's candidate rows as bare [`Def`]s — the walk's
/// honest multi-answer. A use leaf that binds nothing of its own has
/// no declaration to name (the spelling is the candidate), so it rides
/// the answer only through the hover's rendering, which reads
/// [`ambiguous_scope`]'s rows again for the provenance (which `use`
/// bound each — a bare `Def` carries none).
fn refusal_defs(w: &World, file: FileId, name: &str) -> Option<Vec<Def>> {
    let cands = ambiguous_scope(w, file, name)?;
    let defs: Vec<Def> = cands
        .iter()
        .filter_map(|c| match &c.row {
            CandRow::Ty(t) => Some(node_def(w, t.file, t.h)),
            CandRow::Let(l) => Some(node_def(w, l.file, l.h)),
            CandRow::Fn(f) => Some(node_def(w, f.file, f.h)),
            CandRow::Mod(m) => Some(node_def(w, m.file, m.h)),
            // a leaf that binds nothing: no declaration to name
            CandRow::Leaf(_) => None,
        })
        .collect();
    Some(defs)
}

/// a resolved member target as [`Def`]s — the target's declaring node
/// in its own file (the member/enum-member views carry the owning
/// type beside the node)
fn target_defs(w: &World, target: &MemberTarget) -> Vec<Def> {
    let def = match target {
        MemberTarget::Member(m) | MemberTarget::EnumMember(m) | MemberTarget::IfaceMember(m) => {
            node_def(w, m.file, m.h)
        }
        MemberTarget::ImplFn(f) => node_def(w, f.file, f.h),
    };
    vec![def]
}

/// the own-lane leaf: the walked mod path's scope answers the
/// declaring type / module-let / free-fn defs — each row VISIBLE from
/// the importing file (`rut_lir::check::vis_crosses`: "import is no
/// visibility bypass" — a leaf the use line refuses jumps nowhere),
/// the module's `pub use` rows never followed (the compiler's binding
/// refuses a re-export import)
fn own_leaf_defs(w: &World, file: FileId, walked: &str, name: &str) -> Vec<Def> {
    let Some(f) = own_scope(w, file, walked) else {
        return Vec::new();
    };
    let from = w.file(file).stamp.mod_path.as_deref().unwrap_or("");
    let allows = |vis: Vis| rut_lir::check::vis_crosses(from, walked, vis);
    let scope = w.file(f);
    let mut defs: Vec<Def> = Vec::new();
    if let Some(t) = scope.ty(name).filter(|t| allows(t.vis())) {
        defs.push(Def::Node { file: f, node: t.h });
    }
    if let Some(l) = scope
        .lets()
        .into_iter()
        .find(|l| l.name() == name && allows(l.vis()))
    {
        defs.push(Def::Node { file: f, node: l.h });
    }
    for fv in scope
        .fns()
        .into_iter()
        .filter(|f| f.name() == name && f.owner().is_none() && allows(f.vis()))
    {
        defs.push(Def::Node {
            file: f,
            node: fv.h,
        });
    }
    defs
}

/// the own-lane interior segment: the walked path's mounting `mod
/// NAME;` edge in the parent module's file — the foreign lane's
/// target law, keyed on the own package
fn own_edge_defs(w: &World, file: FileId, walked: &str) -> Vec<Def> {
    let parent_path = rut_lir::check::parent_of(walked);
    let Some(name) = walked.rsplit('/').next() else {
        return Vec::new();
    };
    let Some(parent) = own_scope(w, file, parent_path) else {
        return Vec::new();
    };
    let Some(edge) = w.file(parent).mods().into_iter().find(|m| m.name() == name) else {
        return Vec::new();
    };
    vec![Def::Node {
        file: parent,
        node: edge.h,
    }]
}

/// the scope file for a resolved own mod path — the queried file's
/// own entry when the path IS its own module, else the world file
/// stamped for the `(pkg, path)` pair. `None` = nothing speaks for
/// the path (an unmounted module — the callers answer empty).
fn own_scope(w: &World, file: FileId, walked: &str) -> Option<FileId> {
    if w.file(file).stamp.mod_path.as_deref() == Some(walked) {
        return Some(file);
    }
    crate::resolve::mods::find_mod(w, w.file(file).stamp.module.as_deref(), walked)
}

/// the pkg's ROOT file — where a pkg-head click lands. Among the
/// world's `matches_pkg` files only the root-shaped ones count: the
/// lib root `mod.rut` (`mod_path == Some("")` — how a dep walk /
/// bundle loader stamps the root) first, then the flat roots
/// (`mod_path == None` — how a host `calc.d.rut` and a flat dep's
/// root file mount), tie-broken body-over-decl (the
/// `prefer_bodies` law). Equals once more tie-broken toward the
/// MOUNTED closure: a scheme'd origin (`file://`, `bundle:`) is a real
/// artifact this workspace pinned, the scheme-less one an ambient
/// embed answering for a name nothing mounted — the mount wins, the
/// embed is the fallback. No root-shaped file → `None` — an honest
/// miss, never a wrong jump.
pub(crate) fn pkg_root_index(w: &World, file: FileId, pkg: &str) -> Option<FileId> {
    // the queried file is no package's root by a path coincidence (a
    // document that IS the root rides the own lane before this door)
    let mut roots: Vec<FileId> = w
        .chain(file)
        .filter(|id| {
            *id != file
                && matches_pkg(w.file(*id), pkg)
                && crate::resolve::mods::mod_path_of(w.file(*id)).is_empty()
        })
        .collect();
    roots.sort_by_key(|id| {
        let f = w.file(*id);
        (
            f.stamp.mod_path.is_none(),
            f.stamp.mode == crate::semantic::Mode::Decl,
            !f.stamp.origin.contains("://"),
        )
    });
    roots.into_iter().next()
}

/// where an imported name declares — strictly inside the named pkg's
/// files AND inside the walked mod path (`use pouch::a::b::{ C }`
/// resolves C only in pouch's module `a/b`; a walk the world cannot
/// satisfy falls back to the flat pkg — better a broad answer than
/// none, the `chain_for` law). Only `pub` rows bind a cross-package
/// leaf (the compiler's leaf gate — a private or `pub(pkg)` row the
/// use line refuses), and the queried file itself never exports to
/// its own import (layer 1 speaks for it).
fn use_graph_defs(
    w: &World,
    file: FileId,
    pkg: &str,
    mod_path: Option<&str>,
    name: &str,
) -> Vec<Def> {
    let in_scope: Vec<FileId> = match mod_path {
        Some(p) => {
            let scoped: Vec<FileId> = w
                .chain(file)
                .filter(|id| {
                    matches_pkg(w.file(*id), pkg)
                        && crate::resolve::mods::mod_path_of(w.file(*id)) == p
                })
                .collect();
            if scoped.is_empty() {
                w.chain(file)
                    .filter(|id| matches_pkg(w.file(*id), pkg))
                    .collect()
            } else {
                scoped
            }
        }
        None => w
            .chain(file)
            .filter(|id| matches_pkg(w.file(*id), pkg))
            .collect(),
    };
    let mut out: Vec<(&File, Def)> = Vec::new();
    for id in in_scope {
        if id == file {
            continue;
        }
        let f = w.file(id);
        if let Some(t) = f.ty(name).filter(|t| t.vis() == Vis::Pub) {
            out.push((
                f,
                Def::Node {
                    file: id,
                    node: t.h,
                },
            ));
        }
        if let Some(l) = f
            .lets()
            .into_iter()
            .find(|l| l.name() == name && l.vis() == Vis::Pub)
        {
            out.push((
                f,
                Def::Node {
                    file: id,
                    node: l.h,
                },
            ));
        }
        for fv in f
            .fns()
            .into_iter()
            .filter(|f| f.name() == name && f.owner().is_none() && f.vis() == Vis::Pub)
        {
            out.push((
                f,
                Def::Node {
                    file: id,
                    node: fv.h,
                },
            ));
        }
    }
    // the body over the decl surface (`prefer_bodies`): a pkg indexing
    // both its `.d.rut` and its `mod.rut` resolves the leaf from the
    // body — the surface is the fallback when it is all there is
    prefer_bodies(&out)
        .into_iter()
        .map(|(_, def)| def)
        .collect()
}

/// a type head's declaration targets — typeDefinition's computed-head
/// lane (today's `ty_name_locations`): the type name here was REACHED
/// THROUGH A VALUE (a member's declared type, a binding's inferred
/// head, a module let's), not spelled at the cursor, so the
/// file-scope law does not gate it — the use graph restricts the
/// search ([`chain_for`]), the body over the decl surface
/// ([`prefer_bodies`]), and `None` when the name has no indexed decl
/// anywhere (a bare miss, kept distinct from an empty hit list).
/// Spelled type tokens answer through the file scope (the name
/// lanes).
pub(crate) fn type_targets(w: &World, file: FileId, name: &str) -> Option<Vec<Def>> {
    let defs: Vec<Def> = chain_for(w, file, name)
        .into_iter()
        .filter_map(|f| f.ty(name).map(|t| node_def(w, f, t.h)))
        .collect();
    // the body over the decl surface (`prefer_bodies`), so a
    // both-files dep's type lands in the body here too
    let pairs: Vec<(&File, Def)> = defs.into_iter().map(|d| (w.file(d.file()), d)).collect();
    let hits = prefer_bodies(&pairs);
    match hits.as_slice() {
        [] => None,
        _ => Some(hits.into_iter().map(|(_, d)| d).collect()),
    }
}

/// the world restricted to the pkg that declares `name` — the
/// computed-head lane's gate (typeDefinition's value-reached type
/// heads; the spelling lanes answer through the file scope instead).
/// An import the world cannot satisfy falls back to the flat world
/// (navigation for a computed head: better a broad answer than none)
fn chain_for<'a>(w: &'a World, file: FileId, name: &str) -> Vec<&'a File> {
    let imported = import_map(w, file);
    match imported.get(name) {
        Some(pkg) => {
            let filtered: Vec<&File> = w
                .chain(file)
                .map(|id| w.file(id))
                .filter(|f| matches_pkg(f, pkg))
                .collect();
            if filtered.is_empty() {
                w.chain(file).map(|id| w.file(id)).collect()
            } else {
                filtered
            }
        }
        None => w.chain(file).map(|id| w.file(id)).collect(),
    }
}
