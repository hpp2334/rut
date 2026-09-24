//! THE LAW, ENFORCED (docs/t1-design.md §2.1, the restructure survey
//! §5.1 contract): biz code composes WIDGETS. It never sinks to the
//! DOM's vocabulary — no crossing is spelled beyond the one clock
//! name, no tag or style token is written, no listener id is named.
//! The gate reads the biz layer's sources (`rut/app/` — the entry plus
//! the two view builders, whole-file, comments included, NO whitelist)
//! and fails LOUD on violations.
//!
//! Four checks:
//!
//!   1. the app's import set is EXACTLY the survey §5.1's thirteen
//!      packages, with each package's contributed names pinned —
//!      `web` contributes ONLY `tim_after` (the one crossing name the
//!      biz layer may spell, now imported honestly by name), `t1`
//!      exactly the framework's four names, `todos` the domain types
//!      (phase 2: the atom store's `TodoStore` — the store internals
//!      `atom`/`derived` are NOT importable by biz, and the set
//!      equality is what enforces that), each component its
//!      constructor(s). A second crossing package, a store internal,
//!      a lowering name — nothing rides in: the set equality is the
//!      law.
//!   2. the biz files contain NO `Node`, no `ui_` (the crossing
//!      prefix), no `class=`/quoted `"class"`, and no tag literals or
//!      DOM method names — the manual-DOM shape is dead and stays
//!      dead. (Bare `web::` left this list in phase 1: the sanctioned
//!      `use web::{tim_after};` contains it; the name pins above are
//!      the crossing law now.)
//!   3. the render path is really the framework's (`t1_render` once
//!      per turn, in the entry's paint) and events really enter
//!      through subjects (`t1_subject`, `.subject(...)`), with keyed
//!      rows (`.key(...)` in the row builder) — the widgets are the
//!      whole UI story, not a veneer. Phase 2 (survey §5.1's
//!      addition): `store.refresh()` appears in `paint` BEFORE
//!      `view` — the within-turn atom flush is part of the render
//!      law now.
//!   4. the component packages keep their own layer honest by import
//!      scoping (survey §3.3): each imports `widget` and NOTHING else
//!      — the lowering and the core stay framework-private.
//!
//! The appkit retirement (survey §2.5, P2) is asserted here too: the
//! word survives nowhere in `src/` — the concatenation module is gone,
//! and this is the one-line assert that keeps it gone.

/// The biz layer's sources — the files this gate exists to guard.
const APP: &str = include_str!("../rut/app/app/app.rut");
const TODO_LIST: &str = include_str!("../rut/app/todo_list/todo_list.rut");
const TODO_ROW: &str = include_str!("../rut/app/todo_row/todo_row.rut");

/// The app's import law (survey §5.1): package -> the EXACT name list
/// its `use` may contribute. Set equality on the packages; exact
/// equality on each package's names.
const APP_IMPORTS: &[(&str, &[&str])] = &[
    ("web", &["tim_after"]),
    ("todos", &["TodoStore", "Todo"]),
    ("t1", &["T1Root", "t1_mount", "t1_render", "t1_subject"]),
    ("widget", &["Widget"]),
    ("todo_list", &["todo_list"]),
    ("todo_row", &["todo_row"]),
    ("col", &["col"]),
    ("row", &["row"]),
    ("text", &["text", "done", "pending", "muted"]),
    ("button", &["btn", "quiet"]),
    ("checkbox", &["check"]),
    ("field", &["field"]),
    ("card", &["card"]),
];

/// One `use` statement: the package and the names it contributes.
fn parse_use(src: &str, use_kw_at: usize) -> Option<(String, Vec<String>)> {
    let rest = &src[use_kw_at + 3..];
    let rest = rest.trim_start();
    let pkg_end = rest.find("::")?;
    let pkg = rest[..pkg_end].trim().to_string();
    let after = &rest[pkg_end + 2..];
    let brace = after.find('{')?;
    let close = after[brace + 1..].find('}')? + brace + 1;
    let names: Vec<String> = after[brace + 1..close]
        .split(',')
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .collect();
    Some((pkg, names))
}

/// A source's `use` statements, in source order — the whole-file scan:
/// no whitelist, no exceptions.
fn imports(src: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut rest = src;
    let mut base = 0usize;
    while let Some(at) = rest.find("use ") {
        let before = &rest[..at];
        // whole-line `use` starts only — a word ending in "use" is not one
        let line_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
        let indent_ok = before[line_start..].trim().is_empty();
        let candidate = parse_use(src, base + at);
        rest = &rest[at + 4..];
        base += at + 4;
        if indent_ok {
            if let Some(parsed) = candidate {
                out.push(parsed);
            }
        }
    }
    out
}

