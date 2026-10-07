//! Operators (equal widths, traps by default; the wrapping
//! &op family) and assignment: the mut-binding law,
//! compound forms, field/index targets.

use crate::check::TcResult;
use rut_core::ops::*;
use rut_core::types::*;
use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    // ---- operators (equal widths; traps by default) ----

    pub(crate) fn compile_binary(
        &mut self,
        _node: NodeHandle<AnyExpr>,
        op: rut_ast::ast::BinOp,
        lhs: NodeHandle<AnyExpr>,
        rhs: NodeHandle<AnyExpr>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        use rut_ast::ast::BinOp::*;
        if matches!(op, And | Or) {
            return self.compile_shortcircuit(op, lhs, rhs, sp);
        }
        // bidirectional unification: an expected type from the
        // enclosing position (return/assignment/argument) seeds the lhs, then
        // rhs adapts to the lhs. Without it, `1.0 / x as f64` and unannotated
        // `4.0 * pi` would default the lhs to f32 and mismatch.
        // `==`/`!=` EXCLUDE the seed: the comparison's result is `bool`, and
        // seeding an operand with it would funnel a `?bool` lhs into its
        // payload before the null-test sees the nullable (`if (b == nil)`,
        // the store's lane probes) — the operands type by the shapes at
        // hand, the pointer-vs-pointee law below decides.
        let seed = if matches!(op, Eq | Ne) { None } else { expected };
        let lt = self.compile_expr(lhs, seed)?;
        let lhs_reg = self.last_reg;
        // arithmetic/ordinal: a scalar-pointee pointer on the left reads
        // its pointee; the right side derefs through the
        // compile_expr funnel. `==`/`!=` keep pointer identity and decide
        // after both sides are typed.
        let (lt, lhs_reg) = if !matches!(op, Eq | Ne) {
            self.deref_ptr(lt, lhs_reg, sp.lo)
        } else {
            (lt, lhs_reg)
        };
        let mut lt = lt;
        let mut lhs_reg = lhs_reg;
        // the rhs hint: an expected `?T` boxes values at let/arg/return/
        // field-store positions, but a `==` operand is NOT such a position
        // (pointer-vs-pointee compares stay). Against a
        // nullable lhs the rhs takes the PAYLEE type (the lhs derefs in
        // the mirror below); only the `nil` literal takes the nullable
        // hint, typing as `?T` for the identity null-slot compare.
        let rhs_hint = match (op, self.ctx.types.kind(lt).clone()) {
            (Eq | Ne, TyKind::Opt { elem }) => {
                if matches!(self.ctx.ast.expr(rhs), ExprKind::Lit(Lit::Nil)) { Some(lt) } else { Some(elem) }
            }
            _ => Some(lt),
        };
        let rt_ = self.compile_expr(rhs, rhs_hint)?;
        let rhs_reg = self.last_reg;
        // `==`/`!=`: a pointer against its own pointee type compares the
        // VALUE (the funnel already deref'd the right-hand side); pointer-
        // vs-pointer — including `nil` — stays identity
        if matches!(op, Eq | Ne) && rt_ != lt {
            if let TyKind::Opt { elem } = self.ctx.types.kind(lt).clone() {
                if rt_ == elem {
                    let (t2, r2) = self.deref_ptr(lt, lhs_reg, sp.lo);
                    lt = t2;
                    lhs_reg = r2;
                }
            }
        }
        if lt != rt_ {
            self.ctx.err(sp, format!(
                "operands must have equal width: `{}` vs `{}` —convert first",
                self.ctx.type_name(lt), self.ctx.type_name(rt_)
            ));
            return Err(());
        }
        let ty = lt;
        match op {
            Eq | Ne => {
                // the `==` law: primitives by value, string and
                // bytes by content, everything else CELL IDENTITY (the raw
                // slot compare — by-reference sharing makes structural
                // equality unobservable); Option/Result is a compile error
                let eq = op == Eq;
                let dst = self.new_reg(TY_BOOL);
                match self.ctx.types.kind(ty).clone() {
                    TyKind::Prim(p) => {
                        let cop = if eq { CmpOp::Eq } else { CmpOp::Ne };
                        self.emit(cmpop(cop, p, dst, lhs_reg, rhs_reg), sp.lo);
                    }
                    TyKind::Str => {
                        self.emit(Op::StrCmp { eq, dst, a: lhs_reg, b: rhs_reg }, sp.lo);
                    }
                    TyKind::Bytes => {
                        self.emit(Op::ArrayCmp { eq, dst, a: lhs_reg, b: rhs_reg }, sp.lo);
                    }
                    _ => {
                        self.emit(Op::RefEq { eq, dst, a: lhs_reg, b: rhs_reg }, sp.lo);
                    }
                }
                Ok(TY_BOOL)
            }
            Lt | Gt | Le | Ge => {
                let prim = if let TyKind::Prim(p) = self.ctx.types.kind(ty) { Some(*p) } else { None };
                let Some(prim) = prim else {
                    self.ctx.err(sp, format!("ordering comparisons need numbers —`{}` has none", self.ctx.type_name(ty)));
                    return Err(());
                };
                let cmp = match op {
                    Lt => CmpOp::Lt,
                    Gt => CmpOp::Gt,
                    Le => CmpOp::Le,
                    _ => CmpOp::Ge,
                };
                let dst = self.new_reg(TY_BOOL);
                self.emit(cmpop(cmp, prim, dst, lhs_reg, rhs_reg), sp.lo);
                Ok(TY_BOOL)
            }
            Add | Sub | Mul | Div | Mod => {
                let prim = if let TyKind::Prim(p) = self.ctx.types.kind(ty) { Some(*p) } else { None };
                let Some(prim) = prim.filter(|p| p.is_int() || p.is_float()) else {
                    self.ctx.err(sp, format!("arithmetic needs numbers —found `{}`", self.ctx.type_name(ty)));
                    return Err(());
                };
                let aop = match op {
                    Add => ArithOp::Add,
                    Sub => ArithOp::Sub,
                    Mul => ArithOp::Mul,
                    Div => ArithOp::Div,
                    _ => ArithOp::Mod,
                };
                let dst = self.new_reg(ty);
                self.emit(arith(aop, prim, dst, lhs_reg, rhs_reg), sp.lo);
                Ok(ty)
            }
            BitAnd | BitOr | BitXor | Shl | Shr => {
                let prim = if let TyKind::Prim(p) = self.ctx.types.kind(ty) { Some(*p) } else { None };
                let Some(prim) = prim.filter(|p| p.is_int()) else {
                    self.ctx.err(sp, format!("bit operations need integers —found `{}`", self.ctx.type_name(ty)));
                    return Err(());
                };
                let bop = match op {
                    BitAnd => BitOp::And,
                    BitOr => BitOp::Or,
                    BitXor => BitOp::Xor,
                    Shl => BitOp::Shl,
                    _ => BitOp::Shr,
                };
                let dst = self.new_reg(ty);
                self.emit(bitop(bop, prim, dst, lhs_reg, rhs_reg), sp.lo);
                Ok(ty)
            }
            And | Or => unreachable!(),
        }
    }

    pub(crate) fn compile_shortcircuit(&mut self, op: rut_ast::ast::BinOp, lhs: NodeHandle<AnyExpr>, rhs: NodeHandle<AnyExpr>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        let lt = self.compile_expr(lhs, Some(TY_BOOL))?;
        if lt != TY_BOOL {
            self.ctx.err(sp, format!("`&&`/`||` need `bool` operands, found `{}`", self.ctx.type_name(lt)));
        }
        let lreg = self.last_reg;
        let dst = self.new_reg(TY_BOOL);
        // short-circuit: `&&` false short-circuits; `||` true short-circuits.
        // The rhs's CODE sits under l_rhs, so it only evaluates (or traps)
        // on the path that needs it; each path writes the result into dst.
        let l_short = self.new_label(); // value already decided here
        let l_rhs = self.new_label();
        let l_end = self.new_label();
        match op {
            rut_ast::ast::BinOp::And => self.br(lreg, l_rhs, l_short),
            _ => self.br(lreg, l_short, l_rhs),
        }
        self.bind(l_short);
        let short_reg = self.new_reg(TY_BOOL);
        let is_and = op == rut_ast::ast::BinOp::And;
        self.emit(Op::ConstRaw { dst: short_reg, bits: (!is_and) as u64 }, sp.lo);
        self.emit(Op::Mov { dst, src: short_reg }, sp.lo);
        self.jmp(l_end);
        self.bind(l_rhs);
        let rt_ = self.compile_expr(rhs, Some(TY_BOOL))?;
        if rt_ != TY_BOOL {
            self.ctx.err(sp, format!("`&&`/`||` need `bool` operands, found `{}`", self.ctx.type_name(rt_)));
        }
        self.emit(Op::Mov { dst, src: self.last_reg }, sp.lo);
        self.bind(l_end);
        // the value lives in `dst`; move it out so last_reg holds it
        let out = self.new_reg(TY_BOOL);
        self.emit(Op::Mov { dst: out, src: dst }, sp.lo);
        Ok(TY_BOOL)
    }

    // ---- assignment (mut-binding law) ----

    pub(crate) fn compile_assign(
        &mut self,
        _node: NodeHandle<AnyExpr>,
        op: Option<rut_ast::ast::BinOp>,
        target: NodeHandle<AnyExpr>,
        value: NodeHandle<AnyExpr>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<()> {
        match self.ctx.ast.expr(target).clone() {
            ExprKind::Path { segs } if segs.len() >= 2 => {
                // `p.x = 4`: the head must be a local; build the GetF chain,
                // assign the last field
                if segs[0].generics.is_empty() {
                    if let Some(l) = self.lookup(segs[0].name).cloned() {
                        // the mut-binding law is uniform under
                        // the by-reference regime: a `?T` head
                        // derefs, then writes through the shared cell — the
                        // head binding itself must be `let mut`
                        if !l.is_mut && !l.loop_var {
                            self.ctx.err(sp, format!(
                                "assignment through `{}` requires a `let mut` binding (the mut-binding law)",
                                self.ctx.name(segs[0].name)
                            ));
                            return Err(());
                        }
                        // the chain head: a promoted binding loads its
                        // shared cell's value first (the capture law's
                        // accessor); an ordinary one chains off its
                        // register raw, as before
                        let mut cur = if l.cell.is_some() { self.read_local(&l, sp.lo) } else { l.reg };
                        let mut cur_ty = l.ty;
                        if matches!(self.ctx.types.kind(cur_ty).clone(), TyKind::Opt { .. }) {
                            let (t, d) = self.deref_for_use(cur_ty, cur, sp.lo);
                            cur = d;
                            cur_ty = t;
                        }
                        for seg in &segs[1..segs.len() - 1] {
                            // a pointer mid-chain derefs before the next field
                            if matches!(self.ctx.types.kind(cur_ty).clone(), TyKind::Opt { .. }) {
                                let (t, d) = self.deref_for_use(cur_ty, cur, sp.lo);
                                cur = d;
                                cur_ty = t;
                            }
                            let TyKind::Data { fields } = self.ctx.types.kind(cur_ty).clone() else {
                                self.ctx.err(sp, "field assignment through a non-record");
                                return Err(());
                            };
                            let Some(fidx) = fields.iter().position(|f| f.name == seg.name) else {
                                self.ctx.err(sp, format!("`{}` has no field `{}`", self.ctx.type_name(cur_ty), self.ctx.name(seg.name)));
                                return Err(());
                            };
                            let fty = fields[fidx].ty;
                            let dst = self.new_reg(fty);
                            self.emit(Op::GetF { dst, obj: cur, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                            cur = dst;
                            cur_ty = fty;
                        }
                        let last = &segs[segs.len() - 1];
                        let TyKind::Data { fields } = self.ctx.types.kind(cur_ty).clone() else {
                            self.ctx.err(sp, "field assignment through a non-record");
                            return Err(());
                        };
                        let Some(fidx) = fields.iter().position(|f| f.name == last.name) else {
                            self.ctx.err(sp, format!("`{}` has no field `{}`", self.ctx.type_name(cur_ty), self.ctx.name(last.name)));
                            return Err(());
                        };
                        let fty = fields[fidx].ty;
                        match op {
                            None => {
                                let t = self.compile_expr(value, Some(fty))?;
                                if !self.same_ty(t, fty) {
                                    self.ctx.err(sp, "field assignment type mismatch");
                                }
                                self.emit(Op::SetF { obj: cur, field: fidx as u32, val: self.last_reg, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                            }
                            Some(bin) => {
                                let cur_v = self.new_reg(fty);
                                self.emit(Op::GetF { dst: cur_v, obj: cur, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                                // a `?T` field computes at T: the
                                // accumulator derefs, the result re-boxes
                                let (cty, cur_v) = self.deref_for_use(fty, cur_v, sp.lo);
                                let t = self.compile_expr(value, Some(cty))?;
                                if !self.same_ty(t, cty) {
                                    self.ctx.err(sp, "assignment type mismatch");
                                }
                                let val_reg = self.last_reg;
                                let res = self.new_reg(cty);
                                self.emit_compound(bin, cty, cur_v, val_reg, res, sp)?;
                                let res = self.coerce_to(cty, fty, res, sp.lo);
                                self.emit(Op::SetF { obj: cur, field: fidx as u32, val: res, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                            }
                        }
                        return Ok(());
                    }
                }
                self.ctx.err(sp, "invalid assignment target");
                Err(())
            }
            ExprKind::Path { segs } if segs.len() == 1 => {
                let name = segs[0].name;
                let Some(l) = self.lookup(name).cloned() else {
                    self.ctx.err(sp, format!("unknown name `{}`", self.ctx.name(name)));
                    return Err(());
                };
                if !l.is_mut && !l.loop_var {
                    self.ctx.err(sp, format!(
                        "assignment to `{}` requires a `let mut` binding (the mut-binding law)",
                        self.ctx.name(name)
                    ));
                    return Err(());
                }
                match op {
                    None => {
                        // `s = f"{s}{..}"` builds into `s` in place (amortized
                        // growth, no copy of the growing prefix each step);
                        // see `compile_fstr_into`. A promoted binding skips
                        // the in-place form: the concat must land in the
                        // cell, not in the register that holds the cell.
                        if l.cell.is_none() && l.ty == TY_STR && self.try_accumulate_fstr(value, name, l.reg, sp)? {
                            return Ok(());
                        }
                        let t = self.compile_expr(value, Some(l.ty))?;
                        // implicit widening at the assignment:
                        // a concrete value coerces to a trait-typed binding
                        if !self.widens_val(value, t, l.ty) {
                            self.ctx.err(sp, format!(
                                "assignment type mismatch: `{}` vs `{}`",
                                self.ctx.type_name(l.ty), self.ctx.type_name(t)
                            ));
                        }
                        // concrete → slot widening boxes scalars (the slot
                        // ABI): the binding always holds a cell
                        self.widen_to_slot(t, l.ty, sp.lo);
                        // the capture law's write path: a promoted
                        // binding stores into its shared cell (`SetF`),
                        // an ordinary one takes the value's handle (the
                        // sharing law — a share, never a copy)
                        let src = self.last_reg;
                        self.write_local(name, src, sp.lo);
                        // origin counting: a concrete value
                        // re-pins the origin; anything else erases it —
                        // a stale origin could statically bind the WRONG
                        // impl, which must be unrepresentable
                        let origins = if t != l.ty
                            && !matches!(self.ctx.types.kind(t), TyKind::IfaceObj { .. })
                        {
                            vec![t]
                        } else {
                            Vec::new()
                        };
                        self.set_origins(name, origins);
                        // async weave: mirror the write into
                        // the frame cell — the park's ret releases
                        // registers, the fields are what survive
                        self.mirror_local(name, sp.lo);
                    }
                    Some(bin) => {
                        // compound form: the accumulator reads through
                        // the capture law's accessor, the result stores
                        // through the write path
                        let cur = self.read_local(&l, sp.lo);
                        let t = self.compile_expr(value, Some(l.ty))?;
                        if t != l.ty {
                            self.ctx.err(sp, format!(
                                "assignment type mismatch: `{}` vs `{}`",
                                self.ctx.type_name(l.ty), self.ctx.type_name(t)
                            ));
                        }
                        let val_reg = self.last_reg;
                        let res = self.new_reg(l.ty);
                        self.emit_compound(bin, l.ty, cur, val_reg, res, sp)?;
                        // res is freshly computed (arith) — a handle move
                        self.write_local(name, res, sp.lo);
                        // async weave: mirror the write (above)
                        self.mirror_local(name, sp.lo);
                    }
                }
                Ok(())
            }            ExprKind::Field { recv, name } => {
                if !self.check_recv_mut(recv, sp, "field assignment") {
                    return Err(());
                }
                let rt = self.compile_expr(recv, None)?;
                let rreg = self.last_reg;
                // `p.f = ..` through a nullable auto-derefs —
                // so does `arr[i].f = ..` when the elements are `?T`
                let (rt, rreg) = self.deref_for_use(rt, rreg, sp.lo);
                let TyKind::Data { fields } = self.ctx.types.kind(rt).clone() else {
                    self.ctx.err(sp, "field assignment needs a struct/class receiver");
                    return Err(());
                };
                let Some(fidx) = fields.iter().position(|f| f.name == name) else {
                    self.ctx.err(sp, format!("`{}` has no field `{}`", self.ctx.type_name(rt), self.ctx.name(name)));
                    return Err(());
                };
                let fty = fields[fidx].ty;
                match op {
                    None => {
                        let t = self.compile_expr(value, Some(fty))?;
                        if !self.same_ty(t, fty) {
                            self.ctx.err(sp, "field assignment type mismatch");
                        }
                        self.emit(Op::SetF { obj: rreg, field: fidx as u32, val: self.last_reg, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                    }
                    Some(bin) => {
                        let cur = self.new_reg(fty);
                        self.emit(Op::GetF { dst: cur, obj: rreg, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                        // a `?T` field computes at T: the
                        // accumulator derefs, the result re-boxes
                        let (cty, cur) = self.deref_for_use(fty, cur, sp.lo);
                        let t = self.compile_expr(value, Some(cty))?;
                        if !self.same_ty(t, cty) {
                            self.ctx.err(sp, "assignment type mismatch");
                        }
                        let val_reg = self.last_reg;
                        let res = self.new_reg(cty);
                        self.emit_compound(bin, cty, cur, val_reg, res, sp)?;
                        let res = self.coerce_to(cty, fty, res, sp.lo);
                        self.emit(Op::SetF { obj: rreg, field: fidx as u32, val: res, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                    }
                }
                Ok(())
            }
            ExprKind::Index { recv, idx } => {
                if !self.check_recv_mut(recv, sp, "index assignment") {
                    return Err(());
                }
                let rt = self.compile_expr(recv, None)?;
                let rreg = self.last_reg;
                // `xs[i] = ..` through a pointer auto-derefs
                let (rt, rreg) = self.deref_for_use(rt, rreg, sp.lo);
                let Some(info) = self.slice_info(rt) else {
                    self.ctx.err(sp, "index assignment needs a sequence (Vec or Array)");
                    return Err(());
                };
                let elem = info.elem;
                let it = self.compile_expr(idx, Some(TY_I32))?;
                if it != TY_I32 {
                    self.ctx.err(sp, "index must be i32");
                }
                let ireg = self.last_reg;
                match op {
                    None => {
                        let t = self.compile_expr(value, Some(elem))?;
                        if !self.same_ty(t, elem) {
                            self.ctx.err(sp, "element assignment type mismatch");
                        }
                        let vreg = self.last_reg;
                        self.emit_slice_set(rreg, ireg, vreg, &info, sp.lo)?;
                    }
                    Some(bin) => {
                        let cur = self.emit_slice_get(rreg, ireg, &info, sp.lo)?;
                        // a `?T` element computes at T: the
                        // accumulator derefs, the result re-boxes
                        let (cty, cur) = self.deref_for_use(elem, cur, sp.lo);
                        let t = self.compile_expr(value, Some(cty))?;
                        if !self.same_ty(t, cty) {
                            self.ctx.err(sp, "element assignment type mismatch");
                        }
                        let val_reg = self.last_reg;
                        let res = self.new_reg(cty);
                        self.emit_compound(bin, cty, cur, val_reg, res, sp)?;
                        let res = self.coerce_to(cty, elem, res, sp.lo);
                        self.emit_slice_set(rreg, ireg, res, &info, sp.lo)?;
                    }
                }
                Ok(())
            }
            _ => {
                self.ctx.err(sp, "invalid assignment target");
                Err(())
            }
        }
    }

    pub(crate) fn emit_compound(
        &mut self,
        bin: rut_ast::ast::BinOp,
        ty: TypeId,
        a: u16,
        b: u16,
        dst: u16,
        sp: rut_lexer::span::Span,
    ) -> TcResult<()> {
        use rut_ast::ast::BinOp::*;
        let prim = if let TyKind::Prim(p) = self.ctx.types.kind(ty) { Some(*p) } else { None };
        match bin {
            Add | Sub | Mul | Div | Mod => {
                let Some(prim) = prim.filter(|p| p.is_int() || p.is_float()) else {
                    self.ctx.err(sp, format!("arithmetic needs numbers —found `{}`", self.ctx.type_name(ty)));
                    return Err(());
                };
                let aop = match bin {
                    Add => ArithOp::Add,
                    Sub => ArithOp::Sub,
                    Mul => ArithOp::Mul,
                    Div => ArithOp::Div,
                    _ => ArithOp::Mod,
                };
                self.emit(arith(aop, prim, dst, a, b), sp.lo);
            }
            BitAnd | BitOr | BitXor | Shl | Shr => {
                let Some(prim) = prim.filter(|p| p.is_int()) else {
                    self.ctx.err(sp, format!("bit operations need integers —found `{}`", self.ctx.type_name(ty)));
                    return Err(());
                };
                let bop = match bin {
                    BitAnd => BitOp::And,
                    BitOr => BitOp::Or,
                    BitXor => BitOp::Xor,
                    Shl => BitOp::Shl,
                    _ => BitOp::Shr,
                };
                self.emit(bitop(bop, prim, dst, a, b), sp.lo);
            }
            And | Or | Eq | Ne | Lt | Gt | Le | Ge => {
                self.ctx.err(sp, "this operator has no compound-assignment form");
                return Err(());
            }
        }
        Ok(())
    }

    pub(crate) fn check_formattable(&mut self, ty: TypeId, sp: rut_lexer::span::Span) -> TcResult<()> {
        // Table: ints, floats, bool, char, string, enum
        match self.ctx.types.kind(ty) {
            TyKind::Prim(_) | TyKind::Str | TyKind::Enum { .. } => Ok(()),
            _ => {
                self.ctx.err(sp, format!(
                    "`{}` is not formattable inside f\"...\" —use `debug.str(x)` or a `to_string()` method",
                    self.ctx.type_name(ty)
                ));
                Err(())
            }
        }
    }

}
