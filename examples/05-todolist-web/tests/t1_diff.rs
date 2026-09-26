//! The t1 keyed diff, the listener lifecycle, and the trap matrix
//! through the patcher (design §4.2/§4.3/§6): patch-in-place deltas
//! only, keyed reorder = re-append (element identity preserved through
//! the twin's handles — a rebuild would mint fresh node ids and fresh
//! listener ids), register-on-appear / retire-on-remove with the
//! registry tracking live ids only, stale fire = the LOUD trap, and
//! the unknown-handle patch firing loud, never silent.

mod t1_support;

use t1_support::{
    els_has, els_len, host_listener_len, listener_of, log_of, make_host, node_id, regs_has,
    regs_len, render, snap, Host,
};

fn class_of(host: &Host, hook: &str) -> String {
    snap(host, hook).attrs.get("class").unwrap().clone()
}

/// Every path a row owns (rows_a/rows_b/rows_toggle shape).
fn row_paths(n: i64) -> [String; 4] {
    [
        format!("list/row-{n}"),
        format!("list/row-{n}/mark-{n}"),
        format!("list/row-{n}/lbl-{n}"),
        format!("list/row-{n}/del-{n}"),
    ]
}

// ---- patch-in-place: the minimal-op proofs ---------------------------

#[test]
fn the_first_render_creates_and_the_second_changes_nothing() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    assert_eq!(snap(&host, "list").child_tags, vec!["div", "div"]);

    let before: Vec<u32> =
        row_paths(1).iter().chain(row_paths(2).iter()).map(|p| node_id(&mut host, &app, p)).collect();
    let listeners = host_listener_len(&host);
    let regs = regs_len(&mut host, &app);
    let els = els_len(&mut host, &app);

    // the SAME tree again: identity survives everywhere — no rebuild,
    // no re-listen, no table churn
    render(&mut host, &app, "t1p_rows_a");
    let after: Vec<u32> =
        row_paths(1).iter().chain(row_paths(2).iter()).map(|p| node_id(&mut host, &app, p)).collect();
    assert_eq!(before, after, "an unchanged render must not mint a single node");
    assert_eq!(host_listener_len(&host), listeners);
    assert_eq!(regs_len(&mut host, &app), regs);
    assert_eq!(els_len(&mut host, &app), els);
    assert_eq!(log_of(&mut host, &app), "");
}

#[test]
fn a_text_change_patches_text_and_never_relistens() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_solo_a");
    let line = node_id(&mut host, &app, "solo/line");
    let listeners = host_listener_len(&host);

    render(&mut host, &app, "t1p_solo_b");
    assert_eq!(snap(&host, "line").text.as_deref(), Some("after"));
    assert_eq!(node_id(&mut host, &app, "solo/line"), line, "the node survived the patch");
    // the OTHER reads are untouched — the delta was text-only
    assert_eq!(class_of(&host, "line"), "t1-text t1-text--body");
    assert_eq!(class_of(&host, "solo"), "t1-col t1-gap-8");
    assert_eq!(host_listener_len(&host), listeners, "a text change must not re-listen");
}

#[test]
fn a_toggle_is_one_class_token_patch() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    let mark1 = node_id(&mut host, &app, "list/row-1/mark-1");
    let mark1_listener = listener_of(&mut host, &app, "list/row-1/mark-1");
    let row2: Vec<u32> = row_paths(2).iter().map(|p| node_id(&mut host, &app, p)).collect();
    let listeners = host_listener_len(&host);

    render(&mut host, &app, "t1p_rows_toggle");
    // row 1's checkbox flipped its token IN PLACE
    assert_eq!(class_of(&host, "mark-1"), "t1-check t1-check--on");
    assert_eq!(node_id(&mut host, &app, "list/row-1/mark-1"), mark1);
    assert_eq!(
        listener_of(&mut host, &app, "list/row-1/mark-1"),
        mark1_listener,
        "the toggle's listener id is the one minted when the row appeared"
    );
    // row 2 did not move by a byte
    assert_eq!(row_paths(2).iter().map(|p| node_id(&mut host, &app, p)).collect::<Vec<u32>>(), row2);
    assert_eq!(class_of(&host, "mark-2"), "t1-check t1-check--off");
    assert_eq!(host_listener_len(&host), listeners);
}

