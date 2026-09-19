//! `ink`'s host half (RFC 0028): the native-module functions
//! `rt:log::create_logger` / `rt:log::logger_log`.
//!
//! The logger's state is an `opaque` handle owning the name string
//! (RFC 0014) — rut never sees the host's layout. The `rt:log` module is
//! mounted by the driver (`rut-driver`); a host installs the bodies.
//! Bindings are the MAGIC shape (RFC 0023/0025): the `&str` params are
//! zero-copy borrows of the block store, scoped to exactly the call by
//! the handler's HRTB — the old `arg_bytes` stash is gone.

use std::cell::RefCell;
use std::rc::Rc;

use rut_core::types::TY_OPAQUE;
use rut_vm::interp::{HostRegistry, Vm};
use rut_vm::Trap;
use rut_vm::OpaqueRef;

/// Install `rt:log`'s bodies, routing messages to `sink`. The bindings
/// are TYPED (RFC 0025): the derived signatures match rut/rt/rt.d.rut,
/// and `verify_against` checks the contract at load time.
pub fn install_std_log<F>(hosts: &mut HostRegistry, sink: F)
where
    F: FnMut(&str) + 'static,
{
    let sink = Rc::new(RefCell::new(sink));
    let sink2 = sink.clone();
    hosts.register::<_, (&str,), OpaqueRef, _>(
        "rt:log::create_logger",
        |vm: &mut Vm, name: &str| vm.alloc_opaque_str(name.to_string()),
    );
    hosts.register::<_, (OpaqueRef, i32, &str), (), _>(
        "rt:log::logger_log",
        move |_vm: &mut Vm, _logger: OpaqueRef, _level: i32, msg: &str| -> Result<(), Trap> {
            // zero-copy: `msg` borrows the block store for exactly this call
            (sink2.borrow_mut())(msg);
            Ok(())
        },
    );
}
