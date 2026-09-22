//! The app's twin gate: the REAL `todolist.rut` (plus the linked
//! `store.rut` and the inlined `t1.rut`) driven end to end on the fake
//! DOM — scripted sessions asserting the tree AND the turn order, on
//! the LOWERED DOM (the framework's tags and tokens). The laws checked
//! here are the phase-1 laws, re-proven through the widgets:
//!
//! * an add is a REQUEST — the pending line paints now, the list
//!   changes only when the timer answers (two outstanding adds commit
//!   in deadline-then-sequence order; toggle 250 / add 400 / remove
//!   120 make deadline order visible ACROSS kinds);
//! * input values are EVENT-CARRIED (the field's value rides the
//!   typing rows' detail; the click row carries none);
//! * a stale or unknown listener id traps LOUD — now in the
//!   framework's registry (`t1_subject`), since repaints no longer
//!   churn ids: a row's listener lives as long as the row does, and
//!   dies with it;
//! * the re-entrancy guard's absorb behavior at app scale;
//! * unknown event kinds trap loud.
//!
//! The tests' vocabulary is HOOKS (§6 of the design): a listener id is
//! found through the element the page itself tagged, never predicted
//! from the registration counter — the phase-1 `Ledger` (id-churn
//! arithmetic) is dead, and good riddance. The store's own laws are
//! `tests/store.rs` (DOM-free); the phase-1 crossing and trap matrix
//! is `tests/host_surface.rs`; the framework's own suite is
//! `tests/t1_lowering.rs` + `tests/t1_diff.rs`; the biz-law gate is
//! `tests/app_law.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use rut_driver::Session;
use rut_vm::interp::{HostHooks, HostRegistry, Vm};
use rut_vm::OpaqueRef;

use todolist_web::fake_dom::{FakeDom, Snapshot};
use todolist_web::host::WebHost;
use todolist_web::{hosts, mount, state, DomBackend, WebState};

const APP: &str = include_str!("../todolist.rut");

/// Boot the app on the twin: mount (core + pouch + nmap_host + nmapset
/// + store + web + t1 — the app session), compile, bind BOTH body sets,
/// `verify_against`, `Vm::new`, seed the static page, run the boot turn
/// — which mounts the framework, paints the first tree, and returns the
/// app container the pump re-passes every turn.
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

/// One element's shape, read by the hook the lowering emitted as `id`.
fn snap(host: &WebHost<FakeDom>, hook: &str) -> Snapshot {
    host.with_dom(|d| {
        d.snapshot_by_id(hook).unwrap_or_else(|| panic!("no element '#{hook}' in the twin's tree"))
    })
}

/// The class attribute — the lowered token string the twin asserts
/// (the twin ignores CSS; the TOKENS are the styling contract, §5).
fn class_of(host: &WebHost<FakeDom>, hook: &str) -> String {
    snap(host, hook).attrs.get("class").cloned().unwrap_or_default()
}

/// The host listener row serving `hook`'s element (§6: hooks are the
/// tests' vocabulary): find the element through the tree, match its
/// node against the host's own listener rows. No id arithmetic — the
/// registration counter is plumbing.
fn listener_of(host: &WebHost<FakeDom>, hook: &str) -> i64 {
    let nid = host.with_dom(|d| d.get(hook).expect("hook present in the tree").id);
    host.state
        .borrow()
        .listeners
        .iter()
        .find(|(_, row)| row.el.id == nid)
        .map(|(id, _)| *id)
        .unwrap_or_else(|| panic!("no listener registered for #{hook}"))
}

/// The user types into the field, then the field's "input" listener
/// fires — the value travels as the event row's detail.
fn type_into(host: &mut WebHost<FakeDom>, text: &str) {
    assert!(host.state.borrow_mut().dom.set_value_by_id("new-todo", text));
    let id = listener_of(host, "new-todo");
    host.fire_listener(id).unwrap();
}

/// Click a widget by hook — the add button, a row's check, a del.
fn click(host: &mut WebHost<FakeDom>, hook: &str) {
    let id = listener_of(host, hook);
    host.fire_listener(id).unwrap();
}

// ---- the boot shape ----