#[test]
fn a_field_value_patch_rides_the_input_crossing() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_field_a");
    let field_node = node_id(&mut host, &app, "form/new-todo");
    let listeners = host_listener_len(&host);
    assert_eq!(snap(&host, "new-todo").attrs.get("value"), None);

    render(&mut host, &app, "t1p_field_b");
    assert_eq!(snap(&host, "new-todo").attrs.get("value").unwrap(), "milk");
    assert_eq!(node_id(&mut host, &app, "form/new-todo"), field_node);
    assert_eq!(host_listener_len(&host), listeners, "the field's input listener stays put");
}

// ---- the keyed diff: reorder, append, remove, mixed ------------------

#[test]
fn a_keyed_reorder_moves_without_rebuilding_or_relistening() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");

    let ids: Vec<u32> =
        row_paths(1).iter().chain(row_paths(2).iter()).map(|p| node_id(&mut host, &app, p)).collect();
    let mark1 = listener_of(&mut host, &app, "list/row-1/mark-1");
    let del1 = listener_of(&mut host, &app, "list/row-1/del-1");
    let mark2 = listener_of(&mut host, &app, "list/row-2/mark-2");
    let listeners = host_listener_len(&host);
    let els = els_len(&mut host, &app);

    render(&mut host, &app, "t1p_rows_b"); // rows swap: tea first

    // every node survived: reorder = re-append, zero creates, zero
    // re-registrations (a rebuild would have minted fresh ids)
    let after: Vec<u32> =
        row_paths(1).iter().chain(row_paths(2).iter()).map(|p| node_id(&mut host, &app, p)).collect();
    assert_eq!(ids, after, "a keyed reorder must not rebuild a single row node");
    assert_eq!(listener_of(&mut host, &app, "list/row-1/mark-1"), mark1);
    assert_eq!(listener_of(&mut host, &app, "list/row-1/del-1"), del1);
    assert_eq!(listener_of(&mut host, &app, "list/row-2/mark-2"), mark2);
    assert_eq!(host_listener_len(&host), listeners);
    assert_eq!(els_len(&mut host, &app), els);
    assert_eq!(regs_len(&mut host, &app), 4);

    // and the moved rows still deliver: fire tea's (row-2's) mark —
    // its subject routes through the same registry row as before
    host.fire_listener(mark2).unwrap();
    assert_eq!(log_of(&mut host, &app), "toggle:2");
}

#[test]
fn the_flat_reorder_swaps_the_twin_order() {
    // order is directly readable when the keyed children carry text
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_flat_a");
    assert_eq!(snap(&host, "flat").child_texts, vec!["a", "b", "c"]);

    render(&mut host, &app, "t1p_flat_b"); // c,x,a: move c, move a, create x, drop b
    assert_eq!(snap(&host, "flat").child_texts, vec!["c", "x", "a"]);
}

#[test]
fn a_keyed_append_creates_only_the_new_row() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    let before: Vec<u32> =
        row_paths(1).iter().chain(row_paths(2).iter()).map(|p| node_id(&mut host, &app, p)).collect();
    let max_before = before.iter().copied().max().unwrap();
    let mark1 = listener_of(&mut host, &app, "list/row-1/mark-1");

    render(&mut host, &app, "t1p_rows_d"); // + row-3 (jam)

    assert_eq!(snap(&host, "list").child_tags, vec!["div", "div", "div"]);
    // rows 1-2 untouched, row-3 fresh (its node ids are brand new)
    assert_eq!(
        row_paths(1).iter().chain(row_paths(2).iter()).map(|p| node_id(&mut host, &app, p)).collect::<Vec<u32>>(),
        before
    );
    assert!(node_id(&mut host, &app, "list/row-3") > max_before, "row-3 was created, not reused");
    assert_eq!(snap(&host, "lbl-3").text.as_deref(), Some("jam"));
    assert_eq!(listener_of(&mut host, &app, "list/row-1/mark-1"), mark1);
    assert_eq!(regs_len(&mut host, &app), 6, "two listeners joined: mark-3, del-3");
    assert!(els_has(&mut host, &app, "list/row-3/del-3"));
}

