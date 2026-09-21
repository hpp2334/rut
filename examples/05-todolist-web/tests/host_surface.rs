//! The twin gate (survey §5.4): the REAL `web.d.rut` surface bound to
//! the fake DOM, driven end to end through real rut calls — every
//! crossing, the full trap matrix (unknown id, kind mismatch, DOM
//! exception carried, the re-entrancy guard, listener drift), the
//! tim_after/on_event round trip, and the RFC 0025 boot contract both
//! ways. This is what keeps `cargo test --workspace` a meaningful gate
//! for a web example.

use std::cell::RefCell;
use std::rc::Rc;

use rut_driver::Session;
use rut_vm::interp::{HostHooks, HostRegistry, Vm};
use rut_vm::{OpaqueRef, Trap};

use todolist_web::fake_dom::FakeDom;
use todolist_web::host::WebHost;
use todolist_web::{hosts, mount, state, WebState};

const HARNESS: &str = include_str!("harness.rut");

/// Boot the harness on the twin: mount, compile, bind, `verify_against`
/// (the happy-path RFC 0025 check), `Vm::new`, seed the static page,
/// run the `main` turn. Every test's setup IS the boot contract.
fn make_host() -> WebHost<FakeDom> {
    let mut session = Session::new();
    mount::mount_host_session(&mut session).expect("the host session mounts");
    let expected = session.expected_host_fns();
    let prog = mount::compile_app(&mut session, HARNESS).expect("the harness compiles");

    let (slot, sink) = state::weak_sink_slot::<FakeDom>();
    let shared = Rc::new(RefCell::new(WebState::new(FakeDom::new(sink))));
    state::bind_weak_sink(&slot, &shared);
    shared.borrow_mut().dom.seed_page("div", "app"); // the twin's index.html

    let mut hosts = HostRegistry::new();
    hosts::install_web_hosts(&mut hosts, &shared);
    hosts.verify_against(&expected);

    let vm = Vm::new(Rc::new(prog), &mount::limits(), HostHooks::default(), hosts)
        .expect("the vm boots");
    let mut host = WebHost::new(shared, vm);
    host.boot().expect("the boot turn runs");
    host
}

fn log_rows(host: &mut WebHost<FakeDom>, log: &OpaqueRef) -> Vec<String> {
    host.with_dom(|d| d.child_texts_of(log).unwrap())
}

// ---- the boot shape ----

#[test]
fn rfc0025_the_full_contract_boots() {
    let _ = make_host();
}

#[test]
fn boot_builds_the_static_dom() {
    let mut host = make_host();
    let app: OpaqueRef = host.call("probe_get", ("app",)).unwrap();
    assert_eq!(
        host.with_dom(|d| d.children_of(&app).unwrap()),
        vec!["input", "button", "ul"]
    );
    let input: OpaqueRef = host.call("probe_get", ("new-todo",)).unwrap();
    assert_eq!(
        host.with_dom(|d| d.attr_of(&input, "placeholder").unwrap()),
        "What needs doing?"
    );
    let add: OpaqueRef = host.call("probe_get", ("add-btn",)).unwrap();
    assert_eq!(host.with_dom(|d| d.tag_of(&add).unwrap()), "button");
    assert_eq!(host.with_dom(|d| d.text_of(&add).unwrap()), "Add");
}

#[test]
fn listener_ids_are_from_one_and_increment() {
    let mut host = make_host();
    let log: OpaqueRef = host.call("probe_get", ("log",)).unwrap();
    // main registered 1..3; the next two mint 4 and 5 — duplicate
    // (element, event) registration is two rows, addEventListener's law
    let a: i64 = host.call("probe_listen", (log.clone(), "click")).unwrap();
    let b: i64 = host.call("probe_listen", (log.clone(), "click")).unwrap();
    assert_eq!((a, b), (4, 5));
    host.fire_listener(4).unwrap();
    host.fire_listener(5).unwrap();
    assert_eq!(log_rows(&mut host, &log), vec!["dom 4 ", "dom 5 "]);
}

// ---- the round trips ----

#[test]
fn dom_event_round_trips_through_on_event() {
    let mut host = make_host();
    host.fire_listener(1).unwrap(); // the add-btn "click" row
    let log: OpaqueRef = host.call("probe_get", ("log",)).unwrap();
    assert_eq!(log_rows(&mut host, &log), vec!["dom 1 "]);
}

#[test]
fn input_values_are_event_carried() {
    let mut host = make_host();
    let input: OpaqueRef = host.call("probe_get", ("new-todo",)).unwrap();
    host.call::<_, ()>("probe_set_input_value", (input.clone(), "milk")).unwrap();
    let log: OpaqueRef = host.call("probe_get", ("log",)).unwrap();
    // every event on an input element carries its current value —
    // "input" rows, keydown rows, all of them
    host.fire_listener(2).unwrap(); // the input's "input" row
    assert_eq!(log_rows(&mut host, &log), vec!["dom 2 milk"]);
    host.fire_listener(3).unwrap(); // the input's keydown row
    assert_eq!(log_rows(&mut host, &log), vec!["dom 2 milk", "dom 3 milk"]);
    // a non-input element's rows carry no detail
    host.fire_listener(1).unwrap(); // the add-btn click
    assert_eq!(log_rows(&mut host, &log), vec!["dom 2 milk", "dom 3 milk", "dom 1 "]);
}

