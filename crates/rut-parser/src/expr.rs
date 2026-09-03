//! Expressions (RFC 0030 SS2): the precedence climb (assignment at the
//! top, down through logicals, relationals, arithmetics to unary/postfix),
//! primary forms (literals, paths, calls, lambdas, f-strings, struct
//! literals), and postfix member/index/invocation.

use rut_lexer::span::Span;
use rut_lexer::token::{FPart, FStrTok, Tok};
use super::*;

impl Parser {
    pub(crate) fn parse_expr(&mut self) -> Option<NodeHandle<AnyExpr>> {
        // C3: expression nesting carries a recursion-safe budget (recursive
        // descent costs ~13 frames per level; RFC 0030 OQ-3's proposed 1024
        // assumes a frame-stack parser. The CONTRACT is "a Diag, never a
        // host stack overflow" — 64 levels fits a 1 MiB stack (Windows main
        // thread, wasm) with margin; brackets and blocks keep NEST_MAX=1024.)
        const EXPR_MAX: u32 = 64;
        if self.depth >= EXPR_MAX {
            self.err(self.span(), "nesting too deep");
            // skip to the matching closer; resync forward only
            let mut depth = 0i32;
            loop {
                match self.tok() {
                    Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                    Tok::RParen | Tok::RBracket | Tok::RBrace => {
                        depth -= 1;
                        if depth <= 0 {
                            if depth == 0 {
                                self.bump();
                            }
                            break;
                        }
                    }
                    Tok::Semi | Tok::Eof => break,
                    _ => {}
                }
                self.bump();
            }
            return None;
        }
        self.depth += 1;
        let r = self.parse_expr_inner();
        self.depth -= 1;
        r
    }

    pub(crate) fn parse_expr_inner(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        // lambda scan 1: `( params ) (: Type)? =>` —scan read-only
        if matches!(self.tok(), Tok::LParen) && self.scan_is_lambda() {
            return self.parse_lambda();
        }
        // lambda scan 2: single-ident form `x =>`
        if let Tok::Ident(name) = self.tok().clone() {
            if !is_reserved_kw(&name)
                && matches!(self.peek(1).tok, Tok::FatArrow)
                && name != "else"
            {
                self.bump(); // ident
                self.bump(); // =>
                let p = self.interner.intern(&name);
                let param = self.member(
                    MemberKind::Param(ParamData { is_mut: false, name: p, ty: None }),
                    sp,
                );
                let body = if matches!(self.tok(), Tok::LBrace) {
                    self.parse_block_stmt()?.into()
                } else {
                    self.parse_expr()?
                };
                return Some(self.expr(
                    ExprKind::Lambda { params: vec![param], ret: None, body },
                    sp.to(self.span()),
                ));
            }
        }
        let lhs = self.parse_or()?;
        let (op, is_assign) = match self.tok() {
            Tok::Eq => (None, true),
            Tok::PlusEq => (Some(BinOp::Add), true),
            Tok::MinusEq => (Some(BinOp::Sub), true),
            Tok::StarEq => (Some(BinOp::Mul), true),
            Tok::SlashEq => (Some(BinOp::Div), true),
            Tok::PercentEq => (Some(BinOp::Mod), true),
            Tok::AmpEq => (Some(BinOp::BitAnd), true),
            Tok::PipeEq => (Some(BinOp::BitOr), true),
            Tok::CaretEq => (Some(BinOp::BitXor), true),
            Tok::ShlEq => (Some(BinOp::Shl), true),
            Tok::ShrEq => (Some(BinOp::Shr), true),
            Tok::AmpPlusEq => (Some(BinOp::WrapAdd), true),
            Tok::AmpMinusEq => (Some(BinOp::WrapSub), true),
            Tok::AmpStarEq => (Some(BinOp::WrapMul), true),
            Tok::AmpShlEq => (Some(BinOp::WrapShl), true),
            _ => (None, false),
        };
        if is_assign {
            self.bump();
            let value = self.parse_expr()?; // right-associative
            match &self.nodes[lhs.id().0 as usize].kind {
                Kind::Expr(ExprKind::Path { .. } | ExprKind::Field { .. } | ExprKind::Index { .. }) => {}
                _ => {
                    let sp = self.nodes[lhs.id().0 as usize].span;
                    self.err(sp, "invalid assignment target —expected a path, field, or index");
                }
            }
            return Some(self.expr(ExprKind::Assign { op, target: lhs, value }, sp.to(self.span())));
        }
        Some(lhs)
    }

