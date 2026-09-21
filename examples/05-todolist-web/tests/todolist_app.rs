//! The app's twin gate: the REAL `todolist.rut` (plus the linked
//! `store.rut`) driven end to end on the fake DOM — scripted sessions
//! asserting the tree AND the turn order. The laws checked here:
//!
//! * an add is a REQUEST — the pending row paints now, the list
//!   changes only when the timer answers (two outstanding adds commit
//!   in deadline-then-sequence order; toggle 250 / add 400 / remove
//!   120 make deadline order visible ACROSS kinds);
//! * input values are EVENT-CARRIED (the field's value rides the
//!   typing rows' detail; the click row carries none);
//! * the listener table is rut's own — re-registration per paint,
//!   stale ids trap LOUD (the app's unknown-id shape);
//! * the re-entrancy guard's absorb behavior at app scale;
//! * unknown event kinds trap loud.
//!
//! The store's own laws are `tests/store.rs` (DOM-free); the phase-1
//! crossing and trap matrix is `tests/host_surface.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use rut_driver::Session;
use rut_vm::interp::{HostHooks, HostRegistry, Vm};
use rut_vm::OpaqueRef;

use todolist_web::fake_dom::FakeDom;
use todolist_web::host::WebHost;
use todolist_web::{hosts, mount, state, WebState};

const APP: &str = include_str!("../todolist.rut");

/// Boot the app on the twin: mount (core + pouch + nmap_host + nmapset
/// + store + web), compile, bind BOTH body sets, `verify_against`,
/// `Vm::new`, seed the static page, run the boot turn — which returns
/// the app container the pump re-passes every turn.
fn make_host() -> (WebHost<FakeDom>, OpaqueRef) {
    let mut session = Session::new();
    mount::mount_app_session(&mut session).expect("the app session mounts");
    let expected = session.expected_host_fns();
    let prog = mount::compile_app(&mut session, APP).expect("the app compiles");

    let (slot, sink) = state::weak_sink_slot::<FakeDom>();
    let shared = Rc::new(RefCell::new(WebState::new(FakeDom::new(sink))));
    state::bind_weak_sink(&slot, &shared);
    shared.borrow_mut().dom.seed_page("div", "app"); // the twin's index.html

    let mut hosts = HostRegistry::new();
    hosts::install_web_hosts(&mut hosts, &shared);
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected);

    let vm = Vm::new(Rc::new(prog), &mount::limits(), HostHooks::default(), hosts)
        .expect("the vm boots");
    let mut host = WebHost::new(shared, vm);
    let app = host.boot().expect("the boot turn runs");
    (host, app)
}

/// One element's shape, read by the id the page itself set.
fn snap(host: &WebHost<FakeDom>, id: &str) -> todolist_web::fake_dom::Snapshot {
    host.with_dom(|d| {
        d.snapshot_by_id(id).unwrap_or_else(|| panic!("no element '#{id}' in the twin's tree"))
    })
}

/// The user types into the field, then the field's "input" listener
/// fires — the value travels as the event row's detail.
fn type_into(host: &mut WebHost<FakeDom>, text: &str) {
    assert!(host.state.borrow_mut().dom.set_value_by_id("new-todo", text));
    host.fire_listener(2).unwrap();
}

/// The deterministic listener-id ledger. Boot registers 1 (add-btn
/// click) and 2 (field input); every paint mints two ids per committed
/// row, in todo order. `rows` tracks the LIVE ids only.
#[derive(Clone)]
struct Ledger {
    next: i64,
    rows: Vec<(i64 /*todo*/, i64 /*mark*/, i64 /*del*/)>,
}

impl Ledger {
    fn start() -> Ledger {
        Ledger { next: 3, rows: Vec::new() }
    }
    /// The next paint, given the todos it will render in order.
    fn paint(&mut self, todos: &[i64]) {
        self.rows = todos
            .iter()
            .map(|t| {
                let m = self.next;
                let d = m + 1;
                self.next += 2;
                (*t, m, d)
            })
            .collect();
    }
    fn mark_of(&self, todo: i64) -> i64 {
        self.rows.iter().find(|r| r.0 == todo).expect("live row").1
    }
    fn del_of(&self, todo: i64) -> i64 {
        self.rows.iter().find(|r| r.0 == todo).expect("live row").2
    }
}

// ---- the boot shape ----

