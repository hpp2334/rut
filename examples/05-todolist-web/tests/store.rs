//! The store's DOM-FREE gate: the domain driven through its entry
//! surface on a plain Vm — no browser, no DOM turns. The laws checked
//! here: a request never touches the list (the list changes only when
//! `answer` lands), answers are data (rejections and lost ids), an
//! unknown tag traps loud, latencies differ per kind (the app-level
//! deadline order rides on this), and the round trips for
//! add/toggle/remove close.
//!
//! THE ATOM TWIN SUITE, third shape (the two-package store): the
//! machine entries are joined by the `atom_*` probe surface — now
//! MINIMAL, because the machinery went private. The laws:
//!
//! * PULL-ON-READ freshness — a get after a write-set recomputes the
//!   stale derived EXACTLY once; repeat gets recompute nothing; two
//!   writes before one get still cost one recompute. There is NO
//!   flush: `store.refresh` is retired, and the twins prove the pull
//!   replaced it.
//! * DISCOVERED deps — `atom_deps` pins counts$'s edges to exactly
//!   the atoms its closure reads (items$=0, reqs$=1 by boot order),
//!   and a write OUTSIDE them (draft$) recomputes nothing. The DAG is
//!   what the program reads; a stale declaration is impossible by
//!   construction.
//! * THE STR CONTENT-SKIP — an equal-content write marks nothing (the
//!   RFC 0044 asymmetry, kept in the one write lane).
//! * DERIVED-ON-DERIVED — the `probe_*` fixture (biz-side now: the
//!   store layer is decoupled from biz, so the topo fixture lives
//!   here): upstream recomputes before downstream, downstream sees the
//!   FRESH upstream, and an upstream-only recompute refreshes
//!   downstream (the value-gen law).
//! * THE CYCLE GUARD — a discovered cycle (a reading its reader) dies
//!   loud, named.
//!
//! The derived-set refusal is a COMPILE fact (Derived implements no
//! Writable — `tests/app_law.rs` pins the vocabulary); there is no
//! runtime test to write, and that is the point.
//!
//! The mount is the MANIFEST route: the biz pkg is the project root
//! (`rut/biz/`, spec `app`); its closure is ui + pouch + nmapset +
//! nmap_host. The session DECLARES the web crossings (ui's t1 does),
//! and the RFC 0025 contract is all-or-nothing — so the fake-DOM
//! bodies bind below, and NO test fires one: the store world is bare
//! (no t1_mount), every entry is a plain rut call.

use std::cell::RefCell;
use std::rc::Rc;

use rut_driver::Session;
use rut_vm::interp::{HostHooks, HostRegistry, Vm};
use rut_vm::OpaqueRef;

use todolist_web::fake_dom::FakeDom;
use todolist_web::{hosts, mount, state, WebState};

/// The project closure (spec `app`): biz + ui + pouch + nmapset +
/// nmap_host, plus the web DECL — bodies bound below, never fired.
fn vm() -> (Vm, OpaqueRef) {
    let (mut session, root) = mount::load_project_session().expect("the project mounts");
    assert_eq!(root, "app", "the store suite's root spec is the app");
    let expected = session.expected_host_fns();
    let prog = mount::compile_manifest(&session, &root).expect("the project compiles");

    let (slot, sink) = state::weak_sink_slot::<FakeDom>();
    let shared = Rc::new(RefCell::new(WebState::new(FakeDom::new(sink))));
    state::bind_weak_sink(&slot, &shared);
    shared.borrow_mut().dom.seed_page("div", "app");

    let mut hosts = HostRegistry::new();
    hosts::install_web_hosts(&mut hosts, &shared);
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected);

    let mut v = Vm::new(Rc::new(prog), &mount::limits(), HostHooks::default(), hosts)
        .expect("the vm boots");
    let c: OpaqueRef = v.call("store_new", ()).unwrap();
    (v, c)
}

// ---- the machine: the entry surface, signatures unchanged ------------

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
    let ans: (String, String) = v.call("store_answer", (c.clone(), tag)).unwrap();
    assert_eq!(ans.0, "added 'milk' as #1");
    assert_eq!(ans.1, "");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] milk");
    assert_eq!(v.call::<_, String>("store_counts", (c.clone(),)).unwrap(), "open=1 done=0 fly=0");
    assert_eq!(v.call::<_, String>("store_last", (c.clone(),)).unwrap(), "added 'milk' as #1");
}

#[test]
fn toggle_round_trip() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();
    let tag: String = v.call("store_request_toggle", (c.clone(), 1_i64)).unwrap();
    assert_eq!(tag, "req:2");
    // the flip is the commit's, not the booking's
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] milk");
    let ans: (String, String) = v.call("store_answer", (c.clone(), tag)).unwrap();
    assert_eq!(ans.0, "toggled #1 to done");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [x] milk");
}

