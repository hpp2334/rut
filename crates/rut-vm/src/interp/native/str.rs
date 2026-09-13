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
                // `dst` IS the first part and that cell is uniquely owned:
                // append in place rather than copying the growing prefix.
                // The `bytes_decode` lowering is `out = out + c` per code
                // point, so copying would be quadratic. `rc == 1` means no
                // other slot aliases the cell; a later part must not either.
                if let Some(d) = dst {
                    let target = self.reg(d);
                    let tref = unsafe { target.r };
                    if d == args[0]
                        && !tref.is_null()
                        && cell_of(target).refs.get() == 1
                        && !args[1..].iter().any(|a| unsafe { self.reg(*a).r } == tref)
                    {
                        for a in &args[1..] {
                            let extra = cell_of(self.reg(*a)).as_bytes();
                            self.heap.append_bytes(target, extra)?;
                        }
                        return Ok(());
                    }
                }
                // size once — the parts' byte lengths are all known
                let total: usize = args.iter().map(|a| cell_of(self.reg(*a)).as_bytes().len()).sum();
                let mut out = Vec::with_capacity(total);
                for a in args {
                    out.extend_from_slice(cell_of(self.reg(*a)).as_bytes());
                }
                let c = self.heap.alloc_str_bytes(out)?;
                self.store_result(dst, c)?;
            }
            Nat::StrLen => {
                // ASCII is the common case: the flag is cached on the cell,
                // so `len` is the byte length with no UTF-8 walk at all.
                let n = cell_of(self.reg(recv.unwrap())).char_len() as i64;
                if let Some(d) = dst {
                    self.cur_regs[d as usize] = Slot::int(n);
                }
            }
            Nat::StrJoin => {
                // join every element of an `Array<str>`: one sizing pass,
                // then one copy into a single allocation (RFC 0032 §1.1 R2).
                let arr = self.reg(args[0]);
                let total = match &cell_of(arr).data {
                    crate::heap::CellData::Array { items, .. } => {
                        let items = items.borrow();
                        let mut n = 0usize;
                        for i in 0..items.len() {
                            if let Some(s) = items.get(i) {
                                n += cell_of(s).as_str().len();
                            }
                        }
                        n
                    }
                    _ => return Err(Trap::new(TrapKind::Invalid, "string_join on non-array")),
                };
                let mut out = Vec::with_capacity(total);
                if let crate::heap::CellData::Array { items, .. } = &cell_of(arr).data {
                    let items = items.borrow();
                    for i in 0..items.len() {
                        if let Some(s) = items.get(i) {
                            out.extend_from_slice(cell_of(s).as_bytes());
                        }
                    }
                }
                let c = self.heap.alloc_str_bytes(out)?;
                self.store_result(dst, c)?;
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
