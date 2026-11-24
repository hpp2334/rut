//! [`UseView`] — one imported name of a use edge (a brace list's names
//! share the node; each is its own row).

use rut_ast::{AnyHandle, UseNode, Vis};

use super::decl_src;
use crate::world::File;

/// one imported name of a use edge (a brace list's names share the
/// node; each is its own row)
#[derive(Clone, Copy)]
pub(crate) struct UseView<'a> {
    pub(crate) file: &'a File,
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
        ast.name(d.names[self.i].ident)
    }

    pub(crate) fn vis(&self) -> Vis {
        self.d().get(self.file.doc.ast()).vis()
    }

    pub(crate) fn pkg(&self) -> &'a str {
        let ast = self.file.doc.ast();
        let d = self.d().get(ast);
        ast.name(d.path[0].ident)
    }

    pub(crate) fn path(&self) -> Vec<String> {
        let ast = self.file.doc.ast();
        self.d()
            .get(ast)
            .path
            .iter()
            .map(|n| ast.name(n.ident).to_string())
            .collect()
    }

    /// the use statement as spelled — the verbatim `span × src` slice,
    /// trailing `;` trimmed: a candidate's provenance (which `use` a
    /// row rode)
    pub(crate) fn src(&self) -> String {
        let ast = self.file.doc.ast();
        decl_src(self.file.doc.src(), ast.span_of(self.h))
    }

    /// the statement's line in its own file
    pub(crate) fn line(&self) -> u32 {
        1 + self
            .file
            .doc
            .line_of(self.file.doc.ast().span_of(self.h).lo)
    }
}

/// one glob leaf — `use pkg::*` (the brace star rides beside the
/// names too: `use pkg::{ A, * }` fans the named rows above AND this
/// row). A glob fans out NO name rows in [`File::uses`] — the
/// star is never offered — so the glob lanes (the import expansion in
/// `import_map`, the re-export arm in the use tier) read these rows.
#[derive(Clone)]
pub(crate) struct GlobUse<'a> {
    /// the walked path's head — the pkg the use spells
    pub(crate) pkg: &'a str,
    /// the interior segments between the head and the star
    /// (`use pkg::a::b::*` → `["a", "b"]`)
    pub(crate) interior: Vec<String>,
    /// the declared visibility — a `pub use` glob re-exports
    pub(crate) vis: Vis,
}

impl GlobUse<'_> {
    /// the glob as spelled — `m::*`, `calc::math::*` (a candidate's
    /// provenance; the brace form `use m::{ A, * }` reads `m::*`)
    pub(crate) fn spelling(&self) -> String {
        let mut out = String::from(self.pkg);
        for seg in &self.interior {
            out.push_str("::");
            out.push_str(seg);
        }
        out.push_str("::*");
        out
    }
}