#[test]
fn boot_builds_the_shell_and_paints_empty() {
    let (host, _app) = make_host();
    // the shell is ONE widget root under the seeded element — the
    // hand-built field/button/line/list quartet is the framework's
    // create path now
    let app = snap(&host, "app");
    assert_eq!(app.child_tags, vec!["div"]);
    // the root's tokens: a padded, gapped column (the ladder's rungs)
    assert_eq!(class_of(&host, "root"), "t1-col t1-gap-12 t1-pad-16");
    // the head row: field + primary Add, by hook and token
    let field = snap(&host, "new-todo");
    assert_eq!(field.tag, "input");
    assert_eq!(field.attrs.get("placeholder").unwrap(), "What needs doing?");
    assert_eq!(class_of(&host, "new-todo"), "t1-field");
    let add = snap(&host, "add-btn");
    assert_eq!(add.tag, "button");
    assert_eq!(add.text.as_deref(), Some("Add"));
    assert_eq!(class_of(&host, "add-btn"), "t1-btn t1-btn--primary");
    // the counts line and the (empty) list card
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 0 in flight — booted — type a title, press Add")
    );
    assert_eq!(class_of(&host, "status"), "t1-text t1-text--muted");
    let list = snap(&host, "list");
    assert_eq!(list.tag, "div");
    assert!(list.child_tags.is_empty());
    assert_eq!(list.child_texts, Vec::<String>::new());
    // no requests at boot — the page books nothing
    assert!(host.with_dom(|d| d.pending_timers()).is_empty());
}

// ---- the add round trip, one and two outstanding ----

#[test]
fn an_add_is_a_request_until_the_timer_answers() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    click(&mut host, "add-btn");

    // the pending line paints NOW; no committed row exists; one timer
    let list = snap(&host, "list");
    assert_eq!(list.child_tags, vec!["span"]);
    assert_eq!(list.child_texts, vec!["... milk"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 1 in flight — requested add 'milk'")
    );
    assert_eq!(host.with_dom(|d| d.pending_timers()), vec![(400, "req:1".to_string())]);
    // the field cleared: the draft moved into the request — the DIFF
    // cleared it (the value patch rode the framework's input crossing)
    assert_eq!(snap(&host, "new-todo").attrs.get("value").unwrap(), "");

    // the answer lands as its own turn
    host.advance(400).unwrap();
    let list = snap(&host, "list");
    assert_eq!(list.child_tags, vec!["div"], "no pending lines remain");
    assert_eq!(list.child_texts, Vec::<String>::new());
    // the committed row: check + title + del, the lowered shape — the
    // checkbox contributes no text (its glyph is its class token)
    let row = snap(&host, "row-1");
    assert_eq!(row.child_tags, vec!["button", "span", "button"]);
    assert_eq!(row.child_texts, vec!["milk", "del"]);
    assert_eq!(class_of(&host, "mark-1"), "t1-check t1-check--off");
    assert_eq!(class_of(&host, "lbl-1"), "t1-text t1-text--body");
    assert_eq!(class_of(&host, "del-1"), "t1-btn t1-btn--quiet");
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("1 open | 0 done | 0 in flight — added 'milk' as #1")
    );
}

#[test]
fn two_outstanding_adds_commit_in_deadline_then_sequence_order() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    click(&mut host, "add-btn");
    type_into(&mut host, "tea");
    click(&mut host, "add-btn");

    // both pending lines, book order; both timers share the deadline
    let list = snap(&host, "list");
    assert_eq!(list.child_tags, vec!["span", "span"]);
    assert_eq!(list.child_texts, vec!["... milk", "... tea"]);
    assert_eq!(
        host.with_dom(|d| d.pending_timers()),
        vec![(400, "req:1".to_string()), (400, "req:2".to_string())]
    );

    // one advance fires both, seq order: milk commits first, tea second
    host.advance(400).unwrap();
    assert_eq!(snap(&host, "row-1").child_texts, vec!["milk", "del"]);
    assert_eq!(snap(&host, "row-2").child_texts, vec!["tea", "del"]);
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
    // the draft mirrors through the diff onto the input crossing
    assert_eq!(snap(&host, "new-todo").attrs.get("value").unwrap(), "mi");
    type_into(&mut host, "milk");
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 0 in flight — typing 'milk'")
    );
    click(&mut host, "add-btn");
    host.advance(400).unwrap();
    // the draft came from the typing rows' details — "mi" never books
    assert_eq!(snap(&host, "row-1").child_texts, vec!["milk", "del"]);
}

