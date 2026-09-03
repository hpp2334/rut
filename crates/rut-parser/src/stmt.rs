//! Statements and patterns (RFC 0030 SS2): blocks, let/if/while/for,
//! when heads, and the pattern grammar (literals, ctors, alternatives).

use rut_lexer::span::Span;
use rut_lexer::token::Tok;
use super::*;

impl Parser {
    // ---- statements ----

    pub(crate) fn parse_block_body(&mut self, lo: u32) -> Option<NodeHandle<BlockNode>> {
        if !self.enter() {
            // depth-exceeded: consume to the matching brace so the cursor
            // stays monotone and the enclosing item can continue
            let mut depth = 1;
            while !self.at_eof() {
                match self.tok() {
                    Tok::LBrace => {
                        depth += 1;
                        self.bump();
                    }
                    Tok::RBrace => {
                        self.bump();
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {
                        self.bump();
                    }
                }
            }
            return Some(self.block(Vec::new(), Span::new(lo, self.span().hi)));
        }
        let mut stmts = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            let before = self.pos;
            match self.parse_stmt() {
                Some(s) => stmts.push(s),
                None => {
                    if self.pos == before {
                        self.bump();
                    }
                    self.sync_stmt();
                }
            }
        }
        self.leave();
        Some(self.block(stmts, Span::new(lo, self.span().hi)))
    }

    /// `{ stmts }` in statement position (callers of `parse_block_body`
    /// inline it after consuming `{`; this one starts at the brace).
    pub(crate) fn parse_block_stmt(&mut self) -> Option<NodeHandle<BlockNode>> {
        let lo = self.expect(Tok::LBrace)?.lo;
        self.parse_block_body(lo)
    }

    pub(crate) fn parse_stmt(&mut self) -> Option<NodeHandle<AnyStmt>> {
        let sp = self.span();
        match self.tok().clone() {
            Tok::Ident(kw) => match kw.as_str() {
                "let" => self.parse_let_stmt(),
                "if" => Some(self.parse_if()?.into()),
                "while" => {
                    self.bump();
                    self.expect(Tok::LParen);
                    let cond = self.parse_expr()?;
                    self.expect(Tok::RParen);
                    let body = self.parse_block_stmt()?;
                    Some(self.stmt(StmtKind::While { cond, body }, sp.to(self.span())))
                }
                "for" => self.parse_for(),
                "return" => {
                    self.bump();
                    let value = if matches!(self.tok(), Tok::Semi) {
                        None
                    } else {
                        Some(self.parse_expr()?)
                    };
                    self.expect(Tok::Semi);
                    Some(self.stmt(StmtKind::Return { value }, sp.to(self.span())))
                }
                "when" => {
                    let scrut_and_arms = self.parse_when_head()?;
                    let (scrut, arms) = scrut_and_arms;
                    Some(self.stmt(StmtKind::WhenStmt { scrut, arms }, sp.to(self.span())))
                }
                "break" => {
                    self.bump();
                    self.expect(Tok::Semi);
                    Some(self.stmt(StmtKind::Break, sp))
                }
                "continue" => {
                    self.bump();
                    self.expect(Tok::Semi);
                    Some(self.stmt(StmtKind::Continue, sp))
                }
                _ => {
                    let e = self.parse_expr()?;
                    self.expect(Tok::Semi);
                    Some(self.stmt(StmtKind::ExprStmt(e), sp.to(self.span())))
                }
            },
            _ => {
                let e = self.parse_expr()?;
                self.expect(Tok::Semi);
                Some(self.stmt(StmtKind::ExprStmt(e), sp.to(self.span())))
            }
        }
    }

