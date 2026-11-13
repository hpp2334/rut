//! The recovery marker — where the hole sits (D10).
//!
//! An `ErrorNode` occupies a child SLOT (`ItemRef::Error`,
//! `StmtRef::Error`, `ExprRef::Error`, `ParamRef::Error`, …) and says:
//! this region did not parse; here is its span and its diags. The
//! RECOVERY LADDER: damage to a construct's PARTS → an ErrorNode in
//! the member slot, the construct allocates anyway (RECOVERED); damage
//! to a construct's IDENTITY (its name) → the construct never
//! allocates, the region becomes an ErrorNode at its stream position
//! (FENCED); LEX damage → a root-level ErrorNode. An ErrorNode is a
//! marked wrapper, never a fake construct.

use rut_lexer::diag::Diag;
use rut_lexer::span::Span;

#[derive(Clone, Debug)]
pub struct ErrorNode {
    pub span: Span,
    pub diags: Vec<Diag>,
}

use crate::plain_alloc;
plain_alloc!(ErrorNode);
