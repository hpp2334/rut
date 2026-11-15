//! The token pass — classes from the lexed stream alone: keywords and
//! primitives by text, literals, operators, f-string tiling.

use rut_lexer::span::Span;
use rut_lexer::token::{FPart, Tok, Token};

use super::legend::{is_keyword, is_primitive_ty, TokenType};

/// Token-level classification — no AST needed. FStr is tiled separately
/// (`classify_token`); structural punctuation stays unclassified (themes
/// already paint it, and it keeps token streams small).
pub(crate) fn token_type(tok: &Tok) -> Option<TokenType> {
    match tok {
        Tok::Bool(_) => Some(TokenType::Keyword),
        Tok::Int(..) | Tok::Float(..) => Some(TokenType::Number),
        Tok::Str(_) | Tok::RawStr(_) => Some(TokenType::String),
        Tok::Ident(s) => {
            // keyword first: `nil` is reserved AND a type spelling — the
            // literal reads as a keyword constant, like `true`/`false`
            if is_keyword(s) {
                Some(TokenType::Keyword)
            } else if s == "Self" || is_primitive_ty(s) {
                Some(TokenType::Type)
            } else {
                None
            }
        }
        Tok::Eof => None,
        Tok::LParen
        | Tok::RParen
        | Tok::LBrace
        | Tok::RBrace
        | Tok::LBracket
        | Tok::RBracket
        | Tok::Comma
        | Tok::Semi
        | Tok::Colon
        | Tok::Dot => None,
        // arrows, `?` `@` `~`, and the arithmetic/bitwise families
        //
        _ => Some(TokenType::Operator),
    }
}

/// One token; f-strings tile exactly — string for the literal gaps, then
/// recursion into each hole's fully-lexed token stream (real spans inside
/// the literal's span).
pub(crate) fn classify_token(t: &Token, out: &mut Vec<(Span, TokenType)>) {
    if let Tok::FStr(f) = &t.tok {
        let mut cursor = t.span.lo;
        for part in &f.parts {
            if let FPart::Hole(hole) = part {
                if let (Some(first), Some(last)) = (hole.first(), hole.last()) {
                    if first.span.lo > cursor {
                        out.push((Span::new(cursor, first.span.lo), TokenType::String));
                    }
                    for ht in hole {
                        classify_token(ht, out);
                    }
                    cursor = last.span.hi;
                }
            }
        }
        if t.span.hi > cursor {
            out.push((Span::new(cursor, t.span.hi), TokenType::String));
        }
    } else if let Some(ty) = token_type(&t.tok) {
        out.push((t.span, ty));
    }
}
