//! Building the index — one pass over a document's module items,
//! recovering verbatim signatures and doc comments from the source.
//! The decl layer (the lsp-features survey §3.2): fields, enum
//! members, module lets, and use-imported names all carry their name
//! spans — the decl nodes RECORD them now (`name_span`, v2's parse
//! verdict; the token-scan recovery is gone) — and docs ride the
//! nodes themselves (`doc: DocData`, grafted at allocation when the
//! parse ran with `include_comments`), so decl-site hover and the
//! definition layer are span-exact with no side tables.
//!
//! Item spans are EXACT (v2's own-terminator law): a bodied decl ends
//! at its body's `}`, a bodiless one at its own `;` — so the verbatim
//! signature is a plain slice of `span × src`, cut at the body per the
//! node's shape. Error slots (items and members alike) render no rows:
//! errors are the panel's business (`ast.errors()`), not the index's.

use rut_ast::{
    Ast, BlockNode, DocData, EnumMemberNode, FieldDeclNode, FieldRef, Handle, IdentId, ItemRef,
    MethodDeclNode, MethodRef, ModuleLetNode, ParamRef, Vis,
};
use rut_semantic::LineIndex;
use rut_semantic::Span;
use rut_semantic::Tok;
use rut_semantic::Token;

use super::infer;
use super::types::{
    ty_head, ty_src, DefIndex, FnDef, ImplDef, LetDef, MemberSrc, ModDef, TyDef, TyForm, UseDef,
};

/// A `let` awaiting its type resolution: the node + its span (the
/// node's own `name_span`/`doc` fields are the row's truth).
type PendingLet = (Handle<ModuleLetNode>, Span);

/// `line` of a byte offset, 1-based
fn line_of(src: &str, lo: u32) -> u32 {
    1 + src[..lo as usize].matches('\n').count() as u32
}

/// the doc chain a decl node carries, as rendered lines — the parse's
/// own graft (D7: the Go-convention rules, byte-identical to the
/// comment-table scan this replaces). Empty when the parse ran without
/// `include_comments` or nothing attaches
fn doc_of(d: &DocData, src: &str) -> Vec<String> {
    d.lines(src).map(|l| l.trim_start().to_string()).collect()
}

