use ls_types::*;
use rut_lexer::lexer::canonical;
use rut_lexer::span::Span;
use rut_lsp_wasm::analysis::analyze_unstamped;
use rut_lsp_wasm::analysis::*;

/// the modeless analyze died with the mode leak — these tests pin
/// grammar behavior through the unstamped facade entry, the uri
/// spelling the mode (the filename convention)
const IMPL_URI: &str = "file:///t/x.rut";
use rut_ast::parser::Mode;
use rut_lsp_wasm::check;
use rut_lsp_wasm::check::{FileSrc, PkgFiles};
use rut_lsp_wasm::LineIndex;
use std::collections::BTreeMap;

#[test]
fn analyze_end_to_end() {
    let a = analyze_unstamped(
        IMPL_URI,
        "entry fn main() -> nil {\n    let x = ;\n}\n".to_string(),
    );
    assert!(!a.diags.is_empty(), "parse error must surface");
    assert!(a.diags[0].severity == Some(DiagnosticSeverity::ERROR));
    assert_eq!(a.diags[0].source.as_deref(), Some("rut"));

    let clean = analyze_unstamped(IMPL_URI, "enum Color { Red }\n".to_string());
    assert!(clean.diags.is_empty());
    assert_eq!(clean.symbols.len(), 1);
    assert_eq!(clean.symbols[0].name, "Color");
}

#[test]
fn encode_is_delta_correct() {
    let src = "enum Color { Red }\nfn f(x: i32) -> nil {}\n";
    let a = analyze_unstamped(IMPL_URI, src.to_string());
    // decode back: every token's absolute (line, char, len) must land
    // on the exact text it classified
    let mut line = 0u32;
    let mut ch = 0u32;
    for t in &a.tokens {
        line += t.delta_line;
        ch = if t.delta_line == 0 {
            ch + t.delta_start
        } else {
            t.delta_start
        };
        let byte_lo = {
            let ix = LineIndex::new(&canonical(src));
            ix.byte(&canonical(src), line, ch)
        };
        let text = &canonical(src)[byte_lo as usize..(byte_lo + t.length) as usize];
        assert!(!text.is_empty());
        assert!(
            text.chars().all(|c| !c.is_whitespace()),
            "token must not span whitespace: {text:?}"
        );
    }
}

#[test]
fn pub_on_a_use_classifies_as_the_keyword() {
    // `pub use` — the re-export prefix: `pub` is a reserved word, so
    // the token layer owns it (Keyword, index 0 — the same class it
    // carries on every item), and the use's own names keep the cap-law
    // classes (`util` a namespace head, `helper` a function leaf)
    let src = "pub use util::{ helper };\n";
    let a = analyze_unstamped(IMPL_URI, src.to_string());
    let s = canonical(src).into_owned();
    let mut line = 0u32;
    let mut ch = 0u32;
    let mut seen: Vec<(String, u32)> = Vec::new();
    for t in &a.tokens {
        line += t.delta_line;
        ch = if t.delta_line == 0 {
            ch + t.delta_start
        } else {
            t.delta_start
        };
        let ix = LineIndex::new(&s);
        let lo = ix.byte(&s, line, ch) as usize;
        seen.push((s[lo..lo + t.length as usize].to_string(), t.token_type));
    }
    let ty_of = |w: &str| {
        seen.iter()
            .find(|(text, _)| text == w)
            .map(|(_, ty)| *ty)
            .unwrap_or_else(|| panic!("`{w}` not classified: {seen:?}"))
    };
    assert_eq!(ty_of("pub"), 0, "the Keyword index: {seen:?}");
    assert_eq!(ty_of("use"), 0, "{seen:?}");
    // the cap law rides unchanged beneath the modifier
    assert_eq!(
        ty_of("util"),
        rut_semantic::TokenType::Namespace.index(),
        "{seen:?}"
    );
    assert_eq!(
        ty_of("helper"),
        rut_semantic::TokenType::Function.index(),
        "{seen:?}"
    );
}

#[test]
fn decl_mode_for_dot_d_rut() {
    assert_eq!(mode_of("x/y/plugin.d.rut"), Mode::Decl);
    assert_eq!(mode_of("x/y/main.rut"), Mode::Impl);
    // the host default root routes Decl like every `.d.rut` surface
    assert_eq!(mode_of("mod.d.rut"), Mode::Decl);
    assert_eq!(mode_of("net/mod.d.rut"), Mode::Decl);
}

// ---- the semantic lane (B1+B2): merge + check ----

/// one file of a hand-assembled group (normalized like every lane)
fn file(uri: &str, text: &str) -> FileSrc {
    FileSrc {
        uri: uri.to_string(),
        text: rut_lexer::lexer::canonical(text).into_owned(),
        mode: Mode::Impl,
        vis: rut_ast::Vis::Pub,
    }
}

fn group(module: &str, root: FileSrc) -> PkgFiles {
    let mut files = BTreeMap::new();
    files.insert(String::new(), root);
    PkgFiles {
        module: module.to_string(),
        files,
    }
}

#[test]
fn merge_dedupes_true_duplicates_only() {
    let d1 = Diagnostic {
        range: range_of(&LineIndex::new("ab"), "ab", Span::new(0, 1)),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("rut".into()),
        message: "one".into(),
        ..Default::default()
    };
    let mut d2 = d1.clone();
    d2.message = "two".into();
    let mut d3 = d1.clone();
    d3.range = range_of(&LineIndex::new("ab"), "ab", Span::new(1, 2));
    let merged = merge_diags(vec![d1.clone()], vec![d1, d2, d3]);
    // the same range+message collapsed; the distinct message and the
    // distinct site (the B3 multi-site law) both stay
    assert_eq!(merged.len(), 3, "{merged:?}");
    assert_eq!(merged[0].message, "one");
    assert_eq!(merged[1].message, "two");
    // d3: same message as d1, a range of its own — never collapsed
    assert_eq!(merged[2].message, "one");
    assert_ne!(merged[2].range, merged[0].range);
}