    pub(crate) fn parse_lambda(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let params = self.parse_params()?;
        let ret = if self.eat_punct(Tok::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(Tok::FatArrow);
        let body = if matches!(self.tok(), Tok::LBrace) {
            self.parse_block_stmt()?.into()
        } else {
            self.parse_expr()?
        };
        Some(self.expr(ExprKind::Lambda { params, ret, body }, sp.to(self.span())))
    }

    // precedence climb: || < && < == != < relational+is < | ^ < & < << >> < + - < * / %
    pub(crate) fn parse_or(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut lhs = self.parse_and()?;
        while matches!(self.tok(), Tok::PipePipe) {
            self.bump();
            let rhs = self.parse_and()?;
            lhs = self.expr(ExprKind::Binary { op: BinOp::Or, lhs, rhs }, sp.to(self.span()));
        }
        Some(lhs)
    }
    pub(crate) fn parse_and(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut lhs = self.parse_eq()?;
        while matches!(self.tok(), Tok::AmpAmp) {
            self.bump();
            let rhs = self.parse_eq()?;
            lhs = self.expr(ExprKind::Binary { op: BinOp::And, lhs, rhs }, sp.to(self.span()));
        }
        Some(lhs)
    }
    pub(crate) fn parse_eq(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut lhs = self.parse_rel()?;
        loop {
            let op = match self.tok() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_rel()?;
            lhs = self.expr(ExprKind::Binary { op, lhs, rhs }, sp.to(self.span()));
        }
        Some(lhs)
    }
    /// Relational + `is` —`is` is NON-associative (RFC 0012 §3)
    pub(crate) fn parse_rel(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let lhs = self.parse_bit_or()?;
        let op = match self.tok() {
            Tok::Lt => Some(BinOp::Lt),
            Tok::Gt => Some(BinOp::Gt),
            Tok::LtEq => Some(BinOp::Le),
            Tok::GtEq => Some(BinOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let rhs = self.parse_bit_or()?;
            return Some(self.expr(ExprKind::Binary { op, lhs, rhs }, sp.to(self.span())));
        }
        if self.at_kw("is") {
            self.bump();
            // naming position: bare trait/instantiation or concrete type,
            // never `dyn`-prefixed (RFC 0030 §2)
            if self.at_kw("dyn") {
                self.err_here("the `is` right-hand side is a naming position —no `dyn` prefix (RFC 0012 §3)");
                self.bump();
            }
            let ty = self.parse_type()?;
            return Some(self.expr(ExprKind::Is { expr: lhs, ty }, sp.to(self.span())));
        }
        Some(lhs)
    }
    pub(crate) fn parse_bit_or(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut lhs = self.parse_bit_and()?;
        loop {
            let op = match self.tok() {
                Tok::Pipe => BinOp::BitOr,
                Tok::Caret => BinOp::BitXor,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_bit_and()?;
            lhs = self.expr(ExprKind::Binary { op, lhs, rhs }, sp.to(self.span()));
        }
        Some(lhs)
    }
    pub(crate) fn parse_bit_and(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut lhs = self.parse_shift()?;
        while matches!(self.tok(), Tok::Amp) {
            self.bump();
            let rhs = self.parse_shift()?;
            lhs = self.expr(ExprKind::Binary { op: BinOp::BitAnd, lhs, rhs }, sp.to(self.span()));
        }
        Some(lhs)
    }
    pub(crate) fn parse_shift(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut lhs = self.parse_add()?;
        loop {
            let op = match self.tok() {
                Tok::Shl => BinOp::Shl,
                Tok::Shr => BinOp::Shr,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_add()?;
            lhs = self.expr(ExprKind::Binary { op, lhs, rhs }, sp.to(self.span()));
        }
        Some(lhs)
    }
    pub(crate) fn parse_add(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.tok() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                // wrapping binary ops (RFC 0004 §3) sit at additive precedence
                Tok::AmpPlus => BinOp::WrapAdd,
                Tok::AmpMinus => BinOp::WrapSub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_mul()?;
            lhs = self.expr(ExprKind::Binary { op, lhs, rhs }, sp.to(self.span()));
        }
        Some(lhs)
    }
    pub(crate) fn parse_mul(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.tok() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                Tok::AmpStar => BinOp::WrapMul,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            lhs = self.expr(ExprKind::Binary { op, lhs, rhs }, sp.to(self.span()));
        }
        Some(lhs)
    }

    pub(crate) fn parse_unary(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let op = match self.tok() {
            Tok::Minus => Some(UnOp::Neg),
            Tok::Bang => Some(UnOp::Not),
            Tok::Tilde => Some(UnOp::BitNot),
            _ => None,
        };
        if let Some(op) = op {
            if !self.enter() {
                self.leave();
                return self.parse_postfix();
            }
            self.bump();
            let expr = self.parse_unary()?;
            self.leave();
            return Some(self.expr(ExprKind::Unary { op, expr }, sp.to(self.span())));
        }
        // `await` binds a unary-level operand; `await select {..}` is special
        if self.at_kw("await") {
            self.bump();
            if self.at_kw("select") && matches!(self.peek(1).tok, Tok::LBrace) {
                let arms = self.parse_select_arms()?;
                let sel = self.expr(ExprKind::Select { arms }, sp.to(self.span()));
                return Some(sel);
            }
            let expr = self.parse_unary()?;
            return Some(self.expr(ExprKind::Await { expr }, sp.to(self.span())));
        }
        self.parse_postfix()
    }

    pub(crate) fn parse_select_arms(&mut self) -> Option<Vec<NodeHandle<AnyArm>>> {
        self.bump(); // select
        self.expect(Tok::LBrace);
        let mut arms = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            let sp = self.span();
            let fut = self.parse_expr()?;
            let bind = if self.at_kw("as") {
                // `as` is reserved (RFC 0002 §4) —legal ONLY here (RFC 0019 §3)
                self.bump();
                self.expect_ident("a binding name")
            } else {
                None
            };
            self.expect(Tok::Arrow);
            let body = self.parse_expr()?;
            self.eat_punct(Tok::Comma);
            arms.push(self.arm(ArmKind::SelectArm { fut, bind, body }, sp.to(self.span())));
        }
        Some(arms)
    }

    /// Postfix loop: `.name` `.name<..>(..)` `(..)` `[..]` `?` —chains
    /// compose without special cases (RFC 0030 §3, prec 13).
    pub(crate) fn parse_postfix(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        let mut e = self.parse_primary()?;
        if !self.enter() {
            self.leave();
            return Some(e);
        }
        loop {
            match self.tok() {
                Tok::Dot => {
                    self.bump();
                    let name = self.expect_ident("a member name")?;
                    let mut generics = Vec::new();
                    if matches!(self.tok(), Tok::Lt) && self.scan_is_generic_args(false) {
                        self.bump();
                        loop {
                            if self.eat_punct(Tok::Gt) {
                                break;
                            }
                            let arg = if matches!(self.tok(), Tok::Int(..)) {
                                let ex = self.parse_unary()?;
                                self.typ(TypeKind::TyConst(ex), self.span())
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
                    if matches!(self.tok(), Tok::LParen) {
                        self.bump();
                        let args = self.parse_call_args()?;
                        e = self.expr(
                            ExprKind::Method { recv: e, name, generics, args },
                            sp.to(self.span()),
                        );
                    } else {
                        e = self.expr(ExprKind::Field { recv: e, name }, sp.to(self.span()));
                    }
                }
                Tok::LParen => {
                    self.bump();
                    let args = self.parse_call_args()?;
                    e = self.expr(ExprKind::Call { callee: e, args }, sp.to(self.span()));
                }
                Tok::LBracket => {
                    self.bump();
                    let idx = self.parse_expr()?;
                    self.expect(Tok::RBracket);
                    e = self.expr(ExprKind::Index { recv: e, idx }, sp.to(self.span()));
                }
                Tok::Question => {
                    self.bump();
                    e = self.expr(ExprKind::Try { expr: e }, sp.to(self.span()));
                }
                _ => break,
            }
        }
        self.leave();
        Some(e)
    }

    pub(crate) fn parse_call_args(&mut self) -> Option<Vec<NodeHandle<AnyExpr>>> {
        let mut args = Vec::new();
        loop {
            if self.eat_punct(Tok::RParen) {
                break;
            }
            let a = self.parse_expr()?;
            args.push(a);
            if !self.eat_punct(Tok::Comma) {
                self.expect(Tok::RParen);
                break;
            }
        }
        Some(args)
    }

    pub(crate) fn parse_primary(&mut self) -> Option<NodeHandle<AnyExpr>> {
        let sp = self.span();
        match self.tok().clone() {
            Tok::Int(v, sfx) => {
                self.bump();
                Some(self.expr(ExprKind::Lit(Lit::Int(v, sfx)), sp))
            }
            Tok::Float(bits, sfx) => {
                self.bump();
                Some(self.expr(ExprKind::Lit(Lit::Float(bits, sfx)), sp))
            }
            Tok::Str(s) => {
                self.bump();
                Some(self.expr(ExprKind::Lit(Lit::Str(s)), sp))
            }
            Tok::RawStr(s) => {
                self.bump();
                Some(self.expr(ExprKind::Lit(Lit::RawStr(s)), sp))
            }
            Tok::Char(c) => {
                self.bump();
                Some(self.expr(ExprKind::Lit(Lit::Char(c)), sp))
            }
            Tok::Bool(b) => {
                self.bump();
                Some(self.expr(ExprKind::Lit(Lit::Bool(b)), sp))
            }
            Tok::FStr(f) => {
                self.bump();
                self.parse_fstring(f, sp)
            }
            Tok::LParen => {
                self.bump();
                let mut first = true;
                let mut e = None;
                loop {
                    if self.eat_punct(Tok::RParen) {
                        break;
                    }
                    if !first {
                        // RFC 0030 §4.2: a comma inside parens that is not a
                        // lambda errors AT THE COMMA —there are no tuples
                        if matches!(self.tok(), Tok::Comma) {
                            self.err_here("there are no tuples (RFC 0009) —if you meant a lambda, add `=>`");
                            self.bump();
                            continue;
                        }
                    }
                    let x = self.parse_expr();
                    if first {
                        e = x;
                        first = false;
                    } else if x.is_none() {
                        break;
                    }
                }
                e
            }
            Tok::LBracket => {
                self.bump();
                let mut elems = Vec::new();
                loop {
                    if self.eat_punct(Tok::RBracket) {
                        break;
                    }
                    elems.push(self.parse_expr()?);
                    if !self.eat_punct(Tok::Comma) {
                        self.expect(Tok::RBracket);
                        break;
                    }
                }
                Some(self.expr(ExprKind::ArrayLit { elems }, sp.to(self.span())))
            }
            Tok::Ident(name) => {
                if name == "when" && matches!(self.peek(1).tok, Tok::LParen) {
                    let (scrut, arms) = self.parse_when_head()?;
                    return Some(self.expr(ExprKind::WhenExpr { scrut, arms }, sp.to(self.span())));
                }
                if name == "self" {
                    self.bump();
                    let seg = PathSeg { name: self.interner.intern("self"), generics: Vec::new() };
                    return Some(self.expr(ExprKind::Path { segs: vec![seg] }, sp));
                }
                if name == "Self" {
                    self.bump();
                    // `Self { .. }` —the class-private literal (RFC 0010 §1)
                    if matches!(self.tok(), Tok::LBrace) {
                        let ty_name = self.interner.intern("Self");
                        return self.parse_struct_body(sp, ty_name);
                    }
                    let seg = PathSeg { name: self.interner.intern("Self"), generics: Vec::new() };
                    return Some(self.expr(ExprKind::Path { segs: vec![seg] }, sp));
                }
                // struct literal: `Ident {` (dataclass only —classes have no
                // instance literal; the checker rejects `Circle { .. }`)
                if matches!(self.peek(1).tok, Tok::LBrace) && !is_reserved_kw(&name) {
                    return self.parse_struct_lit(sp, &name);
                }
                self.bump();
                let first = PathSeg { name: self.interner.intern(&name), generics: Vec::new() };
                let mut segs = vec![first];
                // generic args on the head segment: `Vec<f32>(1024)`,
                // `downcast<Point>(o)`, `MyMap<K, V>.new(..)`, `Option<T>.Some(x)`
                if matches!(self.tok(), Tok::Lt) && self.scan_is_generic_args(false) {
                    self.bump();
                    let mut generics = Vec::new();
                    loop {
                        if self.eat_punct(Tok::Gt) {
                            break;
                        }
                        let arg = if matches!(self.tok(), Tok::Int(..)) {
                            let ex = self.parse_unary()?;
                            self.typ(TypeKind::TyConst(ex), self.span())
                        } else {
                            self.parse_type()?
                        };
                        generics.push(arg);
                        if !self.eat_punct(Tok::Comma) {
                            self.expect_gt();
                            break;
                        }
                    }
                    segs[0].generics = generics;
                }
                while self.eat_punct(Tok::Dot) {
                    let name = self.expect_ident("a member name")?;
                    let mut generics = Vec::new();
                    if matches!(self.tok(), Tok::Lt) && self.scan_is_generic_args(false) {
                        self.bump();
                        loop {
                            if self.eat_punct(Tok::Gt) {
                                break;
                            }
                            generics.push(self.parse_type()?);
                            if !self.eat_punct(Tok::Comma) {
                                self.expect_gt();
                                break;
                            }
                        }
                    }
                    if matches!(self.tok(), Tok::LParen) {
                        // a call after the segment: `p.area(..)`, `Vec.from(..)`,
                        // `MyMap<K, V>.new(..)` —a METHOD node over the path
                        // built so far (the resolver decides static vs instance)
                        self.bump();
                        let args = self.parse_call_args()?;
                        let recv = self.expr(ExprKind::Path { segs }, sp);
                        return Some(self.expr(
                            ExprKind::Method { recv, name, generics, args },
                            sp.to(self.span()),
                        ));
                    }
                    segs.push(PathSeg { name, generics });
                }
                Some(self.expr(ExprKind::Path { segs }, sp.to(self.span())))
            }
            _ => {
                let found = self.peek(0).describe();
                self.err_here(format!("expected an expression, found {found}"));
                None
            }
        }
    }

    /// Dataclass literal / `Self { .. }`: `Name { field: expr, .. }`
    pub(crate) fn parse_struct_lit(&mut self, sp: Span, name: &str) -> Option<NodeHandle<AnyExpr>> {
        let ty_name = self.interner.intern(name);
        self.bump(); // the ident
        self.parse_struct_body(sp, ty_name)
    }

    /// struct literal body: the cursor sits ON the `{`
    pub(crate) fn parse_struct_body(&mut self, sp: Span, ty_name: IdentId) -> Option<NodeHandle<AnyExpr>> {
        self.bump(); // {
        let mk_ty = |p: &mut Parser| {
            p.typ(
                TypeKind::TyPath {
                    segs: vec![PathSeg { name: ty_name, generics: Vec::new() }],
                    is_dyn: false,
                },
                sp,
            )
        };
        if !self.enter() {
            self.leave();
            self.sync_stmt();
            let ty = mk_ty(self);
            return Some(self.expr(ExprKind::Struct { ty, fields: Vec::new() }, sp));
        }
        let ty = mk_ty(self);
        let mut fields = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            let Some(f) = self.expect_ident("a field name") else {
                self.sync_stmt();
                break;
            };
            self.expect(Tok::Colon);
            let v = self.parse_expr()?;
            fields.push((f, v));
            if !self.eat_punct(Tok::Comma) {
                self.expect(Tok::RBrace);
                break;
            }
        }
        self.leave();
        Some(self.expr(ExprKind::Struct { ty, fields }, sp.to(self.span())))
    }

    /// f-string: holes were lexed as token streams (RFC 0030 §1.1) —parse
    /// each hole as an expression with real spans inside the literal.
    pub(crate) fn parse_fstring(&mut self, f: FStrTok, sp: Span) -> Option<NodeHandle<AnyExpr>> {
        let mut parts = Vec::new();
        for part in f.parts {
            match part {
                FPart::Lit(s) => parts.push(FPartAst::Lit(s)),
                FPart::Hole(hole_toks) => {
                    // sub-parse over the hole's token slice: same grammar, a
                    // frame over the sub-slice, not a nested parse call site
                    let mut sub = Parser {
                        toks: hole_toks,
                        pos: 0,
                        diags: Vec::new(),
                        mode: Mode::Impl,
                        depth: 0,
                        depth_reported: false,
                        nodes: std::mem::take(&mut self.nodes),
                        interner: std::mem::take(&mut self.interner),
                    };
                    let expr = sub.parse_expr();
                    self.diags.append(&mut sub.diags);
                    self.nodes = std::mem::take(&mut sub.nodes);
                    self.interner = std::mem::take(&mut sub.interner);
                    match expr {
                        Some(e) => parts.push(FPartAst::Hole(e)),
                        None => {
                            self.err(sp, "could not parse the format-string placeholder");
                            parts.push(FPartAst::Lit(String::new()));
                        }
                    }
                }
            }
        }
        Some(self.expr(ExprKind::FStr { parts }, sp))
    }
}