#[test]
fn an_empty_draft_is_gated_client_side() {
    let (mut host, _app) = make_host();
    click(&mut host, "add-btn"); // click with nothing typed
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
        click(&mut host, "add-btn");
        host.advance(400).unwrap();
    }
    // the second round trip REJECTED; the list holds one row
    assert_eq!(snap(&host, "list").child_tags, vec!["div"]);
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
    click(&mut host, "add-btn");
    host.advance(400).unwrap();

    click(&mut host, "mark-1");
    // the row shows the in-flight cue while the answer travels: the
    // title's variant patched to pending with the "... " text — one
    // token patch plus one text patch, the row's other widgets untouched
    assert_eq!(class_of(&host, "lbl-1"), "t1-text t1-text--pending");
    assert_eq!(snap(&host, "lbl-1").text.as_deref(), Some("... milk"));
    assert_eq!(class_of(&host, "mark-1"), "t1-check t1-check--off", "the flip is the commit's");
    // the deadline is ABSOLUTE in the twin clock: booked at now=400
    assert_eq!(host.with_dom(|d| d.pending_timers()), vec![(650, "req:2".to_string())]);
    host.advance(250).unwrap();
    // the commit: one token patch on the check, one on the title
    assert_eq!(class_of(&host, "mark-1"), "t1-check t1-check--on");
    assert_eq!(class_of(&host, "lbl-1"), "t1-text t1-text--done");
    assert_eq!(snap(&host, "lbl-1").text.as_deref(), Some("milk"));
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
        click(&mut host, "add-btn");
        host.advance(400).unwrap();
    }
    click(&mut host, "del-1");
    assert_eq!(class_of(&host, "lbl-1"), "t1-text t1-text--pending", "the flight paints");
    host.advance(120).unwrap();
    // the removal is fast: milk's row is gone, tea is now first
    assert_eq!(snap(&host, "list").child_tags, vec!["div"]);
    assert_eq!(snap(&host, "row-2").child_texts, vec!["tea", "del"]);
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
        click(&mut host, "add-btn");
        host.advance(400).unwrap();
    }

    // a slow add and a fast toggle, booked back to back
    type_into(&mut host, "jam");
    click(&mut host, "add-btn"); // req:3 @ 400
    click(&mut host, "mark-1"); // req:4 @ 250
    // the twin clock is absolute: booked at now=800
    assert_eq!(
        host.with_dom(|d| d.pending_timers()),
        vec![(1050, "req:4".to_string()), (1200, "req:3".to_string())]
    );

    // 250ms: the TOGGLE lands first (milk flips), the add still flies
    host.advance(250).unwrap();
    assert_eq!(class_of(&host, "mark-1"), "t1-check t1-check--on");
    assert_eq!(class_of(&host, "lbl-1"), "t1-text t1-text--done");
    assert_eq!(snap(&host, "list").child_tags, vec!["div", "div", "span"]);
    assert_eq!(snap(&host, "list").child_texts, vec!["... jam"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("1 open | 1 done | 1 in flight — toggled #1 to done")
    );

    // 150ms more: the add lands, appended last
    host.advance(150).unwrap();
    assert_eq!(snap(&host, "list").child_tags, vec!["div", "div", "div"]);
    assert!(snap(&host, "list").child_texts.is_empty());
    assert_eq!(snap(&host, "row-3").child_texts, vec!["jam", "del"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("2 open | 1 done | 0 in flight — added 'jam' as #3")
    );
}

#[test]
fn a_lost_id_is_an_answer_not_a_wedge() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    click(&mut host, "add-btn");
    host.advance(400).unwrap();

    // book a slow toggle and a fast remove on the same row
    click(&mut host, "mark-1"); // req:2 @ 250
    click(&mut host, "del-1"); // req:3 @ 120
    host.advance(120).unwrap();
    assert_eq!(snap(&host, "list").child_tags, Vec::<String>::new());
    host.advance(130).unwrap();
    // the toggle's answer names the loss; the page keeps working
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("0 open | 0 done | 0 in flight — skipped toggle #1 — the row is gone")
    );
    assert!(host.with_dom(|d| d.pending_timers()).is_empty());
}

