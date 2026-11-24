//! Go-to-definition and go-to-type-definition — the feature doors
//! over the unified walk. One classification
//! ([`crate::resolve::cursor`]) and one walk
//! ([`crate::resolve::walk::resolve`]) answer what the caret names as
//! [`crate::resolve::Def`]s; this module locates them
//! ([`crate::resolve::Def::location`]) and carries the survey §3.3's
//! honesty rules:
//!
//! * a miss is an empty vector; multiple hits come back as multiple
//!   locations — the LSP-native candidate list (a peek), never a
//!   wrong silent jump;
//! * **the one-name silence**: a name the file binds twice by
//!   distinct bindings — a one-name violation the checker refuses —
//!   jumps NOWHERE at a use site and at every bare-name spelling the
//!   binding layer cannot resolve (never a pick between them; the
//!   hover lists the candidates). Decl sites answer before the gate —
//!   a file the law refuses still jumps the decl under the cursor to
//!   ITSELF;
//! * un-imported names answer NOTHING (the file-scope law —
//!   sibling modules may share a name, so a name the file neither
//!   declares nor imports is no jump, never a foreign decl);
//! * no visibility checking on the spelling lanes (the engine's
//!   `pub(super)`/`pub(pkg)` rules gate the use lanes inside the
//!   walk); generics resolve at the head.
//!
//! The silence gate ([`definition_silent`]) is the one law's
//! definition-side half — `references` rides it at the same cursor
//! kinds for its query AND every scanned site (a refused name counts
//! nothing, on either side of the query).

use crate::resolve::cursor::{Cursor, UseAt};
use crate::world::collection::{one_name_refused, MemberHit, MemberTarget};
use crate::world::infer::QualHit;
use crate::world::types::head_of_ty;
use crate::world::{File, FileId, World};

use rut_ast::UseNode;

use crate::resolve::DefLocation;

// ---- definition ----

/// `textDocument/definition` — the ident at `pos` to its declaration.
/// Empty when nothing resolves (keywords, unknown names, primitives
/// with no surface decl).
///
/// The resolver family's bodies-flavored door: body-vs-surface is the
/// pick policy INSIDE the walk (`prefer_bodies` — the
/// mounted-over-embed rule, then the body over the `.d.rut` surface).
/// The plan's declaration flavor (`get_declaration`) is consciously
/// superseded — the wire this face serves spells no
/// `textDocument/declaration` request (the three-lane ABI is frozen),
/// so the door has no caller; the pick policy's single decision point
/// makes the inverted walk a local change when one arrives.
pub fn definition(w: &World, file: FileId, pos: u32) -> Vec<DefLocation> {
    let c = crate::resolve::cursor::cursor(w, file, pos);
    let defs = crate::resolve::walk::resolve(w, file, &c);
    // the one-name law's silence (see the module doc)
    if definition_silent(w, file, pos, &c) {
        return Vec::new();
    }
    defs.iter().map(|d| d.location(w)).collect()
}

/// the one-name law's silence for definition, at today's logical
/// points (by cursor kind): a use leaf the file binds twice, and a
/// bare name the binding layer cannot resolve beside a second
/// distinct binding. Decl sites answered before the gate (the walk's
/// `Decl` arm) — never silenced. `references` rides the same gate on
/// its query and on every scanned site.
pub(crate) fn definition_silent(w: &World, file: FileId, pos: u32, c: &Cursor) -> bool {
    let d = w.file(file);
    let ast = d.doc.ast();
    match c {
        Cursor::Use {
            node,
            at: UseAt::Leaf(i),
            ..
        } => {
            let uh = node
                .downcast::<UseNode>()
                .expect("a use cursor rides a use node");
            let ud = uh.get(ast);
            one_name_refused(w, file, ast.name(ud.names[*i].ident))
        }
        Cursor::Name { id, .. } => {
            rut_ast::resolve_binding(d.doc.bindings(), pos, *id).is_none()
                && one_name_refused(w, file, ast.name(*id))
        }
        _ => false,
    }
}

