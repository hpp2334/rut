use rut_lsp::completion::*;
use rut_lsp::hover::DefIndex;
use rut_lsp::mods::DocMods;

/// completions with the cursor just after `needle` in `src`
fn complete_after(src: &str, needle: &str) -> Vec<CompletionOut> {
    let s = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let idx = rut_lsp::hover::index(&s, &ast, &toks);
    let idxs = [&idx];
    let pos = s.rfind(needle).unwrap() as u32 + needle.len() as u32;
    complete(&idxs, &toks, &ast, pos, &s, &DocMods::default())
}

fn labels(items: &[CompletionOut]) -> Vec<&str> {
    items.iter().map(|c| c.label.as_str()).collect()
}

#[test]
fn member_completion_lists_impl_methods_and_fields() {
    let src = "\
struct Counter {
n: i32;
}
impl Counter {
fn bump(mut self) -> i32 { return self.n; }
fn reset(mut self) -> nil { }
}
fn use_it(c: Counter) -> i32 {
return c.;
}
";
    let items = complete_after(src, "c.");
    let ls = labels(&items);
    assert!(ls.contains(&"bump"), "{ls:?}");
    assert!(ls.contains(&"reset"), "{ls:?}");
    assert!(ls.contains(&"n"), "fields complete too: {ls:?}");
}

#[test]
fn member_completion_gates_foreign_interface_members() {
    // the foreign interface's members complete only once the doc
    // uses it; the document's own interface needs no gate
    let surf_src = "interface Greeter {\nfn greet(self) -> nil;\n}\n";
    let s2 = rut_lexer::lexer::normalize(surf_src);
    let (stoks, _) = rut_lexer::lexer::lex(&s2);
    let (sast, _) = rut_parser::parse(&s2, rut_parser::Mode::Impl);
    let surf = rut_lsp::hover::index(&s2, &sast, &stoks);

    let mk = |doc: &str| {
        let d2 = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&d2);
        let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
        let di = rut_lsp::hover::index(&d2, &ast, &toks);
        let idxs = [&di, &surf];
        let pos = d2.rfind("g.").unwrap() as u32 + 2;
        complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default())
    };

    let gated = "fn go(g: Greeter) -> nil { g. }\n";
    assert!(
        !labels(&mk(gated)).contains(&"greet"),
        "unused foreign interface must not complete"
    );

    let used = "use greets::{ Greeter };\nfn go(g: Greeter) -> nil { g. }\n";
    assert!(
        labels(&mk(used)).contains(&"greet"),
        "used interface completes"
    );

    // the document's own interface is in scope natively
    let own = "interface Local {\nfn hi(self) -> nil;\n}\nfn go(l: Local) -> nil { l. }\n";
    let d2 = rut_lexer::lexer::normalize(own);
    let (toks, _) = rut_lexer::lexer::lex(&d2);
    let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&d2, &ast, &toks);
    let idxs = [&di];
    let pos = d2.rfind("l.").unwrap() as u32 + 2;
    assert!(
        labels(&complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default())).contains(&"hi"),
        "own interface completes"
    );
}

#[test]
fn bare_completion_has_the_keyword_table_and_decls() {
    let src = "interface Shape {\nfn area(self) -> f64;\n}\nentry fn main() -> nil { }\n";
    let items = complete_after(src, "entry fn main() -> nil { }");
    let ls = labels(&items);
    // the final keyword table — the current spellings, no retired ones
    for kw in ["interface", "async", "use", "impl", "let", "fn"] {
        assert!(ls.contains(&kw), "keyword `{kw}` completes: {ls:?}");
    }
    // `trait` retired to an ordinary identifier — the keyword table
    // must not offer it
    assert!(
        !ls.contains(&"trait"),
        "the retired spelling must not complete: {ls:?}"
    );
    assert!(!ls
        .iter()
        .any(|l| matches!(*l, "import" | "suspend" | "from" | "dyn")));
    // decls: the document's interface (as an interface) and fn
    let shape = items
        .iter()
        .find(|c| c.label == "Shape")
        .expect("the decl completes");
    assert!(
        matches!(shape.kind, CompletionKind::Interface),
        "{:?}",
        shape.kind
    );
    assert!(ls.contains(&"main"), "{ls:?}");
    // methods are reached through a receiver, not named bare
    assert!(!ls.contains(&"area"), "{ls:?}");
}

#[test]
fn bare_completion_gates_pub_builtin_core_names() {
    // the `pub builtin` rows (the disposal pair and the weak
    // reference) complete only when the doc's `use` names them —
    // the compiler's ambient split, mirrored so a completion never
    // offers a name the compile rejects. Ambient core rows
    // (`prelude builtin`) stay ungated.
    let core_src = "pub builtin class Iterable<E> {\nfn next(mut self) -> ?E;\n}\n\
                        pub builtin class Disposal {\nfn dispose(mut self, cx: DisposalContext);\n}\n\
                        pub builtin class DisposalContext { }\n\
                        pub builtin class Weak { }\n\
                        prelude builtin class StackTrace { }\n";
    let c2 = rut_lexer::lexer::normalize(core_src);
    let (ctoks, _) = rut_lexer::lexer::lex(&c2);
    let (cast, _) = rut_parser::parse(&c2, rut_parser::Mode::Decl);
    let mut core = rut_lsp::hover::index(&c2, &cast, &ctoks);
    // the manifest module name — the stamp every face lays down
    // (deps::index_dep, std_surface), and what the bare tier's
    // `(name, pkg)` imported gate matches against
    core.module = Some("core".to_string());

    let mk = |doc: &str| {
        let d2 = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&d2);
        let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
        let di = rut_lsp::hover::index(&d2, &ast, &toks);
        let idxs = [&di, &core];
        let pos = d2.len() as u32;
        complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default())
    };

    let bare = "entry fn main() -> nil { }\n";
    let items = mk(bare);
    let ls = labels(&items);
    assert!(
        !ls.contains(&"Disposal"),
        "unused `pub builtin` must not complete: {ls:?}"
    );
    assert!(
        !ls.contains(&"DisposalContext"),
        "unused `pub builtin` must not complete: {ls:?}"
    );
    assert!(
        !ls.contains(&"Iterable"),
        "the gated builtin name must not complete: {ls:?}"
    );
    assert!(
        !ls.contains(&"Weak"),
        "the gated weak reference must not complete: {ls:?}"
    );
    assert!(
        ls.contains(&"StackTrace"),
        "the ambient row still completes: {ls:?}"
    );

    let imported =
        "use core::{ Disposal, DisposalContext, Iterable, Weak };\nfn main() -> nil { }\n";
    let items = mk(imported);
    let ls = labels(&items);
    assert!(
        ls.contains(&"Disposal"),
        "the imported builtin class completes: {ls:?}"
    );
    assert!(
        ls.contains(&"DisposalContext"),
        "the imported class completes: {ls:?}"
    );
    assert!(
        ls.contains(&"Iterable"),
        "the imported generic builtin class completes: {ls:?}"
    );
    assert!(
        ls.contains(&"Weak"),
        "the imported weak reference completes: {ls:?}"
    );
}