#[test]
fn tim_after_round_trips_as_ev_timer() {
    let mut host = make_host();
    assert_eq!(host.with_dom(|d| d.pending_timers()), vec![(400, "boot".to_string())]);
    let log: OpaqueRef = host.call("probe_get", ("log",)).unwrap();
    host.advance(399).unwrap(); // not yet
    assert!(log_rows(&mut host, &log).is_empty());
    host.advance(1).unwrap(); // the deadline hits
    assert_eq!(log_rows(&mut host, &log), vec!["timer boot"]);
    assert!(host.with_dom(|d| d.pending_timers()).is_empty());
}

#[test]
fn timers_fire_in_deadline_then_sequence_order() {
    let mut host = make_host();
    host.call::<_, ()>("probe_after", (50i64, "a")).unwrap();
    host.call::<_, ()>("probe_after", (150i64, "b")).unwrap();
    host.call::<_, ()>("probe_after", (50i64, "c")).unwrap(); // same deadline, later
    let log: OpaqueRef = host.call("probe_get", ("log",)).unwrap();
    host.advance(50).unwrap();
    assert_eq!(log_rows(&mut host, &log), vec!["timer a", "timer c"]);
    host.advance(100).unwrap();
    assert_eq!(log_rows(&mut host, &log), vec!["timer a", "timer c", "timer b"]);
}

// ---- the trap matrix (survey §4.3) ----

#[test]
fn trap_unknown_id_is_loud() {
    let mut host = make_host();
    let err = host.call::<_, OpaqueRef>("probe_get", ("nope",)).unwrap_err();
    assert!(err.msg.contains("web::ui_get: no element '#nope'"), "{}", err.msg);
}

#[test]
fn trap_kind_mismatch_names_both_sides() {
    let mut host = make_host();
    let log: OpaqueRef = host.call("probe_get", ("log",)).unwrap(); // a `ul`
    let err = host.call::<_, ()>("probe_set_input_value", (log, "x")).unwrap_err();
    assert!(err.msg.contains("web::ui_set_input_value: boundary: got `ul` where `HtmlInputElement` binds"), "{}", err.msg);
}

#[test]
fn trap_foreign_opaque_is_not_an_element_handle() {
    let mut host = make_host();
    let junk: OpaqueRef = host.call("probe_box_int", ()).unwrap();
    let err = host.call::<_, ()>("probe_set_text", (junk, "x")).unwrap_err();
    assert!(err.msg.contains("web::ui_set_text:"), "{}", err.msg);
    assert!(err.msg.contains("not a host payload"), "{}", err.msg);
}

#[test]
fn trap_dom_exceptions_are_carried() {
    let mut host = make_host();
    let err = host.call::<_, OpaqueRef>("probe_create", ("bad tag",)).unwrap_err();
    assert!(err.msg.contains("web::ui_create: InvalidCharacterError"), "{}", err.msg);
    let app: OpaqueRef = host.call("probe_get", ("app",)).unwrap();
    let err = host.call::<_, ()>("probe_attr", (app, "bad name", "v")).unwrap_err();
    assert!(err.msg.contains("web::ui_attr: InvalidCharacterError"), "{}", err.msg);
}

#[test]
fn trap_append_hierarchy_carries_the_exception() {
    let mut host = make_host();
    let outer: OpaqueRef = host.call("probe_create", ("div",)).unwrap();
    let inner: OpaqueRef = host.call("probe_create", ("div",)).unwrap();
    host.call::<_, ()>("probe_append", (outer.clone(), inner.clone())).unwrap();
    // the ancestor into its own descendant
    let err = host.call::<_, ()>("probe_append", (inner.clone(), outer.clone())).unwrap_err();
    assert!(err.msg.contains("web::ui_append: HierarchyRequestError"), "{}", err.msg);
    // itself
    let err = host.call::<_, ()>("probe_append", (outer.clone(), outer)).unwrap_err();
    assert!(err.msg.contains("HierarchyRequestError"), "{}", err.msg);
}

#[test]
fn ui_remove_is_a_dom_negative_not_an_error() {
    let mut host = make_host();
    let app: OpaqueRef = host.call("probe_get", ("app",)).unwrap();
    let stray: OpaqueRef = host.call("probe_create", ("div",)).unwrap(); // detached
    let ok: bool = host.call("probe_remove", (app.clone(), stray)).unwrap();
    assert!(!ok, "removing a non-child is false, no trap");
    let input: OpaqueRef = host.call("probe_get", ("new-todo",)).unwrap();
    let ok: bool = host.call("probe_remove", (app, input)).unwrap();
    assert!(ok);
}