#[test]
fn boot_builds_the_shell_and_paints_empty() {
    let (host, _app) = make_host();
    let app = snap(&host, "app");
    assert_eq!(app.child_tags, vec!["input", "button", "p", "ul"]);
    let field = snap(&host, "new-todo");
    assert_eq!(field.attrs.get("placeholder").unwrap(), "What needs doing?");
    let add = snap(&host, "add-btn");
    assert_eq!(add.text.as_deref(), Some("Add"));
    let list = snap(&host, "list");
    assert!(list.child_tags.is_empty());
    assert_eq!(list.child_texts, Vec::<String>::new());
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 0 in flight — booted — type a title, press Add")
    );
    // no requests at boot — the page books nothing
    assert!(host.with_dom(|d| d.pending_timers()).is_empty());
}

// ---- the add round trip, one and two outstanding ----

#[test]
fn an_add_is_a_request_until_the_timer_answers() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    host.fire_listener(1).unwrap();

    // the pending row paints NOW; no committed row exists; one timer
    let list = snap(&host, "list");
    assert_eq!(list.child_tags, vec!["li"]);
    assert_eq!(list.child_texts, vec!["... milk"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 1 in flight — requested add 'milk'")
    );
    assert_eq!(host.with_dom(|d| d.pending_timers()), vec![(400, "req:1".to_string())]);
    // the field cleared: the draft moved into the request
    assert_eq!(snap(&host, "new-todo").attrs.get("value").unwrap(), "");

    // the answer lands as its own turn
    host.advance(400).unwrap();
    let list = snap(&host, "list");
    assert_eq!(list.child_tags, vec!["li"]);
    assert_eq!(list.child_texts, Vec::<String>::new(), "no pending rows remain");
    let row = snap(&host, "row-1");
    assert_eq!(row.child_texts, vec!["[ ]", "milk", "del"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("1 open | 0 done | 0 in flight — added 'milk' as #1")
    );
}

#[test]
fn two_outstanding_adds_commit_in_deadline_then_sequence_order() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    host.fire_listener(1).unwrap();
    type_into(&mut host, "tea");
    host.fire_listener(1).unwrap();

    // both pending rows, book order; both timers share the deadline
    let list = snap(&host, "list");
    assert_eq!(list.child_tags, vec!["li", "li"]);
    assert_eq!(list.child_texts, vec!["... milk", "... tea"]);
    assert_eq!(
        host.with_dom(|d| d.pending_timers()),
        vec![(400, "req:1".to_string()), (400, "req:2".to_string())]
    );

    // one advance fires both, seq order: milk commits first, tea second
    host.advance(400).unwrap();
    assert_eq!(snap(&host, "row-1").child_texts, vec!["[ ]", "milk", "del"]);
    assert_eq!(snap(&host, "row-2").child_texts, vec!["[ ]", "tea", "del"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("2 open | 0 done | 0 in flight — added 'tea' as #2")
    );
    assert!(host.with_dom(|d| d.pending_timers()).is_empty());
}

#[test]
fn input_values_are_event_carried_and_the_latest_wins() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "mi");
    type_into(&mut host, "milk");
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 0 in flight — typing 'milk'")
    );
    host.fire_listener(1).unwrap();
    host.advance(400).unwrap();
    // the draft came from the typing rows' details — "mi" never books
    assert_eq!(snap(&host, "row-1").child_texts, vec!["[ ]", "milk", "del"]);
}

#[test]
fn an_empty_draft_is_gated_client_side() {
    let (mut host, _app) = make_host();
    host.fire_listener(1).unwrap(); // click with nothing typed
    assert!(host.with_dom(|d| d.pending_timers()).is_empty(), "no request booked");
    assert_eq!(snap(&host, "list").child_tags, Vec::<String>::new());
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 0 in flight — type a title first")
    );
}

