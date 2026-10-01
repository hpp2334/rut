//! Builtin receivers: `bytes(n)` allocation, the `?T` erasure-box recovery, and static calls to named builtins.

use crate::check::{DataDecl, TcResult};
use rut_core::sym;
use crate::lir::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    /// `bytes(n)` — a zeroed immutable buffer of `n` octets.
    pub(crate) fn compile_bytes_alloc(&mut self, args: Vec<NodeHandle<AnyExpr>>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        let len_reg = match args.len() {
            0 => {
                let z = self.new_reg(TY_I32);
                self.emit(Op::ConstRaw { dst: z, bits: 0 }, sp.lo);
                z
            }
            1 => {
                let t = self.compile_expr(args[0], Some(TY_I32))?;
                if t != TY_I32 {
                    self.ctx.err(sp, format!("bytes(n) takes an `i32` length, found `{}`", self.ctx.type_name(t)));
                }
                self.last_reg
            }
            _ => {
                self.ctx.err(sp, "bytes() or bytes(n)");
                return Err(());
            }
        };
        let dst = self.new_reg(TY_BYTES);
        self.emit(Op::ArrNew { dst, ty: TY_BYTES, len: len_reg, repr: self.ctx.types.repr_of(TY_U8) }, sp.lo);
        Ok(TY_BYTES)
    }

    /// The `?T` erasure-box recovery (refval-round2): tidof +
    /// icmp + br (the reified-type-id check) + the ALIAS
    /// handoff. No allocation on EITHER branch — the `(T, bool)` tuple's
    /// `MakeRecord` mint (a record cell + field copy + bool + rc per
    /// call, ~175 ns/get in the round-1 attribution) is gone: a match
    /// hands the box ITSELF back as the `?T` (one `MovRef` retain — the
    /// box shares its inner cell, the one-cell law; a prim payload's
    /// bits were copied at `opaque(v)` construction, so the handoff is
    /// value semantics for free), a mismatch is the null slot (nil).
    /// The `?T` auto-deref (`GetF` field 0) reads through the box.
    pub(crate) fn emit_opaque_downcast(&mut self, want: TypeId, orecv: u16, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // refval-round2: downcast yields `?T` — nil on a mismatch (RFC
        // 0014 amended); the zero-value-on-false `(T, bool)` is gone
        let nty = self.ctx.mk_opt(want);
        let tid_reg = self.new_reg(TY_U32);
        self.emit(Op::TidOf { dst: tid_reg, obj: orecv }, sp.lo);
        let want_reg = self.new_reg(TY_U32);
        let wk = self.konst(ConstVal::TypeId(want));
        self.emit(Op::Const { dst: want_reg, k: wk as u32 }, sp.lo);
        let eq = self.new_reg(TY_BOOL);
        self.emit(cmpop(CmpOp::Eq, PrimTy::U32, eq, tid_reg, want_reg), sp.lo);
        let dst = self.new_reg(nty);
        let l_some = self.new_label();
        let l_none = self.new_label();
        let l_end = self.new_label();
        self.br(eq, l_some, l_none);
        self.bind(l_some);
        // the ALIAS handoff: the result register takes the box's handle
        // (MovRef = one retain). The box is the `?T` — its payload slot
        // IS field 0 to every nullable use (deref, nil check).
        self.emit(Op::MovRef { dst, src: orecv }, sp.lo);
        self.jmp(l_end);
        self.bind(l_none);
        // the mismatch: the null slot — no cell, no zero value
        self.emit(Op::ConstRaw { dst, bits: 0 }, sp.lo);
        self.bind(l_end);
        Ok(nty)
    }

    /// Another module's impl row answering a NO-SELF static through the
    /// type name: `(extern impl index, method index)` for the (trait,
    /// class-template) impl whose method `name` takes no receiver.
    /// The trait must be callable in scope (the use-both law).
    fn extern_impl_no_self_static(&self, target_template: TypeId, name: IdentId) -> Option<(usize, usize)> {
        for (eidx, im) in self.ctx.extern_impls.iter().enumerate() {
            if im.target != target_template {
                continue;
            }
            let tname = im.trait_name;
            let callable = self.ctx.find_trait(tname).is_some()
                || self.ctx.used.contains(&tname)
                || self.ctx.extern_traits.contains_key(&tname);
            if !callable {
                continue;
            }
            let tdesc = self.ctx.trait_by_id(im.trait_id);
            for (midx, tm) in tdesc.methods.iter().enumerate() {
                if tm.name != name {
                    continue;
                }
                // v1 shape law: the member must be no-self IN THE TRAIT
                // (the descriptor's params are the whole parameter list —
                // a receiver method's call form is the value dot-call,
                // routed elsewhere). `FromFlow::from_flow` is the shape's
                // first citizen; arity mismatches diagnose at the call.
                return Some((eidx, midx));
            }
        }
        None
    }

    pub(crate) fn compile_static_call(
        &mut self,
        base: IdentId,
        base_generics: Vec<NodeHandle<AnyTy>>,
        member: IdentId,
        member_generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        // core builtin statics: the erasure primitive's
        // `new`/`downcast` statics and the `bytes`/`str` constructors —
        // AMBIENT now: the arms fire
        // whenever core is mounted, no `use` required
        let core_ty = self.ctx.extern_native_types.get(&base).copied();
        // Explicit type args on a static head are meaningful only where the
        // member can use them (`Vec<u32>.from(..)` — the element type);
        // everywhere else they stay unsupported rather than silently ignored.
        // A used class (own or linked — the linkable-classes phase) takes
        // them the same way.
        let is_data = self.ctx.find_data(base).is_some()
            || self.ctx.extern_types.contains_key(&base)
            || self.ctx.extern_generics.contains_key(&base);
        if !base_generics.is_empty() && !is_data {
            self.ctx.err(sp, format!(
                "generic type paths (`{}<..>.{}`) are not supported in this build",
                self.ctx.name(base), self.ctx.name(member)
            ));
            return Err(());
        }
        // a used namespace's members (`Math.sqrt`) —
        // routed by the bound head, name-generic
        if self.ctx.is_extern_namespace(base) {
            return self.compile_namespace_member(base, member, &args, expected, sp);
        }
        match (base, member) {
            (sym::BYTES, sym::FROM) => {
                // bytes.from(a) — copy an Array<u8> into an immutable
                // buffer
                if args.len() != 1 {
                    self.ctx.err(sp, "bytes.from(source) takes one `Array<u8>`");
                    return Err(());
                }
                let hint = Some(self.ctx.mk_array(TY_U8));
                let at = self.compile_expr(args[0], hint)?;
                match self.ctx.types.kind(at) {
                    TyKind::Array { elem } if *elem == TY_U8 => {}
                    _ => {
                        self.ctx.err(sp, format!("bytes.from expects `Array<u8>` —found `{}`", self.ctx.type_name(at)));
                        return Err(());
                    }
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_BYTES);
                self.emit(Op::Own { dst, src, ty: TY_BYTES }, sp.lo);
                return Ok(TY_BYTES);
            }
            (sym::BYTES, sym::ZEROED) => {
                // bytes.zeroed(n) — n zeroed octets
                if args.len() != 1 {
                    self.ctx.err(sp, "bytes.zeroed(n) takes one `i32`");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_I32))?;
                if t != TY_I32 {
                    self.ctx.err(sp, "bytes.zeroed takes an `i32`");
                }
                let len_reg = self.last_reg;
                let dst = self.new_reg(TY_BYTES);
                self.emit(Op::ArrNew { dst, ty: TY_BYTES, len: len_reg, repr: self.ctx.types.repr_of(TY_U8) }, sp.lo);
                return Ok(TY_BYTES);
            }
            (sym::STR, sym::FROM_CODE) => {
                // str.from_code(n) -> str — the 1-codepoint str for the
                // codepoint `n` (`char` is gone)
                if args.len() != 1 {
                    self.ctx.err(sp, "str.from_code(n) takes one `u32` codepoint");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_U32))?;
                if t != TY_U32 {
                    self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                        "str.from_code takes a `u32`, found `{}`",
                        self.ctx.type_name(t)
                    ));
                }
                let cp = self.last_reg;
                let dst = self.new_reg(TY_STR);
                { let (argv_off, argc) = self.pool_args(&(vec![cp])); self.emit(Op::CallNat { nat: Nat::StrFromCode, recv: NOREG, argv_off, argc, dst: dst }, sp.lo); }
                return Ok(TY_STR);
            }
            (sym::WEAK, sym::NEW) => {
                // Weak.new(v) — the weak box's construction (v1):
                // the class-method form of the old type-call.
                // Exactly one argument; its type is the instantiation's
                // elem and must be a reference type (every non-primitive
                // is — the `Weak<i32>` admission diagnoses here, the
                // `is_ref` gate). The op consumes the argument's
                // temporary (the one consuming op).
                if args.len() != 1 {
                    self.ctx.err(sp, "Weak.new(v) takes exactly one argument — the value to weak-reference");
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
            _ => {}
        }
        // the erasure primitive's member static (builtin-
        // surface phase 2): `opaque.downcast<T>(o)` — construction moved
        // to the call form `opaque(v)`. The lowercase
        // spelling IS the interner's own (`sym::OPAQUE`)
        if core_ty == Some(rut_core::binary::NativeTy::Opaque) && base == sym::OPAQUE {
            match member {
                sym::DOWNCAST => {
                    if args.len() != 1 || member_generics.len() != 1 {
                        self.ctx.err(sp, "opaque.downcast<T>(o) takes one explicit type argument and one value");
                        return Err(());
                    }
                    let want = self.resolve_type_now(member_generics[0]);
                    // The erasure box recovers concrete types; the ONE trait-
                    // object exception is the engine-woven Future (the
                    // async lane's crossings — phase 2b): the box stores
                    // the construction site's `Future<T>` object spelling,
                    // so `downcast<Future<nil>>` matches the sleep mint's
                    // seal by the same id. Every other trait object keeps
                    // the refusal (the amendment lands with the
                    // any-removal phase).
                    let downcastable = match self.ctx.types.kind(want) {
                        TyKind::TraitObj { trait_id } => {
                            let tid = *trait_id;
                            self.ctx
                                .trait_inst
                                .iter()
                                .find(|(_, &id)| id == tid)
                                .map(|((n, _), _)| *n == sym::FUTURE)
                                .unwrap_or(false)
                        }
                        _ => true,
                    };
                    if !downcastable {
                        self.ctx.err(sp, "downcast needs a CONCRETE type —trait objects have no recovery path");
                        return Err(());
                    }
                    let t = self.compile_expr(args[0], Some(TY_OPAQUE))?;
                    if t != TY_OPAQUE {
                        self.ctx.err(sp, "downcast takes an `opaque` box");
                        return Err(());
                    }
                    let orecv = self.last_reg;
                    return self.emit_opaque_downcast(want, orecv, sp);
                }
                _ => {
                    self.ctx.err(sp, format!(
                        "`opaque` has no static `{}` — the primitive's member is `downcast<T>`; construct with `opaque(v)`",
                        self.ctx.name(member)
                    ));
                    return Err(());
                }
            }
        }
        // enum helpers: Color.to_int(c)
        if self.ctx.name(member) == "to_int" {
            if let Some(e) = self.ctx.find_enum(base).cloned() {
                let _ = e;
                self.ctx.err(sp, "enum to_int/from_int are not supported in this build");
                return Err(());
            }
        }
        // class method call: `Circle.new(..)` — resolve the
        // class BY NAME first, then its member: searching for the first
        // class with a same-named method would shadow every later class
        // (two `new`s in one module made the second uncallable)
        if let Some((dname, d)) = self.ctx.datas.iter().find(|(n, _)| *n == base).map(|(n, d)| (*n, d.clone())) {
            if let Some((_, fmnode)) = d.methods.iter().find(|(m, _)| *m == member).cloned() {
                // The mint: (decl name, decl, method node, class args).
                //
                // The class is looked up BY NAME: `HashMap<K, i64>` IS
                // the generic class (one name, one decl — the row form
                // is repealed), so the explicit-arguments arm instanti-
                // ates it directly and the inference arm follows the
                // expected type's own instantiation.
                let mint: (IdentId, DataDecl, NodeHandle<MethodDeclNode>, Vec<TypeId>) = if !base_generics.is_empty() {
                    let args: Vec<TypeId> = base_generics.iter().map(|g| self.resolve_type_now(*g)).collect();
                    (dname, d.clone(), fmnode, args)
                } else if !d.generics.is_empty() && self.current_class == Some(dname) {
                    // a `Self`-ish call inside the class body: the enclosing
                    // method's class args
                    let args: Vec<TypeId> = d
                        .generics
                        .iter()
                        .map(|g| {
                            self.subst
                                .iter()
                                .find(|(n, _)| n == g)
                                .map(|(_, t)| *t)
                                .unwrap_or(TY_I32)
                        })
                        .collect();
                    (dname, d.clone(), fmnode, args)
                } else if !d.generics.is_empty() {
                    // infer from the expected type: `let b: Box<i32> = Box.new(..)`
                    match expected.and_then(|e| self.ctx.inst_data.get(&e).cloned()) {
                        Some((ed, eargs)) if ed == dname => (dname, d.clone(), fmnode, eargs),
                        _ => {
                            self.ctx.err(sp, format!(
                                "cannot infer the type arguments for `{b}` — write `{b}<..>.{m}(..)` or annotate the binding",
                                b = self.ctx.name(base), m = self.ctx.name(member)
                            ));
                            return Err(());
                        }
                    }
                } else {
                    (dname, d.clone(), fmnode, vec![])
                };
                let (dname, d, mnode, class_args) = mint;
                if !d.generics.is_empty() && class_args.len() != d.generics.len() {
                    self.ctx.err(sp, format!(
                        "`{}`<..> takes {} type argument(s), {} given",
                        self.ctx.name(dname),
                        d.generics.len(),
                        class_args.len()
                    ));
                    return Err(());
                }
                let self_ty = if d.generics.is_empty() {
                    d.ty
                } else {
                    self.ctx.mk_data_inst(dname, class_args.clone(), sp)
                };
                return self.compile_direct_method(dname, class_args, self_ty, mnode, args, sp);
            }
            // no inherent member: a trait impl's NO-SELF static —
            // `Vec.from_flow(it)` (the FromFlow surface). The class
            // instantiation rides the same explicit-or-expected mint.
            if !d.methods.iter().any(|(m, _)| *m == member) {
                let concrete = if !d.generics.is_empty() {
                    let cargs = if !base_generics.is_empty() {
                        base_generics.iter().map(|g| self.resolve_type_now(*g)).collect()
                    } else {
                        match expected.and_then(|e| self.ctx.inst_data.get(&e).cloned()) {
                            Some((ed, eargs)) if ed == dname => eargs,
                            _ => {
                                self.ctx.err(sp, format!(
                                    "cannot infer the type arguments for `{b}` — write `{b}<..>.{m}(..)` or annotate the binding",
                                    b = self.ctx.name(base), m = self.ctx.name(member)
                                ));
                                return Err(());
                            }
                        }
                    };
                    self.ctx.mk_data_inst(dname, cargs, sp)
                } else {
                    d.ty
                };
                if self.find_trait_impl_method(concrete, member).is_some() {
                    return self.compile_trait_name_static_call(concrete, member, args, expected, sp);
                }
            }
        }
        // used classes (the linkable-classes phase): the surface's
        // inherent rows answer static (class-method) calls — a plain
        // class binds the exporter's fn, a generic class resolves its
        // arguments here (explicit ones, else the expected type's own
        // instantiation) and mints the mirror instantiation the
        // owner's unit compiles. (Generic classes ride `extern_types`
        // too — the template row — so the generic arm answers first.)
        if let Some(g) = self.ctx.extern_generics.get(&base).cloned() {
            // a trait impl's no-self static over the used generic class —
            // `Vec.from_flow(it)` (the FromFlow sink), the impl row
            // riding the exporter's surface
            if let Some((eidx, midx)) = self.extern_impl_no_self_static(g.template, member) {
                let class_args = if !base_generics.is_empty() {
                    base_generics.iter().map(|gn| self.resolve_type_now(*gn)).collect()
                } else {
                    // infer from the expected type's instantiation
                    match expected.and_then(|e| self.ctx.inst_data.get(&e).cloned()) {
                        Some((ed, eargs)) if ed == base => eargs,
                        _ => {
                            self.ctx.err(sp, format!(
                                "cannot infer the type arguments for `{b}` — write `{b}<..>.{m}(..)` or annotate the binding",
                                b = self.ctx.name(base), m = self.ctx.name(member)
                            ));
                            return Err(());
                        }
                    }
                };
                let concrete = self.ctx.mk_data_inst(base, class_args.clone(), sp);
                let subst = self
                    .ctx
                    .inst_data
                    .get(&concrete)
                    .cloned()
                    .map(|(_, a)| a)
                    .unwrap_or(class_args);
                return self.compile_extern_impl_template_static(eidx, midx, concrete, subst, args, expected, sp);
            }
            if let Some(ih) = self.ctx.extern_inherents.iter().position(|x| x.target == g.template) {
                if let Some(midx) = self.ctx.extern_inherents[ih].methods.iter().position(|(n, .., has_self, _)| !*has_self && *n == member) {
                    let class_args = if !base_generics.is_empty() {
                        base_generics.iter().map(|gn| self.resolve_type_now(*gn)).collect()
                    } else {
                        // infer from the expected type's instantiation
                        match expected.and_then(|e| self.ctx.inst_data.get(&e).cloned()) {
                            Some((ed, eargs)) if ed == base => eargs,
                            _ => {
                                self.ctx.err(sp, format!(
                                    "cannot infer the type arguments for `{b}` — write `{b}<..>.{m}(..)` or annotate the binding",
                                    b = self.ctx.name(base), m = self.ctx.name(member)
                                ));
                                return Err(());
                            }
                        }
                    };
                    let self_ty = self.ctx.mk_data_inst(base, class_args.clone(), sp);
                    return self.compile_extern_class_method_call(ih, midx, Some(base), class_args, Some(self_ty), args, sp);
                }
            }
        }
        if let Some(&t) = self.ctx.extern_types.get(&base) {
            if let Some(ih) = self.ctx.extern_inherents.iter().position(|x| x.target == t) {
                if let Some(midx) = self.ctx.extern_inherents[ih].methods.iter().position(|(n, .., has_self, _)| !*has_self && *n == member) {
                    return self.compile_extern_class_method_call(ih, midx, None, vec![], None, args, sp);
                }
            }
            // no inherent row: a trait impl's no-self static over the
            // used class (`Vec.from_flow(it)` — the FromFlow sink)
            if self.find_trait_impl_method(t, member).is_some() {
                return self.compile_trait_name_static_call(t, member, args, expected, sp);
            }
        }
        // enum statics: `Color.default()` — the decl's own methods,
        // the struct/class arm's shape with no generics and no
        // instantiation mint
        if let Some(e) = self.ctx.find_enum(base).cloned() {
            if let Some((_, mnode)) = e.methods.iter().find(|(m, _)| *m == member).cloned() {
                return self.compile_direct_method(base, vec![], e.ty, mnode, args, sp);
            }
        }
        let msg = self
            .ctx
            .not_in_core_scope(base)
            .unwrap_or_else(|| format!("unknown name `{}.{}`", self.ctx.name(base), self.ctx.name(member)));
        self.ctx.err(sp, msg);
        Err(())
    }

}
