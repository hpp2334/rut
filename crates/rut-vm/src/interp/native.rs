//! `CallNat` natives (RFC 0032 §1.1 R2) and per-type `render` (RFC 0007 §2).
use super::*;

impl Vm {
    // ---- natives (RFC 0032 §1.1 R2: named things are natives, not ops) ----

    pub(super) fn call_nat(&mut self, nat: Nat, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        macro_rules! r {
            ($i:expr) => {
                self.cur_regs[$i as usize]
            };
        }
        match nat {
            Nat::Print => {
                let s = cell_of(r!(args[0])).as_str().to_string();
                if let Some(h) = &self.hooks.print {
                    (h.borrow_mut())(&s);
                }
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(0);
                }
            }
            Nat::Str => {
                let v = r!(args[0]);
                // a string already formats to itself — alias the cell instead
                // of re-rendering a copy (the `f"{s}"` identity; this is the
                // bulk of the cost in string-churn workloads like fasta).
                // Gate on the *static* type: the slot of a non-string arg is
                // not a cell handle, so `cell_of` must not touch it.
                if matches!(self.prog.types.kind(self.regs_ty(args[0])), TyKind::Str) {
                    if let Some(d) = dst {
                        self.heap.retain(v);
                        let old = self.cur_regs[d as usize];
                        self.cur_regs[d as usize] = v;
                        self.heap.release(old);
                    }
                    return Ok(());
                }
                let s = self.render(v, args[0])?;
                let c = self.heap.alloc_str(s)?;
                self.store_result(dst, c)?;
            }
            Nat::Concat => {
                // size once — the parts' lengths are all known
                let total: usize = args.iter().map(|a| cell_of(r!(*a)).as_str().len()).sum();
                let mut out = String::with_capacity(total);
                for a in args {
                    out.push_str(cell_of(r!(*a)).as_str());
                }
                let c = self.heap.alloc_str(out)?;
                self.store_result(dst, c)?;
            }
            Nat::StrLen => {
                let n = cell_of(r!(recv.unwrap())).as_str().chars().count() as i64;
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(n);
                }
            }
            Nat::ArrLen => {
                let cell = cell_of(r!(recv.unwrap()));
                let n = match &cell.data {
                    crate::heap::CellData::Array { items, .. } => items.borrow().len() as i64,
                    _ => return Err(Trap::new(TrapKind::Invalid, "len on non-sequence")),
                };
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(n);
                }
            }
            Nat::StrEncode => {
                let bytes = cell_of(r!(recv.unwrap())).as_str().to_string().into_bytes();
                let c = self.heap.alloc_bytes(bytes)?;
                self.store_result(dst, c)?;
            }
            Nat::BytesDecode => {
                let s = String::from_utf8_lossy(&cell_of(r!(recv.unwrap())).bytes_copy()).into_owned();
                let c = self.heap.alloc_str(s)?;
                self.store_result(dst, c)?;
            }
        }
        Ok(())
    }
    /// Per-type formatting — the RFC 0007 §2 table. The register's static
    /// type says how to read the slot.
    pub(super) fn render(&self, v: Slot, reg: Reg) -> Result<String, Trap> {
        let ty = self.regs_ty(reg);
        match self.prog.types.kind(ty) {
            TyKind::Prim(PrimTy::F32) => Ok(format!("{}", unsafe { v.f } as f32)),
            TyKind::Prim(PrimTy::F64) => Ok(format!("{}", unsafe { v.f })),
            TyKind::Prim(PrimTy::Bool) => Ok(if v.as_bool() { "true".into() } else { "false".into() }),
            TyKind::Prim(PrimTy::Char) => Ok(v.as_char().to_string()),
            TyKind::Prim(_) => Ok(unsafe { v.i }.to_string()),
            TyKind::Str => Ok(cell_of(v).as_str().to_string()),
            TyKind::Enum { members } => {
                let Some(member) = cell_of(v).as_enum_member() else {
                    return Err(Trap::new(TrapKind::Invalid, "render on non-enum"));
                };
                Ok(members[member as usize].0.clone())
            }
            _ => Err(Trap::new(
                TrapKind::Invalid,
                "this type is not formattable inside f\"...\" (RFC 0007 §2)",
            )),
        }
    }
}