#[test]
fn the_server_rejects_a_duplicate_as_an_answer() {
    let (mut host, _app) = make_host();
    for _ in 0..2 {
        type_into(&mut host, "milk");
        host.fire_listener(1).unwrap();
        host.advance(400).unwrap();
    }
    // the second round trip REJECTED; the list holds one row
    assert_eq!(snap(&host, "list").child_tags, vec!["li"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("1 open | 0 done | 0 in flight — rejected 'milk' — already on the list")
    );
}

// ---- toggle and remove round trips; deadline order across kinds ----

#[test]
fn toggle_round_trip_paints_the_flight() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    host.fire_listener(1).unwrap();
    host.advance(400).unwrap();

    let mut led = Ledger::start();
    led.paint(&[1]);
    host.fire_listener(led.mark_of(1)).unwrap();
    // the row shows the in-flight mark while the answer travels
    assert_eq!(snap(&host, "row-1").child_texts, vec!["[ ] ...", "milk", "del"]);
    // the deadline is ABSOLUTE in the twin clock: booked at now=400
    assert_eq!(host.with_dom(|d| d.pending_timers()), vec![(650, "req:2".to_string())]);
    host.advance(250).unwrap();
    assert_eq!(snap(&host, "row-1").child_texts, vec!["[x]", "milk", "del"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 1 done | 0 in flight — toggled #1 to done")
    );
}

#[test]
fn remove_round_trip() {
    let (mut host, _app) = make_host();
    for title in ["milk", "tea"] {
        type_into(&mut host, title);
        host.fire_listener(1).unwrap();
        host.advance(400).unwrap();
    }
    let mut led = Ledger::start();
    led.paint(&[1]); // milk's commit paint
    led.paint(&[1]); // tea's add-REQUEST turn repaints [1]
    led.paint(&[1, 2]); // tea's commit paint
    host.fire_listener(led.del_of(1)).unwrap();
    led.paint(&[1, 2]); // the remove-request repaint
    host.advance(120).unwrap();
    // the removal is fast: milk's row is gone, tea is now first
    assert_eq!(snap(&host, "list").child_tags, vec!["li"]);
    assert_eq!(snap(&host, "row-2").child_texts, vec!["[ ]", "tea", "del"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("1 open | 0 done | 0 in flight — removed 'milk' (#1)")
    );
}

#[test]
fn deadline_order_across_kinds_is_visible_between_advances() {
    let (mut host, _app) = make_host();
    for title in ["milk", "tea"] {
        type_into(&mut host, title);
        host.fire_listener(1).unwrap();
        host.advance(400).unwrap();
    }
    let mut led = Ledger::start();
    led.paint(&[1]); // milk's commit paint
    led.paint(&[1]); // tea's add-REQUEST repaint
    led.paint(&[1, 2]); // tea's commit paint

    // a slow add and a fast toggle, booked back to back
    type_into(&mut host, "jam");
    host.fire_listener(1).unwrap(); // req:3 @ 400
    led.paint(&[1, 2]); // the add-request turn repaints the rows
    host.fire_listener(led.mark_of(1)).unwrap(); // req:4 @ 250
    led.paint(&[1, 2]); // the toggle-request turn repaints them again
    // the twin clock is absolute: booked at now=800
    assert_eq!(
        host.with_dom(|d| d.pending_timers()),
        vec![(1050, "req:4".to_string()), (1200, "req:3".to_string())]
    );

    // 250ms: the TOGGLE lands first (milk flips), the add still flies
    host.advance(250).unwrap();
    assert_eq!(snap(&host, "row-1").child_texts, vec!["[x]", "milk", "del"]);
    assert_eq!(snap(&host, "list").child_tags, vec!["li", "li", "li"]);
    assert_eq!(snap(&host, "list").child_texts, vec!["... jam"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("1 open | 1 done | 1 in flight — toggled #1 to done")
    );

    // 150ms more: the add lands, appended last
    host.advance(150).unwrap();
    assert_eq!(snap(&host, "list").child_tags, vec!["li", "li", "li"]);
    assert!(snap(&host, "list").child_texts.is_empty());
    assert_eq!(snap(&host, "row-3").child_texts, vec!["[ ]", "jam", "del"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("2 open | 1 done | 0 in flight — added 'jam' as #3")
    );
}

#[test]
fn a_lost_id_is_an_answer_not_a_wedge() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    host.fire_listener(1).unwrap();
    host.advance(400).unwrap();

    let mut led = Ledger::start();
    led.paint(&[1]);
    // book a slow toggle and a fast remove on the same row — the mark
    // first, because each request turn repaints and rotates the ids
    host.fire_listener(led.mark_of(1)).unwrap(); // req:2 @ 250
    led.paint(&[1]); // the toggle-request repaint
    host.fire_listener(led.del_of(1)).unwrap(); // req:3 @ 120
    led.paint(&[1]); // the remove-request repaint
    host.advance(120).unwrap();
    assert_eq!(snap(&host, "list").child_tags, Vec::<String>::new());
    host.advance(130).unwrap();
    // the toggle's answer names the loss; the page keeps working
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 0 in flight — skipped toggle #1 — the row is gone")
    );
    assert!(host.with_dom(|d| d.pending_timers()).is_empty());}

// ---- the app's own trap shapes ----

#[test]
fn a_stale_row_listener_traps_loud() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    host.fire_listener(1).unwrap();
    host.advance(400).unwrap(); // paint A: milk's ids are 3,4

    let mut led = Ledger::start();
    led.paint(&[1]);
    let stale_mark = led.mark_of(1);

    type_into(&mut host, "tea");
    host.fire_listener(1).unwrap();
    host.advance(400).unwrap(); // paint B: milk re-registered as 5,6

    // paint B dropped id 3 from rut's table — firing it is the app's
    // LOUD unknown-id trap, not a silent no-op
    let err = host.fire_listener(stale_mark).unwrap_err();
    assert!(
        err.msg.contains("app: listener '3' answered no action — stale or unknown id"),
        "{}",
        err.msg
    );
    // and the LIVE ids still work after the trapped turn
    let mut led2 = Ledger::start();
    led2.paint(&[1]); // milk's commit paint
    led2.paint(&[1]); // tea's add-REQUEST repaint
    led2.paint(&[1, 2]); // tea's commit paint
    host.fire_listener(led2.mark_of(2)).unwrap();
    host.advance(250).unwrap();
    assert_eq!(snap(&host, "row-2").child_texts, vec!["[x]", "tea", "del"]);
}

#[test]
fn an_unknown_event_kind_traps_loud() {
    let (mut host, app) = make_host();
    let err = host
        .call::<_, ()>("on_event", (app, 7i32, "1".to_string(), "".to_string()))
        .unwrap_err();
    assert!(err.msg.contains("app: unknown event kind 7"), "{}", err.msg);
}

#[test]
fn a_foreign_container_traps_loud() {
    let (mut host, app) = make_host();
    // the store surface answers its own container — a different type,
    // so on_event's downcast fails LOUD (host drift, named as such)
    let foreign: OpaqueRef = host.call("store_new", ()).unwrap();
    let err = host
        .call::<_, ()>("on_event", (foreign, 1i32, "1".to_string(), "".to_string()))
        .unwrap_err();
    assert!(err.msg.contains("app: the event container is not an AppRoot"), "{}", err.msg);
    // the real container is unharmed: a live turn still runs
    type_into(&mut host, "milk");
    host.fire_listener(1).unwrap();
    host.advance(400).unwrap();
    assert_eq!(snap(&host, "row-1").child_texts, vec!["[ ]", "milk", "del"]);
    let _ = app;
}

// ---- the guard's absorb behavior, at app scale ----

#[test]
fn events_firing_mid_turn_queue_and_run_as_the_next_turn() {
    let (mut host, _app) = make_host();
    host.state.borrow_mut().dom.set_value_by_id("new-todo", "milk");
    let before = snap(&host, "status").text.unwrap_or_default();
    // a synchronous DOM dispatch inside a live turn (the focus() class):
    // the row can only join the queue — no nested turn runs
    host.in_sync_dom_dispatch(|h| h.state.borrow_mut().push_dom_event(2));
    assert_eq!(host.state.borrow().queue.len(), 1);
    assert_eq!(snap(&host, "status").text.unwrap_or_default(), before, "no turn ran mid-turn");
    // the next pump drains it, FIFO: the typing turn processed the carry
    host.pump().unwrap();
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 0 in flight — typing 'milk'")
    );
}