#[test]
fn no_package_context_is_parse_only() {
    let src = "entry fn main() -> nil {\n    let x = ;\n}\n";
    let parse = analyze_unstamped(IMPL_URI, src.to_string()).diags;
    assert!(!parse.is_empty());
    let merged = check::merged_doc_diags(
        "file:///ws/p.rut",
        src,
        None, // no pkg context — today's answer, byte for byte
        None,
        Vec::new(),
        parse.clone(),
    );
    assert_eq!(merged.len(), parse.len());
}

#[test]
fn check_enabled_doc_maps_lir_diags_to_ranges() {
    // the diagnosis pair, half one: a bool→str assignment mismatch
    // surfaces through the checker with the range the editor highlights
    let src = "use kit::{ Task };\nfn main() -> nil {\n    let mut checked: str = \" \";\n    checked = true;\n}\n";
    let uri = "file:///ws/app/mod.rut";
    let parse = analyze_unstamped(IMPL_URI, src.to_string()).diags;
    assert!(parse.is_empty(), "the doc parses clean: {parse:?}");
    let dep = group(
        "kit",
        file(
            "file:///ws/kit/mod.rut",
            "pub struct Task {\n    id: u32;\n}\n",
        ),
    );
    let app = group("app", file(uri, src));
    let merged = check::merged_doc_diags(
        uri,
        src,
        Some("app".into()),
        Some(String::new()),
        vec![dep, app],
        parse,
    );
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert!(
        merged[0].message.contains("assignment type mismatch"),
        "{}",
        merged[0].message
    );
    assert_eq!(merged[0].source.as_deref(), Some("rut"));
    // the range covers the offending assignment (line 3) — the v2
    // exact-span law: an assignment node ends at its VALUE (the
    // statement's own `;` is the terminator, never the expression's)
    let norm = rut_lexer::lexer::canonical(src).into_owned();
    let lo = norm.find("checked = true").expect("the assignment") as u32;
    let want = range_of(
        &LineIndex::new(&norm),
        &norm,
        Span::new(lo, lo + "checked = true".len() as u32),
    );
    assert_eq!(merged[0].range, want, "{:?}", merged[0].range);

    // half two: the missing record field flags at the literal
    let src = "use taskkit::{ Task };\nfn main() -> nil {\n    let t = Task { title: \"x\" };\n}\n";
    let dep = group(
        "taskkit",
        file(
            "file:///ws/taskkit/mod.rut",
            "pub struct Task {\n    id: u32;\n    title: str;\n}\n",
        ),
    );
    let app = group("app", file(uri, src));
    let merged = check::merged_doc_diags(
        uri,
        src,
        Some("app".into()),
        Some(String::new()),
        vec![dep, app],
        Vec::new(),
    );
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert!(
        merged[0].message.contains("`id` is missing"),
        "{}",
        merged[0].message
    );
    assert_eq!(merged[0].range.start.line, 2, "the Task literal's line");

    // a parse error is NOT doubled: the check lane re-parses the doc,
    // and the merge collapses the true duplicate. (The v2 recovery
    // ladder emits TWO rows for this degenerate body — the init hole
    // and the missing `;` — so the law is pinned as "the check lane
    // adds nothing", not as a v1 row count.)
    let bad = "fn main() -> nil {\n    let x = ;\n}\n";
    let parse = analyze_unstamped(IMPL_URI, bad.to_string()).diags;
    assert!(!parse.is_empty());
    let n_parse = parse.len();
    let app = group("app", file(uri, bad));
    let merged = check::merged_doc_diags(
        uri,
        bad,
        Some("app".into()),
        Some(String::new()),
        vec![app],
        parse,
    );
    assert_eq!(
        merged.len(),
        n_parse,
        "the check lane adds nothing to a parse-error doc: {merged:?}"
    );
}

#[test]
fn unattributable_spans_skip() {
    // a cyclic use between two dep groups is the graph walk's
    // span-0 refusal (`file: None`) — it must not light up anywhere:
    // the merge degrades to the parse set (clean here), never a
    // misplaced squiggle
    let doc = "use aa::{ A };\nfn main() -> nil {\n    let a = A { x: 1 };\n    let _x = a.x;\n}\n";
    let uri = "file:///ws/app3/mod.rut";
    let aa = group(
        "aa",
        file(
            "file:///ws/aa/mod.rut",
            "use bb::{ B };\npub struct A {\n    x: i32;\n}\n",
        ),
    );
    let bb = group(
        "bb",
        file(
            "file:///ws/bb/mod.rut",
            "use aa::{ A };\npub struct B {\n    y: i32;\n}\n",
        ),
    );
    let app = group("app3", file(uri, doc));
    let parse = analyze_unstamped(IMPL_URI, doc.to_string()).diags;
    assert!(parse.is_empty(), "{parse:?}");
    let merged = check::merged_doc_diags(
        uri,
        doc,
        Some("app3".into()),
        Some(String::new()),
        vec![aa, bb, app],
        parse,
    );
    assert!(
        merged.is_empty(),
        "span-0 refusals publish nothing: {merged:?}"
    );
}
