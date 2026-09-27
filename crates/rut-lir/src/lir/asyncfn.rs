//! The async weave (RFC 0018): an `async fn(cx: RunContext, ..)` compiles
//! into the engine-woven `impl Future<T>` for a hidden frame type — the
//! frame IS the coroutine, the body becomes its `yield`.
//!
//! The woven FuncCode (one per async fn, existing ops only):
//!
//! ```text
//! entry:  jmp l_dispatch
//! s0:     cancelled-probe → drop path | restore locals → BODY
//!            … await expansion at each await site:
//!               getf  state←fut          ; the done probe (null = done)
//!               br    state → drive, ready
//!            drive:  calli fut.yield(cx)  ; cold-poll one step
//!               getf  state←fut
//!               br    state → park, ready
//!            park:  setf fut.awaiter←frame ; setf frame.pending←fut
//!               enumnew sK ; setf frame.state←sK
//!               ret                       ; suspension — the frame parks
//!            sK:  cancelled-probe → drop path | clear pending edges
//!               → restore locals → the continuation
//!            drop: setf state←null ; drop ref locals (reverse order) ; ret
//! done:   setf state←null                  ; completion discards the value
//!         ret
//! l_retire: ret                            ; a re-drive of a finished frame
//! l_dispatch: getf state←frame ; brtable state [s0..sK] default l_retire
//! ```
//!
//! Laws this file implements: the cx-per-frame protocol (the frame's
//! state field IS the checkpoint; the driving loop re-mints the cx per
//! drive), cell-backed locals (every binding mirrors into the frame
//! cell — `bind_local`/`mirror_local` — so the park's `Ret` releases
//! only staging registers), and cancellation as the context probe
//! (abort flags the frame; the resumed probe at its checkpoint runs the
//! drop path in RFC 0016 §3 order — reverse binding order).

use super::*;
use crate::check::ImplDecl;
use rut_core::async_frame as af;
use rut_core::ops::Op;

type Layout = crate::check::AsyncLayout;

/// Mint the async machinery of one async fn (idempotent per fid): the
/// hidden frame type, the checkpoint enum, the `Future<ret>`
/// instantiation, the compiler-written impl row (empty — the vtable
/// fill carries the row), and the yield slot.
pub(crate) fn ensure_layout(ctx: &mut Ctx, fid: u32, fname: IdentId, ret_ty: TypeId, param_fields: Vec<(IdentId, TypeId)>) -> Layout {
    if let Some(l) = ctx.async_layout.get(&fid) {
        return *l;
    }
    let ckpt_name = ctx.intern(&format!("{}{}", af::CKPT_PREFIX, ctx.name(fname)));
    let s0 = ctx.intern("#s0");
    let ckpt_ty = ctx.types.intern(RutType {
        name: ckpt_name,
        kind: TyKind::Enum { members: vec![(s0, 0)] },
    });
    let frame_name = ctx.intern(&format!("{}{}", af::FRAME_PREFIX, ctx.name(fname)));
    let mut fields = vec![
        FieldInfo { name: ctx.intern("state"), ty: ckpt_ty },
        FieldInfo { name: ctx.intern("cancelled"), ty: TY_BOOL },
        FieldInfo { name: ctx.intern("awaiter"), ty: TY_OPAQUE },
        FieldInfo { name: ctx.intern("pending"), ty: TY_OPAQUE },
    ];
    for (n, t) in &param_fields {
        fields.push(FieldInfo { name: *n, ty: *t });
    }
    let frame_ty = ctx.types.intern(RutType {
        name: frame_name,
        kind: TyKind::Data { fields },
    });
    // the compiler-written impl: `impl Future<ret> for #frame@<fn>` —
    // an ordinary ImplDecl row (dispatch, widening, `is` all see it);
    // the vtable row itself rides the extra fill (no AST method nodes)
    let fut_inst = ctx.mk_future_inst(sym::FUTURE, ret_ty);
    ctx.impls.push(ImplDecl {
        trait_id: fut_inst,
        trait_name: sym::FUTURE,
        target: frame_ty,
        target_data: None,
        trait_arg_nodes: vec![],
        is_template: false,
        inherent: false,
        methods: vec![],
    });
    let slot = ctx.trait_slot(fut_inst, 0).expect("Future has exactly one member");
    ctx.extra_vtable_fills.push((frame_ty, slot, fid));
    ctx.frame_yield_slot.insert(frame_ty, slot);
    ctx.async_fns.insert(fname);
    let l = Layout { frame_ty, ckpt_ty, fut_inst, yield_slot: slot };
    ctx.async_layout.insert(fid, l);
    l
}

/// Patch the checkpoint enum after the body compiles: one member per
/// state (`s0` + one per await). In-place — the type id is stable, and
/// `NewCell` reads the frame's final field list at runtime.
pub(crate) fn patch_members(ctx: &mut Ctx, ckpt_ty: TypeId, states: u32) {
    let di = ctx.types.dense(ckpt_ty) as usize;
    let mut members = Vec::new();
    for i in 0..states {
        members.push((ctx.intern(&format!("#s{i}")), i as i64));
    }
    ctx.types.types[di].kind = TyKind::Enum { members };
}

