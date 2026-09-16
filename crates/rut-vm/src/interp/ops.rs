//! Object/array/sum/call op bodies shared by `run_loop` and `step`.
use super::*;

impl Vm {
    /// move a sum payload into dst with ref discipline
    pub(super) fn move_sum_val(&mut self, dst: Reg, val: Slot, ty: TypeId) {
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = val;
        if self.is_ref(ty) {
            self.heap.retain(val);
            self.heap.release(old);
        }
    }

    pub(super) fn effective_ty(&self, cell: &crate::heap::CellVal) -> TypeId {
        match &cell.data {
            crate::heap::CellData::OpaqueBox { val_ty, .. } => *val_ty,
            // a host payload box's rut type is the box itself (RFC 0023):
            // `o is Opaque` is true, `o is T` misses for every rut T
            crate::heap::CellData::HostBoxed { .. } => cell.ty,
            _ => cell.ty,
        }
    }

    // ---- op bodies shared by `step` and the `run_loop` fast path ----

    /// `ArrGet` — the fast path calls this directly, so it must stay small
    /// and inlinable.
    #[inline(always)]
    pub(super) fn op_arr_get(&mut self, dst: Reg, arr: Reg, idx: Reg, repr: Repr) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let cell = cell_of(self.cur_regs[arr as usize]);
        let v = seq_get(cell, i)?;
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = v;
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `ArrSet` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_arr_set(&mut self, arr: Reg, idx: Reg, val: Reg, repr: Repr) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let cell = cell_of(self.cur_regs[arr as usize]);
        let v = self.cur_regs[val as usize];
        let old = seq_set(cell, i, v)?;
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `ArrGetF` — fused `obj.field[idx]`; the field handle is borrowed, not
    /// retained (the owner record keeps the array alive).
    #[inline(always)]
    pub(super) fn op_arr_get_f(&mut self, dst: Reg, obj: Reg, field: u32, idx: Reg, repr: Repr) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let arr = {
            let obj_cell = cell_of(self.cur_regs[obj as usize]);
            match &obj_cell.data {
                CellData::Record { fields } => fields
                    .borrow()
                    .get(field as usize)
                    .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?,
                _ => return Err(Trap::new(TrapKind::Invalid, "field-array get on non-record")),
            }
        };
        let v = seq_get(cell_of(arr), i)?;
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = v;
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `ArrSetF` — fused `obj.field[idx] = val`; field borrowed.
    #[inline(always)]
    pub(super) fn op_arr_set_f(&mut self, obj: Reg, field: u32, idx: Reg, val: Reg, repr: Repr) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let v = self.cur_regs[val as usize];
        let arr = {
            let obj_cell = cell_of(self.cur_regs[obj as usize]);
            match &obj_cell.data {
                CellData::Record { fields } => fields
                    .borrow()
                    .get(field as usize)
                    .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?,
                _ => return Err(Trap::new(TrapKind::Invalid, "field-array set on non-record")),
            }
        };
        let old = seq_set(cell_of(arr), i, v)?;
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `GetF` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_getf(&mut self, dst: Reg, obj: Reg, field: u32, repr: Repr) -> Result<(), Trap> {
        let cell = cell_of(self.nil_checked(self.cur_regs[obj as usize])?);
        let v = match &cell.data {
            CellData::Record { fields } => fields
                .borrow()
                .get(field as usize)
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?,
            _ => return Err(Trap::new(TrapKind::Invalid, "field on non-record")),
        };
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = v;
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `SetF` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_setf(&mut self, obj: Reg, field: u32, val: Reg, repr: Repr) -> Result<(), Trap> {
        let cell = cell_of(self.nil_checked(self.cur_regs[obj as usize])?);
        let v = self.cur_regs[val as usize];
        let old = if let CellData::Record { fields } = &cell.data {
            fields
                .borrow_mut()
                .set(field as usize, v)
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?
        } else {
            return Err(Trap::new(TrapKind::Invalid, "field-set on non-record"));
        };
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `MakePtr` — box `v` into a fresh one-slot cell (RFC 0005). A value
    /// payload deep-copies (RFC 0009/0016 v1.1); ref payloads (str/bytes/
    /// `*T`/closures) share. `ty` is the pointer's own type.
    #[inline(always)]
    pub(super) fn op_make_ptr(&mut self, dst: Reg, src: Reg, ty: TypeId) -> Result<(), Trap> {
        let raw = self.cur_regs[src as usize];
        let elem = match self.prog.types.kind(ty) {
            TyKind::Ptr { elem } => *elem,
            _ => unreachable!("MakePtr on a non-pointer type"),
        };
        let v = if self.prog.types.is_value(elem) {
            self.heap.clone_val(raw, elem, &self.prog.types)?
        } else {
            self.heap.retain(raw);
            raw
        };
        let c = self.heap.alloc_record_zeroed(ty, 1)?;
        if let CellData::Record { fields } = &cell_of(c).data {
            fields.borrow_mut().set(0, v);
        }
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = c;
        if self.is_ref(ty) {
            self.heap.release(old);
        }
        Ok(())
    }

    /// `ArrGetRef` — `for (let v of xs)` element reference (RFC 0012 §6):
    /// box the element into a fresh one-slot cell of the pointer type.
    /// Ref-typed elements alias the stored slot (writes through `v.f` hit
    /// the sequence); scalars box a per-iteration copy, which a scalar
    /// cannot be written through anyway.
    #[inline(always)]
    pub(super) fn op_arr_get_ref(&mut self, dst: Reg, arr: Reg, idx: Reg, ty: TypeId) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let v = seq_get(cell_of(self.cur_regs[arr as usize]), i)?;
        let elem = match self.prog.types.kind(ty) {
            TyKind::Ptr { elem } => *elem,
            _ => unreachable!("ArrGetRef over a non-pointer type"),
        };
        let c = self.heap.alloc_record_zeroed(ty, 1)?;
        if let CellData::Record { fields } = &cell_of(c).data {
            fields.borrow_mut().set(0, v);
        }
        if self.prog.types.repr_of(elem).is_ref() {
            self.heap.retain(v);
        }
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = c;
        self.heap.release(old); // the pointer type is always a ref
        Ok(())
    }

    /// `OnDrop` — attach a cleanup closure to a cell (RFC 0016 §3).
    #[inline(always)]
    pub(super) fn op_on_drop(&mut self, obj: Reg, cleanup: Reg) -> Result<(), Trap> {
        self.heap
            .set_drop_fn(self.cur_regs[obj as usize], self.cur_regs[cleanup as usize])
    }

    /// Structural equality on values (RFC 0009/0016 v1.1): records and
    /// arrays compare field-by-field / element-by-element recursively,
    /// `str`/`bytes` by content, `*T` and boundary objects by identity.
    /// Value graphs are acyclic (only `*T`/closures close cycles and both
    /// compare by identity), so the recursion terminates.
    pub(super) fn vals_equal(&self, a: Slot, b: Slot, ty: TypeId) -> Result<bool, Trap> {
        if unsafe { a.i == b.i } {
            return Ok(true); // same cell or same scalar bits
        }
        match self.prog.types.kind(ty).clone() {
            TyKind::Data { fields } => {
                let ca = cell_of(a);
                let cb = cell_of(b);
                let (fa, fb) = match (&ca.data, &cb.data) {
                    (CellData::Record { fields: fa }, CellData::Record { fields: fb }) => (fa.borrow(), fb.borrow()),
                    _ => return Err(Trap::new(TrapKind::Invalid, "value compare on a non-record")),
                };
                for (i, f) in fields.iter().enumerate() {
                    let xa = fa.get(i).unwrap_or_else(Slot::null);
                    let xb = fb.get(i).unwrap_or_else(Slot::null);
                    if !self.vals_equal(xa, xb, f.ty)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            TyKind::Array { elem } => {
                let ca = cell_of(a);
                let cb = cell_of(b);
                let (ia, ib) = match (&ca.data, &cb.data) {
                    (CellData::Array { items: ia, elem: _ }, CellData::Array { items: ib, .. }) => (ia.borrow(), ib.borrow()),
                    _ => return Err(Trap::new(TrapKind::Invalid, "value compare on a non-array")),
                };
                if ia.len() != ib.len() {
                    return Ok(false);
                }
                for i in 0..ia.len() {
                    let xa = ia.get(i).unwrap_or_else(Slot::null);
                    let xb = ib.get(i).unwrap_or_else(Slot::null);
                    if !self.vals_equal(xa, xb, elem)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            TyKind::Str => {
                let sa = cell_of(a).as_bytes();
                let sb = cell_of(b).as_bytes();
                Ok(sa == sb)
            }
            TyKind::Bytes => {
                let sa = cell_of(a).as_bytes();
                let sb = cell_of(b).as_bytes();
                Ok(sa == sb)
            }
            TyKind::Enum { .. } => Ok(Slot::same_ref(a, b)),
            _ => Ok(Slot::same_ref(a, b)),
        }
    }

    /// `CloneVal` — deep-copy a value (RFC 0009/0016 v1.1): records and
    /// arrays clone into fresh cells; ref-typed children share.
    #[inline(always)]
    pub(super) fn op_clone_val(&mut self, dst: Reg, src: Reg, ty: TypeId) -> Result<(), Trap> {
        let v = self.heap.clone_val(self.cur_regs[src as usize], ty, &self.prog.types).map_err(|t| {
            let f = &self.prog.funcs[self.cur_func as usize];
            Trap::new(t.kind, format!("{} (in {} @ pc {} r{src}, type {}, bits {:#x})", t.msg, f.name, self.cur_pc, self.prog.types.name(ty), unsafe { self.cur_regs[src as usize].i }))
        })?;
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = v;
        if self.is_ref(ty) {
            self.heap.release(old);
        }
        Ok(())
    }

    /// `nil` legality (RFC 0005): a null slot reaching a dereference is
    /// the `NilDeref` trap — never a silent read.
    #[inline]
    pub(super) fn nil_checked(&self, s: Slot) -> Result<Slot, Trap> {
        if unsafe { s.r.is_null() } {
            return Err(Trap::new(TrapKind::NilDeref, "nil dereference"));
        }
        Ok(s)
    }

    /// `MakeRecord` — fused record literal (one alloc, all fields written).
    #[inline(always)]
    pub(super) fn op_make_record(&mut self, dst: Reg, ty: TypeId, vals: &[Reg]) -> Result<(), Trap> {
        let c = self.heap.alloc_record_zeroed(ty, vals.len())?;
        if let CellData::Record { fields } = &cell_of(c).data {
            let mut fb = fields.borrow_mut();
            for &i in &self.data_ref_fields[ty as usize] {
                let v = self.cur_regs[vals[i as usize] as usize];
                self.heap.retain(v);
            }
            for (i, &vre) in vals.iter().enumerate() {
                fb.set(i, self.cur_regs[vre as usize]);
            }
        }
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = c;
        self.heap.release(old);
        Ok(())
    }

    /// `Call` — shared by `step` and the `run_loop` fast path (avoids
    /// cloning the `args` vector per call).
    #[inline(always)]
    pub(super) fn op_call(&mut self, func: u32, args: &[Reg], dst: Option<Reg>) {
        let nregs = self.prog.funcs[func as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        for (i, &a) in args.iter().enumerate() {
            regs[i] = self.cur_regs[a as usize];
            if self.is_ref(self.param_ty(func, i)) {
                self.heap.retain(regs[i]);
            }
        }
        self.enter(func, regs, dst);
    }

    /// `CallM` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_call_m(&mut self, func: u32, recv: Reg, args: &[Reg], dst: Option<Reg>) {
        let nregs = self.prog.funcs[func as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        regs[0] = self.cur_regs[recv as usize];
        if self.is_ref(self.param_ty(func, 0)) {
            self.heap.retain(regs[0]);
        }
        for (i, &a) in args.iter().enumerate() {
            regs[i + 1] = self.cur_regs[a as usize];
            if self.is_ref(self.param_ty(func, i + 1)) {
                self.heap.retain(regs[i + 1]);
            }
        }
        self.enter(func, regs, dst);
    }

    /// `CallI` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_call_i(&mut self, slot: u32, recv: Reg, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        let ty = cell_of(self.cur_regs[recv as usize]).ty;
        let fid = self
            .prog
            .vtables
            .get(ty as usize)
            .and_then(|v| v.get(slot as usize))
            .and_then(|f| *f)
            .ok_or_else(|| {
                Trap::new(
                    TrapKind::Invalid,
                    format!(
                        "no impl for trait slot {slot} on {} — `is` would have said false",
                        self.prog.types.name(ty)
                    ),
                )
            })?;
        let nregs = self.prog.funcs[fid as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        regs[0] = self.cur_regs[recv as usize];
        if self.is_ref(self.param_ty(fid, 0)) {
            self.heap.retain(regs[0]);
        }
        for (i, &a) in args.iter().enumerate() {
            regs[i + 1] = self.cur_regs[a as usize];
            if self.is_ref(self.param_ty(fid, i + 1)) {
                self.heap.retain(regs[i + 1]);
            }
        }
        self.enter(fid, regs, dst);
        Ok(())
    }

    /// `CallFn` — shared by `step` and the `run_loop` fast path. Reads the
    /// captures by reference (no per-call `Vec` clone).
    #[inline(always)]
    pub(super) fn op_call_fn(&mut self, fval: Reg, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        let cell = cell_of(self.cur_regs[fval as usize]);
        let (fid, captures): (u32, &[Slot]) = match &cell.data {
            CellData::Closure { func, captures } => (*func, captures.as_slice()),
            _ => return Err(Trap::new(TrapKind::Invalid, "call on non-closure")),
        };
        let nparams = self.prog.funcs[fid as usize].params.len();
        let ncaptures = self.prog.funcs[fid as usize].n_captures as usize;
        let declared = nparams.saturating_sub(ncaptures);
        let nregs = self.prog.funcs[fid as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        for (i, &a) in args.iter().enumerate().take(declared) {
            regs[i] = self.cur_regs[a as usize];
            if self.is_ref(self.param_ty(fid, i)) {
                self.heap.retain(regs[i]);
            }
        }
        for (i, &c) in captures.iter().enumerate() {
            if declared + i < regs.len() {
                regs[declared + i] = c;
                let ty = self.param_ty(fid, declared + i);
                if self.is_ref(ty) {
                    self.heap.retain(c);
                }
            }
        }
        self.enter(fid, regs, dst);
        Ok(())
    }

    /// `ArrLit` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_arr_lit(&mut self, dst: Reg, ty: TypeId, elems: &[Reg]) -> Result<(), Trap> {
        let elem = match self.prog.types.kind(ty) {
            TyKind::Array { elem } => *elem,
            _ => TY_ANY,
        };
        let vals: Vec<Slot> = elems.iter().map(|&e| self.cur_regs[e as usize]).collect();
        let c = self.heap.alloc_array(elem, vals, &self.prog.types)?;
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = c;
        self.heap.release(old);
        Ok(())
    }

    pub(super) fn store_result(&mut self, dst: Option<Reg>, v: Slot) -> Result<(), Trap> {
        if let Some(d) = dst {
            let old = self.cur_regs[d as usize];
            self.cur_regs[d as usize] = v;
            // store_result is used for freshly minted cells: the register
            // now owns the mint's single reference; release the old ref if
            // the dst register is ref-typed
            let ty = self.regs_ty(d);
            if self.is_ref(ty) {
                self.heap.release(old);
            }
        } else {
            self.heap.release(v);
        }
        Ok(())
    }
}