// ---- the honest bare scope (A2): a foreign decl is a PLAIN bare
// item only when the document's own `use` names it — every other
// foreign name rides only the auto-import tier (the `use` insert
// edit rides the item), and a transitive-only dep never offers. ----

/// a direct dep index — the `gadgets` pkg with one pub fn and one
/// pub struct, built the way the dep walk does (`index_dep`:
/// module stamped; the direct-dep marker stamped — the root
/// manifest's own dep row)
fn tag_dep() -> DefIndex {
    let mut dep = rut_lsp::deps::index_dep(
        "gadgets",
        "file:///deps/gadgets/lib.rut",
        "pub struct Widget {\nid: i32;\n}\npub fn tag() -> i32 { return 1; }\n",
        rut_parser::Mode::Impl,
        None,
        &[],
    );
    dep.direct_dep = true;
    dep
}

/// completions over `doc` with `dep` in the chain, the cursor at
/// the end of the document
fn complete_with_dep(doc: &str, dep: &DefIndex) -> Vec<CompletionOut> {
    let d2 = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&d2);
    let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&d2, &ast, &toks);
    let idxs = [&di, dep];
    complete(
        &idxs,
        &toks,
        &ast,
        d2.len() as u32,
        &d2,
        &DocMods::default(),
    )
}

#[test]
fn a_foreign_dep_fn_rides_only_the_import_tier_until_used() {
    let dep = tag_dep();
    // bare doc: `tag` is NOT a plain bare item — the only offer is
    // the auto-import one (the `use gadgets::tag;` edit rides it)
    let items = complete_with_dep("entry fn main() -> nil { }\n", &dep);
    assert!(
        items
            .iter()
            .all(|c| c.label != "tag" || !c.additional_text_edits.is_empty()),
        "the dep fn must not complete plain: {:?}",
        labels(&items)
    );
    let imp = import_item(&items, "tag").expect("the direct dep's fn auto-imports");
    assert_eq!(imp.detail, "gadgets::tag — import");

    // after the `use`, `tag` IS a plain bare item — and the
    // auto-import tier never re-offers it
    let items = complete_with_dep("use gadgets::{ tag };\nentry fn main() -> nil { }\n", &dep);
    let plain = items
        .iter()
        .find(|c| c.label == "tag")
        .expect("the imported fn completes bare");
    assert!(
        plain.additional_text_edits.is_empty(),
        "the imported fn completes plain, no insert: {plain:?}"
    );
    assert!(
        import_item(&items, "tag").is_none(),
        "never re-offered: {:?}",
        labels(&items)
    );
}

#[test]
fn a_transitive_only_dep_never_auto_imports() {
    // the compiler's closure check (`Session::resolve_use` →
    // `declares`) refuses `use` of a mounted-but-undeclared
    // package — the auto-import tier mirrors it: an index NOT
    // stamped `direct_dep` never rides the tier
    let mut dep = tag_dep();
    dep.direct_dep = false; // mounted, but the doc's manifest doesn't declare it
    let items = complete_with_dep("entry fn main() -> nil { }\n", &dep);
    assert!(
        import_item(&items, "tag").is_none(),
        "a transitive-only dep's fn must not auto-import: {:?}",
        labels(&items)
    );
    assert!(
        items.iter().all(|c| c.label != "tag"),
        "and no plain offer either: {:?}",
        labels(&items)
    );
}

#[test]
fn a_name_reachable_two_ways_offers_exactly_one_item() {
    // A4: `tag` completes bare (the doc's `use` names it) AND the
    // auto-import tier would offer it — the merged list retains
    // the plain one, exactly one item by the label
    let dep = tag_dep();
    let items = complete_with_dep("use gadgets::{ tag };\nentry fn main() -> nil { }\n", &dep);
    let count = items.iter().filter(|c| c.label == "tag").count();
    assert_eq!(
        count,
        1,
        "one `tag` item, got {count}: {:?}",
        labels(&items)
    );
    let the = items.iter().find(|c| c.label == "tag").unwrap();
    assert!(
        the.additional_text_edits.is_empty(),
        "the plain (bare) item wins: {the:?}"
    );
}

// ---- the brace-form use-path shapes (A6 regression pins — the
// `{` trigger char's tier shapes) ----

#[test]
fn the_brace_form_rides_the_names_tier() {
    // `use gadgets::{⏐` — exports; the comma form too
    let dep = tag_dep();
    for doc in ["use gadgets::{", "use gadgets::{ tag, "] {
        let items = complete_with_dep(doc, &dep);
        let ls = labels(&items);
        assert!(
            ls.contains(&"Widget"),
            "exports complete in `{doc}`: {ls:?}"
        );
        assert!(ls.contains(&"tag"), "{ls:?}");
    }
    // `use gadgets::{ Widget, ⏐` — still the full exports
    let items = complete_with_dep("use gadgets::{ Widget, ", &dep);
    assert!(labels(&items).contains(&"tag"), "{:?}", labels(&items));
    // `use gadgets::{ Widget, ta⏐` — the prefix filters
    let items = complete_with_dep("use gadgets::{ Widget, ta", &dep);
    let ls = labels(&items);
    assert!(ls.contains(&"tag"), "{ls:?}");
    assert!(
        !ls.contains(&"Widget"),
        "the prefix filters the brace form: {ls:?}"
    );
    // an unmounted head stays an honest empty
    assert!(complete_with_dep("use ghost::{", &dep).is_empty());
}

#[test]
fn string_receiver_completes_str_surface() {
    // std index ahead of the doc: `str`'s builtin contract supplies
    // the member list
    let core_src = "prelude builtin primitive str {\nfn len(self) -> i32;\n}\n";
    let c2 = rut_lexer::lexer::normalize(core_src);
    let (ctoks, _) = rut_lexer::lexer::lex(&c2);
    let (cast, _) = rut_parser::parse(&c2, rut_parser::Mode::Decl);
    let core = rut_lsp::hover::index(&c2, &cast, &ctoks);

    let doc = "fn f(s: str) -> i32 { return s. }\n";
    let d2 = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&d2);
    let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&d2, &ast, &toks);
    let idxs = [&di, &core];
    let pos = d2.rfind("s.").unwrap() as u32 + 2;
    let items = complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default());
    assert!(labels(&items).contains(&"len"), "{:?}", labels(&items));
}

