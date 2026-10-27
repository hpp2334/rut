//! Go-to-definition and go-to-type-definition — the lsp-features survey
//! §3.3 resolution order, one pure query shared by both faces:
//!
//! 1. **within-file binding layer** — locals/params/iter-vars: a LOOKUP
//!    against phase 1's binding pass (`decl_ident_span` is the target),
//!    shadow-aware and scope-exit correct;
//! 2. **within-file decl layer** — fields/enum members at their exact
//!    recovered name spans, module lets, fns/types/impl methods (token-
//!    recovered name idents), all recorded by the phase-0/1 index;
//! 3. **cross-file through the use graph** — a name the document imports
//!    (`use pouch::Vec;`) resolves only inside indexes whose explicit
//!    module name matches the named pkg, or — for the unnamed
//!    (workspace) indexes — whose origin does (path segment or file
//!    stem — how `rut.jsonc` `name` works, without re-implementing the
//!    manifest loader);
//!    un-imported names keep hover's flat-chain semantics. An OWN
//!    package head — the doc's own pkg name, or one of its own mod
//!    edges (the ancestor-walk spelling, the doc context keyed) — rides
//!    the intra-package lanes instead: the head opens the resolved
//!    module's file top, a middle segment lands on its mounting edge,
//!    the leaf on the walked mod path's decl;
//! 4. **stdlib** through the embedded surface — the `include_str!`
//!    origins carry the true `rut/...` path (`DefIndex::src_path`), so a
//!    jump lands in the real source file.
//!
//! Layering mirrors `hover` exactly (decl sites, then member position
//! with the known-receiver-is-final law, then type position, then the
//! binding pass, then the bare-name chain) so the two queries can't
//! disagree about what an ident means. Multiple hits come back as
//! multiple locations — the LSP-native candidate list (a peek), never a
//! wrong silent jump. A miss is an empty vector. Honest limits (the
//! survey §3.3): no visibility checking (the engine's `pub(mod)` rules
//! are not modeled); generics resolve at the head.

use std::collections::HashMap;

use ls_types::Range;
use rut_ast::ast::Ast;
use rut_lexer::span::Span;
use rut_lexer::token::{Tok, Token};
use rut_parser::Mode;

use crate::hover::lookup::{self, MemberHit, MemberTarget};
use crate::hover::types::head_of_ty;
use crate::hover::{bindings, DefIndex, FnDef, MemberSrc, TyDef};
use crate::line_index::LineIndex;
use crate::mods::DocMods;

/// one resolved jump target — the uri is the doc itself, a workspace
/// path/URI, or (for the embedded std surface) the true repo-relative
/// source path; the client face resolves it against the workspace
#[derive(Debug, Clone, PartialEq)]
pub struct DefLocation {
    pub uri: String,
    pub range: Range,
}

/// everything one query needs — built per request over the normalized
/// source (the same `doc_ctx` pipeline hover rides). `idxs` is the
/// lookup chain with the open document FIRST; `references` builds the
/// same shape per candidate file, so the matcher runs unchanged there.
pub struct Ctx<'a> {
    pub doc_uri: &'a str,
    pub toks: &'a [Token],
    pub ast: &'a Ast,
    /// the lookup chain: the open document first, then std + workspace
    pub idxs: &'a [&'a DefIndex],
    /// the face's knowledge of the open document's place in its
    /// package's mod tree — keys the OWN-package lanes (the
    /// `mod NAME;` edge click, an own-pkg/own-mod use head); the
    /// default (unknown) keeps them off
    pub doc_mods: &'a DocMods,
}

impl Ctx<'_> {
    /// the document's own index — the chain head
    pub(crate) fn doc(&self) -> &DefIndex {
        self.idxs[0]
    }
}

/// byte span -> LSP range over the TARGET index's own text
pub(crate) fn loc_at(doc_uri: &str, i: &DefIndex, span: Span) -> DefLocation {
    let uri = if i.origin == doc_uri {
        doc_uri.to_string()
    } else {
        i.src_path.clone().unwrap_or_else(|| origin_uri(&i.origin))
    };
    DefLocation {
        uri,
        range: range_of(&i.lines, &i.src, span),
    }
}

