//! Types and the two far-decision balancing scans (§4.2):
//! scan_is_lambda ( ( ... ) then => ) and scan_is_generic_args
//! ( < ... > then ( or . ). Read-only lookahead; no backtracking.

use rut_ast::{
    PathSeg, TyArrayNode, TyConstNode, TyFnNode, TyOptNode, TyPathNode, TyRef, TyTupleNode,
    TyUnionNode,
};
use rut_lexer::span::Span;
use rut_lexer::token::Tok;

use crate::expr::{ExprFrame, ExprMode};
use crate::frame::{Done, Frame, Step};
use crate::Parser;
use rut_core::sym;

pub(crate) struct TypeFrame {
    lo: Span,
    /// A completed type continues on `|` as a union bound —
    /// only where unions are legal (the alias target and `requires`
    /// bounds); elsewhere `|` stays the binary operator
    allow_union: bool,
    stage: TyStage,
}

enum TyStage {
    Init,
    /// `?T` — the nullable
    Opt,
    /// `*T` — the removed pointer spelling, recovered
    LegacyPtr,
    /// `[T]` — the array type
    Bracket,
    /// `(A, B)` / `(T)` — tuple, grouping
    Tuple {
        elems: Vec<TyRef>,
    },
    /// `fn(...)`: collecting parameter types
    FnParams {
        params: Vec<TyRef>,
    },
    /// `fn(...) ->`: waiting for the return type
    FnRet {
        params: Vec<TyRef>,
    },
    /// a path type, possibly mid-dot-chain
    Path {
        segs: Vec<PathSeg>,
    },
    /// inside a `<..>` after a segment name
    GenArgs {
        segs: Vec<PathSeg>,
        seg_name: rut_ast::IdentId,
        args: Vec<TyRef>,
    },
    /// `A | B` — a union bound
    Union {
        elems: Vec<TyRef>,
    },
}

impl TypeFrame {
    pub(crate) fn new(p: &Parser) -> Self {
        TypeFrame {
            lo: p.span(),
            allow_union: false,
            stage: TyStage::Init,
        }
    }

    /// A position where a union is legal: the alias target
    /// (`type U = A | B;`) and a `requires` bound (`T requires A | B`).
    pub(crate) fn new_bound(p: &Parser) -> Self {
        TypeFrame {
            lo: p.span(),
            allow_union: true,
            stage: TyStage::Init,
        }
    }

    pub(crate) fn step(&mut self, p: &mut Parser) -> Step {
        match self.stage {
            TyStage::Init => {
                // `?T` — the nullable type: prefix, binds tightest
                if p.eat_punct(Tok::Question) {
                    self.stage = TyStage::Opt;
                    return Step::Push(Frame::Type(TypeFrame::new(p)));
                }
                // the removed pointer spelling `*T`: diagnose and recover
                if matches!(p.tok(), Tok::Star) {
                    p.err_here("the pointer spelling `*T` was renamed — write `?T`");
                    p.bump();
                    self.stage = TyStage::LegacyPtr;
                    return Step::Push(Frame::Type(TypeFrame::new(p)));
                }
                // `[T]` — the array type
                if p.eat_punct(Tok::LBracket) {
                    self.stage = TyStage::Bracket;
                    return Step::Push(Frame::Type(TypeFrame::new(p)));
                }
                // tuple type `(A, B)` / grouping `(T)`
                if p.eat_punct(Tok::LParen) {
                    self.stage = TyStage::Tuple { elems: Vec::new() };
                    return self.tuple_top(p);
                }
                // fn type: `fn(Store, P) -> R`
                if p.at_kw("fn") && matches!(p.peek(1).tok, Tok::LParen) {
                    p.bump();
                    p.expect(Tok::LParen);
                    self.stage = TyStage::FnParams { params: Vec::new() };
                    return self.fnparams_top(p);
                }
                // a trait name in type position is the object type
                self.stage = TyStage::Path { segs: Vec::new() };
                self.path_run(p)
            }
            _ => unreachable!("stepped a suspended type frame"),
        }
    }

    /// `::`-separated path segments; generic args pause at a child frame
    fn path_run(&mut self, p: &mut Parser) -> Step {
        loop {
            let Some(name) = p.expect_ident("a type name") else {
                return Step::Pop(Done::Failed);
            };
            if matches!(p.tok(), Tok::Lt) {
                // type position: `<` is always generic args
                p.bump();
                let segs = match &mut self.stage {
                    TyStage::Path { segs } => std::mem::take(segs),
                    _ => unreachable!(),
                };
                self.stage = TyStage::GenArgs {
                    segs,
                    seg_name: name.id,
                    args: Vec::new(),
                };
                return self.genargs_top(p);
            }
            if let TyStage::Path { segs, .. } = &mut self.stage {
                segs.push(PathSeg {
                    name: name.id,
                    generics: Vec::new(),
                });
            }
            if !p.eat_punct(Tok::Colon2) {
                return self.path_pop(p);
            }
        }
    }

    fn path_resume(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Colon2) {
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
        let t = TyPathNode {
            span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
            segs,
        }
        .allocate(&mut p.ast);
        self.pop_or_union(p, t)
    }

