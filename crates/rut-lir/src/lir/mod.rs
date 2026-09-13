//! Body compilation —RFC 0031 §2/§3 fused into one walk (M1
//! simplification: bidirectional inference + emission together; the
//! separate SSA-ish HIR of RFC 0031 §3 is a later pass). Output: typed
//! register bytecode per function (RFC 0032), monomorphized (RFC 0013 §2).

use rut_ast::ast::*;
use crate::check::{Ctx, FnKey, Inst, TcResult};
use rut_lexer::span::Span;
use rut_core::binary::ConstVal;
use rut_core::ops::*;
use rut_core::types::*;

mod call;
mod expr;
mod generic;
mod intrinsic;
mod lit;
mod ops;
mod peephole;
mod slice;
mod sroa;
mod stmt;
mod utf8;

const NEST_MAX: u32 = 1024;

#[derive(Clone, Copy)]
pub(crate) struct Local {
    name: IdentId,
    reg: u16,
    ty: TypeId,
    is_mut: bool,
    /// for-c induction variables are loop-owned (RFC 0008 §1)
    loop_var: bool,
}

pub struct FnCompiler<'a, 'b> {
    ctx: &'b mut Ctx<'a>,
    regs: Vec<TypeId>,
    code: Vec<Op>,
    spans: Vec<(u32, u32)>,
    locals: Vec<Local>,
    ret_ty: TypeId,
    self_ty: Option<TypeId>,
    subst: Vec<(IdentId, TypeId)>,
    current_class: Option<IdentId>,
    depth: u32,
    loops: Vec<(u32, u32)>, // (continue label, break label)
    labels: Vec<Option<u32>>,
    fixups: Vec<(usize, u32, bool)>, // (op index, label id, is_else_target)
    span: u32,
    /// the register holding the value produced by the last compile_expr
    last_reg: u16,
    /// while inlining a `Slice` accessor (RFC 0005), reads of this local
    /// (`self`) use the receiver register directly — no copy, so element
    /// access pays no per-access ref copy
    inline_self: Option<(IdentId, u16)>,
    /// while inlining a returned method body: `(result reg, end label)` —
    /// `return` writes here and jumps, instead of emitting `Op::Ret`
    inline_ret: Option<(u16, u32)>,
    /// class methods currently being inlined — a recursion guard
    inline_stack: Vec<(IdentId, IdentId)>,
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// Compile one instantiation into `ctx.funcs[fid]`.
    pub fn compile(ctx: &mut Ctx<'a>, inst: &Inst, fid: u32) -> TcResult<()> {
        // lambdas carry their capture list in ctx.lambda_info
        if let FnKey::Lambda(lambda_node) = inst.key {
            return Self::compile_lambda_fn(ctx, inst, fid, lambda_node);
        }
        let (node, self_ty, is_method, class_name) = match &inst.key {
            FnKey::Free(name) => {
                let Some(n) = ctx.fn_nodes.iter().find(|(n, _)| n == name).map(|(_, n)| *n) else {
                    return Ok(()); // unknown fn —already diagnosed
                };
                (n.id(), None, false, None)
            }
            FnKey::Method { data, name } => {
                let Some((_, d)) = ctx.datas.iter().find(|(n, _)| n == data) else {
                    return Ok(());
                };
                let d = d.clone();
                let Some(m) = d.methods.iter().find(|(n, _)| n == name).map(|(_, n)| *n) else {
                    return Ok(());
                };
                // a generic class's method is monomorphized per instantiation:
                // `self_ty` is `Name<args>` from the Inst substitution, not the
                // uninstantiated template
                let self_ty = if d.generics.is_empty() {
                    d.ty
                } else {
                    let args: Vec<TypeId> = d
                        .generics
                        .iter()
                        .map(|g| {
                            inst.subst
                                .iter()
                                .find(|(n, _)| n == g)
                                .map(|(_, t)| *t)
                                .unwrap_or(TY_I32)
                        })
                        .collect();
                    ctx.mk_data_inst(*data, args)
                };
                (m.id(), Some(self_ty), true, Some(*data))
            }
            FnKey::ImplMethod { idx, name } => {
                let im = ctx.impls[*idx].clone();
                let Some(m) = im.methods.iter().find(|(n, _)| n == name).map(|(_, n)| *n) else {
                    return Ok(());
                };
                let cname = ctx.datas.iter().find(|(_, d)| d.ty == im.target).map(|(n, _)| *n);
                (m.id(), Some(im.target), true, cname)
            }
            FnKey::Lambda(_) => unreachable!(),
        };
        let is_suspend = match ctx.ast.kind(node) {
            Kind::Item(ItemKind::Fn(f)) => f.is_suspend,
            Kind::Member(MemberKind::MethodDecl(m)) => m.is_suspend,
            _ => return Ok(()),
        };
        let (params, ret, generics, body, fn_name) = match ctx.ast.kind(node) {
            Kind::Item(ItemKind::Fn(f)) => (
                f.params.clone(),
                f.ret,
                f.generics.clone(),
                f.body.id(), // Fn bodies are always blocks
                ctx.name(f.name).to_string(),
            ),
            Kind::Member(MemberKind::MethodDecl(m)) => (
                m.params.clone(),
                m.ret,
                m.generics.clone(),
                m.body.map(|b| b.id()).unwrap_or(node), // bodiless shouldn't be queued
                format!("{}$", ctx.name(m.name)),
            ),
            _ => return Ok(()),
        };
        if is_suspend {
            ctx.err(
                ctx.ast.span(node),
                "`suspend` functions are not supported in this build (RFC 0018 —M3)",
            );
            return Err(());
        }
        let mut c = FnCompiler {
            ctx,
            regs: Vec::new(),
            code: Vec::new(),
            spans: Vec::new(),
            locals: Vec::new(),
            ret_ty: TY_UNIT,
            self_ty,
            subst: inst.subst.clone(),
            current_class: class_name,
            depth: 0,
            loops: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            span: 0,
            last_reg: 0,
            inline_self: None,
            inline_ret: None,
            inline_stack: Vec::new(),
        };
        // signature: params (self first for methods), resolved under subst
        let mut param_tys: Vec<TypeId> = Vec::new();
        for p in &params {
            let ty = match c.ctx.ast.param(*p) {
                MemberKind::SelfParam(_) => self_ty.unwrap_or(TY_UNIT),
                MemberKind::Param(ParamData { ty: Some(t), .. }) => c.resolve_type_now(*t),
                MemberKind::Param(ParamData { ty: None, name, .. }) => {
                    c.ctx.err(
                        c.ctx.ast.span(p.id()),
                        format!(
                            "parameter `{}` needs a type annotation here",
                            c.ctx.name(*name)
                        ),
                    );
                    TY_I32
                }
                _ => TY_I32,
            };
            param_tys.push(ty);
        }
        // ret
        let ret_ty = ret.map(|r| c.resolve_type_now(r)).unwrap_or(TY_UNIT);
        c.ret_ty = ret_ty;
        // bind params as locals
        for (i, p) in params.iter().enumerate() {
            match c.ctx.ast.param(*p) {
                MemberKind::SelfParam(SelfParamData { is_mut }) => {
                    let reg = c.new_reg(self_ty.unwrap_or(TY_UNIT));
                    // bind `self` —the ident exists iff the body spells it
                    // (the parser interns it on `self` paths)
                    if let Some(sid) = c.ctx.lookup_name("self") {
                        c.locals.push(Local {
                            name: sid,
                            reg,
                            ty: self_ty.unwrap_or(TY_UNIT),
                            is_mut: *is_mut,
                            loop_var: false,
                        });
                    }
                }
                MemberKind::Param(ParamData { name, is_mut, .. }) => {
                    let reg = c.new_reg(param_tys[i]);
                    c.locals.push(Local {
                        name: *name,
                        reg,
                        ty: param_tys[i],
                        is_mut: *is_mut,
                        loop_var: false,
                    });
                }
                _ => {}
            }
        }
        let _ = generics; // user generic methods unsupported (diag at call)
        // body
        if c.compile_block(body).is_err() {
            return Err(());
        }
        // implicit `return` for unit fns; non-unit fns must return on all
        // paths (checked loosely: a final Ret with default value)
        c.emit(Op::Ret { val: None }, 0);
        c.resolve_labels();
        let (code, spans) = sroa::run(c.code, c.spans);
        let (code, spans) = peephole::run(code, spans);
        let regs = c.regs;
        let fc = rut_core::binary::FuncCode {
            name: fn_name,
            params: param_tys,
            ret: ret_ty,
            is_method,
            n_captures: 0,
            regs,
            code,
            spans,
            host: None,
        };
        let f = &mut c.ctx.funcs[fid as usize];
        *f = fc;
        Ok(())
    }

