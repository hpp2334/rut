use rut_ast::ast::Ast;
use rut_lexer::lexer::{lex, normalize};
use rut_lexer::token::Token;
use rut_lsp::definition::*;
use rut_lsp::hover::DefIndex;
use rut_lsp::line_index::LineIndex;
use rut_lsp::mods::DocMods;
use rut_parser::parse;
use rut_parser::Mode;

const DOC: &str = "file:///t/main.rut";

struct Doc {
    src: String,
    toks: Vec<Token>,
    ast: Ast,
    index: DefIndex,
}

fn doc_at(uri: &str, src: &str) -> Doc {
    let src = normalize(src);
    let (toks, _) = lex(&src);
    let (ast, _) = parse(&src, Mode::Impl);
    let mut index = rut_lsp::hover::index(&src, &ast, &toks);
    index.origin = uri.to_string();
    Doc {
        src,
        toks,
        ast,
        index,
    }
}

fn doc(src: &str) -> Doc {
    doc_at(DOC, src)
}

/// run `query` against the ctx built from `d` + `extra` (the chain
/// lives here, so the Ctx's borrow of it is valid for the call);
/// the doc context rides as the face's knowledge (`dm`)
fn run(
    d: &Doc,
    extra: &[DefIndex],
    dm: &DocMods,
    query: impl Fn(&Ctx) -> Vec<DefLocation>,
) -> Vec<DefLocation> {
    let idxs: Vec<&DefIndex> = std::iter::once(&d.index).chain(extra.iter()).collect();
    let ctx = Ctx {
        doc_uri: &d.index.origin,
        toks: &d.toks,
        ast: &d.ast,
        idxs: &idxs,
        doc_mods: dm,
    };
    query(&ctx)
}

/// byte offset of the `n`th (1-based) occurrence of `needle`
fn at(src: &str, needle: &str, occurrence: usize) -> u32 {
    let mut from = 0;
    for _ in 0..occurrence {
        match src[from..].find(needle) {
            Some(i) => from += i + needle.len(),
            None => panic!("needle {needle:?} not found (occurrence {occurrence})"),
        }
    }
    (from - needle.len()) as u32
}

/// the byte span a DefLocation points at, over `src`
fn span_of(src: &str, ix: &LineIndex, l: &DefLocation) -> (u32, u32) {
    (
        ix.byte(src, l.range.start.line, l.range.start.character),
        ix.byte(src, l.range.end.line, l.range.end.character),
    )
}

fn def(d: &Doc, pos: u32, extra: &[DefIndex]) -> Vec<DefLocation> {
    def_as(d, pos, extra, &DocMods::default())
}

/// the doc-context lane — the own-package queries ride a `DocMods`
fn def_as(d: &Doc, pos: u32, extra: &[DefIndex], dm: &DocMods) -> Vec<DefLocation> {
    run(d, extra, dm, |c| definition(c, pos))
}

fn typedef(d: &Doc, pos: u32, extra: &[DefIndex]) -> Vec<DefLocation> {
    run(d, extra, &DocMods::default(), |c| type_definition(c, pos))
}

const WIDGET_DOC: &str = "class Widget {\n    id: i32;\n}\n";
const WIDGET_URI: &str = "file:///ws/gadgets/lib.rut";

fn widget_index() -> DefIndex {
    doc_at(WIDGET_URI, WIDGET_DOC).index
}

// ---- layer 1: the binding pass ----

#[test]
fn local_binding_jumps_to_declaring_ident() {
    let src = "fn go() -> i32 {\n    let c = 1;\n    return c;\n}\n";
    let d = doc(src);
    let hits = def(&d, at(&d.src, "return c;", 1) + 7, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, DOC);
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "let c", 1) + 4; // the `c` inside `let c`
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
}