// ---- the use-path tiers ----

/// completions with the cursor just after `needle`, over the std
/// surface (the doc itself carries nothing)
fn complete_over_std(doc: &str, needle: &str) -> Vec<CompletionOut> {
    let s = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&s, &ast, &toks);
    let std = rut_lsp::std_surface::indexes();
    let mut chain: Vec<&rut_lsp::hover::DefIndex> = vec![&di];
    chain.extend(std.iter());
    let pos = s.rfind(needle).unwrap() as u32 + needle.len() as u32;
    complete(&chain, &toks, &ast, pos, &s, &DocMods::default())
}

#[test]
fn use_path_completion_lists_the_chain_module_names() {
    // `use ⏐` — module names from every NAMED index, the std
    // surface included (std is not special; no hardcoded table)
    let items = complete_over_std("use ", "use ");
    let ls = labels(&items);
    for want in ["pouch", "core", "nmapset", "calc", "json"] {
        assert!(ls.contains(&want), "module `{want}` completes: {ls:?}");
    }
    let pouch = items.iter().find(|c| c.label == "pouch").unwrap();
    assert!(
        matches!(pouch.kind, CompletionKind::Module),
        "{:?}",
        pouch.kind
    );
    // the partial prefix filters
    let items = complete_over_std("use po", "use po");
    let ls = labels(&items);
    assert!(ls.contains(&"pouch"), "{ls:?}");
    assert!(!ls.contains(&"core"), "{ls:?}");
    // and the keyword tier stays OUT of the use path
    assert!(!ls.contains(&"fn"), "{ls:?}");
}

#[test]
fn use_path_completion_lists_the_modules_exports() {
    // `use pouch::V⏐` — the module's public names
    let items = complete_over_std("use pouch::V", "use pouch::V");
    let ls = labels(&items);
    assert!(ls.contains(&"Vec"), "{ls:?}");
    // the brace form rides the same tier
    let items = complete_over_std("use pouch::{V", "use pouch::{V");
    assert!(labels(&items).contains(&"Vec"), "{:?}", labels(&items));
    // a private-less name never offers (nmapset's internals are
    // pub-less and left the surface)
    let items = complete_over_std("use nmapset::H", "use nmapset::H");
    let ls = labels(&items);
    assert!(ls.contains(&"HashMap"), "{ls:?}");
    assert!(ls.contains(&"HashSet"), "{ls:?}");
}

#[test]
fn use_path_completion_lists_the_dep_namespace_shape() {
    // a dep index (the calc shape: namespace Math + consts): the
    // names tier offers the minted head, the fns, and the consts
    let src = "pub host fn sqrt(x: f64) -> f64;\n";
    let consts = vec![("PI".to_string(), std::f64::consts::PI)];
    let calc = rut_lsp::deps::index_dep(
        "calc",
        "file:///deps/calc.d.rut",
        src,
        rut_parser::Mode::Decl,
        Some("Math"),
        &consts,
    );
    let doc = "use calc::";
    let s = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&s, &ast, &toks);
    let idxs = [&di, &calc];
    let items = complete(&idxs, &toks, &ast, s.len() as u32, &s, &DocMods::default());
    let ls = labels(&items);
    assert!(ls.contains(&"Math"), "the namespace head completes: {ls:?}");
    assert!(ls.contains(&"sqrt"), "the module's fns complete: {ls:?}");
    assert!(ls.contains(&"PI"), "the consts complete: {ls:?}");
    let math = items.iter().find(|c| c.label == "Math").unwrap();
    assert!(
        matches!(math.kind, CompletionKind::Module),
        "{:?}",
        math.kind
    );
    let pi = items.iter().find(|c| c.label == "PI").unwrap();
    assert!(matches!(pi.kind, CompletionKind::Const), "{:?}", pi.kind);
}

// ---- the file-module walk (use-path + position tiers) ----

/// a mod-carrying dep, parsed and STAMPED the way the faces do:
/// `gadgets` — root (`""`) declaring `pub mod layout;` +
/// `mod secret;`; `layout` a `pub mod grid;` child with a pub
/// struct, a private fn, and a `pub(super)` fn; `layout/grid` and
/// `secret` leaves
fn gadget_dep() -> Vec<DefIndex> {
    let mk = |mod_path: &str, src: &str| {
        let s = rut_lexer::lexer::normalize(src);
        let (toks, _) = rut_lexer::lexer::lex(&s);
        let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
        let mut idx = rut_lsp::hover::index(&s, &ast, &toks);
        idx.module = Some("gadgets".to_string());
        idx.mod_path = Some(mod_path.to_string());
        idx
    };
    vec![
        mk("", "pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n"),
        mk(
            "layout",
            "pub mod grid;\npub struct Column {\n    w: i32;\n}\nfn internal() -> i32 { return 2; }\npub(super) fn for_parent() -> i32 { return 3; }\n",
        ),
        mk("layout/grid", "pub struct Cell {\n    x: i32;\n}\npub fn mk() -> Cell { return Cell { x: 1 }; }\n"),
        mk("secret", "pub fn open() -> i32 { return 4; }\nfn hidden() -> i32 { return 5; }\n"),
    ]
}

/// completions over the gadget dep with the cursor after `needle`
fn complete_over_gadgets(doc: &str, needle: &str, doc_mods: DocMods) -> Vec<CompletionOut> {
    let s = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&s, &ast, &toks);
    let dep = gadget_dep();
    let mut chain: Vec<&DefIndex> = vec![&di];
    chain.extend(dep.iter());
    let pos = s
        .rfind(needle)
        .unwrap_or_else(|| panic!("needle {needle:?} not found in:\n{s}")) as u32
        + needle.len() as u32;
    complete(&chain, &toks, &ast, pos, &s, &doc_mods)
}

