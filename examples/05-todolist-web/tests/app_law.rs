//! THE LAW, ENFORCED (docs/t1-design.md §2.1): biz code composes
//! WIDGETS. It never sinks to the DOM's vocabulary — no crossing is
//! imported, no tag or style token is spelled, no listener id is
//! named. Phase 2's rewrite made `todolist.rut` clean; this gate keeps
//! it clean: it reads the app's source and fails LOUD on violations.
//!
//! Three honest checks, no whitelist, no exceptions (the scan runs
//! over the whole file, comments included):
//!
//!   1. the import set is EXACTLY { appkit } — the app unit's one
//!      splice, the store plus the widget framework verbatim (the
//!      mount's own note records why they ride as one: the graph
//!      splices each use's transitive sources per use, and store and
//!      t1 both ride pouch). No crossing package is imported. (The
//!      clock name `tim_after` is the one non-widget call the app
//!      spells — the host's time, RFC 0018's own law — and it resolves
//!      through the appkit unit's bound surface, the inline-splice
//!      law. It is not an import, and the exact-import assertion keeps
//!      any second surface from riding in.)
//!   2. the file contains NO `Node`, no `web::`, no `ui_` (the
//!      crossing prefix: get/create/set/attr/append/clear/listen), no
//!      `class` attribute writes, and no tag literals — the manual DOM
//!      building of phase 1 is dead and stays dead.
//!   3. the render path is really the framework's (`t1_render` once
//!      per turn) and events really enter through subjects
//!      (`t1_subject`, `.subject(...)`), with keyed rows (`.key(...)`)
//!      — the widgets are the whole UI story, not a veneer.

/// The app source — the file this gate exists to guard.
const APP: &str = include_str!("../todolist.rut");

/// The pkgs the app may import, exactly: the app unit's one splice —
/// the store (the "server") + the widget framework, verbatim.
const ALLOWED_PKG_IMPORTS: &[&str] = &["appkit"];

/// The file's `use` statements' package names, in source order. A
/// multi-line `use t1::{ ... };` still names its pkg on the first line.
fn imports() -> Vec<String> {
    let mut out = Vec::new();
    for line in APP.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("use ") {
            if let Some(pkg) = rest.split("::").next() {
                out.push(pkg.trim().to_string());
            }
        }
    }
    out
}

#[test]
fn the_app_imports_widgets_and_the_store_only() {
    assert_eq!(
        imports(),
        ALLOWED_PKG_IMPORTS.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "the app's import set changed — biz may import exactly the app unit's \
         one splice (appkit: the store + the widget framework); a crossing \
         package has no business in the app unit (docs/t1-design.md §2.1)"
    );
}

#[test]
fn the_app_never_sinks_to_the_dom_vocabulary() {
    // `Node` — the DOM node vocabulary; elements are the lowering's
    //     private business.
    // `web::` — the crossing namespace, as an import or a qualified
    //     call.
    // `ui_` — the crossing prefix: get/create/set_text/attr/append/
    //     remove/clear/set_input_value/listen are all dead in biz.
    // `class=` / a quoted "class" — raw styling; tokens are the
    //     lowering's emission, never the app's spelling.
    // quoted tag literals and the DOM method names — the phase-1
    //     manual building, banned outright.
    const FORBIDDEN: &[&str] = &[
        "Node",
        "web::",
        "ui_",
        "class=",
        "\"class\"",
        "\"div\"",
        "\"span\"",
        "\"button\"",
        "\"input\"",
        "\"li\"",
        "\"ul\"",
        "\"p\"",
        "\"h1\"",
        "\"a\"",
        "createElement",
        "textContent",
        "setAttribute",
        "addEventListener",
        "appendChild",
        "removeChild",
    ];
    for token in FORBIDDEN {
        assert!(
            !APP.contains(token),
            "the app names {token} — biz code composes widgets \
             (docs/t1-design.md §2.1); the DOM vocabulary leaked back in"
        );
    }
}

#[test]
fn the_widgets_are_the_whole_ui_story() {
    assert!(
        APP.contains("t1_render("),
        "the paint must be the framework's — one t1_render per turn"
    );
    assert!(
        APP.contains("t1_subject("),
        "DOM events must enter through the framework's subject lookup"
    );
    assert!(APP.contains(".subject("), "widgets fire SEMANTIC subjects");
    assert!(APP.contains(".key("), "rows are keyed widgets — the diff's identity");
    assert!(APP.contains("check("), "the done mark is the framework's checkbox");
}
