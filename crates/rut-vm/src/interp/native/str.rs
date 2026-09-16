//! String natives (RFC 0032 §1.1 R2): `str`/`concat` (the `f""`
//! desugaring, RFC 0007 §2), `string_len` (the char count, RFC 0008), the
//! host print sink, and per-type `render`.
use super::*;

impl Vm {
    pub(super) fn call_str_nat(&mut self, nat: Nat, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        match nat {
            Nat::Str => {
                let v = self.reg(args[0]);
                let ty = self.regs_ty(args[0]);
                // a char renders by encoding straight into a fresh block —
                // no intermediate `String` (the f-string `{c}` churn path:
                // string-building loops hit this once per piece)
                if matches!(self.prog.types.kind(ty), TyKind::Prim(PrimTy::Char)) {
                    let c = char::from_u32(unsafe { v.i } as u32).unwrap_or('\u{FFFD}');
                    let c = self.heap.alloc_char(c)?;
                    self.store_result(dst, c)?;
                    return Ok(());
                }
                // a string already formats to itself — alias the cell instead
                // of re-rendering a copy (the `f"{s}"` identity; this is the
                // bulk of the cost in string-churn workloads like fasta).
                // Gate on the *static* type: the slot of a non-string arg is
                // not a cell handle, so `cell_of` must not touch it.
                if matches!(self.prog.types.kind(ty), TyKind::Str) {
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
                    let target = self.reg(d);
                    let tref = unsafe { target.r };
                    if d == args[0]
                        && !tref.is_null()
                        && cell_of(target).refs.get() == 1
                        // an owned cell only: appending through a VIEW
                        // would write into its parent (RFC 0042)
                        && matches!(&cell_of(target).data, CellData::Str(_))
                        && !args[1..].iter().any(|a| unsafe { self.reg(*a).r } == tref)
                    {
                        // one growth decision for ALL parts, then cheap
                        // appends — the amortized O(1) accumulator path
                        let total: usize =
                            args[1..].iter().map(|a| cell_of(self.reg(*a)).as_bytes().len()).sum();
                        self.heap.reserve_append(target, total)?;
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
            Nat::StrSlice => {
                // s.slice(from, to) — an O(1) window (RFC 0042): codepoint
                // bounds here, byte offsets inside. The view retains the
                // root owned str; view-of-view flattens onto the root.
                let s = self.reg(recv.unwrap());
                let from = unsafe { self.reg(args[0]).i };
                let to = unsafe { self.reg(args[1]).i };
                let cell = cell_of(s);
                if !matches!(&cell.data, CellData::Str(_) | CellData::StrView { .. }) {
                    return Err(Trap::new(TrapKind::Invalid, "slice on non-string"));
                }
                let clen = cell.char_len() as i64;
                if from < 0 || to < from || to > clen {
                    return Err(Trap::new(
                        TrapKind::IndexOutOfBounds,
                        format!("slice {from}..{to} out of bounds (len {clen})"),
                    ));
                }
                // flatten onto the root owned str
                let mut parent = s;
                let mut base = 0u32;
                let root_ascii = loop {
                    let c = cell_of(parent);
                    match &c.data {
                        CellData::StrView { parent: p, off, .. } => {
                            base += *off;
                            parent = *p;
                        }
                        CellData::Str(v) => break v.ascii,
                        _ => return Err(Trap::new(TrapKind::Invalid, "slice on non-string")),
                    }
                };
                // codepoint bounds -> byte offsets within the window
                let vbytes = cell.as_bytes();
                let (bfrom, bto) = if cell.str_ascii() {
                    (from as usize, to as usize)
                } else {
                    let text = cell.as_str();
                    (
                        text.char_indices().nth(from as usize).map(|(k, _)| k).unwrap_or(text.len()),
                        text.char_indices().nth(to as usize).map(|(k, _)| k).unwrap_or(text.len()),
                    )
                };
                let c = self.heap.alloc_str_view(
                    parent,
                    base + bfrom as u32,
                    (bto - bfrom) as u32,
                    root_ascii,
                )?;
                self.store_result(dst, c)?;
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
