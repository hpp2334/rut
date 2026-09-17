//! Body compilation —RFC 0031 §2/§3 fused into one walk (M1
//! simplification: bidirectional inference + emission together; the
//! separate SSA-ish HIR of RFC 0031 §3 is a later pass). Output: typed
//! register bytecode per function (RFC 0032), monomorphized (RFC 0013 §2).

use rut_ast::ast::*;
use crate::check::{Ctx, FnKey, Inst, TcResult};
use rut_lexer::span::Span;
use rut_core::binary::ConstVal;
use rut_core::ops::*;
use std::collections::HashMap;
use rut_core::sym;
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

/// The per-function operand pools (RFC 0032): ops address their variadic
/// `Reg` lists and `BrTable` arms by `(off, len)` span into these tables
/// instead of owning a `Vec` — that is what keeps `Op` at 16 bytes. Lists
/// are interned (deduplicated) at emission; rewriters that remap registers
/// re-intern, so sharing stays consistent. The empty list is the span
/// `(0, 0)` and is never stored.
pub(crate) struct Pools {
    pub argv: Vec<Reg>,
    argv_ix: HashMap<Vec<Reg>, u32>,
    pub labels: Vec<Label>,
}

impl Pools {
    fn new() -> Pools {
        Pools { argv: Vec::new(), argv_ix: HashMap::new(), labels: Vec::new() }
    }

    /// Intern an argument list; returns its `(off, argc)` span.
    pub(crate) fn args(&mut self, a: &[Reg]) -> (u32, u16) {
        if a.is_empty() {
            return (0, 0);
        }
        debug_assert!(a.len() <= u16::MAX as usize, "operand list longer than the register file");
        if let Some(&off) = self.argv_ix.get(a) {
            return (off, a.len() as u16);
        }
        let off = self.argv.len() as u32;
        self.argv.extend_from_slice(a);
        self.argv_ix.insert(a.to_vec(), off);
        (off, a.len() as u16)
    }

    /// `[recv] ++ args` — the callee's whole parameter list in one span
    /// (`CallM`/`CallI`: the receiver is `argv[0]`).
    pub(crate) fn recv_args(&mut self, recv: Reg, a: &[Reg]) -> (u32, u16) {
        let mut l = Vec::with_capacity(a.len() + 1);
        l.push(recv);
        l.extend_from_slice(a);
        self.args(&l)
    }

    /// Intern a branch table; returns its `(off, count)` span.
    pub(crate) fn table(&mut self, t: &[Label]) -> (u32, u16) {
        if t.is_empty() {
            return (0, 0);
        }
        // branch tables are rare (`when` chains); no dedup map for them —
        // identical tables simply share nothing
        let off = self.labels.len() as u32;
        self.labels.extend_from_slice(t);
        (off, t.len() as u16)
    }

    pub(crate) fn slice(&self, off: u32, len: u16) -> &[Reg] {
        &self.argv[off as usize..off as usize + len as usize]
    }
}

#[derive(Clone)]
pub(crate) struct Local {
    name: IdentId,
    reg: u16,
    ty: TypeId,
    is_mut: bool,
    /// for-c induction variables are loop-owned (RFC 0008 §1)
    loop_var: bool,
    /// origin counting (RFC 0012 §5): the concrete types a trait-typed
    /// binding is known to hold. Single origin ⇒ static dispatch;
    /// empty ⇒ unknown/multiple ⇒ vtable. Only direct constructions
    /// (a widening let, a copied binding, a specialized parameter)
    /// populate it — conservative by construction.
    origins: Vec<TypeId>,
}