#[test]
fn use_path_completion_walks_the_dep_mod_tree() {
    // `use gadgets::⏐` — the root's exports PLUS its pub mod
    // children; the private `secret` edge never offers, and the
    // child's members live under their path (no Column leak)
    let items = complete_over_gadgets("use gadgets::", "use gadgets::", DocMods::default());
    let ls = labels(&items);
    assert!(
        ls.contains(&"layout"),
        "the pub mod child completes: {ls:?}"
    );
    assert!(ls.contains(&"tag"), "the root's fn completes: {ls:?}");
    assert!(
        !ls.contains(&"secret"),
        "a bare `mod` edge never crosses: {ls:?}"
    );
    assert!(
        !ls.contains(&"Column"),
        "child members stay under their path: {ls:?}"
    );
    let layout = ls.iter().find(|l| **l == "layout").unwrap();
    let items = complete_over_gadgets("use gadgets::", "use gadgets::", DocMods::default());
    let item = items.iter().find(|c| c.label == "layout").unwrap();
    assert!(
        matches!(item.kind, CompletionKind::Module),
        "{:?}",
        item.kind
    );
    assert!(item.detail.contains("layout/mod.rut"), "{:?}", item.detail);
    let _ = layout;

    // the prefix filters the tier
    let items = complete_over_gadgets("use gadgets::la", "use gadgets::la", DocMods::default());
    let ls = labels(&items);
    assert!(ls.contains(&"layout"), "{ls:?}");
    assert!(!ls.contains(&"tag"), "{ls:?}");

    // `use gadgets::layout::⏐` — the walked module's exports, the
    // pub gate on fns now binding (crossed), plus its pub mod
    // children
    let items = complete_over_gadgets(
        "use gadgets::layout::",
        "use gadgets::layout::",
        DocMods::default(),
    );
    let ls = labels(&items);
    assert!(ls.contains(&"Column"), "{ls:?}");
    assert!(ls.contains(&"grid"), "the nested pub mod completes: {ls:?}");
    assert!(
        !ls.contains(&"for_parent"),
        "only `pub` leaf names cross packages: {ls:?}"
    );
    assert!(
        !ls.contains(&"internal"),
        "a private fn never crosses: {ls:?}"
    );

    // deeper: `use gadgets::layout::grid::⏐`
    let items = complete_over_gadgets(
        "use gadgets::layout::grid::",
        "use gadgets::layout::grid::",
        DocMods::default(),
    );
    let ls = labels(&items);
    assert!(ls.contains(&"Cell"), "{ls:?}");
    assert!(ls.contains(&"mk"), "{ls:?}");

    // the brace form rides the same walk
    let items = complete_over_gadgets(
        "use gadgets::layout::{C",
        "use gadgets::layout::{C",
        DocMods::default(),
    );
    let ls = labels(&items);
    assert!(ls.contains(&"Column"), "{ls:?}");

    // a private edge is a dead end; a ghost is a miss — both empty
    assert!(complete_over_gadgets(
        "use gadgets::secret::",
        "use gadgets::secret::",
        DocMods::default()
    )
    .is_empty());
    assert!(complete_over_gadgets(
        "use gadgets::ghost::",
        "use gadgets::ghost::",
        DocMods::default()
    )
    .is_empty());
}

#[test]
fn position_path_completion_offers_the_module_members_by_tier() {
    // the open document IS the gadgets root: `mod layout;` +
    // `mod secret;` declared in it; the doc mods say so. Positions
    // qualify with `.` (`let c: layout.Column`) — the real spelling
    let doc = DocMods {
        mod_path: Some(String::new()),
        pkg: Some("gadgets".to_string()),
    };
    let doc_src = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout.\n}\n";
    let items = complete_over_gadgets(doc_src, "let c: layout.", doc.clone());
    let ls = labels(&items);
    assert!(ls.contains(&"Column"), "the pub type completes: {ls:?}");
    assert!(
        ls.contains(&"grid"),
        "any declared edge walks intra-package: {ls:?}"
    );
    assert!(
        ls.contains(&"for_parent"),
        "pub(super) reaches the parent (the root): {ls:?}"
    );
    assert!(
        !ls.contains(&"internal"),
        "a private fn of a child stays hidden: {ls:?}"
    );

    // the `::` spelling rides the same tier (the plan's letter)
    let doc_src = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout::\n}\n";
    let items = complete_over_gadgets(doc_src, "let c: layout::", doc.clone());
    assert!(labels(&items).contains(&"Column"), "{:?}", labels(&items));

    // a child module's own file: `secret`'s members from the root —
    // pub yes, private no
    let doc_src = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let f: secret.\n}\n";
    let items = complete_over_gadgets(doc_src, "let f: secret.", doc.clone());
    let ls = labels(&items);
    assert!(ls.contains(&"open"), "{ls:?}");
    assert!(
        !ls.contains(&"hidden"),
        "a private fn of a sibling is not visible: {ls:?}"
    );

    // the own-pkg head names the root module: `gadgets.` from the
    // root offers the root's own surface + children
    let doc_src = "mod layout;\nmod secret;\npub fn root_fn() -> i32 { return 9; }\nentry fn main() -> nil {\n    let t: gadgets.\n}\n";
    let items = complete_over_gadgets(doc_src, "let t: gadgets.", doc.clone());
    let ls = labels(&items);
    assert!(ls.contains(&"root_fn"), "{ls:?}");
    assert!(ls.contains(&"layout"), "{ls:?}");

    // a deeper doc (layout/grid): a sibling via the parent scope,
    // and the pkg head reaching the root's decls
    let grid_doc = DocMods {
        mod_path: Some("layout/grid".into()),
        pkg: Some("gadgets".to_string()),
    };
    let grid_src = "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let c: layout.Column\n}\n";
    let items = complete_over_gadgets(grid_src, "let c: layout.", grid_doc.clone());
    let ls = labels(&items);
    assert!(
        ls.contains(&"Column"),
        "the sibling-of-parent resolves: {ls:?}"
    );
    // grid is a descendant of layout: layout's PRIVATES are visible
    // (the declaring module + descendants law)
    let grid_src2 = "pub struct Cell {\n    x: i32;\n}\nentry fn main() -> nil {\n    let f: layout.internal\n}\n";
    let items = complete_over_gadgets(grid_src2, "let f: layout.", grid_doc);
    let ls = labels(&items);
    assert!(
        ls.contains(&"internal"),
        "a descendant reads the parent's privates: {ls:?}"
    );

    // no doc mods (unknown provenance): the position tier stays off
    // — a plain type receiver falls through to the member tier
    // (empty for an unknown receiver — never wrong names)
    let flat_src = "mod layout;\nmod secret;\nentry fn main() -> nil {\n    let c: layout.\n}\n";
    let items = complete_over_gadgets(flat_src, "let c: layout.", DocMods::default());
    let ls = labels(&items);
    assert!(!ls.contains(&"Column"), "no doc mods, no mod tier: {ls:?}");
}

