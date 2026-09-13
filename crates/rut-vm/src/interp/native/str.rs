//! String natives (RFC 0032 §1.1 R2): `str`/`concat` (the `f""`
//! desugaring, RFC 0007 §2), `string_len` (the char count, RFC 0008), the
//! host print sink, and per-type `render`.
use super::*;

impl Vm {
    pub(super) fn call_str_nat(&mut self, nat: Nat, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        match nat {
            Nat::Str => {
                let v = self.reg(args[0]);
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
                // a one-part concat is the identity — alias the existing cell
                // instead of allocating a copy of it.
                if args.len() == 1 {
                    let v = self.reg(args[0]);
                    if let Some(d) = dst {
                        self.heap.retain(v);
                        let old = self.cur_regs[d as usize];
                        self.cur_regs[d as usize] = v;
                        self.heap.release(old);
                    }
                    return Ok(());
                }
                // size once — the parts' lengths are all known
                let total: usize = args.iter().map(|a| cell_of(self.reg(*a)).as_str().len()).sum();
                let mut out = String::with_capacity(total);
                for a in args {
                    out.push_str(cell_of(self.reg(*a)).as_str());
                }
                let c = self.heap.alloc_str(out)?;
                self.store_result(dst, c)?;
            }
            Nat::StrLen => {
                let s = cell_of(self.reg(recv.unwrap())).as_str();
                // ASCII is the common case: `len` skips the UTF-8 decode that
                // `chars().count()` walks.
                let n = if s.is_ascii() { s.len() } else { s.chars().count() } as i64;
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(n);
                }
            }
            _ => unreachable!("call_str_nat: non-string native"),
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
            TyKind::Prim(p) => Ok(match p {
                // unsigned widths must format unsigned — the slot is an i64,
                // so a u64 with bit 63 set would otherwise print negative
                PrimTy::U8 => (unsafe { v.i } as u8).to_string(),
                PrimTy::U16 => (unsafe { v.i } as u16).to_string(),
                PrimTy::U32 => (unsafe { v.i } as u32).to_string(),
                PrimTy::U64 => (unsafe { v.i } as u64).to_string(),
                _ => unsafe { v.i }.to_string(),
            }),
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
