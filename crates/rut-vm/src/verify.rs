//! Load-time verification: corrupted binaries never execute.
//! Structural re-checks per function: register indices vs the signature,
//! jump targets in range, call arities vs callee signatures, native/type
//! operands present. A failure is a load error naming the function.

use rut_core::binary::{FuncCode, Program};
use rut_core::ops::{reg_opt, Op};
use rut_core::types::{Repr, TY_U8, TyKind};

pub fn verify(prog: &Program) -> Result<(), String> {
    let ntypes = prog.types.types.len() as u32;
    for (fi, f) in prog.funcs.iter().enumerate() {
        // bodyless host functions have no code to verify; a BODYLESS
        // mirror stub (empty code, no params — every compiled body ends
        // in its implicit Ret) that lost the ledger claim was
        // redirected at link, and one nothing claimed stays loud at
        // its (arity-mismatched) call site
        if f.host_id.is_some() || (f.code.is_empty() && f.params.is_empty()) {
            continue;
        }
        let nregs = f.regs.len();
        let ctx = |m: String| format!("function {} (#{fi}): {m}", prog.func_name(f));
        for (pc, op) in f.code.iter().enumerate() {
            let bad = |m: String| ctx(format!("op @{pc}: {m}"));
            // every register operand must be in range
            for r in regs_of(op, f) {
                if r as usize >= nregs {
                    return Err(bad(format!("register r{r} out of range ({nregs} regs)")));
                }
            }
            // pooled operand spans must be in range
            if let Some((off, argc)) = op.argv_span() {
                if off as usize + argc as usize > f.argv.len() {
                    return Err(bad(format!("argv span {off}+{argc} out of range ({} entries)", f.argv.len())));
                }
            }
            if let Some((off, count)) = op.label_span() {
                if off as usize + count as usize > f.labels.len() {
                    return Err(bad(format!("label span {off}+{count} out of range ({} entries)", f.labels.len())));
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
                Op::BrTable { table_off, count, default, .. } => {
                    let arms = &f.labels[*table_off as usize..*table_off as usize + *count as usize];
                    for t in arms.iter().chain(std::iter::once(default)) {
                        if *t as usize >= f.code.len() {
                            return Err(bad(format!("brtable target {t} out of range")));
                        }
                    }
                }
                Op::Call { func, argv_off, argc, .. } | Op::CallM { func, argv_off, argc, .. } => {
                    let callee = prog
                        .funcs
                        .get(*func as usize)
                        .ok_or_else(|| bad(format!("call target #{func} out of range")))?;
                    if matches!(op, Op::CallM { .. }) && !callee.is_method {
                        return Err(bad("CallM to a non-method".into()));
                    }                    // the pool span is the callee's whole parameter list
                    // (`CallM` folds the receiver into `argv[0]`)
                    let n = *argc as usize;
                    if n != callee.params.len() {
                        return Err(bad(format!(
                            "call arity: {n} args for {} params",
                            callee.params.len()
                        )));
                    }
                    let _ = (argv_off, n);
                }
                Op::CallI { slot, .. } => {
                    if *slot as usize >= prog.iface_slots.len() {
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
                    // `p.x` auto-derefs: a pointer register's
                    // field 0 is the pointee cell (a ref slot); reads
                    // against a Data register index the record's fields
                    let fields = match prog.types.kind(ty) {
                        TyKind::Data { fields } => Some(fields),
                        TyKind::Opt { .. } => None,
                        // the async weave's driven-half field ops on a
                        // future-spelled register: the register holds an
                        // engine frame record the static type erases
                        // (`Future<T>` is the handle, the frame is the
                        // representation); the repr law below is what
                        // protects RC, and the field index is checked at
                        // runtime against the concrete record
                        TyKind::IfaceObj { .. } | TyKind::Future { .. } => None,
                        _ => None,
                    };
                    match fields {
                        Some(fields) => {
                            let Some(fi) = fields.get(*field as usize) else {
                                return Err(bad(format!("field index {field} out of range")));
                            };
                            // the baked repr must match the field's static type
                            // — a mismatch would mis-handle RC
                            if prog.types.repr_of(fi.ty) != *repr {
                                return Err(bad(
                                    "field access repr does not match the field type".into(),
                                ));
                            }
                        }
                        None => {
                            // pointer box: the only access is the payload
                            // slot (field 0) at the pointee's own repr —
                            // ref pointees share the cell, scalar pointees
                            // read the boxed copy
                            let elem_repr = match prog.types.kind(ty) {
                                TyKind::Opt { elem } => prog.types.repr_of(*elem),
                                // the async weave's driven-half field ops
                                // on a future-spelled register: the frame
                                // record is the representation, the index
                                // is checked at runtime against it
                                TyKind::IfaceObj { .. } | TyKind::Future { .. } => {
                                    if *repr != Repr::Ref {
                                        return Err(bad(
                                            "field access on a future must be a ref slot".into(),
                                        ));
                                    }
                                    return Ok(());
                                }
                                _ => return Err(bad("field access on a non-record".into())),
                            };
                            if *field != 0 || elem_repr != *repr {
                                return Err(bad(
                                    "field access on a pointer must be the payload slot".into(),
                                ));
                            }
                        }
                    }
                }
                Op::ArrGet { arr, repr, .. } | Op::ArrSet { arr, repr, .. } => {
                    if std::env::var("RUT_DEBUG_LINK").is_ok() {
                        let aty = f.regs.get(*arr as usize).copied();
                        if let Some(ty) = aty {
                            if !matches!(prog.types.kind(ty), TyKind::Array { .. } | TyKind::Bytes) {
                                eprintln!("DBG arr fn={} op@{} arr-reg={} ty={} raw={} scope={} local={} | reg15={:?} ops={:?}", prog.interner.name(f.name), pc, arr, prog.type_name(ty), ty, rut_core::id::scope_of(ty), rut_core::id::local_of(ty), f.regs.get(15), f.code.iter().skip(pc.saturating_sub(4)).take(6).collect::<Vec<_>>());
                            }
                        }
                    }
                    let ety = match prog.types.kind(f.regs[*arr as usize]) {
                        TyKind::Array { elem } => *elem,
                        TyKind::Bytes => TY_U8,
                        _ => return Err(bad("array op on a non-sequence register".into())),
                    };
                    if !elem_repr_ok(&prog.types, ety, *repr, matches!(op, Op::ArrSet { .. })) {
                        return Err(bad("array op repr does not match the element type".into()));
                    }
                }
                Op::ArrNew { ty, repr, .. } => {
                    let ety = match prog.types.kind(*ty) {
                        TyKind::Array { elem } => *elem,
                        TyKind::Bytes => TY_U8,
                        _ => return Err(bad("arrnew over a non-array type".into())),
                    };
                    if rut_core::types::arr_elem_repr(&prog.types, ety) != *repr {
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
                    if !elem_repr_ok(&prog.types, ety, *repr, matches!(op, Op::ArrSetF { .. })) {
                        return Err(bad("field-array op repr does not match the element type".into()));
                    }
                }
                Op::MakeRecord { ty, argv_off, argc, .. } => {
                    let vals = &f.argv[*argv_off as usize..*argv_off as usize + *argc as usize];
                    match prog.types.kind(*ty) {
                    TyKind::Data { fields } => {
                        if vals.len() != fields.len() {
                            return Err(bad(
                                "MakeRecord value count does not match the field count".into(),
                            ));
                        }
                        for (i, &v) in vals.iter().enumerate() {
                            // the register's type and the field's type may
                            // be two units' rows for ONE instantiation (a
                            // wrapper construction crosses: the consumer
                            // mints the record over the owner's claimed
                            // row) — the layout law compares the SPELLING
                            // (name + kind), never two units' dense ids
                            let reg_ty = f.regs[v as usize];
                            let same = reg_ty == fields[i].ty || {
                                let rn = |t: u32| prog.interner.name(prog.types.type_at(t).name).to_string();
                                let kd = |t: u32| format!("{:?}", prog.types.kind(t));
                                rn(reg_ty) == rn(fields[i].ty) && kd(reg_ty) == kd(fields[i].ty)
                            };
                            if !same {
                                return Err(bad(
                                    "MakeRecord value type does not match the field type".into(),
                                ));
                            }
                        }
                    }
                    _ => return Err(bad("MakeRecord over a non-record type".into())),
                    }
                }
                Op::IsIface { want, .. } => {
                    if *want as usize >= prog.ifaces.len() {
                        return Err(bad("IsIface want not in the trait table".into()));
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

/// Is `repr` a legal baked element representation for an array op over
/// elements of type `ety`? Either the plain resolution
/// (`arr_elem_repr` — the emit sites' choice) or, post-peephole, the
/// refinement forms with the SAME payload prim: `OptPrimRaw` on stores
/// (the MakeOpt elision) and `OptPrimLoad` on reads (the deref fold).
/// A refinement onto a non-`?prim` element, or a payload-prim mismatch,
/// is a miscompile and must fail verification.
fn elem_repr_ok(types: &rut_core::types::TypeTable, ety: u32, repr: Repr, is_store: bool) -> bool {
    if rut_core::types::arr_elem_repr(types, ety) == repr {
        return true;
    }
    let Some(p) = repr.opt_prim() else { return false };
    let TyKind::Opt { elem } = types.kind(ety) else { return false };
    let TyKind::Prim(q) = types.kind(*elem) else { return false };
    if p != *q {
        return false;
    }
    match repr {
        Repr::OptPrimRaw(_) => is_store,
        Repr::OptPrimLoad(_) => !is_store,
        _ => false,
    }
}

fn regs_of(op: &Op, f: &FuncCode) -> Vec<u16> {
    let mut v = Vec::new();
    // pooled operand lists first (the span covers every list-carrying op)
    if let Some((off, argc)) = op.argv_span() {
        v.extend_from_slice(&f.argv[off as usize..off as usize + argc as usize]);
    }
    let mut push = |r: u16| v.push(r);
    match op {
        // single-dst ops
        Op::Mov { dst, src } | Op::MovRef { dst, src } => {
            push(*dst);
            push(*src);
        }
        Op::Const { dst, .. } | Op::ConstRaw { dst, .. } | Op::NewCell { dst, .. }
        | Op::ArrNew { dst, .. } | Op::ArrLit { dst, .. } | Op::EnumNew { dst, .. }
        | Op::Panic { msg: dst } => push(*dst),
        Op::MakeRecord { dst, .. } => push(*dst),
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
        | Op::WSubI { dst, a, b, .. } | Op::WMulI { dst, a, b, .. } | Op::AndI { dst, a, b, .. } | Op::OrI { dst, a, b, .. }
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
        Op::MakeOpt { dst, src, .. } => {
            push(*dst);
            push(*src);
        }
        Op::WeakNew { dst, src, .. } => {
            push(*dst);
            push(*src);
        }
        Op::WeakUpgrade { recv, dst, .. } => {
            push(*recv);
            push(*dst);
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
        Op::Unbox { dst, box_: a, .. } => {
            push(*dst);
            push(*a);
        }
        Op::Box { dst, val, .. } => {
            push(*dst);
            push(*val);
        }
        Op::TidOf { dst, obj } | Op::IsType { dst, obj, .. } | Op::IsIface { dst, obj, .. } => {
            push(*dst);
            push(*obj);
        }
        Op::Br { cond, .. } => push(*cond),
        Op::BrTable { idx, .. } => push(*idx),
        Op::Call { dst, .. } | Op::CallM { dst, .. } | Op::CallI { dst, .. } | Op::CallFn { dst, .. } => {
            if *dst != rut_core::ops::NOREG {
                v.push(*dst);
            }
        }
        Op::CallNat { recv, dst, .. } => {
            if *recv != rut_core::ops::NOREG {
                v.push(*recv);
            }
            if *dst != rut_core::ops::NOREG {
                v.push(*dst);
            }
        }
        Op::Ret { val } => val.into_iter().for_each(|r| v.push(*r)),
        Op::GetF { dst, obj, .. } => {
            push(*dst);
            push(*obj);
        }
        Op::MakeClosure { dst, .. } => push(*dst),
        Op::Conv { dst, src, .. } => {
            push(*dst);
            push(*src);
        }
        Op::StrCodeAt { dst, s, idx } => {
            push(*dst);
            push(*s);
            push(*idx);
        }
        Op::Jmp { .. } | Op::LoopHead => {}
        #[allow(unreachable_patterns)]
        Op::Pad { .. } => unreachable!("layout pin, never constructed"),
    }
    v
}

fn tys_of(op: &Op) -> Vec<u32> {
    match op {
        Op::NewCell { ty, .. } | Op::Own { ty, .. }
        | Op::ArrNew { ty, .. } | Op::ArrLit { ty, .. } | Op::EnumNew { ty, .. }
        | Op::IsType { want: ty, .. } | Op::Unbox { ty, .. }
        | Op::Box { ty, .. } | Op::MakeRecord { ty, .. } | Op::MakeOpt { ty, .. }
        | Op::WeakNew { ty, .. } | Op::WeakUpgrade { ty, .. } => vec![*ty],
        _ => Vec::new(),
    }
}