/// `textDocument/typeDefinition` — the survey prices this cheap and it
/// is: resolve the expression's type head (the receiver rules — member
/// declared types, binding types, module lets), then jump to that
/// type's declaration. The same classification, one step further: a
/// member position resolves the member's declared type; every other
/// position keys the name lanes on the cursor's own recorded name and
/// locates the type head's targets
/// ([`crate::resolve::walk::type_targets`]).
pub fn get_type_definition(w: &World, file: FileId, pos: u32) -> Vec<DefLocation> {
    let doc = w.file(file);
    let c = crate::resolve::cursor::cursor(w, file, pos);
    // the fence residue and the Self spellings name no declaration
    if matches!(c, Cursor::Nothing | Cursor::Selfy { .. }) {
        return Vec::new();
    }
    // member position: the member's DECLARED type — `w.item` (a
    // `?Circle` field) jumps to the `Circle` decl; a resolved receiver
    // is final (the use-gate law)
    let member_node = match &c {
        Cursor::Member {
            node,
            name: Some(_),
            ..
        } => Some(*node),
        Cursor::Path {
            node,
            segs,
            at,
            is_ty: _,
        } if at + 1 == segs.len() => Some(*node),
        _ => None,
    };
    if let Some(h) = member_node {
        match crate::world::collection::member(w, file, pos, h) {
            MemberHit::Found(target) => {
                let ty_text = match &target {
                    MemberTarget::Member(m)
                    | MemberTarget::EnumMember(m)
                    | MemberTarget::IfaceMember(m) => m.ty(),
                    MemberTarget::ImplFn(f) => f.ret(),
                };
                return locate_ty_head(w, file, ty_text.as_deref().map(head_of_ty));
            }
            MemberHit::None => return Vec::new(),
            MemberHit::UnknownReceiver => {} // fall through
        }
    }
    // the name lanes key on the cursor's own recorded name (a member
    // position names no bare name — the member token is not a
    // name-lane key)
    let Some(name_id) = cursor_name(w, file, &c) else {
        return Vec::new();
    };
    let name = doc.doc.ast().name(name_id).to_string();
    // a type token: its own declaration (definition's answer). A type
    // the one-name law refuses — beside a same-named decl or leaf —
    // answers definition's way: its own decl site is ITSELF, every
    // other spelling of the name jumps nowhere (never a pick between
    // the bindings)
    if crate::world::infer::is_cap(&name) || crate::semantic::is_primitive_ty(&name) {
        if one_name_refused(w, file, &name) {
            return match &c {
                // the decl under the cursor is itself (a name position
                // carries no decl site — an honest [])
                Cursor::Decl(d) => vec![d.location(w)],
                _ => Vec::new(),
            };
        }
        if let Some(defs) = crate::resolve::walk::type_targets(w, file, &name) {
            return defs.iter().map(|d| d.location(w)).collect();
        }
        if crate::semantic::is_primitive_ty(&name) {
            return Vec::new();
        }
    }
    // locals / module lets — their type head's declaration
    if let Some(b) = rut_ast::resolve_binding(doc.doc.bindings(), pos, name_id) {
        let files: Vec<&File> = w.chain(file).map(|id| w.file(id)).collect();
        let qual = |segs: &[String], is_ty: bool| -> Option<QualHit> {
            let scope = crate::world::collection::qualified_scope(w, file, segs, is_ty)?;
            Some(if is_ty {
                QualHit::Ty(scope.types.first()?.name().to_string())
            } else {
                QualHit::Fn(scope.fns.into_iter().next()?)
            })
        };
        return locate_ty_head(
            w,
            file,
            crate::world::infer::binding_ty(doc, &files, b, &qual)
                .as_deref()
                .map(head_of_ty),
        );
    }
    locate_ty_head(
        w,
        file,
        crate::world::infer::module_let_ty(files_of(w, file), &name),
    )
}

/// the computed head's locations — `type_targets`' defs located (an
/// absent head is an empty answer, never a wrong jump)
fn locate_ty_head(w: &World, file: FileId, head: Option<String>) -> Vec<DefLocation> {
    head.and_then(|h| crate::resolve::walk::type_targets(w, file, &h))
        .map(|defs| defs.iter().map(|d| d.location(w)).collect())
        .unwrap_or_default()
}

/// the name the cursor's classification rode — the name lanes' key
/// for typeDefinition. A member position names no bare name (the
/// member token is not a name-lane key); a use edge neither.
fn cursor_name(w: &World, file: FileId, c: &Cursor) -> Option<rut_core::IdentId> {
    let ast = w.file(file).doc.ast();
    match c {
        Cursor::Decl(d) => match d {
            crate::resolve::Def::Node { node, .. } => {
                crate::resolve::def::decl_ident(ast, *node).map(|(id, _)| id)
            }
            crate::resolve::Def::Binding { file: bf, row } => {
                Some(w.file(*bf).doc.bindings()[*row].name)
            }
            crate::resolve::Def::Module { .. } => None,
        },
        Cursor::Name { id, .. } => Some(*id),
        Cursor::Path { node, at, .. } => Some(crate::resolve::cursor::path_seg(ast, *node, *at).0),
        Cursor::Member { .. } | Cursor::Use { .. } | Cursor::Selfy { .. } | Cursor::Nothing => None,
    }
}

/// the query chain as a plain file window — the inference pass's
/// shape (the queried file first, the world after)
fn files_of(w: &World, file: FileId) -> Vec<&File> {
    w.chain(file).map(|id| w.file(id)).collect()
}