/// Append a binding's field to the frame type (the `mk_data_inst`
/// in-place patch law) — the runtime allocation grows with the body.
pub(crate) fn append_frame_field(ctx: &mut Ctx, frame_ty: TypeId, name: IdentId, ty: TypeId) {
    let di = ctx.types.dense(frame_ty) as usize;
    if let TyKind::Data { fields } = &mut ctx.types.types[di].kind {
        fields.push(FieldInfo { name, ty });
    }
}

/// Compile one async fn instantiation into `ctx.funcs[fid]`: the weave
/// sketched in the module docs. Dispatched from `FnCompiler::compile`
/// when the fn node is `is_async`.
pub(crate) fn compile_async_fn(ctx: &mut Ctx, inst: &Inst, fid: u32) -> TcResult<()> {
    let FnKey::Free(name) = inst.key else {
        unreachable!("async dispatch only queues free fns")
    };
    let Some(node) = ctx.fn_nodes.iter().find(|(n, _)| *n == name).map(|(_, n)| *n) else {
        return Ok(()); // unknown fn — already diagnosed
    };
    let Kind::Item(ItemKind::Fn(f)) = ctx.ast.kind(node.id()) else {
        return Ok(());
    };
    let f = f.clone();
    let sp = ctx.ast.span(node.id());
    if !f.generics.is_empty() {
        ctx.err(sp, "generic async fns are not woven in this build — RFC 0019's join generalizes the machinery (v1: monomorphic async fns)");
        return Err(());
    }
    // the frozen cx law (RFC 0012 §7): the first parameter IS the cx
    let Some(first) = f.params.first() else {
        ctx.err(sp, "an async fn takes `cx: RunContext` as its first parameter (RFC 0012 §7) — the engine mints it at call sites");
        return Err(());
    };
    let cx_param = match ctx.ast.param(*first) {
        MemberKind::Param(ParamData { ty: Some(t), name, .. }) => Some((*t, *name)),
        _ => None,
    };
    let Some((cx_ty_node, cx_name)) = cx_param else {
        ctx.err(sp, "an async fn takes `cx: RunContext` as its first parameter (RFC 0012 §7) — the engine mints it at call sites");
        return Err(());
    };
    let cx_ty = ctx.resolve_type(cx_ty_node, &[]);
    if cx_ty != ctx.run_context_ty() {
        ctx.err(sp, format!(
            "an async fn's first parameter is `{}` — `cx: RunContext` expected (RFC 0012 §7)",
            ctx.type_name(cx_ty)
        ));
        return Err(());
    }
    let ret_ty = f.ret.map(|r| ctx.resolve_type(r, &[])).unwrap_or(TY_NIL);
    // user params (after the cx) become the frame's first cell fields;
    // the yield's argv carries only (frame, cx) — the call site writes
    // the params into the frame before it is ever driven
    let mut param_fields: Vec<(IdentId, TypeId)> = Vec::new();
    for p in &f.params[1..] {
        if let MemberKind::Param(ParamData { ty: Some(t), name, .. }) = ctx.ast.param(*p) {
            param_fields.push((*name, ctx.resolve_type(*t, &[])));
        } else {
            ctx.err(ctx.ast.span(p.id()), "parameter needs a type annotation");
            return Err(());
        }
    }
    let layout = ensure_layout(ctx, fid, name, ret_ty, param_fields.clone());
    let cx_ty = ctx.run_context_ty();
    let frame_local = ctx.intern("#frame");

    let mut c = FnCompiler {
        ctx,
        regs: Vec::new(),
        code: Vec::new(),
        pools: Pools::new(),
        spans: Vec::new(),
        locals: Vec::new(),
        ret_ty: TY_NIL,
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
        emit_closure: false,
        union_bounds: std::collections::HashMap::new(),
        union_syms: std::collections::HashMap::new(),
        async_frame: None,
    };
    // argv pair: reg 0 the frame, reg 1 the cx (the FuncCode params are
    // exactly these two — the driving loop supplies both per drive)
    let frame_reg = c.new_reg(layout.frame_ty);
    let cx_reg = c.new_reg(cx_ty);
    let state_reg = c.new_reg(layout.ckpt_ty);
    c.locals.push(Local {
        name: frame_local,
        reg: frame_reg,
        ty: layout.frame_ty,
        is_mut: false,
        loop_var: false,
        origins: Vec::new(),
        field: NO_FIELD,
    });
    // the cx binding: user-visible by its declared name, register-live
    // only — each drive mints a fresh cx, so it is never cell-backed
    c.locals.push(Local {
        name: cx_name,
        reg: cx_reg,
        ty: cx_ty,
        is_mut: false,
        loop_var: false,
        origins: Vec::new(),
        field: NO_FIELD,
    });
    // the user params: fields LOCALS_BASE.. — restore at every arm
    for (i, (pname, pty)) in param_fields.iter().enumerate() {
        let reg = c.new_reg(*pty);
        let field = af::LOCALS_BASE + i as u32;
        c.locals.push(Local {
            name: *pname,
            reg,
            ty: *pty,
            is_mut: true,
            loop_var: false,
            origins: Vec::new(),
            field,
        });
    }
    c.async_frame = Some(AsyncFrame {
        frame_reg,
        cx_reg,
        frame_ty: layout.frame_ty,
        ckpt_ty: layout.ckpt_ty,
        next_state: 1,
        next_field: af::LOCALS_BASE + param_fields.len() as u32,
        arm_labels: Vec::new(),
    });

    // entry: the dispatch lives at the END (the arm labels exist by
    // then); pc0 jumps to it
    let l_dispatch = c.new_label();
    let l_retire = c.new_label();
    let l_s0 = c.new_label();
    c.jmp(l_dispatch);

    // s0 — the fresh start: the probe fires here too, so an abort that
    // lands before the first drive never runs the body
    c.bind(l_s0);
    let l_drop0 = c.new_label();
    let l_live0 = c.new_label();
    emit_cancelled_probe(&mut c, sp.lo, l_drop0, l_live0);
    c.bind(l_live0);
    emit_restore(&mut c, sp.lo);
    if c.compile_block(f.body).is_err() {
        return Err(());
    }
    // completion — the value is discarded (no handle can receive it in v1)
    emit_completion(&mut c, sp.lo);
    // the s0 drop path — an abort that lands before the first drive (or
    // while parked at state s0) never runs the body
    c.bind(l_drop0);
    emit_drop_path(&mut c, sp.lo);
    c.bind(l_retire);
    c.emit(Op::Ret { val: None }, sp.lo);

    // dispatch — the resume law: the state field names the arm
    c.bind(l_dispatch);
    c.emit(
        Op::GetF {
            dst: state_reg,
            obj: frame_reg,
            field: af::STATE_FIELD,
            repr: c.ctx.types.repr_of(layout.ckpt_ty),
        },
        sp.lo,
    );
    let mut table_labels = vec![l_s0];
    let arm_labels = c.async_frame.as_ref().map(|f| f.arm_labels.clone()).unwrap_or_default();
    table_labels.extend(arm_labels.iter().copied());
    let (table_off, count) = c.pools.table(&table_labels);
    {
        let brt = c.code.len();
        c.fixups.push((brt, l_retire, true));
        c.emit(Op::BrTable { idx: state_reg, table_off, count, default: 0 }, sp.lo);
    }
    // well-formedness (RFC 0033 §2): the fn ends in Ret — the dispatch
    // always jumps, this is the unreachable tail
    c.emit(Op::Ret { val: None }, sp.lo);

    c.resolve_labels();
    // the brtable's arms: fill the table pool directly (the fixup
    // contract fills one entry per fixup — the weave owns its table)
    for (i, l) in table_labels.iter().enumerate() {
        let target = c.labels[*l as usize].unwrap_or(0);
        c.pools.labels[table_off as usize + i] = target;
    }
    patch_members(c.ctx, layout.ckpt_ty, table_labels.len() as u32);

    let (code, spans) = sroa::run(c.code, c.spans, &mut c.pools);
    let (code, spans, pools) = peephole::run(code, spans, c.pools);
    let Pools { argv, labels, .. } = pools;
    let regs = c.regs;
    let cx_ty = c.ctx.run_context_ty();
    let fc = rut_core::binary::FuncCode {
        name,
        params: vec![layout.frame_ty, cx_ty],
        ret: TY_NIL,
        is_method: false,
        n_captures: 0,
        regs,
        argv,
        labels,
        code,
        spans,
        pos: vec![],
        host_id: None,
    };
    let f2 = &mut c.ctx.funcs[fid as usize];
    *f2 = fc;
    Ok(())
}