#[test]
fn the_use_path_tier_answers_an_own_package_head() {
    // the open document IS the gadgets root: `use gadgets::layout::⏐`
    // rides the own lane — the position machinery's visible-members
    // gate, so `pub(super)` reaches the root where the cross-package
    // tier's pub-only crossing law stays shut
    let doc = DocMods {
        mod_path: Some(String::new()),
        pkg: Some("gadgets".to_string()),
    };
    let items = complete_over_gadgets(
        "use gadgets::layout::",
        "use gadgets::layout::",
        doc.clone(),
    );
    let ls = labels(&items);
    assert!(ls.contains(&"Column"), "{ls:?}");
    assert!(
        ls.contains(&"for_parent"),
        "the own lane gates by visibility, not the pub-crossing law: {ls:?}"
    );
    assert!(
        !ls.contains(&"internal"),
        "a private fn stays hidden: {ls:?}"
    );

    // the bare own-pkg head: the doc's own rows are the scope (the
    // freshness law) — the declared children offer
    let doc_src = "mod layout;\nuse gadgets::";
    let items = complete_over_gadgets(doc_src, "use gadgets::", doc);
    let ls = labels(&items);
    assert!(ls.contains(&"layout"), "the own root's edges offer: {ls:?}");

    // an own mod edge head — `use layout::⏐` from the root walks
    // the ancestor spelling to the same tier
    let doc_src = "mod layout;\nuse layout::";
    let items = complete_over_gadgets(
        doc_src,
        "use layout::",
        DocMods {
            mod_path: Some(String::new()),
            pkg: Some("gadgets".to_string()),
        },
    );
    let ls = labels(&items);
    assert!(
        ls.contains(&"Column"),
        "the own mod edge head rides the same tier: {ls:?}"
    );
    assert!(ls.contains(&"grid"), "{ls:?}");
}

// ---- the type-head tier (after `Type::`) ----

#[test]
fn colon2_completion_lists_the_enum_members() {
    // `Color::⏐` — the enum head's member list rides name
    // resolution, not a receiver dot
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::\n}\n";
    let items = complete_after(src, "Color::");
    let ls = labels(&items);
    assert!(ls.contains(&"Red"), "{ls:?}");
    assert!(ls.contains(&"Green"), "{ls:?}");
    let red = items.iter().find(|c| c.label == "Red").unwrap();
    assert!(
        matches!(red.kind, CompletionKind::EnumMember),
        "{:?}",
        red.kind
    );
    // the typed prefix filters the member list
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::R\n}\n";
    let items = complete_after(src, "Color::R");
    let ls = labels(&items);
    assert!(ls.contains(&"Red"), "{ls:?}");
    assert!(!ls.contains(&"Green"), "{ls:?}");
}

#[test]
fn colon2_completion_lists_the_namespace_statics() {
    // `Math::⏐` over the calc dep — the namespace head's consts
    // and fns (the mint the manifest's namespace/consts ride)
    let src = "pub host fn sqrt(x: f64) -> f64;\n";
    let consts = vec![("PI".to_string(), std::f64::consts::PI)];
    let calc = rut_lsp::deps::index_dep(
        "calc",
        "file:///deps/calc.d.rut",
        src,
        rut_parser::Mode::Decl,
        Some("Math"),
        &consts,
    );
    let doc = "entry fn main() -> nil {\n    let x = Math::\n}\n";
    let s = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&s, &ast, &toks);
    let idxs = [&di, &calc];
    let pos = s.rfind("Math::").unwrap() as u32 + "Math::".len() as u32;
    let items = complete(&idxs, &toks, &ast, pos, &s, &DocMods::default());
    let ls = labels(&items);
    assert!(ls.contains(&"sqrt"), "the namespace's fns complete: {ls:?}");
    assert!(ls.contains(&"PI"), "the consts complete: {ls:?}");
    let pi = items.iter().find(|c| c.label == "PI").unwrap();
    assert!(matches!(pi.kind, CompletionKind::Const), "{:?}", pi.kind);
    let sqrt = items.iter().find(|c| c.label == "sqrt").unwrap();
    assert!(matches!(sqrt.kind, CompletionKind::Fn), "{:?}", sqrt.kind);
}

#[test]
fn colon2_completion_lists_the_type_head_associated_fns() {
    // `Counter::⏐` — the statics + the inherent impl block's fns
    // (the owner string IS the type's name); instance fields stay
    // OFF — they ride a receiver dot, never `::`
    let src = "\
class Counter {
n: i32;
pub static COUNT: i32;
}
impl Counter {
fn bump(mut self) -> i32 { return self.n; }
fn mk() -> Counter { return Counter { n: 0 }; }
}
entry fn main() -> nil {
    let c = Counter::
}
";
    let items = complete_after(src, "Counter::");
    let ls = labels(&items);
    assert!(ls.contains(&"bump"), "{ls:?}");
    assert!(ls.contains(&"mk"), "{ls:?}");
    assert!(ls.contains(&"COUNT"), "the static rides the head: {ls:?}");
    let count = items.iter().find(|c| c.label == "COUNT").unwrap();
    assert!(
        matches!(count.kind, CompletionKind::Field),
        "{:?}",
        count.kind
    );
    assert!(
        !ls.contains(&"n"),
        "instance fields ride a receiver: {ls:?}"
    );
}

#[test]
fn colon2_interface_head_and_deeper_walks_offer_nothing() {
    // an interface head: its signatures ride receivers, never `::`
    // — the known head is FINAL (empty), never the bare tier
    let src = "interface Greeter {\nfn greet(self) -> nil;\n}\nentry fn main() -> nil {\n    let g = Greeter::\n}\n";
    assert!(
        complete_after(src, "Greeter::").is_empty(),
        "{:?}",
        complete_after(src, "Greeter::")
    );
    // a known head with a non-matching prefix: filtered empty, final
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::X\n}\n";
    assert!(complete_after(src, "Color::X").is_empty());
    // a type head is not a namespace: nothing descends past a member
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::Red::\n}\n";
    assert!(complete_after(src, "Color::Red::").is_empty());
}

// ---- the type tier at annotation boundaries (A3) ----

