//! The t1 lowering snapshots (design §6, the twin as the framework's
//! test bed): build a widget tree, render once, and assert the twin's
//! tag / class-token / text / attr tree — every WKind and every
//! variant at least once, plus the fixed utility ladder and the
//! lowering's LOUD boundaries (controls with children, missing
//! variants, off-ladder tokens, listeners with no subject).
//!
//! These tests pin the §4.1 table itself — the whole styling contract
//! phase 2's stylesheet keys on. The twin asserts TOKENS, not pixels.

mod t1_support;

use t1_support::{ els_has, make_host, render, snap, Host };

fn class_of(host: &Host, hook: &str) -> String {
    snap(host, hook).attrs.get("class").unwrap().clone()
}

// ---- the §4.1 table, row by row --------------------------------------

#[test]
fn every_kind_lowers_to_its_tag_and_tokens() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");

    // layout kinds: div + their base token (root also carries ladder)
    assert_eq!(class_of(&host, "kinds"), "t1-col t1-gap-16 t1-pad-16");
    assert_eq!(snap(&host, "kinds").tag, "div");
    assert_eq!(class_of(&host, "head"), "t1-row t1-gap-8");
    assert_eq!(snap(&host, "head").tag, "div");
    assert_eq!(class_of(&host, "card"), "t1-card t1-pad-12");
    assert_eq!(snap(&host, "card").tag, "div");
    assert_eq!(class_of(&host, "sp"), "t1-spacer t1-size-8");
    assert_eq!(snap(&host, "sp").tag, "div");

    // controls
    assert_eq!(snap(&host, "new-todo").tag, "input");
    assert_eq!(class_of(&host, "new-todo"), "t1-field");
    assert_eq!(snap(&host, "add-btn").tag, "button");
    assert_eq!(class_of(&host, "add-btn"), "t1-btn t1-btn--primary");
    assert_eq!(snap(&host, "add-btn").text.as_deref(), Some("Add"));
    assert_eq!(snap(&host, "row-1").tag, "div");
    assert_eq!(class_of(&host, "row-1"), "t1-row");
    assert_eq!(class_of(&host, "mark-1"), "t1-check t1-check--off");
    assert_eq!(class_of(&host, "mark-2"), "t1-check t1-check--on");
}

#[test]
fn every_text_variant_lowers_to_its_token() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");

    assert_eq!(class_of(&host, "title"), "t1-text t1-text--title");
    assert_eq!(snap(&host, "title").text.as_deref(), Some("Today"));
    assert_eq!(class_of(&host, "body-line"), "t1-text t1-text--body");
    assert_eq!(class_of(&host, "lbl-2"), "t1-text t1-text--done");
    assert_eq!(snap(&host, "lbl-2").text.as_deref(), Some("tea"));
    assert_eq!(class_of(&host, "pend"), "t1-text t1-text--pending");
    assert_eq!(snap(&host, "pend").text.as_deref(), Some("... jam"));
    assert_eq!(class_of(&host, "muted-line"), "t1-text t1-text--muted");
    assert_eq!(snap(&host, "muted-line").tag, "span");
}

#[test]
fn the_quiet_button_variant_lowers() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");
    assert_eq!(class_of(&host, "del-1"), "t1-btn t1-btn--quiet");
    assert_eq!(snap(&host, "del-1").text.as_deref(), Some("del"));
}

#[test]
fn a_checkbox_is_a_framework_owned_button_with_a_role() {
    // the recorded deviation: never a native input — the framework
    // owns the visual so a toggle is one class-token patch
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");

    assert_eq!(snap(&host, "mark-1").tag, "button");
    assert_eq!(snap(&host, "mark-1").attrs.get("role").unwrap(), "checkbox");
    assert_eq!(class_of(&host, "mark-1"), "t1-check t1-check--off");
    assert_eq!(snap(&host, "mark-2").attrs.get("role").unwrap(), "checkbox");
    assert_eq!(class_of(&host, "mark-2"), "t1-check t1-check--on");
}

#[test]
fn hooks_land_as_id_attrs_and_texts_as_text() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");

    assert_eq!(snap(&host, "new-todo").attrs.get("id").unwrap(), "new-todo");
    assert_eq!(
        snap(&host, "new-todo").attrs.get("placeholder").unwrap(),
        "What needs doing?"
    );
    assert_eq!(snap(&host, "title").text.as_deref(), Some("Today"));
    assert_eq!(snap(&host, "row-1").child_tags, vec!["button", "span", "button"]);
    assert_eq!(snap(&host, "row-1").child_texts, vec!["milk", "del"]);
}