/// The cancelled probe at an arm: the frame's flag (read directly —
/// the cx protocol's `cancelled()` view of it) branches to the drop
/// path or the live continuation.
fn emit_cancelled_probe(c: &mut FnCompiler, sp_lo: u32, l_drop: u32, l_live: u32) {
    let frame_reg = c.async_frame.as_ref().map(|f| f.frame_reg).unwrap_or(0);
    let flag = c.new_reg(TY_BOOL);
    c.emit(
        Op::GetF { dst: flag, obj: frame_reg, field: af::CANCELLED_FIELD, repr: c.ctx.types.repr_of(TY_BOOL) },
        sp_lo,
    );
    c.br(flag, l_drop, l_live);
}

/// Restore every cell-backed local in scope from the frame cell — the
/// uniform law: no liveness analysis, the fields ARE the storage.
fn emit_restore(c: &mut FnCompiler, sp_lo: u32) {
    let frame_reg = c.async_frame.as_ref().map(|f| f.frame_reg).unwrap_or(0);
    let backs: Vec<(u16, u32, TypeId)> = c
        .locals
        .iter()
        .filter(|l| l.field != NO_FIELD)
        .map(|l| (l.reg, l.field, l.ty))
        .collect();
    for (reg, field, ty) in backs {
        let repr = c.ctx.types.repr_of(ty);
        c.emit(Op::GetF { dst: reg, obj: frame_reg, field, repr }, sp_lo);
    }
}

