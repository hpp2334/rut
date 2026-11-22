//! [`MemberView`] — a field, a method, or an enum member of a
//! [`TyView`]'s body.

use rut_ast::{AnyHandle, Ast, EnumMemberNode, FieldDeclNode, MethodDeclNode};

use super::{decl_src, doc_lines, sig_src, TyView};
use crate::world::collection::MountedFile;
use crate::world::types::ty_src;

/// a member view — a field, a method, or an enum member of
/// [`TyView`]'s body
#[derive(Clone, Copy)]
pub(crate) struct MemberView<'a> {
    pub(crate) file: &'a MountedFile,
    /// the owning type decl
    pub(crate) ty: TyView<'a>,
    pub(crate) h: AnyHandle,
}

impl<'a> MemberView<'a> {
    fn ast(&self) -> &'a Ast {
        self.file.doc.ast()
    }

    fn text(&self) -> &'a str {
        self.file.doc.src()
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.ast();
        macro_rules! nm {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return ast.name(d.get(ast).name.ident);
                }
            };
        }
        nm!(FieldDeclNode);
        nm!(MethodDeclNode);
        nm!(EnumMemberNode);
        unreachable!("a member view addresses a member node")
    }

    /// verbatim source of the declaration (signature for methods) —
    /// modifiers included: the decl span starts at the member's first
    /// token
    pub(crate) fn src(&self) -> String {
        let ast = self.ast();
        let src = self.text();
        if let Some(d) = self.h.downcast::<FieldDeclNode>() {
            let d = d.get(ast);
            return decl_src(src, d.span);
        }
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            let d = d.get(ast);
            return sig_src(src, ast, d.span, d.body);
        }
        if let Some(d) = self.h.downcast::<EnumMemberNode>() {
            let d = d.get(ast);
            return d.span.text(src).to_string();
        }
        unreachable!("a member view addresses a member node")
    }

    /// the declared type as written — fields: the type; methods: the
    /// return (`None` = unwritten, i.e. `nil`-returning); enum members
    /// carry none
    pub(crate) fn ty(&self) -> Option<String> {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<FieldDeclNode>() {
            return Some(ty_src(ast, d.get(ast).ty));
        }
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            return d.get(ast).ret.map(|r| ty_src(ast, r));
        }
        None
    }

    /// the `static` verdict, derived from the field's modifier nodes —
    /// the `::` completion tier's gate (methods are never static)
    pub(crate) fn is_static(&self) -> bool {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<FieldDeclNode>() {
            return d.get(ast).is_static();
        }
        false
    }

    /// byte span of the name identifier — the field / enum-member
    /// decl-site hover and the definition lookup key on it. The
    /// synthesized newtype's `inner` records no name token (the span
    /// degenerates empty) — the field's own extent stands in.
    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<FieldDeclNode>() {
            let d = d.get(ast);
            return Some(if d.name.span.hi > d.name.span.lo {
                d.name.span
            } else {
                d.span
            });
        }
        macro_rules! ns {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return Some(d.get(ast).name.span);
                }
            };
        }
        ns!(MethodDeclNode);
        ns!(EnumMemberNode);
        unreachable!("a member view addresses a member node")
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
        dd!(FieldDeclNode);
        dd!(MethodDeclNode);
        dd!(EnumMemberNode);
        unreachable!("a member view addresses a member node")
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.ast().span_of(self.h)
    }

    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}
