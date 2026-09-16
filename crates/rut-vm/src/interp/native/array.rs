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

    /// `v.slice(from, to)` — an O(1) window over the backing array
    /// (RFC 0042 §6). recv = the backing array cell (or a box/window to
    /// flatten); args = [from, to, live_len]. Bounds are the caller's
    /// LIVE length, not the capacity. The result is an `ArrView` cell —
    /// the caller boxes it as `*Vec<T>`.
    pub(super) fn nat_arr_slice(&mut self, recv: Option<Reg>, args: &[Reg], dst: Option<Reg>) -> Result<(), Trap> {
        let mut parent = self.reg(recv.unwrap());
        let from = unsafe { self.reg(args[0]).i };
        let to = unsafe { self.reg(args[1]).i };
        let mut live = unsafe { self.reg(args[2]).i };
        let mut base: u32 = 0;
        // flatten windows onto the root backing array (view-of-view)
        let root = loop {
            let c = cell_of(parent);
            match &c.data {
                crate::heap::CellData::ArrView { parent: p, off, len } => {
                    base += *off;
                    live = *len as i64;
                    parent = *p;
                }
                crate::heap::CellData::Array { .. } => break parent,
                _ => return Err(Trap::new(TrapKind::Invalid, "slice on non-sequence")),
            }
        };
        if from < 0 || to < from || to > live {
            return Err(Trap::new(
                TrapKind::IndexOutOfBounds,
                format!("slice {from}..{to} out of bounds (len {live})"),
            ));
        }
        let c = self.heap.alloc_arr_view(root, base + from as u32, (to - from) as u32)?;
        self.store_result(dst, c)?;
        Ok(())
    }
}