/// Completion: the DONE sentinel (null) into the state field, then a
/// plain Ret — a well-formed exit the driving loop reads off the state.
fn emit_completion(c: &mut FnCompiler, sp_lo: u32) {
    let frame_reg = c.async_frame.as_ref().map(|f| f.frame_reg).unwrap_or(0);
    let null = c.emit_null(sp_lo);
    c.emit(
        Op::SetF {
            obj: frame_reg,
            field: af::STATE_FIELD,
            val: null,
            repr: Repr::Ref,
        },
        sp_lo,
    );
    c.emit(Op::Ret { val: None }, sp_lo);
}

/// The drop path (RFC 0016 §3 order): retire the state first (a second
/// abort answers `false`), clear this frame's pending edge (the awaited
/// frame's awaiter slot must not keep a retired frame alive), then
/// release every ref-typed cell-backed local — `SetF null` drops the
/// refcount, the pending-drop drain runs the attached `on_drop`
/// callbacks LIFO, so they fire in reverse declaration order — then a
/// plain Ret.
fn emit_drop_path(c: &mut FnCompiler, sp_lo: u32) {
    let frame_reg = c.async_frame.as_ref().map(|f| f.frame_reg).unwrap_or(0);
    let null = c.emit_null(sp_lo);
    c.emit(
        Op::SetF { obj: frame_reg, field: af::STATE_FIELD, val: null, repr: Repr::Ref },
        sp_lo,
    );
    // clear the pending edge, if one is set (the s0 drop path never parked)
    let pend = c.new_reg(c.async_frame.as_ref().map(|f| f.frame_ty).unwrap_or(TY_OPAQUE));
    c.emit(
        Op::GetF { dst: pend, obj: frame_reg, field: af::PENDING_FIELD, repr: Repr::Ref },
        sp_lo,
    );
    let l_clr = c.new_label();
    let l_done = c.new_label();
    c.br(pend, l_clr, l_done);
    c.bind(l_clr);
    let null = c.emit_null(sp_lo);
    c.emit(
        Op::SetF { obj: pend, field: af::AWAITER_FIELD, val: null, repr: Repr::Ref },
        sp_lo,
    );
    c.emit(
        Op::SetF { obj: frame_reg, field: af::PENDING_FIELD, val: null, repr: Repr::Ref },
        sp_lo,
    );
    c.bind(l_done);
    // Release every ref-typed cell-backed local, in BINDING order: the
    // pending-drop drain runs LIFO, so the callbacks fire in reverse
    // declaration order (RFC 0016 §3 — the later local's cleanup runs
    // first). `SetF null` drops the refcount; the frame cell's own
    // release later finds nulls.
    let backs: Vec<(u32, TypeId)> = c
        .locals
        .iter()
        .filter(|l| l.field != NO_FIELD)
        .map(|l| (l.field, l.ty))
        .collect();
    for (field, ty) in backs {
        if !c.ctx.types.is_ref(ty) {
            continue;
        }
        let null = c.emit_null(sp_lo);
        c.emit(
            Op::SetF { obj: frame_reg, field, val: null, repr: c.ctx.types.repr_of(ty) },
            sp_lo,
        );
    }
    c.emit(Op::Ret { val: None }, sp_lo);
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// The cx protocol members, inlined as field ops (RFC 0018): the cx
    /// record's one field is the frame edge; `checkpoint` reads the frame's
    /// state, `cancelled` reads its flag. `next_checkpoint` stays
    /// weave-only in v1 — a checkpoint value is the checkpoint ENUM's
    /// member singleton, and a runtime u32 cannot name one without new
    /// vocabulary; the weave writes the states it owns.
    pub(crate) fn compile_cx_member(
        &mut self,
        name: IdentId,
        cx_reg: u16,
        args: &[NodeHandle<AnyExpr>],
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let sp_lo = sp.lo;
        if name == sym::CHECKPOINT && args.is_empty() {
            let frame_ty = self
                .async_frame
                .as_ref()
                .map(|f| f.frame_ty)
                .unwrap_or(TY_OPAQUE);
            let frame = self.new_reg(frame_ty);
            self.emit(Op::GetF { dst: frame, obj: cx_reg, field: 0, repr: Repr::Ref }, sp_lo);
            let state = self.new_reg(TY_U32);
            self.emit(
                Op::GetF {
                    dst: state,
                    obj: frame,
                    field: rut_core::async_frame::STATE_FIELD,
                    repr: Repr::Ref,
                },
                sp_lo,
            );
            return Ok(TY_U32);
        }
        if name == sym::CANCELLED && args.is_empty() {
            // the frame edge: inside an async body the cx's frame IS
            // this fn's frame — the staging register takes its type so
            // the field read verifies against the record layout
            let frame_ty = self
                .async_frame
                .as_ref()
                .map(|f| f.frame_ty)
                .unwrap_or(TY_OPAQUE);
            let frame = self.new_reg(frame_ty);
            self.emit(Op::GetF { dst: frame, obj: cx_reg, field: 0, repr: Repr::Ref }, sp_lo);
            let flag = self.new_reg(TY_BOOL);
            let repr = self.ctx.types.repr_of(TY_BOOL);
            self.emit(
                Op::GetF {
                    dst: flag,
                    obj: frame,
                    field: rut_core::async_frame::CANCELLED_FIELD,
                    repr,
                },
                sp_lo,
            );
            return Ok(TY_BOOL);
        }
        if name == sym::NEXT_CHECKPOINT {
            self.ctx.err(sp, "`cx.next_checkpoint(v)` is the weave's own edge in this build — the weave records each await's resume state; user-written checkpoints land with RFC 0019");
            return Err(());
        }
        self.ctx.err(sp, format!(
            "`RunContext` has no member `{}` with {} argument(s) — its members are `checkpoint`/`next_checkpoint`/`cancelled` (RFC 0012 §7)",
            self.ctx.name(name),
            args.len()
        ));
        Err(())
    }
}

