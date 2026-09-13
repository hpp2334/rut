//! Load-time verification — RFC 0033 §2: corrupted binaries never execute.
//! Structural re-checks per function: register indices vs the signature,
//! jump targets in range, call arities vs callee signatures, native/type
//! operands present. A failure is a load error naming the function.

use rut_core::binary::Program;
use rut_core::ops::Op;
use rut_core::types::{TY_U8, TyKind};

pub fn verify(prog: &Program) -> Result<(), String> {
    let ntypes = prog.types.types.len() as u32;
    for (fi, f) in prog.funcs.iter().enumerate() {
        let nregs = f.regs.len();
        let ctx = |m: String| format!("function {} (#{fi}): {m}", f.name);
        for (pc, op) in f.code.iter().enumerate() {
            let bad = |m: String| ctx(format!("op @{pc}: {m}"));
            // every register operand must be in range
            for r in regs_of(op) {
                if r as usize >= nregs {
                    return Err(bad(format!("register r{r} out of range ({nregs} regs)")));
                }
            }
            // type operands must be in the table
            for t in tys_of(op) {
                if t != u32::MAX && t >= ntypes {
                    return Err(bad(format!("type id {t} out of range")));
                }
            }
            match op {
                Op::Jmp { target } => {
                    if *target as usize >= f.code.len() {
                        return Err(bad(format!("jump target {target} out of range")));
                    }
                }
                Op::Br { then_t, else_t, .. } => {
                    for t in [then_t, else_t] {
                        if *t as usize >= f.code.len() {
                            return Err(bad(format!("branch target {t} out of range")));
                        }
                    }
                }
                Op::BrTable { table, default, .. } => {
                    for t in table.iter().chain(std::iter::once(default)) {
                        if *t as usize >= f.code.len() {
                            return Err(bad(format!("brtable target {t} out of range")));
                        }
                    }
                }
                Op::Call { func, args, .. } => {
                    let callee = prog
                        .funcs
                        .get(*func as usize)
                        .ok_or_else(|| bad(format!("call target #{func} out of range")))?;
                    if args.len() != callee.params.len() {
                        return Err(bad(format!(
                            "call arity: {} args for {} params",
                            args.len(),
                            callee.params.len()
                        )));
                    }
                }
                Op::CallM { func, args, .. } => {
                    let callee = prog
                        .funcs
                        .get(*func as usize)
                        .ok_or_else(|| bad(format!("call target #{func} out of range")))?;
                    if !callee.is_method {
                        return Err(bad("CallM to a non-method".into()));
                    }
                    if args.len() + 1 != callee.params.len() {
                        return Err(bad(format!(
                            "method arity: {} args + self for {} params",
                            args.len(),
                            callee.params.len()
                        )));
                    }
                }
                Op::CallI { slot, .. } => {
                    if *slot as usize >= prog.trait_slots.len() {
                        return Err(bad(format!("trait slot {slot} out of range")));
                    }
                }
                Op::MakeClosure { func, .. } => {
                    if *func as usize >= prog.funcs.len() {
                        return Err(bad(format!("closure target #{func} out of range")));
                    }
                }
                Op::EnumNew { ty, member, .. } => {
                    if let TyKind::Enum { members } = prog.types.kind(*ty) {
                        if *member as usize >= members.len() {
                            return Err(bad("enum member out of range".into()));
                        }
                    } else {
                        return Err(bad("EnumNew over a non-enum type".into()));
                    }
                }
                Op::GetF { obj, field, repr, .. } | Op::SetF { obj, field, repr, .. } => {
                    let ty = f.regs[*obj as usize];
                    match prog.types.kind(ty) {
                        TyKind::Data { fields } => {
                            let Some(fi) = fields.get(*field as usize) else {
                                return Err(bad(format!("field index {field} out of range")));
                            };
                            // the baked repr must match the field's static type
                            // (RFC 0033 §2) — a mismatch would mis-handle RC
                            if prog.types.repr_of(fi.ty) != *repr {
                                return Err(bad(
                                    "field access repr does not match the field type".into(),
                                ));
                            }
                        }
                        _ => return Err(bad("field access on a non-record register".into())),
                    }
                }
                Op::ArrGet { arr, repr, .. } | Op::ArrSet { arr, repr, .. } => {
                    let ety = match prog.types.kind(f.regs[*arr as usize]) {
                        TyKind::Array { elem } => *elem,
                        TyKind::Bytes => TY_U8,
                        _ => return Err(bad("array op on a non-sequence register".into())),
                    };
                    if prog.types.repr_of(ety) != *repr {
                        return Err(bad("array op repr does not match the element type".into()));
                    }
                }
                Op::ArrNew { ty, repr, .. } => {
                    let ety = match prog.types.kind(*ty) {
                        TyKind::Array { elem } => *elem,
                        TyKind::Bytes => TY_U8,
                        _ => return Err(bad("arrnew over a non-array type".into())),
                    };
                    if prog.types.repr_of(ety) != *repr {
                        return Err(bad("arrnew repr does not match the element type".into()));
                    }
                }
                Op::ArrGetF { obj, field, repr, .. } | Op::ArrSetF { obj, field, repr, .. } => {
                    let ety = match prog.types.kind(f.regs[*obj as usize]) {
                        TyKind::Data { fields } => match fields.get(*field as usize) {
                            Some(fi) => match prog.types.kind(fi.ty) {
                                TyKind::Array { elem } => *elem,
                                _ => return Err(bad("field-array op on a non-array field".into())),
                            },
                            None => return Err(bad("field-array op field index out of range".into())),
                        },
                        _ => return Err(bad("field-array op on a non-record register".into())),
                    };
                    if prog.types.repr_of(ety) != *repr {
                        return Err(bad("field-array op repr does not match the element type".into()));
                    }
                }
                Op::MakeRecord { ty, vals, .. } => match prog.types.kind(*ty) {
                    TyKind::Data { fields } => {
                        if vals.len() != fields.len() {
                            return Err(bad(
                                "MakeRecord value count does not match the field count".into(),
                            ));
                        }
                        for (i, &v) in vals.iter().enumerate() {
                            if f.regs[v as usize] != fields[i].ty {
                                return Err(bad(
                                    "MakeRecord value type does not match the field type".into(),
                                ));
                            }
                        }
                    }
                    _ => return Err(bad("MakeRecord over a non-record type".into())),
                },
                Op::IsTrait { want, .. } => {
                    if *want as usize >= prog.traits.len() {
                        return Err(bad("IsTrait want not in the trait table".into()));
                    }
                }
                Op::AddF { prim, a, b, .. }
                | Op::SubF { prim, a, b, .. }
                | Op::MulF { prim, a, b, .. }
                | Op::DivF { prim, a, b, .. }
                | Op::ModF { prim, a, b, .. } => {
                    if !prim.is_float() {
                        return Err(bad("float arith op with a non-float primitive".into()));
                    }
                    for r in [a, b] {
                        match prog.types.kind(f.regs[*r as usize]) {
                            TyKind::Prim(p) if p == prim => {}
                            _ => {
                                return Err(bad(
                                    "float arith operand is not the embedded primitive".into(),
                                ))
                            }
                        }
                    }
                }
                Op::NegF { prim, a, .. } => {
                    if !prim.is_float() {
                        return Err(bad("float neg with a non-float primitive".into()));
                    }
                    match prog.types.kind(f.regs[*a as usize]) {
                        TyKind::Prim(p) if p == prim => {}
                        _ => {
                            return Err(bad("float neg operand is not the embedded primitive".into()))
                        }
                    }
                }
                Op::EqF { a, b, .. }
                | Op::NeF { a, b, .. }
                | Op::LtF { a, b, .. }
                | Op::GtF { a, b, .. }
                | Op::LeF { a, b, .. }
                | Op::GeF { a, b, .. } => {
                    for r in [a, b] {
                        match prog.types.kind(f.regs[*r as usize]) {
                            TyKind::Prim(p) if p.is_float() => {}
                            _ => return Err(bad("float compare operand is not a float".into())),
                        }
                    }
                }
                Op::AddI { prim, a, b, .. }
                | Op::SubI { prim, a, b, .. }
                | Op::MulI { prim, a, b, .. }
                | Op::DivI { prim, a, b, .. }
                | Op::ModI { prim, a, b, .. }
                | Op::WAddI { prim, a, b, .. }
                | Op::WSubI { prim, a, b, .. }
                | Op::WMulI { prim, a, b, .. }
                | Op::WDivI { prim, a, b, .. }
                | Op::WModI { prim, a, b, .. }
                | Op::AndI { prim, a, b, .. }
                | Op::OrI { prim, a, b, .. }
                | Op::XorI { prim, a, b, .. }
                | Op::ShlI { prim, a, b, .. }
                | Op::ShrI { prim, a, b, .. }
                | Op::WrapShlI { prim, a, b, .. } => {
                    if !prim.is_int() {
                        return Err(bad("int op with a non-integer primitive".into()));
                    }
                    for r in [a, b] {
                        match prog.types.kind(f.regs[*r as usize]) {
                            TyKind::Prim(p) if p == prim => {}
                            _ => {
                                return Err(bad(
                                    "int op operand is not the embedded primitive".into(),
                                ))
                            }
                        }
                    }
                }
                Op::NegI { prim, a, .. } => {
                    if !prim.is_int() {
                        return Err(bad("int neg with a non-integer primitive".into()));
                    }
                    match prog.types.kind(f.regs[*a as usize]) {
                        TyKind::Prim(p) if p == prim => {}
                        _ => return Err(bad("int neg operand is not the embedded primitive".into())),
                    }
                }
                Op::EqI { prim, a, b, .. }
                | Op::NeI { prim, a, b, .. }
                | Op::LtI { prim, a, b, .. }
                | Op::GtI { prim, a, b, .. }
                | Op::LeI { prim, a, b, .. }
                | Op::GeI { prim, a, b, .. } => {
                    if prim.is_float() {
                        return Err(bad("int compare with a float primitive".into()));
                    }
                    for r in [a, b] {
                        match prog.types.kind(f.regs[*r as usize]) {
                            TyKind::Prim(p) if p == prim => {}
                            _ => {
                                return Err(bad(
                                    "int compare operand is not the embedded primitive".into(),
                                ))
                            }
                        }
                    }
                }
                Op::Conv { from, to, dst, src, .. } => {
                    if !matches!(prog.types.kind(f.regs[*src as usize]), TyKind::Prim(p) if p == from)
                        || !matches!(prog.types.kind(f.regs[*dst as usize]), TyKind::Prim(p) if p == to)
                    {
                        return Err(bad("conv operand types do not match the embedded primitives".into()));
                    }
                }
                Op::OptSome { ty, .. } | Op::OptNone { ty, .. } => {
                    if !matches!(prog.types.kind(*ty), TyKind::Option { .. }) {
                        return Err(bad("Option op over a non-Option type".into()));
                    }
                }
                Op::ResOk { ty, .. } | Op::ResErr { ty, .. } => {
                    if !matches!(prog.types.kind(*ty), TyKind::Result { .. }) {
                        return Err(bad("Result op over a non-Result type".into()));
                    }
                }
                _ => {}
            }
        }
        match f.code.last() {
            Some(Op::Ret { .. }) => {}
            _ => return Err(ctx("function must end in Ret".into())),
        }
    }
    Ok(())
}

