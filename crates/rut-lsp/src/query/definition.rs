//! Go-to-definition and go-to-type-definition — the lsp-features survey
//! §3.3 resolution order, one pure query every host shares:
//!
//! 1. **within-file binding layer** — locals/params/iter-vars: a LOOKUP
//!    against the binding pass (`decl_ident_span` is the target),
//!    shadow-aware and scope-exit correct;
//! 2. **within-file decl layer** — fields/enum members at their exact
//!    recorded name spans, module lets, fns/types/impl methods, all
//!    keyed on the decl maps' node name spans;
//! 3. **cross-file through the use graph** — a name the document imports
//!    (`use pouch::Vec;`) resolves only inside files whose explicit
//!    module name matches the named pkg, or — for the unnamed
//!    (workspace) files — whose origin does (path segment or file
//!    stem — how `rut.jsonc` `name` works, without re-implementing the
//!    manifest loader);
//!    un-imported names keep hover's flat-chain semantics. An OWN
//!    package head — the doc's own pkg name, or one of its own mod
//!    edges (the ancestor-walk spelling, the doc context keyed) — rides
//!    the intra-package lanes instead: the head opens the resolved
//!    module's file top, a middle segment lands on its mounting edge,
//!    the leaf on the walked mod path's decl;
//! 4. **the ambient prelude** — the one embedded file's
//!    `include_str!` origin carries the true `rut/...` path
//!    (`Mount::src_path`), so a jump lands in the real source file.
//!
//! Layering mirrors `hover` exactly (decl sites, then member position
//! with the known-receiver-is-final law, then type position, then the
//! binding pass, then the bare-name chain) so the two queries can't
//! disagree about what an ident means. Multiple hits come back as
//! multiple locations — the LSP-native candidate list (a peek), never a
//! wrong silent jump. A miss is an empty vector. Honest limits (the
//! survey §3.3): no visibility checking (the engine's `pub(super)`/
//! `pub(pkg)` rules are not modeled); generics resolve at the head.

use std::collections::HashMap;

use crate::hover::bindings;
use crate::hover::collection::{Chain, MemberHit, MemberTarget, MountedFile};
use crate::hover::lookup;
use crate::hover::types::head_of_ty;
use crate::mods::DocMods;
use crate::semantic::Span;
use crate::semantic::{Tok, Token};
use ls_types::Range;

/// one resolved jump target — the uri is the doc itself, a workspace
/// path/URI, or (for the embedded prelude) the true repo-relative
/// source path; the client face resolves it against the workspace
#[derive(Debug, Clone, PartialEq)]
pub struct DefLocation {
    pub uri: String,
    pub range: Range,
}

/// everything one query needs — the collection's chain (the open
/// document at the head) plus the face's knowledge of where the
/// document sits. `references` builds the same shape per candidate
/// file, so the matcher runs unchanged there.
pub struct Ctx<'a> {
    pub doc_uri: &'a str,
    /// the open document (the chain head's — the fresh entry the sync
    /// lane keeps)
    pub doc: &'a crate::semantic::Document,
    /// the lookup chain: the open document first, then the prelude + workspace
    pub chain: &'a Chain<'a>,
    /// the face's knowledge of the open document's place in its
    /// package's mod tree — keys the OWN-package lanes (the
    /// `mod NAME;` edge click, an own-pkg/own-mod use head); the
    /// default (unknown) keeps them off
    pub doc_mods: &'a DocMods,
}

impl<'a> Ctx<'a> {
    /// the document's own mounted file — the chain head
    pub(crate) fn doc_file(&self) -> &'a MountedFile {
        self.chain.head()
    }

    /// the tokens the query positions speak (the doc's own lex)
    pub(crate) fn toks(&self) -> &[Token] {
        self.doc.toks()
    }

    pub(crate) fn ast(&self) -> &rut_ast::Ast {
        self.doc.ast()
    }
}