/// The await expansion (`ExprKind::Await` inside an async body). The
/// operand must be an ENGINE-WOVEN future — the probe reads the
/// engine-reserved state field, so a user impl has nothing to read.
pub(crate) fn compile_await(
    c: &mut FnCompiler,
    expr: NodeHandle<AnyExpr>,
    sp: rut_lexer::span::Span,
) -> TcResult<TypeId> {
    let Some(frame) = c.async_frame.as_ref().map(|f| AsyncFrameView {
        frame_reg: f.frame_reg,
        cx_reg: f.cx_reg,
        frame_ty: f.frame_ty,
    }) else {
        c.ctx.err(sp, "`await` outside an async fn — async/await is the Future-only vocabulary (RFC 0018); drive futures from an async body or through a launcher");
        return Err(());
    };
    let t = c.compile_expr(expr, None)?;
    // v1 await-legality (the survey's pinned refinement): the operand
    // must reach an engine-woven frame — either its hidden frame type
    // (a bare async-call result) or a `Future<..>` object spelling (an
    // annotated binding, the sleep surface). A concrete non-engine type
    // diagnoses: the probe reads the engine-reserved state field, and a
    // user `impl Future` has nothing to read — launcher territory, and
    // join lands with RFC 0019.
    let ok = match c.ctx.types.kind(t) {
        TyKind::Data { .. } => {
            if c.ctx.engine_frames.contains(&t) {
                true
            } else {
                let tn = c.ctx.type_name(t).to_string();
                if tn.starts_with("LaunchedFutureHandle") {
                    c.ctx.err(sp, "cannot `await` a LaunchedFutureHandle — the receipt is not a Future and cannot be re-launched; join lands with RFC 0019");
                } else {
                    c.ctx.err(sp, format!(
                        "`await` targets engine-woven futures (async-fn results, `sleep`) — `{tn}` is not one; user `impl Future` types drive through launchers, and join lands with RFC 0019"
                    ));
                }
                false
            }
        }
        TyKind::TraitObj { trait_id } => {
            c.ctx
                .trait_inst
                .iter()
                .find(|(_, &id)| id == *trait_id)
                .map(|((n, _), _)| *n == sym::FUTURE)
                .unwrap_or(false)
        }
        _ => {
            c.ctx.err(sp, format!(
                "`await` needs a Future — `{}` is not one (RFC 0018)",
                c.ctx.type_name(t)
            ));
            false
        }
    };
    if !ok {
        return Err(());
    }
    let fut = c.last_reg;
    // the CallI aims at the operand's own `Future::yield` slot: the
    // engine frame's registered row (a bare async-call result) or the
    // trait object's instantiation (an annotated binding, `sleep`)
    let yield_slot = match c.ctx.types.kind(t) {
        TyKind::Data { .. } => *c
            .ctx
            .frame_yield_slot
            .get(&t)
            .expect("engine frame has a registered yield slot"),
        TyKind::TraitObj { trait_id } => c
            .ctx
            .trait_slot(*trait_id, 0)
            .expect("a Future instantiation has exactly one member"),
        _ => unreachable!("legality checked above"),
    };
    let k = c.async_frame.as_ref().expect("checked above").next_state;
    let ckpt_ty = c.async_frame.as_ref().expect("checked above").ckpt_ty;
    let l_sresume = c.new_label();
    {
        let f = c.async_frame.as_mut().expect("checked above");
        f.next_state = k + 1;
        f.arm_labels.push(l_sresume);
    }
    let sp_lo = sp.lo;
    // probe: null state = already done → straight to the resume arm;
    // anything else (fresh or parked) → drive it one step
    let st = c.new_reg(TY_OPAQUE);
    c.emit(
        Op::GetF { dst: st, obj: fut, field: af::STATE_FIELD, repr: Repr::Ref },
        sp_lo,
    );
    let l_drive = c.new_label();
    c.br(st, l_drive, l_sresume);
    c.bind(l_drive);
    {
        let (argv_off, argc) = c.pool_recv_args(fut, &[frame.cx_reg]);
        c.emit(
            Op::CallI { slot: yield_slot, argv_off, argc, dst: NOREG },
            sp_lo,
        );
    }
    let st2 = c.new_reg(TY_OPAQUE);
    c.emit(
        Op::GetF { dst: st2, obj: fut, field: af::STATE_FIELD, repr: Repr::Ref },
        sp_lo,
    );
    let l_park = c.new_label();
    c.br(st2, l_park, l_sresume);
    // park: register the awaiter edges, record MY resume state, ret —
    // the frame suspends; the driving loop re-enqueues it when the
    // awaited frame completes (or the timer fires)
    c.bind(l_park);
    {
        let frame_reg = frame.frame_reg;
        c.emit(
            Op::SetF { obj: fut, field: af::AWAITER_FIELD, val: frame_reg, repr: Repr::Ref },
            sp_lo,
        );
        c.emit(
            Op::SetF { obj: frame_reg, field: af::PENDING_FIELD, val: fut, repr: Repr::Ref },
            sp_lo,
        );
        let e = c.new_reg(ckpt_ty);
        c.emit(Op::EnumNew { dst: e, ty: ckpt_ty, member: k }, sp_lo);
        c.emit(
            Op::SetF {
                obj: frame_reg,
                field: af::STATE_FIELD,
                val: e,
                repr: Repr::Ref,
            },
            sp_lo,
        );
        c.emit(Op::Ret { val: None }, sp_lo);
    }
    // the resume arm — the brtable lands here after the park, and the
    // ready paths fall in: the cancelled probe first, then the pending
    // edges clear, then the locals restore, then the continuation
    c.bind(l_sresume);
    let l_drop = c.new_label();
    let l_live = c.new_label();
    emit_cancelled_probe(c, sp_lo, l_drop, l_live);
    c.bind(l_live);
    {
        let frame_reg = frame.frame_reg;
        let pend = c.new_reg(frame.frame_ty);
        c.emit(
            Op::GetF { dst: pend, obj: frame_reg, field: af::PENDING_FIELD, repr: Repr::Ref },
            sp_lo,
        );
        let l_clr = c.new_label();
        let l_rest = c.new_label();
        c.br(pend, l_clr, l_rest);
        c.bind(l_clr);
        let null = c.emit_null(sp_lo);
        c.emit(
            Op::SetF { obj: pend, field: af::AWAITER_FIELD, val: null, repr: Repr::Ref },
            sp_lo,
        );
        c.emit(
            Op::SetF { obj: frame_reg, field: af::PENDING_FIELD, val: null, repr: Repr::Ref },
            sp_lo,
        );
        c.bind(l_rest);
    }
    emit_restore(c, sp_lo);
    // the continuation label: the arm jumps to the code right after the
    // await (which may sit inside a loop — the state machine re-enters
    // the loop body at its continuation)
    let l_cont = c.new_label();
    c.jmp(l_cont);
    c.bind(l_drop);
    emit_drop_path(c, sp_lo);
    c.bind(l_cont);
    Ok(TY_NIL)
}

