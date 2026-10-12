//! Classifier + symbol tests.

use rut_lexer::span::Span;
use rut_lexer::token::Token;
use rut_parser::{parse, Mode};

use super::recover::push_name;
use super::*;

fn classify_src(src: &str) -> Vec<(Span, TokenType)> {
    let (toks, _) = rut_lexer::lexer::lex(src);
    let (ast, _) = parse(src, Mode::Impl);
    classify(&toks, &ast)
}

fn span_text(src: &str, sp: Span) -> &str {
    &src[sp.lo as usize..sp.hi as usize]
}

fn find(src: &str, spans: &[(Span, TokenType)], text: &str) -> Vec<TokenType> {
    spans
        .iter()
        .filter(|(sp, _)| span_text(src, *sp) == text)
        .map(|(_, ty)| *ty)
        .collect()
}

#[test]
fn output_is_sorted_and_disjoint() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../demo/src/examples/literals/mod.rut");
    let src = std::fs::read_to_string(&p).unwrap();
    let spans = classify_src(&src);
    let mut prev_hi = 0;
    for (sp, _) in &spans {
        assert!(sp.lo < sp.hi, "empty span at {}", sp.lo);
        assert!(sp.lo >= prev_hi, "overlap at {}", sp.lo);
        assert!(sp.hi as usize <= src.len(), "out of file");
        prev_hi = sp.hi;
    }
}

#[test]
fn decls_and_keywords() {
    let src = "enum Color { Red, Green }\nfn area(r: f32) -> f32 { return r * 2.0f32; }\n";
    let spans = classify_src(src);
    assert_eq!(find(src, &spans, "enum"), vec![TokenType::Keyword]);
    assert_eq!(find(src, &spans, "Color"), vec![TokenType::Enum]);
    assert_eq!(find(src, &spans, "Red"), vec![TokenType::EnumMember]);
    assert_eq!(find(src, &spans, "Green"), vec![TokenType::EnumMember]);
    assert_eq!(find(src, &spans, "area"), vec![TokenType::Function]);
    // the decl is a parameter; body uses are lowercase single-seg paths
    assert_eq!(find(src, &spans, "r"), vec![TokenType::Parameter, TokenType::Variable]);
    assert_eq!(find(src, &spans, "f32"), vec![TokenType::Type, TokenType::Type]);
    assert_eq!(find(src, &spans, "2.0f32"), vec![TokenType::Number]);
    assert_eq!(find(src, &spans, "*"), vec![TokenType::Operator]);
}

#[test]
fn members_fields_and_locals() {
    let src = "class Circle {\n    pub r: f32;\n    fn scale(mut self, k: f32) -> Self { self.r = self.r * k; }\n}\n";
    let spans = classify_src(src);
    assert_eq!(find(src, &spans, "Circle"), vec![TokenType::Class]);
    // the declared field + both `self.r` path tails
    assert_eq!(find(src, &spans, "r"), vec![TokenType::Property, TokenType::Property, TokenType::Property]);
    assert_eq!(find(src, &spans, "scale"), vec![TokenType::Method]);
    assert_eq!(find(src, &spans, "k"), vec![TokenType::Parameter, TokenType::Variable]);
    // `self` stays a keyword everywhere (param + both path prefixes)
    assert_eq!(find(src, &spans, "self"), vec![TokenType::Keyword, TokenType::Keyword, TokenType::Keyword]);
    assert_eq!(find(src, &spans, "Self"), vec![TokenType::Type]);
}

#[test]
fn f_string_tiles_and_lexes_holes() {
    let src = "fn f() -> nil { log.info(f\"{color_name(Color::Red)} area={c.area()}\"); }";
    let spans = classify_src(src);
    // the whole literal tiles with strings + hole tokens, no overlap
    let lit = src.find("f\"").unwrap() as u32;
    let lit_end = src.find("\");").unwrap() as u32 + 1; // past the closing quote
    let inside: Vec<_> = spans
        .iter()
        .filter(|(sp, _)| sp.lo >= lit && sp.hi <= lit_end)
        .collect();
    assert!(!inside.is_empty());
    let mut prev_hi = lit;
    for (sp, _) in &inside {
        assert!(sp.lo >= prev_hi, "gap/overlap inside f-string at {}", sp.lo);
        prev_hi = sp.hi;
    }
    assert_eq!(prev_hi, lit_end);
    // hole contents classify: `Color` type, `Red` enumMember, `area` method
    assert_eq!(find(src, &spans, "Color"), vec![TokenType::Type]);
    assert_eq!(find(src, &spans, "Red"), vec![TokenType::EnumMember]);
    assert_eq!(find(src, &spans, "area"), vec![TokenType::Method]);
}

