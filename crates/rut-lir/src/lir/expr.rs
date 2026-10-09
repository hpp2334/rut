//! Expression compilation: the value-in-last_reg convention, literals with
//! bidirectional inference, path/field-chain reads,
//! enum members, and module-let loads.

use crate::check::{float_suffix_ty, int_suffix_ty, numeric_prim, TcResult};
use rut_core::binary::ConstVal;
use rut_core::ops::*;
use rut_core::sym;
use rut_core::types::*;
use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {
    // ---- expressions ----

    /// Compile an expression; the value lands in a fresh register (the
    /// last register). `expected` flows down for bidirectional literal
    /// inference.
    pub(crate) fn compile_expr(&mut self, node: NodeHandle<AnyExpr>, expected: Option<TypeId>) -> TcResult<TypeId> {
        if !self.enter() {
            self.leave();
            return Err(());
        }
        let mut r = self.compile_expr_inner(node, expected);
        self.leave();
        // nullable coercion: the mirror pair at value positions —
        // a `?T` where `T` is expected reads its payload (field 0, the old
        // `*p` deref), a `T` where `?T` is expected boxes into a fresh
        // one-slot cell (the old `&v`). The pair is TRANSITIVE: nesting is
        // a real type (`??T` is spellable, and a `Vec<?T>`'s backing slot
        // is `??T`), so the funnel steps until the expected type is met.
        // Arguments, returns, lets, assignments all flow through here.
        // Positions without an expected type (receivers, `==` operands,
        // `let w = p`) keep the nullable: identity, aliasing and field/
        // method deref are native.
        if let Ok(mut t) = r {
            if let Some(e) = expected {
                if e != t {
                    let lo = self.ctx.ast.span(node.id()).lo;
                    while t != e {
                        let step = match (self.ctx.types.kind(t).clone(), self.ctx.types.kind(e).clone()) {
                            (TyKind::Opt { elem }, _) if elem == e => {
                                let src = self.last_reg;
                                let d = self.new_reg(elem);
                                self.emit(Op::GetF { dst: d, obj: src, field: 0, repr: self.ctx.types.repr_of(elem) }, lo);
                                Some(elem)
                            }
                            (_, TyKind::Opt { elem }) if elem == t => {
                                let src = self.last_reg;
                                let dst = self.new_reg(e);
                                self.emit(Op::MakeOpt { dst, src, ty: e }, lo);
                                Some(e)
                            }
                            _ => None,
                        };
                        match step {
                            Some(nt) => t = nt,
                            None => break,
                        }
                    }
                    r = Ok(t);
                }
            }
        }
        // copy-by-value boundary: every VALUE-typed
        // expression result is a fresh cell the consumer owns — records and
        // arrays deep-copy here, once, at the expression boundary. Receivers
        // and assignment-target chains bypass this wrapper and alias.
        r
    }

    pub(crate) fn compile_expr_inner(&mut self, node: NodeHandle<AnyExpr>, expected: Option<TypeId>) -> TcResult<TypeId> {
        let sp = self.ctx.ast.span(node.id());
        self.span = sp.lo;
        match self.ctx.ast.expr(node).clone() {
            ExprKind::Lit(lit) => {
                let (ty, reg) = self.load_lit(lit, expected, sp)?;
                let _ = reg;
                Ok(ty)
            }
            ExprKind::Path { segs } => self.compile_path(node, segs, expected, sp),
            ExprKind::Call { callee, args } => self.compile_call(node, callee, args, expected, sp),
            ExprKind::Method { recv, name, generics, args } => {
                self.compile_method(recv, name, generics, args, expected, sp)
            }
            ExprKind::Field { recv, name } => self.compile_field(recv, name, sp),
            ExprKind::Tuple { elems } => {
                // `(a, b, ..)`: a record with numeric fields.
                // `()` no longer reaches here — the parser rejects it and
                // points at `nil` (v1.2).
                //
                // The expected type flows in FIELD-BY-FIELD (err-channel
                // phase 3): a tuple literal in a typed position (`return
                // (nil, "")` under `-> (?T, str)`) hints each element with
                // its field's type, so the per-element coercion funnel
                // boxes `T → ?T` and types `nil` AS the nullable (the
                // null slot) — the same field-by-field shape the crossing
                // rule already speaks.
                let hints: Option<Vec<TypeId>> = expected.and_then(|e| {
                    match self.ctx.types.kind(e).clone() {
                        TyKind::Data { fields } if fields.len() == elems.len() => {
                            Some(fields.iter().map(|f| f.ty).collect())
                        }
                        _ => None,
                    }
                });
                let mut etys = Vec::new();
                let mut vals = Vec::new();
                for (i, e) in elems.iter().enumerate() {
                    let t = self.compile_expr(*e, hints.as_ref().map(|h| h[i]))?;
                    etys.push(t);
                    vals.push(self.last_reg);
                }
                let ty = self.ctx.mk_tuple(etys);
                let dst = self.new_reg(ty);
                { let (argv_off, argc) = self.pool_args(&(vals)); self.emit(Op::MakeRecord { dst: dst, ty: ty, argv_off, argc }, sp.lo); }
                Ok(ty)
            }
            ExprKind::Index { recv, idx } => {
                let rt = self.compile_expr(recv, None)?;
                let rreg = self.last_reg;
                // `p[i]` auto-derefs
                let (rt, rreg) = self.deref_for_use(rt, rreg, sp.lo);
                let it = self.compile_expr(idx, Some(TY_I32))?;
                if it != TY_I32 {
                    self.ctx.err(sp, format!("index must be `i32`, found `{}`", self.ctx.type_name(it)));
                }
                let ireg = self.last_reg;
                if let Some(info) = self.slice_info(rt) {
                    self.emit_slice_get(rreg, ireg, &info, sp.lo)?;
                    return Ok(info.elem);
                }
                self.ctx.err(sp, format!("indexing needs a sequence (Vec, Array, or bytes) —found `{}`", self.ctx.type_name(rt)));
                Err(())
            }
            ExprKind::Unary { op, expr } => {
                use rut_ast::ast::UnOp::*;
                let t = match op {
                    // `*p`/`&v` are gone — the parser diagnoses
                    // them; the coercions they performed are implicit at
                    // value positions (the compile_expr wrapper: `?T → T`
                    // reads the payload, `T → ?T` boxes).
                    Not => self.compile_expr(expr, Some(TY_BOOL))?,
                    // `-lit` in a typed position still adapts the literal
                    // (`-2.0` passed to an `f64` param), so forward
                    // `expected` — but see through one nullable layer
                    //: negation computes at the payload type,
                    // the funnel re-boxes the result
                    _ => self.compile_expr(expr, self.numeric_hint(expected))?,
                };
                // capture the operand's register BEFORE new_reg — it
                // overwrites last_reg with the destination
                let src = self.last_reg;
                let dst = self.new_reg(t);
                match op {
                    Neg => {
                        let prim = if let TyKind::Prim(p) = self.ctx.types.kind(t) { Some(*p) } else { None };
                        let Some(prim) = prim else {
                            self.ctx.err(sp, "negation needs a number");
                            return Err(());
                        };
                        self.emit(negop(prim, dst, src), sp.lo);
                    }
                    Not => {
                        if t != TY_BOOL {
                            self.ctx.err(sp, "`!` needs a bool");
                        }
                        self.emit(Op::Not { dst, a: src }, sp.lo);
                    }
                    BitNot => {
                        self.ctx.err(sp, "`~` is not supported in this build");
                        return Err(());
                    }
                }
                Ok(t)
            }
            ExprKind::Binary { op, lhs, rhs } => self.compile_binary(node, op, lhs, rhs, expected, sp),
            ExprKind::Assign { op, target, value } => {
                self.compile_assign(node, op, target, value, sp)?;
                Ok(TY_NIL)
            }
            ExprKind::Lambda { params, ret, body } => {
                self.compile_lambda(node.id(), params, ret, body, expected, sp)
            }
            ExprKind::Try { .. } => {
                // `?` was Result-syntax: removed with the sums
                self.ctx.err(sp, "`?` was removed — errors are `(T, err)` records; test the second element");
                Err(())
            }
            ExprKind::Await { expr } => {
                // the landing: the await expansion runs inside
                // an async body — elsewhere it diagnoses
                crate::lir::asyncfn::compile_await(self, expr, sp)
            }
            ExprKind::AsyncBlock { body } => {
                // the async primitive (v20): mint the frame, answer
                // `Future<T>`. Legal everywhere — minting is pure; only
                // `await` needs an async body.
                crate::lir::asyncfn::compile_async_block(self, body, expected, sp)
            }
            ExprKind::FStr { parts } => self.compile_fstr(parts, expected, sp),
            ExprKind::Struct { ty, fields } => self.compile_struct(ty, fields, expected, sp),
            ExprKind::ArrayLit { elems } => {
                let arr_ty = self.compile_array_lit(elems, expected, sp)?;
                Ok(arr_ty)
            }
            ExprKind::ArrayRepeat { value, count } => {
                self.compile_array_repeat(value, count, expected, sp)
            }
            ExprKind::WhenExpr { scrut, arms } => self.compile_when(scrut, &arms, expected, sp),
            ExprKind::Is { expr, ty } => {
                let rt = self.compile_expr(expr, None)?;
                let recv = self.last_reg;
                let dst = self.new_reg(TY_BOOL);
                // resolve the RHS (a naming position) — a single-segment
                // name, generic arguments allowed (an instantiated record
                // names itself: `v is Vec<i64>`)
                if let TypeKind::TyPath { segs } = self.ctx.ast.ty(ty) {
                    if segs.len() == 1 {
                        let n = self.ctx.name(segs[0].name).to_string();
                        // trait RHS —capability probe (or fold). A generic
                        // head (`Vec<i64>`) is never a trait spelling.
                        if segs[0].generics.is_empty() {
                            if let Some(tid) = self.ctx.iface_id_of(segs[0].name) {
                                match self.ctx.types.kind(rt).clone() {
                                    TyKind::IfaceObj { iface_id } if iface_id == tid => {
                                        self.emit(Op::ConstRaw { dst, bits: 1 }, sp.lo); // fold
                                    }
                                    TyKind::IfaceObj { .. } => {
                                        self.emit(Op::IsIface { dst, obj: recv, want: tid }, sp.lo);
                                    }
                                    TyKind::Opaque => {
                                        self.emit(Op::IsIface { dst, obj: recv, want: tid }, sp.lo);
                                    }
                                    exact => {
                                        let _ = exact;
                                        // exact receiver: fold — the type
                                        // satisfies the member set or it does
                                        // not (structural, compile-time)
                                        let has = self.ctx.check_satisfies(rt, tid).is_ok();
                                        self.emit(Op::ConstRaw { dst, bits: has as u64 }, sp.lo);
                                    }
                                }
                                return Ok(TY_BOOL);
                            }
                            // Opaque RHS — ambient like the primitive
                            // revised, builtin-surface phase 2): the surface
                            // spelling IS the boot name (`sym::OPAQUE`)
                            if n == "opaque"
                                && self.ctx.extern_native_types.get(&segs[0].name).copied()
                                    == Some(rut_core::binary::NativeTy::Opaque)
                            {
                                match self.ctx.types.kind(rt).clone() {
                                    TyKind::Opaque => {
                                        self.emit(Op::ConstRaw { dst, bits: 1 }, sp.lo);
                                    }
                                    _ => {
                                        self.emit(Op::ConstRaw { dst, bits: 0 }, sp.lo);
                                    }
                                }
                                return Ok(TY_BOOL);
                            }
                        }
                        // concrete RHS — an instantiated record names
                        // itself (`v is Vec<i64>` resolves through the
                        // same instantiation the spelling mints)
                        let want = self.resolve_type_now(ty);
                        if !self.ctx.types.is_ref(want) && want != TY_STR {
                            self.ctx.err(sp, format!(
                                "`is` needs a concrete (cell) or interface type —`{}` is by-value",
                                self.ctx.type_name(want)
                            ));
                        }
                        match self.ctx.types.kind(rt).clone() {
                            TyKind::Opaque => {
                                self.emit(Op::IsType { dst, obj: recv, want }, sp.lo);
                            }
                            _ => {
                                // exact receiver: fold;
                                // mirrors of one instantiation fold too
                                let same = self.same_ty(rt, want);
                                self.emit(Op::ConstRaw { dst, bits: same as u64 }, sp.lo);
                            }
                        }
                        return Ok(TY_BOOL);
                    }
                }
                self.ctx.err(sp, "the `is` right-hand side must be a type name");
                Err(())
            }
            ExprKind::Cast { expr, ty } => {
                // `expr as T`: the numeric cast — truncating,
                // C/Rust semantics, one `Op::Conv`. The RHS is a naming
                // position restricted to the numeric primitives.
                let mut from = self.compile_expr(expr, None)?;
                let mut src = self.last_reg;
                // a `?T` operand reads its payload first (the
                // old `*x` deref, now implicit at this value position)
                if let TyKind::Opt { elem } = self.ctx.types.kind(from).clone() {
                    if let TyKind::Prim(p) = self.ctx.types.kind(elem) {
                        if numeric_prim(*p) {
                            let d = self.new_reg(elem);
                            self.emit(
                                Op::GetF { dst: d, obj: src, field: 0, repr: self.ctx.types.repr_of(elem) },
                                sp.lo,
                            );
                            src = d;
                            from = elem;
                        }
                    }
                }
                let want = self.resolve_type_now(ty);
                let Some(to) = (if let TyKind::Prim(p) = self.ctx.types.kind(want) {
                    numeric_prim(*p).then_some(*p)
                } else {
                    None
                }) else {
                    self.ctx.err(sp, format!(
                        "`as` converts between numeric primitives —`{}` is not one",
                        self.ctx.type_name(want)
                    ));
                    return Err(());
                };
                let TyKind::Prim(from_prim) = self.ctx.types.kind(from).clone() else {
                    self.ctx.err(sp, format!(
                        "`as` converts between numeric primitives —found `{}`",
                        self.ctx.type_name(from)
                    ));
                    return Err(());
                };
                let dst = self.new_reg(want);
                self.emit(Op::Conv { dst, src, from: from_prim, to }, sp.lo);
                Ok(want)
            }
            _ => {
                self.ctx.err(sp, "unsupported expression in this build");
                Err(())
            }
        }
    }

    // last register allocated (the value produced by compile_expr)
    // NOTE: maintained by every producing helper via self.new_reg

    /// Bidirectional inference sees through one nullable
    /// layer: in an expected `?T` position an unsuffixed numeric literal
    /// adapts to `T`, and the funnel's `T → ?T` coercion boxes it.
    fn numeric_hint(&self, expected: Option<TypeId>) -> Option<TypeId> {
        match expected {
            Some(e) => match self.ctx.types.kind(e).clone() {
                TyKind::Opt { elem } if matches!(self.ctx.types.kind(elem), TyKind::Prim(_)) => Some(elem),
                _ => Some(e),
            },
            None => None,
        }
    }

    pub(crate) fn load_lit(&mut self, lit: Lit, expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<(TypeId, u16)> {
        match lit {
            Lit::Int(v, sfx) => {
                let ty = match sfx {
                    Some(s) => int_suffix_ty(s),
                    None => {
                        // an unsuffixed literal defaults to
                        // `i32` and adapts bidirectionally — but only while
                        // it FITS that default. Past it the literal must
                        // declare itself: `Vec<u64>.from([..])` no longer
                        // silently absorbs a 20-digit literal, so a dropped
                        // or doubled digit can't masquerade as a value.
                        let over_default = v > i32::MAX as u64;
                        if over_default {
                            let hint = match expected.map(|e| self.ctx.types.kind(e).clone()) {
                                Some(TyKind::Prim(p)) if p.is_float() => {
                                    format!(" —in a float position write `{v}.0`")
                                }
                                _ => String::new(),
                            };
                            self.ctx.err(sp, format!(
                                "integer literal {v} exceeds the `i32` default —add an explicit suffix like `u64`{hint}"
                            ));
                        }
                        match self.numeric_hint(expected) {
                            // bidirectional inference: an int
                            // literal adapts to the expected numeric type — int
                            // widths AND float positions (`x: f32 = 1`)
                            Some(e) if matches!(self.ctx.types.kind(e), TyKind::Prim(p) if p.is_int()) => e,
                            Some(e) if matches!(self.ctx.types.kind(e), TyKind::Prim(p) if p.is_float()) => {
                                let f = v as f64;
                                let reg = self.new_reg(e);
                                if e == TY_F32 {
                                    self.emit(Op::ConstRaw { dst: reg, bits: ((f as f32) as f64).to_bits() }, sp.lo);
                                } else {
                                    self.emit(Op::ConstRaw { dst: reg, bits: f.to_bits() }, sp.lo);
                                }
                                return Ok((e, reg));
                            }
                            // past the default without a suffix there is no
                            // honest type left — `u64` is the smallest width
                            // that holds the value and keeps codegen going
                            // for the remaining diagnostics
                            _ if over_default => TY_U64,
                            _ => TY_I32,
                        }
                    }
                };
                // range check
                if let TyKind::Prim(p) = self.ctx.types.kind(ty) {
                    let (min, max): (i128, i128) = match p {
                        PrimTy::U8 => (0, u8::MAX as i128),
                        PrimTy::U16 => (0, u16::MAX as i128),
                        PrimTy::U32 => (0, u32::MAX as i128),
                        PrimTy::U64 => (0, u64::MAX as i128),
                        PrimTy::I8 => (i8::MIN as i128, i8::MAX as i128),
                        PrimTy::I16 => (i16::MIN as i128, i16::MAX as i128),
                        PrimTy::I32 => (i32::MIN as i128, i32::MAX as i128),
                        PrimTy::I64 => (i64::MIN as i128, i64::MAX as i128),
                        _ => (i64::MIN as i128, i64::MAX as i128),
                    };
                    if (v as i128) < min || (v as i128) > max {
                        self.ctx.err(sp, format!(
                            "integer literal {v} does not fit `{}`",
                            self.ctx.type_name(ty)
                        ));
                    }
                }
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: v }, sp.lo);
                Ok((ty, reg))
            }
            Lit::Float(bits, sfx) => {
                // the same law as the ints above, for the `f32` default: an
                // unsuffixed literal whose magnitude only exists in f64
                // must say so (precision is not the trigger — range is)
                if sfx.is_none() {
                    let x = f64::from_bits(bits);
                    if !x.is_finite() || x.abs() > f32::MAX as f64 {
                        self.ctx.err(sp, format!(
                            "float literal {:e} exceeds the `f32` default —add an explicit `f64` suffix",
                            x
                        ));
                    }
                }
                let ty = match sfx {
                    Some(s) => float_suffix_ty(s),
                    None => match self.numeric_hint(expected) {
                        Some(e) if matches!(self.ctx.types.kind(e), TyKind::Prim(p) if p.is_float()) => e,
                        _ => TY_F32,
                    },
                };
                // The lexer stores literals as f64 bits. Narrow that payload
                // to the register's width so an `f32` literal actually carries
                // f32 precision in its slot — arithmetic and `str()` re-narrow,
                // but float comparisons read the slot back as f64.
                let bits = if ty == TY_F32 {
                    (f64::from_bits(bits) as f32 as f64).to_bits()
                } else {
                    bits
                };
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                Ok((ty, reg))
            }
            Lit::Str(s) | Lit::RawStr(s) => {
                let k = self.konst(ConstVal::Str(s));
                let reg = self.new_reg(TY_STR);
                self.emit(Op::Const { dst: reg, k: k as u32 }, sp.lo);
                Ok((TY_STR, reg))
            }
            Lit::Bool(b) => {
                let reg = self.new_reg(TY_BOOL);
                self.emit(Op::ConstRaw { dst: reg, bits: b as u64 }, sp.lo);
                Ok((TY_BOOL, reg))
            }
            Lit::Nil => {
                // `nil`: typed by the expected position —
                // `let p: *T = nil`, `p == nil` — and otherwise the `nil`
                // type's own value. The null slot IS zero bits (Slot::null).
                let ty = match expected {
                    Some(e) if matches!(self.ctx.types.kind(e), TyKind::Opt { .. }) => e,
                    _ => TY_NIL,
                };
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: 0 }, sp.lo);
                Ok((ty, reg))
            }
        }
    }

    pub(crate) fn compile_path(&mut self, _node: NodeHandle<AnyExpr>, segs: Vec<PathSeg>, expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // single name: local / module let / enum member of... / builtin value
        if segs.len() == 1 && segs[0].generics.is_empty() {
            let name = segs[0].name;
            if let Some(l) = self.lookup(name).cloned() {
                // inlined `Slice` accessor: `self` is the receiver register
                // itself — no copy (sequence access)
                if let Some((sid, reg)) = self.inline_self {
                    if sid == name {
                        self.last_reg = reg;
                        return Ok(l.ty);
                    }
                }
                // the capture law's read accessor: a promoted binding
                // reads its shared cell, an ordinary one copies its
                // register — ONE funnel for every read
                let reg = self.read_local(&l, sp.lo);
                return Ok(l.ty);
            }
            // a builtin fn name in value position — point at the call form
            if (self.ctx.extern_native_fns.contains(&name)
                && name == sym::PANIC)
                || name == sym::TYPE_ID
            {
                self.ctx.err(sp, format!("`{}` is a function —call it: `{}(..)`", self.ctx.name(name), self.ctx.name(name)));
                return Err(());
            }
            // module let (load-time, M1: literals only)
            if let Some((_, ty_node, init, lvis)) = self.ctx.find_let(name).cloned() {
                // a module let of ANOTHER module is not in scope bare
                // (single names resolve in the current module only)
                if self.ctx.mod_of(init.id()) != self.ctx.cur_mod {
                    self.ctx.err(sp, format!(
                        "`{}` is declared in module `{}` — qualify it: `{}::{}`",
                        self.ctx.name(name),
                        self.ctx.mod_of(init.id()),
                        self.ctx.mod_of(init.id()),
                        self.ctx.name(name)
                    ));
                    return Err(());
                }
                let _ = lvis;
                let ty = ty_node.map(|t| self.resolve_type_now(t)).unwrap_or(TY_I32);
                let _reg = self.load_const_let(init, ty, expected, sp)?;
                return Ok(ty);
            }
            // used constant (native modules: `calc::PI`)
            if let Some((ty, bits)) = self.ctx.extern_const(name) {
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                return Ok(ty);
            }
            // a fn name in VALUE position: the bare fn-path lane —
            // the fn's own value (call-position resolution keeps its
            // fast path; only this fallthrough gains the arm)
            if let Some(r) = self.compile_fn_path_value(name, sp) {
                return r;
            }
            let msg = self
                .ctx
                .not_in_core_scope(name)
                .unwrap_or_else(|| format!("unknown name `{}`", self.ctx.name(name)));
            self.ctx.err(sp, msg);
            return Err(());
        }
        // `<namespace>.CONST` — a used namespace's constants
        // (`Math.PI`). Name-generic: routed by the bound
        // namespace head, never by a hardcoded string.
        if segs.len() == 2 && self.ctx.is_extern_namespace(segs[0].name) {
            if let Some((ty, bits)) = self.ctx.extern_const(segs[1].name) {
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                return Ok(ty);
            }
            let ns = self.ctx.name(segs[0].name).to_string();
            let m = self.ctx.name(segs[1].name).to_string();
            self.ctx.err(sp, format!("`{ns}.{m}` is not a namespace constant"));
            return Err(());
        }
        // two segments: local field chain `p.x`, `req.value.reply` — locals
        // shadow type names (a local is a value position)
        if segs.len() >= 2 && segs[0].generics.is_empty() {
            if let Some(l) = self.lookup(segs[0].name).cloned() {
                // the chain head: a promoted binding loads its shared
                // cell's value first (the capture law's accessor); an
                // ordinary one chains off its register raw, as before
                let mut cur = if l.cell.is_some() { self.read_local(&l, sp.lo) } else { l.reg };
                let mut cur_ty = l.ty;
                for seg in &segs[1..] {
                    if !seg.generics.is_empty() {
                        self.ctx.err(sp, "generic arguments are not valid on a field");
                        return Err(());
                    }
                    // `p.x` auto-deref through a nullable: the
                    // payload slot at the payload's own repr, transitively
                    // for `??T`
                    let (dty, dreg) = self.deref_for_use(cur_ty, cur, sp.lo);
                    cur = dreg;
                    cur_ty = dty;
                    let TyKind::Data { fields } = self.ctx.types.kind(cur_ty).clone() else {
                        self.ctx.err(sp, format!(
                            "`{}` has no field `{}` — `{}` is not a record",
                            self.ctx.type_name(cur_ty), self.ctx.name(seg.name), self.ctx.type_name(cur_ty)
                        ));
                        return Err(());
                    };
                    let Some(fidx) = fields.iter().position(|f| f.name == seg.name) else {
                        self.ctx.err(sp, format!(
                            "`{}` has no field `{}`",
                            self.ctx.type_name(cur_ty), self.ctx.name(seg.name)
                        ));
                        return Err(());
                    };
                    let fty = fields[fidx].ty;
                    let dst = self.new_reg(fty);
                    self.emit(Op::GetF { dst, obj: cur, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                    cur = dst;
                    cur_ty = fty;
                }
                return Ok(cur_ty);
            }
        }
        // two segments: Enum.Member value
        if segs.len() == 2 {
            let base = segs[0].name;
            let member = segs[1].name;
            if !segs[0].generics.is_empty() || !segs[1].generics.is_empty() {
                self.ctx.err(sp, "generic paths are not supported in this build");
                return Err(());
            }
            // a LOCAL enum (the decl's member list) or a USED enum (the
            // carried descriptor row — the member values ride it)
            let enum_hit: Option<(TypeId, Vec<(IdentId, i64)>)> =
                if let Some(e) = self.ctx.enum_here(base) {
                    Some((e.ty, e.members.iter().map(|m| (*m, 0)).collect()))
                } else if self.ctx.use_bound_here(base) {
                    self.ctx.extern_enum(base)
                } else {
                    None
                };
            if let Some((ty, members)) = enum_hit {
                if let Some(i) = members.iter().position(|(m, _)| *m == member) {
                    let reg = self.new_reg(ty);
                    self.emit(Op::EnumNew { dst: reg, ty, member: i as u32 }, sp.lo);
                    return Ok(ty);
                }
                self.ctx.err(sp, format!("`{}` is not a member of enum {}", self.ctx.name(member), self.ctx.name(base)));
                return Err(());
            }
        }
        // qualified value path (`mod.CONST`, `mod.Enum.Member`) —
        // the mod walk (phase 3); `None` falls through when the head
        // is not a module of the current scope or its ancestors
        if let Some(r) = self.compile_qualified_value(&segs, expected, sp) {
            return r;
        }
        // a used package's head: uses are the only cross-package door
        if self.ctx.used_pkgs.contains(self.ctx.name(segs[0].name)) {
            self.package_head_error(&segs, segs[segs.len() - 1].name, sp);
            return Err(());
        }
        self.ctx.err(
            sp,
            format!(
                "unknown name `{}`",
                segs.iter().map(|s| self.ctx.name(s.name).to_string()).collect::<Vec<_>>().join(".")
            ),
        );
        Err(())
    }

    /// The qualified VALUE path (`layout.LIMIT`, `mod.Color.Red`):
    /// the head names a child `mod` of the current module or of an
    /// ancestor; further segments walk child mods until one names an
    /// ENUM of the walked module (the trailing `Enum.Member` pair), or
    /// the leaf resolves among that module's decls (a fn value, a
    /// module let, a type — which is no value). Gated by the crossing
    /// predicate. `None` = the head is not a module — the caller's
    /// ladder carries on (namespace constants, plain unknown-name).
    pub(crate) fn compile_qualified_value(
        &mut self,
        segs: &[PathSeg],
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> Option<TcResult<TypeId>> {
        if segs.len() < 2 || !segs[0].generics.is_empty() {
            return None;
        }
        let from = self.ctx.cur_mod.clone();
        let Some(mut m) = self.ctx.resolve_mod_head(&from, segs[0].name) else {
            return None; // the caller's pkg-head check diagnoses
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
            // a non-mod segment: the trailing `Enum.Member` pair, or
            // the leaf
            if i != segs.len() - 1 {
                if i == segs.len() - 2 {
                    let member = segs[segs.len() - 1].name;
                    if let Some(e) = self.ctx.enum_in(&m, seg.name) {
                        if !self.ctx.vis_crosses(&from, &m, e.vis) {
                            self.ctx.err(
                                sp,
                                format!(
                                    "{m}.{} — {}",
                                    self.ctx.name(seg.name),
                                    self.ctx.vis_hint(seg.name, &from, &m, e.vis)
                                ),
                            );
                            return Some(Err(()));
                        }
                        let members: Vec<IdentId> = e.members.clone();
                        if let Some(idx) = members.iter().position(|&mm| mm == member) {
                            let _ = idx;
                            let reg = self.new_reg(e.ty);
                            let mi = members.iter().position(|&mm| mm == member).unwrap();
                            self.emit(Op::EnumNew { dst: reg, ty: e.ty, member: mi as u32 }, sp.lo);
                            return Some(Ok(e.ty));
                        }
                        self.ctx.err(sp, format!(
                            "`{}` is not a member of enum {}",
                            self.ctx.name(member),
                            self.ctx.name(seg.name)
                        ));
                        return Some(Err(()));
                    }
                }
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
            // the leaf: a fn value, a module let, or a type (not a value)
            let name = seg.name;
            if !seg.generics.is_empty() {
                self.ctx.err(sp, "generic arguments are not valid in a value path");
                return Some(Err(()));
            }
            if let Some(fnode) = self.ctx.fn_in(&m, name) {
                if !self.ctx.vis_crosses(&from, &m, self.ctx.ast.fn_decl(fnode).vis) {
                    self.ctx.err(
                        sp,
                        format!(
                            "{m}.{} — {}",
                            self.ctx.name(name),
                            self.ctx.vis_hint(name, &from, &m, self.ctx.ast.fn_decl(fnode).vis)
                        ),
                    );
                    return Some(Err(()));
                }
                // names are package-unique: the fn-value lane resolves
                // the same fn the bare spelling would in its own module
                return Some(
                    self.compile_fn_path_value(name, sp)
                        .expect("the module's fn resolves in the fn-value lane"),
                );
            }
            if let Some((_, ty_node, init, lvis)) = self.ctx.let_in(&m, name) {
                if !self.ctx.vis_crosses(&from, &m, lvis) {
                    self.ctx.err(
                        sp,
                        format!(
                            "{m}.{} — {}",
                            self.ctx.name(name),
                            self.ctx.vis_hint(name, &from, &m, lvis)
                        ),
                    );
                    return Some(Err(()));
                }
                let ty = ty_node.map(|t| self.resolve_type_now(t)).unwrap_or(TY_I32);
                match self.load_const_let(init, ty, expected, sp) {
                    Ok(_) => return Some(Ok(ty)),
                    Err(()) => return Some(Err(())),
                }
            }
            if let Some(e) = self.ctx.enum_in(&m, name) {
                if !self.ctx.vis_crosses(&from, &m, e.vis) {
                    self.ctx.err(
                        sp,
                        format!(
                            "{m}.{} — {}",
                            self.ctx.name(name),
                            self.ctx.vis_hint(name, &from, &m, e.vis)
                        ),
                    );
                    return Some(Err(()));
                }
                self.ctx.err(sp, format!(
                    "`{}` is an enum — construct a member: `{}.Member`",
                    self.ctx.name(name),
                    self.ctx.name(name)
                ));
                return Some(Err(()));
            }
            if self.ctx.data_in(&m, name).is_some() {
                self.ctx.err(sp, format!(
                    "`{}` is a type — construct it (a literal, a class method, a newtype call), never name it bare",
                    self.ctx.name(name)
                ));
                return Some(Err(()));
            }
            self.ctx.err(sp, format!(
                "`{}` is not declared in module `{m}` ({})",
                self.ctx.name(name),
                crate::check::ModInputs::display(&m),
            ));
            return Some(Err(()));
        }
        // the receiver was ALL mods — a module is not a value
        self.ctx.err(
            sp,
            format!("`{}` is a module — modules are not values", self.ctx.name(segs[segs.len() - 1].name)),
        );
        Some(Err(()))
    }

    // ---- the bare fn-path lane ----

    /// A fn NAME in value position resolves to the fn's value — the
    /// closure shape the fn-typed indirect-call lane already consumes.
    /// Resolution mirrors the call path: the enclosing unit's fns, then
    /// imported fns; `None` falls through to the ladder's unknown-name
    /// diagnostic (the name is not a fn).
    pub(crate) fn compile_fn_path_value(
        &mut self,
        name: IdentId,
        sp: rut_lexer::span::Span,
    ) -> Option<TcResult<TypeId>> {
        // the enclosing unit's fn (compile_free_fn_call's own lookup) —
        // single names resolve in the CURRENT module (phase 3; flat
        // packages gate identically: every fn is the root's)
        if self.ctx.fn_is_here(name) {
            let fnode = self.ctx.fn_nodes.iter().find(|(n, _)| *n == name).map(|(_, n)| *n).unwrap();
            let fd = self.ctx.ast.fn_decl(fnode).clone();
            if fd.is_async {
                self.ctx.err(sp, format!(
                    "`{}` is async —call it; async fns have no value in this build",
                    self.ctx.name(name)
                ));
                return Some(Err(()));
            }
            if !fd.generics.is_empty() {
                self.ctx.err(sp, format!(
                    "`{}` is generic —call it (spelling the type arguments there); a bare fn path has none to give",
                    self.ctx.name(name)
                ));
                return Some(Err(()));
            }
            // the fn's own signature — a non-generic fn spells no free
            // generics, so this env resolves it exactly
            let ptys: Vec<TypeId> = fd
                .params
                .iter()
                .map(|p| match self.ctx.ast.param(*p) {
                    MemberKind::Param(ParamData { ty: Some(t), .. }) => self.resolve_type_now(*t),
                    _ => TY_I32,
                })
                .collect();
            let ret_ty = fd.ret.map(|r| self.resolve_type_now(r)).unwrap_or(TY_NIL);
            // the SAME instantiation key the direct-call lane mints —
            // the value and every direct call share one compiled body
            let inst = crate::check::Inst {
                key: crate::check::FnKey::Free(name),
                subst: vec![],
                iface_origins: vec![],
            };
            let fid = self.ctx.ensure_inst(inst);
            return Some(self.make_fn_value(fid, ptys, ret_ty, sp));
        }
        // an imported fn (`use pkg::name`): the value binds the
        // exporter's scope-qualified fn — the same id the direct-call
        // lane emits; the link relocates closure targets like call ones
        if let Some(ef) = self.ctx.extern_fn(name).cloned() {
            if ef.is_async {
                self.ctx.err(sp, format!(
                    "`{}` is async —call it; async fns have no value in this build",
                    self.ctx.name(name)
                ));
                return Some(Err(()));
            }
            return Some(self.make_fn_value(ef.func, ef.params, ef.ret, sp));
        }
        // a used GENERIC fn has no monomorphic value — diagnose at the
        // use site instead of the fallthrough's unknown-name
        if self.ctx.extern_generic_fn(name).is_some() {
            self.ctx.err(sp, format!(
                "`{}` is generic —call it; generic fns have no value without type arguments",
                self.ctx.name(name)
            ));
            return Some(Err(()));
        }
        None
    }

    /// The fn value itself: an empty-capture closure over `fid`, typed
    /// `fn(params) -> ret` — minted once per USE (never per call), the
    /// exact register shape a fn-typed param, local or field carries.
    fn make_fn_value(
        &mut self,
        fid: u32,
        ptys: Vec<TypeId>,
        ret: TypeId,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let fty = self.ctx.mk_fn_ty(ptys, ret);
        let dst = self.new_reg(fty);
        { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::MakeClosure { dst: dst, func: fid, argv_off, argc }, sp.lo,); }
        Ok(fty)
    }

    pub(crate) fn load_const_let(&mut self, init: NodeHandle<AnyExpr>, ty: TypeId, _expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<u16> {
        match self.ctx.ast.expr(init).clone() {
            ExprKind::Lit(Lit::Int(v, sfx)) => {
                // the same default-width law as `load_lit` — module scope
                // gets no free pass
                if sfx.is_none() && v > i32::MAX as u64 {
                    self.ctx.err(sp, format!(
                        "integer literal {v} exceeds the `i32` default —add an explicit suffix like `u64`"
                    ));
                }
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: v }, sp.lo);
                Ok(reg)
            }
            ExprKind::Lit(Lit::Float(b, _)) => {
                // same f32 narrowing as `load_lit`
                let bits = if ty == TY_F32 {
                    (f64::from_bits(b) as f32 as f64).to_bits()
                } else {
                    b
                };
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits }, sp.lo);
                Ok(reg)
            }
            ExprKind::Lit(Lit::Bool(v)) => {
                let reg = self.new_reg(ty);
                self.emit(Op::ConstRaw { dst: reg, bits: v as u64 }, sp.lo);
                Ok(reg)
            }
            ExprKind::Lit(Lit::Str(s)) | ExprKind::Lit(Lit::RawStr(s)) => {
                let k = self.konst(ConstVal::Str(s));
                let reg = self.new_reg(TY_STR);
                self.emit(Op::Const { dst: reg, k: k as u32 }, sp.lo);
                Ok(reg)
            }
            _ => {
                self.ctx.err(sp, "module `let` initializers must be literals in this build");
                Err(())
            }
        }
    }
}
