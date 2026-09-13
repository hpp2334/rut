//! `std:log`'s host half (RFC 0028): the native-module functions
//! `rt:log::create_logger` / `rt:log::logger_log`.
//!
//! The logger's state is an `Opaque` handle owning the name string
//! (RFC 0014) — rut never sees the host's layout. The `rt:log` module is
//! mounted by the driver (`rut-driver`); a host installs the bodies.

use std::cell::RefCell;
use std::rc::Rc;

use rut_vm::interp::Vm;
use rut_vm::Value;

/// Install `rt:log`'s bodies, routing messages to `sink`.
pub fn install_std_log<F>(vm: &mut Vm, sink: F)
where
    F: FnMut(&str) + 'static,
{
    let sink = Rc::new(RefCell::new(sink));
    vm.register_host_fn("rt:log::create_logger", |vm, args| {
        let name = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let s = vm.heap.alloc_str(name)?;
        let p = vm.heap.alloc_opaque(s, rut_core::types::TY_STR)?;
        let ptr = unsafe { p.r };
        Ok(Value::Opaque(vm.heap.opaque_handle(ptr)))
    });
    vm.register_host_fn("rt:log::logger_log", move |_vm, args| {
        if let Some(Value::Str(msg)) = args.get(2) {
            (sink.borrow_mut())(msg);
        }
        Ok(Value::Unit)
    });
}
