//! The store's DOM-FREE gate: the "server" half driven through its
//! entry surface on a plain Vm — no `web` surface mounted, no DOM, no
//! timers. The laws checked here: a request never touches the list (the
//! list changes only when `answer` lands), answers are data (rejections
//! and lost ids), an unknown tag traps loud, latencies differ per kind
//! (the app-level deadline order rides on this), and the round trips
//! for add/toggle/remove close.

use std::rc::Rc;

use rut_vm::interp::{HostHooks, HostRegistry, Vm};
use rut_vm::OpaqueRef;

use todolist_web::mount;

const STORE: &str = include_str!("../store.rut");

/// core + pouch ONLY — no `web`, no `nmap_host`: the store compiles and
/// runs with no page and no clock in the session at all.
fn vm() -> (Vm, OpaqueRef) {
    let mut session = Session::new();
    mount::mount_store_session(&mut session).expect("the store session mounts");
    let prog = mount::compile_app(&mut session, STORE).expect("the store compiles");
    let mut v = Vm::new(Rc::new(prog), &mount::limits(), HostHooks::default(), HostRegistry::new())
        .expect("the vm boots");
    let c: OpaqueRef = v.call("store_new", ()).unwrap();
    (v, c)
}

use rut_driver::Session;

#[test]
fn a_request_never_touches_the_list() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    assert_eq!(tag, "req:1");
    // the board is empty until the answer lands — an add IS a request
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "");
    assert_eq!(v.call::<_, String>("store_counts", (c.clone(),)).unwrap(), "open=0 done=0 fly=1");
    assert_eq!(v.call::<_, String>("store_fly", (c.clone(),)).unwrap(), "req:1@400");
    assert_eq!(v.call::<_, String>("store_last", (c.clone(),)).unwrap(), "");
}

#[test]
fn the_answer_commits_the_add() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    assert_eq!(
        v.call::<_, String>("store_answer", (c.clone(), tag)).unwrap(),
        "added 'milk' as #1"
    );
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] milk");
    assert_eq!(v.call::<_, String>("store_counts", (c.clone(),)).unwrap(), "open=1 done=0 fly=0");
    assert_eq!(v.call::<_, String>("store_last", (c.clone(),)).unwrap(), "added 'milk' as #1");
}

#[test]
fn toggle_round_trip() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, String>("store_answer", (c.clone(), tag)).unwrap();

    let t2: String = v.call("store_request_toggle", (c.clone(), 1i64)).unwrap();
    assert_eq!(t2, "req:2");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] milk");
    let line = v.call::<_, String>("store_answer", (c.clone(), t2)).unwrap();
    assert_eq!(line, "toggled #1 to done");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [x] milk");
    assert_eq!(v.call::<_, String>("store_counts", (c.clone(),)).unwrap(), "open=0 done=1 fly=0");
}

#[test]
fn remove_round_trip() {
    let (mut v, c) = vm();
    for title in ["milk", "tea"] {
        let tag: String = v.call("store_request_add", (c.clone(), title)).unwrap();
        v.call::<_, String>("store_answer", (c.clone(), tag)).unwrap();
    }
    let t3: String = v.call("store_request_remove", (c.clone(), 1i64)).unwrap();
    assert_eq!(t3, "req:3");
    let line = v.call::<_, String>("store_answer", (c.clone(), t3)).unwrap();
    assert_eq!(line, "removed 'milk' (#1)");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#2 [ ] tea");
}

#[test]
fn latencies_differ_per_kind() {
    let (mut v, c) = vm();
    let a: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, String>("store_answer", (c.clone(), a)).unwrap();
    let t: String = v.call("store_request_toggle", (c.clone(), 1i64)).unwrap();
    let _ = t;
    let r: String = v.call("store_request_remove", (c.clone(), 1i64)).unwrap();
    let _ = r;
    // adds are slow, removes are fast — the app's deadline order rides
    // on exactly these three numbers
    assert_eq!(
        v.call::<_, String>("store_fly", (c.clone(),)).unwrap(),
        "req:2@250|req:3@120"
    );
}

#[test]
fn a_rejected_title_is_an_answer_not_a_trap() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, String>("store_answer", (c.clone(), tag)).unwrap();

    // the duplicate
    let d: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    let line = v.call::<_, String>("store_answer", (c.clone(), d)).unwrap();
    assert_eq!(line, "rejected 'milk' — already on the list");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] milk");
    assert_eq!(v.call::<_, String>("store_counts", (c.clone(),)).unwrap(), "open=1 done=0 fly=0");

    // the empty title (the app's client gate never sends one; the
    // server rejects it anyway)
    let e: String = v.call("store_request_add", (c.clone(), "")).unwrap();
    let line = v.call::<_, String>("store_answer", (c.clone(), e)).unwrap();
    assert_eq!(line, "rejected '' — the title is empty");
}

#[test]
fn an_unknown_tag_traps_loud() {
    let (mut v, c) = vm();
    let err = v.call::<_, String>("store_answer", (c.clone(), "req:9".to_string())).unwrap_err();
    assert!(
        err.msg.contains("server: no request 'req:9'"),
        "{}",
        err.msg
    );
}

#[test]
fn an_unknown_todo_answers_an_empty_tag() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_toggle", (c.clone(), 42i64)).unwrap();
    assert_eq!(tag, "", "the UI turns this into its loud listener-drift trap");
    let tag: String = v.call("store_request_remove", (c.clone(), 42i64)).unwrap();
    assert_eq!(tag, "");
}

#[test]
fn a_lost_id_is_an_answer_through_the_table() {
    let (mut v, c) = vm();
    let a: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, String>("store_answer", (c.clone(), a)).unwrap();

    // book a remove (fast) and a toggle (slow) on the same todo; the
    // remove's answer lands first and the toggle's id is lost — the
    // answer SAYS so instead of trapping or wedging
    let r: String = v.call("store_request_remove", (c.clone(), 1i64)).unwrap();
    let t: String = v.call("store_request_toggle", (c.clone(), 1i64)).unwrap();
    assert_eq!(v.call::<_, String>("store_answer", (c.clone(), r)).unwrap(), "removed 'milk' (#1)");
    assert_eq!(
        v.call::<_, String>("store_answer", (c.clone(), t)).unwrap(),
        "skipped toggle #1 — the row is gone"
    );
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "");
}
