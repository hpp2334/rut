//! THE LAW, ENFORCED (docs/t1-design.md §2.1, the restructure survey
//! §5.1 contract, the two-package shape): biz code composes WIDGETS and
//! speaks THROUGH THE HANDLES (tur's law: the app never touches a
//! store object). It never sinks to the DOM's
//! vocabulary — no crossing is spelled beyond the one clock name, no
//! tag or style token is written, no listener id is named. The gate
//! reads the biz layer's source (the biz module's five files, spliced
//! the manifest's way — whole-text,
//! comments included, NO whitelist) and fails LOUD on violations.
//!
//! Five checks:
//!
//!   1. the app's import set is EXACTLY the two-package vocabulary —
//!      `ui` (the store + the handle types + the event binding + the
//!      widget type + the framework's four names + the component
//!      constructors), `pouch` (Vec), and `web` contributing ONLY
//!      `tim_after`. The store's machinery traits (Readable/Writable)
//!      are ui-PRIVATE — they cannot be imported even by name; the set
//!      equality is the second half of that law.
//!   2. the biz file contains NO `Node`, no `ui_` (the crossing
//!      prefix), no `class=`/quoted `"class"`, and no tag literals or
//!      DOM method names.
//!   3. the render path is really the framework's (`t1_render` once
//!      per turn, in the entry's paint), events really enter through
//!      the framework's mutation resolution (`t1_event`, widgets
//!      carrying `.on_click(...)`/`.on_input(...)`), rows are keyed —
//!      and THE FLUSH IS GONE: `refresh` appears nowhere (pull-on-read
//!      is the freshness law; the RENDER DOOR is the recompute point),
//!      the view wires REACTIVE PROPS (`.text_of`/`.value_of`/`live`,
//!      never a pulled value), the view half never writes, THE APP
//!      CODE never pulls or pushes a handle (`$.get(`/`$.set(` absent
//!      — the ctx forms own every read and write), and THE STORE'S
//!      OWN VERBS appear nowhere — they are ui-module-private.
//!   4. THE MACHINERY TOMBSTONES: the retired API's names — TodoStore,
//!      Rail, Seen, Atom, StrAtom, ProbeStore, refresh, drain, stale —
//!      survive nowhere in biz. The store is the ui package's; biz
//!      holds handles and speaks get/set.
//!   5. the appkit retirement (survey §2.5, P2): the word survives
//!      nowhere in `src/`.

/// The biz layer's source — the module this gate exists to guard: the
/// base plus `entry.libs`, spliced the manifest's way (RFC 0041 §5) —
/// the gate reads what the program actually compiles.
static BIZ: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    splice(
        include_str!("../rut/biz/biz.rut"),
        &[
            include_str!("../rut/biz/domain.rut"),
            include_str!("../rut/biz/world.rut"),
            include_str!("../rut/biz/entries.rut"),
            include_str!("../rut/biz/app.rut"),
        ],
    )
});

/// The APP CODE alone (the base, the domain, the world, the shell) —
/// entries.rut is the FROZEN host-test surface (survey §5.3): its
/// handle verbs are the test probes' reads, pinned by the twins. The
/// no-pull/no-push law below governs the app code; the test surface is
/// exempt by the freeze.
static APP_CODE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    splice(
        include_str!("../rut/biz/biz.rut"),
        &[
            include_str!("../rut/biz/domain.rut"),
            include_str!("../rut/biz/world.rut"),
            include_str!("../rut/biz/app.rut"),
        ],
    )
});

/// The loader's splice law, restated for the gate: base first, then
/// libs in manifest order, '\n'-joined (crate::mount::biz_source is
/// the same law).
fn splice(base: &str, libs: &[&str]) -> String {
    let mut src = base.to_string();
    for part in libs {
        src.push('\n');
        src.push_str(part);
    }
    src
}