// ---- keys, paths, and the handle table -------------------------------

#[test]
fn keys_default_from_hooks_and_build_paths() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");

    // the whole path chain exists under the root's key
    assert!(els_has(&mut host, &app, "kinds"));
    assert!(els_has(&mut host, &app, "kinds/row-1"));
    assert!(els_has(&mut host, &app, "kinds/row-1/mark-1"));
    assert!(els_has(&mut host, &app, "kinds/row-1/lbl-1"));
    assert!(!els_has(&mut host, &app, "kinds/row-9"));
}

#[test]
fn hooked_children_take_their_place_in_the_path_tree() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");
    // hooked children are reachable as paths; nothing auto-keyed
    // exists here because every child in the kinds tree names a hook
    assert!(els_has(&mut host, &app, "kinds/card/muted-line"));
    assert!(els_has(&mut host, &app, "kinds/head/new-todo"));
    assert!(els_has(&mut host, &app, "kinds/head/add-btn"));
    assert!(!els_has(&mut host, &app, "kinds/head/@0"), "hooks win over auto keys");
}

// ---- the fixed utility ladder ----------------------------------------

#[test]
fn the_ladder_emits_only_its_rungs() {
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");
    assert_eq!(class_of(&host, "kinds"), "t1-col t1-gap-16 t1-pad-16");
    assert_eq!(class_of(&host, "head"), "t1-row t1-gap-8");
    assert_eq!(class_of(&host, "card"), "t1-card t1-pad-12");
    assert_eq!(class_of(&host, "sp"), "t1-spacer t1-size-8");
}

#[test]
fn off_ladder_tokens_trap_loud() {
    for probe in ["t1p_trap_bad_gap", "t1p_trap_bad_pad", "t1p_trap_bad_size"] {
        let (mut host, app) = make_host();
        let err = host.call::<_, ()>(probe, (app,)).unwrap_err();
        assert!(
            err.msg.contains("is off the utility ladder — 0/4/8/12/16 only"),
            "{probe}: {}",
            err.msg
        );
    }
}

// ---- the lowering's loud boundaries ----------------------------------

#[test]
fn a_control_with_children_is_a_lowering_panic() {
    // the fat struct's one dishonesty, caught at the boundary
    let (mut host, app) = make_host();
    let err = host.call::<_, ()>("t1p_trap_text_children", (app,)).unwrap_err();
    assert!(
        err.msg.contains("t1: Text takes no children — build layout with Column/Row/Card"),
        "{}",
        err.msg
    );
}

#[test]
fn a_text_without_a_variant_is_a_lowering_panic() {
    let (mut host, app) = make_host();
    let err = host.call::<_, ()>("t1p_trap_text_no_variant", (app,)).unwrap_err();
    assert!(
        err.msg.contains("t1: Text without a variant — construct with text/done/title/pending/muted"),
        "{}",
        err.msg
    );
}

#[test]
fn a_listener_without_a_subject_is_a_create_panic() {
    let (mut host, app) = make_host();
    let err = host.call::<_, ()>("t1p_trap_listener_no_mutation", (app,)).unwrap_err();
    assert!(
        err.msg.contains("listens but carries no mutation — set .on_click(...)/.on_input(...)"),
        "{}",
        err.msg
    );
}

#[test]
fn a_surviving_key_that_changes_kind_traps_loud() {
    let (mut host, app) = make_host();
    let err = host.call::<_, ()>("t1p_trap_kind_change", (app,)).unwrap_err();
    assert!(
        err.msg.contains("t1: 'x' changed kind Column -> Text — keys are identity"),
        "{}",
        err.msg
    );
}

// ---- the wire-up smoke -----------------------------------------------

#[test]
fn a_mounted_tree_fires_its_subjects_through_the_pump() {
    // the tiny smoke the phase asks for: a t1 tree mounted through the
    // REAL host mount, a click fired at a hook's listener (found via
    // the framework's handle table + the host's listener rows), the
    // SUBJECT delivered through the framework's registry
    let (mut host, app) = make_host();
    render(&mut host, &app, "t1p_render_kinds");

    let add = t1_support::listener_of(&mut host, &app, "kinds/head/add-btn");
    host.fire_listener(add).unwrap();
    assert_eq!(t1_support::log_of(&mut host, &app), "add");

    let typing = t1_support::listener_of(&mut host, &app, "kinds/head/new-todo");
    host.fire_listener(typing).unwrap();
    assert_eq!(t1_support::log_of(&mut host, &app), "add|type");
}