    /// Compile a lambda instantiation: declared params ++ captures.
    pub(crate) fn compile_lambda_fn(ctx: &mut Ctx<'a>, inst: &Inst, fid: u32, lambda_node: NodeId) -> TcResult<()> {
        // the capture list was recorded when the MakeClosure was emitted;
        // the node we keyed on is the lambda BODY
        let (params, ret, body) = match ctx.ast.expr(NodeHandle::new(lambda_node)) {
            ExprKind::Lambda { params, ret, body } => (params.clone(), *ret, *body),
            _ => return Ok(()),
        };
        let caps = ctx.lambda_info.get(&lambda_node).cloned().unwrap_or_default();
        let mut c = FnCompiler {
            ctx,
            regs: Vec::new(),
            code: Vec::new(),
            spans: Vec::new(),
            locals: Vec::new(),
            ret_ty: TY_UNIT,
            self_ty: None,
            subst: inst.subst.clone(),
            current_class: None,
            depth: 0,
            loops: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            span: 0,
            last_reg: 0,
            inline_self: None,
            inline_ret: None,
            inline_stack: Vec::new(),
        };
        // NOTE: lambda param/ret types were recorded... re-derive:
        // annotations resolve here; unannotated ones took the expected type
        // at the creation site —store those too. For M1 we re-resolve and
        // accept that an unannotated param's type must be reconstructible;
        // the creation site stored only captures. To stay correct, the
        // creation site also stores the resolved param/ret types:
        let saved = c.ctx.lambda_sigs.get(&lambda_node).cloned();
        let mut param_tys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let ty = match c.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => c.resolve_type_now(*t),
                _ => saved.as_ref().and_then(|s| s.0.get(i).copied()).unwrap_or(TY_I32),
            };
            param_tys.push(ty);
        }
        let ret_ty = ret.map(|r| c.resolve_type_now(r)).or(saved.as_ref().map(|s| s.1)).unwrap_or(TY_UNIT);
        c.ret_ty = ret_ty;
        // bind params then captures
        for (i, p) in params.iter().enumerate() {
            if let MemberKind::Param(ParamData { name, is_mut, .. }) = c.ctx.ast.param(*p) {
                let reg = c.new_reg(param_tys[i]);
                c.locals.push(Local { name: *name, reg, ty: param_tys[i], is_mut: *is_mut, loop_var: false });
            }
        }
        for (n, t) in &caps {
            let reg = c.new_reg(*t);
            c.locals.push(Local { name: *n, reg, ty: *t, is_mut: false, loop_var: false });
            // captures are part of the fn's parameter list (after declared)
            param_tys.push(*t);
        }
        let n_caps = caps.len() as u32;
        // body: block or single expression (RFC 0013 §1 arrows)
        if let Some(block) = c.ctx.ast.narrow_block(body) {
            if c.compile_block(block).is_err() {
                return Err(());
            }
            c.emit(Op::Ret { val: None }, 0);
        } else {
            let t = c.compile_expr(body, Some(ret_ty))?;
            if t != ret_ty {
                c.ctx.err(c.ctx.ast.span(body.id()), format!(
                    "lambda returns `{}` but is typed `{}`",
                    c.ctx.types.name(t), c.ctx.types.name(ret_ty)
                ));
            }
            c.emit(Op::Ret { val: Some(c.last_reg) }, 0);
        }
        c.resolve_labels();
        let (code, spans) = sroa::run(c.code, c.spans);
        let (code, spans) = peephole::run(code, spans);
        let regs = c.regs;
        let fc = rut_core::binary::FuncCode {
            name: format!("lambda@{}", body.id().0),
            params: param_tys,
            ret: ret_ty,
            is_method: false,
            n_captures: n_caps,
            regs,
            code,
            spans,
            host: None,
        };
        let f = &mut c.ctx.funcs[fid as usize];
        *f = fc;
        Ok(())
    }

    // ---- infrastructure ----

    pub(crate) fn new_reg(&mut self, ty: TypeId) -> u16 {
        self.regs.push(ty);
        let r = (self.regs.len() - 1) as u16;
        self.last_reg = r;
        r
    }

    pub(crate) fn emit(&mut self, op: Op, span_lo: u32) {
        self.spans.push((self.code.len() as u32, span_lo));
        self.code.push(op);
    }

    pub(crate) fn new_label(&mut self) -> u32 {
        self.labels.push(None);
        (self.labels.len() - 1) as u32
    }
    pub(crate) fn bind(&mut self, l: u32) {
        self.labels[l as usize] = Some(self.code.len() as u32);
    }
    pub(crate) fn resolve_labels(&mut self) {
        for (op_idx, l, is_else) in std::mem::take(&mut self.fixups) {
            let target = self.labels[l as usize].unwrap_or(0);
            match &mut self.code[op_idx] {
                Op::Jmp { target: t } => *t = target,
                Op::Br { then_t, else_t, .. } => {
                    if is_else {
                        *else_t = target;
                    } else {
                        *then_t = target;
                    }
                }
                Op::BrTable { table, default, .. } => {
                    if is_else {
                        *default = target;
                    } else if let Some(first) = table.first_mut() {
                        *first = target;
                    }
                }
                _ => unreachable!(),
            }
        }
    }
    pub(crate) fn jmp(&mut self, l: u32) {
        self.fixups.push((self.code.len(), l, false));
        self.emit(Op::Jmp { target: 0 }, self.span);
    }
    pub(crate) fn br(&mut self, cond: u16, then_l: u32, else_l: u32) {
        self.fixups.push((self.code.len(), then_l, false));
        self.fixups.push((self.code.len(), else_l, true));
        self.emit(Op::Br { cond, then_t: 0, else_t: 0 }, self.span);
    }

    pub(crate) fn resolve_type_now(&mut self, node: NodeHandle<AnyTy>) -> TypeId {
        // `Self` binds to the enclosing type inside method bodies
        if let TypeKind::TyPath { segs, .. } = self.ctx.ast.ty(node) {
            if segs.len() == 1 && segs[0].generics.is_empty() {
                let n = self.ctx.name(segs[0].name).to_string();
                if n == "Self" {
                    if let Some(t) = self.self_ty {
                        return t;
                    }
                    self.ctx.err(self.ctx.ast.span(node.id()), "`Self` outside a type body");
                    return TY_I32;
                }
            }
        }
        self.ctx.resolve_type(node, &self.subst)
    }

    pub(crate) fn lookup(&self, name: IdentId) -> Option<&Local> {
        self.locals.iter().rev().find(|l| l.name == name)
    }

    pub(crate) fn konst(&mut self, v: ConstVal) -> u16 {
        // dedup
        if let Some(i) = self.ctx.consts.iter().position(|c| *c == v) {
            return i as u16;
        }
        self.ctx.consts.push(v);
        (self.ctx.consts.len() - 1) as u16
    }

    pub(crate) fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > NEST_MAX {
            self.ctx.err(
                Span::new(self.span, self.span + 1),
                "expression nesting too deep",
            );
            return false;
        }
        true
    }
    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }

}