/// byte span -> LSP range (the analysis layer's converter, inlined here
/// so the module stays face-agnostic)
pub(crate) fn range_of(index: &LineIndex, src: &str, span: Span) -> Range {
    let (l1, c1) = index.position(src, span.lo);
    let (l2, c2) = index.position(src, span.hi);
    Range {
        start: ls_types::Position {
            line: l1,
            character: c1,
        },
        end: ls_types::Position {
            line: l2,
            character: c2,
        },
    }
}

/// a target index's origin as a URI — workspace origins are already
/// paths or `file://` URIs; the std surface's provenance labels never
/// become jump targets (they carry `src_path` instead)
fn origin_uri(origin: &str) -> String {
    if origin.contains("://") {
        origin.to_string()
    } else if origin.starts_with('/') {
        format!("file://{origin}")
    } else {
        origin.to_string()
    }
}

/// does this index speak for `pkg`? — the explicit manifest module
/// name FIRST (`DefIndex::module`, the dep/std surface's `Some`); on
/// `None` the old derivation verbatim: exact origin (the std surface's
/// label), a path segment named `pkg` (…/pouch/pouch.rut), or the file
/// stem (…/gadgets/lib.rut for pkg `gadgets`)
pub fn matches_pkg(i: &DefIndex, pkg: &str) -> bool {
    if let Some(m) = &i.module {
        return m == pkg;
    }
    let hay = i.src_path.as_deref().unwrap_or(&i.origin);
    if hay == pkg {
        return true;
    }
    hay.split(['/', '\\'])
        .any(|seg| seg.strip_suffix(".rut").unwrap_or(seg) == pkg)
}

/// the document's import map: imported name -> its `use` pkg — the
/// (name, pkg) set shape the completion tiers match foreign names
/// against (the bare tier's imported gate and the auto-import tier's
/// dedup)
pub(crate) fn import_map(doc: &DefIndex) -> HashMap<&str, &str> {
    doc.uses
        .iter()
        .map(|u| (u.name.as_str(), u.pkg.as_str()))
        .collect()
}

/// the chain restricted to the pkg that declares `name` — the use
/// graph's gate. An import the chain cannot satisfy falls back to the
/// flat chain (hover's semantics: better a broad answer than none)
fn chain_for<'a>(
    idxs: &[&'a DefIndex],
    imported: &HashMap<&str, &str>,
    name: &str,
) -> Vec<&'a DefIndex> {
    match imported.get(name) {
        Some(pkg) => {
            let filtered: Vec<&DefIndex> = idxs
                .iter()
                .copied()
                .filter(|i| matches_pkg(i, pkg))
                .collect();
            if filtered.is_empty() {
                idxs.to_vec()
            } else {
                filtered
            }
        }
        None => idxs.to_vec(),
    }
}

/// body-over-decl: a package that indexes BOTH its `.d.rut` surface
/// (`mode == Some(Decl)`) and its `mod.rut` body (`Impl` — or any
/// index whose builder did not stamp) answers from the body; the
/// surface hits ride only when they are all there is. The generic
/// payload keeps one law for every shape the layers collect (spans,
/// `&TyDef`s). A mono-file pkg — body-only (pouch) or surface-only
/// (calc, the host surfaces) — is unchanged: all its hits are one
/// kind.
///
/// One precedence above the body law: a MOUNTED dep (the dep walk's
/// or a pinned bundle's own index) outranks the ambient std embed —
/// the embed is the zero-fetch fast path for toolchain pkgs, a
/// fallback, never the answer when the real thing is mounted. The
/// embed's indexes are the only ones carrying `src_path` (the true
/// `rut/...` repo path), which marks them here.
fn prefer_bodies<'a, T: Copy>(hits: &[(&'a DefIndex, T)]) -> Vec<(&'a DefIndex, T)> {
    let mounted: Vec<(&DefIndex, T)> = hits
        .iter()
        .copied()
        .filter(|(i, _)| i.src_path.is_none())
        .collect();
    let pool: &[(&DefIndex, T)] = if mounted.is_empty() { hits } else { &mounted };
    let bodies: Vec<(&DefIndex, T)> = pool
        .iter()
        .copied()
        .filter(|(i, _)| i.mode != Some(Mode::Decl))
        .collect();
    if bodies.is_empty() {
        pool.to_vec()
    } else {
        bodies
    }
}

