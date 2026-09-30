//! `ink`'s host half: the native-module functions
//! `ink_host::create_logger` / `ink_host::logger_log`.
//!
//! The logger's state is an `opaque` handle owning the name string
//! — rut never sees the host's layout. The `ink_host` module is
//! mounted by the driver (`rut-driver`); a host installs the bodies.
//! Bindings are the MAGIC shape: the `&str` params are
//! zero-copy borrows of the block store, scoped to exactly the call by
//! the handler's HRTB — the old `arg_bytes` stash is gone.

use std::cell::RefCell;
use std::rc::Rc;

use rut_vm::interp::{HostPkg, Vm};
use rut_vm::Trap;
use rut_vm::OpaqueRef;

/// Build `ink_host`'s pkg, routing messages to `sink`. The bindings
/// are TYPED: the derived signatures match
/// rut/ink_host/ink_host.d.rut, and the install checks the contract at
/// boot time.
pub fn pkg<F>(sink: F) -> HostPkg
where
    F: FnMut(&str) + 'static,
{
    let mut pkg = HostPkg::new("ink_host");
    let sink = Rc::new(RefCell::new(sink));
    let sink2 = sink.clone();
    pkg.register::<_, (&str,), OpaqueRef, _>(
        "create_logger",
        |vm: &mut Vm, name: &str| vm.alloc_opaque_str(name.to_string()),
    );
    pkg.register::<_, (OpaqueRef, i32, &str), (), _>(
        "logger_log",
        move |_vm: &mut Vm, _logger: OpaqueRef, _level: i32, msg: &str| -> Result<(), Trap> {
            // zero-copy: `msg` borrows the block store for exactly this call
            (sink2.borrow_mut())(msg);
            Ok(())
        },
    );
    pkg.build()
}
