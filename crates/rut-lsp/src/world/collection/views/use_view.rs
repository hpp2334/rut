//! [`UseView`] — one imported name of a use edge (a brace list's names
//! share the node; each is its own row).

use rut_ast::{AnyHandle, UseNode, Vis};

use crate::world::collection::MountedFile;

/// one imported name of a use edge (a brace list's names share the
/// node; each is its own row)
#[derive(Clone, Copy)]
pub(crate) struct UseView<'a> {
    pub(crate) file: &'a MountedFile,
    pub(crate) h: AnyHandle,
    /// the name's index within the use node's names
    pub(crate) i: usize,
}

impl<'a> UseView<'a> {
    fn d(&self) -> rut_ast::Handle<UseNode> {
        self.h.downcast::<UseNode>().expect("a use view")
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.file.doc.ast();
        let d = self.d().get(ast);
        ast.name(d.names[self.i])
    }

    pub(crate) fn vis(&self) -> Vis {
        self.d().get(self.file.doc.ast()).vis()
    }

    pub(crate) fn pkg(&self) -> &'a str {
        let ast = self.file.doc.ast();
        let d = self.d().get(ast);
        ast.name(d.path[0])
    }

    pub(crate) fn path(&self) -> Vec<String> {
        let ast = self.file.doc.ast();
        self.d()
            .get(ast)
            .path
            .iter()
            .map(|id| ast.name(*id).to_string())
            .collect()
    }
}
