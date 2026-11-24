//! [`LetView`] — a module-scope `let` view.

use rut_ast::{AnyHandle, Ast, ExprRef, ModuleLetNode, Vis};

use super::doc_lines;
use crate::world::ty::Ty;
use crate::world::File;

/// a module-scope `let` view
#[derive(Clone, Copy)]
pub(crate) struct LetView<'a> {
    pub(crate) file: &'a File,
    pub(crate) h: AnyHandle,
}

impl<'a> LetView<'a> {
    fn ast(&self) -> &'a Ast {
        self.file.doc.ast()
    }

    pub(crate) fn name(&self) -> &'a str {
        self.ast().name(
            self.h
                .downcast::<ModuleLetNode>()
                .expect("a let view")
                .get(self.ast())
                .name
                .ident,
        )
    }

    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        Some(
            self.h
                .downcast::<ModuleLetNode>()
                .expect("a let view")
                .get(self.ast())
                .name
                .span,
        )
    }

    /// the declared annotation, as written — the WRITTEN `ty` when the
    /// decl carries one. The inferred-initializer arm lives one layer
    /// down, in the resolver (`resolve::module_let_ty`) — the
    /// view names no layer above `world`.
    pub(crate) fn annotated(&self) -> Option<Ty> {
        let ast = self.ast();
        let d = self
            .h
            .downcast::<ModuleLetNode>()
            .expect("a let view")
            .get(ast);
        d.ty.map(|t| Ty::of(ast, t))
    }

    /// the initializer expression, as written (the resolver prices the
    /// annotation-less arm against it)
    pub(crate) fn init(&self) -> ExprRef {
        self.h
            .downcast::<ModuleLetNode>()
            .expect("a let view")
            .get(self.ast())
            .init
    }

    /// verbatim decl slice, through the initializer (the decl's own
    /// `;` — the exact-span law), trimmed of trailing space only
    pub(crate) fn src(&self) -> String {
        let span = self.span();
        let src = self.file.doc.src();
        src[span.lo as usize..span.hi as usize]
            .trim_end()
            .to_string()
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let d = self
            .h
            .downcast::<ModuleLetNode>()
            .expect("a let view")
            .get(self.ast());
        doc_lines(self.file.doc.src(), &d.doc)
    }

    /// the declared visibility — the position-path completion's tier
    /// gate reads it (`pub(pkg)`/`pub(super)` are not `pub`)
    pub(crate) fn vis(&self) -> Vis {
        self.h
            .downcast::<ModuleLetNode>()
            .expect("a let view")
            .get(self.ast())
            .vis()
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.ast().span_of(self.h)
    }

    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}
