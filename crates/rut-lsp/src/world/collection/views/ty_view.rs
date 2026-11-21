//! [`TyView`] — a type decl view (class/struct/interface/enum/
//! builtin/primitive/host-struct/alias), plus the member-list bridges
//! (`fields_of`/`methods_of`) that fan its child lists into
//! [`MemberView`] rows.

use rut_ast::{
    AliasNode, AnyHandle, Ast, BuiltinPrimitiveNode, BuiltinTyNode, ClassNode, EnumNode, FieldRef,
    InterfaceNode, MemberRef, MethodRef, StructNode, SurfaceStructNode, Vis,
};

use super::{decl_src, doc_lines, head_src, MemberView};
use crate::world::collection::MountedFile;
use crate::world::types::TyForm;

/// a type decl view — every fact derives from the node on read: the
/// head slice joins over the file's own text, the vis/ambient verdicts
/// ride the modifier chain, the members walk the node's child lists.
#[derive(Clone, Copy)]
pub(crate) struct TyView<'a> {
    pub(crate) file: &'a MountedFile,
    pub(crate) h: AnyHandle,
}

impl<'a> TyView<'a> {
    fn ast(&self) -> &'a Ast {
        self.file.doc.ast()
    }

    fn text(&self) -> &'a str {
        self.file.doc.src()
    }

    /// the declaring ident's span (the definition target)
    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        Some(self.name_span_raw())
    }

    /// the node's own recorded name span (the degenerate-empty span
    /// included — fields fall back to their own extent)
    fn name_span_raw(&self) -> rut_lexer::span::Span {
        let ast = self.ast();
        macro_rules! ns {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return d.get(ast).name_span;
                }
            };
        }
        ns!(ClassNode);
        ns!(StructNode);
        ns!(InterfaceNode);
        ns!(EnumNode);
        ns!(BuiltinTyNode);
        ns!(BuiltinPrimitiveNode);
        ns!(SurfaceStructNode);
        ns!(AliasNode);
        unreachable!("a type view addresses a type decl node")
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.ast();
        macro_rules! nm {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return ast.name(d.get(ast).name);
                }
            };
        }
        nm!(ClassNode);
        nm!(StructNode);
        nm!(InterfaceNode);
        nm!(EnumNode);
        nm!(BuiltinTyNode);
        nm!(BuiltinPrimitiveNode);
        nm!(SurfaceStructNode);
        nm!(AliasNode);
        unreachable!("a type view addresses a type decl node")
    }

    pub(crate) fn form(&self) -> TyForm {
        macro_rules! f {
            ($t:ty, $v:expr) => {
                if self.h.is::<$t>() {
                    return $v;
                }
            };
        }
        f!(ClassNode, TyForm::Class);
        f!(StructNode, TyForm::Struct);
        f!(InterfaceNode, TyForm::Interface);
        f!(EnumNode, TyForm::Enum);
        f!(BuiltinTyNode, TyForm::Builtin);
        f!(BuiltinPrimitiveNode, TyForm::Primitive);
        f!(SurfaceStructNode, TyForm::HostStruct);
        f!(AliasNode, TyForm::Alias);
        unreachable!("a type view addresses a type decl node")
    }

    /// the decl head as WRITTEN — the verbatim `span × src` slice from
    /// the item's first token cut at the body's `{`; an alias slices
    /// through its own terminator
    pub(crate) fn head(&self) -> String {
        let ast = self.ast();
        let src = self.text();
        macro_rules! hs {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    let d = d.get(ast);
                    return head_src(src, d.span, d.body_span);
                }
            };
        }
        hs!(ClassNode);
        hs!(StructNode);
        hs!(InterfaceNode);
        hs!(EnumNode);
        hs!(BuiltinTyNode);
        hs!(BuiltinPrimitiveNode);
        hs!(SurfaceStructNode);
        // `type X = A;` — the whole decl is the verbatim slice (target
        // as written, terminator trimmed); the head IS the decl text
        if let Some(d) = self.h.downcast::<AliasNode>() {
            return decl_src(src, d.get(ast).span);
        }
        unreachable!("a type view addresses a type decl node")
    }

    /// the declared visibility — the completion/hover tier gate reads
    /// it (`pub(pkg)`/`pub(super)` are not `pub`). The engine surfaces
    /// (`builtin`/`builtin primitive`/`host struct`) render `pub`: the
    /// row-era shape — their gate is the ambient split, never the vis.
    pub(crate) fn vis(&self) -> Vis {
        let ast = self.ast();
        macro_rules! v {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return d.get(ast).vis();
                }
            };
        }
        v!(ClassNode);
        v!(StructNode);
        v!(InterfaceNode);
        v!(EnumNode);
        v!(AliasNode);
        if self.h.is::<BuiltinTyNode>()
            || self.h.is::<BuiltinPrimitiveNode>()
            || self.h.is::<SurfaceStructNode>()
        {
            return Vis::Pub;
        }
        unreachable!("a type view addresses a type decl node")
    }

    /// `pub` on the declaration — the completion tiers' names face
    /// offers pub rows only, while the INDEX itself stays unfiltered
    pub(crate) fn is_pub(&self) -> bool {
        self.vis() == Vis::Pub
    }

    /// the ambient builtin linkage (`prelude builtin`) — the name
    /// binds in every compilation unit with no `use`. DERIVED from the
    /// modifier chain at read.
    pub(crate) fn ambient(&self) -> bool {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<BuiltinTyNode>() {
            return d.get(ast).ambient();
        }
        if let Some(d) = self.h.downcast::<BuiltinPrimitiveNode>() {
            return d.get(ast).ambient();
        }
        false
    }

    pub(crate) fn fields(&self) -> Vec<MemberView<'a>> {
        let ast = self.ast();
        let mut out = Vec::new();
        macro_rules! flds {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    out.extend(fields_of(self, &d.get(ast).fields));
                }
            };
        }
        flds!(ClassNode);
        flds!(StructNode);
        flds!(SurfaceStructNode);
        // an enum's members ride the fields list (the row-era shape:
        // `Color::Red` resolves through the member list)
        if let Some(d) = self.h.downcast::<EnumNode>() {
            for m in &d.get(ast).members {
                let MemberRef::Member(mh) = m else {
                    continue; // the recovery slot renders no row
                };
                out.push(MemberView {
                    file: self.file,
                    ty: *self,
                    h: mh.any(),
                });
            }
        }
        out
    }

    pub(crate) fn methods(&self) -> Vec<MemberView<'a>> {
        let ast = self.ast();
        let mut out = Vec::new();
        macro_rules! mths {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    out.extend(methods_of(self, &d.get(ast).methods));
                }
            };
        }
        mths!(ClassNode);
        mths!(StructNode);
        mths!(InterfaceNode);
        // the engine surfaces carry their member list under `members`
        if let Some(d) = self.h.downcast::<BuiltinTyNode>() {
            out.extend(methods_of(self, &d.get(ast).members));
        } else if let Some(d) = self.h.downcast::<BuiltinPrimitiveNode>() {
            out.extend(methods_of(self, &d.get(ast).members));
        }
        out
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let ast = self.ast();
        macro_rules! dd {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return doc_lines(self.text(), &d.get(ast).doc);
                }
            };
        }
        dd!(ClassNode);
        dd!(StructNode);
        dd!(InterfaceNode);
        dd!(EnumNode);
        dd!(BuiltinTyNode);
        dd!(BuiltinPrimitiveNode);
        dd!(SurfaceStructNode);
        dd!(AliasNode);
        unreachable!("a type view addresses a type decl node")
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.ast().span_of(self.h)
    }

    /// 1-based line of the declaration — the doc's own line geometry
    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}

fn fields_of<'a>(ty: &TyView<'a>, fields: &[FieldRef]) -> Vec<MemberView<'a>> {
    fields
        .iter()
        .filter_map(|f| {
            let FieldRef::Field(h) = f else {
                return None; // the recovery slot renders no row
            };
            Some(MemberView {
                file: ty.file,
                ty: *ty,
                h: h.any(),
            })
        })
        .collect()
}

fn methods_of<'a>(ty: &TyView<'a>, methods: &[MethodRef]) -> Vec<MemberView<'a>> {
    methods
        .iter()
        .filter_map(|m| {
            let MethodRef::Method(h) = m else {
                return None; // the recovery slot renders no row
            };
            Some(MemberView {
                file: ty.file,
                ty: *ty,
                h: h.any(),
            })
        })
        .collect()
}