#[test]
fn shadowing_picks_the_latest_decl() {
    // rut has no bare-block statements — if blocks are the scope form
    let src = [
        "fn go() -> i32 {",
        "    let v = 1;",
        "    if (v > 0) {",
        "        let v = 2;",
        "        return v;",
        "    }",
        "    return 0;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    let hits = def(&d, at(&d.src, "return v;", 1) + 7, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "let v = 2;", 1) + 4; // the INNER `v`
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
}

#[test]
fn scope_exit_keeps_the_outer_binding() {
    let src = [
        "fn go() -> i32 {",
        "    let v = 1;",
        "    if (v > 0) {",
        "        let v = 2;",
        "    }",
        "    return v;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    let hits = def(&d, at(&d.src, "return v;", 1) + 7, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "let v = 1;", 1) + 4; // the OUTER `v` — the inner one is out of scope
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
}

#[test]
fn param_jumps_to_its_decl() {
    let src = "fn go(p: Point) -> f64 {\n    return p.x;\n}\nstruct Point {\n    x: f64;\n}\n";
    let d = doc(src);
    let hits = def(&d, at(&d.src, "p: Point", 1), &[]); // the param decl site → itself
    assert_eq!(hits.len(), 1);
    let hits = def(&d, at(&d.src, "return p", 1) + 7, &[]); // the use site
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "go(p: Point)", 1) + 3;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
}

// ---- layer 2: the decl layer ----

#[test]
fn field_read_jumps_to_the_field_decl() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "struct Wrap {",
        "    c: Circle;",
        "}",
        "fn go(wrap: Wrap) -> f64 {",
        "    let inner = wrap.c;",
        "    return inner.r;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // `wrap.c` — the `c` after the dot → the field decl ident
    let hits = def(&d, at(&d.src, "wrap.c;", 1) + 5, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "c: Circle;", 1); // the declaring `c`
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
    // the field DECL site answers itself
    let hits = def(&d, at(&d.src, "c: Circle;", 1), &[]);
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 1));
}

#[test]
fn method_call_jumps_to_the_impl_fn_name() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "impl Circle {",
        "    fn area(self) -> f64 { return 3.14; }",
        "}",
        "fn go() -> f64 {",
        "    let c = Circle::new(1.0);",
        "    return c.area();",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    let hits = def(&d, at(&d.src, "c.area();", 1) + 2, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "fn area(self)", 1) + 3;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 4));
}

#[test]
fn enum_member_jumps_to_its_decl_ident() {
    let src = "enum Color { Red, Blue }\nfn pick(c: Color) -> Color {\n    return c;\n}\nfn go() -> Color {\n    return Color::Red;\n}\n";
    let d = doc(src);
    let hits = def(&d, at(&d.src, "Color::Red;", 1) + 7, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "Red,", 1);
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 3));
}

#[test]
fn type_and_fn_names_jump_to_their_decls() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "fn make() -> Circle {",
        "    return Circle::new(1.0);",
        "}",
        "entry fn main() -> nil {",
        "    let c = make();",
        "    let d = make();",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // the type token in `Circle::new` → the class decl ident
    let hits = def(&d, at(&d.src, "Circle::new(1.0);", 1), &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "class Circle", 1) + 6;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
    // a free-fn call site → the fn decl ident
    let hits = def(&d, at(&d.src, "= make();", 2) + 2, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let want = at(&d.src, "fn make()", 1) + 3;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 4));
}

#[test]
fn module_let_use_site_jumps_to_the_decl() {
    let src = "let TOTAL = 41;\nfn bump() -> i32 {\n    return TOTAL + 1;\n}\n";
    let d = doc(src);
    let hits = def(&d, at(&d.src, "return TOTAL", 1) + 7, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "let TOTAL", 1) + 4;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 5));
}

// ---- layer 3: the use graph ----

#[test]
fn use_name_jumps_to_the_exporting_module() {
    let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    let d = doc(src);
    let gadgets = widget_index();
    let extra = [gadgets];
    // the use-statement name itself
    let hits = def(&d, at(&d.src, "use gadgets::{ Widget }", 1) + 15, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, WIDGET_URI);
    let ix = LineIndex::new(&normalize(WIDGET_DOC));
    let want = at(&normalize(WIDGET_DOC), "class Widget", 1) + 6;
    assert_eq!(
        span_of(&normalize(WIDGET_DOC), &ix, &hits[0]),
        (want, want + 6)
    );
    // and a usage site resolves through the same edge
    let hits = def(&d, at(&d.src, "= Widget::new();", 1) + 2, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, WIDGET_URI);
}

