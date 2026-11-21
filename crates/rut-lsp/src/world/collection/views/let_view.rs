//! [`LetView`] — a module-scope `let` view.

use rut_ast::{AnyHandle, Ast, ModuleLetNode, Vis};

use super::doc_lines;
use crate::world::collection::MountedFile;
use crate::world::infer;
use crate::world::types::ty_src;

/// a module-scope `let` view
#[derive(Clone, Copy)]
pub(crate) struct LetView<'a> {
    pub(crate) file: &'a MountedFile,
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
                .name,
        )
    }

    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        Some(
            self.h
                .downcast::<ModuleLetNode>()
                .expect("a let view")
                .get(self.ast())
                .name_span,
        )
    }

    /// the declared annotation as written, else the initializer-derived
    /// type (the same display-side heuristic the binding pass uses,
    /// priced against THIS file's own decls)
    pub(crate) fn ty(&self) -> Option<String> {
        let ast = self.ast();
        let d = self
            .h
            .downcast::<ModuleLetNode>()
            .expect("a let view")
            .get(ast);
        match d.ty {
            Some(t) => Some(ty_src(ast, t)),
            None => infer::expr_ty(self.file, &[self.file], d.init),
        }
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
