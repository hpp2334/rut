//! The analysis envelope's gate — the sync lane's `{diags, tokens,
//! symbols}` answer, driven through the public `Lsp` the way the
//! extension calls it. Pins: the parse lane's diags (severity, source,
//! range), the delta token stream's decode, the filename convention
//! (a `.d.rut` uri parses Decl), the check lane's merge (the stamped
//! doc compiles with its dep closure; an unstamped one stays
//! parse-only; a parse error is never doubled), and the span-0
//! refusal's silence.

use serde_json::Value;

mod common;

use common::*;

/// the parsed diags' (message-contains, line) rows
fn diag_rows(a: &Value) -> Vec<(String, i64)> {
    a["diags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["message"].as_str().unwrap_or_default().to_string(),
                d["range"]["start"]["line"].as_i64().unwrap_or(-1),
            )
        })
        .collect()
}

#[test]
fn analyze_end_to_end() {
    let mut s = S::boot();
    let uri = "file:///t/x.rut";
    let a = s.analyze(uri, "entry fn main() -> nil {\n    let x = ;\n}\n");
    let diags = a["diags"].as_array().unwrap();
    assert!(!diags.is_empty(), "parse error must surface: {a}");
    assert_eq!(diags[0]["severity"], 1, "ERROR severity: {}", diags[0]);
    assert_eq!(diags[0]["source"], "rut", "{}", diags[0]);

    let clean = s.analyze(uri, "enum Color { Red }\n");
    assert!(clean["diags"].as_array().unwrap().is_empty(), "{clean}");
    let syms = clean["symbols"].as_array().unwrap();
    assert_eq!(syms.len(), 1, "{clean}");
    assert_eq!(syms[0]["name"], "Color");
    assert_eq!(syms[0]["kind"], sk::ENUM, "{}", syms[0]);
}

#[test]
fn the_delta_stream_decodes_onto_real_text() {
    // every token's absolute (line, char, len) must land on the exact
    // text it classified — the decode half of the wire contract
    let mut s = S::boot();
    let src = "enum Color { Red }\nfn f(x: i32) -> nil {}\n";
    let uri = "file:///t/delta.rut";
    let a = s.analyze(uri, src);
    let toks = decode_tokens(src, &a);
    assert!(!toks.is_empty());
    for t in &toks {
        assert!(!t.text.is_empty(), "empty token at {}:{}", t.line, t.ch);
        assert!(
            t.text.chars().all(|c| !c.is_whitespace()),
            "token must not span whitespace: {t:?}"
        );
    }
    // spot the decode against known classes: the enum decl + member
    assert_eq!(types_of(&toks, "Color"), vec![t::ENUM], "{toks:?}");
    assert_eq!(types_of(&toks, "Red"), vec![t::ENUM_MEMBER], "{toks:?}");
    assert_eq!(types_of(&toks, "f"), vec![t::FUNCTION], "{toks:?}");
}

#[test]
fn pub_on_a_use_classifies_as_the_keyword() {
    // `pub use` — the re-export prefix: `pub` is a reserved word, so
    // the token layer owns it (Keyword — the same class it carries on
    // every item), and the use's own names keep the cap-law classes
    let mut s = S::boot();
    let src = "pub use util::{ helper };\n";
    let a = s.analyze("file:///t/pubuse.rut", src);
    let toks = decode_tokens(src, &a);
    assert_eq!(types_of(&toks, "pub"), vec![t::KEYWORD], "{toks:?}");
    assert_eq!(types_of(&toks, "use"), vec![t::KEYWORD], "{toks:?}");
    // the cap law rides unchanged beneath the modifier
    assert_eq!(types_of(&toks, "util"), vec![t::NAMESPACE], "{toks:?}");
    assert_eq!(types_of(&toks, "helper"), vec![t::FUNCTION], "{toks:?}");
}

#[test]
fn decl_mode_for_dot_d_rut() {
    // the filename convention, THE authority: a `.d.rut` uri parses in
    // declaration mode (method bodies forbidden — the surface's own
    // grammar), every other uri full Impl. The envelope only sees the
    // uri: the SAME bodiless-method class is clean at `.d.rut` and
    // refused at `.rut`, and the SAME body form is refused at `.d.rut`.
    let mut s = S::boot();
    let bodiless = "class Box {\n    fn make();\n}\n";
    let a = s.analyze("file:///t/plugin.d.rut", bodiless);
    assert!(
        a["diags"].as_array().unwrap().is_empty(),
        "the surface form parses clean in Decl mode: {a}"
    );
    let b = s.analyze("file:///t/plugin.rut", bodiless);
    assert!(
        !b["diags"].as_array().unwrap().is_empty(),
        "the same surface form is refused in Impl mode: {b}"
    );
    // the body form refuses in Decl mode (the body brace is a parse
    // error there), and the same body parses clean at an Impl uri
    let body = "class Box {\n    fn make() -> i32 { return 0; }\n}\n";
    let a = s.analyze("file:///t/mod.d.rut", body);
    assert!(
        !a["diags"].as_array().unwrap().is_empty(),
        "Decl mode forbids bodies: {a}"
    );
    let b = s.analyze("file:///t/net/mod.d.rut", body);
    assert!(
        !b["diags"].as_array().unwrap().is_empty(),
        "the host default root routes Decl too: {b}"
    );
}

