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
        .join("../../demo/src/examples/literals.rut");
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
    let src = "fn f() -> nil { log.info(f\"{color_name(Color.Red)} area={c.area()}\"); }";
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
                   return when (c) { Color.Red -> p, _ -> p, };\n\
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
    // `x as f64` is the conversion family (RFC 0007 §1) — the
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
    let src = "interface Drawable { fn draw(self, g: Canvas) -> nil; }\nimpl Drawable for Circle { fn draw(self, g: Canvas) -> nil {} }\nenum Color { Red }\npub fn main() -> nil {}\n";
    let (toks, _) = rut_lexer::lexer::lex(src);
    let (ast, _) = parse(src, Mode::Impl);
    let syms = symbols(&toks, &ast);
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Drawable", "impl Drawable for Circle", "Color", "main"]);
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