#[test]
fn types_struct_literals_and_patterns() {
    let src = "enum Color { Red, Green }\n\
               struct Point { x: f64; y: f64; }\n\
               fn make(c: Color) -> Point {\n\
                   let p = Point { x: 1, y: 2 };\n\
                   return when (c) { Color::Red -> p, _ -> p, };\n\
               }\n";
    let spans = classify_src(src);
    // decl name, param type, literal type
    assert_eq!(find(src, &spans, "Point"), vec![TokenType::Class, TokenType::Type, TokenType::Type]);
    // field decls + literal labels
    assert_eq!(find(src, &spans, "x"), vec![TokenType::Property, TokenType::Property]);
    // enum decl name, param type, pattern type
    assert_eq!(find(src, &spans, "Color"), vec![TokenType::Enum, TokenType::Type, TokenType::Type]);
    assert_eq!(find(src, &spans, "Red"), vec![TokenType::EnumMember, TokenType::EnumMember]);
    assert_eq!(find(src, &spans, "c"), vec![TokenType::Parameter, TokenType::Variable]);
    assert_eq!(find(src, &spans, "p"), vec![TokenType::Variable, TokenType::Variable, TokenType::Variable]);
}

#[test]
fn recovery_miss_skips_instead_of_guessing() {
    let toks: Vec<Token> = Vec::new();
    let mut out = Vec::new();
    push_name(&toks, Span::new(0, 10), "missing", TokenType::Function, &mut out, false);
    assert!(out.is_empty());
}

#[test]
fn ast_wins_over_token_layer_at_equal_start() {
    // a field named `str` would be token-classified `type`; the AST
    // pass must replace it with `property`. (`nil` cannot be used — it
    // is a reserved word, unlike the contextual primitive names.)
    let src = "struct T { str: i32; }\n";
    let spans = classify_src(src);
    assert_eq!(find(src, &spans, "str"), vec![TokenType::Property]);
}

#[test]
fn cast_types_keep_their_type_color() {
    // `x as f64` is the conversion family — the
    // primitive after `as` stays a type, not a variable
    let src = "fn f(p: Point) -> f64 { return p.x as f64; }\n";
    let spans = classify_src(src);
    assert_eq!(find(src, &spans, "f64"), vec![TokenType::Type, TokenType::Type]);
    assert_eq!(find(src, &spans, "p"), vec![TokenType::Parameter, TokenType::Variable]);
}

#[test]
fn bare_calls_color_their_callee_as_function() {
    // `length(pt)` / `newCanvas()` — the callee gets the function
    // color, overriding the single-seg path's variable class
    let src = "fn go() -> nil { let p = newCanvas(); blit_all(length(p), p); }\n";
    let spans = classify_src(src);
    assert_eq!(find(src, &spans, "newCanvas"), vec![TokenType::Function]);
    assert_eq!(find(src, &spans, "blit_all"), vec![TokenType::Function]);
    assert_eq!(find(src, &spans, "length"), vec![TokenType::Function]);
    assert_eq!(find(src, &spans, "p"), vec![TokenType::Variable, TokenType::Variable, TokenType::Variable]);
}