#[test]
fn no_package_context_is_parse_only_and_a_parse_error_is_never_doubled() {
    // the merge law through one door: the SAME broken text answers the
    // parse set unstamped (no package graph assembles), and the check
    // lane adds nothing when it IS stamped (the re-parse's duplicates
    // collapse — true duplicates only)
    let mut s = S::boot();
    let bad = "fn main() -> nil {\n    let x = ;\n}\n";
    let plain = s.analyze("file:///ws/plain.rut", bad);
    let n = plain["diags"].as_array().unwrap().len();
    assert!(n > 0, "the parse error surfaces: {plain}");

    // stamped (the uri's stamp names its package — opening adopts
    // what the index lane laid down): the checker runs,
    // the merge collapses the re-parsed duplicates onto the parse set
    let app_uri = "file:///ws/dbl/app/mod.rut";
    s.index_mod_at(app_uri, "dbl", "", bad);
    let merged = s.analyze(app_uri, bad);
    assert_eq!(
        merged["diags"].as_array().unwrap().len(),
        n,
        "the check lane adds nothing to a parse-error doc: {merged}"
    );
}

#[test]
fn check_enabled_doc_maps_lir_diags_to_ranges() {
    // the diagnosis pair over the stamped chain, with the exact
    // editor ranges: an assignment node ends at its VALUE (the `;` is
    // the statement's terminator), and the missing-field flag sits at
    // the literal's line
    let mut s = S::boot();
    let dep = "pub struct Task {\n    id: u32;\n}\n";
    s.index_mod_at("file:///ws/rng/kit/mod.rut", "kit", "", dep);
    let uri = "file:///ws/rng/app/mod.rut";
    let doc =
        "use kit::{ Task };\nfn main() -> nil {\n    let checked: str = \" \";\n    checked = true;\n}\n";
    s.index_mod_at(uri, "app", "", doc);
    let a = s.analyze(uri, doc);
    let diags = a["diags"].as_array().unwrap();
    let mismatch = diags
        .iter()
        .find(|d| {
            d["message"]
                .as_str()
                .unwrap_or("")
                .contains("assignment type mismatch")
        })
        .unwrap_or_else(|| panic!("no assignment-mismatch diag: {a}"));
    assert_eq!(mismatch["source"], "rut", "{mismatch}");
    // the range covers the offending assignment — start at `checked`,
    // end at the value (not the `;`)
    let want_lo = byte_of(doc, "checked = true", 1);
    let want_hi = want_lo + "checked = true".len() as u32;
    assert_eq!(
        span_of(doc, mismatch),
        (want_lo as usize, want_hi as usize),
        "{mismatch}"
    );

    // half two: the missing record field flags at the literal
    let dep = "pub struct Task {\n    id: u32;\n    title: str;\n}\n";
    s.index_mod_at("file:///ws/rng/taskkit/mod.rut", "taskkit", "", dep);
    let doc = "use taskkit::{ Task };\nfn main() -> nil {\n    let t = Task { title: \"x\" };\n}\n";
    let uri = "file:///ws/rng/app2/mod.rut";
    s.index_mod_at(uri, "app2", "", doc);
    let a = s.analyze(uri, doc);
    let missing = a["diags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["message"]
                .as_str()
                .unwrap_or("")
                .contains("`id` is missing")
        })
        .unwrap_or_else(|| panic!("no missing-field diag: {a}"));
    assert_eq!(
        missing["range"]["start"]["line"], 2,
        "the Task literal's line: {missing}"
    );
}

#[test]
fn unattributable_spans_skip() {
    // a cyclic use between two dep groups is the graph walk's span-0
    // refusal — it must not light up anywhere: the merge degrades to
    // the parse set (clean here), never a misplaced squiggle
    let mut s = S::boot();
    s.index_mod_at(
        "file:///ws/cyc/aa/mod.rut",
        "aa",
        "",
        "use bb::{ B };\npub struct A {\n    x: i32;\n}\n",
    );
    s.index_mod_at(
        "file:///ws/cyc/bb/mod.rut",
        "bb",
        "",
        "use aa::{ A };\npub struct B {\n    y: i32;\n}\n",
    );
    let uri = "file:///ws/cyc/app/mod.rut";
    let doc = "use aa::{ A };\nfn main() -> nil {\n    let a = A { x: 1 };\n    let _x = a.x;\n}\n";
    s.index_mod_at(uri, "app", "", doc);
    let a = s.analyze(uri, doc);
    assert!(
        a["diags"].as_array().unwrap().is_empty(),
        "span-0 refusals publish nothing: {a} — {:?}",
        diag_rows(&a)
    );
}
