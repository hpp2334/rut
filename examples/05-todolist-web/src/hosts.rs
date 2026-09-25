//! The 11 registry bindings (RFC 0025): the Rust shape IS the `.d.rut`
//! row, written ONCE generic over the backend — the wasm32 half and the
//! twin share every trap law. Fallible bodies (`Result<_, Trap>`) carry
//! backend messages verbatim under `web::<fn>: `; the handle plumbing
//! is `Opaque<El>` in, `Opaque<El>` out.

use std::cell::RefCell;
use std::rc::Rc;

use rut_vm::interp::{HostRegistry, Vm};
use rut_vm::{Opaque, OpaqueRef, Trap};

use crate::backend::DomBackend;
use crate::state::{box_element, web_trap, ListenerRow, WebState};

type Shared<D> = Rc<RefCell<WebState<D>>>;

/// Borrow the boxed element payload for the call's duration; a foreign
/// opaque is a loud trap naming the fn (never a silent zero).
fn element<D: DomBackend>(el: &OpaqueRef, fn_name: &str) -> Result<Opaque<D::El>, Trap> {
    Opaque::<D::El>::from_handle(el).map_err(|e| web_trap(fn_name, e.msg))
}

/// Run one backend op through the box borrow, squashing the two error
/// layers (the box's borrow guard, the backend's own message) into the
/// one `web::<fn>: ` trap shape. Generic over the payload `El` —
/// associated-type projection (`D::El`) does not drive inference.
fn backend_op<T, El: 'static>(
    b: &Opaque<El>,
    fn_name: &str,
    op: impl FnOnce(&El) -> Result<T, String>,
) -> Result<T, Trap> {
    match b.with(op) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(web_trap(fn_name, e)),
        Err(e) => Err(web_trap(fn_name, e.msg)),
    }
}

/// The two-box variant (parent/child ops).
fn backend_op2<T, El: 'static>(
    p: &Opaque<El>,
    c: &Opaque<El>,
    fn_name: &str,
    op: impl FnOnce(&El, &El) -> Result<T, String>,
) -> Result<T, Trap> {
    match p.with(|p| c.with(|c| op(p, c))) {
        Ok(Ok(Ok(v))) => Ok(v),
        Ok(Ok(Err(e))) => Err(web_trap(fn_name, e)), // the backend message
        Ok(Err(e)) => Err(web_trap(fn_name, e.msg)), // a borrow-guard trap
        Err(e) => Err(web_trap(fn_name, e.msg)),     // the outer guard trap
    }
}