#[test]
fn symbols_outline() {
    let src = "interface Drawable { fn draw(self, g: Canvas) -> nil; }\nimpl Circle { fn draw(self, g: Canvas) -> nil {} }\nenum Color { Red }\nentry fn main() -> nil {}\n";
    let (toks, _) = rut_lexer::lexer::lex(src);
    let (ast, _) = parse(src, Mode::Impl);
    let syms = symbols(&toks, &ast);
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Drawable", "impl Circle", "Color", "main"]);
    assert_eq!(syms[0].kind, SymKind::Interface);
    assert_eq!(syms[0].children.len(), 1);
    assert_eq!(syms[1].kind, SymKind::Module);
    assert_eq!(syms[1].children.len(), 1);
    assert_eq!(syms[2].children.len(), 1);
    assert_eq!(syms[3].kind, SymKind::Function);
    // selectionRange stays inside range
    for s in &syms {
        assert!(s.selection.lo >= s.range.lo && s.selection.hi <= s.range.hi, "{}", s.name);
    }
}

#[test]
fn comment_is_appended_to_the_legend() {
    assert_eq!(ALL.len(), 16);
    assert_eq!(TokenType::Comment.index(), 15);
    // the trailing append moved nothing
    assert_eq!(TokenType::Operator.index(), 14);
    assert_eq!(TokenType::Namespace.index(), 13);
    assert_eq!(TokenType::legend().last().copied(), Some("comment"));
}

#[test]
fn colon2_value_paths_classify_by_the_resolution_law() {
    // `::` spells name resolution: interior segments are heads (a
    // lowercase head is a module → namespace, a capitalized one a
    // type), the leaf is the member (a lowercase fn, a capitalized
    // enum member/static); a lone lowercase seg stays a local read and
    // a property never appears — `.` spells value access
    let src = "mod layout;\nenum Color { Red }\nentry fn main() -> nil {\n    let a = layout::mk();\n    let b = Color::Red;\n    let c = i32::abs(-1);\n    let d = b;\n}\n";
    let spans = classify_src(src);
    // the module head — namespace, not a local variable
    assert_eq!(find(src, &spans, "layout"), vec![TokenType::Namespace]);
    // the member fn through the module — function, not a property
    assert_eq!(find(src, &spans, "mk"), vec![TokenType::Function]);
    // the enum head + its member — type, then enumMember (decl + use)
    assert_eq!(find(src, &spans, "Color"), vec![TokenType::Enum, TokenType::Type]);
    assert_eq!(find(src, &spans, "Red"), vec![TokenType::EnumMember, TokenType::EnumMember]);
    // a primitive head stays a type at an interior position (the
    // token layer's class, confirmed by the path walk)
    assert_eq!(find(src, &spans, "i32"), vec![TokenType::Type]);
    assert_eq!(find(src, &spans, "abs"), vec![TokenType::Function]);
    // the lone lowercase path is still the local read (decl + use)
    assert_eq!(find(src, &spans, "b"), vec![TokenType::Variable, TokenType::Variable]);
    // the law: `::` never colors a property
    assert!(!spans.iter().any(|(_, t)| *t == TokenType::Property), "{spans:?}");
}

#[test]
fn use_lines_classify_by_the_capitalization_law() {
    // the use line colors best-effort, without resolution: a path
    // segment reads as its head (lowercase → namespace, capitalized →
    // type), an imported name as the leaf member (capitalized → type,
    // lowercase → function); `entry` before `fn` is token-owned by text
    let src = "use ink::{ Logger };\nentry fn main() -> nil {}\n";
    let spans = classify_src(src);
    assert_eq!(find(src, &spans, "ink"), vec![TokenType::Namespace]);
    assert_eq!(find(src, &spans, "Logger"), vec![TokenType::Type]);
    assert_eq!(find(src, &spans, "entry"), vec![TokenType::Keyword]);

    // a lowercase import colors a function, its single-seg head a module
    let src = "use mod::{ helper };\n";
    let spans = classify_src(src);
    assert_eq!(find(src, &spans, "mod"), vec![TokenType::Namespace]);
    assert_eq!(find(src, &spans, "helper"), vec![TokenType::Function]);
}