#[test]
fn ui_clear_resets_the_subtree() {
    let mut host = make_host();
    let log: OpaqueRef = host.call("probe_get", ("log",)).unwrap();
    host.fire_listener(1).unwrap();
    assert_eq!(log_rows(&mut host, &log).len(), 1);
    host.call::<_, ()>("probe_clear", (log.clone(),)).unwrap();
    assert!(log_rows(&mut host, &log).is_empty());
    // and the page keeps working after the reset
    host.fire_listener(1).unwrap();
    assert_eq!(log_rows(&mut host, &log), vec!["dom 1 "]);
}

// ---- the re-entrancy guard (survey §4.3 shape 4) ----

#[test]
#[should_panic(expected = "web: event during a rut turn — events are queue, never stack")]
fn guard_pumping_inside_a_live_turn_is_the_forbidden_stack() {
    let mut host = make_host();
    host.in_sync_dom_dispatch(|h| h.pump().unwrap());
}

#[test]
fn guard_events_firing_mid_turn_are_queued_not_stacked() {
    let mut host = make_host();
    let log: OpaqueRef = host.call("probe_get", ("log",)).unwrap();
    // a synchronous DOM dispatch during a live turn (the focus()/
    // click() class): the row can only join the queue
    host.in_sync_dom_dispatch(|h| h.state.borrow_mut().push_dom_event(1));
    assert_eq!(host.state.borrow().queue.len(), 1);
    assert!(log_rows(&mut host, &log).is_empty(), "no nested turn ran");
    host.pump().unwrap(); // the next turn drains it, FIFO
    assert_eq!(log_rows(&mut host, &log), vec!["dom 1 "]);
}

#[test]
#[should_panic(expected = "web: listener 99 is not registered")]
fn guard_stale_listener_ids_are_host_drift() {
    let mut host = make_host();
    host.fire_listener(99).unwrap();
}

// ---- the RFC 0025 boot contract, both ways ----

fn mounted_session() -> Session {
    let mut s = Session::new();
    mount::mount_host_session(&mut s).expect("the host session mounts");
    s
}

#[test]
fn the_app_compiles() {
    // the phase-2 app (todolist.rut + the linked store.rut) is this
    // example's page program — its compile gate rides the app session
    // (the phase-1 shell demo grew into it; loader.js fetches it)
    let mut session = Session::new();
    mount::mount_app_session(&mut session).expect("the app session mounts");
    mount::compile_app(&mut session, include_str!("../todolist.rut"))
        .expect("the app compiles");
}

#[test]
#[should_panic(expected = "declared by a mounted package but never bound")]
fn rfc0025_declared_but_unbound_panics_at_verify() {
    let session = mounted_session();
    let expected = session.expected_host_fns();
    let hosts = HostRegistry::new(); // not a single row bound
    hosts.verify_against(&expected);
}

#[test]
fn rfc0025_declared_but_unbound_is_a_vm_construction_error() {
    let mut session = mounted_session();
    let prog = mount::compile_app(&mut session, HARNESS).expect("the harness compiles");
    let err = match Vm::new(Rc::new(prog), &mount::limits(), HostHooks::default(), HostRegistry::new())
    {
        Ok(_) => panic!("the unbound registry must not boot"),
        Err(e) => e,
    };
    assert!(
        err.msg.contains("declared by the program but never bound"),
        "{}",
        err.msg
    );
}

#[test]
#[should_panic(expected = "bound but declared by no mounted package")]
fn rfc0025_bound_but_undeclared_panics_at_verify() {
    // NO `web` module mounted — core only. The binding's shape is the
    // correct one; the SURFACE is what's missing.
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    let expected = session.expected_host_fns();
    let mut hosts = HostRegistry::new();
    hosts.register::<_, (&str,), OpaqueRef, _>("web::ui_get", {
        move |_vm: &mut Vm, _id: &str| -> Result<OpaqueRef, Trap> {
            unreachable!("never called — the contract check runs before any rut code")
        }
    });
    hosts.verify_against(&expected);
}

#[test]
#[should_panic(expected = "signature drift")]
fn rfc0025_signature_drift_panics_at_verify() {
    let session = mounted_session();
    let expected = session.expected_host_fns();
    let (slot, sink) = state::weak_sink_slot::<FakeDom>();
    let shared = Rc::new(RefCell::new(WebState::new(FakeDom::new(sink))));
    state::bind_weak_sink(&slot, &shared);
    let mut hosts = HostRegistry::new();
    hosts::install_web_hosts(&mut hosts, &shared);
    // rebind tim_after with the WRONG shape: (str) instead of (i64, str)
    hosts.register::<_, (String,), (), _>("web::tim_after", {
        move |_vm: &mut Vm, _tag: String| -> Result<(), Trap> {
            unreachable!("never called — the contract check runs first")
        }
    });
    hosts.verify_against(&expected);
}