    pub(crate) fn parse_let_stmt(&mut self) -> Option<NodeHandle<AnyStmt>> {
        let lo = self.bump().span.lo; // let
        let is_mut = self.at_kw("mut") && {
            self.bump();
            true
        };
        let name = self.expect_ident("a binding name")?;
        let ty = if self.eat_punct(Tok::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(Tok::Eq);
        let init = self.parse_expr()?;
        self.expect(Tok::Semi);
        Some(self.stmt(
            StmtKind::LetStmt { is_mut, name, ty, init },
            Span::new(lo, self.span().hi),
        ))
    }

    pub(crate) fn parse_if(&mut self) -> Option<NodeHandle<IfNode>> {
        let lo = self.bump().span.lo; // if
        self.expect(Tok::LParen);
        let cond = self.parse_expr()?;
        self.expect(Tok::RParen);
        let then = self.parse_block_stmt()?;
        let els = if self.at_kw("else") {
            self.bump();
            if self.at_kw("if") {
                Some(ElseBranch::If(self.parse_if()?))
            } else {
                Some(ElseBranch::Block(self.parse_block_stmt()?))
            }
        } else {
            None
        };
        Some(self.if_stmt(cond, then, els, Span::new(lo, self.span().hi)))
    }

    pub(crate) fn parse_for(&mut self) -> Option<NodeHandle<AnyStmt>> {
        let lo = self.bump().span.lo; // for
        self.expect(Tok::LParen);
        if !self.at_kw("let") {
            self.err_here("expected `let` in a for header");
            self.sync_stmt();
            return None;
        }
        self.bump();
        let var = self.expect_ident("a loop variable")?;
        // for-of vs for-c: peek `of` vs `=` (—4 tokens, RFC 0030 §4.1)
        if self.at_kw("of") {
            self.bump();
            let iter = self.parse_expr()?;
            self.expect(Tok::RParen);
            let body = self.parse_block_stmt()?;
            return Some(self.stmt(StmtKind::ForOf { var, iter, body }, Span::new(lo, self.span().hi)));
        }
        self.expect(Tok::Eq);
        let init = self.parse_expr()?;
        self.expect(Tok::Semi);
        let cond = self.parse_expr()?;
        self.expect(Tok::Semi);
        let update = self.parse_expr()?;
        self.expect(Tok::RParen);
        let body = self.parse_block_stmt()?;
        Some(self.stmt(
            StmtKind::ForC { var, init, cond, update, body },
            Span::new(lo, self.span().hi),
        ))
    }

    /// `when ( expr ) { arms }` —shared by stmt and expr positions.
    pub(crate) fn parse_when_head(&mut self) -> Option<(NodeHandle<AnyExpr>, Vec<NodeHandle<AnyArm>>)> {
        self.bump(); // when
        self.expect(Tok::LParen);
        let scrut = self.parse_expr()?;
        self.expect(Tok::RParen);
        self.expect(Tok::LBrace);
        if !self.enter() {
            self.sync_stmt();
            return Some((scrut, Vec::new()));
        }
        let mut arms = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            let before = self.pos;
            // patterns until `->`
            let mut pats = Vec::new();
            loop {
                let Some(p) = self.parse_pattern() else {
                    break;
                };
                pats.push(p);
                if !self.eat_punct(Tok::Comma) {
                    break;
                }
                if matches!(self.tok(), Tok::Arrow) {
                    break;
                }
            }
            self.expect(Tok::Arrow);
            // body: `{` —block arm (peek 1 —RFC 0030 §4.1); else expr arm
            let is_block = matches!(self.tok(), Tok::LBrace);
            let body = if is_block {
                self.parse_block_stmt()?.into()
            } else {
                self.parse_expr()?
            };
            // comma required between expression arms, optional after block
            // arms (RFC 0008 §2 —the corpus uses `,` after both)
            let had_comma = self.eat_punct(Tok::Comma);
            if !is_block && !had_comma && !matches!(self.tok(), Tok::RBrace) {
                self.err_here("expression arms must be comma-separated (RFC 0008 §2)");
            }
            arms.push(self.arm(ArmKind::WhenArm { pats, body }, self.span()));
            if self.pos == before {
                self.bump();
            }
        }
        self.leave();
        Some((scrut, arms))
    }

    pub(crate) fn parse_pattern(&mut self) -> Option<NodeHandle<AnyPat>> {
        let sp = self.span();
        match self.tok().clone() {
            Tok::Int(..) | Tok::Float(..) | Tok::Bool(_) | Tok::Char(_) | Tok::Str(_) | Tok::RawStr(_) => {
                let lit = self.parse_primary()?;
                Some(self.pat(PatKind::PatLit(lit), sp))
            }
            Tok::Minus => {
                // negative literal pattern
                let lit = self.parse_unary()?;
                Some(self.pat(PatKind::PatLit(lit), sp))
            }
            Tok::Ident(kw) if kw == "else" => {
                self.bump();
                Some(self.pat(PatKind::PatElse, sp))
            }
            Tok::Ident(name) if name == "_" => {
                self.bump();
                Some(self.pat(PatKind::PatWild, sp))
            }
            Tok::Ident(_) => {
                // path, possibly with generic args, possibly a constructor
                let mut segs = Vec::new();
                loop {
                    let name = self.expect_ident("a pattern name")?;
                    let mut generics = Vec::new();
                    if matches!(self.tok(), Tok::Lt) && self.scan_is_generic_args(true) {
                        self.bump();
                        loop {
                            if self.eat_punct(Tok::Gt) {
                                break;
                            }
                            let arg = self.parse_type()?;
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
                if matches!(self.tok(), Tok::LParen) {
                    // constructor pattern: args are bindings or `_`
                    self.bump();
                    let mut args = Vec::new();
                    loop {
                        if self.eat_punct(Tok::RParen) {
                            break;
                        }
                        match self.tok().clone() {
                            Tok::Ident(b) if b == "_" => {
                                self.bump();
                                args.push(None);
                            }
                            Tok::Ident(b) => {
                                self.bump();
                                args.push(Some(self.interner.intern(&b)));
                            }
                            _ => {
                                let found = self.peek(0).describe();
                                self.err_here(format!("expected a binding name in a constructor pattern, found {found}"));
                                args.push(None);
                                self.bump();
                            }
                        }
                        if !self.eat_punct(Tok::Comma) {
                            self.expect(Tok::RParen);
                            break;
                        }
                    }
                    Some(self.pat(PatKind::PatCtor { segs, args }, sp))
                } else {
                    Some(self.pat(PatKind::PatPath { segs }, sp))
                }
            }
            _ => {
                let found = self.peek(0).describe();
                self.err_here(format!("expected a pattern, found {found}"));
                None
            }
        }
    }

    // ---- expressions ----

}