#[test]
fn multi_seg_use_paths_classify_each_lane() {
    // `use pkg::mod::{ name };` — the interior segments are the module
    // heads (namespace), the brace leaf the import (function when
    // lowercase) — mirroring the corpus's `use http_host::net::{ .. };`
    let src = "use http_host::net::{ http_send };\nentry fn main() -> nil {}\n";
    let spans = classify_src(src);
    assert_eq!(find(src, &spans, "http_host"), vec![TokenType::Namespace]);
    assert_eq!(find(src, &spans, "net"), vec![TokenType::Namespace]);
    assert_eq!(find(src, &spans, "http_send"), vec![TokenType::Function]);
}

#[test]
fn line_comments_color_exactly_and_stop_at_newline() {
    let src = "// lead\nlet x = 1; // trail\nlet y = 2;\n";
    let spans = classify_source(src, Mode::Impl);
    assert_eq!(
        find(src, &spans, "// lead"),
        vec![TokenType::Comment],
        "the leading line comment is one comment span"
    );
    assert_eq!(
        find(src, &spans, "// trail"),
        vec![TokenType::Comment],
        "the trailing line comment stops before the newline"
    );
    assert_eq!(spans.iter().filter(|(_, t)| *t == TokenType::Comment).count(), 2);
}

#[test]
fn block_comments_color_and_unterminated_clamps() {
    let src = "let x = 1; /* mid */ let y = 2; /* never closed";
    let spans = classify_source(src, Mode::Impl);
    assert_eq!(find(src, &spans, "/* mid */"), vec![TokenType::Comment]);
    let unclosed: Vec<&str> = spans
        .iter()
        .filter(|(_, t)| *t == TokenType::Comment)
        .filter(|(sp, _)| span_text(src, *sp).starts_with("/* never"))
        .map(|(sp, _)| span_text(src, *sp))
        .collect();
    assert_eq!(
        unclosed,
        vec!["/* never closed"],
        "an unterminated block comment clamps at the gap's end"
    );
}

#[test]
fn comments_do_not_touch_adjacent_strings() {
    let src = "let s = \"a\"/*c*/;\nlet t = \"// not a comment\"; // real\n";
    let spans = classify_source(src, Mode::Impl);
    // the string keeps its class; the gap comment is its own span; the
    // run stays sorted and disjoint
    let mut prev_hi = 0;
    for (sp, _) in &spans {
        assert!(sp.lo >= prev_hi, "overlap at {}", sp.lo);
        prev_hi = sp.hi;
    }
    assert_eq!(find(src, &spans, "\"// not a comment\""), vec![TokenType::String]);
    assert_eq!(find(src, &spans, "/*c*/"), vec![TokenType::Comment]);
    assert_eq!(find(src, &spans, "// real"), vec![TokenType::Comment]);
}

#[test]
fn classify_source_tiles_sorted_and_disjoint() {
    // comments at the very start, between every construct, and at the end
    let src = "// top\n/* banner */\nlet a = 1; // one\nfn f(x: i32) -> i32 { x } /* tail */\n";
    let spans = classify_source(src, Mode::Impl);
    let mut prev_hi = 0;
    for (sp, _) in &spans {
        assert!(sp.lo < sp.hi, "empty span at {}", sp.lo);
        assert!(sp.lo >= prev_hi, "overlap at {}", sp.lo);
        assert!(sp.hi as usize <= src.len(), "out of file");
        prev_hi = sp.hi;
    }
    assert!(spans.iter().any(|(_, t)| *t == TokenType::Comment));
    assert!(spans.iter().any(|(_, t)| *t == TokenType::Keyword));
}

#[test]
fn broken_source_still_classifies_with_comments() {
    // missing closing brace — the recovery AST simply yields fewer names;
    // `entry` is token-owned by text now, so like the reserved words
    // (`let`, `fn`) it never drops its class
    let src = "// still colored\nentry fn main( {\n  let x = ;\n";
    let spans = classify_source(src, Mode::Impl);
    assert_eq!(find(src, &spans, "// still colored"), vec![TokenType::Comment]);
    assert_eq!(find(src, &spans, "entry"), vec![TokenType::Keyword]);
    assert_eq!(find(src, &spans, "let"), vec![TokenType::Keyword]);
    assert_eq!(find(src, &spans, "fn"), vec![TokenType::Keyword]);
}