#[test]
fn the_app_imports_exactly_the_project_vocabulary() {
    let got = imports(APP);
    assert_eq!(
        got.len(),
        APP_IMPORTS.len(),
        "the app's import set changed — biz may import exactly the survey \
         §5.1 thirteen packages (the widget vocabulary, the store's \
         domain types, the framework's four names, the two view \
         builders, and web for the one clock name); got: {got:?}"
    );
    for (i, (pkg, names)) in got.iter().enumerate() {
        let (want_pkg, want_names) = APP_IMPORTS[i];
        assert_eq!(pkg, want_pkg, "import #{i}'s package drifted");
        let mut sorted: Vec<String> = names.clone();
        sorted.sort();
        let mut want: Vec<String> = want_names.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(
            sorted, want,
            "`{pkg}`'s contributed names drifted — the per-package name \
             pins are the crossing law (survey §5.1)"
        );
    }
    // P2 (survey §2.5): the appkit name survives nowhere in src/ — the
    // concatenation module is gone and stays gone.
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src_dir).expect("src/ reads") {
        let path = entry.expect("src/ entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("src file reads");
        assert!(
            !text.contains("appkit"),
            "{} still names appkit — the concatenation module is retired \
             (survey §2.5: a dedup gap is a dep-kinds bug report, never \
             a workaround re-entering src/)",
            path.display()
        );
    }
}

#[test]
fn the_biz_layer_never_sinks_to_the_dom_vocabulary() {
    // `Node` — the DOM node vocabulary; elements are the lowering's
    //     private business.
    // `ui_` — the crossing prefix: get/create/set_text/attr/append/
    //     remove/clear/set_input_value/listen are all dead in biz.
    //     (`web::` LEFT this list in phase 1: the sanctioned
    //     `use web::{tim_after};` contains it — the name pins above
    //     are the crossing law now.)
    // `class=` / a quoted "class" — raw styling; tokens are the
    //     lowering's emission, never the app's spelling.
    // quoted tag literals and the DOM method names — the manual
    //     building of the old shape, banned outright.
    const FORBIDDEN: &[&str] = &[
        "Node",
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
    for (file, src) in [("app.rut", APP), ("todo_list.rut", TODO_LIST), ("todo_row.rut", TODO_ROW)] {
        for token in FORBIDDEN {
            assert!(
                !src.contains(token),
                "{file} names {token} — biz code composes widgets \
                 (docs/t1-design.md §2.1); the DOM vocabulary leaked back in"
            );
        }
        // the framework's styling layer biz must never touch directly
        // (the `web` crossing IS sanctioned — the one name, pinned by
        // the import law above)
        for pkg in ["lowering"] {
            let bad = imports(src).iter().any(|(p, _)| p.as_str() == pkg);
            assert!(
                !bad,
                "{file} imports {pkg} — the {pkg} layer is not biz's \
                 to import (survey §3.3/§5.1)"
            );
        }
    }
}

#[test]
fn the_widgets_are_the_whole_ui_story() {
    // the entry: the render is the framework's — ONE t1_render per
    // turn, in paint — and events enter through the subject lookup
    assert_eq!(
        APP.matches("t1_render(").count(),
        1,
        "the paint must be the framework's — one t1_render per turn"
    );
    assert!(
        APP.contains("t1_subject("),
        "DOM events must enter through the framework's subject lookup"
    );
    assert!(APP.contains(".subject("), "widgets fire SEMANTIC subjects");
    assert!(APP.contains("fn paint("), "the turn's ONE render has its fn");
    // phase 2 (survey §5.1): the within-turn atom flush is part of the
    // render law — `store.refresh()` runs in paint, BEFORE view
    let paint_at = APP.find("fn paint(").expect("paint's source is in the app");
    let refresh_at = APP[paint_at..].find("store.refresh();").expect("paint flushes the atom store")
        + paint_at;
    let view_at = APP[paint_at..].find("view(r);").expect("paint builds the view") + paint_at;
    assert!(
        refresh_at < view_at,
        "paint must flush the atom store BEFORE building the view — the \
         within-turn refresh precedes the one render (survey §4.3)"
    );
    // the view builders: keyed rows, the framework's checkbox, the
    // tables beside the subjects they mirror
    for (file, src, pins) in [
        ("todo_row.rut", TODO_ROW, vec![".key(", ".subject(", "check("]),
        ("todo_list.rut", TODO_LIST, vec!["todo_row(", "HashMap<str, i64>"]),
    ] {
        for pin in pins {
            assert!(
                src.contains(pin),
                "{file} lost `{pin}` — the widgets are the whole UI story \
                 (docs/t1-design.md §2.1)"
            );
        }
    }
}
