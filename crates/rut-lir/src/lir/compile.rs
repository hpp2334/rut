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
        let (node, self_ty, is_method, class_name, slot_self) = match &inst.key {
            // handled by the early return above
            FnKey::ForOfEmit { .. } => unreachable!(),
            FnKey::Free(name) => {
                let Some(n) = ctx.fn_nodes.iter().find(|(n, _)| n == name).map(|(_, n)| *n) else {
                    return Ok(()); // unknown fn —already diagnosed
                };
                (n.id(), None, false, None, None)
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
                    ctx.mk_data_inst(*data, args, ctx.ast.span(m.id()))
                };
                (m.id(), Some(self_ty), true, Some(*data), None)
            }
            FnKey::ImplMethod { idx, name, slot_abi } => {
                let im = ctx.impls[*idx].clone();
                let Some(m) = im.methods.iter().find(|(n, _)| n == name).map(|(_, n)| *n) else {
                    return Ok(());
                };
                // self: the concrete target — a generic target instantiates
                // under the Inst substitution; a native builtin target
                // rebuilds its shape (`Array<T>`). A FOREIGN generic's
                // instantiation mirrors through the extern path (the
                // owner compiles the body, the self row is the mirror)
                let (self_ty, cname) = match &im.target_data {
                    Some((dname, params))
                        if dname == &sym::ARRAY && params.len() == 1 =>
                    {
                        let elem = inst
                            .subst
                            .iter()
                            .find(|(n, _)| n == &params[0])
                            .map(|(_, t)| *t)
                            .unwrap_or(TY_I32);
                        (ctx.mk_array(elem), None)
                    }
                    Some((dname, params))
                        if dname == &sym::OPT && params.len() == 1 =>
                    {
                        // `impl I for ?T` — self is the nullable itself
                        // (the rut-json batch phase 1: the body checks
                        // nil and derefs explicitly; the template's
                        // element instantiation comes from the subst)
                        let elem = inst
                            .subst
                            .iter()
                            .find(|(n, _)| n == &params[0])
                            .map(|(_, t)| *t)
                            .unwrap_or(TY_I32);
                        (ctx.mk_opt(elem), None)
                    }
                    Some((dname, params)) => {
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
                        (ctx.mk_data_inst(*dname, args, ctx.ast.span(m.id())), Some(*dname))
                    }
                    _ => {
                        let cname = ctx.datas.iter().find(|(_, d)| d.ty == im.target).map(|(n, _)| *n);
                        (im.target, cname)
                    }
                };
                // a PRIMITIVE target compiles in one of two ABIs (P1,
                // mapset perf plan). SLOT ABI (`slot_abi`): `self` and
                // every `Self`-spelled parameter cross as the trait-object
                // slot (concrete scalars arrive boxed — `widen_to_slot`),
                // and the prologue unboxes into the concrete working
                // registers the body is typed against — this is the
                // variant vtable rows bind (the box cell's own type
                // reaches the vtable). CONCRETE ABI: params
                // cross raw and there is no prologue — body codegen
                // identical to an inherent fn; bare-receiver static
                // calls bind this variant. Ref targets
                // (records/`str`/`bytes`) compile one variant — their
                // signature stays the concrete type (already cell
                // handles).
                let slot_self = if *slot_abi
                    && !im.inherent
                    && matches!(ctx.types.kind(im.target), TyKind::Prim(_))
                {
                    Some(ctx.mk_trait_obj(im.trait_id))
                } else {
                    None
                };
                (m.id(), Some(self_ty), true, cname, slot_self)
            }
            FnKey::Lambda(_) => unreachable!(),
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
            union_syms: std::collections::HashMap::new(),
        async_frame: None,
        };
        // signature: params (self first for methods), resolved under subst.
        // Under the slot ABI (`slot_self`, a prim-target impl method) the
        // receiver and every `Self`-spelled parameter cross as the slot.
        let spells_self = |c: &FnCompiler, t: &NodeHandle<AnyTy>| matches!(
            c.ctx.ast.ty(*t),
            TypeKind::TyPath { ref segs, .. }
                if segs.len() == 1 && segs[0].generics.is_empty() && segs[0].name == sym::SELF_TY
        );
        let mut param_tys: Vec<TypeId> = Vec::new();
        let mut param_is_slot: Vec<bool> = Vec::new();
        for p in &params {
            let (ty, is_slot) = match c.ctx.ast.param(*p) {
                MemberKind::SelfParam(_) => (
                    slot_self.unwrap_or(self_ty.unwrap_or(TY_NIL)),
                    slot_self.is_some(),
                ),
                MemberKind::Param(ParamData { ty: Some(t), .. }) => {
                    let ty = c.resolve_type_now(*t);
                    let is_slot = slot_self.is_some() && spells_self(&c, t);
                    (if is_slot { slot_self.unwrap() } else { ty }, is_slot)
                }
                MemberKind::Param(ParamData { ty: None, name, .. }) => {
                    c.ctx.err(
                        c.ctx.ast.span(p.id()),
                        format!(
                            "parameter `{}` needs a type annotation here",
                            c.ctx.name(*name)
                        ),
                    );
                    (TY_I32, false)
                }
                _ => (TY_I32, false),
            };
            param_tys.push(ty);
            param_is_slot.push(is_slot);
        }
        // ret
        let ret_ty = ret.map(|r| c.resolve_type_now(r)).unwrap_or(TY_NIL);
        c.ret_ty = ret_ty;
        // this instantiation's concrete origins for the fn's trait-typed
        // parameters, in declaration order
        let mut param_origins = inst.trait_origins.clone();
        let mut param_origins_iter = param_origins.drain(..);
        // bind params as locals — argv maps positionally onto the callee's
        // registers, so the parameter registers stay contiguous here; the
        // slot-ABI unboxes run in a second pass below
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
                    });
                }
                MemberKind::Param(ParamData { name, is_mut, .. }) => {
                    let reg = c.new_reg(param_tys[i]);
                    // trait-typed parameters: the Inst carries this call's
                    // concrete origin (a trait parameter IS an implicit
                    // generic bound) — single origin ⇒ static
                    let origins = if matches!(c.ctx.types.kind(param_tys[i]), TyKind::TraitObj { .. }) {
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
                    });
                }
                _ => {}
            }
        }
        // slot-ABI prologue (prim-target impl methods): each slot
        // parameter unboxes into the concrete working register the body
        // was typed against — the box cell's own type reaches the vtable,
        // the payload is the body's value
        if slot_self.is_some() {
            let concrete = self_ty.unwrap_or(TY_NIL);
            for (i, p) in params.iter().enumerate() {
                if !param_is_slot[i] {
                    continue;
                }
                let name = match c.ctx.ast.param(*p) {
                    MemberKind::SelfParam(_) => sym::SELF,
                    MemberKind::Param(ParamData { name, .. }) => *name,
                    _ => continue,
                };
                let li = c.locals.iter().position(|l| l.name == name).expect("param local");
                let preg = c.locals[li].reg;
                let u = c.new_reg(concrete);
                c.emit(Op::Unbox { dst: u, box_: preg, ty: concrete }, 0);
                c.locals[li].reg = u;
                c.locals[li].ty = concrete;
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
            union_syms: std::collections::HashMap::new(),
        async_frame: None,
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
                c.locals.push(Local { name: *name, reg, ty: param_tys[i], is_mut: *is_mut, loop_var: false, origins: Vec::new(), field: NO_FIELD });
            }
        }
        for (n, t, m) in &caps {
            let reg = c.new_reg(*t);
            c.locals.push(Local { name: *n, reg, ty: *t, is_mut: *m, loop_var: false, origins: Vec::new(), field: NO_FIELD });
            // captures are part of the fn's parameter list (after declared)
            param_tys.push(*t);
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
            union_bounds: std::collections::HashMap::new(),
            union_syms: std::collections::HashMap::new(),
        async_frame: None,
        };
        // the loop variable: the closure's parameter — a fresh binding
        // per iteration by construction (each emit call is a fresh frame);
        // loop-owned, so writes through it (the shared element) are legal
        let reg = c.new_reg(elem_ty);
        c.locals.push(Local { name: var, reg, ty: elem_ty, is_mut: false, loop_var: true, origins: Vec::new(), field: NO_FIELD });
        for (n, t, m) in &caps {
            let reg = c.new_reg(*t);
            c.locals.push(Local { name: *n, reg, ty: *t, is_mut: *m, loop_var: false, origins: Vec::new(), field: NO_FIELD });
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
        for (_, t, _) in &caps {
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
