//! Calls and member resolution: builtin/free-fn/method/static dispatch,
//! generic instantiation by unification (RFC 0013 SS2), the RFC 0012
//! vtable-always rule for trait members, trait-typed receivers, and field reads.

use crate::check::TcResult;
use rut_core::sym;
use super::*;

// ============ part 3: calls & members ============

// The `impl FnCompiler` is split across submodules by dispatch kind —
// one impl block per file, the crate's seam pattern: `builtin` (bytes
// alloc, `?T` recovery, static builtins), `free` (free fns + direct
// methods), `method` (receiver dispatch + the missing-method
// diagnostic), `trait_static` (registered/generic/extern trait
// statics + the vtable finisher), `inline` (inherent calls + the
// inline-at-site fast paths), `field` (field reads + union guards).
// Shared arg-widening and receiver-mut helpers stay here.
mod builtin;
mod field;
mod free;
mod inline;
mod method;
mod trait_static;

impl<'a, 'b> FnCompiler<'a, 'b> {
    pub(crate) fn compile_call(
        &mut self,
        node: NodeHandle<AnyExpr>,
        callee: NodeHandle<AnyExpr>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let _ = node;
        // fn-typed local call: `f(x)` where f is a param/local of fn type
        if let ExprKind::Path { segs } = self.ctx.ast.expr(callee).clone() {
            if segs.len() == 1 && self.lookup(segs[0].name).is_some() {
                let ft = self.compile_expr(callee, None)?;
                // the callee derefs (RFC 0044): `let f =
                // opaque.downcast<fn(..)>(..)` lands `?fn` — the erased
                // program calls through its payload
                let (ft, _) = self.deref_for_use(ft, self.last_reg, sp.lo);
                match self.ctx.types.kind(ft).clone() {
                    TyKind::Fn { .. } => {
                        let freg = self.last_reg;
                        return self.finish_call_fn(freg, args, expected, sp);
                    }
                    _ => {
                        self.ctx.err(sp, format!("`{}` is not callable", self.ctx.type_name(ft)));
                        return Err(());
                    }
                }
            }
        }
        if let ExprKind::Path { segs } = self.ctx.ast.expr(callee).clone() {
            return self.compile_path_call(segs, args, expected, sp);
        }
        // fn-typed value call: `f(x)` where f: fn(T) -> U — the callee
        // derefs first (see the local arm above)
        let ft = self.compile_expr(callee, None)?;
        let (ft, _) = self.deref_for_use(ft, self.last_reg, sp.lo);
        match self.ctx.types.kind(ft).clone() {
            TyKind::Fn { .. } => {
                let freg = self.last_reg;
                self.finish_call_fn(freg, args, expected, sp)
            }
            _ => {
                self.ctx.err(sp, format!("`{}` is not callable", self.ctx.type_name(ft)));
                Err(())
            }
        }
    }

