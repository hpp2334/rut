//! Markdown rendering — verbatim-signature code blocks, provenance
//! lines, doc paragraphs; a candidate list on ambiguity. Plus the
//! phase-1 shapes: the binding pass's identifier hover, the M7
//! primitive blurbs, and the module-let decl render. Every verbatim
//! slice is a span×src join on the OWNING file's document (the views'
//! law) — nothing rendered is stored text.

use crate::doc::is_cap;
use crate::resolve::cursor::{self, Cursor, UseAt};
use crate::resolve::def::{Def, View};
use crate::resolve::{ambiguous_scope, binding_ty, CandRow, Candidate};
use crate::world::ty::{Ty, TyForm};
use crate::world::views::{FnView, LetView, MemberView, ModView, TyView};
use crate::world::{File, FileId, World};
use rut_ast::{
    AnyHandle, BindKind, Binding, EnumMemberNode, FieldDeclNode, IdentId, ModDeclNode,
    ModuleLetNode, UseNode,
};

use super::lookup::HoverOut;

fn code_block(body: &str) -> String {
    format!("```rut\n{body}\n```")
}

fn provenance(origin: &str, line: u32) -> String {
    format!("— {origin}:{line}")
}

pub(crate) fn render_ty(t: &TyView) -> String {
    let mut out = String::new();
    // the decl head is the view's own verbatim slice (`head()` — from
    // the first token through the name and generics, modifiers
    // included); the arms shape only the BODY layout. No synthesized
    // text: the LSP verbatim law renders source-less rows nowhere
    match t.form() {
        TyForm::Alias => {
            // `type X = A;` — the whole decl as written
            out.push_str(&code_block(&format!("{};", t.head())));
        }
        TyForm::Enum => {
            // the member rows are the source-true slices
            let members: Vec<String> = t.fields().into_iter().map(|f| f.src()).collect();
            out.push_str(&code_block(&format!(
                "{} {{ {} }}",
                t.head(),
                members.join(", ")
            )));
        }
        TyForm::Interface | TyForm::Builtin | TyForm::Primitive => {
            let mut body = String::new();
            for m in t.methods() {
                body.push_str("    ");
                body.push_str(&m.src());
                body.push('\n');
            }
            out.push_str(&code_block(&format!("{} {{\n{}}}", t.head(), body)));
        }
        // a dep's host surface renders like a class body: the consts as
        // fields, the module's fns as the method count
        TyForm::Class | TyForm::Struct | TyForm::HostStruct => {
            let mut body = String::new();
            for f in t.fields() {
                body.push_str("    ");
                body.push_str(&f.src());
                body.push('\n');
            }
            out.push_str(&code_block(&format!("{} {{\n{}}}", t.head(), body)));
            let methods = t.methods();
            if !methods.is_empty() {
                out.push_str(&format!(
                    "\n{} method{} — hover one for its signature",
                    methods.len(),
                    if methods.len() == 1 { "" } else { "s" }
                ));
            }
        }
    }
    let origin = &t.file.stamp.origin;
    if !origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(origin, t.line())));
    }
    for d in t.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

pub(crate) fn render_member(m: &MemberView, via: Option<String>) -> String {
    let ty = &m.ty;
    let mut out = code_block(&m.src());
    match via {
        Some(v) => out.push_str(&format!("\nfrom `{v}`")),
        None if ty.form() == TyForm::Interface => {
            out.push_str(&format!("\ndeclared in `{}`", ty.name()));
        }
        None => out.push_str(&format!("\nin `{}`", ty.name())),
    }
    let origin = &m.file.stamp.origin;
    if !origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(origin, m.line())));
    }
    for d in m.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

pub(crate) fn render_fn_hits(one: &FnView) -> String {
    let mut out = code_block(&one.src());
    if let Some(o) = one.owner() {
        out.push_str(&format!("\nin `{o}`"));
    }
    let origin = &one.file.stamp.origin;
    if !origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(origin, one.line())));
    }
    for d in one.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

