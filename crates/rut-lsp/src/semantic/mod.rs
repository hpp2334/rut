//! Semantic tokens + document symbols — the M1 classifier (RFC 0041 §2,
//! rut-lsp). Pure rut-side, no LSP types: token-level classes come from the
//! lexed stream (keywords by text — RFC 0002 §4/§5; literals; f-string
//! tiling with fully-lexed holes — RFC 0030 §1.1), identifier classes from
//! a flat walk of the AST arena. Names are interner ids whose nodes carry
//! whole-construct spans, so name positions are **recovered**: a bounded
//! scan for the matching `Ident` token inside the node's span (the parser
//! itself matched by text, so this is reliable; a miss skips the name —
//! never a wrong color). Value-position paths have no resolution in M1 —
//! they classify by capitalization convention, like a TextMate grammar
//! would, and full resolution lands with M2 modules.

mod legend;
mod names;
mod recover;
mod symbols;
mod tokens;

use rut_ast::ast::Ast;
use rut_lexer::span::Span;
use rut_lexer::token::Token;

use self::names::classify_ast;
use self::recover::finalize;
use self::tokens::{classify_token, flatten_holes};

pub use self::legend::{is_keyword, is_primitive_ty, TokenType, ALL};
pub use self::symbols::{symbols, RawSymbol, SymKind};
pub use self::tokens::token_type;

/// Classify a whole document: token-level classes, then AST-level name
/// classes (which win at equal positions). Output is sorted by start and
/// guaranteed non-overlapping — LSP's semantic-token contract.
pub fn classify(toks: &[Token], ast: &Ast) -> Vec<(Span, TokenType)> {
    let mut out: Vec<(Span, TokenType)> = Vec::new();
    for t in toks {
        classify_token(t, &mut out);
    }
    // name recovery runs over a FLAT token stream — f-string holes are
    // fully lexed with real spans (RFC 0030 §1.1) but nested inside the
    // FStr token; the AST's hole expressions recover their names against
    // the spliced view
    let flat = flatten_holes(toks);
    classify_ast(&flat, ast, &mut out);
    finalize(out)
}

#[cfg(test)]
mod tests;
