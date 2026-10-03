//! The function-compilation driver: `compile` dispatches an instantiation to the fused typecheck+emit walk (lambda and `for..of` emit closures get their own entries) — the entry the monomorphization queue (check/inst.rs) drives.

use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    /// Compile one instantiation into `ctx.funcs[fid]`.
    pub fn compile(ctx: &mut Ctx<'a>, inst: &Inst, fid: u32) -> TcResult<()> {
        // lambdas carry their capture list in ctx.lambda_info
        if let FnKey::Lambda(lambda_node) = inst.key {
            return Self::compile_lambda_fn(ctx, inst, fid, lambda_node);
        }
        // a desugared `for..of` emit closure carries its signature in
        // ctx.for_of_sigs
        if let FnKey::ForOfEmit { body, var } = inst.key {
            return Self::compile_for_of_emit_fn(ctx, fid, body, var);
        }
        // an engine-backed thunk: a bodyless FuncCode whose
        // host_id names the embedder's registered body
        if let FnKey::HostThunk(thunk_name) = inst.key {
            return super::asyncfn::compile_host_thunk(ctx, fid, thunk_name);
        }
        // an `async { }` block: the weave reads its mint plan (captures +
        // answer type) from ctx.async_block_sigs
        if let FnKey::AsyncBlock(node) = inst.key {
            return super::asyncfn::compile_async_block_fn(ctx, fid, node);
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
                let found = if let Some((_, d)) = ctx.datas.iter().find(|(n, _)| n == data) {
                    let d = d.clone();
                    let m = d.methods.iter().find(|(n, _)| n == name).map(|(_, n)| *n);
                    m.map(|m| {
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
                            ctx.mk_data_inst(*data, args, ctx.ast.span(m.id()))
                        };
                        (m.id(), Some(self_ty), true, Some(*data))
                    })
                } else if let Some((_, e)) = ctx.enums.iter().find(|(n, _)| n == data) {
                    // an enum's inherent method: concrete, no generics —
                    // `Self` is the enum's own type, no class context
                    // (only generic-class bounds care)
                    let e = e.clone();
                    e.methods
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, m)| (m.id(), Some(e.ty), true, None))
                } else {
                    None
                };
                let Some((node, self_ty, has_self, cname)) = found else {
                    return Ok(());
                };
                (node, self_ty, has_self, cname)
            }
            FnKey::Lambda(_) | FnKey::AsyncBlock(_) => unreachable!(),
            FnKey::HostThunk(_) => unreachable!(),
        };
        let is_async = match ctx.ast.kind(node) {
            Kind::Item(ItemKind::Fn(f)) => f.is_async,
            Kind::Member(MemberKind::MethodDecl(m)) => m.is_async,
            _ => return Ok(()),
        };
        if is_async {
            // the landing: free async fns weave into the
            // engine-backed Future impls; async METHODS diagnose (the
            // loop's tasks are free fns in v1 — join lands later)
            if !matches!(inst.key, FnKey::Free(_)) {
                ctx.err(
                    ctx.ast.span(node),
                    "async methods are not woven in this build — the loop's tasks are async free fns",
                );
                return Err(());
            }
            return super::asyncfn::compile_async_fn(ctx, inst, fid);
        }
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
        // Type-union bounds in scope for THIS instantiation (native-fastpath
        // phase 1): the item's own bounds plus — for a
        // class method — the class's `requires`. Only union-SPELLED
        // bounds register: a trait bound stays admission-only, and its
        // method calls keep resolving per instantiation (the
        // single-trait `K requires Trait` law).
        let mut union_bounds: std::collections::HashMap<IdentId, NodeHandle<AnyTy>> =
            std::collections::HashMap::new();
        match ctx.ast.kind(node) {
            Kind::Item(ItemKind::Fn(f)) => {
                let bounds = f.bounds.clone();
                for (g, b) in bounds {
                    if ctx.bound_is_union(b) {
                        union_bounds.insert(g, b);
                    }
                }
            }
            Kind::Member(MemberKind::MethodDecl(m)) => {
                let mbounds = m.bounds.clone();
                for (g, b) in mbounds {
                    if ctx.bound_is_union(b) {
                        union_bounds.insert(g, b);
                    }
                }
            }
            _ => {}
        }
        // the class's own bounds when this method is one of a generic
        // class's (its methods or a trait impl over the class)
        if let Some(cn) = class_name {
            let crequires = ctx.find_data(cn).map(|d| d.requires.clone());
            if let Some(rs) = crequires {
                for (g, b) in rs {
                    if ctx.bound_is_union(b) {
                        union_bounds.insert(g, b);
                    }
                }
            }
        }
        // interface bounds in scope (the mirror-body law's lookup): a
        // NON-union bound naming an interface arms param ident →
        // interface id, so a member call on a carried instantiation
        // binds through the bound's descriptor
        let mut iface_bounds: std::collections::HashMap<IdentId, Vec<u32>> =
            std::collections::HashMap::new();
        {
            let bounds: Vec<(IdentId, NodeHandle<AnyTy>)> = match ctx.ast.kind(node) {
                Kind::Item(ItemKind::Fn(f)) => f.bounds.clone(),
                Kind::Member(MemberKind::MethodDecl(m)) => m.bounds.clone(),
                _ => Vec::new(),
            };
            for (g, b) in &bounds {
                let members = ctx.resolve_bound_members(*b, &[]);
                for m in &members {
                    if let crate::check::BoundMember::Trait(tid) = m {
                        iface_bounds.entry(*g).or_default().push(*tid);
                    }
                }
            }
            if let Some(cn) = class_name {
                let rs = ctx.find_data(cn).map(|d| d.requires.clone()).unwrap_or_default();
                for (g, b) in &rs {
                    let members = ctx.resolve_bound_members(*b, &[]);
                    for m in &members {
                        if let crate::check::BoundMember::Trait(tid) = m {
                            iface_bounds.entry(*g).or_default().push(*tid);
                        }
                    }
                }
            }
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
            union_bounds,
            iface_bounds,
            union_syms: std::collections::HashMap::new(),
        async_frame: None,
        assigned: std::collections::HashSet::new(),
        captured: std::collections::HashSet::new(),
        cell_counter: 0,
        async_infer: false,
        async_founds: Vec::new(),
        };
        // the capture law's pre-pass: names first (a fn-level walk over
        // the body), promotion at each `bind_local` once types resolve
        c.capture_pre_pass(body);
        // signature: params (self first for methods), resolved under subst.
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
        let mut param_origins = inst.iface_origins.clone();
        let mut param_origins_iter = param_origins.drain(..);
        // bind params as locals — argv maps positionally onto the callee's
        // registers, so the parameter registers stay contiguous here
        for (i, p) in params.iter().enumerate() {
            match c.ctx.ast.param(*p) {
                MemberKind::SelfParam(SelfParamData { is_mut }) => {
                    let reg = c.new_reg(param_tys[i]);
                    // bind `self` (well-known symbol): the local is only ever
                    // FOUND where the body spells `self`, so binding it
                    // unconditionally is safe
                    c.locals.push(Local {
                        name: sym::SELF,
                        reg,
                        ty: param_tys[i],
                        is_mut: *is_mut,
                        loop_var: false,
                        origins: Vec::new(),
                        field: NO_FIELD,
                        cell: None,
                    });
                }
                MemberKind::Param(ParamData { name, is_mut, .. }) => {
                    let reg = c.new_reg(param_tys[i]);
                    // trait-typed parameters: the Inst carries this call's
                    // concrete origin (a trait parameter IS an implicit
                    // generic bound) — single origin ⇒ static
                    let origins = if matches!(c.ctx.types.kind(param_tys[i]), TyKind::IfaceObj { .. }) {
                        param_origins_iter.next().map(|t| vec![t]).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    // union-bound provenance: a param whose
                    // declared type NODE spells a union-bounded generic
                    // records that generic — method calls on it gate on
                    // every member of the bound
                    if let MemberKind::Param(ParamData { ty: Some(tn), .. }) = c.ctx.ast.param(*p) {
                        c.note_union_binding(*name, Some(*tn));
                    }
                    c.locals.push(Local {
                        name: *name,
                        reg,
                        ty: param_tys[i],
                        is_mut: *is_mut,
                        loop_var: false,
                        origins,
                        field: NO_FIELD,
                        cell: None,
                    });
                }
                _ => {}
            }
        }
        // the capture law: params promote too (a captured param
        // reassigned anywhere in the fn shares its slot).
        for li in 0..c.locals.len() {
            c.promote_param(li, 0);
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
        let (code, spans) = sroa::run(c.code, c.spans, &mut c.pools, &c.ctx.disposal_dense_set());
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
            // pc → (line, col) is filled by the DRIVER, while the module
            // source is in hand — the compiler's span table stays
            // byte-offset-only
            pos: vec![],
            host_id: None,
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
            union_bounds: std::collections::HashMap::new(),
            iface_bounds: std::collections::HashMap::new(),
            union_syms: std::collections::HashMap::new(),
        async_frame: None,
        assigned: std::collections::HashSet::new(),
        captured: std::collections::HashSet::new(),
        cell_counter: 0,
        async_infer: false,
        async_founds: Vec::new(),
        };
        // NOTE: lambda param/ret types were recorded... re-derive:
        // annotations resolve here; unannotated ones took the expected type
        // at the creation site —store those too. For M1 we re-resolve and
        // accept that an unannotated param's type must be reconstructible;
        // the creation site stored only captures. To stay correct, the
        // creation site also stores the resolved param/ret types:
        let saved = c.ctx.lambda_sigs.get(&lambda_node).cloned();
        // the creation site's substitution re-arms this compiler's type
        // env: a lambda inside a generic fn/method resolves annotations
        // against the enclosing generics
        c.subst = saved.as_ref().map(|s| s.2.clone()).unwrap_or_default();
        // the creation site resolved every annotation under ITS
        // substitution — inside a generic fn/method a lambda body
        // re-resolves under an empty env (the lambda's Inst is
        // unit-local), so the stored types are the only ones that name
        // the enclosing generics. Prefer them; re-resolution is the
        // fallback (and agrees wherever the env is empty anyway).
        let mut param_tys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let ty = match c.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                    saved.as_ref().and_then(|s| s.0.get(i).copied()).unwrap_or_else(|| c.resolve_type_now(*t))
                }
                _ => saved.as_ref().and_then(|s| s.0.get(i).copied()).unwrap_or(TY_I32),
            };
            param_tys.push(ty);
        }
        let ret_ty = saved.as_ref().map(|s| s.1)
            .or_else(|| ret.map(|r| c.resolve_type_now(r)))
            .unwrap_or(TY_NIL);
        c.ret_ty = ret_ty;
        // the lambda body's own pre-pass: this frame promotes its own
        // bindings (nested closure sites under the body)
        c.capture_pre_pass(body.id());
        // bind params then captures
        for (i, p) in params.iter().enumerate() {
            if let MemberKind::Param(ParamData { name, is_mut, .. }) = c.ctx.ast.param(*p) {
                let reg = c.new_reg(param_tys[i]);
                c.locals.push(Local { name: *name, reg, ty: param_tys[i], is_mut: *is_mut, loop_var: false, origins: Vec::new(), field: NO_FIELD, cell: None });
            }
        }
        for (cap) in &caps {
            let Capture { name: n, ty: t, is_mut: m, cell: by_cell } = *cap;
            // a promoted capture's parameter carries the SHARED cell's
            // handle — the local re-binds cell-backed so both frames
            // route through one slot (stay-linked for the binding's scope)
            let reg = c.new_reg(by_cell.unwrap_or(t));
            c.locals.push(Local { name: n, reg, ty: t, is_mut: m, loop_var: false, origins: Vec::new(), field: NO_FIELD, cell: by_cell });
            // captures are part of the fn's parameter list (after declared)
            param_tys.push(by_cell.unwrap_or(t));
        }
        let n_caps = caps.len() as u32;
        // body: block or single expression (arrows)
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
        let (code, spans) = sroa::run(c.code, c.spans, &mut c.pools, &c.ctx.disposal_dense_set());
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
            // pc → (line, col) is filled by the DRIVER, while the module
            // source is in hand — the compiler's span table stays
            // byte-offset-only
            pos: vec![],
            host_id: None,
        };
        let f = &mut c.ctx.funcs[fid as usize];
        *f = fc;
        Ok(())
    }

    /// Compile a desugared `for..of` emit closure: one
    /// parameter `v: E`, return `bool` — the body runs, then `true`;
    /// `break`/`continue` were translated to returns at their sites.
    ///
    /// Under the sugar law the loop var is ONE variable reassigned per
    /// iteration: when the loop var itself is captured, the closure
    /// binds it cell-backed on the shared cell (the creation site
    /// pooled it) and stores the incoming element into the cell at
    /// frame entry — a stashed inner lambda sees the value current at
    /// call time, exactly like the fused `while`-shaped form.
    fn compile_for_of_emit_fn(ctx: &mut Ctx<'a>, fid: u32, body: NodeId, var: IdentId) -> TcResult<()> {
        let Some((elem_ty, caps, var_cell)) = ctx.for_of_sigs.get(&body.0).cloned() else {
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
            union_bounds: std::collections::HashMap::new(),
            iface_bounds: std::collections::HashMap::new(),
            union_syms: std::collections::HashMap::new(),
        async_frame: None,
        assigned: std::collections::HashSet::new(),
        captured: std::collections::HashSet::new(),
        cell_counter: 0,
        async_infer: false,
        async_founds: Vec::new(),
        };
        // the emit closure's own pre-pass: its frame promotes its own
        // bindings (nested lambdas / for-of bodies under the loop body)
        c.capture_pre_pass(body);
        // the incoming element: the desugar's per-iteration store source
        let preg = c.new_reg(elem_ty);
        if let Some(cty) = var_cell {
            // the loop var, cell-backed on the SHARED cell (argv[1]);
            // frame entry stores the element into it
            let crec = c.new_reg(cty);
            c.locals.push(Local { name: var, reg: crec, ty: elem_ty, is_mut: false, loop_var: true, origins: Vec::new(), field: NO_FIELD, cell: Some(cty) });
            let repr = c.ctx.types.repr_of(elem_ty);
            c.emit(Op::SetF { obj: crec, field: 0, val: preg, repr }, 0);
        } else {
            // the loop variable: the closure's parameter — a fresh binding
            // per iteration by construction (each emit call is a fresh frame);
            // loop-owned, so writes through it (the shared element) are legal
            c.locals.push(Local { name: var, reg: preg, ty: elem_ty, is_mut: false, loop_var: true, origins: Vec::new(), field: NO_FIELD, cell: None });
        }
        for (cap) in &caps {
            let Capture { name: n, ty: t, is_mut: m, cell: by_cell } = *cap;
            let reg = c.new_reg(by_cell.unwrap_or(t));
            c.locals.push(Local { name: n, reg, ty: t, is_mut: m, loop_var: false, origins: Vec::new(), field: NO_FIELD, cell: by_cell });
        }
        let block: NodeHandle<BlockNode> = NodeHandle::new(body);
        if c.compile_block(block).is_err() {
            return Err(());
        }
        let t = c.new_reg(TY_BOOL);
        c.emit(Op::ConstRaw { dst: t, bits: 1 }, 0);
        c.emit(Op::Ret { val: Some(t) }, 0);
        c.resolve_labels();
        let (code, spans) = sroa::run(c.code, c.spans, &mut c.pools, &c.ctx.disposal_dense_set());
        let (code, spans, pools) = peephole::run(code, spans, c.pools);
        let Pools { argv, labels, .. } = pools;
        let mut param_tys = vec![elem_ty];
        if let Some(cty) = var_cell {
            param_tys.push(cty);
        }
        for (cap) in &caps {
            param_tys.push(cap.cell.unwrap_or(cap.ty));
        }
        let regs = c.regs;
        let fc = rut_core::binary::FuncCode {
            name: c.ctx.intern(&format!("forof@{}", body.0)),
            params: param_tys,
            ret: TY_BOOL,
            is_method: false,
            n_captures: caps.len() as u32 + var_cell.iter().map(|_| 1usize).sum::<usize>() as u32,
            regs,
            argv,
            labels,
            code,
            spans,
            // pc → (line, col) is filled by the DRIVER, while the module
            // source is in hand — the compiler's span table stays
            // byte-offset-only
            pos: vec![],
            host_id: None,
        };
        let f = &mut c.ctx.funcs[fid as usize];
        *f = fc;
        Ok(())
    }
}
