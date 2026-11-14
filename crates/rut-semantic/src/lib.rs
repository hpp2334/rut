//! Semantic tokens + document symbols — the M1 classifier (rut-semantic). Pure rut-side, no LSP types: token-level classes come from the
//! lexed stream (keywords by text; literals; f-string
//! tiling with fully-lexed holes), identifier classes from
//! a walk of the slot tree. Names are interner ids whose nodes record
//! the name token's own span, so the AST walk emits each name at its
//! recorded position — the one guard left: texts the token layer owns
//! by word shape (keywords, `Self`) do not emit, and the token pass's
//! class stands. `::` paths spell name resolution — their
//! segments classify as the law's heads and members (modules read as
//! `namespace`, never properties); value access after `.` rides the
//! Field/Method nodes and keeps its own classes.

mod doc;
mod legend;
mod names;
mod recover;
mod symbols;
mod tokens;

use rut_ast::Ast;

use self::names::classify_ast;
use self::recover::finalize;
use self::tokens::classify_token;

pub use self::doc::{mode_of, Document};
pub use self::legend::{is_keyword, is_primitive_ty, TokenType};
pub(crate) use self::symbols::symbols;
pub use self::symbols::{SymKind, Symbol};

mod line_index;
pub use self::line_index::LineIndex;

// the document layer's vocabulary re-exports — ONLY what this crate's
// own API hands out, so the layer above (`rut-lsp-wasm`) needs no
// direct frontend Cargo dep to spell what Document returns
pub use rut_ast::parser::{is_reserved_kw, Mode, PRIMITIVE_TYS, RESERVED_KW};
pub use rut_lexer::diag::Diag;
pub use rut_lexer::lexer::canonical;
pub use rut_lexer::span::Span;
pub use rut_lexer::token::{FloatSuffix, IntSuffix, Tok, Token};

/// Classify a whole document: token-level classes, then AST-level name
/// classes at their recorded spans (which win at equal positions).
/// Output is sorted by start and guaranteed non-overlapping — LSP's
/// semantic-token contract. The classifier's lane: `Document::spans`
/// rides it lazily; no consumer calls it by hand.
pub(crate) fn classify(toks: &[Token], ast: &Ast) -> Vec<(Span, TokenType)> {
    let mut out: Vec<(Span, TokenType)> = Vec::new();
    for t in toks {
        classify_token(t, &mut out);
    }
    classify_ast(ast, &mut out);
    finalize(out)
}
