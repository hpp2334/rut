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
            #[allow(unreachable_patterns)]
            Op::Pad { .. } => unreachable!("layout pin, never constructed"),
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
            // threaded scalar ops are handled by rut-vm-threaded; they never
            // reach this match
            Op::AddI { .. } | Op::SubI { .. } | Op::WAddI { .. } | Op::WSubI { .. }
            | Op::WMulI { .. } | Op::LtI { .. }
            | Op::AddF { .. } | Op::SubF { .. } | Op::MulF { .. } => {
                unreachable!("threaded scalar ops are handled by rut-vm-threaded")
            }
            // remaining scalar ops run here (the `T_SLOW` path)
            Op::MulI { prim, dst, a, b } => {
                let v = self.arith_int::<{ IOP_MUL }, false>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::DivI { prim, dst, a, b } => {
                let v = self.arith_int::<{ IOP_DIV }, false>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::ModI { prim, dst, a, b } => {
                let v = self.arith_int::<{ IOP_MOD }, false>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::AndI { prim, dst, a, b } => {
                let v = self.bitop_int::<{ BOP_AND }>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::OrI { prim, dst, a, b } => {
                let v = self.bitop_int::<{ BOP_OR }>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::XorI { prim, dst, a, b } => {
                let v = self.bitop_int::<{ BOP_XOR }>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::ShlI { prim, dst, a, b } => {
                let v = self.bitop_int::<{ BOP_SHL }>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::ShrI { prim, dst, a, b } => {
                let v = self.bitop_int::<{ BOP_SHR }>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::WrapShlI { prim, dst, a, b } => {
                let v = self.bitop_int::<{ BOP_WRAPSHL }>(prim, r!(a), r!(b))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::EqI { prim, dst, a, b } => {
                let v = self.cmp_int::<{ COP_EQ }>(prim, r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::NeI { prim, dst, a, b } => {
                let v = self.cmp_int::<{ COP_NE }>(prim, r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::GtI { prim, dst, a, b } => {
                let v = self.cmp_int::<{ COP_GT }>(prim, r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::LeI { prim, dst, a, b } => {
                let v = self.cmp_int::<{ COP_LE }>(prim, r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::GeI { prim, dst, a, b } => {
                let v = self.cmp_int::<{ COP_GE }>(prim, r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::NegI { prim, dst, a } => {
                let v = self.neg_int(prim, r!(a))?;
                self.cur_regs[dst as usize] = v;
            }
            Op::DivF { prim, dst, a, b } => {
                self.cur_regs[dst as usize] = self.arith_float::<{ FOP_DIV }>(prim, r!(a), r!(b));
            }
            Op::ModF { prim, dst, a, b } => {
                self.cur_regs[dst as usize] = self.arith_float::<{ FOP_MOD }>(prim, r!(a), r!(b));
            }
            Op::NegF { prim, dst, a } => {
                self.cur_regs[dst as usize] = self.neg_float(prim, r!(a));
            }
            Op::EqF { dst, a, b } => {
                let v = self.cmp_float::<{ COP_EQ }>(r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::LtF { dst, a, b } => {
                let v = self.cmp_float::<{ COP_LT }>(r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::NeF { dst, a, b } => {
                let v = self.cmp_float::<{ COP_NE }>(r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::GtF { dst, a, b } => {
                let v = self.cmp_float::<{ COP_GT }>(r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::LeF { dst, a, b } => {
                let v = self.cmp_float::<{ COP_LE }>(r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::GeF { dst, a, b } => {
                let v = self.cmp_float::<{ COP_GE }>(r!(a), r!(b));
                self.cur_regs[dst as usize] = Slot::bool(v);
            }
            Op::StrCmp { eq, dst, a, b } => {
                // UTF-8 byte equality IS string equality
                let sa = cell_of(r!(a)).as_bytes();
                let sb = cell_of(r!(b)).as_bytes();
                self.cur_regs[dst as usize] = Slot::bool((sa == sb) == eq);
            }
            Op::ArrayCmp { eq, dst, a, b } => {
                let same = cell_of(r!(a)).array_eq(cell_of(r!(b)));
                self.cur_regs[dst as usize] = Slot::bool(same == eq);
            }
            Op::RefEq { eq, dst, a, b } => {
                let v = Slot::same_ref(r!(a), r!(b)) == eq;
                self.cur_regs[dst as usize] = Slot::bool(v);
            }

            Op::ValEq { dst, a, b, ty, eq } => {
                let same = self.vals_equal(r!(a), r!(b), ty)?;
                self.cur_regs[dst as usize] = Slot::bool(same == eq);
            }


            Op::Jmp { target } => self.cur_pc = target,
            Op::Br { cond, then_t, else_t } => {
                self.cur_pc = if r!(cond).as_bool() { then_t } else { else_t };
            }
            Op::BrTable { idx, table_off, count, default } => {
                let prog = Rc::clone(&self.prog);
                let table = &prog.funcs[self.cur_func as usize].labels
                    [table_off as usize..table_off as usize + count as usize];
                let cell = cell_of(r!(idx));
                let m = cell.as_enum_member().unwrap_or(u32::MAX) as usize;
                self.cur_pc = table.get(m).copied().unwrap_or(default);
            }

            Op::Call { func, argv_off, argc, dst } => self.op_call(func, argv_off, argc, dst),
            Op::CallM { func, argv_off, argc, dst } => self.op_call(func, argv_off, argc, dst),
            Op::CallI { slot, argv_off, argc, dst } => self.op_call_i(slot, argv_off, argc, dst)?,
            Op::CallFn { fval, argv_off, argc, dst } => self.op_call_fn(fval, argv_off, argc, dst)?,
            Op::CallNat { nat, recv, argv_off, argc, dst } => self.call_nat(nat, recv, argv_off, argc, dst)?,
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
            Op::MakeRecord { dst, ty, argv_off, argc } => self.op_make_record(dst, ty, argv_off, argc)?,
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

            Op::MakePtr { dst, src, ty } => self.op_make_ptr(dst, src, ty)?,
            Op::OnDrop { obj, cleanup } => self.op_on_drop(obj, cleanup)?,
            Op::CloneVal { dst, src, ty } => self.op_clone_val(dst, src, ty)?,

            Op::ArrNew { dst, ty, len, repr } => {
                let elem = match self.prog.types.kind(ty) {
                    TyKind::Array { elem } => *elem,
                    TyKind::Bytes => rut_core::types::TY_U8,
                    _ => return Err(Trap::new(TrapKind::Invalid, "arrnew on non-array")),
                };
                let n = unsafe { r!(len).i }.max(0) as usize;
                let c = self.heap.alloc_array_filled(elem, n, default_slot_repr(repr), &self.prog.types)?;
                let old = self.cur_regs[dst as usize];
                self.cur_regs[dst as usize] = c;
                self.heap.release(old);
            }
            Op::ArrLit { dst, ty, argv_off, argc } => self.op_arr_lit(dst, ty, argv_off, argc)?,
            Op::ArrGet { dst, arr, idx, repr } => self.op_arr_get(dst, arr, idx, repr)?,
            Op::ArrSet { arr, idx, val, repr } => self.op_arr_set(arr, idx, val, repr)?,
            Op::ArrGetF { dst, obj, field, idx, repr } => self.op_arr_get_f(dst, obj, field, idx, repr)?,
            Op::ArrGetRef { dst, arr, idx, ty } => self.op_arr_get_ref(dst, arr, idx, ty)?,
            Op::ArrSetF { obj, field, idx, val, repr } => self.op_arr_set_f(obj, field, idx, val, repr)?,

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
            Op::TidOf { dst, obj } => {
                let cell = cell_of(r!(obj));
                // a host payload box has no rut runtime type (RFC 0023):
                // report the sentinel so `downcast<T>` compares false for
                // every T and yields None — never a trap (RFC 0014)
                let ty = if matches!(cell.data, CellData::HostBoxed { .. }) {
                    rut_core::types::HOST_BOX_TID
                } else {
                    self.effective_ty(cell)
                };
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

            Op::MakeClosure { dst, func, argv_off, argc } => {
                let prog = Rc::clone(&self.prog);
                let captures = self.cur_argv(&prog, argv_off, argc);
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
                let c = if str_cell.str_ascii() {
                    match str_cell.as_bytes().get(i as usize) {
                        Some(b) => *b as char,
                        None => {
                            return Err(Trap::new(
                                TrapKind::IndexOutOfBounds,
                                format!(
                                    "string index {i} out of bounds ({} bytes)",
                                    str_cell.as_bytes().len()
                                ),
                            ))
                        }
                    }
                } else {
                    let text = str_cell.as_str();
                    text.chars().nth(i as usize).ok_or_else(|| {
                        Trap::new(
                            TrapKind::IndexOutOfBounds,
                            format!("string index {i} out of bounds ({} chars)", text.chars().count()),
                        )
                    })?
                };
                self.cur_regs[dst as usize] = Slot::ch(c);
            }
        }
        Ok(())
    }
}