#[test]
fn the_use_graph_restricts_the_search_to_the_imported_pkg() {
    let src = "use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget::new();\n}\n";
    let d = doc(src);
    let mut foreign = doc("class Widget {\n    other: i32;\n}\n").index;
    foreign.origin = "file:///ws/other/lib.rut".to_string();
    let extra = [widget_index(), foreign];
    let hits = def(&d, at(&d.src, "= Widget::new();", 1) + 2, &extra);
    assert_eq!(
        hits.len(),
        1,
        "ambiguity from the OTHER pkg must not leak: {hits:?}"
    );
    assert_eq!(hits[0].uri, WIDGET_URI);
}

#[test]
fn unimported_names_keep_the_flat_chain() {
    let src = "entry fn main() -> nil {\n    let w = Widget::new();\n}\n";
    let d = doc(src);
    let extra = [widget_index()];
    let hits = def(&d, at(&d.src, "= Widget::new();", 1) + 2, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, WIDGET_URI);
}

// ---- the pkg's two files: the body over the decl surface ----

#[test]
fn a_both_files_dep_answers_from_the_body() {
    // the dep walk indexes a pkg's `.d.rut` surface (Decl) AND its
    // root `mod.rut` body (Impl): the leaf resolves to the BODY's
    // declaring ident (a use site too), and the pkg head lands at
    // the body root's file top (0:0)
    let src = "use pkg::{ make };\nentry fn main() -> nil {\n    let x = make();\n}\n";
    let d = doc(src);
    let surface = rut_lsp::deps::index_dep(
        "pkg",
        "file:///deps/pkg/pkg.d.rut",
        "pub host fn make() -> i32;\n",
        Mode::Decl,
        None,
        &[],
    );
    let body = rut_lsp::deps::index_dep_mod(
        "pkg",
        "file:///deps/pkg/mod.rut",
        "",
        "pub fn make() -> i32 { return 1; }\n",
        Mode::Impl,
        None,
        &[],
    );
    let extra = [surface, body];
    let body_src = normalize("pub fn make() -> i32 { return 1; }\n");
    let ix = LineIndex::new(&body_src);
    // the use leaf: the body's `make`, not the surface's
    let hits = def(&d, at(&d.src, "use pkg::{ make }", 1) + 11, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///deps/pkg/mod.rut");
    let want = at(&body_src, "fn make()", 1) + 3;
    assert_eq!(span_of(&body_src, &ix, &hits[0]), (want, want + 4));
    // a use site rides the same preference
    let hits = def(&d, at(&d.src, "= make();", 1) + 2, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///deps/pkg/mod.rut");
    // the pkg head: the body root, file top
    let hits = def(&d, at(&d.src, "use pkg::", 1) + 4, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///deps/pkg/mod.rut");
    assert_eq!(span_of(&body_src, &ix, &hits[0]), (0, 0));
}

#[test]
fn a_decl_only_host_pkg_lands_in_the_surface() {
    // a host pkg indexes its `.d.rut` surface ALONE (calc's
    // shape): leaf and pkg head both land in it — the surface is
    // all there is, so it is kept
    let src = "use calc::{ sqrt };\nentry fn main() -> nil {\n    let x = sqrt(1.0);\n}\n";
    let d = doc(src);
    let surface = rut_lsp::deps::index_dep(
        "calc",
        "file:///deps/calc/calc.d.rut",
        "pub host fn sqrt(x: f64) -> f64;\n",
        Mode::Decl,
        None,
        &[],
    );
    let extra = [surface];
    let surface_src = normalize("pub host fn sqrt(x: f64) -> f64;\n");
    let ix = LineIndex::new(&surface_src);
    let want = at(&surface_src, "fn sqrt", 1) + 3;
    // the leaf
    let hits = def(&d, at(&d.src, "use calc::{ sqrt }", 1) + 12, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///deps/calc/calc.d.rut");
    assert_eq!(span_of(&surface_src, &ix, &hits[0]), (want, want + 4));
    // the pkg head: the flat root, file top
    let hits = def(&d, at(&d.src, "use calc::", 1) + 4, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///deps/calc/calc.d.rut");
    assert_eq!(span_of(&surface_src, &ix, &hits[0]), (0, 0));
}

#[test]
fn a_mounted_root_beats_the_ambient_surface_twin() {
    // the transitive-mount world: a pkg the walk mounted (a real
    // file:// origin) AND the ambient std surface's embed of the
    // same name (scheme-less `rut/…`) both carry a flat Decl root —
    // the mount wins for the pkg head and the leaf: the workspace
    // pinned these bytes, the embed is only ever the fallback
    let src = "use ink_host::{ create_logger };\nentry fn main() -> nil {\n    create_logger(\"a\");\n}\n";
    let d = doc(src);
    let ambient = rut_lsp::deps::index_dep(
        "ink_host",
        "rut/ink_host/src/ink_host.d.rut",
        "pub host fn create_logger(name: str) -> opaque;\n",
        Mode::Decl,
        None,
        &[],
    );
    let mounted = rut_lsp::deps::index_dep(
        "ink_host",
        "file:///ws/.rut/deps/ink-eb2b568316ef/deps/ink_host@7e9a5d738fbd9797/src/ink_host.d.rut",
        "pub host fn create_logger(name: str) -> opaque;\n",
        Mode::Decl,
        None,
        &[],
    );
    // the chain order puts the ambient surface first (registered at
    // init, before any mount)
    let extra = [ambient, mounted];
    // the pkg head: the MOUNTED surface's file top, never the embed
    let hits = def(&d, at(&d.src, "use ink_host::", 1) + 4, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0].uri,
        "file:///ws/.rut/deps/ink-eb2b568316ef/deps/ink_host@7e9a5d738fbd9797/src/ink_host.d.rut"
    );
    assert_eq!(span_of(&d.src, &LineIndex::new(&d.src), &hits[0]), (0, 0));
    // the leaf: the ambiguity peek may answer both twins, the
    // MOUNTED one among them (the real defs store coalesces the
    // module's twins to one — one answer, the mounted file)
    let hits = def(
        &d,
        at(&d.src, "use ink_host::{ create_logger }", 1) + 18,
        &extra,
    );
    assert!(
        hits.iter()
            .any(|h| h.uri.starts_with("file:///ws/.rut/deps/")),
        "{hits:?}"
    );
}

#[test]
fn a_ctor_decl_click_answers_itself() {
    // `[constructor] fn new` — the one name every class reuses. The
    // decl click answers ITSELF (the decl-site law covers fn
    // decls), never the cross-pkg fan-out of every other class's
    // ctor the name layers would peek
    let src = [
        "class Logger {",
        "    internal: opaque;",
        "}",
        "",
        "impl Logger {",
        "    [constructor] pub fn new(name: str) -> Self {",
        "        return Self { internal: 1 };",
        "    }",
        "}",
    ]
    .join("\n");
    let d = doc(&src);
    let other = doc_at(
        "file:///ws/flow/lib.rut",
        "class Ctx {\n    internal: opaque;\n}\n\nimpl Ctx {\n    [constructor] pub fn new(cap: i32) -> Self {\n        return Self { internal: 2 };\n    }\n}\n",
    )
    .index;
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "fn new", 1) + 3;
    let hits = def(&d, want, &[other]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, DOC);
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 3));
}

#[test]
fn a_fn_decl_click_answers_itself_despite_a_foreign_twin() {
    // a plain fn decl click: the name layers would peek every pkg
    // carrying the same fn name — the decl-site law answers the
    // doc's own declaration, full stop
    let src = "fn make() -> i32 { return 1; }\nentry fn main() -> nil {\n    let x = make();\n}\n";
    let d = doc(src);
    let other = doc_at(
        "file:///ws/other/lib.rut",
        "fn make() -> i32 { return 2; }\n",
    )
    .index;
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "fn make", 1) + 3;
    let hits = def(&d, want, &[other]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, DOC);
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 4));
}