/// the designated constructor's section — appended to the class block
/// when the hover rides a class callee: the label, the verbatim impl
/// slice (`[constructor] pub fn new(..) -> Self` whole), the ctor's
/// doc lines. The door guarantees the impl is the class's OWN file's
/// (the compiler's inherent-impl rule — a cross-module impl is no
/// ctor of the class), so no provenance line rides: the class block
/// above already cites that file.
pub(crate) fn render_ctor_section(ctor: &FnView) -> String {
    let mut out = format!("constructor:\n{}", code_block(&ctor.src()));
    for d in ctor.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

/// the newtype's synthesized mint line — a positional newtype's only
/// constructor is the compiler-provided mint, source-less BY DESIGN
/// (no ctor decl exists to slice; the LSP verbatim law renders
/// inferred/semantic facts). `None` when the decl carries no field —
/// a damaged shape renders no mint, never a wrong one.
pub(crate) fn render_newtype_mint(t: &TyView) -> Option<String> {
    t.mint_sig()
        .map(|sig| format!("constructor — the newtype mint: {sig}"))
}

/// the candidate list — one bullet per binding: the verbatim decl
/// (source-less only for a leaf that binds nothing), its kind, the
/// use edge it rode when one bound it, and the owning file's
/// provenance
pub(crate) fn render_candidates(many: &[Candidate]) -> String {
    let mut out = String::from("multiple definitions:");
    for c in many {
        out.push_str("\n- ");
        out.push_str(&c.line());
    }
    out
}

impl Candidate<'_> {
    /// one bullet: `` `pub fn mk(n: i32) -> i32` — fn — via `use m::{
    /// mk }` — file:///ws/app/src/m/mod.rut:13 ``
    fn line(&self) -> String {
        let (src, kind, provenance) = match &self.row {
            CandRow::Ty(t) => (
                t.head(),
                match t.form() {
                    TyForm::Class => "class",
                    TyForm::Struct => "struct",
                    TyForm::Interface => "interface",
                    TyForm::Enum => "enum",
                    TyForm::Builtin => "builtin",
                    TyForm::Primitive => "builtin primitive",
                    TyForm::HostStruct => "host struct",
                    TyForm::Alias => "alias",
                }
                .to_string(),
                Some((t.file.stamp.origin.clone(), t.line())),
            ),
            CandRow::Let(l) => (
                l.src(),
                "module let".to_string(),
                Some((l.file.stamp.origin.clone(), l.line())),
            ),
            CandRow::Fn(f) => (
                f.src(),
                match f.owner() {
                    Some(o) => format!("fn (method of `{o}`)"),
                    None => "fn".to_string(),
                },
                Some((f.file.stamp.origin.clone(), f.line())),
            ),
            CandRow::Mod(m) => (
                m.src(),
                "module".to_string(),
                Some((m.file.stamp.origin.clone(), m.line())),
            ),
            // a use leaf that binds nothing: no row of its own — the
            // spelling is the candidate
            CandRow::Leaf(u) => (
                u.src(),
                "use leaf (nothing binds)".to_string(),
                Some((u.file.stamp.origin.clone(), u.line())),
            ),
        };
        let mut out = format!("`{}` — {kind}", src.replace('\n', " "));
        if let Some(u) = &self.via {
            out.push_str(&format!(" — via `{}`", u.src().replace('\n', " ")));
        }
        if let Some((origin, line)) = provenance.filter(|(o, _)| !o.is_empty()) {
            out.push_str(&format!(" — {origin}:{line}"));
        }
        out
    }
}