/// a type's definition target span — the declaring ident (token
/// recovery at build time); the whole decl is the fallback when
/// recovery failed on a degenerate parse
pub(crate) fn ty_span(t: &TyDef) -> Span {
    t.name_span.unwrap_or(t.span)
}

pub(crate) fn fn_span(f: &FnDef) -> Span {
    f.name_span.unwrap_or(f.span)
}

fn member_span(m: &MemberSrc) -> Option<Span> {
    m.name_span
}

// ---- definition ----

/// `textDocument/definition` — the ident at `pos` to its declaration.
/// Empty when nothing resolves (keywords, unknown names, primitives
/// with no surface decl).
pub fn definition(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    dedupe_hits(definition_targets(ctx, pos))
}

/// identical answers are one answer. The chain lawfully carries a
/// doc's stamped twin beside its own index (the twin may lag an open
/// edit; the doc index is freshest), and a name both carry resolves to
/// the same (uri, span) twice. The ambiguity layers answer every hit
/// ON PURPOSE — a peek, not a guess — but a doubled target is not a
/// peek; only DISTINCT targets ride.
fn dedupe_hits(locs: Vec<DefLocation>) -> Vec<DefLocation> {
    let mut out: Vec<DefLocation> = Vec::with_capacity(locs.len());
    for l in locs {
        if !out.iter().any(|e| e.uri == l.uri && e.range == l.range) {
            out.push(l);
        }
    }
    out
}