#[test]
fn a_doubled_twin_answer_is_one_answer() {
    // the stamped twin: an open doc is indexed by its analyze lane
    // AND stamped by the mount lane at the same origin — a name both
    // carry resolved twice. Identical answers are one answer; the
    // ambiguity peek is for DISTINCT targets
    let src = "class Widget {\n    id: i32;\n}\n";
    let d = doc(src);
    let twin = doc_at(DOC, src).index; // the same origin, stamped twin
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "Widget", 1);
    let hits = def(&d, want, &[twin]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, DOC);
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
}

#[test]
fn a_mounted_member_beats_the_ambient_twin() {
    // the member lane rides the same mounted-over-embed law the ty
    // and fn layers carry: the chain holds class Logger twice — the
    // ambient std surface (scheme-less `rut/…` src_path) and the dep
    // walk's mounted tree (a real file:// origin) — and BOTH member
    // shapes land in the mount: the impl method and the field read
    let src = [
        "use ink::{ Logger };",
        "",
        "entry fn main() -> nil {",
        "    let log = Logger(\"a\");",
        "    log.info(\"x\");",
        "    print(log.internal);",
        "}",
    ]
    .join("\n");
    let d = doc(&src);
    let surface = "class Logger {\n    internal: opaque;\n}\n\nimpl Logger {\n    pub fn info(msg: str) -> nil { }\n}\n";
    let mut ambient = doc_at("rut/ink/src/mod.rut", surface).index;
    ambient.src_path = Some("rut/ink/src/mod.rut".into());
    let mounted = doc_at("file:///ws/.rut/deps/ink-eb2b568316ef/src/mod.rut", surface).index;
    let extra = [ambient, mounted];
    let surface_src = normalize(surface);
    let six = LineIndex::new(&surface_src);
    // the impl method: `info` in `log.info("x")`
    let hits = def(&d, at(&d.src, "log.info", 1) + 4, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0].uri,
        "file:///ws/.rut/deps/ink-eb2b568316ef/src/mod.rut"
    );
    let want = at(&surface_src, "fn info", 1) + 3;
    assert_eq!(span_of(&surface_src, &six, &hits[0]), (want, want + 4));
    // the field read: `internal` in `print(log.internal)`
    let hits = def(&d, at(&d.src, "log.internal", 1) + 4, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0].uri,
        "file:///ws/.rut/deps/ink-eb2b568316ef/src/mod.rut"
    );
}

