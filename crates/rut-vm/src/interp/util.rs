//! Free helpers: host-boundary `Slot`↔`Value` conversion and the
//! sequence/sum accessors used by the op bodies.
use super::*;

pub(super) fn slot_to_value(v: Slot, ty: TypeId, prog: &Program, heap: &Heap) -> Value {
    use rut_core::types::TyKind;
    match prog.types.kind(ty) {
        TyKind::Prim(PrimTy::F32) | TyKind::Prim(PrimTy::F64) => Value::F64(unsafe { v.f }),
        TyKind::Prim(PrimTy::Bool) => Value::Bool(v.as_bool()),
        TyKind::Prim(PrimTy::Char) => Value::Char(v.as_char()),
        TyKind::Prim(_) | TyKind::Nil => Value::I64(unsafe { v.i }),
        TyKind::Str => Value::Str(cell_of(v).as_str().to_string()),
        // bytes is the one sequence that crosses (RFC 0023 §2, RFC 0004)
        TyKind::Bytes => Value::Bytes(cell_of(v).bytes_copy()),
        // an `opaque` box crosses as its handle: the handle owns a fresh
        // arena reference; the pending slot reference is released by do_ret
        TyKind::Opaque => {
            let p = unsafe { v.r };
            debug_assert!(!p.is_null(), "opaque slot without a cell");
            Value::Opaque(heap.opaque_handle(p))
        }
        // tuples cross field-by-field (RFC 0007 v1.1)
        TyKind::Data { fields } => {
            let cell = cell_of(v);
            let mut out = Vec::with_capacity(fields.len());
            if let CellData::Record { fields: slots } = &cell.data {
                let sb = slots.borrow();
                for (i, f) in fields.iter().enumerate() {
                    if let Some(sv) = sb.get(i) {
                        out.push(slot_to_value(sv, f.ty, prog, heap));
                    }
                }
            }
            Value::Tuple(out)
        }
        _ => Value::I64(unsafe { v.i }),
    }
}

pub(super) fn value_kind_name(v: &Value) -> &'static str {
    v.kind_name()
}

pub(super) fn seq_get(cell: &crate::heap::CellVal, i: i64) -> Result<Slot, Trap> {
    // the hot path first: a plain array — ONE unsigned compare folds the
    // negative and overflow checks, then a direct element read
    if let CellData::Array { items, .. } = &cell.data {
        let d = items.borrow();
        if (i as u64) < d.len as u64 {
            // covers i < 0 AND i >= len
            return Ok(unsafe { d.read_unchecked(i as usize) });
        }
        return Err(Trap::new(
            TrapKind::IndexOutOfBounds,
            format!("array index {i} out of bounds (len {})", d.len),
        ));
    }
    match &cell.data {
        // an array window: element i is parent[off + i], bounds vs the
        // window (RFC 0042 §6)
        CellData::ArrView { parent, off, len } => {
            if i as u32 >= *len {
                return Err(Trap::new(TrapKind::IndexOutOfBounds, format!("view index {i} out of bounds (len {len})")));
            }
            let p = cell_of(*parent);
            let items = match &p.data {
                CellData::Array { items, .. } => items.borrow(),
                _ => return Err(Trap::new(TrapKind::Invalid, "view over a non-array backing")),
            };
            items.get(*off as usize + i as usize).ok_or_else(|| {
                Trap::new(TrapKind::IndexOutOfBounds, format!("view index {i} out of bounds (len {len})"))
            })
        }
        _ => Err(Trap::new(TrapKind::Invalid, "index on non-sequence")),
    }
}

pub(super) fn seq_set(cell: &crate::heap::CellVal, i: i64, v: Slot) -> Result<Slot, Trap> {
    // the hot path first: a plain array — ONE unsigned compare folds the
    // negative and overflow checks, then a direct element write
    if let CellData::Array { items, .. } = &cell.data {
        let mut d = items.borrow_mut();
        if (i as u64) < d.len as u64 {
            // covers i < 0 AND i >= len
            return Ok(unsafe { d.write_unchecked(i as usize, v) });
        }
        return Err(Trap::new(
            TrapKind::IndexOutOfBounds,
            format!("index {i} out of bounds (len {})", d.len),
        ));
    }
    match &cell.data {
        // WRITE-THROUGH: a window write hits the parent (RFC 0042 §6)
        CellData::ArrView { parent, off, len } => {
            if i as u32 >= *len {
                return Err(Trap::new(TrapKind::IndexOutOfBounds, format!("view index {i} out of bounds (len {len})")));
            }
            let p = cell_of(*parent);
            let mut items = match &p.data {
                CellData::Array { items, .. } => items.borrow_mut(),
                _ => return Err(Trap::new(TrapKind::Invalid, "view over a non-array backing")),
            };
            let idx = *off as usize + i as usize;
            items.set(idx, v).ok_or_else(|| {
                Trap::new(TrapKind::IndexOutOfBounds, format!("view index {i} out of bounds (len {len})"))
            })
        }
        _ => Err(Trap::new(TrapKind::Invalid, "index-set on non-sequence")),
    }
}