fn definition_targets(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    let Some(t) = lookup::tok_at(ctx.toks, pos) else {
        return Vec::new();
    };
    let Tok::Ident(name) = &t.tok else {
        return Vec::new();
    };
    if crate::semantic::is_keyword(name) {
        return Vec::new();
    }
    let idxs = ctx.idxs;
    let imported = import_map(ctx.doc());

    // 1. decl sites — the hovered span EXACTLY equals a recorded name
    //    span (field / enum member / module let / fn decl — free fns,
    //    impl methods, `[constructor]`s alike): the declaration IS
    //    the answer (go-to-def on a decl → itself)
    if decl_site(ctx.doc(), t.span, name) {
        return vec![DefLocation {
            uri: ctx.doc_uri.to_string(),
            range: range_of(&ctx.doc().lines, &ctx.doc().src, t.span),
        }];
    }

    // 1b. the mod-edge click — the hovered ident IS a `mod NAME;`
    //     edge's declaring ident: the mounted child's file top (the
    //     own-package lanes; an unstamped doc or an unmounted child is
    //     a miss, never a wrong jump)
    if let Some(loc) = mod_edge_target(ctx, t.span, name) {
        return vec![loc];
    }

    // 2. use-statement names and path segments — the survey's
    //    use-graph edge: ctrl+click on `Vec` inside `use pouch::Vec;`
    //    jumps to the exporting module's decl (strictly inside the
    //    named pkg, now strictly inside the walked mod path too), a
    //    MOD PATH segment jumps to the `mod NAME;` edge that mounts
    //    it, and the PKG HEAD jumps to the pkg's root index at the
    //    file top
    if let Some(locs) = use_site_targets(ctx, t.span, name) {
        return locs;
    }

    let binds = bindings::collect(ctx.ast, ctx.toks, idxs);

    // decl sites of the binding layer — the hovered ident IS a recorded
    // declaring ident (a param's, or a let the decl_site spans missed):
    // go-to-def on a decl → itself. Params sit before their scope body,
    // so the scope-containment resolve can't see them
    if binds
        .iter()
        .any(|b| b.name == *name && b.decl_ident_span == t.span)
    {
        return vec![DefLocation {
            uri: ctx.doc_uri.to_string(),
            range: range_of(&ctx.doc().lines, &ctx.doc().src, t.span),
        }];
    }

    // 3. member position — `recv.member` resolves through phase 1's
    //    receiver inference (params, lets, field reads, chained calls,
    //    for-of vars); a resolved receiver never falls through (the
    //    use-gate law hover established), an unknown one keeps hover's
    //    bare-name fallback
    if let Some(recv) = lookup::member_context(ctx.toks, t) {
        match lookup::member_target(idxs, ctx.ast, &binds, pos, name, &recv) {
            MemberHit::Found(target) => {
                return member_locations(ctx, &target);
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through to the name layers
        }
    }

    // 4. type position — capitalized / primitive names, restricted to
    //    the use graph for imported names
    if crate::hover::infer::is_cap(name) || rut_parser::is_primitive_ty(name) {
        if let Some(locs) = ty_name_locations(ctx, idxs, &imported, name) {
            return locs;
        }
        // a primitive token with no surface decl (i32, bool …) is a
        // miss — never a wrong jump
        if rut_parser::is_primitive_ty(name) {
            return Vec::new();
        }
    }

    // 5. the binding layer — locals/params/iter-vars, shadow-correct;
    //    the declaring ident span is phase 1's span-first contract
    if let Some(b) = bindings::resolve(&binds, pos, name) {
        return vec![DefLocation {
            uri: ctx.doc_uri.to_string(),
            range: range_of(&ctx.doc().lines, &ctx.doc().src, b.decl_ident_span),
        }];
    }

    // 6. module-let use sites — every hit (the candidate list), the
    //    body over the decl surface (`prefer_bodies`)
    let chain = chain_for(idxs, &imported, name);
    let mut lets: Vec<(&DefIndex, Span)> = Vec::new();
    for i in &chain {
        for l in i.lets.iter().filter(|l| l.name == *name) {
            lets.push((*i, l.name_span.unwrap_or(l.span)));
        }
    }
    let lets = prefer_bodies(&lets);
    if !lets.is_empty() {
        return lets
            .into_iter()
            .map(|(i, sp)| loc_at(ctx.doc_uri, i, sp))
            .collect();
    }

    // 7. fn names — free fns and impl methods by name; ambiguity
    //    returns every hit (a peek, not a guess), the body over the
    //    decl surface (`prefer_bodies`)
    let mut fns: Vec<(&DefIndex, Span)> = Vec::new();
    for i in &chain {
        for f in i.fns.iter().filter(|f| f.name == *name) {
            fns.push((*i, fn_span(f)));
        }
    }
    prefer_bodies(&fns)
        .into_iter()
        .map(|(i, sp)| loc_at(ctx.doc_uri, i, sp))
        .collect()
}

/// `textDocument/typeDefinition` — the survey prices this cheap and it
/// is: resolve the expression's type head (the phase-1 rules — member
/// declared types, binding types, module lets), then jump to that
/// type's declaration. The same layers, one step further.
pub fn type_definition(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    dedupe_hits(type_definition_targets(ctx, pos))
}

fn type_definition_targets(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    let Some(t) = lookup::tok_at(ctx.toks, pos) else {
        return Vec::new();
    };
    let Tok::Ident(name) = &t.tok else {
        return Vec::new();
    };
    if crate::semantic::is_keyword(name) {
        return Vec::new();
    }
    let idxs = ctx.idxs;
    let imported = import_map(ctx.doc());
    let binds = bindings::collect(ctx.ast, ctx.toks, idxs);

    // member position: the member's DECLARED type — `w.item` (a
    // `?Circle` field) jumps to the `Circle` decl; a resolved receiver
    // is final (the use-gate law)
    if let Some(recv) = lookup::member_context(ctx.toks, t) {
        match lookup::member_target(idxs, ctx.ast, &binds, pos, name, &recv) {
            MemberHit::Found(target) => {
                let ty_text = match &target {
                    MemberTarget::Member(_, _, m, _)
                    | MemberTarget::EnumMember(_, _, m)
                    | MemberTarget::IfaceMember(_, _, m) => m.ty.clone(),
                    MemberTarget::ImplFn(_, f) => f.ret.clone(),
                };
                return ty_text
                    .as_deref()
                    .map(head_of_ty)
                    .and_then(|head| ty_name_locations(ctx, idxs, &imported, &head))
                    .unwrap_or_default();
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through
        }
    }

    // a type token: its own declaration (definition's answer)
    if crate::hover::infer::is_cap(name) || rut_parser::is_primitive_ty(name) {
        if let Some(locs) = ty_name_locations(ctx, idxs, &imported, name) {
            return locs;
        }
        if rut_parser::is_primitive_ty(name) {
            return Vec::new();
        }
    }

    // locals / module lets — their type head's declaration
    if let Some(b) = bindings::resolve(&binds, pos, name) {
        return b
            .ty_head()
            .and_then(|head| ty_name_locations(ctx, idxs, &imported, &head))
            .unwrap_or_default();
    }
    match crate::hover::infer::module_let_ty(idxs, name) {
        Some(head) => ty_name_locations(ctx, idxs, &imported, &head).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// a type head's declaration location(s) — the use graph restricts the
/// search; `None` when the name has no indexed decl anywhere (a bare
/// miss, kept distinct from an empty hit list)
fn ty_name_locations(
    ctx: &Ctx,
    idxs: &[&DefIndex],
    imported: &HashMap<&str, &str>,
    name: &str,
) -> Option<Vec<DefLocation>> {
    let chain = chain_for(idxs, imported, name);
    let hits: Vec<(&DefIndex, &TyDef)> = chain
        .iter()
        .filter_map(|i| i.ty(name).map(|t| (*i, t)))
        .collect();
    // the body over the decl surface (`prefer_bodies`), so a
    // both-files dep's type lands in the body here too
    let hits = prefer_bodies(&hits);
    match hits.as_slice() {
        [] => None,
        _ => Some(
            hits.iter()
                .map(|(i, t)| loc_at(ctx.doc_uri, i, ty_span(t)))
                .collect(),
        ),
    }
}

/// where an imported name declares — strictly inside the named pkg's
/// indexes AND inside the walked mod path (`use pouch::a::b::{ C }`
/// resolves C only in pouch's module `a/b`; a walk the chain cannot
/// satisfy falls back to the flat pkg — better a broad answer than
/// none, the `chain_for` law)
fn use_graph_targets<'a>(
    idxs: &[&'a DefIndex],
    pkg: &str,
    mod_path: Option<&str>,
    name: &str,
) -> Vec<(&'a DefIndex, Span)> {
    let in_scope: Vec<&DefIndex> = match mod_path {
        Some(p) => {
            let scoped: Vec<&DefIndex> = idxs
                .iter()
                .copied()
                .filter(|i| matches_pkg(i, pkg) && crate::mods::mod_path_of(i) == p)
                .collect();
            if scoped.is_empty() {
                idxs.iter()
                    .copied()
                    .filter(|i| matches_pkg(i, pkg))
                    .collect()
            } else {
                scoped
            }
        }
        None => idxs
            .iter()
            .copied()
            .filter(|i| matches_pkg(i, pkg))
            .collect(),
    };
    let mut out: Vec<(&DefIndex, Span)> = Vec::new();
    for i in in_scope {
        if let Some(t) = i.ty(name) {
            out.push((i, ty_span(t)));
        }
        if let Some(l) = i.lets.iter().find(|l| l.name == name) {
            out.push((i, l.name_span.unwrap_or(l.span)));
        }
        for f in i.fns.iter().filter(|f| f.name == name && f.owner.is_none()) {
            out.push((i, fn_span(f)));
        }
    }
    // the body over the decl surface (`prefer_bodies`): a pkg indexing
    // both its `.d.rut` and its `mod.rut` resolves the leaf from the
    // body — the surface is the fallback when it is all there is
    prefer_bodies(&out)
}

/// the pkg's ROOT index — where a pkg-head click lands. Among the
/// chain's `matches_pkg` indexes only the root-shaped ones count: the
/// lib root `mod.rut` (`mod_path == Some("")` — how a dep walk /
/// bundle loader stamps the root) first, then the flat roots
/// (`mod_path == None` — how a host `calc.d.rut` and the std
/// surface's root files index), tie-broken body-over-decl (the
/// `prefer_bodies` law). Equals once more tie-broken toward the
/// MOUNTED closure: a scheme'd origin (`file://`, `bundle:`) is a real
/// artifact this workspace pinned, the scheme-less one the ambient std
/// surface answering for a name nothing mounted — the mount wins, the
/// embed is the fallback. No root-shaped index → `None` — an honest
/// miss, never a wrong jump.
fn pkg_root_index<'a>(idxs: &[&'a DefIndex], pkg: &str) -> Option<&'a DefIndex> {
    let mut roots: Vec<&DefIndex> = idxs
        .iter()
        .copied()
        .filter(|i| matches_pkg(i, pkg) && crate::mods::mod_path_of(i).is_empty())
        .collect();
    roots.sort_by_key(|i| {
        (
            i.mod_path.is_none(),
            i.mode == Some(Mode::Decl),
            !i.origin.contains("://"),
        )
    });
    roots.into_iter().next()
}

/// the use-statement layer: the hovered ident is a leaf name (jump to
/// the exporting decl through the walked mod path), the pkg head
/// (jump to the pkg's root index at the file top), or a middle path
/// segment (jump to the `mod NAME;` edge that mounts it). An OWN
/// package head — the doc's own pkg name, or one of its own mod edges
/// (the ancestor-walk spelling) — commits to the intra-package lanes:
/// the head opens the resolved module's file top, a middle segment
/// lands on its mounting edge, and a dead end is an honest empty,
/// never a foreign fallback.
fn use_site_targets(ctx: &Ctx, span: Span, name: &str) -> Option<Vec<DefLocation>> {
    let doc = ctx.doc();
    for u in &doc.uses {
        // the leaf name
        if u.name == name && u.name_span == Some(span) {
            let pkg = &u.path[0];
            // the OWN lane — the walk is the intra-package one (any
            // declared edge; the doc's own rows read for its own scope)
            if let Some(start) = own_lane_start(ctx, pkg) {
                let walked = crate::mods::walk_position_mods(
                    ctx.idxs,
                    ctx.doc_mods.pkg.as_deref(),
                    &start,
                    &u.path[1..],
                );
                return Some(match walked {
                    Some(p) => own_leaf_targets(ctx, &p, name),
                    None => Vec::new(),
                });
            }
            let walked = if u.path.len() == 1 {
                Some(String::new())
            } else {
                crate::mods::walk_use_mods(ctx.idxs, pkg, &u.path[1..])
            };
            return Some(
                use_graph_targets(ctx.idxs, pkg, walked.as_deref(), name)
                    .into_iter()
                    .map(|(i, sp)| loc_at(ctx.doc_uri, i, sp))
                    .collect(),
            );
        }
        // a path segment — the pkg head included: `k == 0` opens the
        // pkg's ROOT index at the file top (`Span::new(0, 0)`, the
        // convention the minted namespace head uses too)
        for (k, sp) in u.path_spans.iter().enumerate() {
            if *sp != Some(span) {
                continue;
            }
            let pkg = &u.path[0];
            // the OWN lane — `k == 0` opens the resolved module's file
            // top (the root for the pkg name, the child file for an own
            // mod edge), deeper segments the mounting edge
            if let Some(start) = own_lane_start(ctx, pkg) {
                return Some(if k == 0 {
                    match own_scope(ctx, &start) {
                        Some(i) => vec![loc_at(ctx.doc_uri, i, Span::new(0, 0))],
                        None => Vec::new(),
                    }
                } else {
                    match crate::mods::walk_position_mods(
                        ctx.idxs,
                        ctx.doc_mods.pkg.as_deref(),
                        &start,
                        &u.path[1..=k],
                    ) {
                        Some(p) => own_edge_target(ctx, &p)
                            .map(|l| vec![l])
                            .unwrap_or_default(),
                        None => Vec::new(),
                    }
                });
            }
            if k == 0 {
                return Some(
                    pkg_root_index(ctx.idxs, pkg)
                        .map(|i| vec![loc_at(ctx.doc_uri, i, Span::new(0, 0))])
                        .unwrap_or_default(),
                );
            }
            let walked = crate::mods::walk_use_mods(ctx.idxs, pkg, &u.path[1..=k])?;
            let parent =
                crate::mods::find_mod(ctx.idxs, Some(pkg), crate::mods::parent_of(&walked))?;
            let edge = parent.mods.iter().find(|m| m.name == u.path[k])?;
            return Some(vec![loc_at(
                ctx.doc_uri,
                parent,
                edge.name_span.unwrap_or(edge.span),
            )]);
        }
    }
    None
}

/// the OWN-package lanes' start path for a use head, when the head
/// speaks for the doc's own package at all: the package's own name
/// starts at the root module, any other spelling resolves through the
/// position head walk (the doc's own rows first, then its ancestors —
/// the compiler's bare-head law). `None` = a foreign head; the caller
/// rides the cross-package lanes.
fn own_lane_start(ctx: &Ctx, head: &str) -> Option<String> {
    crate::mods::resolve_position_head(ctx.doc_mods, ctx.doc(), ctx.idxs, head)
}

/// the scope index for a resolved own mod path — the open document's
/// own rows when the path IS its own module (the freshest rows — the
/// chain's stamped twin may lag an open edit), else the chain index
/// stamped for the `(pkg, path)` pair. `None` = nothing speaks for the
/// path (an unmounted module — the callers answer empty).
fn own_scope<'a>(ctx: &'a Ctx<'a>, walked: &str) -> Option<&'a DefIndex> {
    if ctx.doc_mods.mod_path.as_deref() == Some(walked) {
        return Some(ctx.doc());
    }
    crate::mods::find_mod(ctx.idxs, ctx.doc_mods.pkg.as_deref(), walked)
}

/// the own-lane leaf: the walked mod path's scope answers the
/// declaring type / module-let / free-fn spans (the use-graph's
/// collector, scoped to the one module)
fn own_leaf_targets(ctx: &Ctx, walked: &str, name: &str) -> Vec<DefLocation> {
    let Some(i) = own_scope(ctx, walked) else {
        return Vec::new();
    };
    let mut hits: Vec<(&DefIndex, Span)> = Vec::new();
    if let Some(t) = i.ty(name) {
        hits.push((i, ty_span(t)));
    }
    if let Some(l) = i.lets.iter().find(|l| l.name == name) {
        hits.push((i, l.name_span.unwrap_or(l.span)));
    }
    for f in i.fns.iter().filter(|f| f.name == name && f.owner.is_none()) {
        hits.push((i, fn_span(f)));
    }
    hits.into_iter()
        .map(|(i, sp)| loc_at(ctx.doc_uri, i, sp))
        .collect()
}

/// the own-lane interior segment: the walked path's mounting `mod
/// NAME;` edge in the parent module's index — the foreign lane's
/// target law, keyed on the own package
fn own_edge_target(ctx: &Ctx, walked: &str) -> Option<DefLocation> {
    let parent_path = crate::mods::parent_of(walked);
    let name = walked.rsplit('/').next()?;
    let parent = own_scope(ctx, parent_path)?;
    let edge = parent.mods.iter().find(|m| m.name == name)?;
    Some(loc_at(
        ctx.doc_uri,
        parent,
        edge.name_span.unwrap_or(edge.span),
    ))
}

/// the `mod NAME;` edge click — the hovered ident IS one of the doc's
/// own edges (the exact recorded name span): the child's mod path is
/// `child_path(doc's mod path, NAME)` and the jump opens the mounted
/// child at its file top (`Span::new(0, 0)`, the pkg-head convention).
/// Honest misses: an unstamped doc answers nothing; a child the chain
/// does not carry an empty jump; the `(pkg, mod_path)` pair keys the
/// match — another package's same-path child never leaks in.
fn mod_edge_target(ctx: &Ctx, span: Span, name: &str) -> Option<DefLocation> {
    ctx.doc()
        .mods
        .iter()
        .find(|m| m.name == name && m.name_span == Some(span))?;
    let from = ctx.doc_mods.mod_path.as_deref()?;
    let child = crate::mods::child_path(from, name);
    let mounted = crate::mods::find_mod(ctx.idxs, ctx.doc_mods.pkg.as_deref(), &child)?;
    Some(loc_at(ctx.doc_uri, mounted, Span::new(0, 0)))
}

fn decl_site(doc: &DefIndex, span: Span, name: &str) -> bool {
    doc.types.iter().any(|t| {
        t.fields
            .iter()
            .any(|f| f.name == name && f.name_span == Some(span))
    }) || doc
        .lets
        .iter()
        .any(|l| l.name == name && l.name_span == Some(span))
        // fn decls ride the same law — the name a ctor shares with
        // every other class's ctor is exactly the case the name layers
        // must never answer with a cross-pkg fan-out
        || doc
            .fns
            .iter()
            .any(|f| f.name == name && f.name_span == Some(span))
}

fn member_locations(ctx: &Ctx, target: &MemberTarget) -> Vec<DefLocation> {
    match target {
        MemberTarget::Member(i, _, m, _)
        | MemberTarget::EnumMember(i, _, m)
        | MemberTarget::IfaceMember(i, _, m) => match member_span(m) {
            Some(sp) => vec![loc_at(ctx.doc_uri, i, sp)],
            // a member whose name recovery failed has no span to jump
            // to — an empty answer, never a wrong jump
            None => Vec::new(),
        },
        MemberTarget::ImplFn(i, f) => vec![loc_at(ctx.doc_uri, i, fn_span(f))],
    }
}
