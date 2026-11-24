//! Lookup — the hover feature's entry. The one classification
//! ([`crate::resolve::cursor`]) answers what sits under the caret,
//! the one walk ([`crate::resolve::walk::resolve`]) resolves it to
//! `Def`s, and the render half ([`super::render`]) renders the
//! markdown. The `Self` spellings render here: the type slot and the
//! receiver name the enclosing type (HOME-scoped — the only file
//! whose spans index the cursor's offsets); the type spelling in a
//! value slot names no value.

use super::render::render_ty;
use crate::doc::Span;
use crate::resolve::cursor::{self, Cursor};
use crate::resolve::enclosing_type;
use crate::world::{FileId, World};

/// The hover result: markdown body + the span it decorates.
pub struct HoverOut {
    pub markdown: String,
    pub span: Span,
}

/// The world is the answer: the queried file first (its own entry —
/// an open document's buffer is the truth), then the ambient prelude,
/// then the rest of the workspace.
pub fn hover(w: &World, file: FileId, pos: u32) -> Option<HoverOut> {
    let c = cursor::cursor(w, file, pos);
    match &c {
        Cursor::Nothing => None,
        Cursor::Selfy { span, .. } => {
            // the Self spellings — the type slot and the receiver name
            // the enclosing type; the type spelling in a value slot
            // names no value (today's law, keyed on the node kind the
            // cursor's flag cannot carry)
            let d = w.file(file);
            let ast = d.doc.ast();
            let h = crate::doc::node_at_caret(&d.doc, pos)?;
            if let Some(s) = h.downcast::<rut_ast::SelfNode>() {
                if s.get(ast).is_ty {
                    return None;
                }
            }
            let ty = enclosing_type(std::iter::once(d), span.lo)?;
            Some(HoverOut {
                markdown: render_ty(&ty),
                span: *span,
            })
        }
        // classify → resolve → render
        _ => super::render::hover(
            w,
            file,
            pos,
            &c,
            &crate::resolve::walk::resolve(w, file, &c),
        ),
    }
}