struct AsyncFrameView {
    frame_reg: u16,
    cx_reg: u16,
    frame_ty: TypeId,
}

/// The async call site (`compile_free_fn_call`'s is_async arm): mint
/// the frame cell, store the user params, arm the entry state — the
/// call's value IS the frame, typed as its hidden type (which widens
/// to `Future<T>` through the registered impl).
pub(crate) fn compile_async_call(
    c: &mut FnCompiler,
    name: IdentId,
    fd: &rut_ast::ast::FnData,
    args: &[NodeHandle<AnyExpr>],
    expected: Option<TypeId>,
    sp: rut_lexer::span::Span,
) -> TcResult<TypeId> {
    let sp_lo = sp.lo;
    if !fd.generics.is_empty() {
        c.ctx.err(sp, "generic async fns are not woven in this build (RFC 0018 v1)");
        return Err(());
    }
    let inst = crate::check::Inst {
        key: crate::check::FnKey::Free(name),
        subst: vec![],
        trait_origins: vec![],
    };
    let fid = c.ctx.ensure_inst(inst.clone());
    if !c.ctx.async_layout.contains_key(&fid) {
        c.ctx.compile_queue(inst)?;
    }
    // the weave's own legality ran above (the cx law, generics) — its
    // diagnostics precede the call-shape checks
    let layout = *c
        .ctx
        .async_layout
        .get(&fid)
        .ok_or_else(|| ())?; // the weave failed (diagnostics recorded)
    let want = fd.params.len().saturating_sub(1);
    if args.len() != want {
        c.ctx.err(sp, format!(
            "call arity: {} arg(s) for {} parameter(s) — the cx is engine-minted, not passed",
            args.len(), want
        ));
        return Err(());
    }
    let ret_ty = fd.ret.map(|r| c.resolve_type_now(r)).unwrap_or(TY_NIL);
    // compile the args against the declared (post-cx) parameter types
    let mut aregs = Vec::new();
    let mut atys = Vec::new();
    for (i, a) in args.iter().enumerate() {
        let pt = match c.ctx.ast.param(fd.params[i + 1]) {
            MemberKind::Param(ParamData { ty: Some(t), .. }) => c.resolve_type_now(*t),
            _ => TY_I32,
        };
        let t = c.compile_expr(*a, Some(pt))?;
        if !c.widens(t, pt) {
            c.ctx.err(c.ctx.ast.span(a.id()), format!(
                "argument {} is `{}`, `{}` expected",
                i + 1, c.ctx.type_name(t), c.ctx.type_name(pt)
            ));
        }
        c.widen_to_slot(t, pt, sp_lo);
        aregs.push(c.last_reg);
        atys.push(pt);
    }
    // the frame cell: fields are zeroed, the engine state arms s0
    let frame = c.new_reg(layout.frame_ty);
    c.emit(Op::NewCell { dst: frame, ty: layout.frame_ty }, sp_lo);
    for (i, &r) in aregs.iter().enumerate() {
        let field = af::LOCALS_BASE + i as u32;
        let repr = c.ctx.types.repr_of(atys[i]);
        c.emit(Op::SetF { obj: frame, field, val: r, repr }, sp_lo);
    }
    let e = c.new_reg(layout.ckpt_ty);
    c.emit(Op::EnumNew { dst: e, ty: layout.ckpt_ty, member: 0 }, sp_lo);
    c.emit(
        Op::SetF {
            obj: frame,
            field: af::STATE_FIELD,
            val: e,
            repr: c.ctx.types.repr_of(layout.ckpt_ty),
        },
        sp_lo,
    );
    // the call's value IS the frame cell (last_reg is the convention)
    c.last_reg = frame;
    c.ctx.engine_frames.insert(layout.frame_ty);
    // the consume's spelling: a `Future<..>`-typed context (the
    // launcher's generic parameter, an annotated let) takes the trait
    // object — the value stays the frame cell, and the generic unify
    // reads the instantiation's args (`T := ret`). A placeholder-typed
    // context (the generic launcher's hint) takes the CONCRETE
    // instantiation so the unification has real args to read. A bare
    // context keeps the hidden frame type — `await`'s engine-woven
    // provenance.
    if let Some(e) = expected {
        if let TyKind::TraitObj { trait_id } = c.ctx.types.kind(e) {
            let hit = c
                .ctx
                .trait_inst
                .iter()
                .find(|(_, &id)| id == *trait_id)
                .map(|(k, _)| k.clone());
            if let Some((tname, targs)) = hit {
                if tname == sym::FUTURE && !targs.is_empty() {
                    if targs[0] == ret_ty {
                        return Ok(e);
                    }
                    if c.ctx.type_name(targs[0]).starts_with('#') {
                        let obj = c.ctx.mk_trait_obj(layout.fut_inst);
                        return Ok(obj);
                    }
                }
            }
        }
    }
    Ok(layout.frame_ty)
}