/// the identifier hover — the binding row rendered: the binding as it
/// binds (`const c: ?Circle`, `let x`, `p: Point`), then the kind; an
/// INFERRED type says so (the display-side scoping ruling: declared
/// annotations render as written — the row's span join — inference is
/// labeled). `name` is the interner spelling (the rows carry ids);
/// `ty` is the resolver's declared-or-inferred answer.
pub(crate) fn binding_markdown(b: &Binding, name: &str, ty: Option<Ty>) -> String {
    // the binding as it binds — the head keyword rides along (`const`
    // vs `let`) for a let-form binding and a const loop var; a param
    // renders bare (params are mutable by default)
    let kw = match (b.kind, b.is_const) {
        (BindKind::Let, true) => "const ",
        (BindKind::Let, false) => "let ",
        (BindKind::IterVar, true) => "const ",
        _ => "",
    };
    let head = match &ty {
        Some(t) => format!("{kw}{name}: {t}"),
        None => format!("{kw}{name}"),
    };
    let mut out = code_block(&head);
    out.push_str(&format!("\n{}", b.kind.label()));
    if b.ty_span.is_none() && ty.is_some() {
        out.push_str(" — type inferred");
    }
    out
}

/// M7 — the numeric/bool primitives have no surface decl to index
/// (`str`/`bytes`/`opaque` do, through core's `builtin primitive`
/// statements, and those keep their rich hover): synthesize the static
/// blurb from rut-core's documented surface. Truthful constants only;
/// `None` for names this table doesn't own
pub(crate) fn render_primitive(name: &str) -> Option<String> {
    let body = match name {
        "bool" => {
            return Some(format!(
                "{}\n\nthe two booleans — `true` / `false`",
                code_block("bool")
            ));
        }
        "f32" => format!(
            "{}\n\nIEEE-754 binary32 — the unsuffixed float default\n\nliterals `1.0`, `1.0f32`",
            code_block("f32")
        ),
        "f64" => format!(
            "{}\n\nIEEE-754 binary64 — double precision\n\nliterals `1.0f64`",
            code_block("f64")
        ),
        "i8" => prim_line(name, "signed", 8, "-128 ..= 127"),
        "i16" => prim_line(name, "signed", 16, "-32768 ..= 32767"),
        "i32" => prim_line(name, "signed", 32, "-2147483648 ..= 2147483647"),
        "i64" => prim_line(
            name,
            "signed",
            64,
            "-9223372036854775808 ..= 9223372036854775807",
        ),
        "u8" => prim_line(name, "unsigned", 8, "0 ..= 255"),
        "u16" => prim_line(name, "unsigned", 16, "0 ..= 65535"),
        "u32" => prim_line(name, "unsigned", 32, "0 ..= 4294967295"),
        "u64" => prim_line(name, "unsigned", 64, "0 ..= 18446744073709551615"),
        _ => return None,
    };
    Some(body)
}

fn prim_line(name: &str, signedness: &str, bits: u32, range: &str) -> String {
    format!(
        "{}\n\n{signedness} {bits}-bit integer — {range}\n\nliterals `0`, `0{name}`; the `wrapping_*` / `saturating_*` / `checked_*` members are compiler-lowered (declared on the primitive's own body in `core.d.rut`)",
        code_block(name)
    )
}

/// the module-let decl render — verbatim slice, provenance, doc; the
/// type line shows the annotation or the inferred head
pub(crate) fn render_let(w: &World, l: &LetView) -> String {
    let mut out = code_block(&l.src());
    out.push_str("\nmodule let");
    if let Some(ty) = crate::resolve::module_let_ty(w, w.id_of(l.file), l) {
        out.push_str(&format!(" — `{ty}`"));
    }
    let origin = &l.file.stamp.origin;
    if !origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(origin, l.line())));
    }
    for d in l.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

/// the resolved file-module render — a use-path segment's target (or
/// any resolved module): the mounted file as the subject (its
/// root-relative path — the display law's fact), the child count,
/// provenance
pub(crate) fn render_module_target(w: &World, f: &File, path: &str) -> String {
    let file = crate::resolve::mods::display_of_single(w, f, path);
    let mut out = code_block(&file);
    out.push_str("\nfile module");
    match f.mods().len() {
        0 => out.push_str("\nno child modules"),
        1 => out.push_str("\n1 child module"),
        n => out.push_str(&format!("\n{n} child modules")),
    }
    if !f.stamp.origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(&f.stamp.origin, 1)));
    }
    out
}

