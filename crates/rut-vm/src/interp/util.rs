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
        // an Opaque box crosses as its handle: the handle owns a fresh
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
    if i < 0 {
        return Err(Trap::new(TrapKind::IndexOutOfBounds, format!("array index {i} out of bounds")));
    }
    match &cell.data {
        CellData::Array { items, .. } => {
            let items = items.borrow();
            items.get(i as usize).ok_or_else(|| {
                Trap::new(TrapKind::IndexOutOfBounds, format!("array index {i} out of bounds (len {})", items.len()))
            })
        }
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
    if i < 0 {
        return Err(Trap::new(TrapKind::IndexOutOfBounds, format!("array index {i} out of bounds")));
    }
    match &cell.data {
        CellData::Array { items, .. } => {
            let mut items = items.borrow_mut();
            let len = items.len();
            items.set(i as usize, v).ok_or_else(|| {
                Trap::new(TrapKind::IndexOutOfBounds, format!("index {i} out of bounds (len {len})"))
            })
        }
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
