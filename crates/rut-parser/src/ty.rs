//! Types (RFC 0030 §2) and the two far-decision balancing scans (§4.2):
//! scan_is_lambda ( ( ... ) then => ) and scan_is_generic_args
//! ( < ... > then ( or . ). Read-only lookahead; no backtracking.

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use rut_core::sym;
use crate::expr::{ExprFrame, ExprMode};
use crate::frame::{Done, Frame, Step};
use crate::Parser;

pub(crate) struct TypeFrame {
    lo: Span,
    stage: TyStage,
}

enum TyStage {
    Init,
    /// `*T` — pointer (RFC 0005)
    Ptr,
    /// `[T]` — the array type (RFC 0005)
    Bracket,
    /// `(A, B)` / `(T)` — tuple, grouping (RFC 0007); `()` is rejected —
    /// the empty type is spelled `nil` (v1.2)
    Tuple { elems: Vec<NodeHandle<AnyTy>> },
    /// `fn(...)`: collecting parameter types
    FnParams { params: Vec<NodeHandle<AnyTy>> },
    /// `fn(...) ->`: waiting for the return type
    FnRet { params: Vec<NodeHandle<AnyTy>> },
    /// a path type, possibly mid-dot-chain
    Path { segs: Vec<PathSeg> },
    /// inside a `<..>` after a segment name
    GenArgs { segs: Vec<PathSeg>, seg_name: IdentId, args: Vec<NodeHandle<AnyTy>> },
}

impl TypeFrame {
    pub(crate) fn new(p: &Parser) -> Self {
        TypeFrame { lo: p.span(), stage: TyStage::Init }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        match self.stage {
            TyStage::Init => {
                // pointer type `*T` (RFC 0005)
                if p.eat_punct(Tok::Star) {
                    self.stage = TyStage::Ptr;
                    return Step::Push(Frame::Type(TypeFrame::new(p)));
                }
                // `[T]` — the array type (RFC 0005)
                if p.eat_punct(Tok::LBracket) {
                    self.stage = TyStage::Bracket;
                    return Step::Push(Frame::Type(TypeFrame::new(p)));
                }
                // tuple type `(A, B)` / grouping `(T)` (RFC 0007); `()`
                // is the nil type and must be spelled `nil` (v1.2)
                if p.eat_punct(Tok::LParen) {
                    self.stage = TyStage::Tuple { elems: Vec::new() };
                    return self.tuple_top(p);
                }
                // fn type: `fn(Store, P) -> R` — params are bare types
                if p.at_kw("fn") && matches!(p.peek(1).tok, Tok::LParen) {
                    p.bump();
                    p.expect(Tok::LParen);
                    self.stage = TyStage::FnParams { params: Vec::new() };
                    return self.fnparams_top(p);
                }
                // a trait name in type position is the object type —
                // the engine decides dispatch (RFC 0012 §3)
                self.stage = TyStage::Path { segs: Vec::new() };
                self.path_run(p)
            }
            _ => unreachable!("stepped a suspended type frame"),
        }
    }

    /// dotted path segments; generic args pause at a child frame and
    /// resume through `genargs_done` — the loop is iterative (C2)
    fn path_run(&mut self, p: &mut Parser) -> Step {
        loop {
            let Some(name) = p.expect_ident("a type name") else {
                return Step::Pop(Done::Failed);
            };
            if matches!(p.tok(), Tok::Lt) {
                // type position: `<` is always generic args — no ambiguity
                p.bump();
                let segs = match &mut self.stage {
                    TyStage::Path { segs } => std::mem::take(segs),
                    _ => unreachable!(),
                };
                self.stage = TyStage::GenArgs { segs, seg_name: name, args: Vec::new() };
                return self.genargs_top(p);
            }
            if let TyStage::Path { segs, .. } = &mut self.stage {
                segs.push(PathSeg { name, generics: Vec::new() });
            }
            if !p.eat_punct(Tok::Dot) {
                return self.path_pop(p);
            }
        }
    }