// ---- the app's own trap shapes ----

#[test]
fn a_stale_row_listener_traps_loud() {
    let (mut host, _app) = make_host();
    type_into(&mut host, "milk");
    click(&mut host, "add-btn");
    host.advance(400).unwrap(); // milk's row appears — its ids are live

    let stale = listener_of(&host, "mark-1");
    // milk leaves the list — the framework retires the row subtree's
    // ids from its registry (repaints mint nothing; only a REMOVAL
    // orphans an id)
    click(&mut host, "del-1");
    host.advance(120).unwrap();
    assert_eq!(snap(&host, "list").child_tags, Vec::<String>::new());

    // firing the retired id is the FRAMEWORK's LOUD stale trap (the
    // phase-1 unknown-id law, re-pointed at the registry per §4.3)
    let err = host.fire_listener(stale).unwrap_err();
    assert!(
        err.msg.contains(&format!("t1: listener '{stale}' answered no subject — stale or unknown")),
        "{}",
        err.msg
    );

    // and the LIVE ids still work after the trapped turn
    type_into(&mut host, "tea");
    click(&mut host, "add-btn");
    host.advance(400).unwrap();
    click(&mut host, "mark-2");
    host.advance(250).unwrap();
    assert_eq!(class_of(&host, "mark-2"), "t1-check t1-check--on");
    assert_eq!(snap(&host, "row-2").child_texts, vec!["tea", "del"]);
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
    click(&mut host, "add-btn");
    host.advance(400).unwrap();
    assert_eq!(snap(&host, "row-1").child_texts, vec!["milk", "del"]);
    let _ = app;
}

// ---- the guard's absorb behavior, at app scale ----

#[test]
fn events_firing_mid_turn_queue_and_run_as_the_next_turn() {
    let (mut host, _app) = make_host();
    host.state.borrow_mut().dom.set_value_by_id("new-todo", "milk");
    let typing = listener_of(&host, "new-todo");
    let before = snap(&host, "status").text.unwrap_or_default();
    // a synchronous DOM dispatch inside a live turn (the focus() class):
    // the row can only join the queue — no nested turn runs
    host.in_sync_dom_dispatch(|h| h.state.borrow_mut().push_dom_event(typing));
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

    // add two — the pending lines paint NOW, in book order
    for title in ["milk", "tea"] {
        type_into(&mut host, title);
        click(&mut host, "add-btn");
    }
    assert_eq!(snap(&host, "list").child_texts, vec!["... milk", "... tea"]);
    host.advance(400).unwrap();
    assert_eq!(snap(&host, "list").child_tags, vec!["div", "div"]);

    // toggle the first — the flight paints as a variant patch
    click(&mut host, "mark-1");
    assert_eq!(class_of(&host, "lbl-1"), "t1-text t1-text--pending");
    host.advance(250).unwrap();
    assert_eq!(class_of(&host, "mark-1"), "t1-check t1-check--on");
    assert_eq!(class_of(&host, "lbl-1"), "t1-text t1-text--done");
    assert_eq!(snap(&host, "lbl-1").text.as_deref(), Some("milk"));

    // add a third
    type_into(&mut host, "jam");
    click(&mut host, "add-btn");
    assert_eq!(snap(&host, "list").child_texts, vec!["... jam"]);
    host.advance(400).unwrap();
    assert_eq!(snap(&host, "list").child_tags, vec!["div", "div", "div"]);

    // remove the done one
    click(&mut host, "del-1");
    host.advance(120).unwrap();
    assert_eq!(snap(&host, "row-2").child_texts, vec!["tea", "del"]);
    assert_eq!(class_of(&host, "mark-2"), "t1-check t1-check--off");
    assert_eq!(snap(&host, "row-3").child_texts, vec!["jam", "del"]);
    assert_eq!(
        snap(&host, "status").text.as_deref(),
        Some("2 open | 0 done | 0 in flight — removed 'milk' (#1)")
    );
    assert!(host.with_dom(|d| d.pending_timers()).is_empty());
}
