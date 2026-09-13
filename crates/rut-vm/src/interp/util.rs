//! Free helpers: host-boundary `Slot`↔`Value` conversion and the
//! sequence/sum accessors used by the op bodies.
use super::*;

pub(super) fn slot_to_value(v: Slot, ty: TypeId, prog: &Program, heap: &Heap) -> Value {
    use rut_core::types::TyKind;
    match prog.types.kind(ty) {
        TyKind::Prim(PrimTy::F32) | TyKind::Prim(PrimTy::F64) => Value::F64(unsafe { v.f }),
        TyKind::Prim(PrimTy::Bool) => Value::Bool(v.as_bool()),
        TyKind::Prim(PrimTy::Char) => Value::Char(v.as_char()),
        TyKind::Prim(_) | TyKind::Unit => Value::I64(unsafe { v.i }),
        TyKind::Str => Value::Str(cell_of(v).as_str().to_string()),
        // bytes is the one sequence that crosses (RFC 0023 §2, RFC 0004)
        TyKind::Bytes => Value::Bytes(cell_of(v).as_bytes().to_vec()),
        TyKind::Option { elem } => {
            let cell = cell_of(v);
            match &cell.data {
                CellData::Sum { tag: 1, .. } => Value::Opt(None),
                CellData::Sum { tag: _, payload } => Value::Opt(Some(Box::new(
                    slot_to_value(payload.expect("Some without payload"), *elem, prog, heap),
                ))),
                _ => Value::Opt(None),
            }
        }
        TyKind::Result { ok, err } => {
            let cell = cell_of(v);
            match &cell.data {
                CellData::Sum { tag: 1, payload } => Value::Res(Err(Box::new(slot_to_value(
                    payload.expect("Err without payload"),
                    *err,
                    prog,
                    heap,
                )))),
                CellData::Sum { tag: _, payload } => Value::Res(Ok(Box::new(slot_to_value(
                    payload.expect("Ok without payload"),
                    *ok,
                    prog,
                    heap,
                )))),
                _ => Value::Res(Ok(Box::new(Value::Unit))),
            }
        }
        // an Opaque box crosses as its handle: the handle owns a fresh
        // arena reference; the pending slot reference is released by do_ret
        TyKind::Opaque => {
            let p = unsafe { v.r };
            debug_assert!(!p.is_null(), "opaque slot without a cell");
            Value::Opaque(heap.opaque_handle(p))
        }
        _ => Value::I64(unsafe { v.i }),
    }
}

pub(super) fn value_kind_name(v: &Value) -> &'static str {
    match v {
        Value::Unit => "unit",
        Value::I64(_) => "an integer",
        Value::F64(_) => "a float",
        Value::Bool(_) => "a bool",
        Value::Char(_) => "a char",
        Value::Str(_) => "a string",
        Value::Bytes(_) => "bytes",
        Value::Opt(_) => "an Option",
        Value::Res(_) => "a Result",
        Value::Opaque(_) => "an Opaque",
    }
}

pub(super) fn seq_get(cell: &crate::heap::CellVal, i: i64) -> Result<Slot, Trap> {
    let items = match &cell.data {
        crate::heap::CellData::Array { items, .. } => items.borrow(),
        _ => return Err(Trap::new(TrapKind::Invalid, "index on non-sequence")),
    };
    items
        .get(i as usize)
        .ok_or_else(|| Trap::new(TrapKind::IndexOutOfBounds, format!("array index {i} out of bounds (len {})", items.len())))
}

pub(super) fn seq_set(cell: &crate::heap::CellVal, i: i64, v: Slot) -> Result<Slot, Trap> {
    let mut items = match &cell.data {
        crate::heap::CellData::Array { items, .. } => items.borrow_mut(),
        _ => return Err(Trap::new(TrapKind::Invalid, "index-set on non-sequence")),
    };
    let len = items.len();
    items
        .set(i as usize, v)
        .ok_or_else(|| Trap::new(TrapKind::IndexOutOfBounds, format!("index {i} out of bounds (len {len})")))
}

pub(super) fn sum_tag(cell: &crate::heap::CellVal) -> Result<u32, Trap> {
    match &cell.data {
        crate::heap::CellData::Sum { tag, .. } => Ok(*tag),
        _ => Err(Trap::new(TrapKind::Invalid, "sum op on non-sum")),
    }
}

pub(super) fn sum_parts(cell: &crate::heap::CellVal) -> Result<(u32, Option<Slot>), Trap> {
    match &cell.data {
        crate::heap::CellData::Sum { tag, payload } => Ok((*tag, *payload)),
        _ => Err(Trap::new(TrapKind::Invalid, "sum op on non-sum")),
    }
}

/// Zero/default element from a baked repr without a type-table lookup
/// (used by `ArrNew`'s zero-fill): primitives default to 0, handles null.
pub(super) fn default_slot_repr(repr: Repr) -> Slot {
    match repr {
        Repr::Prim(_) => Slot::int(0),
        _ => Slot::null(),
    }
}