    /// A completed type either pops — or a `|` follows and it continues
    /// as a union bound. A trailing postfix `?` is the removed spelling.
    fn pop_or_union(&mut self, p: &mut Parser, t: TyRef) -> Step {
        while matches!(p.tok(), Tok::Question) {
            p.err_here("the postfix spelling `T?` was removed — write `?T`");
            p.bump();
        }
        if self.allow_union && p.eat_punct(Tok::Pipe) {
            self.stage = TyStage::Union { elems: vec![t] };
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        Step::Pop(Done::Ty(t))
    }

    /// a union member completed: continue on `|`, else build the node
    fn union_absorb(&mut self, p: &mut Parser, t: TyRef) -> Step {
        let elems = match &mut self.stage {
            TyStage::Union { elems } => elems,
            _ => unreachable!("union absorb at the wrong stage"),
        };
        elems.push(t);
        if p.eat_punct(Tok::Pipe) {
            return Step::Push(Frame::Type(TypeFrame::new(p)));
        }
        let elems = std::mem::take(elems);
        let node = TyUnionNode {
            span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
            elems,
        }
        .allocate(&mut p.ast);
        Step::Pop(Done::Ty(node))
    }

    fn genargs_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::Gt) {
            return self.genargs_done(p);
        }
        // const-generic arg: integer expression
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
            TyStage::GenArgs {
                segs,
                seg_name,
                args,
            } => (std::mem::take(segs), *seg_name, std::mem::take(args)),
            _ => unreachable!(),
        };
        self.stage = TyStage::Path { segs };
        if let TyStage::Path { segs, .. } = &mut self.stage {
            segs.push(PathSeg {
                name: seg_name,
                generics: args,
            });
        }
        self.path_resume(p)
    }

    fn fnparams_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RParen) {
            return self.step_fn_ret(p);
        }
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    /// tuple type element list: `()` is the nil type — spelled `nil`
    fn tuple_top(&mut self, p: &mut Parser) -> Step {
        if p.eat_punct(Tok::RParen) {
            p.err_here("`()` is not a type — the empty type is spelled `nil` (v1.2)");
            return Step::Pop(Done::Ty(self.nil_ty(p)));
        }
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    fn nil_ty(&mut self, p: &mut Parser) -> TyRef {
        let nil = sym::NIL;
        TyPathNode {
            span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
            segs: vec![PathSeg {
                name: nil,
                generics: Vec::new(),
            }],
        }
        .allocate(&mut p.ast)
    }

    /// the parameter list closed (or failed): optional `->` then the
    /// return type — an omitted return is `nil`
    fn step_fn_ret(&mut self, p: &mut Parser) -> Step {
        let params = match &mut self.stage {
            TyStage::FnParams { params } => std::mem::take(params),
            _ => unreachable!(),
        };
        if !p.eat_punct(Tok::Arrow) {
            let ret = self.nil_ty(p);
            self.stage = TyStage::Init;
            let t = TyFnNode {
                span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                params,
                ret,
            }
            .allocate(&mut p.ast);
            return self.pop_or_union(p, t);
        }
        self.stage = TyStage::FnRet { params };
        Step::Push(Frame::Type(TypeFrame::new(p)))
    }

    pub(crate) fn absorb(&mut self, p: &mut Parser, d: Done) -> Step {
        match d {
            Done::Ty(t) => match &mut self.stage {
                TyStage::Union { .. } => self.union_absorb(p, t),
                TyStage::Opt => {
                    let node = TyOptNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        inner: t,
                    }
                    .allocate(&mut p.ast);
                    self.pop_or_union(p, node)
                }
                TyStage::LegacyPtr => {
                    // `*T` recovery: the element type IS the result
                    Step::Pop(Done::Ty(t))
                }
                TyStage::Bracket => {
                    p.expect(Tok::RBracket);
                    let node = TyArrayNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        elem: t,
                    }
                    .allocate(&mut p.ast);
                    self.pop_or_union(p, node)
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
                        let t = elems.into_iter().next().unwrap();
                        return self.pop_or_union(p, t);
                    }
                    let node = TyTupleNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        elems,
                    }
                    .allocate(&mut p.ast);
                    self.pop_or_union(p, node)
                }
                TyStage::FnParams { params } => {
                    params.push(t);
                    if p.eat_punct(Tok::Comma) {
                        self.fnparams_top(p)
                    } else {
                        p.expect(Tok::RParen);
                        self.step_fn_ret(p)
                    }
                }
                TyStage::FnRet { params } => {
                    let params = std::mem::take(params);
                    let node = TyFnNode {
                        span: Span::new(self.lo.lo, p.prev_end.max(self.lo.lo)),
                        params,
                        ret: t,
                    }
                    .allocate(&mut p.ast);
                    self.pop_or_union(p, node)
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
                // const-generic argument
                match &mut self.stage {
                    TyStage::GenArgs { args, .. } => {
                        args.push(
                            TyConstNode {
                                span: e.span(&p.ast),
                                expr: e,
                            }
                            .allocate(&mut p.ast),
                        );
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
                TyStage::FnParams { .. } => self.step_fn_ret(p),
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
    /// continues a generic use.
    pub(crate) fn scan_is_generic_args(&self, _in_pattern: bool) -> bool {
        let mut depth = 1i32; // the `<` at self.pos
        let mut i = self.pos + 1;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::Lt => depth += 1,
                Tok::Shl => depth += 2,
                Tok::Gt => depth -= 1,
                Tok::Shr => depth -= 2,
                Tok::LBracket => depth += 1,
                Tok::RBracket => {
                    depth -= 1;
                    if depth <= 0 {
                        return false;
                    }
                }
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth <= 0 {
                        return false;
                    }
                }
                Tok::Semi | Tok::Eof | Tok::RBrace => return false,
                _ => {}
            }
            i += 1;
            if depth == 0 {
                return matches!(
                    self.toks.get(i).map(|t| &t.tok),
                    Some(Tok::LParen) | Some(Tok::Colon2) | Some(Tok::LBrace)
                );
            }
            if depth < 0 {
                return false;
            }
        }
        false
    }
}
