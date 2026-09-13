//! Operators (RFC 0004 SS3: equal widths, traps by default; the wrapping
//! &op family) and assignment: the mut-binding law (RFC 0003 SS1),
//! compound forms, field/index targets.

use crate::check::TcResult;
use rut_core::ops::*;
use rut_core::types::*;
use super::*;

impl<'a, 'b> FnCompiler<'a, 'b> {

    // ---- operators (RFC 0004 §3: equal widths; traps by default) ----

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
        // bidirectional unification (RFC 0007 §1): an expected type from the
        // enclosing position (return/assignment/argument) seeds the lhs, then
        // rhs adapts to the lhs. Without it, `1.0 / f64(x)` and unannotated
        // `4.0 * pi` would default the lhs to f32 and mismatch.
        let lt = self.compile_expr(lhs, expected)?;
        let lhs_reg = self.last_reg;
        let rt_ = self.compile_expr(rhs, Some(lt))?;
        let rhs_reg = self.last_reg;
        if lt != rt_ {
            self.ctx.err(sp, format!(
                "operands must have equal width (RFC 0004 §3): `{}` vs `{}` —convert first (RFC 0007 §1)",
                self.ctx.types.name(lt), self.ctx.types.name(rt_)
            ));
            return Err(());
        }
        let ty = lt;
        match op {
            Eq | Ne => {
                // the `==` law (RFC 0012 §4): primitives by value, string and
                // bytes by content, everything else cell identity;
                // Option/Result is a compile error
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
                    TyKind::Option { .. } | TyKind::Result { .. } => {
                        self.ctx.err(sp, "`==` on Option/Result is a compile error —use `when`, `is_some()`, or compare the payload (RFC 0005)");
                        return Err(());
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
                    self.ctx.err(sp, format!("ordering comparisons need numbers —`{}` has none", self.ctx.types.name(ty)));
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
                    self.ctx.err(sp, format!("arithmetic needs numbers —found `{}`", self.ctx.types.name(ty)));
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
                    self.ctx.err(sp, format!("bit operations need integers —found `{}`", self.ctx.types.name(ty)));
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
            self.ctx.err(sp, format!("`&&`/`||` need `bool` operands, found `{}`", self.ctx.types.name(lt)));
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
            self.ctx.err(sp, format!("`&&`/`||` need `bool` operands, found `{}`", self.ctx.types.name(rt_)));
        }
        self.emit(Op::Mov { dst, src: self.last_reg }, sp.lo);
        self.bind(l_end);
        // the value lives in `dst`; move it out so last_reg holds it
        let out = self.new_reg(TY_BOOL);
        self.emit(Op::Mov { dst: out, src: dst }, sp.lo);
        Ok(TY_BOOL)
    }

    // ---- assignment (mut-binding law —RFC 0003 §1) ----

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
                    if let Some(l) = self.lookup(segs[0].name).copied() {
                        if !l.is_mut && !l.loop_var {
                            self.ctx.err(sp, format!(
                                "assignment through `{}` requires a `let mut` binding (the mut-binding law, RFC 0003 §1)",
                                self.ctx.name(segs[0].name)
                            ));
                            return Err(());
                        }
                        let mut cur = l.reg;
                        let mut cur_ty = l.ty;
                        for seg in &segs[1..segs.len() - 1] {
                            let TyKind::Data { fields } = self.ctx.types.kind(cur_ty).clone() else {
                                self.ctx.err(sp, "field assignment through a non-record");
                                return Err(());
                            };
                            let Some(fidx) = fields.iter().position(|f| f.name == self.ctx.name(seg.name)) else {
                                self.ctx.err(sp, format!("`{}` has no field `{}`", self.ctx.types.name(cur_ty), self.ctx.name(seg.name)));
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
                        let Some(fidx) = fields.iter().position(|f| f.name == self.ctx.name(last.name)) else {
                            self.ctx.err(sp, format!("`{}` has no field `{}`", self.ctx.types.name(cur_ty), self.ctx.name(last.name)));
                            return Err(());
                        };
                        let fty = fields[fidx].ty;
                        match op {
                            None => {
                                let t = self.compile_expr(value, Some(fty))?;
                                if t != fty {
                                    self.ctx.err(sp, "field assignment type mismatch");
                                }
                                self.emit(Op::SetF { obj: cur, field: fidx as u32, val: self.last_reg, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                            }
                            Some(bin) => {
                                let cur_v = self.new_reg(fty);
                                self.emit(Op::GetF { dst: cur_v, obj: cur, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                                let t = self.compile_expr(value, Some(fty))?;
                                if t != fty {
                                    self.ctx.err(sp, "assignment type mismatch");
                                }
                                let val_reg = self.last_reg;
                                let res = self.new_reg(fty);
                                self.emit_compound(bin, fty, cur_v, val_reg, res, sp)?;
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
                let Some(l) = self.lookup(name).copied() else {
                    self.ctx.err(sp, format!("unknown name `{}`", self.ctx.name(name)));
                    return Err(());
                };
                if !l.is_mut && !l.loop_var {
                    self.ctx.err(sp, format!(
                        "assignment to `{}` requires a `let mut` binding (the mut-binding law, RFC 0003 §1)",
                        self.ctx.name(name)
                    ));
                    return Err(());
                }
                match op {
                    None => {
                        let t = self.compile_expr(value, Some(l.ty))?;
                        if t != l.ty {
                            self.ctx.err(sp, format!(
                                "assignment type mismatch: `{}` vs `{}`",
                                self.ctx.types.name(l.ty), self.ctx.types.name(t)
                            ));
                        }
                        if self.ctx.types.is_ref(l.ty) {
                            self.emit(Op::MovRef { dst: l.reg, src: self.last_reg }, sp.lo);
                        } else {
                            self.emit(Op::Mov { dst: l.reg, src: self.last_reg }, sp.lo);
                        }
                    }
                    Some(bin) => {
                        let cur = self.new_reg(l.ty);
                        if self.ctx.types.is_ref(l.ty) {
                            self.emit(Op::MovRef { dst: cur, src: l.reg }, sp.lo);
                        } else {
                            self.emit(Op::Mov { dst: cur, src: l.reg }, sp.lo);
                        }
                        let t = self.compile_expr(value, Some(l.ty))?;
                        if t != l.ty {
                            self.ctx.err(sp, format!(
                                "assignment type mismatch: `{}` vs `{}`",
                                self.ctx.types.name(l.ty), self.ctx.types.name(t)
                            ));
                        }
                        let val_reg = self.last_reg;
                        let res = self.new_reg(l.ty);
                        self.emit_compound(bin, l.ty, cur, val_reg, res, sp)?;
                        if self.ctx.types.is_ref(l.ty) {
                            self.emit(Op::MovRef { dst: l.reg, src: res }, sp.lo);
                        } else {
                            self.emit(Op::Mov { dst: l.reg, src: res }, sp.lo);
                        }
                    }
                }
                Ok(())
            }            ExprKind::Field { recv, name } => {
                if !self.check_recv_mut(recv, sp, "field assignment") {
                    return Err(());
                }
                let rt = self.compile_expr(recv, None)?;
                let rreg = self.last_reg;
                let TyKind::Data { fields } = self.ctx.types.kind(rt).clone() else {
                    self.ctx.err(sp, "field assignment needs a dataclass/class receiver");
                    return Err(());
                };
                let Some(fidx) = fields.iter().position(|f| f.name == self.ctx.name(name)) else {
                    self.ctx.err(sp, format!("`{}` has no field `{}`", self.ctx.types.name(rt), self.ctx.name(name)));
                    return Err(());
                };
                let fty = fields[fidx].ty;
                match op {
                    None => {
                        let t = self.compile_expr(value, Some(fty))?;
                        if t != fty {
                            self.ctx.err(sp, "field assignment type mismatch");
                        }
                        self.emit(Op::SetF { obj: rreg, field: fidx as u32, val: self.last_reg, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                    }
                    Some(bin) => {
                        let cur = self.new_reg(fty);
                        self.emit(Op::GetF { dst: cur, obj: rreg, field: fidx as u32, repr: self.ctx.types.repr_of(fty) }, sp.lo);
                        let t = self.compile_expr(value, Some(fty))?;
                        if t != fty {
                            self.ctx.err(sp, "assignment type mismatch");
                        }
                        let val_reg = self.last_reg;
                        let res = self.new_reg(fty);
                        self.emit_compound(bin, fty, cur, val_reg, res, sp)?;
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
                        if t != elem {
                            self.ctx.err(sp, "element assignment type mismatch");
                        }
                        let vreg = self.last_reg;
                        self.emit_slice_set(rreg, ireg, vreg, &info, sp.lo)?;
                    }
                    Some(bin) => {
                        let cur = self.emit_slice_get(rreg, ireg, &info, sp.lo)?;
                        let t = self.compile_expr(value, Some(elem))?;
                        if t != elem {
                            self.ctx.err(sp, "element assignment type mismatch");
                        }
                        let val_reg = self.last_reg;
                        let res = self.new_reg(elem);
                        self.emit_compound(bin, elem, cur, val_reg, res, sp)?;
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
                    self.ctx.err(sp, format!("arithmetic needs numbers —found `{}`", self.ctx.types.name(ty)));
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
                    self.ctx.err(sp, format!("bit operations need integers —found `{}`", self.ctx.types.name(ty)));
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
        // RFC 0007 §2 table: ints, floats, bool, char, string, enum
        match self.ctx.types.kind(ty) {
            TyKind::Prim(_) | TyKind::Str | TyKind::Enum { .. } => Ok(()),
            _ => {
                self.ctx.err(sp, format!(
                    "`{}` is not formattable inside f\"...\" —use `debug.str(x)` or a `to_string()` method (RFC 0007 §2)",
                    self.ctx.types.name(ty)
                ));
                Err(())
            }
        }
    }

}
