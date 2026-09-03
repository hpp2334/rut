//! Types and the two far-decision balancing scans (RFC 0030 SS4.2):
//! scan_is_lambda ( ( ... ) then => ) and scan_is_generic_args
//! ( < ... > then ( or . ). Read-only lookahead; no backtracking.

use rut_lexer::token::Tok;
use super::*;

impl Parser {
    // ---- types ----

    /// A type in any position. `dyn` prefixes trait paths in VALUE positions
    /// (RFC 0030 §2); naming positions call `parse_type_naming` instead.
    pub(crate) fn parse_type(&mut self) -> Option<NodeHandle<AnyTy>> {
        let lo = self.span();
        if self.at_kw("fn") && matches!(self.peek(1).tok, Tok::LParen) {
            // fn type: `fn(Store, P): R` —params are bare types
            self.bump();
            self.expect(Tok::LParen);
            let mut params = Vec::new();
            loop {
                if self.eat_punct(Tok::RParen) {
                    break;
                }
                let Some(t) = self.parse_type() else {
                    break;
                };
                params.push(t);
                if !self.eat_punct(Tok::Comma) {
                    self.expect(Tok::RParen);
                    break;
                }
            }
            self.expect(Tok::Colon);
            let ret = self.parse_type()?;
            return Some(self.typ(TypeKind::TyFn { params, ret }, lo.to(self.span())));
        }
        let is_dyn = if self.at_kw("dyn") {
            self.bump();
            true
        } else {
            false
        };
        let mut segs = Vec::new();
        loop {
            let name = self.expect_ident("a type name")?;
            let mut generics = Vec::new();
            if matches!(self.tok(), Tok::Lt) {
                // type position: `<` is always generic args —no ambiguity
                self.bump();
                loop {
                    if self.eat_punct(Tok::Gt) {
                        break;
                    }
                    // const-generic arg: integer expression (RFC 0005)
                    let arg = if matches!(self.tok(), Tok::Int(..))
                        || (matches!(self.tok(), Tok::Minus) && matches!(self.peek(1).tok, Tok::Int(..)))
                    {
                        let e = self.parse_unary()?;
                        self.typ(TypeKind::TyConst(e), self.span())
                    } else {
                        self.parse_type()?
                    };
                    generics.push(arg);
                    if !self.eat_punct(Tok::Comma) {
                        self.expect_gt();
                        break;
                    }
                }
            }
            segs.push(PathSeg { name, generics });
            if self.eat_punct(Tok::Dot) {
                continue;
            }
            break;
        }
        Some(self.typ(TypeKind::TyPath { segs, is_dyn }, lo.to(self.span())))
    }

    /// §4.2 scan 1: `(` —`)` then `=>` (a `: Type` may sit between).
    /// Read-only; never mutates parser state.
    pub(crate) fn scan_is_lambda(&self) -> bool {
        let mut i = self.pos + 1;
        let mut depth = 1i32;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RParen | Tok::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        if i >= self.toks.len() {
            return false;
        }
        // skip `: Type` —scan until `=>` at angle/paren depth 0
        if matches!(self.toks[i].tok, Tok::FatArrow) {
            return true;
        }
        if !matches!(self.toks[i].tok, Tok::Colon) {
            return false;
        }
        let mut j = i + 1;
        let mut angle = 0i32;
        let mut paren = 0i32;
        while j < self.toks.len() {
            match &self.toks[j].tok {
                Tok::FatArrow if angle == 0 && paren == 0 => return true,
                Tok::Lt | Tok::Shl => angle += 1,
                Tok::Gt => angle -= 1,
                Tok::Shr => angle -= 2,
                Tok::LParen => paren += 1,
                Tok::RParen => paren -= 1,
                Tok::Semi | Tok::Eof | Tok::RBrace if angle <= 0 && paren <= 0 => return false,
                _ => {}
            }
            j += 1;
        }
        false
    }

    /// §4.2 scan 2: from a `<` (at `self.pos`), is this a generic-argument
    /// list? Commits iff angle depth returns to 0 and the next token
    /// continues a generic use (`(` call —TypeScript's rule —or `.`
    /// path continuation, which the corpus needs for `MyMap<K, V>.new` /
    /// `Option<T>.Some`). `>>` counts as two closers (span arithmetic).
    pub(crate) fn scan_is_generic_args(&self, _in_pattern: bool) -> bool {
        let mut depth = 1i32; // the `<` at self.pos
        let mut i = self.pos + 1;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::Lt => depth += 1,
                Tok::Shl => depth += 2,
                Tok::Gt => depth -= 1,
                Tok::Shr => depth -= 2,
                Tok::Semi | Tok::Eof | Tok::RParen | Tok::RBrace | Tok::RBracket => return false,
                _ => {}
            }
            i += 1;
            if depth == 0 {
                return matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::LParen) | Some(Tok::Dot));
            }
            if depth < 0 {
                return false;
            }
        }
        false
    }

}