/// verbatim signature: the source up to (not including) the body; a
/// bodiless decl ends at its own `;` (the exact-span law) and the
/// trailing `;` is trimmed off the render
fn sig_src(src: &str, ast: &Ast, span: Span, body: Option<Handle<BlockNode>>) -> String {
    let hi = match body {
        Some(b) => b.span(ast).lo,
        None => span.hi, // the item's own terminator — the `;`
    };
    src[span.lo as usize..hi as usize]
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

fn member(
    src: &str,
    name: &str,
    sig: String,
    ty: Option<String>,
    span: Span,
    name_span: Span,
    doc: Vec<String>,
) -> MemberSrc {
    MemberSrc {
        name: name.to_string(),
        src: sig,
        ty,
        // only a field can be a declared static — the flag's truth rides
        // the row (the field lane overwrites it from the node)
        is_static: false,
        name_span: Some(name_span),
        doc,
        line: line_of(src, span.lo),
    }
}

fn members_of(src: &str, ast: &Ast, methods: &[MethodRef]) -> Vec<MemberSrc> {
    methods
        .iter()
        .filter_map(|m| {
            let MethodRef::Method(h) = m else {
                return None; // the recovery slot renders no row
            };
            Some(method_of(src, ast, *h))
        })
        .collect()
}

/// one method's member row — the decl span starts at the member's
/// FIRST token (the leading modifier when one is spelled), so the
/// verbatim slice IS the whole signature: `pub async fn f(..)`.
/// Truthful to source, no flag re-rendering
fn method_of(src: &str, ast: &Ast, h: Handle<MethodDeclNode>) -> MemberSrc {
    let d = h.get(ast);
    let sp = d.span;
    let sig = sig_src(src, ast, sp, d.body);
    let name = ast.name(d.name);
    member(
        src,
        name,
        sig,
        d.ret.map(|r| ty_src(ast, r)),
        sp,
        d.name_span,
        doc_of(&d.doc, src),
    )
}

/// field member list: the decl slice carries the modifiers (the field
/// span starts at the member's first token) — sliced verbatim; the
/// `static` gate rides the row's own flag (derived from the modifier
/// nodes), never a string match on the render
fn field_members(src: &str, ast: &Ast, fields: &[FieldRef]) -> Vec<MemberSrc> {
    fields
        .iter()
        .filter_map(|f| {
            let FieldRef::Field(h) = f else {
                return None; // the recovery slot renders no row
            };
            let d: &FieldDeclNode = h.get(ast);
            let sp = d.span;
            let decl = src[sp.lo as usize..sp.hi as usize]
                .trim_end()
                .trim_end_matches(';')
                .trim_end()
                .to_string();
            let name = ast.name(d.name);
            let mut row = member(
                src,
                name,
                decl,
                Some(ty_src(ast, d.ty)),
                sp,
                d.name_span,
                doc_of(&d.doc, src),
            );
            row.is_static = d.is_static();
            Some(row)
        })
        .collect()
}

/// enum member list — the members are nodes since v2: their name
/// spans are the parse's own verdict and their doc chains ride the
/// node (the KNOWN DELTA: member docs render where the old scan could
/// not reach)
fn enum_members(src: &str, ast: &Ast, members: &[rut_ast::MemberRef]) -> Vec<MemberSrc> {
    let mut out = Vec::new();
    for m in members {
        let rut_ast::MemberRef::Member(h) = m else {
            continue; // the recovery slot renders no row
        };
        let d: &EnumMemberNode = h.get(ast);
        out.push(member(
            src,
            ast.name(d.name),
            ast.name(d.name).to_string(),
            None,
            d.span,
            d.name_span,
            doc_of(&d.doc, src),
        ));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn ty_def(
    src: &str,
    name: &str,
    name_span: Span,
    form: TyForm,
    vis: Vis,
    generics: Vec<String>,
    fields: Vec<MemberSrc>,
    methods: Vec<MemberSrc>,
    span: Span,
    doc: Vec<String>,
) -> TyDef {
    TyDef {
        name: name.to_string(),
        name_span: Some(name_span),
        form,
        vis,
        is_pub: vis == Vis::Pub,
        // plain decls are not ambient — the `prelude builtin` rows set
        // the flag after this constructor builds them
        ambient: false,
        generics,
        fields,
        methods,
        alias_target: None,
        doc,
        span,
        line: line_of(src, span.lo),
    }
}

fn generics_of(ast: &Ast, gs: &[IdentId]) -> Vec<String> {
    gs.iter().map(|&g| ast.name(g).to_string()).collect()
}

/// declared parameter names in order, `self` excluded — the AST's own
/// data, recorded once at index time; the inlay hints' call-site
/// parameter names price exact-arity matches against this
fn param_names(ast: &Ast, params: &[ParamRef]) -> Vec<String> {
    params
        .iter()
        .filter_map(|p| match p {
            ParamRef::Param(d) => Some(ast.name(d.get(ast).name).to_string()),
            _ => None, // `self` / `mut self`
        })
        .collect()
}

/// Build the index for one document. `toks` is the same lex the parser
/// consumed — the use-path interior segments' name-span recovery rides
/// it (the only span the nodes do not record).
pub fn index(src: &str, ast: &Ast, toks: &[Token]) -> DefIndex {
    index_items(src, ast, toks, ast.root().items(ast))
}

/// [`index`] over a GIVEN item list — one module slice of a document.
/// The deps lane's declaration-entry splitter runs this once per nested
/// inline module of a consolidated `.d.rut` (one parse, per-module
/// indexes, each stamped with its mod path), so a bundle group's whole
/// surface indexes from ONE Decl parse of ONE file.
pub fn index_items(src: &str, ast: &Ast, toks: &[Token], items: &[ItemRef]) -> DefIndex {
    let mut idx = DefIndex::default();
    let mut pending_lets: Vec<PendingLet> = Vec::new();
    for item in items {
        let span = item.span(ast);
        match item {
            ItemRef::Class(h) => {
                let d = h.get(ast);
                let fs = field_members(src, ast, &d.fields);
                let ms = members_of(src, ast, &d.methods);
                idx.types.push(ty_def(
                    src,
                    ast.name(d.name),
                    d.name_span,
                    TyForm::Class,
                    d.vis,
                    generics_of(ast, &d.generics),
                    fs,
                    ms,
                    span,
                    doc_of(&d.doc, src),
                ));
            }
            ItemRef::Struct(h) => {
                let d = h.get(ast);
                let fs = field_members(src, ast, &d.fields);
                let ms = members_of(src, ast, &d.methods);
                idx.types.push(ty_def(
                    src,
                    ast.name(d.name),
                    d.name_span,
                    TyForm::Struct,
                    d.vis,
                    generics_of(ast, &d.generics),
                    fs,
                    ms,
                    span,
                    doc_of(&d.doc, src),
                ));
            }
            ItemRef::Interface(h) => {
                let d = h.get(ast);
                let ms = members_of(src, ast, &d.methods);
                idx.types.push(ty_def(
                    src,
                    ast.name(d.name),
                    d.name_span,
                    TyForm::Interface,
                    d.vis,
                    generics_of(ast, &d.generics),
                    Vec::new(),
                    ms,
                    span,
                    doc_of(&d.doc, src),
                ));
            }
            ItemRef::Enum(h) => {
                let d = h.get(ast);
                // the members are nodes — name spans and docs recorded
                let ms = enum_members(src, ast, &d.members);
                idx.types.push(ty_def(
                    src,
                    ast.name(d.name),
                    d.name_span,
                    TyForm::Enum,
                    d.vis,
                    Vec::new(),
                    ms,
                    Vec::new(),
                    span,
                    doc_of(&d.doc, src),
                ));
            }
            ItemRef::Impl(h) => {
                let d = h.get(ast);
                // the inherent impl owns its methods — the owner string
                // is the target type's name, so an impl method renders
                // like the type's own surface (`impl I for T` is gone)
                let target_head = ty_head(ast, d.target);
                idx.impls.push(ImplDef {
                    target_name: target_head.clone(),
                });
                for m in &d.methods {
                    let MethodRef::Method(mh) = m else {
                        continue; // the recovery slot renders no row
                    };
                    let md: &MethodDeclNode = mh.get(ast);
                    let sp = md.span;
                    let name = ast.name(md.name);
                    idx.fns.push(FnDef {
                        name: name.to_string(),
                        name_span: Some(md.name_span),
                        src: sig_src(src, ast, sp, md.body),
                        ret: md.ret.map(|r| ty_src(ast, r)),
                        params: param_names(ast, &md.params),
                        doc: doc_of(&md.doc, src),
                        vis: md.vis().unwrap_or(Vis::Pub),
                        ambient: false,
                        owner: Some(target_head.clone()),
                        span: sp,
                        line: line_of(src, sp.lo),
                    });
                }
            }
            ItemRef::Fn(h) => {
                let d = h.get(ast);
                idx.fns.push(FnDef {
                    name: ast.name(d.name).to_string(),
                    name_span: Some(d.name_span),
                    src: sig_src(src, ast, span, Some(d.body)),
                    ret: d.ret.map(|r| ty_src(ast, r)),
                    params: param_names(ast, &d.params),
                    doc: doc_of(&d.doc, src),
                    vis: d.vis,
                    ambient: false,
                    owner: None,
                    span,
                    line: line_of(src, span.lo),
                });
            }
            ItemRef::SurfaceFn(h) => {
                let d = h.get(ast);
                // the import-gated builtin linkage (`pub builtin`) — the
                // name completes only through the document's `use`
                if matches!(d.linkage, rut_ast::Linkage::Builtin { ambient: false }) {
                    idx.pub_gated.push(ast.name(d.name).to_string());
                }
                idx.fns.push(FnDef {
                    name: ast.name(d.name).to_string(),
                    name_span: Some(d.name_span),
                    src: sig_src(src, ast, span, None),
                    // surface rows never record a ret (the v1 shape —
                    // inference does not ride the host boundary)
                    ret: None,
                    params: param_names(ast, &d.params),
                    doc: doc_of(&d.doc, src),
                    vis: d.vis,
                    // the ambient spelling (`prelude builtin fn`) binds
                    // in every unit — completion offers it bare
                    ambient: matches!(d.linkage, rut_ast::Linkage::Builtin { ambient: true }),
                    owner: None,
                    span,
                    line: line_of(src, span.lo),
                });
            }
            ItemRef::BuiltinTy(h) => {
                let d = h.get(ast);
                if !d.ambient {
                    idx.pub_gated.push(ast.name(d.name).to_string());
                }
                let ms = members_of(src, ast, &d.members);
                let mut def = ty_def(
                    src,
                    ast.name(d.name),
                    d.name_span,
                    TyForm::Builtin,
                    Vis::Pub,
                    generics_of(ast, &d.generics),
                    Vec::new(),
                    ms,
                    span,
                    doc_of(&d.doc, src),
                );
                // `prelude builtin` — ambient, offered bare
                def.ambient = d.ambient;
                idx.types.push(def);
            }
            ItemRef::BuiltinPrimitive(h) => {
                let d = h.get(ast);
                if !d.ambient {
                    idx.pub_gated.push(ast.name(d.name).to_string());
                }
                let ms = members_of(src, ast, &d.members);
                let mut def = ty_def(
                    src,
                    ast.name(d.name),
                    d.name_span,
                    TyForm::Primitive,
                    Vis::Pub,
                    Vec::new(),
                    Vec::new(),
                    ms,
                    span,
                    doc_of(&d.doc, src),
                );
                def.ambient = d.ambient;
                idx.types.push(def);
            }
            ItemRef::SurfaceStruct(h) => {
                let d = h.get(ast);
                let fs = field_members(src, ast, &d.fields);
                idx.types.push(ty_def(
                    src,
                    ast.name(d.name),
                    d.name_span,
                    TyForm::HostStruct,
                    Vis::Pub,
                    Vec::new(),
                    fs,
                    Vec::new(),
                    span,
                    doc_of(&d.doc, src),
                ));
            }
            ItemRef::BuiltinImpl(_) => {
                // core's `builtin impl i32 { .. }` — no new type; the
                // methods surface through core's own hover data
            }
            ItemRef::Alias(h) => {
                let d = h.get(ast);
                // `type X = A;` / `type X = A | B;` — the
                // target renders as written
                let name = ast.name(d.name);
                idx.types.push(TyDef {
                    name: name.to_string(),
                    name_span: Some(d.name_span),
                    form: TyForm::Alias,
                    vis: d.vis,
                    is_pub: d.vis == Vis::Pub,
                    ambient: false,
                    generics: Vec::new(),
                    fields: Vec::new(),
                    methods: Vec::new(),
                    doc: doc_of(&d.doc, src),
                    span,
                    line: line_of(src, span.lo),
                    alias_target: Some(ty_src(ast, d.target)),
                });
            }
            ItemRef::ModuleLet(h) => {
                // processed after the main walk: the initializer's
                // inference may name THIS document's types
                pending_lets.push((*h, span));
            }
            ItemRef::Use(h) => {
                let d = h.get(ast);
                // `use pouch::{ Vec, Vec2 };` /
                // `use pouch::layout::{ Vec, Vec2 };` — the path idents
                // (package, then modules) match positionally first, then
                // the imported names in order. The leaf idents' spans
                // are recorded on the node; the interior path segments'
                // spans are the one recovery left (the token scan — the
                // node records only the package head's)
                let use_path: Vec<String> =
                    d.path.iter().map(|p| ast.name(*p).to_string()).collect();
                let mut path_spans: Vec<Option<Span>> = vec![None; d.path.len()];
                let mut segs_left = d.path.len();
                for t in toks {
                    if t.span.lo < span.lo || t.span.hi > span.hi {
                        continue;
                    }
                    let Tok::Ident(text) = &t.tok else { continue };
                    if rut_semantic::is_reserved_kw(text) {
                        continue;
                    }
                    if segs_left == 0 {
                        continue;
                    }
                    let at = d.path.len() - segs_left;
                    if *text == ast.name(d.path[at]) {
                        path_spans[at] = Some(t.span);
                        segs_left -= 1;
                    }
                }
                for (name_id, name_span) in d.names.iter().zip(d.name_spans.iter()) {
                    idx.uses.push(UseDef {
                        name: ast.name(*name_id).to_string(),
                        name_span: Some(*name_span),
                        vis: d.vis,
                        pkg: use_path[0].clone(),
                        path: use_path.clone(),
                        path_spans: path_spans.clone(),
                    });
                }
            }
            ItemRef::ModDecl(h) => {
                let d = h.get(ast);
                // `pub? mod NAME;` — the namespace edge. The row carries
                // the ident span (decl-site hover/definition target) and
                // the edge vis; the mounted `NAME/mod.rut` lives in its
                // own index, found through the chain by mod path
                idx.mods.push(ModDef {
                    name: ast.name(d.name).to_string(),
                    name_span: Some(d.name_span),
                    vis: d.vis,
                    doc: doc_of(&d.doc, src),
                    span,
                    line: line_of(src, span.lo),
                });
            }
            ItemRef::ModInline(h) => {
                let d = h.get(ast);
                // the declaration entry's inline module — the SAME
                // namespace edge; the nested declarations index into the
                // child module's own (mod-path-stamped) index, built by
                // the deps lane's one-parse splitter
                idx.mods.push(ModDef {
                    name: ast.name(d.name).to_string(),
                    name_span: Some(d.name_span),
                    vis: d.vis,
                    doc: doc_of(&d.doc, src),
                    span,
                    line: line_of(src, span.lo),
                });
            }
            // the root module never rides an items list; an Error slot
            // renders no rows (errors are the panel's business)
            ItemRef::Module(_) | ItemRef::Error(_) => {}
        }
    }
    // module lets — with the index's types/fns in place so a
    // `let c = Circle::new(..)` at module scope infers its type
    let lets = std::mem::take(&mut pending_lets);
    for (h, span) in lets {
        let d = h.get(ast);
        let text = ast.name(d.name).to_string();
        let ty_text = d.ty.map(|t| ty_src(ast, t)).or_else(|| {
            let done: [&DefIndex; 1] = [&idx];
            infer::expr_ty(ast, &[], &done, d.init)
        });
        // the verbatim slice runs through the decl's own `;` (the
        // exact-span law) — trimmed of trailing space only
        let src_text = src[span.lo as usize..span.hi as usize]
            .trim_end()
            .to_string();
        idx.lets.push(LetDef {
            name: text,
            name_span: Some(d.name_span),
            ty: ty_text,
            src: src_text,
            doc: doc_of(&d.doc, src),
            vis: d.vis,
            span,
            line: line_of(src, span.lo),
        });
    }
    // the definition layer's target text: cross-file and prelude jumps
    // convert their spans to LSP ranges against THIS index's own source
    // (the editor face cannot re-read files)
    idx.src = src.to_string();
    idx.lines = LineIndex::new(src);
    idx
}