#[test]
fn a_keyed_remove_retires_its_subtree_completely() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    let mark2 = listener_of(&mut host, &app, "list/row-2/mark-2");
    let del2 = listener_of(&mut host, &app, "list/row-2/del-2");
    assert_eq!(regs_len(&mut host, &app), 4);
    assert_eq!(els_len(&mut host, &app), 9); // list + 2 rows x 4 nodes

    render(&mut host, &app, "t1p_rows_c"); // row-2 gone

    assert_eq!(snap(&host, "list").child_tags, vec!["div"]);
    assert_eq!(els_len(&mut host, &app), 5, "list + row-1's four nodes; row-2's paths left the table");
    for p in row_paths(2) {
        assert!(!els_has(&mut host, &app, &p), "{p} must leave the handle table");
    }
    // the registry tracks LIVE ids only — row-2's rows are retired
    assert_eq!(regs_len(&mut host, &app), 2);
    assert!(!regs_has(&mut host, &app, mark2), "row-2's rows are retired");
    assert!(!regs_has(&mut host, &app, del2), "row-2's rows are retired");
    // row-1's rows still answer their own mutations
    let mark1 = listener_of(&mut host, &app, "list/row-1/mark-1");
    assert!(regs_has(&mut host, &app, mark1));
    host.fire_listener(mark1).unwrap();
    assert_eq!(log_of(&mut host, &app), "toggle:1");
}

#[test]
fn the_mixed_sequence_moves_creates_and_drops_in_one_turn() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_flat_a");
    let a = node_id(&mut host, &app, "flat/a");
    let c = node_id(&mut host, &app, "flat/c");
    assert!(els_has(&mut host, &app, "flat/b"));

    render(&mut host, &app, "t1p_flat_b"); // [a b c] -> [c x a]

    assert_eq!(snap(&host, "flat").child_texts, vec!["c", "x", "a"]);
    assert_eq!(node_id(&mut host, &app, "flat/a"), a, "a moved, not rebuilt");
    assert_eq!(node_id(&mut host, &app, "flat/c"), c, "c stayed");
    assert!(els_has(&mut host, &app, "flat/x"));
    assert!(!els_has(&mut host, &app, "flat/b"), "b left the handle table");
    assert_eq!(els_len(&mut host, &app), 4); // flat + a + c + x
}

// ---- the listener lifecycle through the patch ------------------------

#[test]
fn listeners_register_on_appear_and_live_as_long_as_their_widget() {
    let (mut host, app) = make_host();
    assert_eq!(host_listener_len(&host), 0);
    render(&mut host, &app, "t1p_rows_a");
    // exactly the rows' listeners: 2 per row, minted ONCE at appear
    assert_eq!(host_listener_len(&host), 4);
    assert_eq!(regs_len(&mut host, &app), 4);
    let mark1 = listener_of(&mut host, &app, "list/row-1/mark-1");
    assert!(regs_has(&mut host, &app, mark1));

    // four more renders (toggle, swap, append, back) — not ONE fresh
    // id for the surviving rows; row-3's brief life minted exactly its
    // own two (the host never retires rows — the twin's documented
    // no-unlisten limit — so 6 total is the whole page's lifetime)
    render(&mut host, &app, "t1p_rows_toggle");
    render(&mut host, &app, "t1p_rows_b");
    render(&mut host, &app, "t1p_rows_d");
    render(&mut host, &app, "t1p_rows_a");
    assert_eq!(host_listener_len(&host), 6, "ids are minted once per widget LIFETIME");
    assert_eq!(regs_len(&mut host, &app), 4, "rut's registry tracks the LIVE rows only");
    assert_eq!(listener_of(&mut host, &app, "list/row-1/mark-1"), mark1);
}