#[test]
fn a_middle_use_segment_jumps_to_the_mod_edge() {
    // `use gadgets::layout::{ Column };` — the `layout` segment
    // jumps to the declaring `pub mod layout;` edge in the pkg
    // root's mod.rut (the dep lane; the e2e face re-covers it)
    let src = "use gadgets::layout::{ Column };\nentry fn main() -> nil {\n}\n";
    let d = doc(src);
    let root = rut_lsp::deps::index_dep_mod(
        "gadgets",
        "file:///deps/gadgets/mod.rut",
        "",
        "pub mod layout;\npub fn tag() -> i32 { return 1; }\n",
        Mode::Impl,
        None,
        &[],
    );
    let layout = rut_lsp::deps::index_dep_mod(
        "gadgets",
        "file:///deps/gadgets/layout/mod.rut",
        "layout",
        "pub struct Column {\n    w: i32;\n}\n",
        Mode::Impl,
        None,
        &[],
    );
    let extra = [root, layout];
    let root_src = normalize("pub mod layout;\npub fn tag() -> i32 { return 1; }\n");
    let ix = LineIndex::new(&root_src);
    // the `layout` segment (not the pkg head, not the leaf)
    let hits = def(&d, at(&d.src, "gadgets::layout", 1) + 9, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///deps/gadgets/mod.rut");
    let want = at(&root_src, "mod layout;", 1) + 4;
    assert_eq!(span_of(&root_src, &ix, &hits[0]), (want, want + 6));
}

#[test]
fn a_pkg_head_with_no_indexed_pkg_is_an_honest_miss() {
    // the use names a pkg the chain does not index: the head click
    // answers empty — never a wrong jump
    let src = "use ghost::{ Thing };\nentry fn main() -> nil {\n}\n";
    let d = doc(src);
    assert!(def(&d, at(&d.src, "use ghost::", 1) + 4, &[]).is_empty());
}

// ---- the own-package lanes (the doc context keys them) ----

/// `app`'s root module: the doc IS `mod.rut` (origin stamped like
/// the faces do), declaring the `mod helpers;` edge
fn app_root() -> Doc {
    doc_at(
        "file:///ws/app/mod.rut",
        "mod helpers;\nentry fn main() -> nil {\n}\n",
    )
}

/// the mounted `helpers` child, stamped the way the faces' mount
/// lays it into the chain
fn app_helpers() -> DefIndex {
    rut_lsp::deps::index_dep_mod(
        "app",
        "file:///ws/app/helpers/mod.rut",
        "helpers",
        "pub struct Slot {\n    n: i32;\n}\n",
        Mode::Impl,
        None,
        &[],
    )
}

/// the doc context for the app root
fn root_dm() -> DocMods {
    DocMods {
        mod_path: Some(String::new()),
        pkg: Some("app".to_string()),
    }
}

#[test]
fn an_edge_click_opens_the_child_file_top() {
    let d = app_root();
    let extra = [app_helpers()];
    let hits = def_as(&d, at(&d.src, "mod helpers;", 1) + 4, &extra, &root_dm());
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///ws/app/helpers/mod.rut");
    assert_eq!(hits[0].range.start.line, 0, "the file top");
    assert_eq!(hits[0].range.start.character, 0);
}

#[test]
fn an_edge_click_on_an_unstamped_doc_is_a_miss() {
    // no doc context — the own lanes stay off (today's answer)
    let d = app_root();
    let extra = [app_helpers()];
    assert!(def(&d, at(&d.src, "mod helpers;", 1) + 4, &extra).is_empty());
}

#[test]
fn an_edge_click_on_an_unmounted_child_is_a_miss() {
    // the doc is stamped but the chain carries no index at the
    // child's mod path — an empty jump, never a wrong one
    let d = app_root();
    assert!(def_as(&d, at(&d.src, "mod helpers;", 1) + 4, &[], &root_dm()).is_empty());
}

#[test]
fn a_nested_edge_click_walks_from_the_doc_s_own_path() {
    // the doc IS `helpers` (mounted file-form): its `mod deep;`
    // edge resolves `helpers/deep`, mounted as `deep.rut`
    let d = doc_at(
        "file:///ws/app/helpers.rut",
        "mod deep;\npub struct Slot {\n    n: i32;\n}\n",
    );
    let deep = rut_lsp::deps::index_dep_mod(
        "app",
        "file:///ws/app/helpers/deep.rut",
        "helpers/deep",
        "pub fn dig() -> i32 { return 1; }\n",
        Mode::Impl,
        None,
        &[],
    );
    let dm = DocMods {
        mod_path: Some("helpers".into()),
        pkg: Some("app".into()),
    };
    let extra = [deep];
    let hits = def_as(&d, at(&d.src, "mod deep;", 1) + 4, &extra, &dm);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///ws/app/helpers/deep.rut");
    assert_eq!(hits[0].range.start.line, 0);
    assert_eq!(hits[0].range.start.character, 0);
}

#[test]
fn an_edge_click_never_leaks_across_packages() {
    // another pkg's index stamped at the SAME mod path must not
    // answer the app root's edge click (the (pkg, path) pair keys)
    let d = app_root();
    let other = rut_lsp::deps::index_dep_mod(
        "other",
        "file:///ws/other/helpers/mod.rut",
        "helpers",
        "pub struct Slot {\n    n: i32;\n}\n",
        Mode::Impl,
        None,
        &[],
    );
    let extra = [other];
    assert!(def_as(&d, at(&d.src, "mod helpers;", 1) + 4, &extra, &root_dm()).is_empty());
}

#[test]
fn an_own_pkg_use_path_resolves_intra_package() {
    // `use app::helpers::{ Slot };` in the app root — the pkg-name
    // head opens the root module (the doc itself) at its top, the
    // segment rides the mounting edge, the leaf lands in the child
    let d = doc_at(
        "file:///ws/app/mod.rut",
        "mod helpers;\nuse app::helpers::{ Slot };\nentry fn main() -> nil {\n}\n",
    );
    let root_twin = rut_lsp::deps::index_dep_mod(
        "app",
        "file:///ws/app/on-disk/mod.rut",
        "",
        "mod helpers;\n",
        Mode::Impl,
        None,
        &[],
    );
    let extra = [root_twin, app_helpers()];
    let dm = root_dm();
    // the pkg-name head: the doc's own rows ARE the root — its own
    // file top (never the lagging on-disk twin)
    let hits = def_as(&d, at(&d.src, "use app::", 1) + 4, &extra, &dm);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///ws/app/mod.rut");
    assert_eq!(hits[0].range.start.line, 0);
    assert_eq!(hits[0].range.start.character, 0);
    // the `helpers` segment: the mounting edge in the root (the
    // doc's own rows carry it)
    let hits = def_as(&d, at(&d.src, "app::helpers", 1) + 6, &extra, &dm);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///ws/app/mod.rut");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "mod helpers;", 1) + 4;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 7));
    // the leaf: the child's declaring ident
    let hits = def_as(&d, at(&d.src, "{ Slot }", 1) + 2, &extra, &dm);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///ws/app/helpers/mod.rut");
    let child = normalize("pub struct Slot {\n    n: i32;\n}\n");
    let cix = LineIndex::new(&child);
    let want = at(&child, "struct Slot", 1) + 7;
    assert_eq!(span_of(&child, &cix, &hits[0]), (want, want + 4));
}

#[test]
fn an_own_mod_edge_use_head_resolves_through_the_ancestor_walk() {
    // `use helpers::deep::{ Gem };` from the app root — the bare
    // head is the root's own mod edge: the head opens the child
    // file at its top, the `deep` segment its mounting edge, the
    // leaf the walked path's decl
    let d = doc_at(
        "file:///ws/app/mod.rut",
        "use helpers::deep::{ Gem };\nmod helpers;\nentry fn main() -> nil {\n}\n",
    );
    let helpers = rut_lsp::deps::index_dep_mod(
        "app",
        "file:///ws/app/helpers/mod.rut",
        "helpers",
        "mod deep;\npub struct Slot {\n    n: i32;\n}\n",
        Mode::Impl,
        None,
        &[],
    );
    let deep = rut_lsp::deps::index_dep_mod(
        "app",
        "file:///ws/app/helpers/deep.rut",
        "helpers/deep",
        "pub struct Gem {\n    g: i32;\n}\n",
        Mode::Impl,
        None,
        &[],
    );
    let extra = [helpers, deep];
    let dm = root_dm();
    // the head: the child file's top (the mod-edge target law)
    let hits = def_as(&d, at(&d.src, "use helpers::", 1) + 4, &extra, &dm);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///ws/app/helpers/mod.rut");
    assert_eq!(hits[0].range.start.line, 0);
    assert_eq!(hits[0].range.start.character, 0);
    // the `deep` segment: the `mod deep;` edge in the helpers module
    let hits = def_as(&d, at(&d.src, "helpers::deep", 1) + 10, &extra, &dm);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///ws/app/helpers/mod.rut");
    let helpers_src = normalize("mod deep;\npub struct Slot {\n    n: i32;\n}\n");
    let hix = LineIndex::new(&helpers_src);
    let want = at(&helpers_src, "mod deep;", 1) + 4;
    assert_eq!(span_of(&helpers_src, &hix, &hits[0]), (want, want + 4));
    // the leaf: the walked path's decl
    let hits = def_as(&d, at(&d.src, "{ Gem }", 1) + 2, &extra, &dm);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "file:///ws/app/helpers/deep.rut");
    let deep_src = normalize("pub struct Gem {\n    g: i32;\n}\n");
    let dix = LineIndex::new(&deep_src);
    let want = at(&deep_src, "struct Gem", 1) + 7;
    assert_eq!(span_of(&deep_src, &dix, &hits[0]), (want, want + 3));
}