/// Mint the engine-backed sleep future once (RFC 0018): the sleep
/// frame, its two-state checkpoint enum, the `Future<nil>` impl row,
/// and the engine-thunk yield (`__sleep_yield` — arm_timer on the
/// fresh arm, done on the resumed one). The host set's `sleep` wraps
/// the minted frame; the VM's timer map does the waiting.
pub(crate) fn ensure_sleep_future(ctx: &mut Ctx) -> TcResult<()> {
    if ctx.sleep_minted {
        return Ok(());
    }
    ctx.sleep_minted = true;
    let ckpt_name = ctx.intern(af::SLEEP_CKPT);
    let s0 = ctx.intern("#s0");
    let s1 = ctx.intern("#s1");
    let ckpt_ty = ctx.types.intern(RutType {
        name: ckpt_name,
        kind: TyKind::Enum { members: vec![(s0, 0), (s1, 1)] },
    });
    let frame_name = ctx.intern(af::SLEEP_FRAME);
    let f_state = ctx.intern("state");
    let f_cancelled = ctx.intern("cancelled");
    let f_awaiter = ctx.intern("awaiter");
    let f_pending = ctx.intern("pending");
    let f_ms = ctx.intern("ms");
    let frame_ty = ctx.types.intern(RutType {
        name: frame_name,
        kind: TyKind::Data {
            fields: vec![
                FieldInfo { name: f_state, ty: ckpt_ty },
                FieldInfo { name: f_cancelled, ty: TY_BOOL },
                FieldInfo { name: f_awaiter, ty: TY_OPAQUE },
                FieldInfo { name: f_pending, ty: TY_OPAQUE },
                FieldInfo { name: f_ms, ty: TY_U32 },
            ],
        },
    });
    let fut_inst = ctx.mk_future_inst(sym::FUTURE, TY_NIL);
    ctx.impls.push(ImplDecl {
        trait_id: fut_inst,
        trait_name: sym::FUTURE,
        target: frame_ty,
        target_data: None,
        trait_arg_nodes: vec![],
        is_template: false,
        inherent: false,
        methods: vec![],
    });
    let slot = ctx.trait_slot(fut_inst, 0).expect("Future has exactly one member");
    let thunk_name = ctx.intern("async_engine::__sleep_yield");
    // the host thunk must EXIST (the wrapper calls it; the join resolves
    // its binding) even though the fill now points at the wrapper
    ctx.ensure_inst(crate::check::Inst {
        key: crate::check::FnKey::HostThunk(thunk_name),
        subst: vec![],
        trait_origins: vec![],
    });
    // the SEALING WRAPPER (the opaque-crossings phase): the yield slot's
    // ABI is engine-wide — recv = the raw frame, argv[1] = the raw cx
    // (every weaved async fn shares it) — while the host row crosses
    // `opaque`. The vtable fill points at this tiny compiler-minted fn,
    // which seals both registers into RFC 0014 boxes and calls the host
    // thunk; the row's wire is then boxes, the shared ABI is untouched.
    let wrap_name = ctx.intern("async_engine::__sleep_yield.wrap");
    let wfid = ctx.ensure_inst(crate::check::Inst {
        key: crate::check::FnKey::HostThunk(wrap_name),
        subst: vec![],
        trait_origins: vec![],
    });
    ctx.extra_vtable_fills.push((frame_ty, slot, wfid));
    ctx.frame_yield_slot.insert(frame_ty, slot);
    ctx.engine_frames.insert(frame_ty);
    Ok(())
}