/// the `pub? mod NAME;` decl render — the edge as spelled (the
/// verbatim slice — never re-synthesized), the kind, the mounted file
/// the namespace resolves to, and the child's own `mod` count (the
/// walkable path segments beneath it). `children` is `None` when the
/// mounted file is not in the chain (an honest dash, never a wrong
/// count).
pub(crate) fn render_mod(f: &File, m: &ModView, mounted: &str, children: Option<usize>) -> String {
    let mut out = code_block(&format!("{};", m.src()));
    out.push_str(&format!("\nfile module — `{}`", mounted));
    if let Some(n) = children {
        out.push_str(&match n {
            0 => "\nno child modules".to_string(),
            1 => "\n1 child module".to_string(),
            n => format!("\n{n} child modules"),
        });
    }
    if !f.stamp.origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(&f.stamp.origin, m.line())));
    }
    for d in m.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

// ---- the feature's render half: classify, resolve, render ----
//
// One markdown per cursor kind, from the walk's answer; the span each
// lane decorates is the cursor's own record. Where a pinned rendering
// needs what a bare `Def` cannot carry — the candidates' `use`
// provenance (rule A), a dead walk's bare-name fall-through, a use
// leaf that binds nothing — the rendering reads the shared resolver
// doors again, exactly where the old lanes read them.

/// the hover's render half — the answer is the walk's `Def`s; the
/// cursor names the lane and carries the span
pub(crate) fn hover(
    w: &World,
    file: FileId,
    pos: u32,
    c: &Cursor,
    defs: &[Def],
) -> Option<HoverOut> {
    let span = cursor::cursor_span(w, file, c);
    let markdown = match c {
        Cursor::Decl(d) => render_decl(w, file, d),
        Cursor::Use { .. } => render_use(w, file, pos, c, defs),
        Cursor::Path { .. } => render_path(w, file, pos, c, defs),
        Cursor::Member { name, .. } => {
            let Some(id) = name else {
                // the typing edge (`recv.`) — nothing to resolve yet
                return None;
            };
            render_member_or_name(w, file, pos, *id, defs)
        }
        Cursor::Name { id, .. } => render_name(w, file, pos, *id, defs),
        Cursor::Selfy { .. } | Cursor::Nothing => None,
    };
    markdown.map(|markdown| HoverOut { markdown, span })
}

/// the decl-site rendering — keyed on the cursor's `Decl` (the
/// position tests live in the classification): the owning type read
/// off `ast.parent` (tree-local, no world sweep), the mod edge's
/// mounted child, a binding row's markdown
fn render_decl(w: &World, file: FileId, d: &Def) -> Option<String> {
    let Def::Node { node, .. } = d else {
        // a binding row's own name — the declared-or-inferred type
        let Def::Binding { file: bf, row } = d else {
            return None;
        };
        let b = &w.file(*bf).doc.bindings()[*row];
        let ty = binding_ty(w, file, b);
        let name = w.file(file).doc.ast().name(b.name);
        return Some(binding_markdown(b, name, ty));
    };
    let di = w.file(file);
    let ast = di.doc.ast();
    if node.is::<FieldDeclNode>() {
        // the owning type: the decl's parent — a type body node
        let parent = ast.parent(*node)?;
        let t = TyView {
            file: di,
            h: parent,
        };
        return Some(match t.form() {
            // `Color::Red`'s decl shows the enum, matching the use site
            TyForm::Enum => render_ty(&t),
            _ => render_member(
                &crate::world::views::MemberView {
                    file: di,
                    ty: t,
                    h: *node,
                },
                None,
            ),
        });
    }
    if node.is::<EnumMemberNode>() {
        let parent = ast.parent(*node)?;
        let t = TyView {
            file: di,
            h: parent,
        };
        return Some(render_ty(&t));
    }
    if node.is::<ModuleLetNode>() {
        let l = crate::world::views::LetView { file: di, h: *node };
        return Some(render_let(w, &l));
    }
    if let Some(mh) = node.downcast::<ModDeclNode>() {
        let md_node = mh.get(ast);
        let m = ModView { file: di, h: *node };
        let child = rut_lir::check::child_path(
            crate::resolve::mods::mod_path_of(di),
            ast.name(md_node.name.ident),
        );
        let mounted = mounted_child(w, file, &child);
        return Some(render_mod(
            di,
            &m,
            &crate::resolve::mods::display_of(w, di.stamp.module.as_deref(), &child),
            mounted.map(|c| w.file(c).mods().len()),
        ));
    }
    // every other decl's own name — a fn, an impl method, a type —
    // IS the declaration (the decl-site law the walk rides: exact,
    // the doc's own row, never the file scope's layers — a file the
    // one-name law refuses still hovers the decl under the cursor)
    if let Some(f) = di.fns().into_iter().find(|f| f.h == *node) {
        return Some(render_fn_hits(&f));
    }
    if let Some(t) = di.types().into_iter().find(|t| t.h == *node) {
        return Some(render_ty(&t));
    }
    None
}

