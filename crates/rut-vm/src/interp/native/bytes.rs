//! The bytes native (RFC 0032 §1.1 R2): `b.clone()` — the one copy
//! escape hatch (RFC 0044). Every other cell type shares on binding;
//! `clone` is the explicit, one-shot buffer copy.
use super::*;

impl Vm {
    pub(in crate::interp) fn nat_bytes_clone(&mut self, recv: Option<Reg>, dst: Option<Reg>) -> Result<(), Trap> {
        use crate::heap::ArrKind;
        let cell = cell_of(self.reg(recv.unwrap()));
        match &cell.data {
            CellData::Array { items, .. } if items.borrow().kind == ArrKind::U8 => {}
            _ => return Err(Trap::new(TrapKind::Invalid, "clone on non-bytes")),
        }
        let src = cell.bytes_copy();
        let c = self.heap.alloc_bytes(src)?;
        self.store_result(dst, c)?;
        Ok(())
    }
}
