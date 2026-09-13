//! Sequence natives (RFC 0032 §1.1 R2): the heap array's runtime length
//! (`Array<T>.len()` / `bytes_len`, RFC 0005/0004).
use super::*;

impl Vm {
    pub(super) fn nat_arr_len(&mut self, recv: Option<Reg>, dst: Option<Reg>) -> Result<(), Trap> {
        let cell = cell_of(self.reg(recv.unwrap()));
        let n = match &cell.data {
            crate::heap::CellData::Array { items, .. } => items.borrow().len() as i64,
            _ => return Err(Trap::new(TrapKind::Invalid, "len on non-sequence")),
        };
        if let Some(d) = dst {
            self.cur_regs[d as usize] = Slot::int(n);
        }
        Ok(())
    }
}