/// The app's import law: package -> the EXACT name list its `use` may
/// contribute. Set equality on the packages; exact equality on each
/// package's names.
const APP_IMPORTS: &[(&str, &[&str])] = &[
    (
        "ui",
        &[
            // the store's public vocabulary — the store itself (boot's
            // mint) and the handle types
            "Store", "Source", "Derived", "Mutation",
            // the widget type and the framework's four names
            "Widget", "T1Root", "t1_mount", "t1_render", "t1_event",
            // the component constructors (+ live: the reactive subtree)
            "col", "row", "card", "field", "check",
            "text", "done", "pending", "muted", "btn", "quiet", "live",
        ],
    ),
    ("pouch", &["Vec"]),
    ("web", &["tim_after"]),
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
    let got = imports(&BIZ);
    assert_eq!(
        got.len(),
        APP_IMPORTS.len(),
        "the app's import set changed — biz may import exactly the \
         two-package vocabulary (the store's public names, the widget \
         vocabulary, the framework's four names, and web for the one \
         clock name); got: {got:?}"
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
    for token in FORBIDDEN {
        assert!(
            !BIZ.contains(token),
            "biz names {token} — biz code composes widgets \
             (docs/t1-design.md §2.1); the DOM vocabulary leaked back in"
        );
    }
}

#[test]
fn the_widgets_are_the_whole_ui_story_and_the_pull_is_the_freshness() {
    // the entry: the render is the framework's — ONE t1_render per
    // turn, in paint — and events enter through the framework's
    // mutation resolution (tur's onClick/onInput, framework-run)
    assert_eq!(
        BIZ.matches("t1_render(").count(),
        1,
        "the paint must be the framework's — one t1_render per turn"
    );
    assert!(
        BIZ.contains("t1_event("),
        "DOM events must enter through the framework's mutation resolution"
    );
    assert!(BIZ.contains(".on_click("), "widgets CARRY their event mutations");
    assert!(BIZ.contains("fn paint("), "the turn's ONE render has its fn");
    // THE FLUSH IS RETIRED (the two-package store's freshness law):
    // pull-on-read — the view's first handle get recomputes what the
    // turn's writes staled, at most once per write-set. No refresh
    // exists anywhere in biz, and the view READS.
    assert!(
        !BIZ.contains("refresh"),
        "the flush is retired — pull-on-read replaced it; no refresh \
         may appear in biz"
    );
    // THE APP CODE NEVER PULLS OR PUSHES (tur's law, completed): the
    // view wires REACTIVE PROPS (deriveds resolved at the render
    // door) and every write rides a mutation — the only reads and
    // writes are the ctx's own, inside derive/mutation closures. The
    // handle verbs exist for the frozen test surface and ui's
    // resolver; the app code's field-reach spellings fail loud.
    assert!(
        BIZ.contains(".text_of(") && BIZ.contains(".value_of("),
        "the view wires reactive props — deriveds, not pulled values"
    );
    assert!(
        BIZ.contains("live("),
        "the list rides a reactive subtree (tur's Each)"
    );
    assert!(
        !BIZ.contains("store.get(") && !BIZ.contains("store.set("),
        "the store's verbs are ui-private — their spellings may not \
         appear anywhere in biz, not even in comments"
    );
    assert!(
        !APP_CODE.contains("$.get(") && !APP_CODE.contains("$.set("),
        "the app code never pulls or pushes handles — reads are \
         reactive props (derive + ctx.get), writes are mutations \
         (ctx.set); the handle verbs are the test surface's, not \
         the app's"
    );
    // THE DOORS: one entry per event class, NAMED FOR THE DOM EVENT
    // that produced it — the glue owns the listener rows and derives
    // the export (on_click, on_input, ...; on_timer for the clock), so
    // the kind-coded mega-entry is a tombstone: the host knows the
    // class when it enqueues; rut never re-derives it from an integer.
    for door in ["entry fn on_click(", "entry fn on_input(", "entry fn on_timer("] {
        assert!(
            BIZ.contains(door),
            "the app answers every event class with a named door — missing {door}"
        );
    }
    assert!(
        !BIZ.contains("entry fn on_event(") && !BIZ.contains("kind: i32"),
        "the kind-coded on_event is gone — doors, not codes"
    );
    assert!(
        !BIZ.contains(".subject(") && !BIZ.contains("fn dispatch("),
        "the subject tables and their dispatcher are gone — events \
         ride the widgets' own mutation props"
    );
    // the view half never writes: between `fn view(` and the view
    // builders there is no set — view NEVER writes (survey §4.2)
    let view_at = BIZ.find("fn view(").expect("the view fn is in biz");
    let views_end = BIZ[view_at..].find("fn todo_row(").expect("the views follow") + view_at;
    let views = &BIZ[view_at..views_end];
    assert!(
        !views.contains(".set("),
        "the view half never writes — reads only (survey §4.2: view \
         never writes an atom)"
    );
    // keyed rows, the framework's checkbox, the event props on the rows
    for pin in [".key(", ".on_click(", ".on_input(", ".on_row(", ".text_of(", ".value_of(", "live(", "check(", "todo_row("] {
        assert!(
            BIZ.contains(pin),
            "biz lost `{pin}` — the widgets are the whole UI story \
             (docs/t1-design.md §2.1)"
        );
    }
}

#[test]
fn the_retired_store_api_survives_nowhere_in_biz() {
    // THE MACHINERY TOMBSTONES. The store is the ui package's kernel;
    // biz holds HANDLES and speaks get/set. The old domain-typed
    // container and the machinery it carried (the rail, the seen
    // generations, the declared DAG, the flush) are gone — their names
    // must not survive in biz, not even in a comment's spelling of an
    // API call. (Readable/Writable cannot appear either — they are
    // ui-private and unimportable; the import law pins that already.)
    const TOMBSTONES: &[&str] = &[
        "TodoStore",
        "ProbeStore",
        ".rail",
        "Rail",
        "Seen",
        "Atom<",
        "StrAtom",
        ".refresh(",
        ".drain(",
        ".stale(",
        ".value()",
        "counts$.value",
        // the surface's own retired spellings: the store verbs moved
        // onto the handles (ui-module-private on the store itself),
        // and the subject machinery died with the mutation props
        "store.get(",
        "store.set(",
        ".subject(",
        "fn dispatch(",
    ];
    for name in TOMBSTONES {
        assert!(
            !BIZ.contains(name),
            "biz still names `{name}` — the retired store API is gone; \
             biz holds handles and speaks their verbs (a.get/a.set/m.run)"
        );
    }
}
