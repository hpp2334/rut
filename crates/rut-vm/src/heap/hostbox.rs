//! `OpaqueBox<T>` — the host's typed view over an `Opaque` box (RFC
//! 0023/0026): news any `'static` Rust value into the arena (`alloc`),
//! borrows it back call-scoped (`with`/`with_mut`), and hands the plain
//! `Value::Opaque` handle to rut. The payload is invisible to rut —
//! `downcast<T>` is `None`, `o is Opaque` is `true` (RFC 0014) — and its
//! `Drop` runs deterministically when the box's rc hits 0 (RFC 0016 §3).

use super::*;

/// Exclusive-borrow sentinel (RFC 0023 §2): 0 = free, `BORROW_MUT` = one
/// exclusive borrow live, else the shared-borrow count.
pub(crate) const BORROW_MUT: u32 = u32::MAX;

/// The host's typed handle. `Clone` bumps the box's rc; dropping the
/// last handle (host or rut side) releases the cell and the payload.
pub struct OpaqueBox<T: 'static> {
    handle: OpaqueRef,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T: 'static> OpaqueBox<T> {
    /// News the box: `val` moves into the VM arena immediately, its
    /// shallow `size_of::<T>()` charged to the heap budget (RFC 0040).
    pub fn alloc(vm: &mut crate::interp::Vm, val: T) -> Result<OpaqueBox<T>, Trap> {
        let slot = vm.heap.alloc_host_box(val)?;
        let handle = vm.heap.opaque_handle_take(unsafe { slot.r });
        Ok(OpaqueBox { handle, _marker: std::marker::PhantomData })
    }

    /// Same check over a bare handle. The payload's own `Any` vtable
    /// carries the type token — `is::<T>` is the check, so a wrong-type
    /// borrow is a checked error, never UB. `type_name` (kept in the
    /// cell, unrecoverable from the erased box) names what it holds.
    pub fn from_handle(h: &OpaqueRef) -> Result<OpaqueBox<T>, Trap> {
        let cell = unsafe { &*h.ptr() };
        let CellData::HostBoxed { payload, type_name, .. } = &cell.data else {
            return Err(Trap::new(
                TrapKind::Invalid,
                "the box holds a rut value, not a host payload — recover it with `downcast<T>` on the rut side (RFC 0014)",
            ));
        };
        if !payload.is::<T>() {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("host box holds `{type_name}`, not `{}`", std::any::type_name::<T>()),
            ));
        }
        Ok(OpaqueBox { handle: h.clone(), _marker: std::marker::PhantomData })
    }

    /// Shared borrow for the closure's duration. Nested `with`s stack;
    /// an active `with_mut` excludes them.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, Trap> {
        let (payload, borrows) = self.boxed()?;
        if borrows.get() == BORROW_MUT {
            return Err(Trap::new(
                TrapKind::Invalid,
                "host box is `&mut`-borrowed by an outer host call (RFC 0023 §2)",
            ));
        }
        borrows.set(borrows.get() + 1);
        let out = f(payload
            .downcast_ref::<T>()
            .expect("OpaqueBox<T> holds a T — checked at from_handle/alloc"));
        borrows.set(borrows.get() - 1);
        Ok(out)
    }

    /// Exclusive borrow for the closure's duration. A re-entrant
    /// `vm.call` reaching another host fn that borrows the same box
    /// traps `borrowed by host` instead of aliasing (RFC 0023 §2).
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R, Trap> {
        let (payload, borrows) = self.boxed_mut()?;
        if borrows.get() != 0 {
            return Err(Trap::new(
                TrapKind::Invalid,
                "host box is borrowed by an outer host call (RFC 0023 §2)",
            ));
        }
        borrows.set(BORROW_MUT);
        let out = f(payload
            .downcast_mut::<T>()
            .expect("OpaqueBox<T> holds a T — checked at from_handle/alloc"));
        borrows.set(0);
        Ok(out)
    }

    /// The erased handle (RFC 0023): the box's `Opaque` identity, for
    /// passing the box through typed `call`/host-fn boundaries.
    pub fn handle(&self) -> &OpaqueRef {
        &self.handle
    }

    /// The erased payload and its borrow guard, checked to be a host box.
    fn boxed(&self) -> Result<(&Box<dyn std::any::Any>, &Cell<u32>), Trap> {
        let cell = unsafe { &*self.handle.ptr() };
        match &cell.data {
            CellData::HostBoxed { payload, borrows, .. } => Ok((payload, borrows)),
            _ => Err(Trap::new(TrapKind::Invalid, "not a host payload box")),
        }
    }

    /// The exclusive variant: reaches the payload mutably through the
    /// cell pointer. Sound under the guard `with_mut` holds — no other
    /// borrow of the box is live, and rut-side ops never touch host
    /// payloads — the same argument the previous raw-payload design made.
    fn boxed_mut(&self) -> Result<(&mut Box<dyn std::any::Any>, &Cell<u32>), Trap> {
        let cell = unsafe { &mut *(self.handle.ptr() as *mut CellVal) };
        match &mut cell.data {
            CellData::HostBoxed { payload, borrows, .. } => Ok((payload, borrows)),
            _ => Err(Trap::new(TrapKind::Invalid, "not a host payload box")),
        }
    }

}

impl<T: 'static> Clone for OpaqueBox<T> {
    fn clone(&self) -> Self {
        OpaqueBox { handle: self.handle.clone(), _marker: std::marker::PhantomData }
    }
}