/// byte span -> LSP range over the TARGET file's own text
pub(crate) fn loc_at(doc_uri: &str, f: &MountedFile, span: Span) -> DefLocation {
    let uri = if f.mount.origin == doc_uri {
        doc_uri.to_string()
    } else {
        f.mount
            .src_path
            .clone()
            .unwrap_or_else(|| origin_uri(&f.mount.origin))
    };
    DefLocation {
        uri,
        range: range_of(f.doc.lines(), f.doc.src(), span),
    }
}

/// byte span -> LSP range (the analysis layer's converter, inlined here
/// so the module stays face-agnostic)
pub(crate) fn range_of(index: &crate::semantic::LineIndex, src: &str, span: Span) -> Range {
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

/// a target file's origin as a URI — workspace origins are already
/// paths or `file://` URIs; the prelude's provenance label never
/// becomes a jump target (it carries `src_path` instead)
fn origin_uri(origin: &str) -> String {
    if origin.contains("://") {
        origin.to_string()
    } else if origin.starts_with('/') {
        format!("file://{origin}")
    } else {
        origin.to_string()
    }
}

/// does this file speak for `pkg`? — the explicit manifest module
/// name FIRST (`Mount::module`, the dep/prelude file's `Some`); on
/// `None` the old derivation verbatim: exact origin, a path segment
/// named `pkg` (…/pouch/pouch.rut), or the file
/// stem (…/gadgets/lib.rut for pkg `gadgets`)
pub fn matches_pkg(f: &MountedFile, pkg: &str) -> bool {
    if let Some(m) = &f.mount.module {
        return m == pkg;
    }
    let hay = f.mount.src_path.as_deref().unwrap_or(&f.mount.origin);
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
pub(crate) fn import_map(doc: &MountedFile) -> HashMap<&str, &str> {
    doc.uses()
        .into_iter()
        .map(|u| (u.name(), u.pkg()))
        .collect()
}

/// the chain restricted to the pkg that declares `name` — the use
/// graph's gate. An import the chain cannot satisfy falls back to the
/// flat chain (hover's semantics: better a broad answer than none)
fn chain_for<'a>(
    chain: &'a Chain<'a>,
    imported: &HashMap<&str, &str>,
    name: &str,
) -> Vec<&'a MountedFile> {
    match imported.get(name) {
        Some(pkg) => {
            let filtered: Vec<&MountedFile> =
                chain.iter().filter(|f| matches_pkg(f, pkg)).collect();
            if filtered.is_empty() {
                chain.iter().collect()
            } else {
                filtered
            }
        }
        None => chain.iter().collect(),
    }
}

/// a type's definition target span — the declaring ident (the node's
/// own recorded name span); the whole decl is the fallback when the
/// name span is missing on a degenerate parse
pub(crate) fn ty_span(t: &crate::hover::collection::TyView) -> Span {
    t.name_span().unwrap_or(t.span())
}

pub(crate) fn fn_span(f: &crate::hover::collection::FnView) -> Span {
    f.name_span().unwrap_or(f.span())
}

// ---- definition ----

/// `textDocument/definition` — the ident at `pos` to its declaration.
/// Empty when nothing resolves (keywords, unknown names, primitives
/// with no surface decl).
pub fn definition(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    dedupe_hits(definition_targets(ctx, pos))
}

