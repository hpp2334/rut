//! `ink`'s host half (RFC 0028): the native-module functions
//! `rt:log::create_logger` / `rt:log::logger_log`.
//!
//! The logger's state is an `Opaque` handle owning the name string
//! (RFC 0014) — rut never sees the host's layout. The `rt:log` module is
//! mounted by the driver (`rut-driver`); a host installs the bodies.

use std::cell::RefCell;
use std::rc::Rc;

use rut_core::types::{TY_I32, TY_NIL, TY_OPAQUE, TY_STR};
use rut_vm::interp::Vm;
use rut_vm::Value;

/// Install `rt:log`'s bodies, routing messages to `sink`. The bindings
/// are TYPED (RFC 0025): the signatures match rut/rt/rt.d.rut, and
/// `Vm::verify_host_fns` checks the contract at load time.
pub fn install_std_log<F>(vm: &mut Vm, sink: F)
where
    F: FnMut(&str) + 'static,
{
    let sink = Rc::new(RefCell::new(sink));
    vm.register_host_fn_sig("rt:log::create_logger", vec![TY_STR], TY_OPAQUE, |vm, args| {
        let name = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let s = vm.heap.alloc_str(name)?;
        let p = vm.heap.alloc_opaque(s, rut_core::types::TY_STR)?;
        let ptr = unsafe { p.r };
        // owning handle: the box was minted this instant, so the Value
        // takes over the mint reference (see OpaqueBox::alloc)
        Ok(Value::Opaque(vm.heap.opaque_handle_take(ptr)))
    });
    vm.register_host_fn_sig(
        "rt:log::logger_log",
        vec![TY_OPAQUE, TY_I32, TY_STR],
        TY_NIL,
        move |vm, args| {
        // zero-copy: borrow the message straight out of the block store
        // (RFC 0023 §2) instead of cloning a `String` per log line
        if let Ok(msg) = vm.arg_bytes(2) {
            if let Ok(msg) = std::str::from_utf8(msg) {
                (sink.borrow_mut())(msg);
                return Ok(Value::Nil);
            }
        }
        // fall back to the copied crossing (non-UTF-8 or missing arg)
        if let Some(Value::Str(msg)) = args.get(2) {
            (sink.borrow_mut())(msg);
        }
        Ok(Value::Nil)
        },
    );
}
