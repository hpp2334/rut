//! THE LAW, ENFORCED (docs/t1-design.md §2.1, the restructure survey
//! §5.1 contract, the two-package shape): biz code composes WIDGETS and
//! speaks THROUGH THE HANDLES (tur's law: the app never touches a
//! store object). It never sinks to the DOM's
//! vocabulary — no crossing is spelled beyond the one clock name, no
//! tag or style token is written, no listener id is named. The gate
//! reads the biz layer's source (the biz module's TREE — every mounted
//! file, whole-text, comments included, NO whitelist) and fails LOUD
//! on violations.
//!
//! Five checks:
//!
//!   1. the app's import set is EXACTLY the two-package vocabulary —
//!      `ui` (the store + the handle types + the event binding + the
//!      widget type + the framework's four names + the component
//!      constructors), `pouch` (Vec), and `web` contributing ONLY
//!      `tim_after`. The store's machinery (the shared plumbing
//!      members) is ui-PRIVATE — it cannot be imported even by name;
//!      the set equality is the second half of that law.
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

/// The biz module's source — the module this gate exists to guard: the
/// FILE-MODULE TREE, every mounted file concatenated in mod-path
/// order (the gate reads what the program actually compiles). The
/// store/atom probes are NOT here: they are a test spec
/// (tests/store_probe/), their own module over the same imports — the
/// app's ABI is `main` plus the event doors, nothing else.
static BIZ: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    tree(
        include_str!("../rut/biz/mod.rut"),
        &[
            include_str!("../rut/biz/app/mod.rut"),
            include_str!("../rut/biz/domain/mod.rut"),
            include_str!("../rut/biz/world/mod.rut"),
        ],
    )
});

/// The module-tree law, restated for the gate: the root first, then
/// the mounted children in mod-path order, '\n'-joined (the mirror
/// crate::mount::biz_pkg offers the same tree).
fn tree(root: &str, children: &[&str]) -> String {
    let mut src = root.to_string();
    for part in children {
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
            // mint) and the handle types (the capability interfaces
            // stay ui-internal: satisfaction is structural, and the
            // bounds resolve without the names crossing)
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

/// One `use` statement: the package (the FIRST path segment — the
/// file-modules grammar spells `use pkg::A::B::{ .. }`, and the pkg
/// head is what the crossing law pins) and the names it contributes.
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
        // a nested segment (a typo'd `a::{ b::{ c } }`) is not a name
        .filter(|n| !n.contains("::{") && *n != "{")
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
    // the file-modules grammar spells one `use` per SEGMENT
    // (`use ui::store::{ .. }`, `use ui::diff::{ .. }`) — the law pins
    // the per-package vocabulary, so the statements aggregate into
    // package unions before the equality
    let got = imports(&BIZ);
    let mut by_pkg: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        Default::default();
    for (pkg, names) in got {
        by_pkg.entry(pkg).or_default().extend(names);
    }
    let want: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        APP_IMPORTS
            .iter()
            .map(|(p, ns)| (p.to_string(), ns.iter().map(|n| n.to_string()).collect()))
            .collect();
    assert_eq!(
        by_pkg, want,
        "the app's import vocabulary changed — biz may import exactly \
         the two-package vocabulary (the store's public names, the \
         widget vocabulary, the framework's four names, and web for \
         the one clock name)"
    );
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
    // handle verbs belong to ui's resolver and the store-probe spec
    // (tests/store_probe/); the module's own text has neither.
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
        !BIZ.contains("$.get(") && !BIZ.contains("$.set("),
        "the module never pulls or pushes handles — reads are \
         reactive props (derive + ctx.get), writes are mutations \
         (ctx.set); the handle verbs live in the store-probe spec"
    );
    // THE ABI IS THE PAGE: exactly `main` + the doors. Every other
    // host-callable surface is a test spec under tests/, never an
    // export of the app module.
    assert_eq!(
        BIZ.match_indices("entry fn ").count(),
        4,
        "the app module exports main + the three event doors — \
         nothing else (probes live in tests/*.rut fixtures)"
    );
    assert!(
        !BIZ.contains("entry fn store_")
            && !BIZ.contains("entry fn atom_")
            && !BIZ.contains("entry fn probe_"),
        "the store/atom probes are a test spec, not app ABI"
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
        // the subject machinery died with the mutation props, and the
        // store's shared plumbing members (atom_id/materialize) are
        // ui-private — the compiler refuses them; the grep pins it
        "store.get(",
        "store.set(",
        ".atom_id(",
        ".materialize(",
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