/// The window intercept for `GetF`/`ArrGetF` (RFC 0042 §6): a scalar
/// field reads the window's own length; a ref field (the `buf` read)
/// hands out THE WINDOW itself (retained), so the element ops through
/// it stay window-relative and bounds-checked. `None` = not a window —
/// the caller proceeds with the record path.
pub(super) fn window_getf(heap: &Heap, cell: &crate::heap::CellVal, obj: Slot, field_repr: Repr) -> Option<Slot> {
    if let CellData::ArrView { len, .. } = &cell.data {
        if field_repr.is_ref() {
            heap.retain(obj);
            return Some(obj);
        }
        return Some(Slot::int(*len as i64));
    }
    None
}

/// The window intercept for `SetF`: a window is a fixed-length view —
/// no field of it can be written (`push`/`pop`/re-backing trap here).
pub(super) fn window_setf_trap(cell: &crate::heap::CellVal) -> Option<Trap> {
    if matches!(cell.data, CellData::ArrView { .. }) {
        return Some(Trap::new(
            TrapKind::Invalid,
            "an array view is fixed-length — copy the elements out to grow or shrink",
        ));
    }
    None
}

/// The window intercept for `ArrSetF` (RFC 0042 §6): writes go through —
/// element `i` of the window is `parent[off + i]`. `None` = not a window.
pub(super) fn window_sets(cell: &crate::heap::CellVal, i: i64, v: Slot) -> Option<Result<Slot, Trap>> {
    if let CellData::ArrView { parent, off, len } = &cell.data {
        if i < 0 || i as u32 >= *len {
            return Some(Err(Trap::new(
                TrapKind::IndexOutOfBounds,
                format!("view index {i} out of bounds (len {len})"),
            )));
        }
        let p = cell_of(*parent);
        let items = match &p.data {
            CellData::Array { items, .. } => items,
            _ => return Some(Err(Trap::new(TrapKind::Invalid, "view over a non-array backing"))),
        };
        let idx = *off as usize + i as usize;
        return Some(match items.borrow_mut().set(idx, v) {
            Some(old) => Ok(old),
            None => Err(Trap::new(
                TrapKind::IndexOutOfBounds,
                format!("view index {i} out of bounds (len {len})"),
            )),
        });
    }
    None
}

/// Zero/default element from a baked repr without a type-table lookup
/// (used by `ArrNew`'s zero-fill): primitives default to 0, handles null.
pub(super) fn default_slot_repr(repr: Repr) -> Slot {
    match repr {
        Repr::Prim(_) => Slot::int(0),
        _ => Slot::null(),
    }
}

// ---- the primitive-optional element store (RFC 0044 §5) ----------------
//
// A `[?prim]` backing (`ArrKind::Opt`) holds each element as a raw payload
// plus a nil tag — never a cell. The element ops route here by their baked
// `Repr::OptPrim*`, so the hot path pays no runtime kind check and the
// generic `seq_get`/`seq_set` never see an opt store (debug-asserted in
// `read_unchecked`/`write_unchecked`). Both engines call these helpers, so
// they execute identical code (lock-step by construction).
//
// The rc law of the raw store: no element is a handle — writes retain
// nothing and the displaced value is raw bits (nothing released); reads
// mint a fresh opt VALUE (value semantics) whose reference passes to the
// destination register, so the caller releases the displaced dst value but
// does NOT retain the mint.

/// One element access resolved against the backing: the items run and the
/// absolute element index, bounds-checked per shape (direct array: one
/// unsigned compare folding negative + overflow, the `seq_get` shape;
/// window: bounds vs the window, RFC 0042 §6). `None` = `cell` is not an
/// array/window at all (a miscompiled op — the caller traps).
fn opt_lookup<'a>(cell: &'a CellVal, i: i64) -> Option<Result<(std::cell::Ref<'a, crate::heap::ArrData>, usize), Trap>> {
    match &cell.data {
        CellData::Array { items, .. } => {
            let d = items.borrow();
            if i as u64 >= d.len as u64 {
                return Some(Err(Trap::new(
                    TrapKind::IndexOutOfBounds,
                    format!("array index {i} out of bounds (len {})", d.len),
                )));
            }
            Some(Ok((d, i as usize)))
        }
        CellData::ArrView { parent, off, len } => {
            let p = cell_of(*parent);
            let CellData::Array { items, .. } = &p.data else {
                return Some(Err(Trap::new(TrapKind::Invalid, "view over a non-array backing")));
            };
            let d = items.borrow();
            if i < 0 || i as u32 >= *len {
                return Some(Err(Trap::new(
                    TrapKind::IndexOutOfBounds,
                    format!("view index {i} out of bounds (len {len})"),
                )));
            }
            Some(Ok((d, *off as usize + i as usize)))
        }
        _ => None,
    }
}