pub struct FnCompiler<'a, 'b> {
    ctx: &'b mut Ctx<'a>,
    regs: Vec<TypeId>,
    code: Vec<Op>,
    pools: Pools,
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
    /// inside a desugared `for..of` emit closure (RFC 0012 §6):
    /// `break` → `return false`, `continue` → `return true`
    emit_closure: bool,
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// Compile one instantiation into `ctx.funcs[fid]`.
    pub fn compile(ctx: &mut Ctx<'a>, inst: &Inst, fid: u32) -> TcResult<()> {
        // lambdas carry their capture list in ctx.lambda_info
        if let FnKey::Lambda(lambda_node) = inst.key {
            return Self::compile_lambda_fn(ctx, inst, fid, lambda_node);
        }
        // a desugared `for..of` emit closure carries its signature in
        // ctx.for_of_sigs (RFC 0012 §6)
        if let FnKey::ForOfEmit { body, var } = inst.key {
            return Self::compile_for_of_emit_fn(ctx, fid, body, var);
        }
        let (node, self_ty, is_method, class_name) = match &inst.key {
            // handled by the early return above
            FnKey::ForOfEmit { .. } => unreachable!(),
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
                // self: the concrete target — a generic target instantiates
                // under the Inst substitution; a native builtin target
                // rebuilds its shape (`Array<T>`)
                let (self_ty, cname) = match &im.target_data {
                    Some((dname, params)) if ctx.find_data(*dname).is_some() => {
                        let args: Vec<TypeId> = params
                            .iter()
                            .map(|g| {
                                inst.subst
                                    .iter()
                                    .find(|(n, _)| n == g)
                                    .map(|(_, t)| *t)
                                    .unwrap_or(TY_I32)
                            })
                            .collect();
                        (ctx.mk_data_inst(*dname, args), Some(*dname))
                    }
                    Some((dname, params))
                        if ctx.extern_native_types.get(dname).copied()
                            == Some(rut_core::binary::NativeTy::Array) =>
                    {
                        let elem = inst
                            .subst
                            .iter()
                            .find(|(n, _)| n == &params[0])
                            .map(|(_, t)| *t)
                            .unwrap_or(TY_I32);
                        (ctx.mk_array(elem), None)
                    }
                    _ => {
                        let cname = ctx.datas.iter().find(|(_, d)| d.ty == im.target).map(|(n, _)| *n);
                        (im.target, cname)
                    }
                };
                (m.id(), Some(self_ty), true, cname)
            }
            FnKey::Lambda(_) => unreachable!(),
        };
        let is_async = match ctx.ast.kind(node) {
            Kind::Item(ItemKind::Fn(f)) => f.is_async,
            Kind::Member(MemberKind::MethodDecl(m)) => m.is_async,
            _ => return Ok(()),
        };
        let (params, ret, generics, body, fn_name) = match ctx.ast.kind(node) {
            Kind::Item(ItemKind::Fn(f)) => (
                f.params.clone(),
                f.ret,
                f.generics.clone(),
                f.body.id(), // Fn bodies are always blocks
                f.name,
            ),
            Kind::Member(MemberKind::MethodDecl(m)) => (
                m.params.clone(),
                m.ret,
                m.generics.clone(),
                m.body.map(|b| b.id()).unwrap_or(node), // bodiless shouldn't be queued
                m.name,
            ),
            _ => return Ok(()),
        };
        if is_async {
            ctx.err(
                ctx.ast.span(node),
                "`async` functions are not supported in this build —cold-poll futures land in M3 (RFC 0018)",
            );
            return Err(());
        }
        let mut c = FnCompiler {
            ctx,
            regs: Vec::new(),
            code: Vec::new(),
            pools: Pools::new(),
            spans: Vec::new(),
            locals: Vec::new(),
            ret_ty: TY_NIL,
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
            emit_closure: false,
        };
        // signature: params (self first for methods), resolved under subst
        let mut param_tys: Vec<TypeId> = Vec::new();
        for p in &params {
            let ty = match c.ctx.ast.param(*p) {
                MemberKind::SelfParam(_) => self_ty.unwrap_or(TY_NIL),
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
        let ret_ty = ret.map(|r| c.resolve_type_now(r)).unwrap_or(TY_NIL);
        c.ret_ty = ret_ty;
        // this instantiation's concrete origins for the fn's trait-typed
        // parameters, in declaration order
        let mut param_origins = inst.trait_origins.clone();
        let mut param_origins_iter = param_origins.drain(..);
        // bind params as locals
        for (i, p) in params.iter().enumerate() {
            match c.ctx.ast.param(*p) {
                MemberKind::SelfParam(SelfParamData { is_mut }) => {
                    let reg = c.new_reg(self_ty.unwrap_or(TY_NIL));
                    // bind `self` (well-known symbol): the local is only ever
                    // FOUND where the body spells `self`, so binding it
                    // unconditionally is safe
                    c.locals.push(Local {
                        name: sym::SELF,
                        reg,
                        ty: self_ty.unwrap_or(TY_NIL),
                        is_mut: *is_mut,
                        loop_var: false,
                        origins: Vec::new(),
                    });
                }
                MemberKind::Param(ParamData { name, is_mut, .. }) => {
                    let reg = c.new_reg(param_tys[i]);
                    // trait-typed parameters: the Inst carries this call's
                    // concrete origin (a trait parameter IS an implicit
                    // generic bound, RFC 0012 §5) — single origin ⇒ static
                    let origins = if matches!(c.ctx.types.kind(param_tys[i]), TyKind::TraitObj { .. }) {
                        param_origins_iter.next().map(|t| vec![t]).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    c.locals.push(Local {
                        name: *name,
                        reg,
                        ty: param_tys[i],
                        is_mut: *is_mut,
                        loop_var: false,
                        origins,
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
        // implicit `return` for nil fns; non-nil fns must return on all
        // paths (checked loosely: a final Ret with default value)
        c.emit(Op::Ret { val: None }, 0);
        c.resolve_labels();
        let (code, spans) = sroa::run(c.code, c.spans, &mut c.pools);
        let (code, spans, pools) = peephole::run(code, spans, c.pools);
        let Pools { argv, labels, .. } = pools;
        let regs = c.regs;
        let fc = rut_core::binary::FuncCode {
            name: fn_name,
            params: param_tys,
            ret: ret_ty,
            is_method,
            n_captures: 0,
            regs,
            argv,
            labels,
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
            pools: Pools::new(),
            spans: Vec::new(),
            locals: Vec::new(),
            ret_ty: TY_NIL,
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
            emit_closure: false,
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
        let ret_ty = ret.map(|r| c.resolve_type_now(r)).or(saved.as_ref().map(|s| s.1)).unwrap_or(TY_NIL);
        c.ret_ty = ret_ty;
        // bind params then captures
        for (i, p) in params.iter().enumerate() {
            if let MemberKind::Param(ParamData { name, is_mut, .. }) = c.ctx.ast.param(*p) {
                let reg = c.new_reg(param_tys[i]);
                c.locals.push(Local { name: *name, reg, ty: param_tys[i], is_mut: *is_mut, loop_var: false, origins: Vec::new() });
            }
        }
        for (n, t) in &caps {
            let reg = c.new_reg(*t);
            c.locals.push(Local { name: *n, reg, ty: *t, is_mut: false, loop_var: false, origins: Vec::new() });
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
                    c.ctx.type_name(t), c.ctx.type_name(ret_ty)
                ));
            }
            c.emit(Op::Ret { val: Some(c.last_reg) }, 0);
        }
        c.resolve_labels();
        let (code, spans) = sroa::run(c.code, c.spans, &mut c.pools);
        let (code, spans, pools) = peephole::run(code, spans, c.pools);
        let Pools { argv, labels, .. } = pools;
        let regs = c.regs;
        let fc = rut_core::binary::FuncCode {
            name: c.ctx.intern(&format!("lambda@{}", body.id().0)),
            params: param_tys,
            ret: ret_ty,
            is_method: false,
            n_captures: n_caps,
            regs,
            argv,
            labels,
            code,
            spans,
            host: None,
        };
        let f = &mut c.ctx.funcs[fid as usize];
        *f = fc;
        Ok(())
    }

    /// Compile a desugared `for..of` emit closure (RFC 0012 §6): one
    /// parameter `v: E`, return `bool` — the body runs, then `true`;
    /// `break`/`continue` were translated to returns at their sites.
    fn compile_for_of_emit_fn(ctx: &mut Ctx<'a>, fid: u32, body: NodeId, var: IdentId) -> TcResult<()> {
        let Some((elem_ty, caps)) = ctx.for_of_sigs.get(&body.0).cloned() else {
            return Ok(()); // creation site already diagnosed
        };
        let mut c = FnCompiler {
            ctx,
            regs: Vec::new(),
            code: Vec::new(),
            pools: Pools::new(),
            spans: Vec::new(),
            locals: Vec::new(),
            ret_ty: TY_BOOL,
            self_ty: None,
            subst: vec![],
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
            emit_closure: true,
        };
        // the loop variable: the closure's parameter — a fresh binding
        // per iteration by construction (each emit call is a fresh frame)
        let reg = c.new_reg(elem_ty);
        c.locals.push(Local { name: var, reg, ty: elem_ty, is_mut: false, loop_var: false, origins: Vec::new() });
        for (n, t) in &caps {
            let reg = c.new_reg(*t);
            c.locals.push(Local { name: *n, reg, ty: *t, is_mut: false, loop_var: false, origins: Vec::new() });
        }
        let block: NodeHandle<BlockNode> = NodeHandle::new(body);
        if c.compile_block(block).is_err() {
            return Err(());
        }
        let t = c.new_reg(TY_BOOL);
        c.emit(Op::ConstRaw { dst: t, bits: 1 }, 0);
        c.emit(Op::Ret { val: Some(t) }, 0);
        c.resolve_labels();
        let (code, spans) = sroa::run(c.code, c.spans, &mut c.pools);
        let (code, spans, pools) = peephole::run(code, spans, c.pools);
        let Pools { argv, labels, .. } = pools;
        let mut param_tys = vec![elem_ty];
        for (_, t) in &caps {
            param_tys.push(*t);
        }
        let regs = c.regs;
        let fc = rut_core::binary::FuncCode {
            name: c.ctx.intern(&format!("forof@{}", body.0)),
            params: param_tys,
            ret: TY_BOOL,
            is_method: false,
            n_captures: caps.len() as u32,
            regs,
            argv,
            labels,
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
        // the file must stay below the NOREG sentinel (u16::MAX) — optional
        // operands use it as "no register" (rut_core::ops::NOREG)
        assert!(self.regs.len() < NOREG as usize, "register file overflow");
        self.regs.push(ty);
        let r = (self.regs.len() - 1) as u16;
        self.last_reg = r;
        r
    }

    /// Move a value into `dst` under v1.1 copy-by-value (RFC 0009/0016):
    /// value types (records, arrays) deep-copy — two bindings never alias;
    /// immutable sequences (`str`/`bytes`), pointers, enums, `Option`/
    /// `Result`, closures and boundary objects share the cell.
    pub(crate) fn mov_value(&mut self, dst: u16, src: u16, ty: TypeId, sp_lo: u32) {
        if self.ctx.types.is_value(ty) && !self.last_reg_is_fresh_value() {
            self.emit(Op::CloneVal { dst, src, ty }, sp_lo);
        } else if self.ctx.types.is_ref(ty) {
            self.emit(Op::MovRef { dst, src }, sp_lo);
        } else {
            self.emit(Op::Mov { dst, src }, sp_lo);
        }
    }

    /// True when `last_reg` was just written by a fresh-producing op (a
    /// call result, a record/array construction, a clone) — the value is
    /// already owned by the consumer, so a boundary clone is pure waste.
    fn last_reg_is_fresh_value(&self) -> bool {
        match self.code.last() {
            // calls: fresh only when the result is kept (`dst != NOREG`)
            Some(Op::Call { dst, .. } | Op::CallM { dst, .. } | Op::CallFn { dst, .. } | Op::CallNat { dst, .. }) => {
                *dst != NOREG
            }
            Some(
                Op::MakeRecord { .. }
                    | Op::MakePtr { .. }
                    | Op::CloneVal { .. }
                    | Op::ArrNew { .. }
                    | Op::ArrLit { .. }
                    | Op::MakeClosure { .. }
                    | Op::Box { .. },
            ) => true,
            _ => false,
        }
    }

    /// An argument register for a value-typed parameter: the callee gets
    /// its own deep copy (RFC 0009/0016 v1.1). Saves/restores `last_reg` —
    /// the convention must survive the clone's temporary.
    pub(crate) fn clone_arg(&mut self, reg: u16, pty: TypeId, sp_lo: u32) -> u16 {
        if self.ctx.types.is_value(pty) {
            let keep = self.last_reg;
            let tmp = self.new_reg(pty);
            self.emit(Op::CloneVal { dst: tmp, src: reg, ty: pty }, sp_lo);
            self.last_reg = keep;
            tmp
        } else {
            reg
        }
    }

    /// `p.m(..)` / `p[i]` auto-deref (RFC 0005): a pointer used as a
    /// receiver or indexee loads its pointee cell first. Returns the
    /// (type, register) to continue from.
    pub(crate) fn deref_for_use(&mut self, ty: TypeId, reg: u16, sp_lo: u32) -> (TypeId, u16) {
        match self.ctx.types.kind(ty).clone() {
            TyKind::Ptr { elem } => {
                let d = self.new_reg(elem);
                self.emit(Op::GetF { dst: d, obj: reg, field: 0, repr: Repr::Ref }, sp_lo);
                (elem, d)
            }
            _ => (ty, reg),
        }
    }

    /// Deref a pointer at a value use site: `v: *T` reads as `T` (RFC
    /// 0012 §6) — the payload slot at the pointee's own repr. Callers
    /// decide which positions deref: value-expected positions via the
    /// compile_expr funnel, arithmetic operands, and `==` against the
    /// pointee type. `p.f`/`p.m()` deref at their own sites and
    /// `*T == *T` stays identity (RFC 0012 §4).
    pub(crate) fn deref_ptr(&mut self, ty: TypeId, reg: u16, sp_lo: u32) -> (TypeId, u16) {
        if let TyKind::Ptr { elem } = self.ctx.types.kind(ty).clone() {
            let d = self.new_reg(elem);
            self.emit(Op::GetF { dst: d, obj: reg, field: 0, repr: self.ctx.types.repr_of(elem) }, sp_lo);
            return (elem, d);
        }
        (ty, reg)
    }

    /// Intern an argument list into the function's operand pool.
    pub(crate) fn pool_args(&mut self, a: &[Reg]) -> (u32, u16) {
        self.pools.args(a)
    }

    /// Intern `[recv] ++ args` (the callee's whole parameter list).
    pub(crate) fn pool_recv_args(&mut self, recv: Reg, a: &[Reg]) -> (u32, u16) {
        self.pools.recv_args(recv, a)
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
                Op::BrTable { table_off, count, default, .. } => {
                    if is_else {
                        *default = target;
                    } else if *count > 0 {
                        self.pools.labels[*table_off as usize] = target;
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
                if segs[0].name == sym::SELF_TY {
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

    /// The origin set of a binding (origin counting, RFC 0012 §5): the
    /// concrete types a trait-typed local is known to hold. Empty =
    /// unknown/multiple.
    pub(crate) fn origins_of(&self, name: IdentId) -> Vec<TypeId> {
        self.locals
            .iter()
            .rev()
            .find(|l| l.name == name)
            .map(|l| l.origins.clone())
            .unwrap_or_default()
    }

    /// Record a binding's origins after a write: a concrete value pins
    /// the origin; a trait-typed value from an untracked source erases
    /// it (branch merges, cross-function values — conservative).
    pub(crate) fn set_origins(&mut self, name: IdentId, origins: Vec<TypeId>) {
        if let Some(l) = self.locals.iter_mut().rev().find(|l| l.name == name) {
            l.origins = origins;
        }
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
