//! The single-op `step` path for ops without a `run_loop` fast arm.
use super::*;

impl Vm {
    pub(super) fn step(&mut self, op: Op) -> Result<(), Trap> {
        macro_rules! r {
            ($i:expr) => {
                self.cur_regs[$i as usize]
            };
        }
        match op {
            Op::LoopHead => {}
            Op::Mov { dst, src } => self.cur_regs[dst as usize] = r!(src),
            Op::MovRef { dst, src } => {
                let v = r!(src);
                self.heap.retain(v);
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = v;
                self.heap.release(old);
            }
            Op::Const { dst, k } => {
                let s = self.const_slots[k as usize];
                // only string consts are heap cells; `type_id`/scalar consts
                // are plain words (RFC 0033 §3)
                let is_ref = matches!(self.prog.consts[k as usize], ConstVal::Str(_));
                if is_ref {
                    self.heap.retain(s);
                }
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = s;
                if is_ref {
                    self.heap.release(old);
                }
            }
            Op::ConstRaw { dst, bits } => self.cur_regs[dst as usize] = Slot { i: bits as i64 },


            Op::Not { dst, a } => {
                let v = !r!(a).as_bool();
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            // specialized scalar ops are their own opcodes and are always
            // fast-pathed
            Op::AddF { .. } | Op::SubF { .. } | Op::MulF { .. } | Op::DivF { .. }
            | Op::ModF { .. } | Op::NegF { .. } | Op::EqF { .. } | Op::NeF { .. }
            | Op::LtF { .. } | Op::GtF { .. } | Op::LeF { .. } | Op::GeF { .. }
            | Op::AddI { .. } | Op::SubI { .. } | Op::MulI { .. } | Op::DivI { .. }
            | Op::ModI { .. } | Op::WAddI { .. } | Op::WSubI { .. } | Op::WMulI { .. }
            | Op::WDivI { .. } | Op::WModI { .. } | Op::AndI { .. } | Op::OrI { .. }
            | Op::XorI { .. } | Op::ShlI { .. } | Op::ShrI { .. } | Op::WrapShlI { .. }
            | Op::EqI { .. } | Op::NeI { .. } | Op::LtI { .. } | Op::GtI { .. }
            | Op::LeI { .. } | Op::GeI { .. } | Op::NegI { .. } => {
                unreachable!("specialized scalar ops are handled in the run_loop fast path")
            }
            Op::StrCmp { eq, dst, a, b } => {
                let sa = cell_of(r!(a)).as_str();
                let sb = cell_of(r!(b)).as_str();
                self.cur_regs[dst as usize] = Slot::bool((sa == sb) == eq);
            }
            Op::BytesCmp { eq, dst, a, b } => {
                let ba = cell_of(r!(a)).as_bytes();
                let bb = cell_of(r!(b)).as_bytes();
                self.cur_regs[dst as usize] = Slot::bool((ba == bb) == eq);
            }
            Op::RefEq { eq, dst, a, b } => {
                let v = Slot::same_ref(r!(a), r!(b)) == eq;
                self.cur_regs[dst as usize] = Slot::bool(v);
            }

            Op::Jmp { target } => self.cur_pc = target,
            Op::Br { cond, then_t, else_t } => {
                self.cur_pc = if r!(cond).as_bool() { then_t } else { else_t };
            }
            Op::BrTable { idx, table, default } => {
                let cell = cell_of(r!(idx));
                let m = cell.as_enum_member().unwrap_or(u32::MAX) as usize;
                self.cur_pc = table.get(m).copied().unwrap_or(default);
            }

            Op::Call { func, args, dst } => self.op_call(func, &args, dst),
            Op::CallM { func, recv, args, dst } => self.op_call_m(func, recv, &args, dst),
            Op::CallI { slot, recv, args, dst } => self.op_call_i(slot, recv, &args, dst)?,
            Op::CallFn { fval, args, dst } => self.op_call_fn(fval, &args, dst)?,
            Op::CallNat { nat, recv, args, dst } => self.call_nat(nat, recv, &args, dst)?,
            // handled in run_loop (root returns surface the run's value)
            Op::Ret { .. } => unreachable!("Op::Ret is handled by the run loop"),
            Op::NewCell { dst, ty } => {
                let n = match self.prog.types.kind(ty) {
                    TyKind::Data { fields } => fields.len(),
                    _ => 0,
                };
                let c = self.heap.alloc_record_zeroed(ty, n)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }
            Op::MakeRecord { dst, ty, vals } => self.op_make_record(dst, ty, &vals)?,
            Op::GetF { dst, obj, field, repr } => self.op_getf(dst, obj, field, repr)?,
            Op::SetF { obj, field, val, repr } => self.op_setf(obj, field, val, repr)?,
            Op::Own { dst, src, ty } => {
                let v = self.heap.own(r!(src), ty, &self.prog.types)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = v;
                if self.is_ref(ty) {
                    self.heap.release(old);
                }
            }

            Op::ArrNew { dst, ty, len, repr } => {
                let elem = match self.prog.types.kind(ty) {
                    TyKind::Vec { elem } => *elem,
                    _ => return Err(Trap::new(TrapKind::Invalid, "arrnew on non-vec")),
                };
                let n = unsafe { r!(len).i }.max(0) as usize;
                let c = self.heap.alloc_vec(elem, n, &self.prog.types)?;
                if let crate::heap::CellData::Vec { items, .. } = &cell_of(c).data {
                    let mut fb = items.borrow_mut();
                    let dflt = default_slot_repr(repr);
                    for _ in 0..n {
                        fb.push(dflt);
                    }
                }
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }
            Op::ArrLit { dst, ty, elems } => self.op_arr_lit(dst, ty, &elems)?,
            Op::ArrGet { dst, arr, idx, repr } => self.op_arr_get(dst, arr, idx, repr)?,
            Op::ArrSet { arr, idx, val, repr } => self.op_arr_set(arr, idx, val, repr)?,

            Op::EnumNew { dst, ty, member } => {
                let c = self.heap.enum_member(ty, member)?;
                // the singleton slot is BORROWED (enum_member leaks the
                // base references) — the destination register takes an
                // OWNED reference, like every other ref-typed store, or
                // frame teardown over-releases and frees the "immortal"
                // cell while the singleton map still points at it
                self.heap.retain(c);
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }
            Op::OptSome { dst, ty, val } => {
                self.alloc_sum(dst, ty, 0, Some(r!(val)))?;
            }
            Op::OptNone { dst, ty } => {
                self.alloc_sum(dst, ty, 1, None)?;
            }
            Op::ResOk { dst, ty, val } => {
                self.alloc_sum(dst, ty, 0, Some(r!(val)))?;
            }
            Op::ResErr { dst, ty, val } => {
                self.alloc_sum(dst, ty, 1, Some(r!(val)))?;
            }
            Op::SumIs { dst, v, want_err } => {
                let tag = sum_tag(cell_of(r!(v)))?;
                self.cur_regs[dst as usize] = Slot::bool((tag == 1) == want_err);
            }
            Op::Unwrap { dst, v, want_err } => {
                let cell = cell_of(r!(v));
                let (tag, payload) = sum_parts(cell)?;
                if (tag == 1) != want_err {
                    return Err(Trap::new(
                        TrapKind::UnwrapNone,
                        if want_err {
                            "`.error` on Ok"
                        } else {
                            "`.value` on None/Err — the value is absent (RFC 0005)"
                        },
                    ));
                }
                let val = payload.unwrap_or(Slot::int(0));
                self.move_sum_val(dst, val, self.sum_payload_ty(cell.ty, want_err));
            }
            Op::UnwrapOr { dst, v, default } => {
                let cell = cell_of(r!(v));
                let (tag, payload) = sum_parts(cell)?;
                let val = if tag == 0 {
                    payload.unwrap_or(Slot::int(0))
                } else {
                    r!(default)
                };
                self.move_sum_val(dst, val, self.sum_payload_ty(cell.ty, false));
            }
            Op::Expect { dst, v, msg } => {
                let cell = cell_of(r!(v));
                let (tag, payload) = sum_parts(cell)?;
                if tag == 1 {
                    let m = cell_of(r!(msg)).as_str().to_string();
                    return Err(Trap::new(TrapKind::UnwrapNone, format!("expect failed: {m}")));
                }
                let val = payload.unwrap_or(Slot::int(0));
                self.move_sum_val(dst, val, self.sum_payload_ty(cell.ty, false));
            }

            Op::TidOf { dst, obj } => {
                let cell = cell_of(r!(obj));
                let ty = self.effective_ty(cell);
                self.cur_regs[dst as usize] = Slot::int(ty as i64);
            }
            Op::IsType { dst, obj, want } => {
                let cell = cell_of(r!(obj));
                let ty = self.effective_ty(cell);
                self.cur_regs[dst as usize] = Slot::bool(ty == want);
            }
            Op::IsTrait { dst, obj, want } => {
                let cell = cell_of(r!(obj));
                let ty = self.effective_ty(cell);
                let has = self
                    .prog
                    .trait_slots
                    .iter()
                    .enumerate()
                    .any(|(slot, &(t, _))| {
                        t == want
                            && self
                                .prog
                                .vtables
                                .get(ty as usize)
                                .and_then(|v| v.get(slot))
                                .is_some_and(|f| f.is_some())
                    });
                self.cur_regs[dst as usize] = Slot::bool(has);
            }
            Op::Unbox { dst, box_, ty } => {
                let Some((val, val_ty)) = cell_of(r!(box_)).as_opaque() else {
                    return Err(Trap::new(TrapKind::BadUnbox, "unbox on non-opaque"));
                };
                if val_ty != ty {
                    return Err(Trap::new(
                        TrapKind::BadUnbox,
                        "unbox type mismatch (the compiler guards this; the trap is the safety net)",
                    ));
                }
                self.move_sum_val(dst, val, ty);
            }
            Op::Box { dst, val, ty } => {
                let v = r!(val);
                if self.is_ref(ty) {
                    self.heap.retain(v);
                }
                let c = self.heap.alloc_opaque(v, ty)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }

            Op::MakeClosure { dst, func, captures } => {
                let caps: Vec<Slot> = captures.iter().map(|&c| r!(c)).collect();
                // capture types are the tail of the callee's params; read them
                // from the function table so the cell need not store them
                let nparams = self.prog.funcs[func as usize].params.len();
                let cap_tys: Vec<TypeId> = (nparams - caps.len()..nparams)
                    .map(|i| self.param_ty(func, i))
                    .collect();
                // captures retained into the cell
                for (&c, &t) in caps.iter().zip(cap_tys.iter()) {
                    if self.is_ref(t) {
                        self.heap.retain(c);
                    }
                }
                let c = self.heap.alloc_closure(func, caps)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }

            Op::Panic { msg } => {
                let m = cell_of(r!(msg)).as_str().to_string();
                return Err(Trap::new(TrapKind::Panic, m));
            }
            Op::Assert { cond, msg } => {
                if !r!(cond).as_bool() {
                    let m = msg
                        .map(|m| cell_of(r!(m)).as_str().to_string())
                        .unwrap_or_else(|| "assertion failed".to_string());
                    return Err(Trap::new(TrapKind::Assert, m));
                }
            }
            Op::Conv { dst, src, from, to } => {
                let v = self.convert(r!(src), from, to)?;
                self.cur_regs[dst as usize] = v;
            }
            Op::StrCharAt { dst, s, idx } => {
                let str_cell = cell_of(r!(s));
                let i = unsafe { r!(idx).i };
                let c = str_cell
                    .as_str()
                    .chars()
                    .nth(i as usize)
                    .ok_or_else(|| {
                        Trap::new(
                            TrapKind::IndexOutOfBounds,
                            format!("string index {i} out of bounds ({} chars)", str_cell.as_str().chars().count()),
                        )
                    })?;
                self.cur_regs[dst as usize] = Slot::ch(c);
            }
            Op::BytesGet { dst, s, idx } => {
                let cell = cell_of(r!(s));
                let i = unsafe { r!(idx).i };
                let b = cell.as_bytes().get(i as usize).copied().ok_or_else(|| {
                    Trap::new(
                        TrapKind::IndexOutOfBounds,
                        format!("bytes index {i} out of bounds (len {})", cell.as_bytes().len()),
                    )
                })?;
                self.cur_regs[dst as usize] = Slot::int(b as i64);
            }
        }
        Ok(())
    }
}