#[test]
fn the_type_tier_answers_a_struct_field_annotation() {
    // `id: u⏐` — the primitive table rides the prefix filter; the
    // keyword table, the doc's fns, and the fn-locals stay OFF
    // (values never ride a type position)
    let src = "\
struct Slot {
id: u
}
fn go(ready: bool) -> nil {
    let count = 1;
    count
}
";
    let items = complete_after(src, "id: u");
    let ls = labels(&items);
    for want in ["u8", "u16", "u32", "u64"] {
        assert!(
            ls.contains(&want),
            "primitive `{want}` completes at the annotation: {ls:?}"
        );
    }
    assert!(!ls.contains(&"i32"), "the prefix filters the tier: {ls:?}");
    assert!(
        !ls.contains(&"use"),
        "no keyword tier at a type position: {ls:?}"
    );
    assert!(!ls.contains(&"fn") && !ls.contains(&"let"), "{ls:?}");
    assert!(!ls.contains(&"go"), "no fns at a type position: {ls:?}");
    assert!(
        !ls.contains(&"ready") && !ls.contains(&"count"),
        "no locals at a type position: {ls:?}"
    );
    let u32 = items.iter().find(|c| c.label == "u32").unwrap();
    assert!(matches!(u32.kind, CompletionKind::Type), "{:?}", u32.kind);
}

#[test]
fn the_type_tier_answers_the_bare_colon_boundary() {
    // `let c: ⏐` — the lone colon (the upgrade of the old honest
    // empty): the full tier, the doc's own types included
    let src = "enum Color { Red }\nentry fn main() -> nil {\n    let c: \n}\n";
    let items = complete_after(src, "let c:");
    let ls = labels(&items);
    for want in ["bool", "str", "f64", "i32", "nil"] {
        assert!(ls.contains(&want), "primitive `{want}` completes: {ls:?}");
    }
    let color = items
        .iter()
        .find(|c| c.label == "Color")
        .expect("the doc's type completes");
    assert!(
        matches!(color.kind, CompletionKind::Type),
        "{:?}",
        color.kind
    );
    assert!(
        !ls.contains(&"fn") && !ls.contains(&"let"),
        "no keywords: {ls:?}"
    );
    assert!(!ls.contains(&"main"), "no fns: {ls:?}");
}

#[test]
fn the_type_tier_answers_the_arrow_boundary() {
    // `fn f() -> ⏐` — the return annotation rides the same tier …
    let src = "struct Thing {\nx: i32;\n}\nfn f() -> \n";
    let items = complete_after(src, "-> ");
    let ls = labels(&items);
    assert!(ls.contains(&"i32") && ls.contains(&"Thing"), "{ls:?}");
    assert!(
        !ls.contains(&"fn") && !ls.contains(&"f"),
        "no keywords, no fns: {ls:?}"
    );
    // … with the partial ident prefix-filtering: `-> T⏐`
    let src = "struct Thing {\nx: i32;\n}\nfn f() -> T\n";
    let items = complete_after(src, "-> T");
    let ls = labels(&items);
    assert!(ls.contains(&"Thing"), "{ls:?}");
    assert!(
        !ls.contains(&"i32"),
        "the prefix filters the primitives: {ls:?}"
    );
}

#[test]
fn the_type_tier_answers_a_param_annotation() {
    // `fn f(x: u⏐` — parameter annotations are type positions too
    let src = "fn f(x: u) -> nil { }\n";
    let items = complete_after(src, "x: u");
    let ls = labels(&items);
    assert!(ls.contains(&"u32") && ls.contains(&"u64"), "{ls:?}");
    assert!(!ls.contains(&"fn"), "{ls:?}");
}

#[test]
fn the_type_tier_enumerates_exactly_the_primitive_table() {
    // the tier's primitive rows ARE the parser's one PRIMITIVE_TYS
    // table — never a duplicated, drifting list
    let src = "entry fn main() -> nil {\n    let c: \n}\n";
    let items = complete_after(src, "let c:");
    let mut got: Vec<&str> = labels(&items)
        .into_iter()
        .filter(|l| rut_parser::is_primitive_ty(l))
        .collect();
    got.sort_unstable();
    let mut want = rut_parser::PRIMITIVE_TYS.to_vec();
    want.sort_unstable();
    assert_eq!(got, want, "the tier's primitives are exactly PRIMITIVE_TYS");
}

#[test]
fn the_type_tier_offers_the_ambient_and_imported_foreign_types() {
    // a foreign doc: its own types plus the ambient builtin row
    // complete; the `pub builtin` gated row completes only once the
    // document's `use` names it
    let core_src = "pub builtin class Weak { }\nprelude builtin class StackTrace { }\n";
    let c2 = rut_lexer::lexer::normalize(core_src);
    let (ctoks, _) = rut_lexer::lexer::lex(&c2);
    let (cast, _) = rut_parser::parse(&c2, rut_parser::Mode::Decl);
    let mut core = rut_lsp::hover::index(&c2, &cast, &ctoks);
    core.module = Some("core".to_string());

    let mk = |doc: &str| {
        let d2 = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&d2);
        let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
        let di = rut_lsp::hover::index(&d2, &ast, &toks);
        let idxs = [&di, &core];
        let pos = d2.rfind("let e:").unwrap() as u32 + "let e:".len() as u32;
        complete(&idxs, &toks, &ast, pos, &d2, &DocMods::default())
    };

    let items = mk("entry fn main() -> nil {\n    let e: \n}\n");
    let ls = labels(&items);
    assert!(
        ls.contains(&"StackTrace"),
        "the ambient builtin type completes: {ls:?}"
    );
    assert!(
        !ls.contains(&"Weak"),
        "the gated builtin type stays off: {ls:?}"
    );

    let items = mk("use core::{ Weak };\nentry fn main() -> nil {\n    let e: \n}\n");
    let ls = labels(&items);
    assert!(
        ls.contains(&"Weak"),
        "the imported builtin type completes: {ls:?}"
    );
}

// ---- the auto-import tier ----

/// the auto-import item for `label`, if the tier offered one (an
/// item with additional text edits)
fn import_item<'a>(items: &'a [CompletionOut], label: &str) -> Option<&'a CompletionOut> {
    items
        .iter()
        .find(|c| c.label == label && !c.additional_text_edits.is_empty())
}

#[test]
fn auto_import_offers_foreign_names_with_the_insert_edit() {
    let doc = "entry fn main() -> nil {\n    let v = Ve\n}\n";
    let s = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&s, &ast, &toks);
    let std = rut_lsp::std_surface::indexes();
    let mut chain: Vec<&rut_lsp::hover::DefIndex> = vec![&di];
    chain.extend(std.iter());
    let pos = s.rfind("Ve").unwrap() as u32 + 2;
    let items = complete(&chain, &toks, &ast, pos, &s, &DocMods::default());
    let imp = import_item(&items, "Vec").expect("Vec auto-imports from pouch");
    assert_eq!(imp.detail, "pouch::Vec — import");
    assert_eq!(
        imp.sort_text.as_deref(),
        Some("~Vec"),
        "the tier sorts after the locals"
    );
    // the insert: no uses, no leading comments → top of the body,
    // a fresh line, over the SAME normalized coordinates
    assert_eq!(imp.additional_text_edits.len(), 1);
    let (range, text) = &imp.additional_text_edits[0];
    assert_eq!((range.start.line, range.start.character), (0, 0));
    assert_eq!(range.end, range.start, "an insertion");
    assert_eq!(text, "use pouch::Vec;\n");
}

