//! The store's DOM-FREE gate: the "server" half driven through its
//! entry surface on a plain Vm — no `web` surface mounted, no DOM, no
//! timers. The laws checked here: a request never touches the list (the
//! list changes only when `answer` lands), answers are data (rejections
//! and lost ids), an unknown tag traps loud, latencies differ per kind
//! (the app-level deadline order rides on this), and the round trips
//! for add/toggle/remove close.
//!
//! PHASE 2 — THE ATOM TWIN SUITE (survey §5.3's new tests, DOM-free is
//! the atom layer's own level): the machine entries above are joined
//! by the `atom_*` probe surface (the rail, the flush, the declared
//! DAG, the derived freshness) and the `probe_*` fixture entries
//! (derived-on-derived, topo order). The laws: set/get with no rail
//! traffic on reads; a write marks exactly its own dirty set once;
//! WITHIN-TURN freshness (a dirty derived read AFTER a write in the
//! same turn sees fresh values, exactly one recompute); staleness is
//! PER-DECLARED-DEP; nothing leaks ACROSS a turn boundary; topo order
//! in one pass (upstream before downstream, downstream fed fresh);
//! equality-skip is the str lane's scope alone (the documented
//! asymmetry); and the counts line — now DERIVED (counts$) — keeps
//! its exact bytes. Family isolation has NO test: the family was
//! REJECTED (survey §4.6); the rejection is the record.
//!
//! The mount is the MANIFEST route at package scale: `rut/store/todos`
//! is its own package (`todos`, now with the `atom` + `derived` pkgs
//! in its manifest closure beside `pouch`), loaded with
//! `load_dir_session` and compiled as the ROOT — which also makes
//! this suite the survey §3.2(b) proof: a linked root calling its
//! deps' class methods across the link boundary (the same path the
//! app's fluent `.gap()` chains ride).

use std::rc::Rc;

use rut_vm::interp::{HostHooks, HostRegistry, Vm};
use rut_vm::OpaqueRef;

use todolist_web::mount;

/// core + the manifest closure of the todos pkg (pouch, the atom pkgs,
/// nmapset, and — through nmapset's own manifest — the `nmap_host`
/// DECL): no `web` surface, no DOM, no timers in the session at all.
/// The rail's `HashMap` (survey §4.2's sketch) rides nmapset, whose
/// map crossings are pure data machinery — the store's own law is
/// untouched: it never touches the clock or the DOM. The bodies bind
/// below (the map host only); `expected_host_fns` proves the session
/// declares nothing else.
fn vm() -> (Vm, OpaqueRef) {
    let (session, root) = mount::load_todos_session().expect("the todos package mounts");
    assert_eq!(root, "todos", "the store suite's root spec is the todos pkg");
    let expected = session.expected_host_fns();
    let prog = mount::compile_manifest(&session, &root).expect("the store compiles");
    let mut hosts = HostRegistry::new();
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected);
    let mut v = Vm::new(Rc::new(prog), &mount::limits(), HostHooks::default(), hosts)
        .expect("the vm boots");
    let c: OpaqueRef = v.call("store_new", ()).unwrap();
    (v, c)
}
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
    let (line, err) = v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();
    assert_eq!((line.as_str(), err.as_str()), ("added 'milk' as #1", ""));
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] milk");
    assert_eq!(v.call::<_, String>("store_counts", (c.clone(),)).unwrap(), "open=1 done=0 fly=0");
    assert_eq!(v.call::<_, String>("store_last", (c.clone(),)).unwrap(), "added 'milk' as #1");
}

#[test]
fn toggle_round_trip() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();

    let t2: String = v.call("store_request_toggle", (c.clone(), 1i64)).unwrap();
    assert_eq!(t2, "req:2");
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] milk");
    let (line, err) = v.call::<_, (String, String)>("store_answer", (c.clone(), t2)).unwrap();
    assert_eq!((line.as_str(), err.as_str()), ("toggled #1 to done", ""));
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [x] milk");
    assert_eq!(v.call::<_, String>("store_counts", (c.clone(),)).unwrap(), "open=0 done=1 fly=0");
}

