//! Calls and member resolution: builtin/free-fn/method/static dispatch,
//! generic instantiation by unification, the
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
// inline-at-site fast paths), `extern_class` (used-class method
// calls — the linkable-classes phase), `field` (field reads + union
// guards). Shared arg-widening and receiver-mut helpers stay here.
mod builtin;
mod extern_class;
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
                // the callee derefs: `let f =
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
            if !self.widens_val(*a, t, ptys[i]) {
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
            // the qualified call (`layout.mk(..)`) rides FIRST: the
            // head names a child mod of the current module or of an
            // ancestor. `None` falls to the static-call form
            // (`Type.member(..)`) — today's arm, unchanged.
            if let Some(r) =
                self.compile_qualified_call(&segs[..1], segs[1].name, &segs[1].generics, &args, expected, sp)
            {
                return r;
            }
            let base = segs[0].name;
            let member = segs[1].name;
            // a used package's name as the head: uses are the only
            // cross-package door — the diagnostic names the use fix
            if self.ctx.used_pkgs.contains(self.ctx.name(base)) {
                self.package_head_error(&segs[..1], segs[1].name, sp);
                return Err(());
            }
            // the static head is a bare type name: it resolves in the
            // current module (phase 3). A type of another module names
            // its qualified spelling.
            if let Some(home) = self.ctx.type_home(base) {
                if home != self.ctx.cur_mod {
                    let q = home.replace('/', ".");
                    let prefix = if q.is_empty() { String::new() } else { format!("{q}.") };
                    self.ctx.err(
                        sp,
                        format!(
                            "`{}` is declared in module `{home}` ({}) — qualify it: `{}{}.{}`(..)",
                            self.ctx.name(base),
                            crate::check::ModInputs::display(&home),
                            prefix,
                            self.ctx.name(base),
                            self.ctx.name(member),
                        ),
                    );
                    return Err(());
                }
            }
            let base_generics = segs[0].generics.clone();
            let member_generics = segs[1].generics.clone();
            return self.compile_static_call(base, base_generics, member, member_generics, args, expected, sp);
        }
        if segs.len() >= 3 {
            // `pkg.a.Type.member(..)` — the unified mod walk, then the
            // leaf or the static form on the walked module's type
            if let Some(r) = self.compile_qualified_call(
                &segs[..segs.len() - 1],
                segs[segs.len() - 1].name,
                &segs[segs.len() - 1].generics,
                &args,
                expected,
                sp,
            ) {
                return r;
            }
            if self.ctx.used_pkgs.contains(self.ctx.name(segs[0].name)) {
                self.package_head_error(&segs[..segs.len() - 1], segs[segs.len() - 1].name, sp);
                return Err(());
            }
            self.ctx.err(
                sp,
                format!(
                    "unknown call path `{}` — calls go through a module (`mod.fn(..)`) or a type (`Type.member(..)`)",
                    segs.iter().map(|s| self.ctx.name(s.name).to_string()).collect::<Vec<_>>().join(".")
                ),
            );
            return Err(());
        }
        let name = segs[0].name;
        let generics = segs[0].generics.clone();
        // core prelude functions: compiler-lowered, visible
        // only when the name was used from the core surface — the
        // prelude is used, never ambient. A local fn of the same name
        // wins when the use statement is absent (fallthrough below).
        // Retired prelude spellings (`assert`, `print`, `i32(x)`, …)
        // are ordinary identifiers: they fall to the resolution miss
        // at the bottom and diagnose as the unknown names they are.
        let core_fn = self.ctx.extern_native_fns.contains(&name);
        match name {
            sym::OPAQUE => {
                // `opaque(v)`: the erasure box. The payload IS `v` —
                // a record/str/bytes binding shares its cell, a primitive
                // is copied, and a `?T` binding stores the nullable box
                // (the old `&v`-then-box shape, one spelling shorter).
                if args.len() != 1 {
                    self.ctx.err(sp, "opaque(v) takes one value");
                    return Err(());
                }
                let t = self.compile_expr(args[0], None)?;
                // The erasure box seals concrete values; the ONE trait-object
                // exception is the engine-woven Future (the async lane's
                // crossings — phase 2b): `launch_future` hands the frame
                // to the driving loop through the erasure box, and the
                // box records the `Future<T>` object spelling so the
                // `downcast<Future<..>>` recovery matches. Every other
                // trait object keeps the refusal (the amendment:
                // with `any` gone, the erasure box is the only value
                // lane — polymorphism crosses sealed).
                let sealed = match self.ctx.types.kind(t) {
                    TyKind::IfaceObj { iface_id } => {
                        let tid = *iface_id;
                        self.ctx
                            .iface_inst
                            .iter()
                            .find(|(_, &id)| id == tid)
                            .map(|((n, _), _)| *n == sym::FUTURE)
                            .unwrap_or(false)
                    }
                    _ => true,
                };
                if !sealed {
                    self.ctx.err(sp, "`opaque` rejects interface objects —they are never boxed");
                    return Err(());
                }
                let src = self.last_reg;
                let dst = self.new_reg(TY_OPAQUE);
                self.emit(Op::Box { dst, val: src, ty: t }, sp.lo);
                return Ok(TY_OPAQUE);
            }
            sym::UNOPAQUE => {
                // `unopaque<T>(b)`: the erasure box's inverse — the
                // typed recovery. Where `downcast<T>` is the checked
                // `?T` (a mismatch is `nil`), `unopaque<T>` is the
                // bad-cast lane: a box whose runtime type is not `T`
                // traps `BadUnbox` naming both types (`Op::Unbox` is
                // the runtime half; the compiler guards what it can
                // know). The seal law is knowable at compile time and
                // enforced HERE: an interface object can never sit in
                // a box (the ONE Future exception rides the async
                // lane), so `unopaque<Drawable>` diagnoses without a
                // run. Every other mismatch is a runtime fact — the
                // box's content is erased statically.
                let (want_node, args) = match generics.as_slice() {
                    [g] => (*g, args),
                    _ => {
                        self.ctx.err(sp, "unopaque<T>(b) takes one explicit type argument and one `opaque` value");
                        return Err(());
                    }
                };
                if args.len() != 1 {
                    self.ctx.err(sp, "unopaque<T>(b) takes one explicit type argument and one `opaque` value");
                    return Err(());
                }
                let want = self.resolve_type_now(want_node);
                let recoverable = match self.ctx.types.kind(want) {
                    TyKind::IfaceObj { iface_id } => {
                        let tid = *iface_id;
                        self.ctx
                            .iface_inst
                            .iter()
                            .find(|(_, &id)| id == tid)
                            .map(|((n, _), _)| *n == sym::FUTURE)
                            .unwrap_or(false)
                    }
                    _ => true,
                };
                if !recoverable {
                    self.ctx.err(sp, "unopaque needs a CONCRETE type — interface objects are never boxed, the recovery can never match");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_OPAQUE))?;
                if t != TY_OPAQUE {
                    self.ctx.err(sp, "unopaque takes an `opaque` box");
                    return Err(());
                }
                let o = self.last_reg;
                let dst = self.new_reg(want);
                self.emit(Op::Unbox { dst, box_: o, ty: want }, sp.lo);
                return Ok(want);
            }
            sym::PANIC if core_fn => {
                if args.len() != 1 {
                    self.ctx.err(sp, "panic(msg) takes a message");
                    return Err(());
                }
                let t = self.compile_expr(args[0], Some(TY_STR))?;
                if t != TY_STR {
                    self.ctx.err(sp, "panic takes a `str`");
                }
                self.emit(Op::Panic { msg: self.last_reg }, sp.lo);
                return Ok(TY_NIL);
            }
            sym::CAPTURE_STACKTRACE if core_fn => {
                // capture_stacktrace() -> StackTrace (err-channel
                // phase 2): the frame walk is the VM's, at
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
                // native sizes once and allocates once.
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
                // compile-time constant —never executed
                if generics.len() != 1 || !args.is_empty() {
                    self.ctx.err(sp, format!("{}<T>() takes one explicit type argument", self.ctx.name(name)));
                    return Err(());
                }
                let t = self.resolve_type_now(generics[0]);
                let dst = self.new_reg(TY_U32);
                // a rebasable const-pool entry: link maps the module-local
                // TypeId into the global table
                let k = self.konst(ConstVal::TypeId(t));
                self.emit(Op::Const { dst, k: k as u32 }, sp.lo);
                return Ok(TY_U32);
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
        // user free fn (monomorphized instantiation) — single names
        // resolve in the CURRENT module (phase 3; flat packages gate
        // identically: every fn is the root's)
        if self.ctx.fn_is_here(name) {
            return self.compile_free_fn_call(name, generics, args, expected, sp);
        }
        // used function: signature from the surface, a direct call to the
        // exporter's scope-qualified id. The binding gate is per file
        // (phase 3): the name binds in the module whose file used it.
        if self.ctx.use_bound_here(name) {
        if let Some(ef) = self.ctx.extern_fn(name).cloned() {            // the host future lane (phase 4): an async host fn's call
            // mints the cold engine-woven frame over `__start`'s state
            // cell — the weave owns the call site
            if ef.is_async {
                return crate::lir::asyncfn::compile_host_async_call(self, name, &ef, &args, expected, sp);
            }
            // the engine-backed sleep future: minting rides
            // the first `__sleep` call — the host set's `sleep` wrapper
            // is the only intended caller
            if name == sym::SLEEP_RAW {
                crate::lir::asyncfn::ensure_sleep_future(self.ctx)?;
            }
            // the structured-competition rows: minting rides the wrapper's
            // call, keyed on the type arguments the wrapper spells (the
            // concrete answers under its own substitution). Type args ride
            // the CALL spelling — the decl rows stay concrete (`opaque`
            // wires), the types are compile-time only.
            // the structured-competition rows: minting rides the wrapper's
            // call, keyed on the type arguments the wrapper spells (the
            // concrete answers under its own substitution). Type args ride
            // the CALL spelling — the decl rows stay concrete (`opaque`
            // wires), the types are compile-time only — so the rows
            // CONSUME their type arguments where every other used fn
            // rejects them.
            // the rows are matched by NAME TEXT (not well-known symbols —
            // the well-known table is binary-format surface, pinned by the
            // committed bundles): the caller's interner binds the extern
            // row and the call from the same table, so the text compare is
            // exact where it runs.
            let row_name = self.ctx.name(name).to_string();
            let row_takes_type_args =
                row_name == "__select2" || row_name == "__select_all" || row_name == "__completer";
            if row_takes_type_args {
                let (want, what) = if row_name == "__select2" {
                    (2, "`__select2<T, U>` takes the two futures' answer types — the typed surface is futures' `select2`")
                } else {
                    (1, "the row takes the answer type as its type argument — the typed surface is futures'")
                };
                if generics.len() != want {
                    self.ctx.err(sp, what);
                    return Err(());
                }
                let resolved: Vec<TypeId> = generics.iter().map(|g| self.resolve_type_now(*g)).collect();
                if row_name == "__select2" {
                    crate::lir::asyncfn::ensure_select_future(self.ctx, resolved[0], resolved[1], sp)?;
                } else if row_name == "__select_all" {
                    crate::lir::asyncfn::ensure_select_all_future(self.ctx, resolved[0], sp)?;
                } else {
                    crate::lir::asyncfn::ensure_completer_future(self.ctx, resolved[0])?;
                }
            }
            if !generics.is_empty() && !row_takes_type_args {
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
                if !self.widens_val(*a, t, ef.params[i]) {
                    self.ctx.err(self.ctx.ast.span(a.id()), format!(
                        "argument {} is `{}`, `{}` expected",
                        i + 1, self.ctx.type_name(t), self.ctx.type_name(ef.params[i])
                    ));
                }
                aregs.push(self.last_reg);
            }
            let dst = if ef.ret == TY_NIL { None } else { Some(self.new_reg(ef.ret)) };
            { let (argv_off, argc) = self.pool_args(&(aregs)); self.emit(Op::Call { func: ef.func, argv_off, argc, dst: opt_reg(dst) }, sp.lo); }
            return Ok(ef.ret);
        }
        }
        // a used GENERIC fn (the linkable-classes phase): the body
        // lives per argument list in the owner — infer the type
        // arguments against the placeholder signature, mint the mirror
        // instantiation, and request the body
        if self.ctx.use_bound_here(name) {
        if let Some(gf) = self.ctx.extern_generic_fn(name).cloned() {
            return self.compile_extern_generic_fn_call(name, &gf, generics, args, expected, sp);
        }
        }
        // builtin bytes type-call: `bytes(n)` zeroed
        if name == sym::BYTES {
            return self.compile_bytes_alloc(args, sp);
        }
        // the weak box's retired type-call spelling: construction is a
        // class method now — `Weak.new(v)` (the Weak arm in
        // compile_static_call carries the semantics). The loud,
        // actionable diagnostic names the fix.
        if name == sym::WEAK {
            self.ctx.err(
                sp,
                "`Weak(v)` is not a function — construction is a class method: `Weak.new(v)`",
            );
            return Err(());
        }
        // the local type-call (`JsonI64(64)`, the designated `Point(..)`)
        // resolves in the CURRENT module (phase 3)
        if let Some(d) = self.ctx.data_here(name) {
            // the newtype decl's call construction — `JsonI64(64)`: the
            // spelled constructor, the manufacture mechanism. The arm
            // stays FIRST (Law: the newtype's call surface already IS
            // the construction — the marker can never compete with it,
            // the checker rejects that pairing outright). An
            // ordinary (braced) class falls through to its designated
            // constructor, the seal holds.
            if d.newtype {
                return self.compile_newtype_ctor(name, &d, generics, args, expected, sp);
            }
            // the `[constructor]` designation — `Point(..)` lowers
            // exactly like `Point.from_xy(..)` (byte-identical: the
            // class-method call is the construction, the bracket only
            // aims it)
            if let Some(&ctor) = self.ctx.class_ctors.get(&name) {
                return self.compile_static_call(name, generics, ctor, vec![], args, expected, sp);
            }
            if d.kind == crate::check::DataKind::Class {
                self.ctx.err(sp, format!(
                    "`{}` constructs through its class methods (`{0}.new(..)`) — mark one `[constructor]` to call the class itself",
                    self.ctx.name(name)
                ));
            } else {
                self.ctx.err(sp, format!(
                    "construction is a method call, never a type-call — use a struct literal `{} {{ .. }}`",
                    self.ctx.name(name)
                ));
            }
            return Err(());
        }
        // a USED newtype class — the surface row's flag arms the same
        // construction at a distance (the third-party adapter: the
        // consumer manufactures the wrapper over the foreign value)
        if self.ctx.extern_newtypes.contains(&name) {
            return self.compile_extern_newtype_ctor(name, generics, args, expected, sp);
        }
        // a used/linked class's designated constructor — the marker
        // byte crossed the surface row, the registry names the member:
        // the identical lowering (`Point(..)` at a distance)
        if let Some(&ctor) = self.ctx.class_ctors.get(&name) {
            return self.compile_static_call(name, generics, ctor, vec![], args, expected, sp);
        }
        // a used type's miss — a known name, a known fix: the used
        // class's construction is a (designated) class method, a used
        // struct's a literal. An unknown name keeps the ordinary miss
        // below.
        if self.ctx.use_bound_here(name) {
        if let Some(&t) = self.ctx.extern_types.get(&name) {
            if self.ctx.extern_classes.contains(&t) {
                self.ctx.err(sp, format!(
                    "`{}` constructs through its class methods (`{0}.new(..)`) — mark one `[constructor]` to call the class itself",
                    self.ctx.name(name)
                ));
            } else {
                self.ctx.err(sp, format!(
                    "construction is a method call, never a type-call — use a struct literal `{} {{ .. }}`",
                    self.ctx.name(name)
                ));
            }
            return Err(());
        }
        }
        let msg = self
            .ctx
            .not_in_core_scope(name)
            .or_else(|| self.ctx.use_elsewhere_hint(name))
            .or_else(|| self.ctx.elsewhere_hint(name, "fn"))
            .unwrap_or_else(|| format!("unknown function `{}`", self.ctx.name(name)));
        self.ctx.err(sp, msg);
        Err(())
    }

    /// The qualified CALL (`layout.column(..)`, two segments): the
    /// head names a child `mod` of the current module or of an
    /// ancestor; the leaf is that module's fn (the ordinary free-fn
    /// call) or its type-call (newtype / designated constructor),
    /// gated by the crossing predicate. `None` = the head is not a
    /// module — the caller's static-call form carries on.
    /// The cross-package head's refusal (positions, phase 3): a used
    /// package's name may head neither a value nor a type position —
    /// uses are the only cross-package door, and the diagnostic spells
    /// the exact `use` that opens it. `segs` is the receiver chain,
    /// `leaf` the trailing name (the method's, or the path's last).
    pub(crate) fn package_head_error(
        &mut self,
        segs: &[PathSeg],
        leaf: IdentId,
        sp: rut_lexer::span::Span,
    ) {
        let mut fix = String::new();
        for s in segs {
            fix.push_str(self.ctx.name(s.name));
            fix.push_str("::");
        }
        self.ctx.err(
            sp,
            format!(
                "`{}` is a package, not a module — uses are the only cross-package door: `use {}{{ {} }}` first",
                self.ctx.name(segs[0].name),
                fix,
                self.ctx.name(leaf)
            ),
        );
    }

    /// The unified qualified CALL (`layout.mk(3)`, `pkg.a.five()`,
    /// `a.c.Circle.area(..)`): `segs` is the receiver path, `name` the
    /// trailing member. The head names a child `mod` of the current
    /// module or of an ancestor (nearest scope wins; the unit's own
    /// pkg name names the root); each further segment walks child mods
    /// until one names a TYPE of the walked module — the static form
    /// (`Type.member(..)`) — or the walk exhausts and `name` is the
    /// leaf (a fn, or the module's type-call). Every crossing runs the
    /// visibility predicate. `None` = the head is not a module — the
    /// caller's ordinary arms carry on.
    pub(crate) fn compile_qualified_call(
        &mut self,
        segs: &[PathSeg],
        name: IdentId,
        generics: &[NodeHandle<AnyTy>],
        args: &[NodeHandle<AnyExpr>],
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> Option<TcResult<TypeId>> {
        if segs.is_empty() || !segs[0].generics.is_empty() {
            return None; // generics on a non-module segment: not a mod path
        }
        let from = self.ctx.cur_mod.clone();
        let Some(mut m) = self.ctx.resolve_mod_head(&from, segs[0].name) else {
            if self.ctx.used_pkgs.contains(self.ctx.name(segs[0].name)) {
                self.package_head_error(segs, name, sp);
                return Some(Err(()));
            }
            return None;
        };
        for (i, seg) in segs.iter().enumerate().skip(1) {
            // a child mod extends the walk
            if let Some((path, _vis)) = self
                .ctx
                .mods
                .scopes
                .get(&m)
                .and_then(|s| s.children.get(self.ctx.name(seg.name)))
            {
                m = path.clone();
                continue;
            }
            // a non-mod segment ends the chain — only as the LAST
            // receiver segment, the walked module's static form
            // (`Type.member(..)`)
            if i != segs.len() - 1 {
                self.ctx.err(
                    sp,
                    format!(
                        "`{}` is not a module under `{}` — only `mod` children extend a qualified path",
                        self.ctx.name(seg.name),
                        m
                    ),
                );
                return Some(Err(()));
            }
            let vis = if let Some(d) = self.ctx.data_in(&m, seg.name) {
                d.vis
            } else if let Some(e) = self.ctx.enum_in(&m, seg.name) {
                e.vis
            } else {
                self.ctx.err(
                    sp,
                    format!(
                        "`{}` is not declared in module `{m}` ({})",
                        self.ctx.name(seg.name),
                        crate::check::ModInputs::display(&m),
                    ),
                );
                return Some(Err(()));
            };
            if !self.ctx.vis_crosses(&from, &m, vis) {
                self.ctx.err(
                    sp,
                    format!(
                        "`{m}.{}(..)` — {}",
                        self.ctx.name(seg.name),
                        self.ctx.vis_hint(seg.name, &from, &m, vis)
                    ),
                );
                return Some(Err(()));
            }
            return Some(self.compile_static_call(
                seg.name,
                seg.generics.clone(),
                name,
                generics.to_vec(),
                args.to_vec(),
                expected,
                sp,
            ));
        }
        // the whole receiver walked as mods: `name` is the leaf
        let found = self.ctx.fn_in(&m, name).is_some()
            || self.ctx.data_in(&m, name).is_some()
            || self.ctx.enum_in(&m, name).is_some()
            || self.ctx.let_in(&m, name).is_some();
        if !found {
            let msg = match self.ctx.decl_home(name) {
                Some(home) => format!(
                    "`{}` is not declared in module `{m}` ({}) — it lives in module `{home}`",
                    self.ctx.name(name),
                    crate::check::ModInputs::display(&m),
                ),
                None => format!(
                    "`{}` is not declared in module `{m}` ({})",
                    self.ctx.name(name),
                    crate::check::ModInputs::display(&m),
                ),
            };
            self.ctx.err(sp, msg);
            return Some(Err(()));
        }
        let vis_gate = |ctx: &Ctx, name: IdentId, vis: Vis| -> Option<String> {
            if ctx.vis_crosses(&from, &m, vis) {
                None
            } else {
                Some(format!("`{m}. {}` — {}", ctx.name(name), ctx.vis_hint(name, &from, &m, vis)))
            }
        };
        if let Some(fnode) = self.ctx.fn_in(&m, name) {
            if let Some(msg) = vis_gate(&self.ctx, name, self.ctx.ast.fn_decl(fnode).vis) {
                self.ctx.err(sp, msg);
                return Some(Err(()));
            }
            // names are package-unique: the ordinary free-fn call
            let g = generics.to_vec();
            return Some(self.compile_free_fn_call(name, g, args.to_vec(), expected, sp));
        }
        if let Some(d) = self.ctx.data_in(&m, name) {
            if let Some(msg) = vis_gate(&self.ctx, name, d.vis) {
                self.ctx.err(sp, msg);
                return Some(Err(()));
            }
            // the type-call arms, exactly like the bare spelling
            if d.newtype {
                let g = generics.to_vec();
                return Some(self.compile_newtype_ctor(name, &d, g, args.to_vec(), expected, sp));
            }
            if let Some(&ctor) = self.ctx.class_ctors.get(&name) {
                return Some(self.compile_static_call(
                    name,
                    generics.to_vec(),
                    ctor,
                    vec![],
                    args.to_vec(),
                    expected,
                    sp,
                ));
            }
            if d.kind == crate::check::DataKind::Class {
                self.ctx.err(sp, format!(
                    "`{}` constructs through its class methods (`{0}.new(..)`) — mark one `[constructor]` to call the class itself",
                    self.ctx.name(name)
                ));
            } else {
                self.ctx.err(sp, format!(
                    "construction is a method call, never a type-call — use a struct literal `{} {{ .. }}`",
                    self.ctx.name(name)
                ));
            }
            return Some(Err(()));
        }
        if let Some(e) = self.ctx.enum_in(&m, name) {
            if let Some(msg) = vis_gate(&self.ctx, name, e.vis) {
                self.ctx.err(sp, msg);
                return Some(Err(()));
            }
            self.ctx.err(sp, format!(
                "`{}` is an enum — construct a member: `{}.Member`",
                self.ctx.name(name),
                self.ctx.name(name)
            ));
            return Some(Err(()));
        }
        // a module let is a constant, not a callable
        if self.ctx.let_in(&m, name).is_some() {
            if let Some((_, _, _, lvis)) = self.ctx.let_in(&m, name) {
                if let Some(msg) = vis_gate(&self.ctx, name, lvis) {
                    self.ctx.err(sp, msg);
                    return Some(Err(()));
                }
            }
            self.ctx.err(
                sp,
                format!("`{}` is a constant — it takes no arguments", self.ctx.name(name)),
            );
            return Some(Err(()));
        }
        None
    }


    /// Implicit widening — exact > interface-typed when the concrete
    /// type SATISFIES the interface structurally (the member-set check
    /// at the boundary: every member present, signature compatible).
    /// Checking, never search; the proven pair records the itable fill
    /// demand. Same-type always widens.
    pub(crate) fn widens(&mut self, from: TypeId, to: TypeId) -> bool {
        if from == to {
            return true;
        }
        // mirrors of one owner-anchored instantiation unify at link —
        // the checker treats the keys as the identity they compile to
        if self.ctx.same_instantiation(from, to) {
            return true;
        }
        if let TyKind::IfaceObj { iface_id } = self.ctx.types.kind(to).clone() {
            match self.ctx.check_satisfies(from, iface_id) {
                Ok(()) => {
                    self.ctx.demand_iface_fill(from, iface_id);
                    return true;
                }
                Err(detail) => {
                    let tname = self.ctx.iface_base_name(iface_id);
                    self.ctx.err(rut_lexer::span::Span::new(self.span, self.span), format!(
                        "`{}` does not satisfy `{}`: {}",
                        self.ctx.type_name(from),
                        tname,
                        detail
                    ));
                    return false;
                }
            }
        }
        false
    }

    /// The value-position widening: [`Self::widens`] plus the
    /// BOXED-TO-BOXED pass. A trait-typed VALUE (`width: Readable<f64>`,
    /// already boxed at its binding) flowing into a DIFFERENT trait's
    /// slot (`set<T>(s: Writable<T>, ..)`) finds no inherent members on
    /// the box — the plain check refuses — but the value is not
    /// anonymous: origin counting pinned its concrete origin at the
    /// boxing site. The pass re-binds through that origin: satisfaction
    /// re-checks there (the write-side law answers with a span — an
    /// origin whose member set lacks the target's members refuses), and
    /// the itable fill demands at the ORIGIN's row — the row the
    /// payload cell carries at runtime, exactly what the callee's
    /// dynamic dispatch reads. This is the fill/dispatch identity law:
    /// a fill recorded against one row (the box) while the dispatch
    /// reads another (the payload) is the trap shape — the demand must
    /// land on the dispatch's row. A value with no tracked origin (a
    /// merge of boxes, an untracked arrival) proves nothing: the honest
    /// structural refusal. `node` names the value expression; the bare
    /// path carries its binding's origins.
    pub(crate) fn widens_val(&mut self, node: NodeHandle<AnyExpr>, from: TypeId, to: TypeId) -> bool {
        if from == to {
            return true;
        }
        if self.ctx.same_instantiation(from, to) {
            return true;
        }
        if matches!(self.ctx.types.kind(from), TyKind::IfaceObj { .. }) {
            if matches!(self.ctx.types.kind(to), TyKind::IfaceObj { .. }) {
                if let TyKind::IfaceObj { iface_id: from_if } = self.ctx.types.kind(from).clone() {
                    if let TyKind::IfaceObj { iface_id: to_if } = self.ctx.types.kind(to).clone() {
                        if from_if != to_if {
                            return self.widen_boxed(node, from, to_if);
                        }
                        // same base, different rows: the member sets are
                        // spelled per instantiation — the plain check's
                        // structural answer
                    }
                }
            } else {
                // the origin-directed unbox: the value's tracked origin
                // names the concrete target exactly — the slot already
                // carries that row's cell (a box is representational),
                // so the static bind through the origin answers. A
                // merged origin set pins no single row: refuse.
                let origins = self.origins_of_expr(node);
                if origins.len() == 1 && origins[0] == to {
                    return true;
                }
            }
        }
        self.widens(from, to)
    }

    /// The boxed pass behind [`Self::widens_val`].
    fn widen_boxed(&mut self, node: NodeHandle<AnyExpr>, from: TypeId, to_if: u32) -> bool {
        let origins = self.origins_of_expr(node);
        let sp = rut_lexer::span::Span::new(self.span, self.span);
        if origins.is_empty() {
            self.ctx.err(sp, format!(
                "`{}` does not satisfy `{}`: no member set is visible through the box — the value's concrete origin is untracked (the origin law)",
                self.ctx.type_name(from),
                self.ctx.iface_base_name(to_if),
            ));
            return false;
        }
        let mut first_err: Option<String> = None;
        for &origin in &origins {
            if let Err(detail) = self.ctx.check_satisfies(origin, to_if) {
                if first_err.is_none() {
                    first_err = Some(format!(
                        "`{}` does not satisfy `{}`: {}",
                        self.ctx.type_name(origin),
                        self.ctx.iface_base_name(to_if),
                        detail,
                    ));
                }
            }
        }
        if let Some(msg) = first_err {
            self.ctx.err(sp, msg);
            return false;
        }
        for &origin in &origins {
            self.ctx.demand_iface_fill(origin, to_if);
        }
        true
    }

    /// The origin set of a VALUE EXPRESSION (origin counting): a bare
    /// path carries its binding's origins; anything else is untracked.
    fn origins_of_expr(&self, node: NodeHandle<AnyExpr>) -> Vec<TypeId> {
        match self.ctx.ast.expr(node) {
            ExprKind::Path { segs } if segs.len() == 1 => self.origins_of(segs[0].name),
            _ => Vec::new(),
        }
    }

    /// Identity-sensitive type equality for the assignment checks: the
    /// ordinary id compare, plus the owner-anchored mirrors (distinct
    /// ids pre-link, one row at link).
    pub(crate) fn same_ty(&self, a: TypeId, b: TypeId) -> bool {
        self.ctx.same_instantiation(a, b)
    }

    pub(crate) fn check_recv_mut(&mut self, recv: NodeHandle<AnyExpr>, sp: rut_lexer::span::Span, what: &str) -> bool {
        match self.ctx.ast.expr(recv).clone() {
            ExprKind::Path { segs } if segs.len() == 1 => {
                if let Some(l) = self.lookup(segs[0].name) {
                    // the mut-binding law is uniform under the by-reference
                    // regime: every binding shares its cell, so
                    // ANY store through the binding — including through a
                    // `?T` head that derefs first — requires `let mut`
                    // (the old pointer exception is gone)
                    if !l.is_mut && !l.loop_var {
                        self.ctx.err(sp, format!(
                            "{what} requires a `let mut` binding (the mut-binding law)"
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