// ---- the module matcher (the explicit name, then the fallback) ----

#[test]
fn the_explicit_module_name_beats_the_path_derivation() {
    // a dep index named `pouch` whose paths spell something else:
    // the explicit module is the WHOLE match
    let src = "class Widget {\n    id: i32;\n}\n";
    let s = normalize(src);
    let (toks, _) = lex(&s);
    let (ast, _) = parse(&s, Mode::Impl);
    let mut idx = rut_lsp::hover::index(&s, &ast, &toks);
    idx.origin = "file:///ws/gadgets/lib.rut".to_string();
    idx.src_path = Some("somewhere/else/lib.rut".to_string());
    idx.module = Some("pouch".to_string());
    assert!(matches_pkg(&idx, "pouch"));
    assert!(
        !matches_pkg(&idx, "gadgets"),
        "the path stem must NOT match once the module is named"
    );
    assert!(!matches_pkg(&idx, "else"));
}

#[test]
fn the_unnamed_fallback_stays_verbatim() {
    // module None → today's derivation, byte-for-byte: the
    // src_path segment, then the origin's file stem
    let src = "class Widget {\n    id: i32;\n}\n";
    let s = normalize(src);
    let (toks, _) = lex(&s);
    let (ast, _) = parse(&s, Mode::Impl);
    let mut idx = rut_lsp::hover::index(&s, &ast, &toks);
    idx.origin = "file:///ws/gadgets/lib.rut".to_string();
    assert!(matches_pkg(&idx, "gadgets"), "the file stem still resolves");
    idx.src_path = Some("rut/pouch/mod.rut".to_string());
    assert!(
        matches_pkg(&idx, "pouch"),
        "the path segment still resolves"
    );
    assert!(
        !matches_pkg(&idx, "pouch.rut"),
        "the segment law strips `.rut` before comparing"
    );
    assert!(
        !matches_pkg(&idx, "gadgets"),
        "src_path replaces origin as the hay"
    );
    // the std surface's exact-origin lane
    idx.origin = "core".to_string();
    idx.src_path = None;
    assert!(matches_pkg(&idx, "core"));
}