/// the use-edge rendering — a path segment renders the module it
/// walks to (kind, mounted file, child count); the pkg head the root.
/// A leaf renders like the name lanes (a use leaf resolves through
/// the world like any use site), except a leaf that names a MODULE —
/// that renders the module like a path segment does.
fn render_use(w: &World, file: FileId, pos: u32, c: &Cursor, defs: &[Def]) -> Option<String> {
    let Cursor::Use { node, at, .. } = c else {
        return None;
    };
    let ast = w.file(file).doc.ast();
    match at {
        // the head and an interior segment render the module they walk
        // to — the head's answer is the walk's, never a private gate
        UseAt::Head | UseAt::Segment(_) => match defs {
            [Def::Module { file: m }] => Some(render_module_target(
                w,
                w.file(*m),
                crate::resolve::mods::mod_path_of(w.file(*m)),
            )),
            [Def::Node { file: pf, node }] => module_of_edge(w, file, *pf, *node)
                .map(|(target, path)| render_module_target(w, w.file(target), &path)),
            // a use the chain cannot walk: no hover, never wrong text
            _ => None,
        },
        UseAt::Leaf(i) => {
            let uh = node
                .downcast::<UseNode>()
                .expect("a use cursor rides a use node");
            let ud = uh.get(ast);
            let n = &ud.names[*i];
            let id = n.ident;
            if let [Def::Module { file: m }] = defs {
                return Some(render_module_target(
                    w,
                    w.file(*m),
                    crate::resolve::mods::mod_path_of(w.file(*m)),
                ));
            }
            // the bare lanes' answer for the leaf's name (today's
            // fall-through: a use leaf resolves through the name
            // lanes; the walk's own use answer locates definition)
            let bare = crate::resolve::walk::name_lane(w, file, id, n.span.lo);
            render_name(w, file, pos, id, &bare)
        }
    }
}

/// the module a `use` path's mounting edge resolves to — the edge's
/// child path, the queried file's own entry when it IS that module
/// (the own-scope law), else the world file stamped for it
fn module_of_edge(
    w: &World,
    asker: FileId,
    parent: FileId,
    edge: AnyHandle,
) -> Option<(FileId, String)> {
    let ast = w.file(parent).doc.ast();
    let mh = edge.downcast::<ModDeclNode>()?;
    let name = ast.name(mh.get(ast).name.ident);
    let child = rut_lir::check::child_path(crate::resolve::mods::mod_path_of(w.file(parent)), name);
    let target = if w.file(asker).stamp.mod_path.as_deref() == Some(child.as_str()) {
        asker
    } else {
        crate::resolve::mods::find_mod(w, w.file(parent).stamp.module.as_deref(), &child)?
    };
    Some((target, child))
}

/// the world file a mod edge's child resolves to — the (module,
/// mod_path) pair when the owner is named; an unnamed owner (an open
/// workspace file) answers any file stamped at the child's mod path
/// (the dep walk's stamps — display only: the child count)
fn mounted_child(w: &World, owner: FileId, child: &str) -> Option<FileId> {
    match crate::resolve::mods::find_mod(w, w.file(owner).stamp.module.as_deref(), child) {
        Some(c) => Some(c),
        None if w.file(owner).stamp.module.is_none() => w
            .chain(owner)
            .find(|id| crate::resolve::mods::mod_path_of(w.file(*id)) == child),
        None => None,
    }
}