#[test]
fn remove_round_trip() {
    let (mut v, c) = vm();
    for title in ["milk", "tea"] {
        let tag: String = v.call("store_request_add", (c.clone(), title)).unwrap();
        v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();
    }
    let t3: String = v.call("store_request_remove", (c.clone(), 1i64)).unwrap();
    assert_eq!(t3, "req:3");
    let (line, err) = v.call::<_, (String, String)>("store_answer", (c.clone(), t3)).unwrap();
    assert_eq!((line.as_str(), err.as_str()), ("removed 'milk' (#1)", ""));
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#2 [ ] tea");
}

#[test]
fn latencies_differ_per_kind() {
    let (mut v, c) = vm();
    let a: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, (String, String)>("store_answer", (c.clone(), a)).unwrap();
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
    v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();

    // the duplicate: the ENTRY-ERR shape — ("", why), the value
    // channel empty, the err channel carrying the reason
    let d: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    let (line, err) = v.call::<_, (String, String)>("store_answer", (c.clone(), d)).unwrap();
    assert_eq!((line.as_str(), err.as_str()), ("", "rejected 'milk' — already on the list"));
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] milk");
    assert_eq!(v.call::<_, String>("store_counts", (c.clone(),)).unwrap(), "open=1 done=0 fly=0");

    // the empty title (the app's client gate never sends one; the
    // server rejects it anyway)
    let e: String = v.call("store_request_add", (c.clone(), "")).unwrap();
    let (line, err) = v.call::<_, (String, String)>("store_answer", (c.clone(), e)).unwrap();
    assert_eq!((line.as_str(), err.as_str()), ("", "rejected '' — the title is empty"));
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
    v.call::<_, (String, String)>("store_answer", (c.clone(), a)).unwrap();

    // book a remove (fast) and a toggle (slow) on the same todo; the
    // remove's answer lands first and the toggle's id is lost — the
    // answer SAYS so instead of trapping or wedging
    let r: String = v.call("store_request_remove", (c.clone(), 1i64)).unwrap();
    let t: String = v.call("store_request_toggle", (c.clone(), 1i64)).unwrap();
    let (line, err) = v.call::<_, (String, String)>("store_answer", (c.clone(), r)).unwrap();
    assert_eq!((line.as_str(), err.as_str()), ("removed 'milk' (#1)", ""));
    let (line, err) = v.call::<_, (String, String)>("store_answer", (c.clone(), t)).unwrap();
    assert_eq!((line.as_str(), err.as_str()), ("", "skipped toggle #1 — the row is gone"));
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "");
}

// ==== the atom twins (phase 2, survey §5.3) ==========================
//
// The rail + flush + declared-DAG laws, driven through the `atom_*`
// probe surface (and the fixture's `probe_*` for the topo law). The
// vm() fixture above is unchanged: same store session, same boot.

/// `set` then `get` returns the value, per atom kind — and a `get`
/// performs NO rail traffic: reads move no generation, dirty nothing.
#[test]
fn atoms_set_and_get_with_no_rail_traffic_on_reads() {
    let (mut v, c) = vm();
    // the str lane (draft$, note$)
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "milk".to_string())).unwrap();
    assert_eq!(
        v.call::<_, String>("atom_str_get", (c.clone(), "draft".to_string())).unwrap(),
        "milk"
    );
    v.call::<_, ()>("atom_str_set", (c.clone(), "note".to_string(), "hello".to_string())).unwrap();
    assert_eq!(
        v.call::<_, String>("atom_str_get", (c.clone(), "note".to_string())).unwrap(),
        "hello"
    );
    // the Vec<Todo> lane (items$) — the board reads it
    v.call::<_, ()>("atom_items_set", (c.clone(), "tea".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] tea");
    // exactly the three writes are dirty, in write order
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "draft|note|items");
    // READS are free: gets and boards move no generation, dirty nothing
    let g = v.call::<_, i64>("atom_gen", (c.clone(), "items".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("store_board", (c.clone(),)).unwrap(), "#1 [ ] tea");
    assert_eq!(
        v.call::<_, String>("atom_str_get", (c.clone(), "draft".to_string())).unwrap(),
        "milk"
    );
    assert_eq!(v.call::<_, i64>("atom_gen", (c.clone(), "items".to_string())).unwrap(), g);
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "");
}