#[test]
fn auto_import_lands_after_the_last_leading_use() {
    let doc = "use core::{ NAN };\n\nentry fn main() -> nil {\n    let v = Ve\n}\n";
    let s = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&s, &ast, &toks);
    let std = rut_lsp::std_surface::indexes();
    let mut chain: Vec<&rut_lsp::hover::DefIndex> = vec![&di];
    chain.extend(std.iter());
    let pos = s.rfind("Ve").unwrap() as u32 + 2;
    let items = complete(&chain, &toks, &ast, pos, &s, &DocMods::default());
    let imp = import_item(&items, "Vec").expect("Vec auto-imports");
    let (range, text) = &imp.additional_text_edits[0];
    // right after the leading use's `;` — column 18 of line 0 —
    // with the fresh use on the next line
    assert_eq!((range.start.line, range.start.character), (0, 18));
    assert_eq!(text, "\nuse pouch::Vec;");
}

#[test]
fn auto_import_skips_leading_comments() {
    let doc = "// the header prose\n\nentry fn main() -> nil {\n    let v = Ve\n}\n";
    let s = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&s, &ast, &toks);
    let std = rut_lsp::std_surface::indexes();
    let mut chain: Vec<&rut_lsp::hover::DefIndex> = vec![&di];
    chain.extend(std.iter());
    let pos = s.rfind("Ve").unwrap() as u32 + 2;
    let items = complete(&chain, &toks, &ast, pos, &s, &DocMods::default());
    let imp = import_item(&items, "Vec").expect("Vec auto-imports");
    let (range, text) = &imp.additional_text_edits[0];
    // top of the file BODY — after the comment block and the blank
    assert_eq!((range.start.line, range.start.character), (2, 0));
    assert_eq!(text, "use pouch::Vec;\n");
}

#[test]
fn already_imported_names_are_not_re_offered() {
    let doc = "use pouch::{ Vec };\nentry fn main() -> nil {\n    let v = Ve\n}\n";
    let s = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let di = rut_lsp::hover::index(&s, &ast, &toks);
    let std = rut_lsp::std_surface::indexes();
    let mut chain: Vec<&rut_lsp::hover::DefIndex> = vec![&di];
    chain.extend(std.iter());
    let pos = s.rfind("Ve").unwrap() as u32 + 2;
    let items = complete(&chain, &toks, &ast, pos, &s, &DocMods::default());
    // the name still completes (the bare tier), but NO import edit
    // rides it — the document already imports it
    assert!(
        import_item(&items, "Vec").is_none(),
        "the already-imported name must not re-offer: {items:?}"
    );
}

// ---- constructor-call inference + the dot guard ----

#[test]
fn constructor_call_binding_completes_the_ink_surface() {
    // `let log = Logger("h")` — the constructor-call form types the
    // binding from the indexed class (the embedded ink surface), so
    // `log.` lists the impl methods (new/debug/info/warn/error/log)
    let doc = "fn go() -> nil {\n    let log = Logger(\"h\");\n    log.\n}\n";
    let items = complete_over_std(doc, "log.");
    let ls = labels(&items);
    for want in ["new", "debug", "info", "warn", "error"] {
        assert!(
            ls.contains(&want),
            "`{want}` completes through the ctor binding: {ls:?}"
        );
    }
    // the bare tier's host fns never dump into a member position
    assert!(!ls.contains(&"abs"), "no host-fn dump after a dot: {ls:?}");
}

#[test]
fn dot_guard_unknown_receiver_is_empty() {
    // `someUnknown.` — the receiver resolves to nothing: an EMPTY
    // list, never the keyword/host-fn dump (a miss is an empty list)
    let doc = "fn go() -> nil {\n    someUnknown.\n}\n";
    let items = complete_over_std(doc, "someUnknown.");
    assert!(
        items.is_empty(),
        "an unresolved receiver completes nothing: {:?}",
        labels(&items)
    );
}

#[test]
fn dot_guard_non_name_receiver_is_empty() {
    // `probe().` — the token before the dot is not a name the tiers
    // understand; the text-backed dot guard keeps the bare tier out
    let doc = "fn go() -> nil {\n    let x = probe().\n}\n";
    let items = complete_after(doc, "probe().");
    assert!(items.is_empty(), "{:?}", labels(&items));
}

// ---- the locals tier (plan3 phase 1, A1) ----

#[test]
fn locals_complete_at_a_bare_position_ahead_of_the_keywords() {
    // `che⏐` mid-block: the fn-local `checked` and the param `ready`
    // ride their own kinds, ahead of the keyword table
    let src = "\
fn go(ready: bool) -> nil {
    let mut checked: str = \" \";
    che
}
";
    let items = complete_after(src, "    che\n");
    let ls = labels(&items);
    assert!(ls.contains(&"checked"), "the local completes: {ls:?}");
    assert!(ls.contains(&"ready"), "the param completes: {ls:?}");
    let checked = items.iter().find(|c| c.label == "checked").unwrap();
    assert!(
        matches!(checked.kind, CompletionKind::Variable),
        "{:?}",
        checked.kind
    );
    assert!(
        checked.detail.contains("let binding") && checked.detail.contains("str"),
        "{}",
        checked.detail
    );
    let ready = items.iter().find(|c| c.label == "ready").unwrap();
    assert!(
        matches!(ready.kind, CompletionKind::Parameter),
        "{:?}",
        ready.kind
    );
    // ahead of the keyword table
    let checked_at = items.iter().position(|c| c.label == "checked").unwrap();
    let let_kw_at = items.iter().position(|c| c.label == "let").unwrap();
    assert!(checked_at < let_kw_at, "locals ride ahead of the keywords");
}

