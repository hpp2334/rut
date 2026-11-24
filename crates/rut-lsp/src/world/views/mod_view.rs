//! [`ModView`] — a `mod NAME;` namespace edge view.

use rut_ast::{AnyHandle, ModDeclNode, Vis};

use super::{decl_src, doc_lines};
use crate::world::File;

/// a `mod NAME;` namespace edge view
#[derive(Clone, Copy)]
pub(crate) struct ModView<'a> {
    pub(crate) file: &'a File,
    pub(crate) h: AnyHandle,
}

impl<'a> ModView<'a> {
    fn d(&self) -> rut_ast::Handle<ModDeclNode> {
        self.h.downcast::<ModDeclNode>().expect("a mod view")
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.file.doc.ast();
        ast.name(self.d().get(ast).name.ident)
    }

    /// the edge's declared visibility (`Vis::Self_` = bare `mod`)
    pub(crate) fn vis(&self) -> Vis {
        self.d().get(self.file.doc.ast()).vis()
    }

    /// the edge as spelled — the verbatim `span × src` slice (span law
    /// v3.1 starts the item at its first modifier), trailing `;`
    /// trimmed: the render never re-synthesizes `pub mod NAME;`
    pub(crate) fn src(&self) -> String {
        decl_src(self.file.doc.src(), self.span())
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let d = self.d().get(self.file.doc.ast());
        doc_lines(self.file.doc.src(), &d.doc)
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.file.doc.ast().span_of(self.h)
    }

    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}