/// identical answers are one answer. The chain lawfully carries a
/// doc's stamped twin beside its own entry (the twin may lag an open
/// edit; the doc entry is freshest), and a name both carry resolves to
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
    let Some(t) = lookup::tok_at(ctx.toks(), pos) else {
        return Vec::new();
    };
    let Tok::Ident(name) = &t.tok else {
        return Vec::new();
    };
    if crate::semantic::is_keyword(name) {
        return Vec::new();
    }
    let chain = ctx.chain;
    let doc = ctx.doc_file();
    let imported = import_map(doc);

    // 1. decl sites — the hovered span EXACTLY equals a recorded name
    //    span (field / enum member / module let / fn decl — free fns,
    //    impl methods, `[constructor]`s alike): the declaration IS
    //    the answer (go-to-def on a decl → itself)
    if decl_site(doc, t.span, name) {
        return vec![DefLocation {
            uri: ctx.doc_uri.to_string(),
            range: range_of(doc.doc.lines(), doc.doc.src(), t.span),
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
    //    it, and the PKG HEAD jumps to the pkg's root file at the
    //    file top
    if let Some(locs) = use_site_targets(ctx, t.span, name) {
        return locs;
    }

    let binds = bindings::collect(ctx.ast(), ctx.toks(), chain);

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
            range: range_of(doc.doc.lines(), doc.doc.src(), t.span),
        }];
    }

    // 3. member position — `recv.member` resolves through the
    //    receiver inference (params, lets, field reads, chained calls,
    //    for-of vars); a resolved receiver never falls through (the
    //    use-gate law hover established), an unknown one keeps hover's
    //    bare-name fallback
    if let Some(recv) = lookup::member_context(ctx.toks(), t) {
        match chain.member(&binds, pos, name, &recv) {
            MemberHit::Found(target) => {
                return member_locations(ctx, &target);
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through to the name layers
        }
    }

    // 4. type position — capitalized / primitive names, restricted to
    //    the use graph for imported names
    if crate::hover::infer::is_cap(name) || crate::semantic::is_primitive_ty(name) {
        if let Some(locs) = ty_name_locations(ctx, chain, &imported, name) {
            return locs;
        }
        // a primitive token with no surface decl (i32, bool …) is a
        // miss — never a wrong jump
        if crate::semantic::is_primitive_ty(name) {
            return Vec::new();
        }
    }

    // 5. the binding layer — locals/params/iter-vars, shadow-correct;
    //    the declaring ident span is the span-first contract
    if let Some(b) = bindings::resolve(&binds, pos, name) {
        return vec![DefLocation {
            uri: ctx.doc_uri.to_string(),
            range: range_of(doc.doc.lines(), doc.doc.src(), b.decl_ident_span),
        }];
    }

    // 6. module-let use sites — every hit (the candidate list), the
    //    body over the decl surface (`prefer_bodies`)
    let scoped = chain_for(chain, &imported, name);
    let mut lets: Vec<(&MountedFile, Span)> = Vec::new();
    for f in &scoped {
        for l in f.lets().into_iter().filter(|l| l.name() == *name) {
            lets.push((f, l.name_span().unwrap_or(l.span())));
        }
    }
    let lets = chain.prefer_bodies(&lets);
    if !lets.is_empty() {
        return lets
            .into_iter()
            .map(|(f, sp)| loc_at(ctx.doc_uri, f, sp))
            .collect();
    }

    // 7. fn names — free fns and impl methods by name; ambiguity
    //    returns every hit (a peek, not a guess), the body over the
    //    decl surface (`prefer_bodies`)
    let mut fns: Vec<(&MountedFile, Span)> = Vec::new();
    for f in &scoped {
        for fv in f.fns().into_iter().filter(|f| f.name() == *name) {
            fns.push((f, fn_span(&fv)));
        }
    }
    chain
        .prefer_bodies(&fns)
        .into_iter()
        .map(|(f, sp)| loc_at(ctx.doc_uri, f, sp))
        .collect()
}

/// `textDocument/typeDefinition` — the survey prices this cheap and it
/// is: resolve the expression's type head (the receiver rules — member
/// declared types, binding types, module lets), then jump to that
/// type's declaration. The same layers, one step further.
pub fn type_definition(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    dedupe_hits(type_definition_targets(ctx, pos))
}