// ---- layer 4: the stdlib surface ----

#[test]
fn std_use_jumps_to_the_true_source_path() {
    let src = "use pouch::{ Vec };\nfn main() -> nil {\n    let v = Vec::new();\n}\n";
    let d = doc(src);
    let extra = rut_lsp::std_surface::indexes();
    let hits = def(&d, at(&d.src, "= Vec::new();", 1) + 2, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    // the TRUE rut/... path, not a label
    assert_eq!(hits[0].uri, "rut/pouch/src/mod.rut");
    // the reported range is the declaring `Vec` ident in the REAL
    // source file (normalize the embedded const like the index did)
    let real = normalize(rut_lsp::std_surface::POUCH);
    let ix = LineIndex::new(&real);
    let want = at(&real, "class Vec", 1) + 6;
    assert_eq!(span_of(&real, &ix, &hits[0]), (want, want + 3));
    // the use-statement name resolves to the same target
    let hits = def(&d, at(&d.src, "use pouch::{ Vec }", 1) + 13, &extra);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].uri, "rut/pouch/src/mod.rut");
    assert_eq!(span_of(&real, &ix, &hits[0]), (want, want + 3));
}

// ---- typeDefinition ----

#[test]
fn type_definition_from_a_binding() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "fn go() -> f64 {",
        "    let c = Circle::new(1.0);",
        "    return c.r;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    let hits = typedef(&d, at(&d.src, "return c.r;", 1) + 7, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "class Circle", 1) + 6;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
}