#[test]
fn a_stale_fire_traps_loud_and_the_page_survives() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    let mark2 = listener_of(&mut host, &app, "list/row-2/mark-2");

    render(&mut host, &app, "t1p_rows_c"); // row-2 removed, ids retired

    // firing the retired id: the host still knows the row (its own
    // documented no-unlisten limit), the framework's registry does
    // NOT — the miss is the LOUD trap, named
    let err = host.fire_listener(mark2).unwrap_err();
    assert!(
        err.msg.contains(&format!("t1: listener '{mark2}' answers no mutation — stale or unknown")),
        "{}",
        err.msg
    );

    // and the live row still works after the trapped turn
    let mark1 = listener_of(&mut host, &app, "list/row-1/mark-1");
    host.fire_listener(mark1).unwrap();
    assert_eq!(log_of(&mut host, &app), "toggle:1");
}

#[test]
fn an_unknown_id_answers_no_mutation_loud() {
    // the host-side row for an id rut never registered: on_event
    // delivers it straight to the registry — the miss is loud
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    let err = host
        .call::<_, ()>("on_event", (app.clone(), 1i32, "4242".to_string(), "".to_string()))
        .unwrap_err();
    assert!(
        err.msg.contains("t1: listener '4242' answers no mutation — stale or unknown"),
        "{}",
        err.msg
    );
}

#[test]
fn every_listener_row_fires_its_own_mutation() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    let mut seen: Vec<&str> = Vec::new();
    for (path, label) in [
        ("list/row-1/mark-1", "toggle:1"),
        ("list/row-1/del-1", "remove:1"),
        ("list/row-2/mark-2", "toggle:2"),
        ("list/row-2/del-2", "remove:2"),
    ] {
        let id = listener_of(&mut host, &app, path);
        assert!(regs_has(&mut host, &app, id), "{path} answers a live mutation");
        host.fire_listener(id).unwrap();
        seen.push(label);
        let joined = seen.join("|");
        assert_eq!(log_of(&mut host, &app), joined, "the row's OWN mutation ran");
    }
}

// ---- the trap matrix through the patcher -----------------------------

#[test]
fn an_unknown_handle_during_a_patch_traps_loud_never_silent() {
    // framework-internal drift, simulated honestly: the handle table
    // loses a row between renders; the next patch names that element
    // and must be LOUD about it
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_solo_a");
    host.call::<_, ()>("t1p_els_steal", (app.clone(), "solo/line".to_string())).unwrap();

    let err = host.call::<_, ()>("t1p_solo_b", (app,)).unwrap_err();
    assert!(
        err.msg.contains("t1: no live handle for 'solo/line' — the patch named an element that never appeared"),
        "{}",
        err.msg
    );
}

#[test]
fn a_stolen_row_handle_surfaces_at_its_own_patch() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    host.call::<_, ()>("t1p_els_steal", (app.clone(), "list/row-2/mark-2".to_string())).unwrap();

    // the toggle turn patches row-2's mark — the drift is named
    let err = host.call::<_, ()>("t1p_rows_toggle", (app,)).unwrap_err();
    assert!(
        err.msg.contains("t1: no live handle for 'list/row-2/mark-2'"),
        "{}",
        err.msg
    );
}

#[test]
fn the_root_key_cannot_change_under_a_patch() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a"); // root key "list"
    let err = host.call::<_, ()>("t1p_solo_b", (app,)).unwrap_err(); // root key "solo"
    assert!(
        err.msg.contains("t1: the root's key changed 'list' -> 'solo' — remount is not a patch"),
        "{}",
        err.msg
    );
}

#[test]
fn a_non_dom_event_kind_traps_in_the_harness() {
    let (mut host, app) = make_host();
    let err = host
        .call::<_, ()>("on_event", (app, 2i32, "req:1".to_string(), "".to_string()))
        .unwrap_err();
    assert!(err.msg.contains("t1h: unknown event kind 2"), "{}", err.msg);
}

// the mount keeps the container honest — the entry shape is the app's
#[test]
fn the_container_round_trips_through_the_pump() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_rows_a");
    host.pump().unwrap(); // an empty queue drains clean
    assert_eq!(log_of(&mut host, &app), "");
}
