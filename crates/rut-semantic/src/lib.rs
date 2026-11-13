//! Semantic tokens + document symbols — the M1 classifier (rut-semantic). Pure rut-side, no LSP types: token-level classes come from the
//! lexed stream (keywords by text; literals; f-string
//! tiling with fully-lexed holes), identifier classes from
//! a walk of the slot tree. Names are interner ids whose nodes carry
//! whole-construct spans, so name positions are **recovered**: a bounded
//! scan for the matching `Ident` token inside the node's span (the parser
//! itself matched by text, so this is reliable; a miss skips the name —
//! never a wrong color). `::` paths spell name resolution — their
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
use self::tokens::{classify_token, flatten_holes};

pub use self::doc::{mode_of, ParsedDoc};
pub use self::legend::{is_keyword, is_primitive_ty, TokenType, ALL};
pub use self::symbols::{symbols, SymKind, Symbol};
pub use self::tokens::token_type;

mod line_index;
pub use self::line_index::LineIndex;

// the document layer's vocabulary re-exports — ONLY what this crate's
// own API hands out, so the layer above (`rut-lsp-wasm`) needs no
// direct frontend Cargo dep to spell what ParsedDoc/classify return
pub use rut_ast::parser::{is_reserved_kw, Mode, ParseOpts, PRIMITIVE_TYS, RESERVED_KW};
pub use rut_lexer::diag::Diag;
pub use rut_lexer::lexer::canonical;
pub use rut_lexer::span::Span;
pub use rut_lexer::token::{Comment, FloatSuffix, IntSuffix, Tok, Token};

/// Classify a whole document: token-level classes, then AST-level name
/// classes (which win at equal positions). Output is sorted by start and
/// guaranteed non-overlapping — LSP's semantic-token contract.
pub fn classify(toks: &[Token], ast: &Ast) -> Vec<(Span, TokenType)> {
    let mut out: Vec<(Span, TokenType)> = Vec::new();
    for t in toks {
        classify_token(t, &mut out);
    }
    // name recovery runs over a FLAT token stream — f-string holes are
    // fully lexed with real spans but nested inside the
    // FStr token; the AST's hole expressions recover their names against
    // the spliced view
    let flat = flatten_holes(toks);
    classify_ast(&flat, ast, &mut out);
    finalize(out)
}

#[cfg(test)]
mod tests;