/// `ArrGet{repr: OptPrim}` — decode element `i`, minting a fresh opt value
/// for non-nil (the mint's rc=1 IS the destination's reference).
#[inline]
pub(super) fn opt_elem_get(heap: &Heap, cell: &CellVal, i: i64, elem: TypeId) -> Result<Slot, Trap> {
    match opt_lookup(cell, i) {
        Some(Ok((d, at))) => match d.opt_raw(at) {
            Some(raw) => heap.alloc_opt_value(elem, raw),
            None => Ok(Slot::null()),
        },
        Some(Err(t)) => Err(t),
        None => Err(Trap::new(TrapKind::Invalid, "element get on non-sequence")),
    }
}

/// `ArrGet{repr: OptPrimLoad}` (the deref-folded form) — the RAW payload;
/// a nil tag is the `NilDeref` trap, exactly the `ArrGet + GetF{0}` pair's
/// behavior minus the mint.
#[inline]
pub(super) fn opt_elem_get_load(cell: &CellVal, i: i64) -> Result<Slot, Trap> {
    match opt_lookup(cell, i) {
        Some(Ok((d, at))) => match d.opt_raw(at) {
            Some(raw) => Ok(raw),
            None => Err(Trap::new(TrapKind::NilDeref, "nil dereference")),
        },
        Some(Err(t)) => Err(t),
        None => Err(Trap::new(TrapKind::Invalid, "element get on non-sequence")),
    }
}

/// `ArrSet{repr: OptPrim}` — encode a proper `?prim` value (cell or null)
/// into the store. Nothing retained; the displaced element is raw bits.
#[inline]
pub(super) fn opt_elem_set(cell: &CellVal, i: i64, v: Slot) -> Result<(), Trap> {
    match opt_lookup_mut(cell, i) {
        Some(Ok((mut d, at))) => {
            d.write_opt(at, v);
            Ok(())
        }
        Some(Err(t)) => Err(t),
        None => Err(Trap::new(TrapKind::Invalid, "element set on non-sequence")),
    }
}

/// `ArrSet{repr: OptPrimRaw}` (the MakeOpt-elided form) — store the RAW
/// payload, some-tag implied. Nothing retained; nothing released.
#[inline]
pub(super) fn opt_elem_set_raw(cell: &CellVal, i: i64, raw: Slot) -> Result<(), Trap> {
    match opt_lookup_mut(cell, i) {
        Some(Ok((mut d, at))) => {
            d.write_opt_raw(at, raw);
            Ok(())
        }
        Some(Err(t)) => Err(t),
        None => Err(Trap::new(TrapKind::Invalid, "element set on non-sequence")),
    }
}

/// The write-side resolver: the items run (exclusively borrowed) plus the
/// absolute element index — same shapes and bounds as [`opt_lookup`].
fn opt_lookup_mut<'a>(
    cell: &'a CellVal,
    i: i64,
) -> Option<Result<(std::cell::RefMut<'a, crate::heap::ArrData>, usize), Trap>> {
    match &cell.data {
        CellData::Array { items, .. } => {
            let d = items.borrow_mut();
            if i as u64 >= d.len as u64 {
                return Some(Err(Trap::new(
                    TrapKind::IndexOutOfBounds,
                    format!("array index {i} out of bounds (len {})", d.len),
                )));
            }
            Some(Ok((d, i as usize)))
        }
        CellData::ArrView { parent, off, len } => {
            let p = cell_of(*parent);
            let CellData::Array { items, .. } = &p.data else {
                return Some(Err(Trap::new(TrapKind::Invalid, "view over a non-array backing")));
            };
            let mut d = items.borrow_mut();
            if i < 0 || i as u32 >= *len {
                return Some(Err(Trap::new(
                    TrapKind::IndexOutOfBounds,
                    format!("view index {i} out of bounds (len {len})"),
                )));
            }
            Some(Ok((d, *off as usize + i as usize)))
        }
        _ => None,
    }
}

/// The elem TypeId of the opt-prim backing an element op targets — direct
/// array (its own elem) or window (the parent's). The mint needs the `?p`
/// type id to build the fresh opt value.
pub(super) fn opt_elem_ty(cell: &CellVal) -> Option<TypeId> {
    match &cell.data {
        CellData::Array { elem, .. } => Some(*elem),
        CellData::ArrView { parent, .. } => match &cell_of(*parent).data {
            CellData::Array { elem, .. } => Some(*elem),
            _ => None,
        },
        _ => None,
    }
}

/// The backing-array cell of a fused field-array access: the record's
/// array field, or the window object itself (RFC 0042 §6).
pub(super) fn f_arr_cell(obj_cell: &CellVal, field: u32) -> Result<&CellVal, Trap> {
    match &obj_cell.data {
        CellData::Record { fields } => {
            let arr = fields
                .borrow()
                .get(field as usize)
                .ok_or_else(|| Trap::new(TrapKind::Invalid, "field index out of range"))?;
            Ok(cell_of(arr))
        }
        CellData::ArrView { .. } => Ok(obj_cell),
        _ => Err(Trap::new(TrapKind::Invalid, "field-array op on non-record")),
    }
}