/// the qualified-path rendering — a mid-segment cursor renders the
/// module the walk lands at (a walk that fails keeps the pre-existing
/// bare-name fall-through on the ridden segment); a tail renders the
/// member target, else the module namespace lane's rows — types in a
/// type position; type → the single let → the fn laws in a value
/// position, the ctor section riding a value-position callee. An
/// unbound head answers nothing (authoritative — the bare lanes never
/// run on a qualified spelling's tail).
fn render_path(w: &World, file: FileId, pos: u32, c: &Cursor, defs: &[Def]) -> Option<String> {
    let Cursor::Path {
        node,
        segs,
        at,
        is_ty,
    } = c
    else {
        return None;
    };
    let ast = w.file(file).doc.ast();
    if at + 1 != segs.len() {
        // a mid-segment cursor — the module the walk lands at
        if let [Def::Module { file: m }] = defs {
            return Some(render_module_target(
                w,
                w.file(*m),
                crate::resolve::mods::mod_path_of(w.file(*m)),
            ));
        }
        if let [Def::Node { file: pf, node }] = defs {
            // the mounting edge renders through the module it mounts
            // (the pinned "file module" block)
            return module_of_edge(w, file, *pf, *node)
                .map(|(target, path)| render_module_target(w, w.file(target), &path));
        }
        // the walk failed: the bare lanes' answer renders like a name
        // position (today's fall-through)
        let (id, _) = cursor::path_seg(ast, *node, *at);
        return render_name(w, file, pos, id, defs);
    }
    // types-first (the lane's precedence), the ctor section riding a
    // value-position callee (`m::Logger(..)`)
    if let Some(t) = defs.iter().find_map(|def| match def.view(w) {
        Some(View::Ty(t)) => Some(t),
        _ => None,
    }) {
        let mut md = render_ty(&t);
        if !*is_ty {
            append_ctor_section(w, file, pos, &t, &mut md);
        }
        return Some(md);
    }
    // the single let (the scope bound the name to exactly one)
    let lets: Vec<LetView> = defs
        .iter()
        .filter_map(|def| match def.view(w) {
            Some(View::Let(l)) => Some(l),
            _ => None,
        })
        .collect();
    if let [one] = lets.as_slice() {
        return Some(render_let(w, one));
    }
    match defs {
        [] => None,
        // a member target — the field, the enum member, the impl fn —
        // renders as its declaration
        [d] => render_def(w, d),
        _ => {
            // the fn laws — 0/1/many, the candidates list for many
            let fns: Vec<FnView> = defs
                .iter()
                .filter_map(|def| match def.view(w) {
                    Some(View::Fn(f)) => Some(f),
                    _ => None,
                })
                .collect();
            match fns.as_slice() {
                [] => None,
                [one] => Some(render_fn_hits(one)),
                many => Some(render_candidates(
                    &many
                        .iter()
                        .map(|f| Candidate::own(CandRow::Fn(*f)))
                        .collect::<Vec<_>>(),
                )),
            }
        }
    }
}