/// The engine-backed sleep pair (RFC 0018): the host THUNK — bodyless,
/// `host_id` names the embedder's registered body, params spell the row's
/// `(opaque, opaque)` crossings — and the sealing WRAPPER the vtable fill
/// binds (`ensure_sleep_future`): params spell the engine-wide yield ABI
/// (the raw frame and cx), the code seals both into RFC 0014 boxes and
/// calls the thunk. The driving loop's call sites (await, drive) keep
/// their raw (frame, cx) registers; the boxes live and die inside the
/// wrapper's own frame (ref-typed locals release at exit).
pub(crate) fn compile_host_thunk(ctx: &mut Ctx, fid: u32, thunk_name: IdentId) -> TcResult<()> {
    const THUNK: &str = "async_engine::__sleep_yield";
    const WRAP: &str = "async_engine::__sleep_yield.wrap";
    if thunk_name == ctx.intern(WRAP) {
        // the wrapper: seal r0 (frame) and r1 (cx), call the host thunk
        // with the boxes. Requires the sleep mint (the frame/cx types and
        // the thunk's fid all exist once `ensure_sleep_future` ran).
        let frame = ctx
            .types
            .types
            .iter()
            .enumerate()
            .find(|(_, t)| ctx.interner.name(t.name) == rut_core::async_frame::SLEEP_FRAME)
            .map(|(i, _)| i as TypeId);
        let cx = ctx
            .types
            .types
            .iter()
            .enumerate()
            .find(|(_, t)| ctx.interner.name(t.name) == rut_core::async_frame::RUN_CONTEXT_TYPE)
            .map(|(i, _)| i as TypeId);
        let (frame_ty, cx_ty) = match (frame, cx) {
            (Some(f), Some(c)) => (f, c),
            _ => {
                ctx.err(
                    ctx.ast.span(ctx.ast.root.id()),
                    "the sleep yield wrapper needs the engine-minted frame and cx types",
                );
                return Err(());
            }
        };
        let thunk_key = crate::check::FnKey::HostThunk(ctx.intern(THUNK));
        let thunk_fid = ctx
            .inst_map
            .get(&crate::check::Inst {
                key: thunk_key,
                subst: vec![],
                trait_origins: vec![],
            })
            .copied()
            .expect("the sleep thunk is ensured before its wrapper compiles");
        let fc = rut_core::binary::FuncCode {
            name: thunk_name,
            regs: vec![frame_ty, cx_ty, TY_OPAQUE, TY_OPAQUE],
            params: vec![frame_ty, cx_ty],
            ret: TY_NIL,
            is_method: false,
            n_captures: 0,
            argv: vec![2, 3],
            labels: vec![],
            code: vec![
                Op::Box { dst: 2, val: 0, ty: frame_ty },
                Op::Box { dst: 3, val: 1, ty: cx_ty },
                Op::Call { func: thunk_fid, argv_off: 0, argc: 2, dst: NOREG },
                // well-formedness (RFC 0033 §2): the fn ends in Ret — the
                // threaded `Call` answers Next(pc+1), so without this the
                // dispatch walks past the code
                Op::Ret { val: None },
            ],
            spans: vec![],
            pos: vec![],
            host_id: None,
        };
        ctx.funcs[fid as usize] = fc;
        return Ok(());
    }
    if thunk_name == ctx.intern(THUNK) {
        // the host thunk: params spell the ROW — both crossings are
        // `opaque` boxes (the wrapper sealed them); `host_id` routes the
        // registry to the registered body
        let fc = rut_core::binary::FuncCode {
            name: thunk_name,
            regs: vec![TY_OPAQUE, TY_OPAQUE],
            params: vec![TY_OPAQUE, TY_OPAQUE],
            ret: TY_NIL,
            is_method: false,
            n_captures: 0,
            argv: vec![],
            labels: vec![],
            code: vec![],
            spans: vec![],
            pos: vec![],
            host_id: Some(thunk_name),
        };
        ctx.funcs[fid as usize] = fc;
        return Ok(());
    }
    Ok(())
}