#[test]
fn locals_offer_loop_variables_where_their_scope_reaches() {
    // the for-of var inside its body; the for-c counter inside the
    // cond/update too (its scope is the whole loop statement)
    let src = "\
fn go(xs: [i32]) -> nil {
    for (let v of xs) {
        vv
    }
}
";
    let items = complete_after(src, "        vv");
    let v = items
        .iter()
        .find(|c| c.label == "v")
        .expect("the for-of var completes");
    assert!(matches!(v.kind, CompletionKind::Variable), "{:?}", v.kind);
    assert!(v.detail.contains("loop variable"), "{}", v.detail);

    let src = "\
fn go(n: i32) -> nil {
    let sum = 0;
    for (let i = 0; i < n; i += su) {
        sum += i;
    }
}
";
    // `su` inside the UPDATE — the counter `i` and the outer `sum`
    // both still in scope
    let items = complete_after(src, "i += su");
    let ls = labels(&items);
    assert!(
        ls.contains(&"i"),
        "the counter completes in the update: {ls:?}"
    );
    assert!(ls.contains(&"sum"), "{ls:?}");
    assert!(ls.contains(&"n"), "the param reaches the loop: {ls:?}");
    let i = items.iter().find(|c| c.label == "i").unwrap();
    assert!(i.detail.contains("i32"), "{}", i.detail);
}

#[test]
fn locals_shadow_inside_and_exit_their_scope() {
    // inside the if-block the INNER `flag` wins (the i32 detail —
    // one entry per name, the innermost in-scope declaration rides)
    let src = "\
fn go(ready: bool) -> nil {
    let flag: str = \"a\";
    if (ready) {
        let flag: i32 = 1;
        fl
    }
    fl
}
";
    let items = complete_after(src, "        fl");
    let flag = items
        .iter()
        .find(|c| c.label == "flag")
        .expect("the shadow completes");
    assert!(
        flag.detail.contains("i32"),
        "the inner shadow wins inside its scope: {}",
        flag.detail
    );
    // scope-exit: a binding declared INSIDE the block is gone past
    // it, while the fn-wide param rides on
    let src = "\
fn go(ready: bool) -> nil {
    if (ready) {
        let tmp: i32 = 1;
        tm
    }
    tm
}
";
    let items = complete_after(src, "        tm");
    assert!(
        labels(&items).contains(&"tmp"),
        "inside its block the local completes"
    );
    let items = complete_after(src, "    tm\n");
    let ls = labels(&items);
    assert!(
        !ls.contains(&"tmp"),
        "the binding does not leak past its block: {ls:?}"
    );
    assert!(
        ls.contains(&"ready"),
        "the fn-wide param rides past the block: {ls:?}"
    );
}

#[test]
fn locals_skip_their_own_declaration_ident() {
    // the cursor inside the declaring ident — `let ch⏐e` — never
    // offers the name being typed; a use after the decl does
    let src = "fn go() -> nil {\n    let che = 1;\n}\n";
    let items = complete_after(src, "let ch");
    assert!(
        !labels(&items).contains(&"che"),
        "the declaration's own ident stays off"
    );
    assert!(
        !labels(&items).contains(&"ch"),
        "the half-typed name stays off: {:?}",
        labels(&items)
    );
    let src = "fn go() -> nil {\n    let che = 1;\n    che\n}\n";
    let items = complete_after(src, "    che\n");
    assert!(labels(&items).contains(&"che"), "{:?}", labels(&items));
}

// ---- the end-of-buffer scope fix (plan3 phase 1, A7) — the doc
// ends at/inside the incomplete statement, the parser ends the
// block's span at the EOF token, the cursor sits ON `hi`; the fn
// body's scope extends through the source end so resolution keeps
// working at the most common typing moment. No scope leaks OUT of
// real (closed) blocks. ----

/// the List probe doc's head — a type with a field and an impl
/// method for the member tier to answer with
const EOF_LIST_HEAD: &str = "\
class List {
    items: i32;
}
impl List {
    fn count(self) -> i32 { return 0; }
}
";

#[test]
fn member_completion_survives_the_end_of_buffer() {
    // THE regression: the text ends EXACTLY at the incomplete stmt
    let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.");
    let items = complete_after(&src, "    list.");
    let ls = labels(&items);
    assert!(ls.contains(&"items"), "the field completes at EOF: {ls:?}");
    assert!(ls.contains(&"count"), "the method completes at EOF: {ls:?}");
    // never the bare keyword dump
    assert!(!ls.contains(&"fn"), "no keyword dump after a dot: {ls:?}");

    // one trailing newline — the same hole (the block span still
    // ends at the EOF token)
    let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.\n");
    let items = complete_after(&src, "    list.");
    let ls = labels(&items);
    assert!(
        ls.contains(&"items") && ls.contains(&"count"),
        "the member list survives the newline: {ls:?}"
    );

    // last stmt before `}` — the closed-block shape guards today's
    // behavior behind the fix
    let src =
        format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    list.\n}}\n");
    let items = complete_after(&src, "    list.");
    let ls = labels(&items);
    assert!(ls.contains(&"items") && ls.contains(&"count"), "{ls:?}");
}

#[test]
fn locals_complete_at_the_end_of_buffer() {
    // A1 at EOF (post-A7): the doc ends mid-statement
    let src = "fn go() -> nil {\n    let mut checked: str = \" \";\n    che";
    let items = complete_after(src, "    che");
    let ls = labels(&items);
    assert!(
        ls.contains(&"checked"),
        "the local completes at EOF: {ls:?}"
    );

    // the receiver itself still resolves for member completion when
    // the receiver is the LAST thing before the cursor
    let src = format!("{EOF_LIST_HEAD}fn go() -> nil {{\n    let list = List.new();\n    li");
    let items = complete_after(&src, "    li");
    assert!(
        labels(&items).contains(&"list"),
        "the local completes at EOF: {:?}",
        labels(&items)
    );
}

#[test]
fn the_end_of_buffer_extension_never_leaks_a_closed_block() {
    // a REAL `}` ends the scope exactly as before: the cursor past
    // it (at the source end) sees no fn-local — no leak out of the
    // closed block, with or without a trailing newline
    for (src, needle) in [
        (
            "fn go() -> nil {\n    let checked: str = \" \";\n}\n",
            "}\n",
        ),
        ("fn go() -> nil {\n    let checked: str = \" \";\n}", "}"),
    ] {
        let items = complete_after(src, needle);
        let ls = labels(&items);
        assert!(
            !ls.contains(&"checked"),
            "no leak past a closed block ({src:?}): {ls:?}"
        );
    }
    // and a closed first fn never leaks into an unterminated second
    let src = "fn a() -> nil {\n    let alpha: i32 = 1;\n}\nfn b() -> nil {\n    let beta: i32 = 2;\n    be";
    let items = complete_after(src, "    be");
    let ls = labels(&items);
    assert!(
        ls.contains(&"beta"),
        "the unterminated fn's local completes: {ls:?}"
    );
    assert!(
        !ls.contains(&"alpha"),
        "the closed fn's local stays in: {ls:?}"
    );
}