    /// resume after a generic-argument list completed one segment
    fn path_resume(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Dot) {
            self.path_run(p)
        } else {
            self.path_pop(p)
        }
    }

    fn path_pop(&mut self, p: &mut Parser) -> Step {
        let segs = match &mut self.stage {
            TyStage::Path { segs } => std::mem::take(segs),
            _ => unreachable!(),
        };
        Step::Pop(Done::Ty(p.typ(TypeKind::TyPath { segs }, self.lo.to(p.span()))))
    }

    fn genargs_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Gt) {
            return self.genargs_done(p);
        }
        // const-generic arg: integer expression (RFC 0005)
        if matches!(p.tok(), Tok::Int(..))
            || (matches!(p.tok(), Tok::Minus) && matches!(p.peek(1).tok, Tok::Int(..)))
        {
            Step::Push(Frame::Expr(ExprFrame::new(p, ExprMode::UnaryOnly)))
        } else {
            Step::Push(Frame::Type(TypeFrame::new(p)))
        }
    }

    fn genargs_done(&mut self, p: &mut Parser) -> Step {
        let (segs, seg_name, args) = match &mut self.stage {
            TyStage::GenArgs { segs, seg_name, args } => {
                (std::mem::take(segs), *seg_name, std::mem::take(args))
            }
            _ => unreachable!(),
        };
        self.stage = TyStage::Path { segs };
        if let TyStage::Path { segs, .. } = &mut self.stage {
            segs.push(PathSeg { name: seg_name, generics: args });
        }
        self.path_resume(p)
    }

    fn fnparams_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RParen) {
            return self.to_fn_ret(p);
        }
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    /// tuple type element list: elements separate on commas, one-element
    /// `(T)` is a grouping; `()` is the nil type — spelled `nil`, not `()`
    fn tuple_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RParen) {
            p.err_here("`()` is not a type — the empty type is spelled `nil` (v1.2)");
            return Step::Pop(Done::Ty(self.nil_ty(p)));
        }
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    fn nil_ty(&mut self, p: &mut Parser) -> NodeHandle<AnyTy> {
        let nil = sym::NIL;
        p.typ(
            TypeKind::TyPath {
                segs: vec![PathSeg { name: nil, generics: Vec::new() }],
            },
            self.lo.to(p.span()),
        )
    }

    /// the parameter list closed (or failed): optional `->` then the
    /// return type — an omitted return is `nil` (RFC 0013 §1, v1.2)
    fn to_fn_ret(&mut self, p: &mut Parser) -> Step {
        let params = match &mut self.stage {
            TyStage::FnParams { params } => std::mem::take(params),
            _ => unreachable!(),
        };
        if !p.eat_punct(Tok::Arrow) {
            let nil = sym::NIL;
            let ret = p.typ(
                TypeKind::TyPath {
                    segs: vec![PathSeg { name: nil, generics: Vec::new() }],
                },
                self.lo.to(p.span()),
            );
            self.stage = TyStage::Init;
            return Step::Pop(Done::Ty(p.typ(
                TypeKind::TyFn { params, ret },
                self.lo.to(p.span()),
            )));
        }
        self.stage = TyStage::FnRet { params };
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Ty(t) => match &mut self.stage {
                TyStage::Ptr => {
                    Step::Pop(Done::Ty(p.typ(TypeKind::TyPtr { inner: t }, self.lo.to(p.span()))))
                }
                TyStage::Bracket => {
                    p.expect(Tok::RBracket);
                    let array = sym::ARRAY;
                    let segs = vec![PathSeg {
                        name: array,
                        generics: vec![t],
                    }];
                    Step::Pop(Done::Ty(p.typ(
                        TypeKind::TyPath { segs },
                        self.lo.to(p.span()),
                    )))
                }
                TyStage::Tuple { elems } => {
                    elems.push(t);
                    if p.eat_punct(Tok::Comma) {
                        return self.tuple_top(p);
                    }
                    p.expect(Tok::RParen);
                    let elems = std::mem::take(elems);
                    if elems.len() == 1 {
                        // `(T)` is a grouping — the type itself
                        return Step::Pop(Done::Ty(elems.into_iter().next().unwrap()));
                    }
                    Step::Pop(Done::Ty(p.typ(TypeKind::TyTuple { elems }, self.lo.to(p.span()))))
                }
                TyStage::FnParams { params } => {
                    params.push(t);
                    if p.eat_punct(Tok::Comma) {
                        self.fnparams_top(p)
                    } else {
                        p.expect(Tok::RParen);
                        self.to_fn_ret(p)
                    }
                }
                TyStage::FnRet { params } => {
                    let params = std::mem::take(params);
                    Step::Pop(Done::Ty(p.typ(TypeKind::TyFn { params, ret: t }, self.lo.to(p.span()))))
                }
                TyStage::GenArgs { args, .. } => {
                    args.push(t);
                    if p.eat_punct(Tok::Comma) {
                        self.genargs_top(p)
                    } else {
                        p.expect_gt();
                        self.genargs_done(p)
                    }
                }
                _ => unreachable!("type frame received a type at the wrong stage"),
            },
            Done::Expr(e) => {
                // const-generic argument (RFC 0005)
                match &mut self.stage {
                    TyStage::GenArgs { args, .. } => {
                        args.push(p.typ(TypeKind::TyConst(e), p.span()));
                    }
                    _ => unreachable!("type frame received an expression at the wrong stage"),
                }
                if p.eat_punct(Tok::Comma) {
                    self.genargs_top(p)
                } else {
                    p.expect_gt();
                    self.genargs_done(p)
                }
            }
            Done::Failed => match self.stage {
                // v1: a failed parameter type breaks to the arrow
                TyStage::FnParams { .. } => self.to_fn_ret(p),
                _ => Step::Pop(Done::Failed),
            },
            _ => unreachable!("type frame receives types or const expressions"),
        }
    }
}

impl Parser {
    /// §4.2 scan 1: `(` … `)` then `=>` (a `-> Type` may sit between).
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
        // skip `-> Type` — scan until `=>` at angle/paren depth 0
        if matches!(self.toks[i].tok, Tok::FatArrow) {
            return true;
        }
        if !matches!(self.toks[i].tok, Tok::Arrow) {
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
    /// continues a generic use (`(` call — TypeScript's rule —or `.`
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
