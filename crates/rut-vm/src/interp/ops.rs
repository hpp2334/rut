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

    /// The rut-visible type of a value slot (RFC 0014/0023): a rut-value
    /// box answers its payload's runtime type, a host payload box the box
    /// itself (TY_OPAQUE — `o is opaque` true, `o is T` misses for every
    /// rut T), anything else its cell's own type. Store entries (the P2
    /// repr) read through the tag; cells fall through.
    pub(super) fn effective_ty(&self, s: Slot) -> TypeId {
        if let Some(e) = crate::heap::store::store_entry(s) {
            return match &e.e {
                crate::heap::store::OpaqueEntry::Host(_) => rut_core::types::TY_OPAQUE,
                crate::heap::store::OpaqueEntry::Rut(r) => r.val_ty,
            };
        }
        cell_of(s).ty
    }

    /// The `is` law's type (RFC 0014, 2026-09): `is` names the BOX, never
    /// the payload — any opaque (host or rut) answers TY_OPAQUE, so
    /// `o is opaque` (or an alias) hits and `o is T` misses for every
    /// payload T; recovery is `downcast<T>` only (its own TidOf keeps
    /// reading the payload). IsTrait probes the same type.
    pub(crate) fn is_ty(&self, s: Slot) -> TypeId {
        if crate::heap::store::is_entry(s) {
            return rut_core::types::TY_OPAQUE;
        }
        cell_of(s).ty
    }

    /// The TidOf/downcast law: a host payload box has no rut runtime type
    /// — report the sentinel so `downcast<T>` compares false for every T
    /// and yields None — never a trap (RFC 0014).
    pub(crate) fn tid_ty(&self, s: Slot) -> TypeId {
        if let Some(e) = crate::heap::store::store_entry(s) {
            if let crate::heap::store::OpaqueEntry::Host(_) = &e.e {
                return rut_core::types::HOST_BOX_TID;
            }
        }
        self.effective_ty(s)
    }

    // ---- op bodies shared by `step` and the `run_loop` fast path ----

    /// `ArrGet` — the fast path calls this directly, so it must stay small
    /// and inlinable.
    #[inline(always)]
    pub(super) fn op_arr_get(&mut self, dst: Reg, arr: Reg, idx: Reg, repr: Repr) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let cell = cell_of(self.cur_regs[arr as usize]);
        // the primitive-optional store (RFC 0044 §5): decode the raw
        // element; non-nil mints a fresh opt VALUE whose reference passes
        // to dst — release the displaced value, retain nothing.
        if let Repr::OptPrim(_) = repr {
            let Some(elem) = opt_elem_ty(cell) else {
                return Err(Trap::new(TrapKind::Invalid, "element get on non-sequence"));
            };
            let v = opt_elem_get(&self.heap, cell, i, elem)?;
            return self.store_dst_ref(dst, v, true);
        }
        // the deref-folded load form: the RAW payload out, nil tag traps
        if let Repr::OptPrimLoad(_) = repr {
            let v = opt_elem_get_load(cell, i)?;
            // dst is a prim register (the payload's own type) — no rc
            self.cur_regs[dst as usize] = v;
            return Ok(());
        }
        let v = seq_get(cell, i)?;
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = v;
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// Store `v` into a ref-typed dst register. `owned = true` marks a value
    /// whose reference the dst TAKES OVER (the fresh opt mint) — only the
    /// displaced value releases; `owned = false` retains first (the shared
    /// handle law).
    #[inline(always)]
    fn store_dst_ref(&mut self, dst: Reg, v: Slot, owned: bool) -> Result<(), Trap> {
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = v;
        if !owned {
            self.heap.retain(v);
        }
        self.heap.release(old);
        Ok(())
    }

    /// `ArrSet` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_arr_set(&mut self, arr: Reg, idx: Reg, val: Reg, repr: Repr) -> Result<(), Trap> {
        let i = unsafe { self.cur_regs[idx as usize].i };
        let cell = cell_of(self.cur_regs[arr as usize]);
        let v = self.cur_regs[val as usize];
        // the primitive-optional store: encode into the raw element; no rc
        // on either side (the displaced element is raw bits)
        if let Repr::OptPrim(_) = repr {
            return opt_elem_set(cell, i, v);
        }
        if let Repr::OptPrimRaw(_) = repr {
            return opt_elem_set_raw(cell, i, v);
        }
        let old = match window_sets(cell, i, v) {
            Some(r) => r?,
            None => seq_set(cell, i, v)?,
        };
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
        let obj_cell = cell_of(self.cur_regs[obj as usize]);
        // the primitive-optional store: resolve the backing (the record's
        // array field, or the window itself), then the same decode/mint law
        // as `op_arr_get`
        if let Repr::OptPrim(_) = repr {
            let arr_cell = f_arr_cell(obj_cell, field)?;
            let Some(elem) = opt_elem_ty(arr_cell) else {
                return Err(Trap::new(TrapKind::Invalid, "element get on non-sequence"));
            };
            let v = opt_elem_get(&self.heap, arr_cell, i, elem)?;
            return self.store_dst_ref(dst, v, true);
        }
        if let Repr::OptPrimLoad(_) = repr {
            let arr_cell = f_arr_cell(obj_cell, field)?;
            let v = opt_elem_get_load(arr_cell, i)?;
            self.cur_regs[dst as usize] = v; // prim dst — no rc
            return Ok(());
        }
        let v = match &obj_cell.data {
            // the common case: a Vec record — read its backing array
            CellData::Record { fields } => {
                let arr = fields
                    .borrow()
                    .get(field as usize)
                    .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?;
                seq_get(cell_of(arr), i)?
            }
            // a window obj: an ELEMENT read through the window —
            // parent[off + i], bounds vs the window (RFC 0042 §6)
            CellData::ArrView { .. } => seq_get(obj_cell, i)?,
            _ => return Err(Trap::new(TrapKind::Invalid, "field-array get on non-record")),
        };
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
        let obj_cell = cell_of(self.cur_regs[obj as usize]);
        // the primitive-optional store: encode; no rc on either side
        if let Repr::OptPrim(_) = repr {
            let arr_cell = f_arr_cell(obj_cell, field)?;
            return opt_elem_set(arr_cell, i, v);
        }
        if let Repr::OptPrimRaw(_) = repr {
            let arr_cell = f_arr_cell(obj_cell, field)?;
            return opt_elem_set_raw(arr_cell, i, v);
        }
        let old = match &obj_cell.data {
            // the common case: a Vec record — write its backing array
            CellData::Record { fields } => {
                let arr = fields
                    .borrow()
                    .get(field as usize)
                    .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?;
                seq_set(cell_of(arr), i, v)?
            }
            _ => return Err(Trap::new(TrapKind::Invalid, "field-array set on non-record")),
        };
        if repr.is_ref() {
            self.heap.retain(v);
            self.heap.release(old);
        }
        Ok(())
    }

    /// `GetF` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_getf(&mut self, dst: Reg, obj: Reg, field: u32, repr: Repr) -> Result<(), Trap> {
        let raw = self.nil_checked(self.cur_regs[obj as usize])?;
        // the downcast ALIAS handoff (RFC 0014, refval-round2): the `?T`
        // result IS the opaque box, so the nullable deref (field 0) reads
        // the box's payload slot at the payload's own repr — a store slot
        // at P2, so the tag routes BEFORE any cell deref. Cell-repr
        // payloads retain their handle below — mutation through the read
        // hits the source cell; prim payloads read the bits copied at
        // construction.
        if crate::heap::store::is_entry(raw) {
            let entry = crate::heap::store::store_entry(raw).unwrap();
            let v = match (&entry.e, field) {
                (crate::heap::store::OpaqueEntry::Rut(r), 0) => r.slot,
                _ => return Err(Trap::new(TrapKind::Invalid, "field on non-record")),
            };
            let old = self.cur_regs[dst as usize];
            self.cur_regs[dst as usize] = v;
            if repr.is_ref() {
                self.heap.retain(v);
                self.heap.release(old);
            }
            return Ok(());
        }
        let cell = cell_of(raw);
        if let Some(v) = window_getf(&self.heap, cell, raw, repr) {
            let old = self.cur_regs[dst as usize];
            self.cur_regs[dst as usize] = v;
            if repr.is_ref() {
                self.heap.release(old);
            }
            return Ok(());
        }
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
        let raw = self.nil_checked(self.cur_regs[obj as usize])?;
        // a store slot is never a record (defensive, the same trap the
        // old box cells hit) — check the tag BEFORE any cell deref
        if crate::heap::store::is_entry(raw) {
            return Err(Trap::new(TrapKind::Invalid, "field-set on non-record"));
        }
        let cell = cell_of(raw);
        if let Some(t) = window_setf_trap(cell) {
            return Err(t);
        }
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

    /// `WeakNew` (RFC 0017 v1) — `Weak(v)`: a WeakBox side cell holding
    /// the referent's UNRETAINED slot word, registered into the
    /// referent's weak list. `ty` is the instantiated `Weak<elem>` id.
    /// THE ONE CONSUMING OP: the incoming reference is released and the
    /// register nulled (frame teardown would otherwise release it again)
    /// — the weak observes the BINDING's lifetime, never the temporary's
    /// (without this, the argument's own +1 pinned the referent until the
    /// frame ended and no death was ever observable mid-frame).
    #[inline(always)]
    pub(super) fn op_weak_new(&mut self, dst: Reg, src: Reg, ty: TypeId) -> Result<(), Trap> {
        let v = self.cur_regs[src as usize];
        let c = self.heap.alloc_weak(v, ty)?;
        // consume the argument: the box took the word, not a reference
        self.heap.release(v);
        self.cur_regs[src as usize] = Slot::null();
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = c;
        if self.is_ref(ty) {
            self.heap.release(old);
        }
        Ok(())
    }

    /// `WeakUpgrade` (RFC 0017 v1) — `w.upgrade()`: the live referent
    /// retained into a fresh `?elem` box (`ty` is the `?elem` id — the
    /// MakeOpt mint minus the boxing of null), or the NULL SLOT when the
    /// referent died: a true `nil`, never a box containing nil. The
    /// referent word is the full slot value (tagged for store entries),
    /// so `retain` routes either shape.
    #[inline(always)]
    pub(super) fn op_weak_upgrade(&mut self, recv: Reg, dst: Reg, ty: TypeId) -> Result<(), Trap> {
        let w = self.cur_regs[recv as usize];
        if unsafe { w.r.is_null() } {
            return Err(Trap::new(TrapKind::NilDeref, "upgrade on nil"));
        }
        let raw = match unsafe { &(*w.r).data } {
            CellData::WeakBox { referent } => referent.get(),
            _ => return Err(Trap::new(TrapKind::Invalid, "upgrade on non-weak")),
        };
        let out = if unsafe { raw.r.is_null() } {
            raw
        } else {
            self.heap.retain(raw);
            match self.heap.alloc_opt_value(ty, raw) {
                Ok(c) => c,
                Err(e) => {
                    // the box mint failed — give the retain back
                    self.heap.release(raw);
                    return Err(e);
                }
            }
        };
        let old = self.cur_regs[dst as usize];
        self.cur_regs[dst as usize] = out;
        if self.is_ref(ty) {
            self.heap.release(old);
        }
        Ok(())
    }

    /// `MakeOpt` — box `v` into a fresh one-slot cell (RFC 0044, the
    /// `T → ?T` coercion). The box ALIASES `v`'s cell — sharing, never a
    /// copy; primitives/`nil` copy the bits. `ty` is the nullable's own
    /// type.
    #[inline(always)]
    pub(super) fn op_make_opt(&mut self, dst: Reg, src: Reg, ty: TypeId) -> Result<(), Trap> {
        let raw = self.cur_regs[src as usize];
        let elem = match self.prog.types.kind(ty) {
            TyKind::Opt { elem } => *elem,
            _ => unreachable!("MakeOpt on a non-nullable type"),
        };
        // the box aliases the payload's cell (RFC 0044): a cell-repr
        // element (or an `fn` value, whose slot is the closure cell)
        // retains the handle — a window box references its window too
        // (RFC 0042 §6). Primitives/`nil` copy the bits.
        let is_fn = matches!(self.prog.types.kind(elem), TyKind::Fn { .. });
        let is_cell_repr = self.prog.types.repr_of(elem).is_ref() || is_fn;
        let raw_null = unsafe { raw.r.is_null() };
        let v = if !is_cell_repr || raw_null {
            raw
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

    /// `OnDrop` — attach a cleanup closure to a cell (RFC 0016 §3).
    #[inline(always)]
    pub(super) fn op_on_drop(&mut self, obj: Reg, cleanup: Reg) -> Result<(), Trap> {
        self.heap
            .set_drop_fn(self.cur_regs[obj as usize], self.cur_regs[cleanup as usize])
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

    /// The op's operand span, sliced from the CURRENT function's pool —
    /// ops address their argument lists by `(off, argc)` (RFC 0032).
    #[inline(always)]
    pub(super) fn cur_argv<'p>(&self, prog: &'p Program, off: u32, argc: u16) -> &'p [Reg] {
        &prog.funcs[self.cur_func as usize].argv[off as usize..off as usize + argc as usize]
    }

    /// `MakeRecord` — fused record literal (one alloc, all fields written).
    #[inline(always)]
    pub(super) fn op_make_record(&mut self, dst: Reg, ty: TypeId, argv_off: u32, argc: u16) -> Result<(), Trap> {
        let prog = Rc::clone(&self.prog);
        let vals = self.cur_argv(&prog, argv_off, argc);
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

    /// `Call`/`CallM` — shared by `step` and the `run_loop` fast path.
    /// The pool span IS the callee's parameter list (for `CallM` the
    /// receiver is `argv[0]`), so one uniform copy loop covers both.
    #[inline(always)]
    pub(super) fn op_call(&mut self, func: u32, argv_off: u32, argc: u16, dst: Reg) {
        let prog = Rc::clone(&self.prog);
        let args = self.cur_argv(&prog, argv_off, argc);
        let nregs = self.prog.funcs[func as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        for (i, &a) in args.iter().enumerate() {
            regs[i] = self.cur_regs[a as usize];
            if self.is_ref(self.param_ty(func, i)) {
                self.heap.retain(regs[i]);
            }
        }
        self.enter(func, regs, reg_opt(dst));
    }

    /// `CallI` — shared by `step` and the `run_loop` fast path; the
    /// receiver is `argv[0]`.
    #[inline(always)]
    pub(super) fn op_call_i(&mut self, slot: u32, argv_off: u32, argc: u16, dst: Reg) -> Result<(), Trap> {
        let prog = Rc::clone(&self.prog);
        let args = self.cur_argv(&prog, argv_off, argc);
        let recv = args[0];
        let ty = self
            .scalar_recv_ty(recv)
            .unwrap_or_else(|| self.effective_ty(self.cur_regs[recv as usize]));
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
                        self.prog.type_name(ty)
                    ),
                )
            })?;
        let nregs = self.prog.funcs[fid as usize].regs.len();
        let mut regs = self.take_regs(nregs);
        for (i, &a) in args.iter().enumerate() {
            regs[i] = self.cur_regs[a as usize];
            if self.is_ref(self.param_ty(fid, i)) {
                self.heap.retain(regs[i]);
            }
        }
        self.enter(fid, regs, reg_opt(dst));
        Ok(())
    }

    /// `CallFn` — shared by `step` and the `run_loop` fast path. Reads the
    /// captures by reference (no per-call `Vec` clone).
    #[inline(always)]
    pub(super) fn op_call_fn(&mut self, fval: Reg, argv_off: u32, argc: u16, dst: Reg) -> Result<(), Trap> {
        let prog = Rc::clone(&self.prog);
        let args = self.cur_argv(&prog, argv_off, argc);
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
        self.enter(fid, regs, reg_opt(dst));
        Ok(())
    }

    /// `ArrLit` — shared by `step` and the `run_loop` fast path.
    #[inline(always)]
    pub(super) fn op_arr_lit(&mut self, dst: Reg, ty: TypeId, argv_off: u32, argc: u16) -> Result<(), Trap> {
        let prog = Rc::clone(&self.prog);
        let elems = self.cur_argv(&prog, argv_off, argc);
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