/// N `set`s on one atom in a turn mark its dirty set ONCE (a set, not
/// a list) while the generation advances per mark.
#[test]
fn a_write_marks_its_dirty_set_once() {
    let (mut v, c) = vm();
    for text in ["a", "b", "c"] {
        v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), text.to_string())).unwrap();
    }
    assert_eq!(
        v.call::<_, i64>("atom_gen", (c.clone(), "draft".to_string())).unwrap(),
        3,
        "the generation advanced by the mark count"
    );
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "draft");
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "", "a set, not a list");
    // the Vec lane: every set marks (no equality skip there) — the
    // dirty-set dedup is the same
    v.call::<_, ()>("atom_items_set", (c.clone(), "milk".to_string())).unwrap();
    v.call::<_, ()>("atom_items_set", (c.clone(), "milk".to_string())).unwrap();
    assert_eq!(v.call::<_, i64>("atom_gen", (c.clone(), "items".to_string())).unwrap(), 2);
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "items");
}

/// Write, then THE flush, then read: fresh values, and the recompute
/// counter shows exactly ONE recomputation for the turn. THE
/// COUNTS-LINE LAW, now proven THROUGH the derivation: counts$ is
/// what the status line reads.
#[test]
fn the_counts_line_is_derived_and_reads_fresh_within_a_turn() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    assert_eq!(
        v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(),
        "0 open | 0 done | 1 in flight"
    );
    assert_eq!(
        v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(),
        1,
        "exactly one recompute for the turn"
    );
    // the answer's own turn: the commit lands, one more recompute
    v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    assert_eq!(
        v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(),
        "1 open | 0 done | 0 in flight"
    );
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), 2);
}

/// Staleness is PER-DECLARED-DEP (survey §4.4 — the declared DAG's
/// truth, pinned): a write OUTSIDE counts$'s [items, reqs] recomputes
/// nothing; a write to either declared dep recomputes exactly once.
#[test]
fn staleness_is_per_declared_dep() {
    let (mut v, c) = vm();
    // the declaration itself, read through the trait
    assert_eq!(v.call::<_, String>("atom_derived_name", (c.clone(),)).unwrap(), "counts");
    assert_eq!(
        v.call::<_, String>("atom_derived_deps", (c.clone(),)).unwrap(),
        "items|reqs"
    );
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap(); // counts$ computes once
    let base: i32 = v.call("atom_recomputes", (c.clone(),)).unwrap();

    // draft$ and note$ are NOT declared deps: the seen gens still
    // match, the flush recomputes nothing
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "milk".to_string())).unwrap();
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    assert_eq!(
        v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(),
        base,
        "a non-dep write recomputes nothing"
    );
    v.call::<_, ()>("atom_str_set", (c.clone(), "note".to_string(), "x".to_string())).unwrap();
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), base);

    // items$ IS one: exactly one recompute
    v.call::<_, ()>("atom_items_set", (c.clone(), "tea".to_string())).unwrap();
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), base + 1);

    // and the reqs leg: the machine's booking dirties counts$ too
    let _ = v.call::<_, String>("store_request_add", (c.clone(), "jam".to_string())).unwrap();
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), base + 2);
}

/// NO cross-turn leakage: the flush drains the turn's writes to
/// empty; a no-op turn recomputes NOTHING, leaves every cached value
/// bit-identical, and the generations move only when writes happen.
#[test]
fn nothing_leaks_across_a_turn_boundary() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    assert_eq!(
        v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(),
        "",
        "the flush drained the turn's writes"
    );
    let before: i32 = v.call("atom_recomputes", (c.clone(),)).unwrap();
    let line: String = v.call("atom_counts", (c.clone(),)).unwrap();

    // a no-op turn: no writes, no marks, no recomputes, clean caches
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    assert_eq!(v.call::<_, i32>("atom_recomputes", (c.clone(),)).unwrap(), before);
    assert_eq!(v.call::<_, String>("atom_counts", (c.clone(),)).unwrap(), line);
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "");
    assert_eq!(
        v.call::<_, i64>("atom_gen", (c.clone(), "items".to_string())).unwrap(),
        1,
        "generations move only when writes happen"
    );
}