fn type_definition_targets(ctx: &Ctx, pos: u32) -> Vec<DefLocation> {
    let Some(t) = lookup::tok_at(ctx.toks(), pos) else {
        return Vec::new();
    };
    let Tok::Ident(name) = &t.tok else {
        return Vec::new();
    };
    if crate::semantic::is_keyword(name) {
        return Vec::new();
    }
    let chain = ctx.chain;
    let imported = import_map(ctx.doc_file());
    let binds = bindings::collect(ctx.ast(), ctx.toks(), chain);

    // member position: the member's DECLARED type — `w.item` (a
    // `?Circle` field) jumps to the `Circle` decl; a resolved receiver
    // is final (the use-gate law)
    if let Some(recv) = lookup::member_context(ctx.toks(), t) {
        match chain.member(&binds, pos, name, &recv) {
            MemberHit::Found(target) => {
                let ty_text = match &target {
                    MemberTarget::Member(_, m, _)
                    | MemberTarget::EnumMember(_, m)
                    | MemberTarget::IfaceMember(_, m) => m.ty(),
                    MemberTarget::ImplFn(f) => f.ret(),
                };
                return ty_text
                    .as_deref()
                    .map(head_of_ty)
                    .and_then(|head| ty_name_locations(ctx, chain, &imported, &head))
                    .unwrap_or_default();
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through
        }
    }

    // a type token: its own declaration (definition's answer)
    if crate::hover::infer::is_cap(name) || crate::semantic::is_primitive_ty(name) {
        if let Some(locs) = ty_name_locations(ctx, chain, &imported, name) {
            return locs;
        }
        if crate::semantic::is_primitive_ty(name) {
            return Vec::new();
        }
    }

    // locals / module lets — their type head's declaration
    if let Some(b) = bindings::resolve(&binds, pos, name) {
        return b
            .ty_head()
            .and_then(|head| ty_name_locations(ctx, chain, &imported, &head))
            .unwrap_or_default();
    }
    match crate::hover::infer::module_let_ty(chain.iter(), name) {
        Some(head) => ty_name_locations(ctx, chain, &imported, &head).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// a type head's declaration location(s) — the use graph restricts the
/// search; `None` when the name has no indexed decl anywhere (a bare
/// miss, kept distinct from an empty hit list)
fn ty_name_locations(
    ctx: &Ctx,
    chain: &Chain,
    imported: &HashMap<&str, &str>,
    name: &str,
) -> Option<Vec<DefLocation>> {
    let scoped = chain_for(chain, imported, name);
    let hits: Vec<(&MountedFile, Span)> = scoped
        .iter()
        .filter_map(|f| f.ty(name).map(|t| (*f, ty_span(&t))))
        .collect();
    // the body over the decl surface (`prefer_bodies`), so a
    // both-files dep's type lands in the body here too
    let hits = chain.prefer_bodies(&hits);
    match hits.as_slice() {
        [] => None,
        _ => Some(
            hits.iter()
                .map(|(f, sp)| loc_at(ctx.doc_uri, f, *sp))
                .collect(),
        ),
    }
}

/// where an imported name declares — strictly inside the named pkg's
/// files AND inside the walked mod path (`use pouch::a::b::{ C }`
/// resolves C only in pouch's module `a/b`; a walk the chain cannot
/// satisfy falls back to the flat pkg — better a broad answer than
/// none, the `chain_for` law)
fn use_graph_targets<'a>(
    chain: &'a Chain<'a>,
    pkg: &str,
    mod_path: Option<&str>,
    name: &str,
) -> Vec<(&'a MountedFile, Span)> {
    let in_scope: Vec<&MountedFile> = match mod_path {
        Some(p) => {
            let scoped: Vec<&MountedFile> = chain
                .iter()
                .filter(|f| matches_pkg(f, pkg) && crate::mods::mod_path_of(f) == p)
                .collect();
            if scoped.is_empty() {
                chain.iter().filter(|f| matches_pkg(f, pkg)).collect()
            } else {
                scoped
            }
        }
        None => chain.iter().filter(|f| matches_pkg(f, pkg)).collect(),
    };
    let mut out: Vec<(&MountedFile, Span)> = Vec::new();
    for f in in_scope {
        if let Some(t) = f.ty(name) {
            out.push((f, ty_span(&t)));
        }
        if let Some(l) = f.lets().into_iter().find(|l| l.name() == name) {
            out.push((f, l.name_span().unwrap_or(l.span())));
        }
        for fv in f
            .fns()
            .into_iter()
            .filter(|f| f.name() == name && f.owner().is_none())
        {
            out.push((f, fn_span(&fv)));
        }
    }
    // the body over the decl surface (`prefer_bodies`): a pkg indexing
    // both its `.d.rut` and its `mod.rut` resolves the leaf from the
    // body — the surface is the fallback when it is all there is
    chain.prefer_bodies(&out)
}

/// the pkg's ROOT file — where a pkg-head click lands. Among the
/// chain's `matches_pkg` files only the root-shaped ones count: the
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
fn pkg_root_index<'a>(chain: &'a Chain<'a>, pkg: &str) -> Option<&'a MountedFile> {
    let mut roots: Vec<&MountedFile> = chain
        .iter()
        .filter(|f| matches_pkg(f, pkg) && crate::mods::mod_path_of(f).is_empty())
        .collect();
    roots.sort_by_key(|f| {
        (
            f.mount.mod_path.is_none(),
            f.mount.mode == Some(crate::semantic::Mode::Decl),
            !f.mount.origin.contains("://"),
        )
    });
    roots.into_iter().next()
}

/// the use-statement layer: the hovered ident is a leaf name (jump to
/// the exporting decl through the walked mod path), the pkg head
/// (jump to the pkg's root file at the file top), or a middle path
/// segment (jump to the `mod NAME;` edge that mounts it). An OWN
/// package head — the doc's own pkg name, or one of its own mod edges
/// (the ancestor-walk spelling) — commits to the intra-package lanes:
/// the head opens the resolved module's file top, a middle segment
/// lands on its mounting edge, and a dead end is an honest empty,
/// never a foreign fallback.
fn use_site_targets(ctx: &Ctx, span: Span, name: &str) -> Option<Vec<DefLocation>> {
    let doc = ctx.doc_file();
    let chain = ctx.chain;
    for u in doc.uses() {
        // the leaf name
        if u.name() == name && u.name_span() == Some(span) {
            let pkg = u.pkg().to_string();
            // the OWN lane — the walk is the intra-package one (any
            // declared edge; the doc's own rows read for its own scope)
            if let Some(start) = own_lane_start(ctx, &pkg) {
                let walked = crate::mods::walk_position_mods(
                    chain,
                    ctx.doc_mods.pkg.as_deref(),
                    &start,
                    &u.path()[1..],
                );
                return Some(match walked {
                    Some(p) => own_leaf_targets(ctx, &p, name),
                    None => Vec::new(),
                });
            }
            let walked = if u.path().len() == 1 {
                Some(String::new())
            } else {
                crate::mods::walk_use_mods(chain, &pkg, &u.path()[1..])
            };
            return Some(
                use_graph_targets(chain, &pkg, walked.as_deref(), name)
                    .into_iter()
                    .map(|(f, sp)| loc_at(ctx.doc_uri, f, sp))
                    .collect(),
            );
        }
        // a path segment — the pkg head included: `k == 0` opens the
        // pkg's ROOT file at the file top (`Span::new(0, 0)`, the
        // convention a pkg-root row uses)
        for (k, sp) in u.path_spans().iter().enumerate() {
            if *sp != Some(span) {
                continue;
            }
            let pkg = u.pkg().to_string();
            // the OWN lane — `k == 0` opens the resolved module's file
            // top (the root for the pkg name, the child file for an own
            // mod edge), deeper segments the mounting edge
            if let Some(start) = own_lane_start(ctx, &pkg) {
                return Some(if k == 0 {
                    match own_scope(ctx, &start) {
                        Some(f) => vec![loc_at(ctx.doc_uri, f, Span::new(0, 0))],
                        None => Vec::new(),
                    }
                } else {
                    match crate::mods::walk_position_mods(
                        chain,
                        ctx.doc_mods.pkg.as_deref(),
                        &start,
                        &u.path()[1..=k],
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
                    pkg_root_index(chain, &pkg)
                        .map(|f| vec![loc_at(ctx.doc_uri, f, Span::new(0, 0))])
                        .unwrap_or_default(),
                );
            }
            let walked = crate::mods::walk_use_mods(chain, &pkg, &u.path()[1..=k])?;
            let parent = crate::mods::find_mod(chain, Some(&pkg), crate::mods::parent_of(&walked))?;
            let edge = parent
                .mods()
                .into_iter()
                .find(|m| m.name() == u.path()[k])?;
            return Some(vec![loc_at(
                ctx.doc_uri,
                parent,
                edge.name_span().unwrap_or(edge.span()),
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
    crate::mods::resolve_position_head(ctx.doc_mods, ctx.doc_file(), ctx.chain, head)
}

/// the scope file for a resolved own mod path — the open document's
/// own entry when the path IS its own module (the freshest decls — the
/// sync lane keeps it fresher than any stamped twin), else the chain
/// file stamped for the `(pkg, path)` pair. `None` = nothing speaks for
/// the path (an unmounted module — the callers answer empty).
fn own_scope<'a>(ctx: &Ctx<'a>, walked: &str) -> Option<&'a MountedFile> {
    if ctx.doc_mods.mod_path.as_deref() == Some(walked) {
        return Some(ctx.doc_file());
    }
    crate::mods::find_mod(ctx.chain, ctx.doc_mods.pkg.as_deref(), walked)
}

/// the own-lane leaf: the walked mod path's scope answers the
/// declaring type / module-let / free-fn spans (the use-graph's
/// collector, scoped to the one module)
fn own_leaf_targets(ctx: &Ctx, walked: &str, name: &str) -> Vec<DefLocation> {
    let Some(f) = own_scope(ctx, walked) else {
        return Vec::new();
    };
    let mut hits: Vec<(&MountedFile, Span)> = Vec::new();
    if let Some(t) = f.ty(name) {
        hits.push((f, ty_span(&t)));
    }
    if let Some(l) = f.lets().into_iter().find(|l| l.name() == name) {
        hits.push((f, l.name_span().unwrap_or(l.span())));
    }
    for fv in f
        .fns()
        .into_iter()
        .filter(|f| f.name() == name && f.owner().is_none())
    {
        hits.push((f, fn_span(&fv)));
    }
    hits.into_iter()
        .map(|(f, sp)| loc_at(ctx.doc_uri, f, sp))
        .collect()
}

/// the own-lane interior segment: the walked path's mounting `mod
/// NAME;` edge in the parent module's file — the foreign lane's
/// target law, keyed on the own package
fn own_edge_target(ctx: &Ctx, walked: &str) -> Option<DefLocation> {
    let parent_path = crate::mods::parent_of(walked);
    let name = walked.rsplit('/').next()?;
    let parent = own_scope(ctx, parent_path)?;
    let edge = parent.mods().into_iter().find(|m| m.name() == name)?;
    Some(loc_at(
        ctx.doc_uri,
        parent,
        edge.name_span().unwrap_or(edge.span()),
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
    ctx.doc_file()
        .mods()
        .into_iter()
        .find(|m| m.name() == name && m.name_span() == Some(span))?;
    let from = ctx.doc_mods.mod_path.as_deref()?;
    let child = crate::mods::child_path(from, name);
    let mounted = crate::mods::find_mod(ctx.chain, ctx.doc_mods.pkg.as_deref(), &child)?;
    Some(loc_at(ctx.doc_uri, mounted, Span::new(0, 0)))
}

fn decl_site(doc: &MountedFile, span: Span, name: &str) -> bool {
    doc.types().iter().any(|t| {
        t.fields()
            .iter()
            .any(|f| f.name() == name && f.name_span() == Some(span))
    }) || doc
        .lets()
        .iter()
        .any(|l| l.name() == name && l.name_span() == Some(span))
        // fn decls ride the same law — the name a ctor shares with
        // every other class's ctor is exactly the case the name layers
        // must never answer with a cross-pkg fan-out
        || doc
            .fns()
            .iter()
            .any(|f| f.name() == name && f.name_span() == Some(span))
}

fn member_locations(ctx: &Ctx, target: &MemberTarget) -> Vec<DefLocation> {
    match target {
        MemberTarget::Member(ty, m, _)
        | MemberTarget::EnumMember(ty, m)
        | MemberTarget::IfaceMember(ty, m) => match m.name_span() {
            Some(sp) => vec![loc_at(ctx.doc_uri, ty.file, sp)],
            // a member whose name span is missing on a degenerate parse
            // has no span to jump to — an empty answer, never a wrong
            // jump
            None => Vec::new(),
        },
        MemberTarget::ImplFn(f) => vec![loc_at(ctx.doc_uri, f.file, fn_span(f))],
    }
}