// ---- the scripted session: the whole story in one page life ----

#[test]
fn the_full_crud_session() {
    let (mut host, _app) = make_host();
    let mut led = Ledger::start();

    // add two
    for title in ["milk", "tea"] {
        type_into(&mut host, title);
        host.fire_listener(1).unwrap();
    }
    assert_eq!(snap(&host, "list").child_texts, vec!["... milk", "... tea"]);
    host.advance(400).unwrap();
    led.paint(&[1]); // milk's commit paint
    led.paint(&[1, 2]); // tea's commit paint
    assert_eq!(snap(&host, "list").child_tags, vec!["li", "li"]);

    // toggle the first
    host.fire_listener(led.mark_of(1)).unwrap();
    led.paint(&[1, 2]); // the toggle-request repaint
    host.advance(250).unwrap();
    led.paint(&[1, 2]); // the toggle-commit repaint
    assert_eq!(snap(&host, "row-1").child_texts[0], "[x]");

    // add a third
    type_into(&mut host, "jam");
    host.fire_listener(1).unwrap();
    led.paint(&[1, 2]); // the add-request repaint
    host.advance(400).unwrap();
    led.paint(&[1, 2, 3]); // the add-commit repaint
    assert_eq!(snap(&host, "list").child_tags, vec!["li", "li", "li"]);

    // remove the done one
    host.fire_listener(led.del_of(1)).unwrap();
    led.paint(&[1, 2, 3]); // the remove-request repaint
    host.advance(120).unwrap();
    led.paint(&[2, 3]); // the remove-commit repaint
    assert_eq!(snap(&host, "row-2").child_texts, vec!["[ ]", "tea", "del"]);
    assert_eq!(snap(&host, "row-3").child_texts, vec!["[ ]", "jam", "del"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("2 open | 0 done | 0 in flight — removed 'milk' (#1)")
    );
    assert!(host.with_dom(|d| d.pending_timers()).is_empty());
}
