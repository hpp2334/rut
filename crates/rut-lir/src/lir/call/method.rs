//! Method dispatch on a value receiver: resolution through inherent and registered trait impls, plus the missing-method diagnostic and receiver naming.

use crate::check::{ImplHit, TcResult};
use rut_core::sym;
use crate::lir::slice::SliceSource;
use crate::lir::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    pub(crate) fn compile_method(
        &mut self,
        recv: NodeHandle<AnyExpr>,
        name: IdentId,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        // static receiver? `Vec.from(..)`, `Option.some(v)`, `Color.to_int(c)`,
        // `Circle.new(..)` arrive as Method over a TYPE-name path — route to
        // the static-call compiler when the head is not shadowed by a local.
        // The head may carry generic args (`Vec<u32>.from(..)`) — they go
        // along; compile_static_call decides which statics can use them.
        if let ExprKind::Path { segs } = self.ctx.ast.expr(recv).clone() {
            if segs.len() == 1 && self.lookup(segs[0].name).is_none() {
                let base = segs[0].name;
                // `Vec` is an ordinary class (pouch), so it routes
                // here through `find_data`, like any other class; the
                // core statics (`opaque`) route only
                // when used
                let is_type = matches!(base, sym::STR | sym::BYTES)
                    || self.ctx.extern_native_types.contains_key(&base)
                    || self.ctx.is_extern_namespace(base)
                    || self.ctx.find_enum(base).is_some()
                    || self.ctx.find_data(base).is_some()
                    || self.ctx.extern_types.contains_key(&base)
                    || self.ctx.extern_generics.contains_key(&base);
                if is_type {
                    return self.compile_static_call(base, segs[0].generics.clone(), name, generics, args, expected, sp);
                }
                // an in-scope TYPE PARAMETER as the static receiver (the
                // rut-json batch phase 1's sanctioned checker gap 2):
                // `T.decode(r)` — the trait's Self param spelled by name
                // (the no-self law). The body compiles per
                // instantiation, so the parameter names its substituted
                // concrete type here; the trait impl on THAT type answers.
                if segs[0].generics.is_empty() {
                    if let Some(&concrete) = self.subst.iter().find(|(n, _)| *n == base).map(|(_, t)| t) {
                        return self.compile_trait_param_static_call(concrete, name, args, expected, sp);
                    }
                }
            }
        }
        // receiver bypass: a mutating method operates
        // on the ORIGINAL binding — a single-name receiver uses its register
        // raw instead of a boundary clone; a PROMOTED binding reads
        // through the capture law's accessor instead (its register
        // holds the shared cell, the value loads from it — method
        // mutation through the read hits the same shared cell)
        let recv_raw = match self.ctx.ast.expr(recv).clone() {
            ExprKind::Path { segs } if segs.len() == 1 && segs[0].generics.is_empty() => {
                let l = self.lookup(segs[0].name).cloned();
                l.map(|l| {
                    let reg = if l.cell.is_some() { self.read_local(&l, sp.lo) } else { l.reg };
                    (l.ty, reg)
                })
            }
            _ => None,
        };
        // an exact nullable trait impl binds the NULLABLE ITSELF, before
        // the auto-deref (the rut-json batch phase 1, sanctioned checker
        // gap 1's dispatch side): `impl I for ?T` means a `?T` receiver
        // calls that impl — nil IS a value it inspects — so the deref
        // fallback below must never steal the call. No local match → the
        // ordinary `p.m(..)` deref law continues unchanged.
        let recv_opt_raw = match recv_raw {
            Some((ty, reg)) => match self.ctx.types.kind(ty) {
                TyKind::Opt { .. } => Some((ty, reg)),
                _ => None,
            },
            None => None,
        };
        if let Some((ty, reg)) = recv_opt_raw {
            if let Some((idx, midx)) = self.find_trait_impl_method(ty, name) {
                return self.compile_trait_static_call(idx, midx, ty, reg, args, expected, sp, false);
            }
        }
        let (rt, rreg) = match recv_raw {
            Some((ty, reg)) => self.deref_for_use(ty, reg, sp.lo),
            None => {
                let rt = self.compile_expr(recv, None)?;
                let rreg = self.last_reg;
                // `p.m(..)` auto-derefs
                self.deref_for_use(rt, rreg, sp.lo)
            }
        };
        // capability resolution through a union bound (native-fastpath
        // phase 1): a method call on a value whose
        // declared type is a union-bounded `K` requires EVERY member of
        // the bound to provide the method — the union admits all of its
        // members at once, so each instantiation must typecheck. Dispatch
        // below stays per-instantiation (the concrete member's impl).
        self.check_union_capability(recv, name, sp);
        // core's builtin-impl numeric methods:
        // `x.wrapping_add(y)` on an integer receiver — ambient on the
        // primitive (no `use`), lowered inline off the receiver's width.
        // The receiver compiled once above; its register is reused, so a
        // side-effecting receiver still evaluates exactly once.
        if let Some(i) = self.ctx.builtin_impl(name, rt) {
            let [rhs, ..] = &args[..] else {
                self.ctx.err(sp, format!(
                    "`{}.{}` takes one argument — `x.{}(y)`",
                    self.ctx.type_name(rt),
                    self.ctx.name(name),
                    self.ctx.name(name)
                ));
                return Err(());
            };
            return self.compile_intrinsic_method(i, rt, rreg, *rhs, sp);
        }
        // primitives have no method syntax: `str`/`bytes`
        // operations are free functions (`string_len`, `string_encode`,
        // `bytes_len`, `bytes_decode`, `bytes_from`) — except `s.code()`,
        // the v1.1 codepoint reader that replaced `char`
        match self.ctx.types.kind(rt) {
            TyKind::Str => {
                if name == sym::CODE && args.is_empty() {
                    // s.code() -> u32 — the FIRST codepoint; traps on empty.
                    // ONE op (the char exorcism: the slot already carried
                    // the codepoint as bits — no char intermediate, no Conv)
                    let dst = self.new_reg(TY_U32);
                    let zero = self.new_reg(TY_I32);
                    self.emit(Op::ConstRaw { dst: zero, bits: 0 }, sp.lo);
                    self.emit(Op::StrCodeAt { dst, s: rreg, idx: zero }, sp.lo);
                    self.last_reg = dst;
                    return Ok(TY_U32);
                }
                if name == sym::ENCODE && args.is_empty() {
                    // s.encode() -> bytes — the UTF-8 octets
                    let dst = self.emit_string_encode(rreg, sp.lo);
                    self.last_reg = dst;
                    return Ok(TY_BYTES);
                }
                if name == sym::SLICE && args.len() == 2 {
                    // s.slice(from, to) — an O(1) view
                    let ft = self.compile_expr(args[0], Some(TY_I32))?;
                    if ft != TY_I32 {
                        self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                        return Err(());
                    }
                    let fr = self.last_reg;
                    let tt = self.compile_expr(args[1], Some(TY_I32))?;
                    if tt != TY_I32 {
                        self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                        return Err(());
                    }
                    let tr = self.last_reg;
                    let dst = self.new_reg(TY_STR);
                    { let (argv_off, argc) = self.pool_args(&(vec![fr, tr])); self.emit(Op::CallNat { nat: Nat::StrSlice, recv: rreg, argv_off, argc, dst: dst }, sp.lo,); }
                    return Ok(TY_STR);
                }
                if name == sym::LEN && args.is_empty() {
                    if let Some(info) = self.slice_info(rt) {
                        self.emit_slice_len(rreg, &info, sp.lo)?;
                        return Ok(TY_I32);
                    }
                }
                if name == sym::CODE_AT && args.len() == 1 {
                    // s.code_at(i) -> u32 (json-perf phase 2) — the
                    // codepoint at codepoint index i; out-of-bounds traps
                    // (the index is a bug, not data — the `slice` law)
                    let it = self.compile_expr(args[0], Some(TY_I32))?;
                    if it != TY_I32 {
                        self.ctx.err(sp, "code_at(i) takes an `i32` index");
                        return Err(());
                    }
                    let ir = self.last_reg;
                    // ONE op — the u32 codepoint (the char exorcism's
                    // re-spell; see s.code())
                    let dst = self.new_reg(TY_U32);
                    self.emit(Op::StrCodeAt { dst, s: rreg, idx: ir }, sp.lo);
                    self.last_reg = dst;
                    return Ok(TY_U32);
                }
                if name == sym::SCAN && args.len() == 2 {
                    // s.scan(from, set) -> i64 (json-perf phase 2) — the
                    // fused host-side scan/classify: walk codepoints from
                    // `from`, classify each through the caller's `[u8]`
                    // table (`set[min(cp, set.len()-1)]`), stop at the
                    // first nonzero class. Returns the packed pair
                    // `(stop_index << 8) | stop_class`; end of input is
                    // `(s.len() << 8) | 0`. The whole per-byte loop runs
                    // in the host — a tokenizer pays O(calls), not
                    // O(bytes) of interpreted ops.
                    let ft = self.compile_expr(args[0], Some(TY_I32))?;
                    if ft != TY_I32 {
                        self.ctx.err(sp, "scan(from, set) takes an `i32` start index");
                        return Err(());
                    }
                    let fr = self.last_reg;
                    let st = self.compile_expr(args[1], Some(TY_BYTES))?;
                    if !matches!(self.ctx.types.kind(st), TyKind::Array { elem } if *elem == TY_U8) {
                        self.ctx.err(sp, "scan(from, set) takes a `[u8]` class table — entry `min(cp, len-1)` classes each codepoint, `0` keeps scanning");
                        return Err(());
                    }
                    let sr = self.last_reg;
                    let dst = self.new_reg(TY_I64);
                    { let (argv_off, argc) = self.pool_args(&(vec![fr, sr])); self.emit(Op::CallNat { nat: Nat::StrScan, recv: rreg, argv_off, argc, dst: dst }, sp.lo); }
                    return Ok(TY_I64);
                }
                if name == sym::STARTS_WITH && args.len() == 2 {
                    // s.starts_with(from, head) -> bool (json-perf phase
                    // 2) — the prefix test at a codepoint offset,
                    // compared host-side (no per-char str cells)
                    let ft = self.compile_expr(args[0], Some(TY_I32))?;
                    if ft != TY_I32 {
                        self.ctx.err(sp, "starts_with(from, head) takes an `i32` start index");
                        return Err(());
                    }
                    let fr = self.last_reg;
                    let ht = self.compile_expr(args[1], Some(TY_STR))?;
                    if ht != TY_STR {
                        self.ctx.err(sp, "starts_with(from, head) takes a `str` head");
                        return Err(());
                    }
                    let hr = self.last_reg;
                    let dst = self.new_reg(TY_BOOL);
                    { let (argv_off, argc) = self.pool_args(&(vec![fr, hr])); self.emit(Op::CallNat { nat: Nat::StrStartsWith, recv: rreg, argv_off, argc, dst: dst }, sp.lo); }
                    return Ok(TY_BOOL);
                }
                // a registered trait impl on the ref target dispatches
                // statically on the bare receiver — the
                // concrete and slot ABIs coincide for ref targets (P1.1),
                // so the raw register crosses as-is (`k.hash()` /
                // `k.hash_eq(..)` in a monomorphized map body, P4)
                if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                    return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
                }
                if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                    return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
                }
                let who = recv_name(&self.ctx, recv);
                self.ctx.err(sp, format!(
                    "`str` has no method `{}` — its members are `len`/`slice`/`code`/`code_at`/`scan`/`starts_with`/`encode` (`string_len({who})` is the free-fn spelling)",
                    self.ctx.name(name)
                ));
                return Err(());
            }
            TyKind::Bytes => {
                if name == sym::DECODE && args.is_empty() {
                    // b.decode() -> str — UTF-8, lossy
                    let dst = self.emit_bytes_decode(rreg, sp.lo);
                    self.last_reg = dst;
                    return Ok(TY_STR);
                }
                if name == sym::LEN && args.is_empty() {
                    if let Some(info) = self.slice_info(rt) {
                        self.emit_slice_len(rreg, &info, sp.lo)?;
                        return Ok(TY_I32);
                    }
                }
                if name == sym::CLONE && args.is_empty() {
                    // b.clone() -> bytes — a one-shot buffer copy (RFC
                    // 0044): the ONLY copy escape hatch. Every binding
                    // shares its cell; this member mints a fresh one.
                    let dst = self.new_reg(TY_BYTES);
                    { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::BytesClone, recv: rreg, argv_off, argc, dst }, sp.lo); }
                    self.last_reg = dst;
                    return Ok(TY_BYTES);
                }
                // a registered trait impl on the ref target dispatches
                // statically on the bare receiver (same law as `str`
                // above — one ABI variant, the raw register crosses)
                if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                    return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
                }
                if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                    return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
                }
                let who = recv_name(&self.ctx, recv);
                self.ctx.err(sp, format!(
                    "`bytes` has no method `{}` — its members are `len`/`decode`/`clone` (`bytes_len({who})` is the free-fn spelling)",
                    self.ctx.name(name)
                ));
                return Err(());
            }
            TyKind::Trace => {
                // the StackTrace member contract (err-channel
                // phase 2): engine-builtins on the snapshot,
                // lowered to the trace natives — lazy symbolication lives
                // in the VM, per access
                match (name, args.len()) {
                    (sym::LEN, 0) => {
                        let dst = self.new_reg(TY_I32);
                        { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::TraceLen, recv: rreg, argv_off, argc, dst }, sp.lo); }
                        return Ok(TY_I32);
                    }
                    (sym::NAME, 1) => {
                        let it = self.compile_expr(args[0], Some(TY_I32))?;
                        if it != TY_I32 {
                            self.ctx.err(sp, "name(i) takes an `i32` index");
                            return Err(());
                        }
                        let ir = self.last_reg;
                        let dst = self.new_reg(TY_STR);
                        { let (argv_off, argc) = self.pool_args(&(vec![ir])); self.emit(Op::CallNat { nat: Nat::TraceName, recv: rreg, argv_off, argc, dst }, sp.lo); }
                        return Ok(TY_STR);
                    }
                    (sym::LINE, 1) => {
                        let it = self.compile_expr(args[0], Some(TY_I32))?;
                        if it != TY_I32 {
                            self.ctx.err(sp, "line(i) takes an `i32` index");
                            return Err(());
                        }
                        let ir = self.last_reg;
                        let dst = self.new_reg(TY_I32);
                        { let (argv_off, argc) = self.pool_args(&(vec![ir])); self.emit(Op::CallNat { nat: Nat::TraceLine, recv: rreg, argv_off, argc, dst }, sp.lo); }
                        return Ok(TY_I32);
                    }
                    (sym::COL, 1) => {
                        let it = self.compile_expr(args[0], Some(TY_I32))?;
                        if it != TY_I32 {
                            self.ctx.err(sp, "col(i) takes an `i32` index");
                            return Err(());
                        }
                        let ir = self.last_reg;
                        let dst = self.new_reg(TY_I32);
                        { let (argv_off, argc) = self.pool_args(&(vec![ir])); self.emit(Op::CallNat { nat: Nat::TraceCol, recv: rreg, argv_off, argc, dst }, sp.lo); }
                        return Ok(TY_I32);
                    }
                    (sym::RENDER, 0) => {
                        let dst = self.new_reg(TY_STR);
                        { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::TraceRender, recv: rreg, argv_off, argc, dst }, sp.lo); }
                        return Ok(TY_STR);
                    }
                    _ => {
                        self.ctx.err(sp, format!(
                            "`StackTrace` has no method `{}` with {} argument(s) — its members are `len`/`name(i)`/`line(i)`/`col(i)`/`render`",
                            self.ctx.name(name),
                            args.len()
                        ));
                        return Err(());
                    }
                }
            }
            TyKind::Weak { elem } => {
                // the Weak member contract: one engine
                // builtin — `upgrade()` answers the live referent as `?T`
                // or nil. For `Weak<?U>` the answer is `??U` (MakeOpt
                // wraps the box — the sticky-`?` law, no unguarded unwrap).
                match (name, args.len()) {
                    (sym::UPGRADE, 0) => {
                        let opt_ty = self.ctx.mk_opt(*elem);
                        let dst = self.new_reg(opt_ty);
                        { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::WeakUpgrade { recv: rreg, dst, ty: opt_ty }, sp.lo); }
                        return Ok(opt_ty);
                    }
                    _ => {
                        self.ctx.err(sp, format!(
                            "`Weak` has no method `{}` with {} argument(s) — its member is `upgrade()`",
                            self.ctx.name(name),
                            args.len()
                        ));
                        return Err(());
                    }
                }
            }
            _ => {}
        }
        // a generic METHOD call (`store.get<T>(a)`) resolves on a user
        // class below (the inherent-method path takes the site's type
        // arguments); every other receiver shape has no generic member
        // surface in this build
        if !generics.is_empty() && !matches!(self.ctx.types.kind(rt), TyKind::Data { .. }) {
            self.ctx.err(sp, "generic method calls are not supported on this receiver — only class methods take type arguments (the `store.get<T>` family)");
            return Err(());
        }
        // `s.len()` — the Slice surface member shared by every sequence;
        // lowered fused, including the `Vec<T>` class
        if name == sym::LEN && args.is_empty() {
            if let Some(info) = self.slice_info(rt) {
                self.emit_slice_len(rreg, &info, sp.lo)?;
                return Ok(TY_I32);
            }
        }
        // `v.slice(from, to)` — an O(1) array window: mints
        // an `ArrView` cell over the backing array and boxes it as
        // `*Vec<T>`/`*Array<T>`. Reads AND writes through the pointer go
        // to the parent (the `*T` aliasing law); the window is
        // fixed-length. str has its own slice (handled above).
        if name == sym::SLICE && args.len() == 2 {
            if let Some(info) = self.slice_info(rt) {
                match &info.source {
                    SliceSource::DataBuf { buf_field, len_field } => {
                        let arr_ty = info.array_ty(self.ctx);
                        let arr = self.new_reg(arr_ty);
                        self.emit(Op::GetF { dst: arr, obj: rreg, field: *buf_field, repr: Repr::Ref }, sp.lo);
                        let live = self.new_reg(TY_I32);
                        self.emit(Op::GetF { dst: live, obj: rreg, field: *len_field, repr: Repr::Prim(PrimTy::I32) }, sp.lo);
                        let ft = self.compile_expr(args[0], Some(TY_I32))?;
                        if ft != TY_I32 {
                            self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                            return Err(());
                        }
                        let fr = self.last_reg;
                        let tt = self.compile_expr(args[1], Some(TY_I32))?;
                        if tt != TY_I32 {
                            self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                            return Err(());
                        }
                        let tr = self.last_reg;
                        let view = self.new_reg(arr_ty);
                        { let (argv_off, argc) = self.pool_args(&(vec![fr, tr, live])); self.emit(Op::CallNat { nat: Nat::ArrSlice, recv: arr, argv_off, argc, dst: view }, sp.lo,); }
                        let ptr_ty = self.ctx.mk_opt(rt);
                        let dst = self.new_reg(ptr_ty);
                        self.emit(Op::MakeOpt { dst, src: view, ty: ptr_ty }, sp.lo);
                        return Ok(ptr_ty);
                    }
                    SliceSource::Array => {
                        let ft = self.compile_expr(args[0], Some(TY_I32))?;
                        if ft != TY_I32 {
                            self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                            return Err(());
                        }
                        let fr = self.last_reg;
                        let tt = self.compile_expr(args[1], Some(TY_I32))?;
                        if tt != TY_I32 {
                            self.ctx.err(sp, "slice(from, to) takes `i32` bounds");
                            return Err(());
                        }
                        let tr = self.last_reg;
                        let live = self.new_reg(TY_I32);
                        { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::ArrLen, recv: rreg, argv_off, argc, dst: live }, sp.lo); }
                        let view = self.new_reg(rt);
                        { let (argv_off, argc) = self.pool_args(&(vec![fr, tr, live])); self.emit(Op::CallNat { nat: Nat::ArrSlice, recv: rreg, argv_off, argc, dst: view }, sp.lo,); }
                        let ptr_ty = self.ctx.mk_opt(rt);
                        let dst = self.new_reg(ptr_ty);
                        self.emit(Op::MakeOpt { dst, src: view, ty: ptr_ty }, sp.lo);
                        return Ok(ptr_ty);
                    }
                    _ => {
                        self.ctx.err(sp, "`slice` on this sequence is not supported");
                        return Err(());
                    }
                }
            }
        }
        // builtin members: `opaque` rejects methods
        // unless the module registered an inherent impl for it
        // (a module-owned `builtin class` takes impls)
        match self.ctx.types.kind(rt).clone() {
            TyKind::Opaque => {
                if let Some((idx, mname)) = self.ctx.impls.iter().enumerate().find_map(|(idx, im)| {
                    if !im.inherent || im.target != TY_OPAQUE {
                        return None;
                    }
                    im.methods.iter().find(|(n, _)| *n == name).map(|(n, _)| (idx, *n))
                }) {
                    return self.compile_native_static_call(idx, mname, vec![], rreg, args, expected, sp);
                }
                self.ctx.err(sp, "`opaque` has no methods in this build —recover with `opaque.downcast<T>(o)`");
                return Err(());
            }
            _ => {}
        }
        // user types: inherent methods first (direct), then trait impls
        // — statically bound for this concrete receiver (nominal, RFC
        // 0012 §5: an impl is registered for exactly this (trait, type))
        if let TyKind::Data { .. } = self.ctx.types.kind(rt).clone() {
            // the cx protocol members inline as
            // FIELD OPS on the engine-minted record — no calls, the
            // frozen surface is the signature set, the lowering is the
            // compiler's (the state field holds the checkpoint enum's
            // singleton; `checkpoint()` answers it — the divergence the
            // survey blessed, invisible off the weave)
            if rt == self.ctx.run_context_ty() {
                return self.compile_cx_member(name, rreg, &args, sp);
            }
            // the receiver is either an instantiated generic (decl + args in
            // `inst_data`) or a local non-generic record
            let target = match self.ctx.inst_data.get(&rt).cloned() {
                Some((dname, cargs)) => Some((dname, cargs)),
                None => self
                    .ctx
                    .datas
                    .iter()
                    .find(|(_, d)| d.ty == rt)
                    .map(|(n, _)| (*n, vec![])),
            };
            let found = target.and_then(|(dname, cargs)| {
                self.ctx
                    .find_data(dname)
                    .and_then(|d| {
                        d.methods
                            .iter()
                            .find(|(mn, _)| *mn == name)
                            .map(|(_, mnode)| (*mnode, cargs.clone()))
                    })
                    .map(|(mnode, cargs)| (dname, cargs, mnode))
            });
            if let Some((dname, class_args, mnode)) = found {
                return self.compile_inherent_call(dname, class_args, rt, mnode, rreg, generics, args, expected, sp);
            }
            // a used class (the linkable-classes phase): the surface's
            // inherent rows answer — a plain class binds the exporter's
            // fn, a generic class mints the mirror instantiation the
            // owner's unit compiles
            if let Some((ih, midx, subst, dname)) = self.find_extern_inherent(rt, name) {
                return self.compile_extern_method_call(ih, midx, &subst, dname, rt, rreg, generics, args, expected, sp);
            }
            if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                // a bare concrete receiver calls the CONCRETE-ABI variant:
                // the receiver register stays raw and the args cross
                // concretely — no box is minted (P1.2); a tiny body inlines
                // at the site (P1.3)
                return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
            }
            // another module's registration: static
            // dispatch through the exporter's compiled fn — the gate
            // (the trait's name was used here) is part of the lookup
            if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
            }
            self.no_method_error(rt, name, sp);
            return Err(());
        }
        // enums: inherent methods answer on the decl (same module) —
        // self methods dispatch like a record's, `Self` binds the
        // enum's own type; a used enum rides the exporter's surface
        // rows. Trait impls are nominal TypeId equality, the same
        // registry every other concrete receiver reads.
        if let TyKind::Enum { .. } = self.ctx.types.kind(rt).clone() {
            if let Some((ename, e)) = self.ctx.enums.iter().find(|(_, e)| e.ty == rt).cloned() {
                if let Some((_, mnode)) = e.methods.iter().find(|(mn, _)| *mn == name).cloned() {
                    return self.compile_inherent_call(ename, vec![], rt, mnode, rreg, generics, args, expected, sp);
                }
            }
            if let Some((ih, midx, subst, dname)) = self.find_extern_inherent(rt, name) {
                return self.compile_extern_method_call(ih, midx, &subst, dname, rt, rreg, generics, args, expected, sp);
            }
            if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
            }
            if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
            }
            self.no_method_error(rt, name, sp);
            return Err(());
        }
        // primitives take trait impls only (the inherent
        // surface is core's `builtin impl`): a bare
        // concrete receiver dispatches the impl's CONCRETE-ABI variant —
        // the scalar crosses as an ordinary argument, no slot box (P1.2)
        if matches!(self.ctx.types.kind(rt), TyKind::Prim(_)) {
            if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
            }
            if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
            }
            self.no_method_error(rt, name, sp);
            return Err(());
        }
        // the array type's inherent impl (`impl [T] { .. }`)
        // — static dispatch through the native shape
        if let TyKind::Array { elem } = self.ctx.types.kind(rt).clone() {
            let hit = self.ctx.impls.iter().enumerate().find_map(|(idx, im)| {
                if !im.inherent {
                    return None;
                }
                match &im.target_data {
                    Some((d, params)) if d == &sym::ARRAY && params.len() == 1 => {
                        im.methods.iter().find(|(n, _)| *n == name).map(|(n, _)| (idx, *n, params[0]))
                    }
                    _ => None,
                }
            });
            if let Some((idx, mname, param)) = hit {
                return self.compile_native_static_call(idx, mname, vec![(param, elem)], rreg, args, expected, sp);
            }
            // a registered TRAIT impl over the array shape (`impl I for
            // [T]` — the rut-json batch phase 1): the same static bind the
            // Data/Prim arms answer, after the inherent surface misses —
            // the receiver stays raw, the element instantiates the template.
            // Another module's registration (flow's `IntoFlow<T> for [T]`)
            // routes through the same extern door the Data arm answers.
            if let Some((idx, midx)) = self.find_trait_impl_method(rt, name) {
                return self.compile_trait_static_call(idx, midx, rt, rreg, args, expected, sp, false);
            }
            if let Some((eidx, midx)) = self.find_extern_trait_impl_method(rt, name) {
                return self.compile_extern_trait_static_call(eidx, midx, rt, rreg, args, expected, sp, false);
            }
        }
        if let TyKind::TraitObj { trait_id } = self.ctx.types.kind(rt).clone() {
            // trait-typed receiver: ONLY that trait's methods.
            // Single concrete origin ⇒ static bind; a merged/loaded/unknown
            // origin consults the value's descriptor (vtable)
            let tdesc = self.ctx.trait_by_id(trait_id).clone();
            if let Some(midx) = tdesc.methods.iter().position(|m| m.name == name) {
                if let ExprKind::Path { segs } = self.ctx.ast.expr(recv).clone() {
                    if segs.len() == 1 {
                        let origins = self.origins_of(segs[0].name);
                        if origins.len() == 1 {
                            // the impl may live in any module:
                            // a local block binds here, another module's
                            // registration binds to its compiled fn. A
                            // parameterized trait impl mints on the miss
                            // (the phase-2 door): the widening let's single
                            // origin unifies against the template and the
                            // slot-ABI variant answers
                            match self.ctx.find_impl_ex(trait_id, origins[0]) {
                                Some(ImplHit::Local(idx)) => {
                                    // origin-pinned trait-object receiver: the box
                                    // is already materialized — call the SLOT
                                    // variant (no new boxes)
                                    return self.compile_trait_static_call(idx, midx, origins[0], rreg, args, expected, sp, true);
                                }
                                Some(ImplHit::Extern(eidx)) => {
                                    return self.compile_extern_trait_static_call(eidx, midx, origins[0], rreg, args, expected, sp, true);
                                }
                                None => {
                                    if let Some(idx) = self.ctx.find_or_mint_impl(trait_id, origins[0]) {
                                        return self.compile_trait_static_call(idx, midx, origins[0], rreg, args, expected, sp, true);
                                    }
                                }
                            }
                        }
                    }
                }
                let slot = self.ctx.trait_slot(trait_id, midx as u32).unwrap();
                return self.finish_trait_call(slot, tdesc.methods[midx].params.clone(), tdesc.methods[midx].ret, rreg, args, expected, sp);
            }
            self.ctx.err(sp, format!(
                "`{}` values reach only `{}`'s methods —`{}` is not one of them",
                self.ctx.name(tdesc.name), self.ctx.name(tdesc.name), self.ctx.name(name)
            ));
            return Err(());
        }
        self.ctx.err(sp, format!("`{}` has no method `{}` in this build", self.ctx.type_name(rt), self.ctx.name(name)));
        Err(())
    }

    /// The diagnostic for a missing method on a user type: when a
    /// matching impl exists under a trait this call site cannot name,
    /// the message says which `use` unlocks it
    /// (the use-both gate). Both local impls and other modules'
    /// registrations count as "exists".
    pub(crate) fn no_method_error(&mut self, rt: TypeId, name: IdentId, sp: rut_lexer::span::Span) {
        let target_matches = |ctx: &Ctx, im: &crate::check::ImplDecl| {
            im.target == rt
                || matches!(&im.target_data, Some((d, _)) if ctx
                    .inst_data
                    .get(&rt)
                    .map_or(false, |(rd, _)| rd == d))
        };
        let hit = self.ctx.impls.iter().find(|im| {
            !im.inherent
                && im.methods.iter().any(|(n, _)| *n == name)
                && target_matches(self.ctx, im)
        });
        let ext_hit = hit.is_none().then(|| {
            self.ctx
                .extern_impls
                .iter()
                .position(|im| im.target == rt && im.methods.iter().any(|(n, _)| *n == name))
        }).flatten();
        let trait_name = hit.map(|im| im.trait_name).or_else(|| {
            ext_hit.map(|e| self.ctx.extern_impls[e].trait_name)
        });
        if let Some(tname) = trait_name {
            let callable = self.ctx.find_trait(tname).is_some()
                || self.ctx.extern_traits.contains_key(&tname)
                || self.ctx.extern_trait_decls.contains_key(&tname);
            if !callable {
                let tid = hit.map(|im| im.trait_id).unwrap_or_else(|| {
                    self.ctx.extern_impls[ext_hit.unwrap()].trait_id
                });
                let spelled = self.ctx.name(self.ctx.trait_by_id(tid).name);
                self.ctx.err(sp, format!(
                    "`{}` has no method `{}` — use `{}` to call its methods on `{}`",
                    self.ctx.type_name(rt),
                    self.ctx.name(name),
                    spelled,
                    self.ctx.type_name(rt)
                ));
                return;
            }
        }
        self.ctx.err(sp, format!("`{}` has no method `{}`", self.ctx.type_name(rt), self.ctx.name(name)));
    }

    /// The used class whose inherent surface answers `name` on `rt`:
    /// `(row, method index, class substitution, decl name when
    /// generic)`. The mirror machinery needs the decl name + argument
    /// types for the owner request; a plain class returns `None` for
    /// both (its fn id rides the surface row).
    pub(crate) fn find_extern_inherent(
        &self,
        rt: TypeId,
        name: IdentId,
    ) -> Option<(usize, usize, Vec<(IdentId, TypeId)>, Option<IdentId>)> {
        let (dname, cargs) = match self.ctx.inst_data.get(&rt) {
            Some((d, args)) => (Some(*d), args.clone()),
            None => (None, vec![]),
        };
        for (i, ih) in self.ctx.extern_inherents.iter().enumerate() {
            let generic = dname.and_then(|d| self.ctx.extern_generics.get(&d));
            let hit = ih.target == rt
                || generic.map_or(false, |g| g.template == ih.target);
            if !hit {
                continue;
            }
            let Some(midx) = ih.methods.iter().position(|m| m.name == name) else {
                continue;
            };
            let subst = generic
                .map(|g| g.params.iter().cloned().zip(cargs.iter().cloned()).collect())
                .unwrap_or_default();
            return Some((i, midx, subst, generic.map(|g| {
                dname.expect("generic implies decl name")
            })));
        }
        None
    }

    /// The registered trait impl on `rt` whose method set contains
    /// `name` — concrete targets by id, generic targets by declaration.
    pub(crate) fn find_trait_impl_method(&self, rt: TypeId, name: IdentId) -> Option<(usize, usize)> {
        for (idx, im) in self.ctx.impls.iter().enumerate() {
            if im.inherent || !im.methods.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let target_matches = im.target == rt
                || matches!(&im.target_data, Some((d, _)) if self
                    .ctx
                    .inst_data
                    .get(&rt)
                    .map_or(false, |(rd, _)| rd == d))
                // structural template targets (the rut-json batch phase 1):
                // a nullable or array receiver binds its element-generic
                // impl (`impl I for ?T` / `impl I for [T]`) by shape
                || matches!(&im.target_data, Some((d, params)) if params.len() == 1
                    && match self.ctx.types.kind(rt) {
                        TyKind::Opt { .. } => *d == sym::OPT,
                        TyKind::Array { .. } => *d == sym::ARRAY,
                        _ => false,
                    });
            if !target_matches {
                continue;
            }
            let tdesc = self.ctx.trait_by_id(im.trait_id);
            if let Some(midx) = tdesc.methods.iter().position(|m| m.name == name) {
                return Some((idx, midx));
            }
        }
        None
    }

}

/// The receiver's source name for a diagnostic (`x` for `x.f()`, else
/// `expr`) — used when a primitive method call is rejected.
fn recv_name(ctx: &crate::check::Ctx, recv: NodeHandle<AnyExpr>) -> String {
    match ctx.ast.expr(recv) {
        ExprKind::Path { segs } => {
            segs.iter().map(|s| ctx.name(s.name).to_string()).collect::<Vec<_>>().join(".")
        }
        _ => "expr".to_string(),
    }
}