/// Install `web`'s bodies. Registration is BEFORE `Vm::new` (the eager
/// join, RFC 0025); `verify_against` against the mounting session's
/// `expected_host_fns` checks the contract both ways before any rut code
/// runs.
pub fn install_web_hosts<D: DomBackend>(hosts: &mut HostRegistry, st: &Shared<D>) {
    // ---- ui_get(id: str) -> opaque ----
    hosts.register::<_, (&str,), OpaqueRef, _>("web::ui_get", {
        let st = st.clone();
        move |vm: &mut Vm, id: &str| -> Result<OpaqueRef, Trap> {
            let el = st.borrow_mut().dom.get(id).map_err(|e| web_trap("ui_get", e))?;
            box_element::<D>(vm, el)
        }
    });

    // ---- ui_create(tag: str) -> opaque ----
    hosts.register::<_, (&str,), OpaqueRef, _>("web::ui_create", {
        let st = st.clone();
        move |vm: &mut Vm, tag: &str| -> Result<OpaqueRef, Trap> {
            let el = st.borrow_mut().dom.create(tag).map_err(|e| web_trap("ui_create", e))?;
            box_element::<D>(vm, el)
        }
    });

    // ---- ui_set_text(el: opaque, text: str) ----
    hosts.register::<_, (OpaqueRef, &str), (), _>("web::ui_set_text", {
        let st = st.clone();
        move |_vm: &mut Vm, el: OpaqueRef, text: &str| -> Result<(), Trap> {
            let b = element::<D>(&el, "ui_set_text")?;
            backend_op(&b, "ui_set_text", |el| st.borrow_mut().dom.set_text(el, text))
        }
    });

    // ---- ui_attr(el: opaque, name: str, value: str) ----
    hosts.register::<_, (OpaqueRef, &str, &str), (), _>("web::ui_attr", {
        let st = st.clone();
        move |_vm: &mut Vm, el: OpaqueRef, name: &str, value: &str| -> Result<(), Trap> {
            let b = element::<D>(&el, "ui_attr")?;
            backend_op(&b, "ui_attr", |el| st.borrow_mut().dom.attr(el, name, value))
        }
    });

    // ---- ui_append(parent: opaque, child: opaque) ----
    hosts.register::<_, (OpaqueRef, OpaqueRef), (), _>("web::ui_append", {
        let st = st.clone();
        move |_vm: &mut Vm, parent: OpaqueRef, child: OpaqueRef| -> Result<(), Trap> {
            let p = element::<D>(&parent, "ui_append")?;
            let c = element::<D>(&child, "ui_append")?;
            backend_op2(&p, &c, "ui_append", |p, c| st.borrow_mut().dom.append(p, c))
        }
    });

    // ---- ui_remove(parent: opaque, child: opaque) -> bool ----
    hosts.register::<_, (OpaqueRef, OpaqueRef), bool, _>("web::ui_remove", {
        let st = st.clone();
        move |_vm: &mut Vm, parent: OpaqueRef, child: OpaqueRef| -> Result<bool, Trap> {
            let p = element::<D>(&parent, "ui_remove")?;
            let c = element::<D>(&child, "ui_remove")?;
            backend_op2(&p, &c, "ui_remove", |p, c| st.borrow_mut().dom.remove(p, c))
        }
    });

    // ---- ui_clear(el: opaque) ----
    hosts.register::<_, (OpaqueRef,), (), _>("web::ui_clear", {
        let st = st.clone();
        move |_vm: &mut Vm, el: OpaqueRef| -> Result<(), Trap> {
            let b = element::<D>(&el, "ui_clear")?;
            backend_op(&b, "ui_clear", |el| st.borrow_mut().dom.clear(el))
        }
    });

    // ---- ui_set_input_value(el: opaque, v: str) ----
    hosts.register::<_, (OpaqueRef, &str), (), _>("web::ui_set_input_value", {
        let st = st.clone();
        move |_vm: &mut Vm, el: OpaqueRef, v: &str| -> Result<(), Trap> {
            let b = element::<D>(&el, "ui_set_input_value")?;
            backend_op(&b, "ui_set_input_value", |el| st.borrow_mut().dom.set_input_value(el, v))
        }
    });

    // ---- ui_listen(el: opaque, event: str) -> i64 ----
    // The glue allocates the id, the backend wires its callback, the
    // registry row keeps (element, event) — event rows carry the
    // element's current value when it IS an input (the detail law).
    hosts.register::<_, (OpaqueRef, &str), i64, _>("web::ui_listen", {
        let st = st.clone();
        move |_vm: &mut Vm, el: OpaqueRef, event: &str| -> Result<i64, Trap> {
            let b = element::<D>(&el, "ui_listen")?;
            let id = st.borrow_mut().next_listener_id();
            let el_row = backend_op(&b, "ui_listen", |el| {
                st.borrow_mut().dom.listen(el, event, id)?;
                Ok(el.clone())
            })?;
            st.borrow_mut()
                .listeners
                .insert(id, ListenerRow { el: el_row, event: event.to_string() });
            Ok(id)
        }
    });

    // ---- tim_after(ms: i64, tag: str) ----
    // A timer, not a scheduler: the host owns time (RFC 0018's own law),
    // the request/response story is rut's.
    hosts.register::<_, (i64, &str), (), _>("web::tim_after", {
        let st = st.clone();
        move |_vm: &mut Vm, ms: i64, tag: &str| -> Result<(), Trap> {
            st.borrow_mut().dom.after(ms, tag).map_err(|e| web_trap("tim_after", e))
        }
    });
}