/// Topo order (the derived-on-derived fixture, survey §5.3): one
/// refresh pass recomputes upstream BEFORE downstream and feeds the
/// downstream the FRESH upstream; a no-op pass recomputes nothing.
#[test]
fn deriveds_recompute_upstream_first_in_one_pass() {
    let (mut v, _c) = vm();
    // the fixture has its OWN container (probes_new) — a ProbeStore,
    // not the TodoStore the machine entries drive
    let pc: OpaqueRef = v.call("probes_new", ()).unwrap();
    // the first flush: both tiers compute, upstream first
    assert_eq!(v.call::<_, String>("probe_refresh", (pc.clone(),)).unwrap(), "up|down");
    assert_eq!(v.call::<_, i32>("probe_lower", (pc.clone(),)).unwrap(), 0);
    // a write: one pass, up THEN down, downstream fed fresh
    v.call::<_, ()>("probe_bump", (pc.clone(), "milk".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("probe_refresh", (pc.clone(),)).unwrap(), "up|down");
    assert_eq!(
        v.call::<_, i32>("probe_lower", (pc.clone(),)).unwrap(),
        2,
        "down read the fresh upstream in the same pass"
    );
    assert_eq!(v.call::<_, i32>("probe_recomputes", (pc.clone(),)).unwrap(), 4);
    // a no-op pass: nothing recomputes
    assert_eq!(v.call::<_, String>("probe_refresh", (pc.clone(),)).unwrap(), "");
    assert_eq!(v.call::<_, i32>("probe_recomputes", (pc.clone(),)).unwrap(), 4);
}

/// Equality-skip scope (the documented asymmetry, survey §4.2): a str
/// atom set to its CURRENT content marks nothing; a Vec atom set
/// marks — every time.
#[test]
fn equality_skip_is_the_str_lanes_scope() {
    let (mut v, c) = vm();
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "milk".to_string())).unwrap();
    assert_eq!(v.call::<_, i64>("atom_gen", (c.clone(), "draft".to_string())).unwrap(), 1);
    // the equal write: no mark, no gen move
    v.call::<_, ()>("atom_str_set", (c.clone(), "draft".to_string(), "milk".to_string())).unwrap();
    assert_eq!(
        v.call::<_, i64>("atom_gen", (c.clone(), "draft".to_string())).unwrap(),
        1,
        "the equal-content write skipped"
    );
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "draft");
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "");
    // the Vec lane marks on EVERY set (cell identity is not a value
    // law rut can lean on — atom.rut's split record)
    v.call::<_, ()>("atom_items_set", (c.clone(), "milk".to_string())).unwrap();
    v.call::<_, ()>("atom_items_set", (c.clone(), "milk".to_string())).unwrap();
    assert_eq!(v.call::<_, i64>("atom_gen", (c.clone(), "items".to_string())).unwrap(), 2);
    assert_eq!(v.call::<_, String>("atom_drain", (c.clone(),)).unwrap(), "items");
}

/// The status line's shape, store-side (survey §4.5): the app's line
/// is `f"{counts$.value()} — {note$.get()}"` — the SAME bytes the 18
/// sessions assert verbatim; the counts half now comes THROUGH the
/// derived.
#[test]
fn the_status_line_is_counts_derived_plus_note() {
    let (mut v, c) = vm();
    let tag: String = v.call("store_request_add", (c.clone(), "milk")).unwrap();
    v.call::<_, (String, String)>("store_answer", (c.clone(), tag)).unwrap();
    // the app would have written the answer line into note$; the
    // probe writes the same line to stand in for the dispatch
    v.call::<_, ()>("atom_str_set", (c.clone(), "note".to_string(), "added 'milk' as #1".to_string()))
        .unwrap();
    v.call::<_, ()>("atom_refresh", (c.clone(),)).unwrap();
    let counts = v.call::<_, String>("atom_counts", (c.clone(),)).unwrap();
    let note = v.call::<_, String>("atom_str_get", (c.clone(), "note".to_string())).unwrap();
    assert_eq!(
        format!("{counts} — {note}"),
        "1 open | 0 done | 0 in flight — added 'milk' as #1"
    );
}