#[test]
fn type_definition_from_a_nullable_field_member() {
    let src = [
        "class Circle {",
        "    r: f64;",
        "}",
        "struct Slot {",
        "    item: ?Circle;",
        "}",
        "fn go(s: Slot) -> f64 {",
        "    return s.item.r;",
        "}",
        "",
    ]
    .join("\n");
    let d = doc(&src);
    // `s.item` — the `?Circle` field's type head is `Circle`
    let hits = typedef(&d, at(&d.src, "s.item.r;", 1) + 2, &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "class Circle", 1) + 6;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
}

#[test]
fn type_definition_on_a_type_token_is_its_own_decl() {
    let src = "class Circle {\n    r: f64;\n}\nfn go() -> f64 {\n    let c = Circle::new(1.0);\n    return c.r;\n}\n";
    let d = doc(src);
    let hits = typedef(&d, at(&d.src, "Circle::new(1.0);", 1), &[]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let ix = LineIndex::new(&d.src);
    let want = at(&d.src, "class Circle", 1) + 6;
    assert_eq!(span_of(&d.src, &ix, &hits[0]), (want, want + 6));
}

#[test]
fn misses_stay_misses() {
    let src = "entry fn main() -> nil {\n}\n";
    let d = doc(src);
    // keywords, unknown names, and decl-less primitives are empty
    assert!(def(&d, 0, &[]).is_empty()); // `fn` keyword
    let src2 = "entry fn main() -> nil {\n    let x = i32::max(1);\n}\n";
    let d2 = doc(src2);
    assert!(
        def(&d2, at(&d2.src, "i32::max", 1), &[]).is_empty(),
        "i32 has no decl to jump to"
    );
    let src3 = "entry fn main() -> nil {\n    let x = mystery();\n}\n";
    let d3 = doc(src3);
    assert!(def(&d3, at(&d3.src, "mystery", 1), &[]).is_empty());
}