fn regs_of(op: &Op) -> Vec<u16> {
    let mut v = Vec::new();
    let mut push = |r: u16| v.push(r);
    match op {
        // single-dst ops
        Op::Mov { dst, src } | Op::MovRef { dst, src } => {
            push(*dst);
            push(*src);
        }
        Op::Const { dst, .. } | Op::ConstRaw { dst, .. } | Op::NewCell { dst, .. }
        | Op::ArrNew { dst, .. } | Op::ArrLit { dst, .. } | Op::EnumNew { dst, .. }
        | Op::OptNone { dst, .. } | Op::Panic { msg: dst } => push(*dst),
        Op::MakeRecord { dst, vals, .. } => {
            push(*dst);
            for &v in vals {
                push(v);
            }
        }
        // two-operand scalar ops
        Op::Not { dst, a } | Op::NegF { dst, a, .. }
        | Op::NegI { dst, a, .. } => {
            push(*dst);
            push(*a);
        }
        // three-operand ops
        Op::StrCmp { dst, a, b, .. } | Op::RefEq { dst, a, b, .. }
        | Op::ArrayCmp { dst, a, b, .. }
        | Op::ArrGet { dst, arr: a, idx: b, .. }
        | Op::ArrGetF { dst, obj: a, idx: b, .. }
        | Op::AddF { dst, a, b, .. } | Op::SubF { dst, a, b, .. } | Op::MulF { dst, a, b, .. }
        | Op::DivF { dst, a, b, .. } | Op::ModF { dst, a, b, .. } | Op::EqF { dst, a, b, .. }
        | Op::NeF { dst, a, b, .. } | Op::LtF { dst, a, b, .. } | Op::GtF { dst, a, b, .. }
        | Op::LeF { dst, a, b, .. } | Op::GeF { dst, a, b, .. }
        | Op::AddI { dst, a, b, .. } | Op::SubI { dst, a, b, .. } | Op::MulI { dst, a, b, .. }
        | Op::DivI { dst, a, b, .. } | Op::ModI { dst, a, b, .. } | Op::WAddI { dst, a, b, .. }
        | Op::WSubI { dst, a, b, .. } | Op::WMulI { dst, a, b, .. } | Op::WDivI { dst, a, b, .. }
        | Op::WModI { dst, a, b, .. } | Op::AndI { dst, a, b, .. } | Op::OrI { dst, a, b, .. }
        | Op::XorI { dst, a, b, .. } | Op::ShlI { dst, a, b, .. } | Op::ShrI { dst, a, b, .. }
        | Op::WrapShlI { dst, a, b, .. } | Op::EqI { dst, a, b, .. } | Op::NeI { dst, a, b, .. }
        | Op::LtI { dst, a, b, .. } | Op::GtI { dst, a, b, .. } | Op::LeI { dst, a, b, .. }
        | Op::GeI { dst, a, b, .. } => {
            push(*dst);
            push(*a);
            push(*b);
        }
        Op::Own { dst, src, .. } => {
            push(*dst);
            push(*src);
        }
        Op::OptSome { dst, val, .. } | Op::ResOk { dst, val, .. } | Op::ResErr { dst, val, .. } => {
            push(*dst);
            push(*val);
        }
        Op::SetF { obj, val, .. } => {
            push(*obj);
            push(*val);
        }
        Op::ArrSet { arr, idx, val, .. } => {
            push(*arr);
            push(*idx);
            push(*val);
        }
        Op::ArrSetF { obj, idx, val, .. } => {
            push(*obj);
            push(*idx);
            push(*val);
        }
        Op::SumIs { dst, v: a, .. } | Op::Unwrap { dst, v: a, .. } | Op::Unbox { dst, box_: a, .. } => {
            push(*dst);
            push(*a);
        }
        Op::UnwrapOr { dst, v: a, default: b } => {
            push(*dst);
            push(*a);
            push(*b);
        }
        Op::Box { dst, val, .. } => {
            push(*dst);
            push(*val);
        }
        Op::Expect { dst, v: a, msg: b } => {
            push(*dst);
            push(*a);
            push(*b);
        }
        Op::TidOf { dst, obj } | Op::IsType { dst, obj, .. } | Op::IsTrait { dst, obj, .. } => {
            push(*dst);
            push(*obj);
        }
        Op::Br { cond, .. } => push(*cond),
        Op::BrTable { idx, .. } => push(*idx),
        Op::Call { args, dst, .. } => {
            v.extend_from_slice(args);
            dst.into_iter().for_each(|d| v.push(*d));
        }
        Op::CallM { recv, args, dst, .. } | Op::CallI { recv, args, dst, .. } => {
            push(*recv);
            v.extend_from_slice(args);
            dst.into_iter().for_each(|d| v.push(*d));
        }
        Op::CallNat { recv, args, dst, .. } => {
            recv.into_iter().for_each(|r| v.push(*r));
            v.extend_from_slice(args);
            dst.into_iter().for_each(|d| v.push(*d));
        }
        Op::CallFn { fval, args, dst } => {
            push(*fval);
            v.extend_from_slice(args);
            dst.into_iter().for_each(|d| v.push(*d));
        }
        Op::Ret { val } => val.into_iter().for_each(|r| v.push(*r)),
        Op::GetF { dst, obj, .. } => {
            push(*dst);
            push(*obj);
        }
        Op::MakeClosure { dst, captures, .. } => {
            push(*dst);
            v.extend_from_slice(captures);
        }
        Op::Conv { dst, src, .. } => {
            push(*dst);
            push(*src);
        }
        Op::StrCharAt { dst, s, idx } => {
            push(*dst);
            push(*s);
            push(*idx);
        }
        Op::Assert { cond, msg } => {
            push(*cond);
            msg.into_iter().for_each(|m| v.push(*m));
        }
        Op::Jmp { .. } | Op::LoopHead => {}
    }
    v
}

fn tys_of(op: &Op) -> Vec<u32> {
    match op {
        Op::NewCell { ty, .. } | Op::Own { ty, .. }
        | Op::ArrNew { ty, .. } | Op::ArrLit { ty, .. } | Op::EnumNew { ty, .. }
        | Op::OptSome { ty, .. } | Op::OptNone { ty, .. } | Op::ResOk { ty, .. }
        | Op::ResErr { ty, .. } | Op::IsType { want: ty, .. } | Op::Unbox { ty, .. }
        | Op::Box { ty, .. } | Op::MakeRecord { ty, .. } => vec![*ty],
        _ => Vec::new(),
    }
}