#[test]
fn remove_round_trip() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();
    let tag: String = v.call("store_request_remove", (c.clone(), 1_i64)).unwrap();
    let ans: (String, String) = v.call("store_answer", (c.clone(), tag)).unwrap();
    assert_eq!(ans.0, "removed 'milk' (#1)");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "");
}

#[test]
fn a_rejected_title_is_an_answer_not_a_trap() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "")).unwrap();
    let ans: (String, String) = v.call("store_answer", (c.clone(), tag)).unwrap();
    assert_eq!(ans.0, "");
    assert_eq!(ans.1, "rejected '' — the title is empty");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "");
}

#[test]
fn a_lost_id_is_an_answer_through_the_table() {
    let (mut v, c) = vm();
    let add: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, (String, String)>("store_answer", (c.clone(), add)).unwrap();
    // toggle and remove the same row; the remove lands first
    let tog: String = v.call("store_request_toggle", (c.clone(), 1_i64)).unwrap();
    let rem: String = v.call("store_request_remove", (c.clone(), 1_i64)).unwrap();
    v.call::<_, (String, String)>("store_answer", (c.clone(), rem)).unwrap();
    let ans: (String, String) = v.call("store_answer", (c.clone(), tog)).unwrap();
    assert_eq!(ans.0, "");
    assert_eq!(ans.1, "skipped toggle #1 — the row is gone");
}

#[test]
fn an_unknown_tag_traps_loud() {
    let (mut v, c) = vm();
    let err = v
        .call::<_, (String, String)>("store_answer", (c.clone(), "req:99".to_string()))
        .unwrap_err();
    assert!(
        err.msg.contains("server: no request 'req:99' — the answer names no booked row"),
        "{}",
        err.msg
    );
}

#[test]
fn an_unknown_todo_answers_an_empty_tag() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_toggle", (c.clone(), 42_i64)).unwrap();
    assert_eq!(tag, "");
    let tag: String = v.call("store_request_remove", (c.clone(), 42_i64)).unwrap();
    assert_eq!(tag, "");
}

#[test]
fn latencies_differ_per_kind() {
    let (mut v, c) = vm();
    // seed one committed row so the toggle/remove bookings have a target
    v.call::<_, ()>("atom_items_set", (c.clone(), "seed".to_string())).unwrap();
    let a: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    let t: String = v.call("store_request_toggle", (c.clone(), 1_i64)).unwrap();
    let r: String = v.call("store_request_remove", (c.clone(), 1_i64)).unwrap();
    assert_eq!(
        v.call::<_, String>("store_fly", (c.clone(),)).unwrap(),
        format!("{a}@400|{t}@250|{r}@120")
    );
}

// ---- the pull-on-read twins ------------------------------------------

#[test]
fn a_get_after_a_write_set_recomputes_exactly_once() {
    let (mut v, c) = vm();
    // the first get computes the never-computed derived
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "0 open | 0 done | 0 in flight");
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), 1);
    // repeat gets recompute nothing — the cache serves
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), 1);
    // one write stales; the next get recomputes exactly once
    v.call::<_, ()>("atom_items_set", (c.clone(), "milk".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "1 open | 0 done | 0 in flight");
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), 2);
    // two writes before the next get: still exactly one recompute —
    // the WRITE-SET is the unit, the old per-write dirty set's law in
    // pull-on-read form
    v.call::<_, ()>("atom_items_set", (c.clone(), "tea".to_string())).unwrap();
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "x".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "1 open | 0 done | 0 in flight");
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), 3);
}

#[test]
fn staleness_is_per_discovered_dep() {
    let (mut v, c) = vm();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "0 open | 0 done | 0 in flight");
    // THE DISCOVERED DAG: counts$ reads items$ and reqs$ — ids 0 and 1
    // by boot order — and nothing else
    assert_eq!(v.call::<_, String>("atom_deps", (c.clone(),)).unwrap(), "0|1");
    let base: i32 = v.call("atom_recomputes", (c.clone(),)).unwrap();
    // a write counts$ never read recomputes NOTHING (draft$ = id 2)
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "milk".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "0 open | 0 done | 0 in flight");
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), base);
    // a write it DID read recomputes exactly once
    v.call::<_, ()>("atom_items_set", (c.clone(), "tea".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "1 open | 0 done | 0 in flight");
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), base + 1);
}