/// the member lane's rendering — a resolved member target renders as
/// its declaration (the interface's declared member through the
/// interface — the receiver's static type IS it; the inherent impl fn
/// as its signature — both are `MethodDeclNode`s, the owning type
/// decides); the unknown-receiver fall-through's rows render like the
/// name lanes (the more informative answer)
fn render_member_or_name(
    w: &World,
    file: FileId,
    pos: u32,
    id: IdentId,
    defs: &[Def],
) -> Option<String> {
    if let [d] = defs {
        let Def::Node { file: df, node } = d else {
            // the fall-through's binding row renders like a name
            // position's
            return render_name(w, file, pos, id, defs);
        };
        let di = w.file(*df);
        let ast = di.doc.ast();
        if node.is::<rut_ast::MethodDeclNode>() {
            // the member law's body-member shapes — the method's
            // parent TYPE BODY renders it (`in \`Circle\`` /
            // `declared in \`Greeter\``); an inherent impl fn renders
            // its signature. The parent kind is the discriminator:
            // the type-decl family vs the impl block.
            let in_type_body = ast.parent(*node).is_some_and(|p| {
                p.is::<rut_ast::ClassNode>()
                    || p.is::<rut_ast::StructNode>()
                    || p.is::<rut_ast::InterfaceNode>()
                    || p.is::<rut_ast::EnumNode>()
                    || p.is::<rut_ast::BuiltinTyNode>()
                    || p.is::<rut_ast::BuiltinPrimitiveNode>()
                    || p.is::<rut_ast::SurfaceStructNode>()
            });
            if let Some(parent) = ast.parent(*node).filter(|_| in_type_body) {
                let t = TyView {
                    file: di,
                    h: parent,
                };
                return Some(render_member(
                    &MemberView {
                        file: di,
                        ty: t,
                        h: *node,
                    },
                    None,
                ));
            }
            return Some(render_fn_hits(&FnView { file: di, h: *node }));
        }
        if matches!(
            d.view(w),
            Some(View::Member(_)) | Some(View::Mod(_)) | Some(View::Let(_))
        ) {
            return render_def(w, d);
        }
    }
    render_name(w, file, pos, id, defs)
}

/// the bare-name rendering — today's hover lanes over the walk's
/// answer, in the same order: the file scope's types (first wins, the
/// ctor section riding a class callee), the primitive blurbs, the
/// binding pass's row, the one-name candidates (provenance — rule A),
/// the bare rows over a multi-answer, the module let, the fns
fn render_name(w: &World, file: FileId, pos: u32, id: IdentId, defs: &[Def]) -> Option<String> {
    let d = w.file(file);
    let name = d.doc.ast().name(id);
    // the FILE SCOPE's answers (the one home — the flat doors
    // delegate to `file_scope`): a capitalized name resolves through
    // the queried file's own decls, its use edges, and the ambient
    // prelude — a name the file never binds answers nothing below,
    // never a foreign decl
    if is_cap(name) || crate::doc::is_primitive_ty(name) {
        // the file scope's first type (the ONE home — a twice-bound
        // name's scope is empty: the candidates answer below, with
        // their provenance)
        if let Some(ty) = crate::resolve::type_decl(w, file, name) {
            let mut md = render_ty(&ty);
            append_ctor_section(w, file, pos, &ty, &mut md);
            return Some(md);
        }
        // M7 — the numeric/bool primitives have no surface decl to
        // index; the static blurb is the answer
        if let Some(md) = render_primitive(name) {
            return Some(md);
        }
    }
    // identifier hover — the binding layer's shadow-correct resolve
    // (params, lets, loop variables; use sites and decl sites alike —
    // the decl sites ride Cursor::Decl)
    if let [Def::Binding { file: bf, row }] = defs {
        let b = &w.file(*bf).doc.bindings()[*row];
        let ty = binding_ty(w, file, b);
        let name_text = d.doc.ast().name(b.name);
        return Some(binding_markdown(b, name_text, ty));
    }
    // THE mid-edit candidates: the file binds the name TWICE by
    // distinct bindings — two decls of its own, two use leaves, a
    // decl beside a leaf — a one-name violation the checker refuses;
    // the hover LISTS every binding
    // (kind + provenance: which decl, which `use` leaf and its
    // source), never a pick. A bare `Def` carries none of
    // that provenance, so the peek reads the rows (rule A). (A local
    // binding answered above — inner layer wins, legal shadowing.)
    if let Some(cands) = ambiguous_scope(w, file, name) {
        return Some(render_candidates(&cands));
    }
    // several defs without a refusal — the file scope's own
    // multi-answer (two files' fns behind one leaf): the bare rows
    if defs.len() > 1 {
        return Some(candidates(w, defs));
    }
    // module-let use sites — the file scope's unique match — then the
    // bare fn (free fns, methods; the scope bound the name to more
    // than one of its own only under a refusal, answered above)
    if let [def] = defs {
        match def.view(w) {
            Some(View::Let(l)) => return Some(render_let(w, &l)),
            Some(View::Fn(f)) => return Some(render_fn_hits(&f)),
            _ => {}
        }
    }
    None
}

