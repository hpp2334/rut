//! `OpaqueBox<T>` — the host's typed view over an `Opaque` box (RFC
//! 0023/0026): news any `'static` Rust value into the arena (`alloc`),
//! borrows it back call-scoped (`with`/`with_mut`), and hands the plain
//! `Value::Opaque` handle to rut. The payload is invisible to rut —
//! `downcast<T>` is `None`, `o is Opaque` is `true` (RFC 0014) — and its
//! `Drop` runs deterministically when the box's rc hits 0 (RFC 0016 §3).

use super::cell::BORROW_MUT;
use super::*;

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

    /// The checked view over a crossing value (`Value::Opaque` from an
    /// entry return or an args slice). The wrong shape or the wrong
    /// payload type is a `Trap` naming both sides — never UB.
    pub fn from_value(v: &Value) -> Result<OpaqueBox<T>, Trap> {
        let Value::Opaque(h) = v else {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("expected an Opaque handle, got {}", v.kind_name()),
            ));
        };
        Self::from_handle(h)
    }

    /// Same check over a bare handle.
    pub fn from_handle(h: &OpaqueRef) -> Result<OpaqueBox<T>, Trap> {
        let cell = unsafe { &*h.ptr() };
        let CellData::HostBoxed { payload } = &cell.data else {
            return Err(Trap::new(
                TrapKind::Invalid,
                "the box holds a rut value, not a host payload — recover it with `downcast<T>` on the rut side (RFC 0014)",
            ));
        };
        if !payload.is::<T>() {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("host box holds `{}`, not `{}`", payload.type_name(), std::any::type_name::<T>()),
            ));
        }
        Ok(OpaqueBox { handle: h.clone(), _marker: std::marker::PhantomData })
    }

    /// Shared borrow for the closure's duration. Nested `with`s stack;
    /// an active `with_mut` excludes them.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, Trap> {
        let payload = self.payload()?;
        if payload.borrows() == BORROW_MUT {
            return Err(Trap::new(
                TrapKind::Invalid,
                "host box is `&mut`-borrowed by an outer host call (RFC 0023 §2)",
            ));
        }
        payload.begin_shared();
        let out = f(unsafe { payload.deref::<T>() });
        payload.end_borrow();
        Ok(out)
    }

    /// Exclusive borrow for the closure's duration. A re-entrant
    /// `vm.call` reaching another host fn that borrows the same box
    /// traps `borrowed by host` instead of aliasing (RFC 0023 §2).
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R, Trap> {
        let payload = self.payload()?;
        if payload.borrows() != 0 {
            return Err(Trap::new(
                TrapKind::Invalid,
                "host box is borrowed by an outer host call (RFC 0023 §2)",
            ));
        }
        payload.begin_mut();
        let out = f(unsafe { payload.deref::<T>() });
        payload.end_borrow();
        Ok(out)
    }

    fn payload(&self) -> Result<&HostPayload, Trap> {
        let cell = unsafe { &*self.handle.ptr() };
        match &cell.data {
            CellData::HostBoxed { payload } => Ok(payload),
            _ => Err(Trap::new(TrapKind::Invalid, "not a host payload box")),
        }
    }

    /// Transfer this reference into a crossing `Value` (the handle
    /// moves; the rc count is unchanged).
    pub fn into_value(self) -> Value {
        Value::Opaque(self.handle)
    }

    /// The plain handle, +1 rc — for storing the box inside other
    /// host state or passing it in an args slice.
    pub fn value(&self) -> Value {
        Value::Opaque(self.handle.clone())
    }
}

impl<T: 'static> Clone for OpaqueBox<T> {
    fn clone(&self) -> Self {
        OpaqueBox { handle: self.handle.clone(), _marker: std::marker::PhantomData }
    }
}