    pub(crate) fn finish_call_fn(
        &mut self,
        freg: u16,
        args: Vec<NodeHandle<AnyExpr>>,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let (ptys, ret) = match self.ctx.types.kind(self.regs[freg as usize]).clone() {
            TyKind::Fn { params, ret } => (params, ret),
            _ => {
                self.ctx.err(sp, "not a function value");
                return Err(());
            }
        };
        // trailing captures are not callable args
        let declared = ptys.len();
        if args.len() != declared {
            self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), declared));
            return Err(());
        }
        let mut aregs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let t = self.compile_expr(*a, Some(ptys[i]))?;
            if !self.widens(t, ptys[i]) {
                self.ctx.err(self.ctx.ast.span(a.id()), format!(
                    "argument {} is `{}`, `{}` expected",
                    i + 1, self.ctx.type_name(t), self.ctx.type_name(ptys[i])
                ));
            }
            aregs.push(self.last_reg);
        }
        let dst = if ret == TY_NIL { None } else { Some(self.new_reg(ret)) };
        { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::CallFn { fval: freg, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
        Ok(ret)
    }

    pub(crate) fn compile_path_call(
        &mut self,
        segs: Vec<PathSeg>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        if segs.len() == 2 {
            let base = segs[0].name;
            let member = segs[1].name;
            let base_generics = segs[0].generics.clone();
            let member_generics = segs[1].generics.clone();
            return self.compile_static_call(base, base_generics, member, member_generics, args, expected, sp);
        }
        if segs.len() != 1 {
            self.ctx.err(sp, "unsupported call path (module loading is not available in this build, RFC 0035)");
            return Err(());
        }
        let name = segs[0].name;
        let generics = segs[0].generics.clone();
        // core prelude functions (RFC 0028): compiler-lowered, visible
        // only when the name was used from the core surface — the
        // prelude is used, never ambient. A local fn of the same name
        // wins when the use statement is absent (fallthrough below).
        let core_fn = self.ctx.extern_native_fns.contains(&name);
        // removed prelude spellings diagnose themselves — compared as
        // symbols against the interned removal table, they are not
        // well-known symbols
        if let Some(msg) = self.ctx.removed_core(name) {
            self.ctx.err(sp, msg);
            return Err(());
        }
        if name == sym::PRINT {
            self.ctx.err(sp, "`print` was removed — use a logger (`use ink::{log}`)");
            return Err(());
        }
        if matches!(name, sym::SIZE_OF | sym::ALIGN_OF) {
            // the repr-C layout contract was removed: records are slot
            // arrays, not byte blocks, so there is no value size/alignment
            self.ctx.err(sp, format!(
                "`{}` was removed — records are stored as one slot per field, not a repr-C block",
                self.ctx.name(name)
            ));
            return Err(());
        }
        match name {
            sym::OPAQUE => {
                // `opaque(v)` (RFC 0014; the RFC 0044 surface swap renamed
                // `opaque(v)`): the erasure box. The payload IS `v` —
                // a record/str/bytes binding shares its cell, a primitive
                // is copied, and a `?T` binding stores the nullable box
                // (the old `&v`-then-box shape, one spelling shorter).
                if args.len() != 1 {
                    self.ctx.err(sp, "opaque(v) takes one value");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                if matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. }) {
                    self.ctx.err(sp, "`opaque` rejects trait objects —they are never boxed (RFC 0014)");
                    return Err(());
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_OPAQUE);
                self.emit(Op::Box { dst, val: src, ty: t }, sp.lo);
                return Ok(TY_OPAQUE);
            }
            sym::ON_DROP if core_fn => {
                // on_drop(p, cleanup) (RFC 0016 §3): cleanup runs when the
                // cell's refcount reaches zero
                if args.len() != 2 {
                    self.ctx.err(sp, "on_drop(p, cleanup) takes two arguments (RFC 0016 §3)");
                    return Err(());
                }
                let pt = self.compile_expr(args[0], None)?;
                if !matches!(self.ctx.types.kind(pt), TyKind::Opt { .. }) {
                    self.ctx.err(sp, "on_drop needs a nullable — `?T` (RFC 0016 §3 + RFC 0044)");
                    return Err(());
                }
                let obj = self.last_reg;
                let ct = self.compile_expr(args[1], None)?;
                if !matches!(self.ctx.types.kind(ct), TyKind::Fn { .. }) {
                    self.ctx.err(sp, "on_drop's cleanup must be a function value — `fn(?T)` (RFC 0016 §3)");
                    return Err(());
                }
                let cleanup = self.last_reg;
                self.emit(Op::OnDrop { obj, cleanup }, sp.lo);
                return Ok(TY_NIL);
            }
            sym::PANIC if core_fn => {
                if args.len() != 1 {
                    self.ctx.err(sp, "panic(msg) takes a message (RFC 0034 §2)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_STR))?;
                if t != TY_STR {
                    self.ctx.err(sp, "panic takes a `str`");
                }
                self.emit(Op::Panic { msg: self.last_reg }, sp.lo);
                return Ok(TY_NIL);
            }
            sym::ASSERT if core_fn => {
                if args.is_empty() || args.len() > 2 {
                    self.ctx.err(sp, "assert(cond, msg?) takes a condition (RFC 0034 §2)");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_BOOL))?;
                if t != TY_BOOL {
                    self.ctx.err(sp, "assert takes a `bool`");
                }
                let cond = self.last_reg;
                let msg = if args.len() == 2 {
                    let t = self.compile_expr(args[1], Some(TY_STR))?;
                    if t != TY_STR {
                        self.ctx.err(sp, "assert message must be a `str`");
                    }
                    Some(self.last_reg)
                } else {
                    None
                };
                self.emit(Op::Assert { cond, msg }, sp.lo);
                return Ok(TY_NIL);
            }
            sym::CAPTURE_STACKTRACE if core_fn => {
                // capture_stacktrace() -> StackTrace (RFC 0036 §2,
                // err-channel phase 2): the frame walk is the VM's, at
                // run time — the native mints the trace cell. Opt-in at
                // the raise site: nothing here touches the hot path.
                if !args.is_empty() {
                    self.ctx.err(sp, "capture_stacktrace() takes no arguments");
                    return Err(());
                }
                let dst = self.new_reg(TY_STACK_TRACE);
                { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::CaptureTrace, recv: NOREG, argv_off, argc, dst }, sp.lo); }
                return Ok(TY_STACK_TRACE);
            }
            sym::STRING_JOIN if core_fn => {
                // join every element of an `Array<str>` in one pass: the
                // native sizes once and allocates once (RFC 0032 §1.1 R2).
                if args.len() != 1 {
                    self.ctx.err(sp, "string_join(parts) takes one `Array<str>`");
                    return Err(());
                }
                let hint = Some(self.ctx.mk_array(TY_STR));
                let at = self.compile_expr(args[0], hint)?;
                match self.ctx.types.kind(at) {
                    TyKind::Array { elem } if *elem == TY_STR => {}
                    _ => {
                        self.ctx.err(sp, format!("string_join expects `Array<str>` —found `{}`", self.ctx.type_name(at)));
                        return Err(());
                    }
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_STR);
                { let (argv_off, argc) = self.pool_args(&(vec![src])); self.emit(Op::CallNat { nat: Nat::StrJoin, recv: NOREG, argv_off, argc, dst: dst }, sp.lo); }
                return Ok(TY_STR);
            }
            sym::TYPE_ID => {
                // compile-time constant —never executed (RFC 0015 §3, 0033 §3)
                if generics.len() != 1 || !args.is_empty() {
                    self.ctx.err(sp, format!("{}<T>() takes one explicit type argument", self.ctx.name(name)));
                    return Err(());
                }
                let t = self.resolve_type_now(generics[0]);
                let dst = self.new_reg(TY_U32);
                // a rebasable const-pool entry: link maps the module-local
                // TypeId into the global table (RFC 0035 §1)
                let k = self.konst(ConstVal::TypeId(t));
                self.emit(Op::Const { dst, k: k as u32 }, sp.lo);
                return Ok(TY_U32);
            }
            sym::I8 | sym::I16 | sym::I32 | sym::I64 | sym::U8 | sym::U16 | sym::U32 | sym::U64
            | sym::F32 | sym::F64 => {
                // removed when the `as` cast landed (RFC 0007 §1) — the
                // conversion family is spelled `x as T` now. The message
                // mirrors the `size_of` removal above.
                self.ctx.err(sp, format!("`{}(x)` was removed — use `x as {}` (RFC 0007 §1)", self.ctx.name(name), self.ctx.name(name)));
                return Err(());
            }
            sym::STR => {
                if args.len() != 1 {
                    self.ctx.err(sp, "str(x) takes one argument");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                self.check_formattable(t, self.ctx.ast.span(args[0].id()))?;
                let src = self.last_reg;
                let dst = self.new_reg(TY_STR);
                { let (argv_off, argc) = self.pool_args(&(vec![src])); self.emit(Op::CallNat { nat: Nat::Str, recv: NOREG, argv_off, argc, dst: dst }, sp.lo); }
                return Ok(TY_STR);
            }
            _ => {}
        }
        // user free fn (monomorphized instantiation, RFC 0013 §2)
        if self.ctx.find_free_fn(name) {
            return self.compile_free_fn_call(name, generics, args, expected, sp);
        }
        // used function: signature from the surface, a direct call to the
        // exporter's scope-qualified id (RFC 0029 surface / RFC 0035 §1)
        if let Some(ef) = self.ctx.extern_fn(name).cloned() {
            // the engine-backed sleep future (RFC 0018): minting rides
            // the first `__sleep` call — the host set's `sleep` wrapper
            // is the only intended caller
            if name == sym::SLEEP_RAW {
                crate::lir::asyncfn::ensure_sleep_future(self.ctx)?;
            }
            if !generics.is_empty() {
                self.ctx.err(sp, format!("`{}` is a used fn and takes no type arguments", self.ctx.name(name)));
                return Err(());
            }
            if args.len() != ef.params.len() {
                self.ctx.err(sp, format!("call arity: {} args for {} params", args.len(), ef.params.len()));
                return Err(());
            }
            let mut aregs = Vec::new();
            for (i, a) in args.iter().enumerate() {
                let t = self.compile_expr(*a, Some(ef.params[i]))?;
                // the any-arg coercion (nmap-hostvals P3): a host-decl
                // `any` param binds EVERY rut type — the arg crosses as
                // its own slot, its call-site static type riding beside
                // it; no box, no copy, no conversion op exists to emit
                if !self.widens(t, ef.params[i]) {
                    self.ctx.err(self.ctx.ast.span(a.id()), format!(
                        "argument {} is `{}`, `{}` expected",
                        i + 1, self.ctx.type_name(t), self.ctx.type_name(ef.params[i])
                    ));
                }
                aregs.push(self.last_reg);
            }
            // the any-answer (nmap-hostvals P3): a `-> any` row has no
            // register type of its own — the CALLER's static V (the
            // context's expected type) types the dst register, and
            // `call_host`'s any write-back reads it (the trust law at
            // the decl site: the host answers the caller's V). Without
            // an expected type the dst keeps the row's own TY_VAL.
            let ret_ty = if ef.ret == rut_core::types::TY_VAL {
                expected.unwrap_or(ef.ret)
            } else {
                ef.ret
            };
            let dst = if ret_ty == TY_NIL { None } else { Some(self.new_reg(ret_ty)) };
            { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: ef.func, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
            return Ok(ret_ty);
        }
        // builtin bytes type-call: `bytes(n)` zeroed (RFC 0004)
        if name == sym::BYTES {
            return self.compile_bytes_alloc(args, sp);
        }
        // the builder's type-call: `StrBuf(cap)` (json-perf phase 2) —
        // one UTF-8 block pre-sized to `cap` octets, geometric growth
        // from there; `finish` materializes the immutable str
        if name == sym::STRBUF {
            let cap = match args.len() {
                0 => {
                    let z = self.new_reg(TY_I32);
                    self.emit(Op::ConstRaw { dst: z, bits: 0 }, sp.lo);
                    z
                }
                1 => {
                    let t = self.compile_expr(args[0], Some(TY_I32))?;
                    if t != TY_I32 {
                        self.ctx.err(sp, format!("StrBuf(cap) takes an `i32` capacity hint, found `{}`", self.ctx.type_name(t)));
                    }
                    self.last_reg
                }
                _ => {
                    self.ctx.err(sp, "StrBuf() or StrBuf(cap)");
                    return Err(());
                }
            };
            let dst = self.new_reg(TY_STRBUF);
            { let (argv_off, argc) = self.pool_args(&(vec![cap])); self.emit(Op::CallNat { nat: Nat::StrBufNew, recv: NOREG, argv_off, argc, dst }, sp.lo); }
            return Ok(TY_STRBUF);
        }
        // the weak box's type-call: `Weak(v)` (RFC 0017 v1) — the
        // `opaque(v)` construction law: a CALL of the type name itself.
        // Exactly one argument; its type is the instantiation's elem and
        // must be a reference type (every non-primitive is — the
        // `Weak<i32>` admission diagnoses here, the `is_ref` gate).
        if name == sym::WEAK {
            if args.len() != 1 {
                self.ctx.err(sp, "Weak(v) takes exactly one argument — the value to weak-reference");
                return Err(());
            }
            let elem = self.compile_expr(args[0], None)?;
            if !self.ctx.types.is_ref(elem) {
                self.ctx.err(sp, format!(
                    "weak needs a reference type — `{}` moves by value",
                    self.ctx.type_name(elem)
                ));
                return Err(());
            }
            let weak_ty = self.ctx.mk_weak(elem);
            let src = self.last_reg;
            let dst = self.new_reg(weak_ty);
            self.emit(Op::WeakNew { dst, src, ty: weak_ty }, sp.lo);
            return Ok(weak_ty);
        }
        if self.ctx.find_data(name).is_some() {
            self.ctx.err(sp, format!(
                "construction is a method call, never a type-call —use a class method ({}.new(..)) or a struct literal `{} {{ .. }}` (RFC 0010 §1)",
                self.ctx.name(name), self.ctx.name(name)
            ));
            return Err(());
        }
        let msg = self
            .ctx
            .not_in_core_scope(name)
            .unwrap_or_else(|| format!("unknown function `{}`", self.ctx.name(name)));
        self.ctx.err(sp, msg);
        Err(())
    }


    /// mut-binding law (RFC 0003 §1): writing through a handle requires the
    /// head binding to be `let mut`
    /// RFC 0012 §4: implicit widening — exact > trait-typed when an impl
    /// is REGISTERED for the (trait, type) pair. Nominal: no structural
    /// shape is ever consulted. Same-type always widens.
    /// RFC 0043 §A5: a value of the enclosing class's generic parameter
    /// also widens through its recorded `requires` bound — the registry
    /// hit normally answers first (admission proved the impl at the
    /// instantiation), so this only carries a body whose impl is not
    /// (yet) registered.
    pub(crate) fn widens(&mut self, from: TypeId, to: TypeId) -> bool {
        if from == to {
            return true;
        }
        // the any-arg coercion (nmap-hostvals P3): a host-decl `any` param
        // binds EVERY rut type — the arg crosses as its own slot, the call
        // site's static type riding beside it (the boundary decodes from
        // `FuncDef.regs` there). No box, no copy: TY_VAL never names a rut
        // value (host-decl-only), so this arm can only fire against a
        // `.d.rut` row's param — and the answer direction runs through the
        // dst register's own expected type, never through this arm.
        if to == rut_core::types::TY_VAL {
            return true;
        }
        if let TyKind::TraitObj { trait_id } = self.ctx.types.kind(to).clone() {
            if self.ctx.find_impl_ex(trait_id, from).is_some() {
                return true;
            }
            // the parameterized-trait-impl door (the phase-2 dispatch
            // half): on a miss, a registered template
            // (`impl Readable<T> for Source<T>`) unifies against the
            // concrete instantiation and MINTS the pair — `Source<str>`
            // widens to `Readable<str>` exactly when the template's
            // substitution says so
            if self.ctx.find_or_mint_impl(trait_id, from).is_some() {
                return true;
            }
            let Some(dname) = self.current_class else {
                return false;
            };
            let Some(d) = self.ctx.find_data(dname).cloned() else {
                return false;
            };
            for (g, bnode) in &d.requires {
                let Some(&conc) = self.subst.iter().find(|(n, _)| n == g).map(|(_, t)| t) else {
                    continue;
                };
                if conc != from {
                    continue;
                }
                let members = self.ctx.resolve_bound_members(*bnode, &self.subst);
                if members
                    .iter()
                    .any(|m| matches!(m, crate::check::BoundMember::Trait(tid) if *tid == trait_id))
                {
                    return true;
                }
            }
            return false;
        }
        false
    }

    pub(crate) fn check_recv_mut(&mut self, recv: NodeHandle<AnyExpr>, sp: rut_lexer::span::Span, what: &str) -> bool {
        match self.ctx.ast.expr(recv).clone() {
            ExprKind::Path { segs } if segs.len() == 1 => {
                if let Some(l) = self.lookup(segs[0].name) {
                    // the mut-binding law is uniform under the by-reference
                    // regime (RFC 0044): every binding shares its cell, so
                    // ANY store through the binding — including through a
                    // `?T` head that derefs first — requires `let mut`
                    // (the old pointer exception is gone)
                    if !l.is_mut && !l.loop_var {
                        self.ctx.err(sp, format!(
                            "{what} requires a `let mut` binding (the mut-binding law, RFC 0003 §1)"
                        ));
                        return false;
                    }
                }
                true
            }
            ExprKind::Field { recv: inner, .. } | ExprKind::Index { recv: inner, .. } => {
                self.check_recv_mut(inner, sp, what)
            }
            _ => true,
        }
    }

}