/// one resolved declaration's markdown — the shared renderer behind
/// the Decl and member lanes (a def's own verbatim slice, its kind,
/// provenance, doc lines)
fn render_def(w: &World, d: &Def) -> Option<String> {
    let view = d.view(w)?;
    Some(match view {
        View::Ty(t) => render_ty(&t),
        View::Fn(f) => render_fn_hits(&f),
        View::Let(l) => render_let(w, &l),
        View::Member(m) => {
            // the owning type decides the shape (the enum block for a
            // variant — matching the use site)
            match m.ty.form() {
                TyForm::Enum => render_ty(&m.ty),
                _ => render_member(&m, None),
            }
        }
        View::Mod(_) => return None,
    })
}

/// the bare candidate rows over the walk's multi-answer — one bullet
/// per def: the verbatim decl, its kind, the owning file's provenance
/// (the provenance reading — which `use` bound each — is
/// [`ambiguous_scope`]'s, rule A)
pub(crate) fn candidates(w: &World, defs: &[Def]) -> String {
    let rows: Vec<Candidate> = defs
        .iter()
        .filter_map(|def| match def.view(w) {
            Some(View::Ty(t)) => Some(Candidate::own(CandRow::Ty(t))),
            Some(View::Let(l)) => Some(Candidate::own(CandRow::Let(l))),
            Some(View::Fn(f)) => Some(Candidate::own(CandRow::Fn(f))),
            Some(View::Mod(m)) => Some(Candidate::own(CandRow::Mod(m))),
            _ => None,
        })
        .collect();
    render_candidates(&rows)
}

/// is the cursor's node the CALLEE of its parent call? The identity
/// discriminator: the node must be the very path node the parent
/// [`rut_ast::CallNode`]'s `callee` slot records — an ARGUMENT path
/// shares the same parent CallNode and never matches
/// (`AnyHandle: PartialEq` is the slot's identity), a type slot rides
/// a `TyPathNode` (a different kind, never this parent), and a
/// `a::b(..)` tail is the whole path node that IS the callee.
fn ctor_callee_position(ast: &rut_ast::Ast, h: AnyHandle) -> bool {
    let Some(parent) = ast.parent(h) else {
        return false;
    };
    let Some(cd) = parent.downcast::<rut_ast::CallNode>() else {
        return false;
    };
    matches!(cd.get(ast).callee, rut_ast::ExprRef::Path(ph) if ph.any() == h)
}

/// the class callee's constructor section — the append's ONE home
/// (the class block stays primary; the section rides after
/// `render_ty`'s output). A class callee answers with the designated
/// ctor's verbatim impl slice; a NEWTYPE answers through the mint arm
/// — never through the marker sweep (the engine's `Ctx::class_ctors`
/// law: a newtype never registers). Every other shape — a non-class,
/// a no-ctor class, a cursor that is not a callee (type position,
/// decl site, argument path, `Logger::new` tail) — appends nothing.
/// The discriminator asks the caret's node — the same probe the
/// classification rode, where the old hover asked it.
fn append_ctor_section(w: &World, file: FileId, pos: u32, ty: &TyView, out: &mut String) {
    let d = w.file(file);
    let ast = d.doc.ast();
    let Some(h) = crate::doc::node_at_caret(&d.doc, pos) else {
        return;
    };
    if !ctor_callee_position(ast, h) || ty.form() != TyForm::Class {
        return;
    }
    if ty.is_newtype() {
        if let Some(mint) = render_newtype_mint(ty) {
            out.push_str("\n\n");
            out.push_str(&mint);
        }
        return;
    }
    if let Some(ctor) = crate::resolve::constructor_of(*ty) {
        out.push_str("\n\n");
        out.push_str(&render_ctor_section(&ctor));
    }
}