#[test]
fn the_str_lanes_content_skip_marks_nothing() {
    let (mut v, c) = vm();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "0 open | 0 done | 0 in flight");
    // equal content: no gen move, no recompute — even though counts$
    // would care about items$, this lane is draft$
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "same".to_string())).unwrap();
    let base: i32 = v.call("atom_recomputes", (c.clone(),)).unwrap();
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "same".to_string())).unwrap();
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "same".to_string())).unwrap();
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), base);
    assert_eq!(v.call::<_, String>("atom_str_get", (c.clone(), "draft".to_string())).unwrap(), "same");
    // a real change lands
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "diff".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("atom_str_get", (c.clone(), "draft".to_string())).unwrap(), "diff");
}

#[test]
fn the_vec_lane_never_skips() {
    let (mut v, c) = vm();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "0 open | 0 done | 0 in flight");
    let base: i32 = v.call("atom_recomputes", (c.clone(),)).unwrap();
    // the same title twice: a FRESH cell every set — two marks, two
    // recomputes (the recorded asymmetry: the general lane never
    // compares)
    v.call::<_, ()>("atom_items_set", (c.clone(), "milk".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "1 open | 0 done | 0 in flight");
    v.call::<_, ()>("atom_items_set", (c.clone(), "milk".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "1 open | 0 done | 0 in flight");
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), base + 2);
}

#[test]
fn the_counts_line_is_derived_and_the_status_line_reads_it() {
    let (mut v, c) = vm();
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "0 open | 0 done | 0 in flight");
    v.call::<_, ()>("atom_items_set", (c.clone(), "milk".to_string())).unwrap();
    v.call::<_, ()>("atom_items_set", (c.clone(), "tea".to_string())).unwrap();
    v.call::<_, ()>("atom_str_set", (c.clone(), "note".to_string(), "hello".to_string())).unwrap();
    // the SAME f-string bytes the app's status line shows and the
    // 18 sessions assert: the counts half through the derivation
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), "2 open | 0 done | 0 in flight");
    assert_eq!(v.call::<_, String>("atom_str_get", (c.clone(), "note".to_string())).unwrap(), "hello");
}

// ---- the derived-on-derived fixture (the topo law, biz-side) ----------

#[test]
fn deriveds_recompute_upstream_first_and_feed_downstream_fresh() {
    let (mut v, c) = vm();
    let p: OpaqueRef = v.call("probes_new", ()).unwrap();
    // the first pull computes the whole chain in topo order: up, then
    // down, downstream fed the FRESH upstream
    assert_eq!(v.call::<_, i64>("probe_lower", (p.clone(),)).unwrap(), 0);
    assert_eq!(v.call::<_, String>("probe_refresh", (p.clone(),)).unwrap(), "up|down");
    // a source write refreshes the chain, upstream first, on ONE pull
    v.call::<_, ()>("probe_bump", (p.clone(), 3_i64)).unwrap();
    assert_eq!(v.call::<_, i64>("probe_lower", (p.clone(),)).unwrap(), 12);
    assert_eq!(v.call::<_, String>("probe_refresh", (p.clone(),)).unwrap(), "up|down");
    // a clean pull recomputes nothing
    assert_eq!(v.call::<_, String>("probe_refresh", (p.clone(),)).unwrap(), "");
}

#[test]
fn an_upstream_only_recompute_still_refreshes_downstream() {
    // the VALUE-gen law: a derived's recompute bumps the derived's own
    // gen, and downstream keys on THAT — so a turn whose only move is
    // upstream's recompute (here: up's formula input changed through a
    // second bump before down ever pulled) still refreshes down.
    let (mut v, c) = vm();
    let p: OpaqueRef = v.call("probes_new", ()).unwrap();
    assert_eq!(v.call::<_, i64>("probe_lower", (p.clone(),)).unwrap(), 0);
    assert_eq!(v.call::<_, String>("probe_refresh", (p.clone(),)).unwrap(), "up|down");
    v.call::<_, ()>("probe_bump", (p.clone(), 5_i64)).unwrap();
    assert_eq!(v.call::<_, i64>("probe_lower", (p.clone(),)).unwrap(), 20);
    // down pulled AFTER up moved: the pair logged exactly one chain
    assert_eq!(v.call::<_, String>("probe_refresh", (p.clone(),)).unwrap(), "up|down");
    assert_eq!(v.call::<_, i32>("probe_recomputes", (p.clone(),)).unwrap(), 4);
}

// ---- the cycle guard ---------------------------------------------------

#[test]
fn a_discovered_cycle_dies_loud() {
    let (mut v, c) = vm();
    let p: OpaqueRef = v.call("probes_cycle_new", ()).unwrap();
    let err = v.call::<_, i64>("probe_cycle_get", (p,)).unwrap_err();
    assert!(
        err.msg.contains("reads itself — a discovered cycle"),
        "{}",
        err.msg
    );
}
